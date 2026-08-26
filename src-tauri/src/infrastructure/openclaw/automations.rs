//! `OpenClawAutomationsAdapter` — non-interactive `openclaw automations` CLI
//! invocations (Phase 7).
//!
//! Every invocation is a structured `executable + argv` request through the
//! `ProcessPort` (S1/S2) with the exact contract argv:
//!
//! - `automations list --all --json`
//! - `automations get <job_id> --json`
//! - `automations add --name <name> <schedule flags> --session main
//!   --system-event <text> --wake <wake> --json` (reminder) /
//!   `--session isolated --message <text> --json` (task)
//! - `automations edit <job_id> --name <name> <schedule flags>
//!   --system-event <text> [--wake <wake>] | --message <text> --json`
//! - `automations enable <job_id> --json` / `automations disable <job_id> --json`
//! - `automations remove <job_id> --json`
//!
//! No `show`/`run`/`runs` (non-goals), no non-goal flags
//! (`--command`/`--script`/`--webhook`/`--model`/`--channel`/...), no
//! `--session current`/`session:<id>`. User text is a single argv element
//! (byte-for-byte, S2). Row/detail parsing is fail-soft (contract §2).

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use crate::domain::models::automations::{
    parse_automation_job, parse_automation_job_row, AutomationJob, AutomationJobRow,
};
use crate::domain::ports::openclaw_automations::OpenClawAutomationsPort;
use crate::domain::ports::process::{ProcessError, ProcessOutput, ProcessPort, ProcessRequest};
use crate::error::AppError;

/// Contract timeout for all automations calls (Phase 7: 30s).
const AUTOMATIONS_TIMEOUT: Duration = Duration::from_secs(30);

/// OpenClaw automations adapter. All CLI invocations go through the
/// `ProcessPort`.
pub struct OpenClawAutomationsAdapter {
    process: Arc<dyn ProcessPort>,
}

impl OpenClawAutomationsAdapter {
    pub fn new(process: Arc<dyn ProcessPort>) -> Self {
        Self { process }
    }

    fn run_cli(
        &self,
        executable: &Path,
        argv: Vec<String>,
        label: &str,
    ) -> Result<ProcessOutput, AppError> {
        let request = ProcessRequest::new(executable.to_path_buf(), argv, AUTOMATIONS_TIMEOUT);
        match self.process.run(&request) {
            Ok(output) => Ok(output),
            Err(ProcessError::NotFound { .. }) => Err(AppError::openclaw_not_found()),
            Err(ProcessError::Timeout { .. }) => Err(AppError::process_timeout(label)),
            Err(ProcessError::SpawnFailed { message }) => {
                Err(AppError::process_failed(label, message))
            }
        }
    }

    /// The schedule flags: `--at <value>` | `--every <value>` |
    /// `--cron <value>` (+ `--tz <iana>` for cron only).
    fn schedule_flags(kind: &str, value: &str, tz: Option<&str>) -> Vec<String> {
        let flag = match kind {
            "every" => "--every",
            "cron" => "--cron",
            _ => "--at",
        };
        let mut flags = vec![flag.to_string(), value.to_string()];
        if kind == "cron" {
            if let Some(tz) = tz {
                flags.push("--tz".to_string());
                flags.push(tz.to_string());
            }
        }
        flags
    }

    /// The payload flags. The fixed pairing (contract §1): reminder →
    /// `main` + `--system-event` (+ `--wake`, default `now`), task →
    /// `isolated` + `--message` (no `--wake`).
    fn payload_flags(payload_kind: &str, text: &str, wake: Option<&str>) -> (String, Vec<String>) {
        if payload_kind == "task" {
            (
                "isolated".to_string(),
                vec!["--message".to_string(), text.to_string()],
            )
        } else {
            (
                "main".to_string(),
                vec![
                    "--system-event".to_string(),
                    text.to_string(),
                    "--wake".to_string(),
                    wake.unwrap_or("now").to_string(),
                ],
            )
        }
    }

