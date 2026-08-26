//! Phase 8.1 contract tests: real `ProcessRunner` + fake `winget`/`node`
//! CLIs with a per-test state sandbox (S5: fake CLI only, no real winget,
//! no system mutation — sandboxes live under the cargo target dir).
//!
//! Adapter/service-driven scenarios run inside serialized sub-scenarios
//! because the fakes receive their sandbox via inherited process
//! environment. The fake `sk-` token in the winget failure output (S3/S8)
//! is asserted to appear only in its masked form.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use clawdesk_lib::application::NodeUpdateService;
use clawdesk_lib::domain::models::windows::NodeDetection;
use clawdesk_lib::domain::ports::node_update::NodeUpdatePort;
use clawdesk_lib::domain::ports::process::{ProcessError, ProcessPort, ProcessRequest};
use clawdesk_lib::domain::ports::windows_system::WindowsSystemPort;
use clawdesk_lib::infrastructure::process::ProcessRunner;
use clawdesk_lib::infrastructure::windows::{NodeUpdateAdapter, WindowsSystemAdapter};

const FAKE_WINGET: &str = env!("CARGO_BIN_EXE_clawdesk-fake-winget");
const FAKE_NODE: &str = env!("CARGO_BIN_EXE_clawdesk-fake-node");
const TIMEOUT: Duration = Duration::from_secs(10);
/// The fake `sk-` token embedded in the winget failure stderr (fake token
/// only — S3).
const FAKE_TOKEN: &str = "fake123456789";
/// The exact install argv the adapter must emit (byte-match contract).
const INSTALL_ARGV: [&str; 8] = [
    "install",
    "--id",
    "OpenJS.NodeJS.LTS",
    "--exact",
    "--silent",
    "--disable-interactivity",
    "--accept-source-agreements",
    "--accept-package-agreements",
];
/// An unsupported Node version for the update scenarios.
const UNSUPPORTED_NODE: &str = "18.19.0";
/// A supported Node version (for the "already supported" precondition).
const SUPPORTED_NODE: &str = "24.15.0";
/// The version the fake winget install provides.
const UPDATED_NODE: &str = "24.15.0";

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

    /// Seeds the simulated Node state (`node.json`).
    fn seed(&self, state: serde_json::Value) {
        fs::write(self.dir.join("node.json"), state.to_string()).expect("seed state");
    }

    /// The captured winget argv lines (one JSON array per line).
    fn captured_winget_argv(&self) -> Vec<Vec<String>> {
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

/// Serializes scenario-based tests: the fakes receive their sandbox through
/// inherited (global) env, so two scenario tests running in parallel would
/// race on `CLAWDESK_FAKE_STATE`/`CLAWDESK_FAKE_CAPTURE`.
static SCENARIO_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Runs one serialized service-driven sub-scenario in its own sandbox.
fn scenario(tag: &str, behavior: Option<&str>, run: impl FnOnce(&Sandbox)) {
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
    ];
    if let Some(behavior) = behavior {
        pairs.push(("CLAWDESK_FAKE_BEHAVIOR".to_string(), behavior.to_string()));
    }
    let refs: Vec<(&str, &str)> = pairs
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    let _guard = GlobalEnvGuard::set(&refs);
    run(&sandbox);
}

/// Copies the fake winget into a scratch dir under the cargo target
/// directory (Windows locks the image of a running executable — retry).
fn fake_winget_copy() -> PathBuf {
    let dir = target_dir().join("clawdesk-fake-winget-node-update");
    fs::create_dir_all(&dir).expect("create scratch dir");
    let destination = dir.join("winget.exe");
    copy_with_retry(FAKE_WINGET, &destination)
}

/// Copies the fake node into a scratch dir under the cargo target
/// directory (used both as the precondition probe target and as the MSI /
/// fallback candidate — the reported version always comes from state).
fn fake_node_copy() -> PathBuf {
    let dir = target_dir().join("clawdesk-fake-node-node-update");
    fs::create_dir_all(&dir).expect("create scratch dir");
    let destination = dir.join("node.exe");
    copy_with_retry(FAKE_NODE, &destination)
}

