//! Phase 1 contract tests: real `ProcessRunner` + fake `openclaw` CLI.
//!
//! Per S5, only the fake CLI fixture is used — no real OpenClaw binary.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use clawdesk_lib::application::EnvironmentService;
use clawdesk_lib::domain::models::openclaw::UpdateState;
use clawdesk_lib::domain::models::windows::Architecture;
use clawdesk_lib::domain::models::{ExecutableDetection, NodeDetection};
use clawdesk_lib::domain::ports::process::{
    ProcessError, ProcessOutput, ProcessPort, ProcessRequest,
};
use clawdesk_lib::domain::ports::{OpenClawPort, WindowsSystemPort};
use clawdesk_lib::infrastructure::openclaw::adapter::OpenClawAdapter;
use clawdesk_lib::infrastructure::openclaw::parse::{
    parse_gateway_json, parse_update_json, parse_version_output,
};
use clawdesk_lib::infrastructure::process::ProcessRunner;
use clawdesk_lib::infrastructure::windows::WindowsSystemAdapter;

const FAKE_OPENCLAW: &str = env!("CARGO_BIN_EXE_clawdesk-fake-openclaw");

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("tests")
        .join("fixtures")
        .join("openclaw")
}

fn run_fake(
    argv: &[&str],
    timeout: Duration,
    extra_envs: &[(&str, &str)],
) -> Result<ProcessOutput, ProcessError> {
    let argv: Vec<String> = argv.iter().map(|arg| arg.to_string()).collect();
    let mut request = ProcessRequest::new(FAKE_OPENCLAW, argv, timeout);
    let payloads = fixtures_dir().to_string_lossy().to_string();
    request
        .env
        .push(("CLAWDESK_FAKE_PAYLOADS".to_string(), payloads));
    for (key, value) in extra_envs {
        request.env.push((key.to_string(), value.to_string()));
    }
    let runner = ProcessRunner;
    runner.run(&request)
}

/// Makes the fake payload dir visible to child processes launched without
/// per-request env (adapter-level tests).
fn ensure_fake_payloads_env() {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        if std::env::var("CLAWDESK_FAKE_PAYLOADS").is_err() {
            std::env::set_var("CLAWDESK_FAKE_PAYLOADS", fixtures_dir());
        }
    });
}

