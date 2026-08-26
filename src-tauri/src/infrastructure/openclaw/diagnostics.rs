//! `OpenClawDiagnosticsAdapter` — Phase 8 read-only diagnostics CLI calls.
//!
//! Every invocation is a structured `executable + argv` request through the
//! `ProcessPort` (S1/S2):
//!
//! - `agents list --json` (15s)
//! - `update status --json` (15s, same as Phase 1) — fail-soft to `Unknown`
//! - `logs --limit <n> --json` (30s) — one-shot tail; the limit is a plain
//!   `u32` rendered by `Display` as a single argv element (S2: no
//!   interpolation, no `--follow`)
//!
//! Row/event parsing is fail-soft (contract §2/§5): unknown fields are
//! ignored, missing fields become `null`, and unclassifiable log lines
//! become `Raw`.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;

use crate::domain::models::diagnostics::{AgentRow, LogEvent, LogsResult, UpdateStatusDetail};
use crate::domain::models::openclaw::UpdateState;
use crate::domain::ports::openclaw_diagnostics::OpenClawDiagnosticsPort;
use crate::domain::ports::process::{ProcessError, ProcessOutput, ProcessPort, ProcessRequest};
use crate::error::AppError;

/// Contract timeout for `agents list` (Phase 8: 15s).
const AGENTS_TIMEOUT: Duration = Duration::from_secs(15);
/// Contract timeout for `update status` (Phase 8: 15s — same as Phase 1).
const UPDATE_TIMEOUT: Duration = Duration::from_secs(15);
/// Contract timeout for `logs --limit` (Phase 8: 30s — tails may be large).
const LOGS_TIMEOUT: Duration = Duration::from_secs(30);

/// OpenClaw diagnostics adapter. All CLI invocations go through the
/// `ProcessPort`.
pub struct OpenClawDiagnosticsAdapter {
    process: Arc<dyn ProcessPort>,
}

impl OpenClawDiagnosticsAdapter {
    pub fn new(process: Arc<dyn ProcessPort>) -> Self {
        Self { process }
    }

    fn run_cli(
        &self,
        executable: &Path,
        argv: Vec<String>,
        timeout: Duration,
        label: &str,
    ) -> Result<ProcessOutput, AppError> {
        let request = ProcessRequest::new(executable.to_path_buf(), argv, timeout);
        match self.process.run(&request) {
            Ok(output) => Ok(output),
            Err(ProcessError::NotFound { .. }) => Err(AppError::openclaw_not_found()),
            Err(ProcessError::Timeout { .. }) => Err(AppError::process_timeout(label)),
            Err(ProcessError::SpawnFailed { message }) => {
                Err(AppError::process_failed(label, message))
            }
        }
    }
}

impl OpenClawDiagnosticsPort for OpenClawDiagnosticsAdapter {
    fn list_agents(&self, executable: &Path) -> Result<Vec<AgentRow>, AppError> {
        const LABEL: &str = "openclaw agents list --json";
        let output = self.run_cli(
            executable,
            vec!["agents".into(), "list".into(), "--json".into()],
            AGENTS_TIMEOUT,
            LABEL,
        )?;
        if output.exit_code != 0 {
            return Err(AppError::openclaw_agents_read_failed(format!(
                "{LABEL}: exit code {}: {}",
                output.exit_code,
                output.stderr.trim()
            )));
        }
        let value = serde_json::from_str::<Value>(&output.stdout).map_err(|err| {
            AppError::openclaw_agents_read_failed(format!(
                "{LABEL}: unparseable output: {} ({err})",
                output.stdout.trim()
            ))
        })?;
        // The rows live either at the top level (array) or under an
        // `agents` key (object envelope) — accept both (fail-soft).
        let rows_value = match &value {
            Value::Array(_) => value,
            Value::Object(map) => map.get("agents").cloned().unwrap_or(Value::Array(vec![])),
            other => {
                return Err(AppError::openclaw_agents_read_failed(format!(
                    "{LABEL}: expected array/object, got {}",
                    other
                )))
            }
        };
        let rows = match &rows_value {
            Value::Array(items) => items,
            other => {
                return Err(AppError::openclaw_agents_read_failed(format!(
                    "{LABEL}: expected a row array, got {}",
                    other
                )))
            }
        };
        Ok(rows.iter().filter_map(parse_agent_row).collect())
    }

