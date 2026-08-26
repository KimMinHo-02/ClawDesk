//! Phase 8 contract tests: real `ProcessRunner` + fake `openclaw` CLI with a
//! per-test state sandbox (S5: fake CLI only, no real OpenClaw, no system
//! mutation — sandboxes live under the cargo target dir).
//!
//! Adapter/service-driven scenarios run inside serialized sub-scenarios
//! because the fake receives its sandbox via inherited process environment.
//! The fake `sk-` token in the log pool (S3/S8) is asserted to appear only
//! in its masked form in every parsed result.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use clawdesk_lib::application::DiagnosticsService;
use clawdesk_lib::domain::models::diagnostics::LogEvent;
use clawdesk_lib::domain::models::openclaw::UpdateState;
use clawdesk_lib::domain::ports::openclaw::OpenClawPort;
use clawdesk_lib::domain::ports::openclaw_diagnostics::OpenClawDiagnosticsPort;
use clawdesk_lib::domain::ports::process::{ProcessError, ProcessPort, ProcessRequest};
use clawdesk_lib::infrastructure::openclaw::{OpenClawAdapter, OpenClawDiagnosticsAdapter};
use clawdesk_lib::infrastructure::process::ProcessRunner;

const FAKE_OPENCLAW: &str = env!("CARGO_BIN_EXE_clawdesk-fake-openclaw");
const TIMEOUT: Duration = Duration::from_secs(10);
/// The fake `sk-` token embedded in the log pool (fake token only — S3).
const FAKE_TOKEN: &str = "fake123456789";

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("tests")
        .join("fixtures")
        .join("openclaw")
}

fn target_dir() -> PathBuf {
    std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target"))
}

fn stamp() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

struct Sandbox {
    dir: PathBuf,
    capture: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        let dir = target_dir().join("clawdesk_test_state").join(format!(
            "{tag}-{}-{}",
            std::process::id(),
            stamp()
        ));
        fs::create_dir_all(&dir).expect("create sandbox");
        let capture = dir.join("capture.jsonl");
        Sandbox { dir, capture }
    }

    fn seed(&self, state: serde_json::Value) {
        fs::write(self.dir.join("openclaw.json"), state.to_string()).expect("seed state");
    }

    fn captured(&self) -> Vec<Vec<String>> {
        let body = fs::read_to_string(&self.capture).expect("capture file exists");
        body.lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                serde_json::from_str::<Vec<String>>(line)
                    .unwrap_or_else(|err| panic!("bad capture line {line:?}: {err}"))
            })
            .collect()
    }
}

/// Temporarily sets inherited-process environment (removed on drop).
struct GlobalEnvGuard {
    keys: Vec<String>,
}

impl GlobalEnvGuard {
    fn set(pairs: &[(&str, &str)]) -> Self {
        let mut guard = GlobalEnvGuard { keys: Vec::new() };
        for (key, value) in pairs {
            std::env::set_var(key, value);
            guard.keys.push(key.to_string());
        }
        guard
    }
}

impl Drop for GlobalEnvGuard {
    fn drop(&mut self) {
        for key in &self.keys {
            std::env::remove_var(key);
        }
    }
}

/// Serializes scenario-based tests: the fake CLI receives its sandbox through
/// inherited (global) env, so two scenario tests running in parallel would
/// race on `CLAWDESK_FAKE_STATE`/`CLAWDESK_FAKE_CAPTURE`.
static SCENARIO_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Runs one serialized adapter/service-driven sub-scenario in its own
/// sandbox.
fn scenario(
    tag: &str,
    behavior: Option<&str>,
    extra_envs: &[(&str, &str)],
    run: impl FnOnce(&Sandbox),
) {
    let _lock = SCENARIO_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let sandbox = Sandbox::new(tag);
    let mut pairs: Vec<(String, String)> = vec![
        (
            "CLAWDESK_FAKE_STATE".to_string(),
            sandbox.dir.to_string_lossy().into_owned(),
        ),
        (
            "CLAWDESK_FAKE_CAPTURE".to_string(),
            sandbox.capture.to_string_lossy().into_owned(),
        ),
        (
            "CLAWDESK_FAKE_PAYLOADS".to_string(),
            fixtures_dir().to_string_lossy().into_owned(),
        ),
    ];
    if let Some(behavior) = behavior {
        pairs.push(("CLAWDESK_FAKE_BEHAVIOR".to_string(), behavior.to_string()));
    }
    for (key, value) in extra_envs {
        pairs.push(((*key).to_string(), (*value).to_string()));
    }
    let refs: Vec<(&str, &str)> = pairs
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    let _guard = GlobalEnvGuard::set(&refs);
    run(&sandbox);
}