/// Copies the fake CLI into a scratch dir under the cargo target directory as
/// `openclaw.exe`, so adapter detection has a real file to find.
fn fake_openclaw_copy() -> PathBuf {
    let target = std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target"));
    let dir = target.join("clawdesk-fake-cli");
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    let destination = dir.join("openclaw.exe");
    // Other tests run the fake CLI concurrently as a child process, and
    // Windows locks the image file of a running executable. Retry briefly.
    let mut attempts = 0u32;
    loop {
        match std::fs::copy(FAKE_OPENCLAW, &destination) {
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

const TIMEOUT: Duration = Duration::from_secs(10);

// --- happy path: fake CLI version / gateway / update parsing -------------

#[test]
fn fake_cli_version_happy_path() {
    let output = run_fake(&["--version"], TIMEOUT, &[]).expect("fake --version should run");
    assert_eq!(output.exit_code, 0);
    let version = parse_version_output(&output.stdout).expect("version should parse");
    assert_eq!(version.raw, "2026.7.1-2");
}

#[test]
fn fake_cli_gateway_happy_path() {
    let output = run_fake(&["gateway", "status", "--json"], TIMEOUT, &[])
        .expect("fake gateway status should run");
    assert_eq!(output.exit_code, 0);
    let status = parse_gateway_json(&output.stdout).expect("gateway payload should parse");
    assert_eq!(status.state, "running");
    assert_eq!(status.version.as_deref(), Some("2026.7.1-2"));
    assert_eq!(status.port, Some(18789));
}

#[test]
fn fake_cli_gateway_stopped_exit_1_still_parses() {
    // The real CLI exits 1 when no gateway is reachable (valid payload).
    let output = run_fake(
        &["gateway", "status", "--json"],
        TIMEOUT,
        &[("CLAWDESK_FAKE_BEHAVIOR", "stopped")],
    )
    .expect("fake gateway status should run");
    assert_eq!(output.exit_code, 1);
    let status = parse_gateway_json(&output.stdout).expect("stopped payload should parse");
    assert_eq!(status.state, "stopped");
    assert_eq!(status.version, None);
    assert_eq!(status.port, Some(18789));
}

#[test]
fn fake_cli_gateway_cli_error_envelope_is_parse_error() {
    let output = run_fake(
        &["gateway", "status", "--json"],
        TIMEOUT,
        &[("CLAWDESK_FAKE_BEHAVIOR", "cli-error")],
    )
    .expect("fake gateway status should run");
    assert_eq!(output.exit_code, 1);
    let err = parse_gateway_json(&output.stdout).expect_err("cli error envelope must not parse");
    assert_eq!(err.code, "openclaw-gateway-parse");
}

#[test]
fn fake_cli_update_updated_happy_path() {
    let output = run_fake(&["update", "status", "--json"], TIMEOUT, &[])
        .expect("fake update status should run");
    assert_eq!(output.exit_code, 0);
    let state = parse_update_json(&output.stdout);
    assert_eq!(state, UpdateState::Updated);
}

#[test]
fn fake_cli_update_available_happy_path() {
    let output = run_fake(
        &["update", "status", "--json"],
        TIMEOUT,
        &[("CLAWDESK_FAKE_UPDATE", "available")],
    )
    .expect("fake update status should run");
    assert_eq!(output.exit_code, 0);
    let state = parse_update_json(&output.stdout);
    assert_eq!(state, UpdateState::UpdateAvailable);
}

// --- failure modes ---------------------------------------------------------

#[test]
fn fake_cli_malformed_version_output_is_parse_error() {
    let output = run_fake(
        &["--version"],
        TIMEOUT,
        &[("CLAWDESK_FAKE_BEHAVIOR", "malformed")],
    )
    .expect("fake --version should run");
    assert_eq!(output.exit_code, 0);
    let err = parse_version_output(&output.stdout).expect_err("malformed version must not parse");
    assert_eq!(err.code, "openclaw-version-parse");
}

#[test]
fn fake_cli_not_json_gateway_output_is_parse_error() {
    let output = run_fake(
        &["gateway", "status", "--json"],
        TIMEOUT,
        &[("CLAWDESK_FAKE_BEHAVIOR", "not-json")],
    )
    .expect("fake gateway status should run");
    assert_eq!(output.exit_code, 0);
    let err = parse_gateway_json(&output.stdout).expect_err("non-json must not parse");
    assert_eq!(err.code, "openclaw-gateway-parse");
}

#[test]
fn run_missing_executable_is_structured_not_found() {
    let request = ProcessRequest::new(
        PathBuf::from(r"C:\clawdesk-missing\definitely-not-here.exe"),
        vec!["--version".to_string()],
        TIMEOUT,
    );
    let runner = ProcessRunner;
    let err = runner
        .run(&request)
        .expect_err("missing executable must fail");
    match err {
        ProcessError::NotFound { executable } => {
            assert!(executable.contains("definitely-not-here"));
        }
        other => panic!("expected NotFound, got {other:?}"),
    }
}

#[test]
fn run_slow_process_times_out() {
    let output = run_fake(
        &["--version"],
        Duration::from_millis(400),
        &[("CLAWDESK_FAKE_BEHAVIOR", "sleep")],
    )
    .expect_err("sleeping fake CLI must time out");
    match output {
        ProcessError::Timeout { executable } => {
            assert!(executable.contains("clawdesk-fake-openclaw"));
        }
        other => panic!("expected Timeout, got {other:?}"),
    }
}

#[test]
fn run_non_zero_exit_reports_code_and_masked_stderr() {
    let output = run_fake(
        &["--version"],
        TIMEOUT,
        &[("CLAWDESK_FAKE_BEHAVIOR", "fail")],
    )
    .expect("failing fake CLI still runs to completion");
    assert_eq!(output.exit_code, 3);
    assert!(
        !output.stderr.contains("sk-fake123456789"),
        "secret must not appear in stderr"
    );
    assert!(output.stderr.contains("sk-****"), "stderr should be masked");
}

// --- OpenClawAdapter over the fake CLI -------------------------------------

#[test]
fn adapter_detects_openclaw_in_search_dir() {
    let executable = fake_openclaw_copy();
    let adapter = OpenClawAdapter::new(
        Arc::new(ProcessRunner),
        vec![executable.parent().unwrap().to_path_buf()],
    );
    assert_eq!(
        adapter.detect_executable(),
        ExecutableDetection::Found {
            path: executable.clone()
        }
    );
}

#[test]
fn adapter_reports_not_found_without_candidates() {
    let target = std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target"));
    let empty_dir = target.join("clawdesk-fake-cli-empty");
    std::fs::create_dir_all(&empty_dir).expect("create empty scratch dir");
    let adapter = OpenClawAdapter::new(Arc::new(ProcessRunner), vec![empty_dir]);
    assert_eq!(adapter.detect_executable(), ExecutableDetection::NotFound);
}

#[test]
fn adapter_end_to_end_version_gateway_update() {
    ensure_fake_payloads_env();
    let executable = fake_openclaw_copy();
    let adapter = OpenClawAdapter::new(
        Arc::new(ProcessRunner),
        vec![executable.parent().unwrap().to_path_buf()],
    );

    let version = adapter
        .version(&executable)
        .expect("version should succeed");
    assert_eq!(version.raw, "2026.7.1-2");

    let gateway = adapter
        .gateway_status(&executable)
        .expect("gateway status should succeed");
    assert_eq!(gateway.state, "running");
    assert_eq!(gateway.version.as_deref(), Some("2026.7.1-2"));
    assert_eq!(gateway.port, Some(18789));

    let update = adapter
        .update_state(&executable)
        .expect("update state should succeed");
    assert_eq!(update, UpdateState::Updated);
}

#[test]
fn adapter_update_state_is_unknown_when_executable_missing() {
    let adapter = OpenClawAdapter::new(Arc::new(ProcessRunner), Vec::new());
    let state = adapter
        .update_state(Path::new(r"C:\clawdesk-missing\openclaw.exe"))
        .expect("process failure should resolve to Unknown, not an error");
    assert_eq!(state, UpdateState::Unknown);
}

// --- real Windows machine (detection only, no mutation) ---------------------

#[cfg(windows)]
mod real_machine {
    use super::*;

    #[test]
    fn os_version_is_windows_10_or_11() {
        let adapter = WindowsSystemAdapter::new(Arc::new(ProcessRunner));
        let version = adapter
            .os_version()
            .expect("os version should be detectable");
        assert!(
            version.build >= 10240,
            "expected Windows 10/11 build, got {}",
            version.build
        );
        assert!(
            matches!(version.major_version, 10 | 11),
            "expected major 10 or 11, got {}",
            version.major_version
        );
    }

    #[test]
    fn architecture_is_x64() {
        let adapter = WindowsSystemAdapter::new(Arc::new(ProcessRunner));
        let architecture = adapter.architecture().expect("this machine must be x64");
        assert_eq!(architecture, Architecture::X64);
    }

    #[test]
    fn node_is_detected() {
        let adapter = WindowsSystemAdapter::new(Arc::new(ProcessRunner));
        let node = adapter
            .detect_node()
            .expect("node detection should not error");
        match node {
            NodeDetection::Found { version } => assert!(!version.is_empty()),
            other => panic!("expected node to be present, got {other:?}"),
        }
    }

    #[test]
    fn environment_service_production_detection_runs() {
        let service = EnvironmentService::production();
        let report = service
            .detect_environment()
            .expect("environment detection should succeed on this machine");
        assert_eq!(report.architecture, Architecture::X64);
        assert!(report.windows_version.build >= 10240);
    }
}