    /// Extracts the new job id from the `add` result (fail-soft, unverified
    /// item 3): `id` preferred, `jobId` fallback.
    fn extract_job_id(label: &str, value: &serde_json::Value) -> Result<String, AppError> {
        let job_id = value
            .get("id")
            .or_else(|| value.get("jobId"))
            .and_then(|v| v.as_str())
            .filter(|id| !id.is_empty());
        match job_id {
            Some(id) => Ok(id.to_string()),
            None => Err(AppError::openclaw_automations_failed(format!(
                "{label}: no job id in the add result"
            ))),
        }
    }
}

/// Extracts the failure message from a CLI JSON failure envelope
/// (`{"ok":false,"error":...}`) — the Phase 1/5 convention. Returns `None`
/// when the stdout carries no envelope.
fn parse_failure_envelope(stdout: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(stdout.trim()).ok()?;
    if value.get("ok").and_then(|ok| ok.as_bool()) != Some(false) {
        return None;
    }
    match &value["error"] {
        serde_json::Value::String(message) => Some(message.clone()),
        serde_json::Value::Object(map) => map
            .get("message")
            .and_then(|m| m.as_str())
            .map(str::to_string),
        other if other.is_null() => None,
        _ => None,
    }
}

/// Builds the failure detail for a non-zero exit (envelope + stderr).
fn failure_detail(label: &str, output: &ProcessOutput) -> String {
    let stderr = output.stderr.trim();
    let detail = match (parse_failure_envelope(&output.stdout), stderr.is_empty()) {
        (Some(envelope), true) => envelope,
        (Some(envelope), false) => format!("{envelope} ({stderr})"),
        (None, false) => format!("exit code {} ({stderr})", output.exit_code),
        (None, true) => format!("exit code {}", output.exit_code),
    };
    format!("{label}: {detail}")
}

/// Parses a successful `--json` stdout document (stable code on parse
/// failure).
fn parse_json(label: &str, output: &ProcessOutput) -> Result<serde_json::Value, AppError> {
    serde_json::from_str::<serde_json::Value>(&output.stdout).map_err(|err| {
        AppError::openclaw_automations_failed(format!(
            "{label}: unparseable output: {} ({err})",
            output.stdout.trim()
        ))
    })
}