/// Copies the fake CLI into a scratch dir under the cargo target directory
/// as `openclaw.exe`, so the real Phase 1 `OpenClawAdapter` detection has a
/// real file to find. Other tests may run the fake concurrently, and Windows
/// locks the image file of a running executable — retry briefly.
fn fake_openclaw_copy() -> PathBuf {
    let dir = target_dir().join("clawdesk-fake-cli-diagnostics");
    fs::create_dir_all(&dir).expect("create scratch dir");
    let destination = dir.join("openclaw.exe");
    let mut attempts = 0u32;
    loop {
        match fs::copy(FAKE_OPENCLAW, &destination) {
            Ok(_) => return destination,
            Err(err) => {
                attempts += 1;
                if attempts >= 100 {
                    panic!("copy fake openclaw: {err}");
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

/// The service under test: the real Phase 1 `OpenClawAdapter` (gateway
/// status — reuse) + the Phase 8 diagnostics adapter, both over the single
/// `ProcessRunner`, discovering the fake CLI as `openclaw.exe`.
fn service() -> DiagnosticsService {
    let executable = fake_openclaw_copy();
    let search_dir = executable.parent().unwrap().to_path_buf();
    let openclaw: Arc<dyn OpenClawPort> = Arc::new(OpenClawAdapter::new(
        Arc::new(ProcessRunner),
        vec![search_dir],
    ));
    let diagnostics: Arc<dyn OpenClawDiagnosticsPort> =
        Arc::new(OpenClawDiagnosticsAdapter::new(Arc::new(ProcessRunner)));
    DiagnosticsService::new(openclaw, diagnostics)
}

/// The service with an empty search dir (detection → `NotFound`, 0 CLI).
fn missing_service() -> DiagnosticsService {
    let openclaw: Arc<dyn OpenClawPort> =
        Arc::new(OpenClawAdapter::new(Arc::new(ProcessRunner), Vec::new()));
    let diagnostics: Arc<dyn OpenClawDiagnosticsPort> =
        Arc::new(OpenClawDiagnosticsAdapter::new(Arc::new(ProcessRunner)));
    DiagnosticsService::new(openclaw, diagnostics)
}

// --- gateway (Phase 1 reuse) ------------------------------------------------------

#[test]
fn gateway_status_reuses_phase1_payload() {
    scenario("diagnostics-gateway", None, &[], |sandbox| {
        let service = service();
        let status = service.gateway_status().expect("gateway status");
        assert_eq!(status.state, "running");
        assert_eq!(status.version.as_deref(), Some("2026.7.1-2"));
        assert_eq!(status.port, Some(18789));
        // The exact Phase 1 argv (0 re-implementation).
        assert_eq!(
            sandbox.captured(),
            vec![vec![
                "gateway".to_string(),
                "status".to_string(),
                "--json".to_string()
            ]]
        );
    });
}

// --- agents ------------------------------------------------------------------------

#[test]
fn agents_exact_argv_and_builtin_rows() {
    scenario("diagnostics-agents-builtin", None, &[], |sandbox| {
        let service = service();
        let rows = service.agents().expect("agents list");
        assert_eq!(rows.len(), 2, "built-in fixture has two agents");
        assert_eq!(rows[0].id, "main");
        assert!(rows[0].default, "main is the default agent");
        assert_eq!(rows[0].name.as_deref(), Some("Main Agent"));
        assert_eq!(rows[0].emoji.as_deref(), Some("🦞"));
        assert_eq!(rows[0].workspace.as_deref(), Some("~/openclaw-main"));
        assert_eq!(rows[0].bindings, Some(2));
        assert_eq!(rows[1].id, "ops");
        assert!(!rows[1].default);
        assert_eq!(rows[1].bindings, None, "optional field absent → None");
        // Exact argv, byte-for-byte.
        assert_eq!(
            sandbox.captured(),
            vec![vec![
                "agents".to_string(),
                "list".to_string(),
                "--json".to_string()
            ]]
        );
    });
}

#[test]
fn agents_unicode_rows_from_state() {
    scenario("diagnostics-agents-unicode", None, &[], |sandbox| {
        sandbox.seed(serde_json::json!({
            "agents": {
                "list": [
                    {"id": "main", "default": true, "name": "주 에이전트", "emoji": "🦞", "workspace": "~/clawdesk", "bindings": 3},
                    {"id": "ops"}
                ]
            }
        }));
        let service = service();
        let rows = service.agents().expect("agents list");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].name.as_deref(), Some("주 에이전트"));
        assert_eq!(rows[0].emoji.as_deref(), Some("🦞"));
        assert_eq!(rows[0].bindings, Some(3));
        assert_eq!(rows[1].id, "ops");
        assert!(rows[1].name.is_none());
    });
}

#[test]
fn agents_nonzero_exit_is_read_failed() {
    scenario(
        "diagnostics-agents-cli-error",
        Some("cli-error"),
        &[],
        |sandbox| {
            let service = service();
            let err = service.agents().expect_err("non-zero exit must fail");
            assert_eq!(err.code, "openclaw-agents-read-failed");
            let _ = sandbox;
        },
    );
}

#[test]
fn agents_malformed_output_is_read_failed() {
    scenario(
        "diagnostics-agents-malformed",
        Some("malformed"),
        &[],
        |sandbox| {
            let service = service();
            let err = service.agents().expect_err("malformed output must fail");
            assert_eq!(err.code, "openclaw-agents-read-failed");
            let _ = sandbox;
        },
    );
}

#[test]
fn agents_missing_executable_is_not_found() {
    scenario("diagnostics-agents-missing", None, &[], |sandbox| {
        let service = missing_service();
        let err = service.agents().expect_err("missing executable must fail");
        assert_eq!(err.code, "openclaw-not-found");
        assert!(!sandbox.capture.exists(), "0 CLI calls");
    });
}

// --- update status ------------------------------------------------------------------

#[test]
fn update_detail_updated_payload() {
    scenario("diagnostics-update-updated", None, &[], |sandbox| {
        let service = service();
        let detail = service.update_status().expect("update detail");
        assert_eq!(detail.state, UpdateState::Updated);
        assert_eq!(detail.current.as_deref(), Some("2026.7.1-2"));
        assert_eq!(detail.latest.as_deref(), Some("2026.7.1-2"));
        // The exact Phase 1 argv (same command, richer parse).
        assert_eq!(
            sandbox.captured(),
            vec![vec![
                "update".to_string(),
                "status".to_string(),
                "--json".to_string()
            ]]
        );
    });
}

#[test]
fn update_detail_available_payload() {
    scenario(
        "diagnostics-update-available",
        None,
        &[("CLAWDESK_FAKE_UPDATE", "available")],
        |sandbox| {
            let service = service();
            let detail = service.update_status().expect("update detail");
            assert_eq!(detail.state, UpdateState::UpdateAvailable);
            assert_eq!(detail.current.as_deref(), Some("2026.7.1"));
            assert_eq!(detail.latest.as_deref(), Some("2026.7.1-2"));
            let _ = sandbox;
        },
    );
}

#[test]
fn update_detail_process_failure_is_unknown() {
    scenario("diagnostics-update-fail", Some("fail"), &[], |sandbox| {
        let service = service();
        let detail = service
            .update_status()
            .expect("fail-soft: process failure is a value, not an error");
        assert_eq!(detail.state, UpdateState::Unknown);
        assert_eq!(detail.current, None);
        assert_eq!(detail.latest, None);
        let _ = sandbox;
    });
}

#[test]
fn update_detail_not_json_is_unknown() {
    scenario(
        "diagnostics-update-not-json",
        Some("not-json"),
        &[],
        |sandbox| {
            let service = service();
            let detail = service
                .update_status()
                .expect("fail-soft: malformed payload is a value, not an error");
            assert_eq!(
                detail,
                clawdesk_lib::domain::models::diagnostics::UpdateStatusDetail::unknown()
            );
            let _ = sandbox;
        },
    );
}

// --- logs ---------------------------------------------------------------------------

#[test]
fn logs_exact_argv_limit_byte_match() {
    scenario("diagnostics-logs-argv", None, &[], |sandbox| {
        let service = service();
        service.logs(50).expect("logs tail");
        // The limit is its own argv element (S2) and `--follow` is absent.
        assert_eq!(
            sandbox.captured(),
            vec![vec![
                "logs".to_string(),
                "--limit".to_string(),
                "50".to_string(),
                "--json".to_string()
            ]]
        );
    });
}

#[test]
fn logs_type_tagged_events_full_tail() {
    scenario("diagnostics-logs-full", None, &[], |sandbox| {
        let service = service();
        let result = service.logs(200).expect("logs tail");
        // meta + 12 pool lines + notice.
        assert_eq!(result.lines.len(), 14);
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
            other => panic!("expected meta first, got {other:?}"),
        }
        match &result.lines[1] {
            LogEvent::Log { level, message, .. } => {
                assert_eq!(level.as_deref(), Some("info"));
                assert!(message.contains("configured provider token"));
            }
            other => panic!("expected log, got {other:?}"),
        }
        match result.lines.last().expect("notice last") {
            LogEvent::Notice { truncated, .. } => assert_eq!(*truncated, Some(true)),
            other => panic!("expected notice last, got {other:?}"),
        }
        // The raw-type pool line survives as a Raw event.
        assert!(result
            .lines
            .iter()
            .any(|line| matches!(line, LogEvent::Raw { .. })));
        assert_eq!(result.source.as_deref(), Some("openclaw-2026-08-26.log"));
        assert!(result.truncated);
        let _ = sandbox;
    });
}

#[test]
fn logs_limit_caps_the_tail() {
    scenario("diagnostics-logs-limit", None, &[], |sandbox| {
        let service = service();
        let result = service.logs(3).expect("logs tail");
        // meta + 3 lines + notice.
        assert_eq!(result.lines.len(), 5);
        assert!(matches!(result.lines[1], LogEvent::Log { .. }));
        assert!(matches!(result.lines[3], LogEvent::Log { .. }));
        let _ = sandbox;
    });
}

#[test]
fn logs_fake_token_is_masked_end_to_end() {
    scenario("diagnostics-logs-masking", None, &[], |sandbox| {
        let service = service();
        let result = service.logs(200).expect("logs tail");
        let rendered = serde_json::to_string(&result).expect("serialize");
        assert!(
            !rendered.contains(FAKE_TOKEN),
            "raw fake token must never appear (S3/S8)"
        );
        assert!(
            rendered.contains("sk-****"),
            "the masked form must appear in the log viewer data"
        );
        let _ = sandbox;
    });
}

#[test]
fn logs_empty_stdout_is_zero_lines_success() {
    scenario("diagnostics-logs-empty", None, &[], |sandbox| {
        sandbox.seed(serde_json::json!({ "logs": { "empty": true } }));
        let service = service();
        let result = service.logs(50).expect("empty tail is a success");
        assert!(result.lines.is_empty());
        assert_eq!(result.source, None);
        assert!(!result.truncated);
        let _ = sandbox;
    });
}

#[test]
fn logs_nonzero_exit_is_read_failed() {
    scenario(
        "diagnostics-logs-cli-error",
        Some("cli-error"),
        &[],
        |sandbox| {
            let service = service();
            let err = service.logs(50).expect_err("non-zero exit must fail");
            assert_eq!(err.code, "openclaw-logs-read-failed");
            let _ = sandbox;
        },
    );
}

#[test]
fn logs_missing_executable_is_not_found() {
    scenario("diagnostics-logs-missing", None, &[], |sandbox| {
        let service = missing_service();
        let err = service.logs(50).expect_err("missing executable must fail");
        assert_eq!(err.code, "openclaw-not-found");
        assert!(!sandbox.capture.exists(), "0 CLI calls");
    });
}

#[test]
fn logs_invalid_limit_has_zero_cli_calls() {
    scenario("diagnostics-logs-limit-guard", None, &[], |sandbox| {
        let service = service();
        for limit in [0u32, 1001] {
            let err = service
                .logs(limit)
                .expect_err("out-of-range limit must fail");
            assert_eq!(err.code, "logs-limit-invalid", "{limit}");
        }
        assert!(!sandbox.capture.exists(), "0 CLI calls on invalid limit");
    });
}

#[test]
fn logs_slow_process_times_out() {
    // Runner-level timeout (the adapter's 30s contract timeout mapping to
    // `process-timeout` is covered by the adapter unit tests: the fake's
    // 3s sleep cannot exceed it).
    let sandbox = Sandbox::new("diagnostics-logs-timeout");
    sandbox.seed(serde_json::json!({}));
    let argv = vec![
        "logs".to_string(),
        "--limit".to_string(),
        "50".to_string(),
        "--json".to_string(),
    ];
    let mut request = ProcessRequest::new(FAKE_OPENCLAW, argv, Duration::from_millis(400));
    request.env.push((
        "CLAWDESK_FAKE_STATE".to_string(),
        sandbox.dir.to_string_lossy().into_owned(),
    ));
    // Own capture file: without it the fake would inherit the concurrent
    // scenario's global `CLAWDESK_FAKE_CAPTURE` and pollute its log.
    request.env.push((
        "CLAWDESK_FAKE_CAPTURE".to_string(),
        sandbox.capture.to_string_lossy().into_owned(),
    ));
    request
        .env
        .push(("CLAWDESK_FAKE_BEHAVIOR".to_string(), "sleep".to_string()));
    let err = ProcessRunner
        .run(&request)
        .expect_err("sleeping fake CLI must time out");
    match err {
        ProcessError::Timeout { executable } => {
            assert!(executable.contains("clawdesk-fake-openclaw"));
        }
        other => panic!("expected Timeout, got {other:?}"),
    }
}

// --- fake-level non-goal guards (per-request env; parallel-safe) ---------------------

#[test]
fn fake_rejects_logs_follow_and_unknown_flags() {
    let sandbox = Sandbox::new("diagnostics-non-goal");
    sandbox.seed(serde_json::json!({}));
    for argv in [
        vec!["logs", "--limit", "5", "--json", "--follow"],
        vec!["logs", "--limit", "5", "--json", "--channel", "discord"],
        vec!["logs", "--follow"],
        vec!["agents", "list"],
    ] {
        let argv: Vec<String> = argv.into_iter().map(str::to_string).collect();
        let mut request = ProcessRequest::new(FAKE_OPENCLAW, argv.clone(), TIMEOUT);
        request.env.push((
            "CLAWDESK_FAKE_STATE".to_string(),
            sandbox.dir.to_string_lossy().into_owned(),
        ));
        // Own capture file: without it the fake would inherit the concurrent
        // scenario's global `CLAWDESK_FAKE_CAPTURE` and pollute its log.
        request.env.push((
            "CLAWDESK_FAKE_CAPTURE".to_string(),
            sandbox.capture.to_string_lossy().into_owned(),
        ));
        // Pin the behavior: a concurrent scenario's global
        // `CLAWDESK_FAKE_BEHAVIOR` must not alter the rejection.
        request
            .env
            .push(("CLAWDESK_FAKE_BEHAVIOR".to_string(), "normal".to_string()));
        let output = ProcessRunner
            .run(&request)
            .expect("non-goal flag must run to completion (exit 2)");
        assert_eq!(output.exit_code, 2, "argv {argv:?} must be rejected");
    }
}