    fn update_detail(&self, executable: &Path) -> Result<UpdateStatusDetail, AppError> {
        const LABEL: &str = "openclaw update status --json";
        let output = match self.run_cli(
            executable,
            vec!["update".into(), "status".into(), "--json".into()],
            UPDATE_TIMEOUT,
            LABEL,
        ) {
            Ok(output) => output,
            // The state simply cannot be determined; report Unknown
            // (Phase 1 policy — no error, no new stable code).
            Err(_) => return Ok(UpdateStatusDetail::unknown()),
        };
        Ok(parse_update_detail_json(&output.stdout))
    }

    fn tail_logs(&self, executable: &Path, limit: u32) -> Result<LogsResult, AppError> {
        // S2: the limit is a plain u32 rendered by Display — a single argv
        // element, never interpolated into a command string. `--follow` is
        // never emitted (non-goal).
        let label = format!("openclaw logs --limit {limit} --json");
        let output = self.run_cli(
            executable,
            vec![
                "logs".to_string(),
                "--limit".to_string(),
                limit.to_string(),
                "--json".to_string(),
            ],
            LOGS_TIMEOUT,
            &label,
        )?;
        if output.exit_code != 0 {
            return Err(AppError::openclaw_logs_read_failed(format!(
                "{label}: exit code {}: {}",
                output.exit_code,
                output.stderr.trim()
            )));
        }
        // An empty stdout (no log lines) is a successful zero-line tail.
        Ok(parse_logs_tail(&output.stdout))
    }
}

/// Parses one agent row (fail-soft: only a non-empty `id` is required).
fn parse_agent_row(raw: &Value) -> Option<AgentRow> {
    let id = raw.get("id")?.as_str()?.to_string();
    if id.trim().is_empty() {
        return None;
    }
    let default = raw
        .get("default")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let name = raw.get("name").and_then(|v| v.as_str()).map(str::to_string);
    let emoji = raw
        .get("emoji")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let workspace = raw
        .get("workspace")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let bindings = raw.get("bindings").and_then(|v| v.as_u64());
    Some(AgentRow {
        id,
        default,
        name,
        emoji,
        workspace,
        bindings,
    })
}

/// Payload for `openclaw update status --json` (Phase 1 shape).
#[derive(serde::Deserialize)]
struct UpdateDetailPayload {
    current: Option<String>,
    latest: Option<String>,
    #[serde(rename = "updateAvailable", default)]
    update_available: Option<bool>,
}

/// Parses `openclaw update status --json` stdout into the Phase 8 detail
/// type.
///
/// Any parse failure or missing data resolves to `UpdateStatusDetail::
/// unknown()` (fail-soft — the state cannot be determined, which is a
/// valid structured answer; no error, no new stable code).
pub(crate) fn parse_update_detail_json(stdout: &str) -> UpdateStatusDetail {
    let payload: UpdateDetailPayload = match serde_json::from_str(stdout) {
        Ok(payload) => payload,
        Err(_) => return UpdateStatusDetail::unknown(),
    };
    let (Some(current), Some(latest)) = (payload.current, payload.latest) else {
        return UpdateStatusDetail::unknown();
    };
    let available = payload.update_available.unwrap_or(current != latest);
    UpdateStatusDetail {
        state: if available {
            UpdateState::UpdateAvailable
        } else {
            UpdateState::Updated
        },
        current: Some(current),
        latest: Some(latest),
    }
}

/// Parses a `logs --limit <n> --json` stdout into `LogsResult`.
///
/// Line-by-line, fail-soft: empty lines are skipped; any line that is not a
/// JSON object with a recognized `type` tag becomes `Raw`. `source` comes
/// from the first `meta` event's `file`; `truncated` from the first
/// `notice` event that reports it.
pub(crate) fn parse_logs_tail(stdout: &str) -> LogsResult {
    let lines: Vec<LogEvent> = stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(parse_log_event)
        .collect();
    let source = lines.iter().find_map(|event| match event {
        LogEvent::Meta { file, .. } => file.clone().filter(|f| !f.trim().is_empty()),
        _ => None,
    });
    let truncated = lines.iter().any(|event| {
        matches!(
            event,
            LogEvent::Notice {
                truncated: Some(true),
                ..
            }
        )
    });
    LogsResult {
        lines,
        source,
        truncated,
    }
}