fn copy_with_retry(source: &str, destination: &Path) -> PathBuf {
    let mut attempts = 0u32;
    loop {
        match fs::copy(source, destination) {
            Ok(_) => return destination.to_path_buf(),
            Err(err) => {
                attempts += 1;
                if attempts >= 100 {
                    panic!("copy fake: {err}");
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

/// The service under test: real `WindowsSystemAdapter` (precondition
/// `detect_node` via the fake node) + real `NodeUpdateAdapter` (fake
/// winget + MSI/fallback candidates), both over the single `ProcessRunner`.
fn service(
    winget: PathBuf,
    msi: PathBuf,
    fallback: Option<PathBuf>,
    node_probe: PathBuf,
) -> NodeUpdateService {
    let process: Arc<dyn ProcessPort> = Arc::new(ProcessRunner);
    let windows: Arc<dyn WindowsSystemPort> = Arc::new(WindowsSystemAdapter::with_node_executable(
        Arc::clone(&process),
        node_probe,
    ));
    let node_update: Arc<dyn NodeUpdatePort> = Arc::new(NodeUpdateAdapter::with_paths(
        Arc::clone(&process),
        winget,
        msi,
        fallback,
    ));
    NodeUpdateService::new(windows, node_update)
}

const MISSING: &str = "C:\\clawdesk-missing\\binary.exe";

// --- preconditions (fail-closed, 0 winget) ------------------------------------------

#[test]
fn precondition_supported_has_zero_winget() {
    scenario("node-update-pre-supported", None, |sandbox| {
        sandbox.seed(serde_json::json!({ "node": { "version": SUPPORTED_NODE } }));
        let node = fake_node_copy();
        let svc = service(fake_winget_copy(), PathBuf::from(MISSING), None, node);
        let err = svc.update_node().expect_err("already supported must fail");
        assert_eq!(err.code, "node-update-not-needed");
        assert!(err.message.contains(SUPPORTED_NODE));
        assert!(!sandbox.capture.exists(), "0 winget calls");
    });
}

#[test]
fn precondition_not_found_has_zero_winget() {
    scenario("node-update-pre-missing", None, |sandbox| {
        sandbox.seed(serde_json::json!({ "node": { "version": UNSUPPORTED_NODE } }));
        let svc = service(
            fake_winget_copy(),
            PathBuf::from(MISSING),
            None,
            PathBuf::from(MISSING),
        );
        let err = svc.update_node().expect_err("missing node must fail");
        assert_eq!(err.code, "node-not-found");
        assert!(!sandbox.capture.exists(), "0 winget calls");
    });
}

// --- update scenarios -----------------------------------------------------------------

#[test]
fn unsupported_update_success_msi_first() {
    scenario("node-update-success", None, |sandbox| {
        sandbox.seed(serde_json::json!({ "node": { "version": UNSUPPORTED_NODE } }));
        let node = fake_node_copy();
        // MSI candidate = the fake node (fresh after install); no PATH
        // fallback needed (the MSI probe must suffice).
        let svc = service(fake_winget_copy(), node.clone(), None, node.clone());
        let detected = svc.update_node().expect("update must succeed");
        assert_eq!(
            detected,
            NodeDetection::Found {
                version: UPDATED_NODE.to_string()
            }
        );
        // probe (`--version`) + the exact install argv, byte-for-byte.
        let argv = sandbox.captured_winget_argv();
        assert_eq!(argv.len(), 2, "probe + install only");
        assert_eq!(argv[0], vec!["--version".to_string()]);
        assert_eq!(
            argv[1],
            INSTALL_ARGV
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
        );
    });
}

#[test]
fn unsupported_update_still_unsupported_is_failed() {
    scenario("node-update-noop", Some("noop"), |sandbox| {
        sandbox.seed(serde_json::json!({ "node": { "version": UNSUPPORTED_NODE } }));
        let node = fake_node_copy();
        let svc = service(fake_winget_copy(), node.clone(), None, node.clone());
        let err = svc
            .update_node()
            .expect_err("still unsupported after the update must fail");
        assert_eq!(err.code, "node-update-failed");
        assert!(err.message.contains(UNSUPPORTED_NODE));
        // The install did run (exit 0) — verification must not trust it.
        let argv = sandbox.captured_winget_argv();
        assert_eq!(argv.len(), 2, "probe + install ran");
    });
}

#[test]
fn install_nonzero_is_failed_and_masked() {
    scenario("node-update-fail", Some("fail"), |sandbox| {
        sandbox.seed(serde_json::json!({ "node": { "version": UNSUPPORTED_NODE } }));
        let node = fake_node_copy();
        let svc = service(fake_winget_copy(), node.clone(), None, node.clone());
        let err = svc.update_node().expect_err("non-zero install must fail");
        assert_eq!(err.code, "node-update-failed");
        // S3/S8: the fake token must never leak in its raw form.
        assert!(!err.message.contains(FAKE_TOKEN), "raw token leaked");
        assert!(err.message.contains("sk-****"), "masked form expected");
        let _ = sandbox;
    });
}

#[test]
fn install_missing_winget_is_not_found() {
    scenario("node-update-no-winget", None, |sandbox| {
        sandbox.seed(serde_json::json!({ "node": { "version": UNSUPPORTED_NODE } }));
        let node = fake_node_copy();
        let svc = service(PathBuf::from(MISSING), node.clone(), None, node);
        let err = svc.update_node().expect_err("missing winget must fail");
        assert_eq!(err.code, "winget-not-found");
        assert!(!sandbox.capture.exists(), "0 winget calls");
    });
}

#[test]
fn path_fallback_used_when_msi_absent() {
    scenario("node-update-fallback", None, |sandbox| {
        sandbox.seed(serde_json::json!({ "node": { "version": UNSUPPORTED_NODE } }));
        let node = fake_node_copy();
        // MSI candidate absent → the injected PATH-style fallback probe
        // must deliver the post-update version.
        let svc = service(
            fake_winget_copy(),
            PathBuf::from(MISSING),
            Some(node.clone()),
            node,
        );
        let detected = svc.update_node().expect("fallback probe must succeed");
        assert_eq!(
            detected,
            NodeDetection::Found {
                version: UPDATED_NODE.to_string()
            }
        );
    });
}

#[test]
fn all_candidates_absent_is_update_failed() {
    scenario("node-update-no-candidates", None, |sandbox| {
        sandbox.seed(serde_json::json!({ "node": { "version": UNSUPPORTED_NODE } }));
        let node = fake_node_copy();
        let svc = service(
            fake_winget_copy(),
            PathBuf::from(MISSING),
            Some(PathBuf::from(MISSING)),
            node,
        );
        let err = svc
            .update_node()
            .expect_err("node undetectable after the update must fail");
        assert_eq!(err.code, "node-update-failed");
    });
}

// --- fake-level non-goal guards + runner timeout (per-request env; parallel-safe) ----

#[test]
fn winget_rejects_unknown_flags() {
    let sandbox = Sandbox::new("node-update-non-goal");
    sandbox.seed(serde_json::json!({ "node": { "version": UNSUPPORTED_NODE } }));
    // The ORIGINAL fake (not the scenario copy): the other fake-level test
    // may run this image concurrently — the copy would hit the Windows
    // image lock (same convention as the diagnostics contract).
    let winget = PathBuf::from(FAKE_WINGET);
    for argv in [
        // wrong package id
        vec!["install", "--id", "OpenJS.NodeJS"],
        // missing required flags
        vec!["install", "--id", "OpenJS.NodeJS.LTS"],
        // upgrade is not the sanctioned invocation
        vec!["upgrade", "--id", "OpenJS.NodeJS.LTS"],
        // unknown subcommand
        vec!["search", "node"],
    ] {
        let argv: Vec<String> = argv.into_iter().map(str::to_string).collect();
        let mut request = ProcessRequest::new(winget.clone(), argv.clone(), TIMEOUT);
        request.env.push((
            "CLAWDESK_FAKE_STATE".to_string(),
            sandbox.dir.to_string_lossy().into_owned(),
        ));
        // Own capture file: without it the fake would inherit the
        // concurrent scenario's global `CLAWDESK_FAKE_CAPTURE`.
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
            .expect("bad argv must run to completion (exit 2)");
        assert_eq!(output.exit_code, 2, "argv {argv:?} must be rejected");
    }
}

#[test]
fn winget_sleep_times_out_at_runner_level() {
    // Runner-level timeout (the adapter's 900s contract budget is asserted
    // in the unit tests; the fake's 3s sleep cannot exceed it here).
    let sandbox = Sandbox::new("node-update-timeout");
    sandbox.seed(serde_json::json!({ "node": { "version": UNSUPPORTED_NODE } }));
    // The ORIGINAL fake (see `winget_rejects_unknown_flags`).
    let winget = PathBuf::from(FAKE_WINGET);
    let argv = vec![
        "install".to_string(),
        "--id".to_string(),
        "OpenJS.NodeJS.LTS".to_string(),
        "--exact".to_string(),
        "--silent".to_string(),
        "--disable-interactivity".to_string(),
        "--accept-source-agreements".to_string(),
        "--accept-package-agreements".to_string(),
    ];
    let mut request = ProcessRequest::new(winget, argv, Duration::from_millis(400));
    request.env.push((
        "CLAWDESK_FAKE_STATE".to_string(),
        sandbox.dir.to_string_lossy().into_owned(),
    ));
    request.env.push((
        "CLAWDESK_FAKE_CAPTURE".to_string(),
        sandbox.capture.to_string_lossy().into_owned(),
    ));
    request
        .env
        .push(("CLAWDESK_FAKE_BEHAVIOR".to_string(), "sleep".to_string()));
    let err = ProcessRunner
        .run(&request)
        .expect_err("sleeping fake winget must time out");
    match err {
        ProcessError::Timeout { executable } => {
            assert!(executable.contains("clawdesk-fake-winget"));
        }
        other => panic!("expected Timeout, got {other:?}"),
    }
}