impl OpenClawAutomationsPort for OpenClawAutomationsAdapter {
    fn list_automations(&self, executable: &Path) -> Result<Vec<AutomationJobRow>, AppError> {
        const LABEL: &str = "openclaw automations list --all --json";
        let output = self.run_cli(
            executable,
            vec![
                "automations".into(),
                "list".into(),
                "--all".into(),
                "--json".into(),
            ],
            LABEL,
        )?;
        if output.exit_code != 0 {
            return Err(AppError::openclaw_automations_failed(failure_detail(
                LABEL, &output,
            )));
        }
        let value = parse_json(LABEL, &output)?;
        // Rows live at the top level (array) or under a `jobs`/`automations`
        // key (object envelope) — accept both (fail-soft, unverified item 1).
        let items = match &value {
            serde_json::Value::Array(items) => items.clone(),
            serde_json::Value::Object(map) => map
                .get("jobs")
                .or_else(|| map.get("automations"))
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        Ok(items.iter().filter_map(parse_automation_job_row).collect())
    }

    fn get_automation(&self, executable: &Path, job_id: &str) -> Result<AutomationJob, AppError> {
        const LABEL: &str = "openclaw automations get";
        let output = self.run_cli(
            executable,
            vec![
                "automations".into(),
                "get".into(),
                job_id.to_string(),
                "--json".into(),
            ],
            LABEL,
        )?;
        if output.exit_code != 0 {
            return Err(AppError::openclaw_automations_failed(failure_detail(
                LABEL, &output,
            )));
        }
        let value = parse_json(LABEL, &output)?;
        Ok(parse_automation_job(&value, job_id))
    }

    fn add_automation(
        &self,
        executable: &Path,
        name: &str,
        schedule_kind: &str,
        schedule_value: &str,
        schedule_tz: Option<&str>,
        payload_kind: &str,
        text: &str,
        wake: Option<&str>,
    ) -> Result<String, AppError> {
        const LABEL: &str = "openclaw automations add";
        let (session, payload_argv) = Self::payload_flags(payload_kind, text, wake);
        let mut argv = vec![
            "automations".to_string(),
            "add".to_string(),
            "--name".to_string(),
            name.to_string(),
        ];
        argv.extend(Self::schedule_flags(
            schedule_kind,
            schedule_value,
            schedule_tz,
        ));
        argv.push("--session".to_string());
        argv.push(session);
        argv.extend(payload_argv);
        argv.push("--json".to_string());
        let output = self.run_cli(executable, argv, LABEL)?;
        if output.exit_code != 0 {
            return Err(AppError::openclaw_automations_failed(failure_detail(
                LABEL, &output,
            )));
        }
        let value = parse_json(LABEL, &output)?;
        Self::extract_job_id(LABEL, &value)
    }

    fn edit_automation(
        &self,
        executable: &Path,
        job_id: &str,
        name: &str,
        schedule_kind: &str,
        schedule_value: &str,
        schedule_tz: Option<&str>,
        payload_kind: &str,
        text: &str,
        wake: Option<&str>,
    ) -> Result<(), AppError> {
        const LABEL: &str = "openclaw automations edit";
        // Edit has no `--session` (contract §4): the payload text flag
        // alone identifies the payload kind.
        let (_session, payload_argv) = Self::payload_flags(payload_kind, text, wake);
        let mut argv = vec![
            "automations".to_string(),
            "edit".to_string(),
            job_id.to_string(),
            "--name".to_string(),
            name.to_string(),
        ];
        argv.extend(Self::schedule_flags(
            schedule_kind,
            schedule_value,
            schedule_tz,
        ));
        argv.extend(payload_argv);
        argv.push("--json".to_string());
        let output = self.run_cli(executable, argv, LABEL)?;
        if output.exit_code != 0 {
            return Err(AppError::openclaw_automations_failed(failure_detail(
                LABEL, &output,
            )));
        }
        Ok(())
    }

    fn set_automation_enabled(
        &self,
        executable: &Path,
        job_id: &str,
        enabled: bool,
    ) -> Result<(), AppError> {
        const LABEL: &str = "openclaw automations enable/disable";
        let action = if enabled { "enable" } else { "disable" };
        let output = self.run_cli(
            executable,
            vec![
                "automations".into(),
                action.into(),
                job_id.to_string(),
                "--json".into(),
            ],
            LABEL,
        )?;
        if output.exit_code != 0 {
            return Err(AppError::openclaw_automations_failed(failure_detail(
                LABEL, &output,
            )));
        }
        Ok(())
    }

    fn remove_automation(&self, executable: &Path, job_id: &str) -> Result<(), AppError> {
        const LABEL: &str = "openclaw automations remove";
        let output = self.run_cli(
            executable,
            vec![
                "automations".into(),
                "remove".into(),
                job_id.to_string(),
                "--json".into(),
            ],
            LABEL,
        )?;
        if output.exit_code != 0 {
            return Err(AppError::openclaw_automations_failed(failure_detail(
                LABEL, &output,
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct ScriptedProcess {
        responses: Arc<std::sync::Mutex<Vec<Result<ProcessOutput, ProcessError>>>>,
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
                    stdout: "{}".into(),
                    stderr: String::new(),
                    exit_code: 0,
                }),
            }
        }
    }

    fn scripted(
        responses: Vec<Result<ProcessOutput, ProcessError>>,
    ) -> (OpenClawAutomationsAdapter, Arc<Mutex<Vec<ProcessRequest>>>) {
        let requests: Arc<Mutex<Vec<ProcessRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let fake = Arc::new(ScriptedProcess {
            responses: Arc::new(Mutex::new(responses)),
            requests: Arc::clone(&requests),
        });
        (OpenClawAutomationsAdapter::new(fake), requests)
    }

    fn output(exit_code: i32, stdout: &str, stderr: &str) -> ProcessOutput {
        ProcessOutput {
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
            exit_code,
        }
    }

    const EXE: &str = "C:\\fake\\openclaw.exe";

    /// The non-goal flags that must never appear in any argv (contract §3).
    const NON_GOAL_FLAGS: [&str; 13] = [
        "--command",
        "--command-argv",
        "--script",
        "--trigger-script",
        "--webhook",
        "--model",
        "--fallbacks",
        "--thinking",
        "--channel",
        "--to",
        "--thread-id",
        "--account",
        "--agent",
    ];

    fn assert_no_non_goal_flags(requests: &Arc<Mutex<Vec<ProcessRequest>>>) {
        for request in requests.lock().unwrap().iter() {
            for arg in &request.argv {
                assert!(
                    !NON_GOAL_FLAGS.contains(&arg.as_str()),
                    "non-goal flag {arg} in argv: {:?}",
                    request.argv
                );
            }
        }
    }

    #[test]
    fn list_automations_exact_argv_and_rows() {
        let body = r#"{"ok":true,"jobs":[
            {"id":"job-1","name":"알림","enabled":true,"status":"ok","nextRunAtMs":123,
             "schedule":{"kind":"at","at":"2099-01-01T00:00:00Z"},
             "payload":{"kind":"reminder","text":"약속"}},
            {"id":"job-2","status":"weird-future-status"},
            {"name":"dropped"}
        ]}"#;
        let (adapter, requests) = scripted(vec![Ok(output(0, body, ""))]);
        let rows = adapter.list_automations(Path::new(EXE)).expect("parse");
        assert_eq!(rows.len(), 2, "id-less rows dropped");
        assert_eq!(rows[0].id, "job-1");
        assert_eq!(rows[0].schedule.as_ref().expect("schedule").kind, "at");
        assert_eq!(rows[1].status.as_deref(), Some("weird-future-status"));
        assert_eq!(
            requests.lock().unwrap()[0].argv,
            vec!["automations", "list", "--all", "--json"]
        );
        assert_eq!(requests.lock().unwrap()[0].timeout, AUTOMATIONS_TIMEOUT);
        assert_no_non_goal_flags(&requests);
    }