/// First non-null string (or number-as-text) among the given keys.
fn opt_text(obj: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        obj.get(*key)
            .and_then(|v| {
                v.as_str()
                    .map(str::to_string)
                    .or_else(|| v.as_u64().map(|n| n.to_string()))
            })
            .filter(|s| !s.trim().is_empty())
    })
}

/// Parses one type-tagged log event line (fail-soft → `Raw`).
fn parse_log_event(line: &str) -> LogEvent {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return LogEvent::Raw {
            line: line.to_string(),
        };
    };
    let Some(obj) = value.as_object() else {
        return LogEvent::Raw {
            line: line.to_string(),
        };
    };
    let Some(kind) = obj.get("type").and_then(|v| v.as_str()) else {
        return LogEvent::Raw {
            line: line.to_string(),
        };
    };
    match kind {
        "log" => LogEvent::Log {
            time: opt_text(obj, &["time"]),
            level: opt_text(obj, &["level"]),
            subsystem: opt_text(obj, &["subsystem"]),
            // The message is the display text; a missing message still
            // yields a (blank) log row rather than dropping the line.
            message: obj
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            hostname: opt_text(obj, &["hostname"]),
            agent_id: opt_text(obj, &["agentId", "agent_id"]),
            session_id: opt_text(obj, &["sessionId", "session_id"]),
            channel: opt_text(obj, &["channel"]),
        },
        "meta" => LogEvent::Meta {
            file: opt_text(obj, &["file"]),
            source: opt_text(obj, &["source"]),
            source_kind: opt_text(obj, &["sourceKind", "source_kind"]),
            service: opt_text(obj, &["service"]),
            cursor: opt_text(obj, &["cursor"]),
            size: obj.get("size").and_then(|v| v.as_u64()),
        },
        "notice" => LogEvent::Notice {
            message: opt_text(obj, &["message"]),
            truncated: obj.get("truncated").and_then(|v| v.as_bool()),
        },
        "raw" => LogEvent::Raw {
            line: obj
                .get("line")
                .and_then(|v| v.as_str())
                .unwrap_or(line)
                .to_string(),
        },
        // `error` (stderr-only per the CLI docs) and any unknown kind:
        // keep the whole line as raw text.
        _ => LogEvent::Raw {
            line: line.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct ScriptedProcess {
        responses: Arc<Mutex<Vec<Result<ProcessOutput, ProcessError>>>>,
        requests: Arc<Mutex<Vec<ProcessRequest>>>,
    }

    impl ProcessPort for ScriptedProcess {
        fn run(&self, request: &ProcessRequest) -> Result<ProcessOutput, ProcessError> {
            self.requests.lock().unwrap().push(request.clone());
            let mut queue = self.responses.lock().unwrap();
            match queue.first().cloned() {
                Some(response) => {
                    let _ = queue.remove(0);
                    response
                }
                None => Ok(ProcessOutput {
                    stdout: "[]".into(),
                    stderr: String::new(),
                    exit_code: 0,
                }),
            }
        }
    }

    fn scripted(
        responses: Vec<Result<ProcessOutput, ProcessError>>,
    ) -> (OpenClawDiagnosticsAdapter, Arc<Mutex<Vec<ProcessRequest>>>) {
        let requests: Arc<Mutex<Vec<ProcessRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let fake = Arc::new(ScriptedProcess {
            responses: Arc::new(Mutex::new(responses)),
            requests: Arc::clone(&requests),
        });
        (OpenClawDiagnosticsAdapter::new(fake), requests)
    }

    fn output(exit_code: i32, stdout: &str, stderr: &str) -> ProcessOutput {
        ProcessOutput {
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
            exit_code,
        }
    }

    const EXE: &str = "C:\\fake\\openclaw.exe";

    // --- agents ----------------------------------------------------------------

    #[test]
    fn list_agents_exact_argv_and_rows() {
        let body = r#"{"ok":true,"agents":[
            {"id":"main","default":true,"name":"Main Agent","emoji":"🦞","workspace":"~/openclaw-main","bindings":2},
            {"id":"ops","name":"운영 에이전트"},
            {"id":"minimal"}
        ]}"#;
        let (adapter, requests) = scripted(vec![Ok(output(0, body, ""))]);
        let rows = adapter.list_agents(Path::new(EXE)).expect("parse");
        assert_eq!(rows.len(), 3, "no row may be dropped");
        assert_eq!(rows[0].id, "main");
        assert!(rows[0].default);
        assert_eq!(rows[0].name.as_deref(), Some("Main Agent"));
        assert_eq!(rows[0].emoji.as_deref(), Some("🦞"));
        assert_eq!(rows[0].workspace.as_deref(), Some("~/openclaw-main"));
        assert_eq!(rows[0].bindings, Some(2));
        // Fail-soft: optional fields absent → None, `default` defaults false.
        assert_eq!(rows[1].id, "ops");
        assert!(!rows[1].default);
        assert_eq!(rows[1].name.as_deref(), Some("운영 에이전트"));
        assert_eq!(rows[1].bindings, None);
        assert_eq!(rows[2].id, "minimal");
        assert!(rows[2].name.is_none());
        let argv = requests.lock().unwrap()[0].argv.clone();
        assert_eq!(argv, vec!["agents", "list", "--json"]);
    }

    #[test]
    fn list_agents_accepts_bare_array_shape() {
        let body = r#"[{"id":"a","default":true}]"#;
        let (adapter, _) = scripted(vec![Ok(output(0, body, ""))]);
        let rows = adapter.list_agents(Path::new(EXE)).expect("parse");
        assert_eq!(rows.len(), 1);
        assert!(rows[0].default);
    }

    #[test]
    fn list_agents_malformed_json_is_read_failed() {
        let (adapter, _) = scripted(vec![Ok(output(
            0,
            r#"{"agents":[{"id":"weather","enabled":tru"#,
            "",
        ))]);
        let err = adapter.list_agents(Path::new(EXE)).expect_err("must fail");
        assert_eq!(err.code, "openclaw-agents-read-failed");
    }

    #[test]
    fn list_agents_nonzero_exit_is_read_failed() {
        let (adapter, _) = scripted(vec![Ok(output(2, "", "boom"))]);
        let err = adapter.list_agents(Path::new(EXE)).expect_err("must fail");
        assert_eq!(err.code, "openclaw-agents-read-failed");
        assert!(err.message.contains("boom"));
    }

    #[test]
    fn list_agents_rows_without_id_are_skipped() {
        let body = r#"{"agents":[{"default":true},{"id":""},{"id":"ok"}]}"#;
        let (adapter, _) = scripted(vec![Ok(output(0, body, ""))]);
        let rows = adapter.list_agents(Path::new(EXE)).expect("parse");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "ok");
    }

    #[test]
    fn list_agents_process_errors_map_to_stable_codes() {
        let (adapter, _) = scripted(vec![Err(ProcessError::NotFound {
            executable: EXE.into(),
        })]);
        assert_eq!(
            adapter.list_agents(Path::new(EXE)).unwrap_err().code,
            "openclaw-not-found"
        );
        let (adapter, _) = scripted(vec![Err(ProcessError::Timeout {
            executable: EXE.into(),
        })]);
        assert_eq!(
            adapter.list_agents(Path::new(EXE)).unwrap_err().code,
            "process-timeout"
        );
        let (adapter, _) = scripted(vec![Err(ProcessError::SpawnFailed {
            message: "spawn failed: denied".into(),
        })]);
        assert_eq!(
            adapter.list_agents(Path::new(EXE)).unwrap_err().code,
            "process-failed"
        );
    }

    // --- update detail -----------------------------------------------------------

    #[test]
    fn update_detail_exact_argv_and_versions() {
        let body = r#"{"current":"2026.7.1","latest":"2026.7.1-2","updateAvailable":true}"#;
        let (adapter, requests) = scripted(vec![Ok(output(0, body, ""))]);
        let detail = adapter.update_detail(Path::new(EXE)).expect("parse");
        assert_eq!(detail.state, UpdateState::UpdateAvailable);
        assert_eq!(detail.current.as_deref(), Some("2026.7.1"));
        assert_eq!(detail.latest.as_deref(), Some("2026.7.1-2"));
        let argv = requests.lock().unwrap()[0].argv.clone();
        assert_eq!(argv, vec!["update", "status", "--json"]);
    }

    #[test]
    fn update_detail_updated_payload() {
        let body = r#"{"current":"2026.7.1-2","latest":"2026.7.1-2","updateAvailable":false}"#;
        let (adapter, _) = scripted(vec![Ok(output(0, body, ""))]);
        let detail = adapter.update_detail(Path::new(EXE)).expect("parse");
        assert_eq!(detail.state, UpdateState::Updated);
        assert_eq!(detail.current.as_deref(), Some("2026.7.1-2"));
    }

    #[test]
    fn update_detail_malformed_is_unknown_not_error() {
        let (adapter, _) = scripted(vec![Ok(output(0, "totally not json", ""))]);
        let detail = adapter
            .update_detail(Path::new(EXE))
            .expect("fail-soft: never an error");
        assert_eq!(detail, UpdateStatusDetail::unknown());
    }

    #[test]
    fn update_detail_missing_fields_is_unknown() {
        let (adapter, _) = scripted(vec![Ok(output(0, r#"{"status":"ok"}"#, ""))]);
        let detail = adapter
            .update_detail(Path::new(EXE))
            .expect("fail-soft: never an error");
        assert_eq!(detail, UpdateStatusDetail::unknown());
    }

    #[test]
    fn update_detail_process_failure_is_unknown() {
        let (adapter, _) = scripted(vec![Err(ProcessError::Timeout {
            executable: EXE.into(),
        })]);
        let detail = adapter
            .update_detail(Path::new(EXE))
            .expect("fail-soft: never an error");
        assert_eq!(detail, UpdateStatusDetail::unknown());
        let (adapter, _) = scripted(vec![Err(ProcessError::NotFound {
            executable: EXE.into(),
        })]);
        let detail = adapter
            .update_detail(Path::new(EXE))
            .expect("fail-soft: never an error");
        assert_eq!(detail, UpdateStatusDetail::unknown());
    }

    // --- logs ----------------------------------------------------------------------

    const TAIL: &str = r#"{"type":"meta","file":"openclaw-2026-08-26.log","source":"file log","sourceKind":"file","service":"gateway","cursor":"0","size":4096}
{"type":"log","time":"2026-08-26T10:00:00.000Z","level":"info","subsystem":"gateway","message":"gateway started","hostname":"host-1"}
{"type":"log","time":"2026-08-26T10:00:01.000Z","level":"info","agent_id":"main","session_id":"s-1","message":"session bootstrapped"}
{"type":"raw","line":"unparsed legacy line"}
{"type":"notice","message":"showing most recent lines","truncated":true}"#;

    #[test]
    fn tail_logs_exact_argv_with_limit() {
        let (adapter, requests) = scripted(vec![Ok(output(0, TAIL, ""))]);
        adapter.tail_logs(Path::new(EXE), 50).expect("parse");
        let request = &requests.lock().unwrap()[0];
        assert_eq!(
            request.argv,
            vec![
                "logs".to_string(),
                "--limit".to_string(),
                "50".to_string(),
                "--json".to_string()
            ]
        );
        // The argv carries the limit as its own element — never `--follow`.
        assert!(!request.argv.iter().any(|arg| arg == "--follow"));
    }

    #[test]
    fn tail_logs_parses_type_tagged_events() {
        let (adapter, _) = scripted(vec![Ok(output(0, TAIL, ""))]);
        let result = adapter.tail_logs(Path::new(EXE), 200).expect("parse");
        assert_eq!(result.lines.len(), 5);
        match &result.lines[0] {
            LogEvent::Meta {
                file,
                source_kind,
                size,
                ..
            } => {
                assert_eq!(file.as_deref(), Some("openclaw-2026-08-26.log"));
                assert_eq!(source_kind.as_deref(), Some("file"));
                assert_eq!(*size, Some(4096));
            }
            other => panic!("expected meta, got {other:?}"),
        }
        match &result.lines[1] {
            LogEvent::Log {
                level,
                subsystem,
                message,
                hostname,
                ..
            } => {
                assert_eq!(level.as_deref(), Some("info"));
                assert_eq!(subsystem.as_deref(), Some("gateway"));
                assert_eq!(message, "gateway started");
                assert_eq!(hostname.as_deref(), Some("host-1"));
            }
            other => panic!("expected log, got {other:?}"),
        }
        match &result.lines[2] {
            LogEvent::Log {
                agent_id,
                session_id,
                ..
            } => {
                assert_eq!(agent_id.as_deref(), Some("main"));
                assert_eq!(session_id.as_deref(), Some("s-1"));
            }
            other => panic!("expected log, got {other:?}"),
        }
        assert!(matches!(result.lines[3], LogEvent::Raw { .. }));
        assert!(matches!(result.lines[4], LogEvent::Notice { .. }));
        // `source` and `truncated` derive from the meta/notice events.
        assert_eq!(result.source.as_deref(), Some("openclaw-2026-08-26.log"));
        assert!(result.truncated);
    }

    #[test]
    fn tail_logs_non_json_line_becomes_raw() {
        let body = "plain text line\n{\"type\":\"log\",\"message\":\"ok\"}\n";
        let (adapter, _) = scripted(vec![Ok(output(0, body, ""))]);
        let result = adapter.tail_logs(Path::new(EXE), 10).expect("parse");
        assert_eq!(result.lines.len(), 2);
        match &result.lines[0] {
            LogEvent::Raw { line } => assert_eq!(line, "plain text line"),
            other => panic!("expected raw, got {other:?}"),
        }
        assert!(!result.truncated);
        assert_eq!(result.source, None);
    }

    #[test]
    fn tail_logs_error_typed_line_becomes_raw() {
        let body = "{\"type\":\"error\",\"message\":\"gateway unreachable\"}\n";
        let (adapter, _) = scripted(vec![Ok(output(0, body, ""))]);
        let result = adapter.tail_logs(Path::new(EXE), 10).expect("parse");
        assert_eq!(result.lines.len(), 1);
        assert!(matches!(result.lines[0], LogEvent::Raw { .. }));
    }

    #[test]
    fn tail_logs_empty_stdout_is_zero_lines_success() {
        let (adapter, _) = scripted(vec![Ok(output(0, "", ""))]);
        let result = adapter
            .tail_logs(Path::new(EXE), 10)
            .expect("empty is success");
        assert!(result.lines.is_empty());
        assert_eq!(result.source, None);
        assert!(!result.truncated);
        let (adapter, _) = scripted(vec![Ok(output(0, "\n  \n", ""))]);
        let result = adapter
            .tail_logs(Path::new(EXE), 10)
            .expect("blank lines only");
        assert!(result.lines.is_empty());
    }

    #[test]
    fn tail_logs_nonzero_exit_is_read_failed() {
        let (adapter, _) = scripted(vec![Ok(output(1, "", "gateway stopped"))]);
        let err = adapter
            .tail_logs(Path::new(EXE), 10)
            .expect_err("must fail");
        assert_eq!(err.code, "openclaw-logs-read-failed");
        assert!(err.message.contains("gateway stopped"));
    }

    #[test]
    fn tail_logs_missing_executable_is_not_found() {
        let (adapter, _) = scripted(vec![Err(ProcessError::NotFound {
            executable: EXE.into(),
        })]);
        assert_eq!(
            adapter.tail_logs(Path::new(EXE), 10).unwrap_err().code,
            "openclaw-not-found"
        );
    }

    #[test]
    fn tail_logs_timeout_is_process_timeout() {
        let (adapter, _) = scripted(vec![Err(ProcessError::Timeout {
            executable: EXE.into(),
        })]);
        assert_eq!(
            adapter.tail_logs(Path::new(EXE), 10).unwrap_err().code,
            "process-timeout"
        );
    }

    // --- parser direct cases -----------------------------------------------------

    #[test]
    fn parse_update_detail_infers_available_from_diff() {
        let detail = parse_update_detail_json(r#"{"current":"1.0.0","latest":"1.0.1"}"#);
        assert_eq!(detail.state, UpdateState::UpdateAvailable);
        assert_eq!(detail.current.as_deref(), Some("1.0.0"));
        assert_eq!(detail.latest.as_deref(), Some("1.0.1"));
    }
}