    #[test]
    fn list_automations_accepts_bare_array_shape() {
        let (adapter, _) = scripted(vec![Ok(output(
            0,
            r#"[{"id":"job-1","enabled":true}]"#,
            "",
        ))]);
        let rows = adapter.list_automations(Path::new(EXE)).expect("parse");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].enabled, Some(true));
    }

    #[test]
    fn list_automations_nonzero_is_failed_with_envelope() {
        let body = r#"{"ok":false,"error":{"type":"cli_error","message":"state db unreadable"}}"#;
        let (adapter, _) = scripted(vec![Ok(output(1, body, ""))]);
        let err = adapter
            .list_automations(Path::new(EXE))
            .expect_err("nonzero exit");
        assert_eq!(err.code, "openclaw-automations-failed");
        assert!(err.message.contains("state db unreadable"));
    }

    #[test]
    fn list_automations_nonzero_without_envelope_keeps_stderr_masked() {
        let (adapter, _) = scripted(vec![Ok(output(2, "", "boom: token sk-fake123456789"))]);
        let err = adapter
            .list_automations(Path::new(EXE))
            .expect_err("nonzero exit");
        assert_eq!(err.code, "openclaw-automations-failed");
        // S3: the secret in stderr must be masked in the error message.
        assert!(!err.message.contains("sk-fake123456789"));
        assert!(err.message.contains("sk-****"));
    }

    #[test]
    fn list_automations_malformed_output_is_failed() {
        let (adapter, _) = scripted(vec![Ok(output(0, "jobs truncated {", ""))]);
        let err = adapter
            .list_automations(Path::new(EXE))
            .expect_err("malformed");
        assert_eq!(err.code, "openclaw-automations-failed");
    }

    #[test]
    fn get_automation_exact_argv_and_detail() {
        let body = r#"{"ok":true,"job":{"id":"job-1","name":"알림","enabled":true,"status":"ok","schedule":{"kind":"cron","cron":"0 9 * * 1","tz":"Asia/Seoul"},"payload":{"kind":"task","text":"보고서"}}}"#;
        let (adapter, requests) = scripted(vec![Ok(output(0, body, ""))]);
        let job = adapter
            .get_automation(Path::new(EXE), "job-1")
            .expect("parse");
        assert_eq!(job.id, "job-1");
        assert_eq!(job.schedule.expect("schedule").kind, "cron");
        assert_eq!(job.payload.expect("payload").kind, "task");
        assert_eq!(
            requests.lock().unwrap()[0].argv,
            vec!["automations", "get", "job-1", "--json"]
        );
    }

    #[test]
    fn add_reminder_exact_argv_with_fixed_session_and_wake_default() {
        let body = r#"{"ok":true,"id":"job-1"}"#;
        let (adapter, requests) = scripted(vec![Ok(output(0, body, ""))]);
        let job_id = adapter
            .add_automation(
                Path::new(EXE),
                "한글 이름 \"공백\"",
                "at",
                "2027-02-01T16:00:00Z",
                None,
                "reminder",
                "따옴표 \"와\" 공백, text",
                None,
            )
            .expect("add");
        assert_eq!(job_id, "job-1");
        assert_eq!(
            requests.lock().unwrap()[0].argv,
            vec![
                "automations",
                "add",
                "--name",
                "한글 이름 \"공백\"",
                "--at",
                "2027-02-01T16:00:00Z",
                "--session",
                "main",
                "--system-event",
                "따옴표 \"와\" 공백, text",
                "--wake",
                "now",
                "--json",
            ]
        );
        assert_eq!(requests.lock().unwrap()[0].timeout, AUTOMATIONS_TIMEOUT);
        assert_no_non_goal_flags(&requests);
    }

    #[test]
    fn add_task_exact_argv_with_no_wake() {
        let body = r#"{"ok":true,"id":"job-2"}"#;
        let (adapter, requests) = scripted(vec![Ok(output(0, body, ""))]);
        let job_id = adapter
            .add_automation(
                Path::new(EXE),
                "task job",
                "cron",
                "0 9 * * 1",
                Some("Asia/Seoul"),
                "task",
                "보고서",
                Some("now"),
            )
            .expect("add");
        assert_eq!(job_id, "job-2");
        let argv = requests.lock().unwrap()[0].argv.clone();
        assert_eq!(
            argv,
            vec![
                "automations",
                "add",
                "--name",
                "task job",
                "--cron",
                "0 9 * * 1",
                "--tz",
                "Asia/Seoul",
                "--session",
                "isolated",
                "--message",
                "보고서",
                "--json",
            ]
        );
        // A task never emits `--wake` (even when a wake value is passed in —
        // the service validates that first).
        assert!(!argv.iter().any(|arg| arg == "--wake"));
        assert_no_non_goal_flags(&requests);
    }

    #[test]
    fn add_job_id_falls_back_to_jobid_field() {
        let (adapter, _) = scripted(vec![Ok(output(0, r#"{"ok":true,"jobId":"job-9"}"#, ""))]);
        let job_id = adapter
            .add_automation(
                Path::new(EXE),
                "n",
                "every",
                "10m",
                None,
                "reminder",
                "t",
                Some("now"),
            )
            .expect("add");
        assert_eq!(job_id, "job-9");
    }

    #[test]
    fn add_without_job_id_is_failed() {
        for body in [r#"{"ok":true}"#, r#"{"id":""}"#, "not json"] {
            let (adapter, _) = scripted(vec![Ok(output(0, body, ""))]);
            let err = adapter
                .add_automation(
                    Path::new(EXE),
                    "n",
                    "every",
                    "10m",
                    None,
                    "reminder",
                    "t",
                    None,
                )
                .expect_err("no job id");
            assert_eq!(err.code, "openclaw-automations-failed", "{body}");
        }
    }

    #[test]
    fn edit_reminder_exact_argv_without_session() {
        let (adapter, requests) = scripted(vec![Ok(output(0, "{}", ""))]);
        adapter
            .edit_automation(
                Path::new(EXE),
                "job-1",
                "새 이름",
                "every",
                "30m",
                None,
                "reminder",
                "새 내용",
                Some("next-heartbeat"),
            )
            .expect("edit");
        assert_eq!(
            requests.lock().unwrap()[0].argv,
            vec![
                "automations",
                "edit",
                "job-1",
                "--name",
                "새 이름",
                "--every",
                "30m",
                "--system-event",
                "새 내용",
                "--wake",
                "next-heartbeat",
                "--json",
            ]
        );
        // Edit has no `--session` (contract §4).
        assert!(!requests.lock().unwrap()[0]
            .argv
            .iter()
            .any(|arg| arg == "--session"));
    }

    #[test]
    fn edit_task_exact_argv() {
        let (adapter, requests) = scripted(vec![Ok(output(0, "{}", ""))]);
        adapter
            .edit_automation(
                Path::new(EXE),
                "job-2",
                "task name",
                "at",
                "2027-03-01T09:00:00Z",
                None,
                "task",
                "보고서",
                None,
            )
            .expect("edit");
        assert_eq!(
            requests.lock().unwrap()[0].argv,
            vec![
                "automations",
                "edit",
                "job-2",
                "--name",
                "task name",
                "--at",
                "2027-03-01T09:00:00Z",
                "--message",
                "보고서",
                "--json",
            ]
        );
    }

    #[test]
    fn enable_disable_remove_exact_argv() {
        let (adapter, requests) = scripted(vec![
            Ok(output(0, "{}", "")),
            Ok(output(0, "{}", "")),
            Ok(output(0, "{}", "")),
        ]);
        adapter
            .set_automation_enabled(Path::new(EXE), "job-1", true)
            .expect("enable");
        adapter
            .set_automation_enabled(Path::new(EXE), "job-1", false)
            .expect("disable");
        adapter
            .remove_automation(Path::new(EXE), "job-1")
            .expect("remove");
        assert_eq!(
            requests.lock().unwrap()[0].argv,
            vec!["automations", "enable", "job-1", "--json"]
        );
        assert_eq!(
            requests.lock().unwrap()[1].argv,
            vec!["automations", "disable", "job-1", "--json"]
        );
        assert_eq!(
            requests.lock().unwrap()[2].argv,
            vec!["automations", "remove", "job-1", "--json"]
        );
        assert_no_non_goal_flags(&requests);
    }

    #[test]
    fn mutations_nonzero_is_failed_with_envelope() {
        let body = r#"{"ok":false,"error":{"type":"cli_error","message":"unknown job: job-404"}}"#;
        let (adapter, _) = scripted(vec![Ok(output(1, body, ""))]);
        assert_eq!(
            adapter
                .get_automation(Path::new(EXE), "job-404")
                .unwrap_err()
                .code,
            "openclaw-automations-failed"
        );
        let (adapter, _) = scripted(vec![Ok(output(1, body, ""))]);
        let err = adapter
            .add_automation(
                Path::new(EXE),
                "n",
                "every",
                "10m",
                None,
                "reminder",
                "t",
                None,
            )
            .unwrap_err();
        assert_eq!(err.code, "openclaw-automations-failed");
        assert!(err.message.contains("unknown job: job-404"));
        let (adapter, _) = scripted(vec![Ok(output(1, body, ""))]);
        assert_eq!(
            adapter
                .edit_automation(
                    Path::new(EXE),
                    "job-404",
                    "n",
                    "every",
                    "10m",
                    None,
                    "reminder",
                    "t",
                    None,
                )
                .unwrap_err()
                .code,
            "openclaw-automations-failed"
        );
        let (adapter, _) = scripted(vec![Ok(output(1, body, ""))]);
        assert_eq!(
            adapter
                .set_automation_enabled(Path::new(EXE), "job-404", true)
                .unwrap_err()
                .code,
            "openclaw-automations-failed"
        );
        let (adapter, _) = scripted(vec![Ok(output(1, body, ""))]);
        assert_eq!(
            adapter
                .remove_automation(Path::new(EXE), "job-404")
                .unwrap_err()
                .code,
            "openclaw-automations-failed"
        );
    }

    #[test]
    fn process_errors_map_to_stable_codes() {
        let (adapter, _) = scripted(vec![Err(ProcessError::NotFound {
            executable: EXE.into(),
        })]);
        assert_eq!(
            adapter.list_automations(Path::new(EXE)).unwrap_err().code,
            "openclaw-not-found"
        );
        let (adapter, _) = scripted(vec![Err(ProcessError::Timeout {
            executable: EXE.into(),
        })]);
        assert_eq!(
            adapter
                .get_automation(Path::new(EXE), "job-1")
                .unwrap_err()
                .code,
            "process-timeout"
        );
        let (adapter, _) = scripted(vec![Err(ProcessError::SpawnFailed {
            message: "spawn failed: denied".into(),
        })]);
        assert_eq!(
            adapter
                .add_automation(
                    Path::new(EXE),
                    "n",
                    "every",
                    "10m",
                    None,
                    "reminder",
                    "t",
                    None,
                )
                .unwrap_err()
                .code,
            "process-failed"
        );
    }
}
