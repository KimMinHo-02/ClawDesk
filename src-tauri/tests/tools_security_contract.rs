//! Phase 5 contract tests: real `ProcessRunner` + fake `openclaw` CLI with a
//! per-test state sandbox (S5: fake CLI only, no real OpenClaw, no system
//! mutation — sandboxes live under the cargo target dir).
//!
//! Adapter/service-driven scenarios run inside serialized sub-scenarios
//! because the fake receives its sandbox via inherited process environment.
//! The audit-timeout mapping (`process-timeout`) is covered by the adapter
//! unit tests (scripted `ProcessError::Timeout`): the fake's `sleep`
//! behavior (3s) cannot exceed the 60s audit contract timeout.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use clawdesk_lib::application::{SecurityProfileService, ToolPolicyService};
use clawdesk_lib::domain::models::openclaw::{GatewayStatus, OpenClawVersion, UpdateState};
use clawdesk_lib::domain::models::{ExecutableDetection, SecurityProfile, ToolPolicy};
use clawdesk_lib::domain::ports::openclaw::OpenClawPort;
use clawdesk_lib::domain::ports::process::{
    ProcessError, ProcessOutput, ProcessPort, ProcessRequest,
};
use clawdesk_lib::error::AppError;
use clawdesk_lib::infrastructure::openclaw::{OpenClawConfigAdapter, OpenClawSecurityAdapter};
use clawdesk_lib::infrastructure::process::ProcessRunner;
use clawdesk_lib::infrastructure::security::SecurityProfileStore;

const FAKE_OPENCLAW: &str = env!("CARGO_BIN_EXE_clawdesk-fake-openclaw");
const TIMEOUT: Duration = Duration::from_secs(10);

/// Flags the cold audit must never carry (non-goals / no credentials).
const AUDIT_FORBIDDEN: [&str; 4] = ["--deep", "--fix", "--token", "--password"];

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

    fn state(&self) -> serde_json::Value {
        fs::read_to_string(self.dir.join("openclaw.json"))
            .expect("state file exists")
            .parse()
            .expect("state is json")
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

    /// Asserts that no captured invocation carries a forbidden audit flag.
    fn assert_no_forbidden_audit_flags(&self) {
        for line in self.captured() {
            for arg in &line {
                assert!(
                    !AUDIT_FORBIDDEN.contains(&arg.as_str()),
                    "forbidden audit flag {arg} in {line:?}"
                );
            }
        }
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
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let _guard = GlobalEnvGuard::set(&refs);
    run(&sandbox);
}

/// OpenClaw port fake: detection resolves to the fake CLI binary.
struct DetectsFakeCli;

impl OpenClawPort for DetectsFakeCli {
    fn detect_executable(&self) -> ExecutableDetection {
        ExecutableDetection::Found {
            path: PathBuf::from(FAKE_OPENCLAW),
        }
    }
    fn version(&self, _exe: &Path) -> Result<OpenClawVersion, AppError> {
        unimplemented!()
    }
    fn version_from_entry(&self, _node: &Path, _entry: &Path) -> Result<OpenClawVersion, AppError> {
        unimplemented!()
    }
    fn gateway_status(&self, _exe: &Path) -> Result<GatewayStatus, AppError> {
        unimplemented!()
    }
    fn update_state(&self, _exe: &Path) -> Result<UpdateState, AppError> {
        unimplemented!()
    }
}

/// OpenClaw port fake: no executable (the stable not-found path).
struct NoOpenClaw;

impl OpenClawPort for NoOpenClaw {
    fn detect_executable(&self) -> ExecutableDetection {
        ExecutableDetection::NotFound
    }
    fn version(&self, _exe: &Path) -> Result<OpenClawVersion, AppError> {
        unimplemented!()
    }
    fn version_from_entry(&self, _node: &Path, _entry: &Path) -> Result<OpenClawVersion, AppError> {
        unimplemented!()
    }
    fn gateway_status(&self, _exe: &Path) -> Result<GatewayStatus, AppError> {
        unimplemented!()
    }
    fn update_state(&self, _exe: &Path) -> Result<UpdateState, AppError> {
        unimplemented!()
    }
}

fn tools_service() -> ToolPolicyService {
    ToolPolicyService::new(
        Arc::new(DetectsFakeCli),
        Arc::new(OpenClawConfigAdapter::new(Arc::new(ProcessRunner))),
    )
}

fn tools_service_no_openclaw() -> ToolPolicyService {
    ToolPolicyService::new(
        Arc::new(NoOpenClaw),
        Arc::new(OpenClawConfigAdapter::new(Arc::new(ProcessRunner))),
    )
}

/// The profile store lives in the sandbox (ClawDesk-owned path injection).
fn security_service(sandbox: &Sandbox) -> SecurityProfileService {
    SecurityProfileService::new(
        Arc::new(DetectsFakeCli),
        Arc::new(OpenClawSecurityAdapter::new(Arc::new(ProcessRunner))),
        Arc::new(OpenClawConfigAdapter::new(Arc::new(ProcessRunner))),
        Arc::new(SecurityProfileStore::new(
            sandbox.dir.join("security-profiles.json"),
        )),
    )
}

fn user_profile(id: &str, name: &str) -> SecurityProfile {
    SecurityProfile {
        id: id.into(),
        name: name.into(),
        base_profile: "messaging".into(),
        allow: Vec::new(),
        deny: vec!["group:automation".into()],
        exec_mode: "ask".into(),
    }
}

// --- fake-level behavior (per-request env; parallel-safe) -------------------------

#[test]
fn security_audit_emits_findings_from_state() {
    let sandbox = Sandbox::new("fake-audit-findings");
    sandbox.seed(serde_json::json!({
        "securityAudit": {
            "findings": [
                {"checkId": "gateway.exposure.open", "severity": "critical",
                 "title": "Gateway is reachable", "detail": "port 18789"},
                {"severity": "warn", "title": "row without checkId must be dropped"}
            ],
            "suppressedFindings": [{"checkId": "s1"}, {"checkId": "s2"}, {"checkId": "s3"}]
        }
    }));
    let output =
        run_fake(&["security", "audit", "--json"], &sandbox, &[]).expect("audit should run");
    assert_eq!(output.exit_code, 0);
    let value: serde_json::Value =
        serde_json::from_str(&output.stdout).expect("payload should parse");
    assert_eq!(value["ok"], true);
    let findings = value["findings"].as_array().expect("findings array");
    // Derived (tools.exec.mode unset → full warning) + one valid passthrough
    // row; the checkId-less passthrough row is kept by the fake (the
    // ClawDesk parser is the one that drops it — verified in unit tests and
    // in the adapter scenario below).
    assert_eq!(findings.len(), 3);
    assert_eq!(
        findings[0]["checkId"],
        "tools.exec.security_full_configured"
    );
    assert_eq!(findings[0]["severity"], "warn");
    assert_eq!(findings[1]["checkId"], "gateway.exposure.open");
    assert_eq!(findings[1]["severity"], "critical");
    assert_eq!(value["summary"]["total"], 3);
    assert_eq!(value["summary"]["critical"], 1);
    assert_eq!(value["summary"]["warn"], 2);
    assert_eq!(
        value["suppressedFindings"]
            .as_array()
            .expect("suppressed array")
            .len(),
        3
    );
    let captured = sandbox.captured();
    assert_eq!(captured, vec![vec!["security", "audit", "--json"]]);
    // Required: the audit argv must never carry the forbidden flags.
    for forbidden in AUDIT_FORBIDDEN {
        assert!(!captured[0].contains(&forbidden.to_string()), "{forbidden}");
    }
}

#[test]
fn security_audit_no_derived_finding_when_exec_restricted() {
    let sandbox = Sandbox::new("fake-audit-restricted");
    sandbox.seed(serde_json::json!({
        "tools": { "exec": { "mode": "deny" } }
    }));
    let output =
        run_fake(&["security", "audit", "--json"], &sandbox, &[]).expect("audit should run");
    let value: serde_json::Value = serde_json::from_str(&output.stdout).expect("payload");
    let findings = value["findings"].as_array().expect("findings array");
    assert!(
        !findings
            .iter()
            .any(|row| row["checkId"] == "tools.exec.security_full_configured"),
        "restricted exec mode must not raise the full-mode warning"
    );
}

#[test]
fn security_audit_rejects_forbidden_flags() {
    let sandbox = Sandbox::new("fake-audit-forbidden");
    sandbox.seed(serde_json::json!({}));
    for flag in AUDIT_FORBIDDEN {
        let output = run_fake(&["security", "audit", "--json", flag], &sandbox, &[])
            .expect("invocation should run");
        assert_eq!(output.exit_code, 2, "{flag} must be rejected");
        assert!(output.stderr.contains("rejects"), "{flag}");
    }
}

#[test]
fn security_audit_failure_output_is_masked() {
    let sandbox = Sandbox::new("fake-audit-masking");
    sandbox.seed(serde_json::json!({}));
    let output = run_fake(
        &["security", "audit", "--json"],
        &sandbox,
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

// --- adapter / service scenarios (serialized sub-scenarios) -----------------------

#[test]
fn get_tool_policy_over_fake_cli() {
    // a) Full shape: exact argv + every field parsed.
    scenario("tool-policy-read-full", None, |sandbox| {
        sandbox.seed(serde_json::json!({
            "tools": {
                "profile": "coding",
                "allow": ["web_search", "image*"],
                "deny": ["group:automation"],
                "exec": {"mode": "ask"},
                "elevated": {"enabled": true},
                "fs": {"workspaceOnly": true}
            }
        }));
        let policy = tools_service().get_tool_policy().expect("read policy");
        assert_eq!(policy.profile.as_deref(), Some("coding"));
        assert_eq!(policy.allow, vec!["web_search", "image*"]);
        assert_eq!(policy.deny, vec!["group:automation"]);
        assert_eq!(policy.exec_mode.as_deref(), Some("ask"));
        assert_eq!(policy.elevated_enabled, Some(true));
        assert_eq!(policy.fs_workspace_only, Some(true));
        assert_eq!(
            sandbox.captured(),
            vec![vec!["config", "get", "tools", "--json"]]
        );
    });

    // b) Missing tools section: fail-soft empty policy (null/empty).
    scenario("tool-policy-read-missing", None, |sandbox| {
        sandbox.seed(serde_json::json!({}));
        let policy = tools_service().get_tool_policy().expect("read policy");
        assert_eq!(policy, ToolPolicy::default());
        assert_eq!(
            sandbox.captured(),
            vec![vec!["config", "get", "tools", "--json"]]
        );
    });
}

#[test]
fn set_tool_profile_over_fake_cli() {
    // a) Happy path: exact dry-run + commit argv, state update, re-read.
    scenario("tool-profile-set", None, |sandbox| {
        sandbox.seed(serde_json::json!({}));
        let service = tools_service();
        service.set_tool_profile("coding").expect("set profile");
        let lines = sandbox.captured();
        assert_eq!(lines.len(), 2, "dry-run + commit");
        assert_eq!(
            lines[0],
            vec![
                "config",
                "set",
                "tools.profile",
                "\"coding\"",
                "--strict-json",
                "--dry-run",
                "--json"
            ]
        );
        assert_eq!(
            lines[1],
            vec![
                "config",
                "set",
                "tools.profile",
                "\"coding\"",
                "--strict-json"
            ]
        );
        let state = sandbox.state();
        assert_eq!(state["tools"]["profile"], "coding");
        // Re-read (what the UI does after the response) shows the new value.
        let policy = service.get_tool_policy().expect("re-read");
        assert_eq!(policy.profile.as_deref(), Some("coding"));
    });

    // b) Invalid enum: tool-profile-invalid, CLI calls 0 times.
    scenario("tool-profile-invalid", None, |sandbox| {
        sandbox.seed(serde_json::json!({}));
        let service = tools_service();
        for profile in ["", "Coding", "default", "coding "] {
            let err = service
                .set_tool_profile(profile)
                .expect_err("must be rejected");
            assert_eq!(err.code, "tool-profile-invalid", "{profile:?}");
        }
        assert!(!sandbox.capture.exists(), "no CLI call may be captured");
        let state = sandbox.state();
        assert!(state.get("tools").is_none(), "state must be unchanged");
    });
}

#[test]
fn set_tool_allow_deny_over_fake_cli() {
    // a) allow: exact argv with --replace, JSON array as a single argv
    //    element, state update.
    scenario("tool-allow-set", None, |sandbox| {
        sandbox.seed(serde_json::json!({}));
        let service = tools_service();
        service
            .set_tool_allow(&["web_search".into(), "image*".into()])
            .expect("set allow");
        let lines = sandbox.captured();
        assert_eq!(lines.len(), 2, "dry-run + commit");
        assert_eq!(
            lines[0],
            vec![
                "config",
                "set",
                "tools.allow",
                r#"["web_search","image*"]"#,
                "--strict-json",
                "--replace",
                "--dry-run",
                "--json"
            ]
        );
        assert_eq!(
            lines[1],
            vec![
                "config",
                "set",
                "tools.allow",
                r#"["web_search","image*"]"#,
                "--strict-json",
                "--replace"
            ]
        );
        let state = sandbox.state();
        assert_eq!(
            state["tools"]["allow"],
            serde_json::json!(["web_search", "image*"])
        );
        let policy = service.get_tool_policy().expect("re-read");
        assert_eq!(policy.allow, vec!["web_search", "image*"]);
    });

    // b) deny: same flow.
    scenario("tool-deny-set", None, |sandbox| {
        sandbox.seed(serde_json::json!({}));
        let service = tools_service();
        service
            .set_tool_deny(&["group:automation".into()])
            .expect("set deny");
        let lines = sandbox.captured();
        assert_eq!(
            lines[1],
            vec![
                "config",
                "set",
                "tools.deny",
                r#"["group:automation"]"#,
                "--strict-json",
                "--replace"
            ]
        );
        let state = sandbox.state();
        assert_eq!(
            state["tools"]["deny"],
            serde_json::json!(["group:automation"])
        );
    });

    // c) Empty arrays are valid (whole-array replace to []).
    scenario("tool-allow-empty", None, |sandbox| {
        sandbox.seed(serde_json::json!({"tools": {"allow": ["old"]}}));
        let service = tools_service();
        service.set_tool_allow(&[]).expect("set empty allow");
        let state = sandbox.state();
        assert_eq!(
            state["tools"]["allow"],
            serde_json::json!([]),
            "replace to []"
        );
    });

    // d) Invalid entries: tool-entry-invalid, CLI calls 0 times.
    scenario("tool-entry-invalid", None, |sandbox| {
        sandbox.seed(serde_json::json!({}));
        let service = tools_service();
        for entries in [
            vec!["../evil".to_string()],
            vec!["a/b".to_string()],
            vec!["a b".to_string()],
            vec![String::new()],
            vec!["x".repeat(129)],
            vec!["group:".to_string()],
        ] {
            let err = service
                .set_tool_allow(&entries)
                .expect_err("must be rejected");
            assert_eq!(err.code, "tool-entry-invalid", "{entries:?}");
            let err = service
                .set_tool_deny(&entries)
                .expect_err("must be rejected");
            assert_eq!(err.code, "tool-entry-invalid", "{entries:?}");
        }
        assert!(!sandbox.capture.exists(), "no CLI call may be captured");
        let state = sandbox.state();
        assert!(state.get("tools").is_none(), "state must be unchanged");
    });
}

#[test]
fn set_exec_mode_over_fake_cli() {
    // a) Happy path: exact argv, state update, re-read.
    scenario("exec-mode-set", None, |sandbox| {
        sandbox.seed(serde_json::json!({}));
        let service = tools_service();
        service.set_exec_mode("ask").expect("set exec mode");
        let lines = sandbox.captured();
        assert_eq!(lines.len(), 2, "dry-run + commit");
        assert_eq!(
            lines[0],
            vec![
                "config",
                "set",
                "tools.exec.mode",
                "\"ask\"",
                "--strict-json",
                "--dry-run",
                "--json"
            ]
        );
        assert_eq!(
            lines[1],
            vec![
                "config",
                "set",
                "tools.exec.mode",
                "\"ask\"",
                "--strict-json"
            ]
        );
        let state = sandbox.state();
        assert_eq!(state["tools"]["exec"]["mode"], "ask");
        let policy = service.get_tool_policy().expect("re-read");
        assert_eq!(policy.exec_mode.as_deref(), Some("ask"));
    });

    // b) Invalid mode: exec-mode-invalid, CLI calls 0 times.
    scenario("exec-mode-invalid", None, |sandbox| {
        sandbox.seed(serde_json::json!({}));
        let service = tools_service();
        for mode in ["", "Full", "deny-all"] {
            let err = service.set_exec_mode(mode).expect_err("must be rejected");
            assert_eq!(err.code, "exec-mode-invalid", "{mode:?}");
        }
        assert!(!sandbox.capture.exists(), "no CLI call may be captured");
    });
}

#[test]
fn security_profile_apply_over_fake_cli() {
    // a) User profile: save (store only) then apply → the four-field write
    //    sequence (each dry-run → commit), state updated.
    scenario("profile-apply-user", None, |sandbox| {
        sandbox.seed(serde_json::json!({}));
        let service = security_service(sandbox);
        service
            .save_security_profile(&user_profile("my-profile", "내 프로필"))
            .expect("save profile");
        assert!(!sandbox.capture.exists(), "save never touches the CLI");

        service.apply_security_profile("my-profile").expect("apply");
        let lines = sandbox.captured();
        assert_eq!(lines.len(), 8, "4 fields x (dry-run + commit)");
        assert_eq!(
            lines[0],
            vec![
                "config",
                "set",
                "tools.profile",
                "\"messaging\"",
                "--strict-json",
                "--dry-run",
                "--json"
            ]
        );
        assert_eq!(
            lines[1],
            vec![
                "config",
                "set",
                "tools.profile",
                "\"messaging\"",
                "--strict-json"
            ]
        );
        assert_eq!(
            lines[2],
            vec![
                "config",
                "set",
                "tools.allow",
                "[]",
                "--strict-json",
                "--replace",
                "--dry-run",
                "--json"
            ]
        );
        assert_eq!(
            lines[3],
            vec![
                "config",
                "set",
                "tools.allow",
                "[]",
                "--strict-json",
                "--replace"
            ]
        );
        assert_eq!(
            lines[4],
            vec![
                "config",
                "set",
                "tools.deny",
                r#"["group:automation"]"#,
                "--strict-json",
                "--replace",
                "--dry-run",
                "--json"
            ]
        );
        assert_eq!(
            lines[5],
            vec![
                "config",
                "set",
                "tools.deny",
                r#"["group:automation"]"#,
                "--strict-json",
                "--replace"
            ]
        );
        assert_eq!(
            lines[6],
            vec![
                "config",
                "set",
                "tools.exec.mode",
                "\"ask\"",
                "--strict-json",
                "--dry-run",
                "--json"
            ]
        );
        assert_eq!(
            lines[7],
            vec![
                "config",
                "set",
                "tools.exec.mode",
                "\"ask\"",
                "--strict-json"
            ]
        );
        let state = sandbox.state();
        assert_eq!(state["tools"]["profile"], "messaging");
        assert_eq!(state["tools"]["allow"], serde_json::json!([]));
        assert_eq!(
            state["tools"]["deny"],
            serde_json::json!(["group:automation"])
        );
        assert_eq!(state["tools"]["exec"]["mode"], "ask");
        sandbox.assert_no_forbidden_audit_flags();
    });

    // b) Builtin profile: the same path, no store involvement.
    scenario("profile-apply-builtin", None, |sandbox| {
        sandbox.seed(serde_json::json!({}));
        let service = security_service(sandbox);
        service
            .apply_security_profile("hardened")
            .expect("apply builtin");
        let lines = sandbox.captured();
        assert_eq!(lines.len(), 8, "4 fields x (dry-run + commit)");
        assert_eq!(
            lines[1],
            vec![
                "config",
                "set",
                "tools.profile",
                "\"messaging\"",
                "--strict-json"
            ]
        );
        assert_eq!(
            lines[5],
            vec![
                "config",
                "set",
                "tools.deny",
                r#"["group:automation","group:runtime","group:fs","sessions_spawn","sessions_send"]"#,
                "--strict-json",
                "--replace"
            ]
        );
        assert_eq!(
            lines[7],
            vec![
                "config",
                "set",
                "tools.exec.mode",
                "\"deny\"",
                "--strict-json"
            ]
        );
        let state = sandbox.state();
        assert_eq!(state["tools"]["exec"]["mode"], "deny");
    });

    // c) Unknown id: security-profile-not-found, CLI calls 0 times.
    scenario("profile-apply-unknown", None, |sandbox| {
        sandbox.seed(serde_json::json!({}));
        let service = security_service(sandbox);
        let err = service
            .apply_security_profile("ghost")
            .expect_err("unknown id");
        assert_eq!(err.code, "security-profile-not-found");
        assert!(!sandbox.capture.exists(), "no CLI call may be captured");
    });

    // d) Save with a builtin id: security-profile-conflict, CLI calls 0.
    scenario("profile-save-builtin-conflict", None, |sandbox| {
        sandbox.seed(serde_json::json!({}));
        let service = security_service(sandbox);
        for id in ["default", "hardened"] {
            let mut profile = user_profile("", "");
            profile.id = id.into();
            profile.name = "x".into();
            let err = service
                .save_security_profile(&profile)
                .expect_err("builtin collision");
            assert_eq!(err.code, "security-profile-conflict", "{id}");
        }
        assert!(!sandbox.capture.exists(), "no CLI call may be captured");
        // The store file must not have been created by a rejected save.
        assert!(
            !sandbox.dir.join("security-profiles.json").exists(),
            "rejected save must not touch the store file"
        );
    });

    // e) Invalid id/name: the stable validation codes, CLI calls 0.
    scenario("profile-save-invalid", None, |sandbox| {
        sandbox.seed(serde_json::json!({}));
        let service = security_service(sandbox);
        let mut bad_id = user_profile("Bad-ID", "x");
        let err = service.save_security_profile(&bad_id).expect_err("bad id");
        assert_eq!(err.code, "security-profile-id-invalid");
        bad_id.id = "ok-id".into();
        bad_id.name = String::new();
        let err = service
            .save_security_profile(&bad_id)
            .expect_err("empty name");
        assert_eq!(err.code, "security-profile-name-invalid");
        assert!(!sandbox.capture.exists(), "no CLI call may be captured");
    });

    // f) Delete: builtin and unknown ids are not-found; a stored user
    //    profile deletes cleanly.
    scenario("profile-delete", None, |sandbox| {
        sandbox.seed(serde_json::json!({}));
        let service = security_service(sandbox);
        assert_eq!(
            service.delete_security_profile("default").unwrap_err().code,
            "security-profile-not-found"
        );
        assert_eq!(
            service.delete_security_profile("ghost").unwrap_err().code,
            "security-profile-not-found"
        );
        service
            .save_security_profile(&user_profile("my-profile", "x"))
            .expect("save");
        service
            .delete_security_profile("my-profile")
            .expect("delete");
        // Save/delete are store-file-only: zero CLI calls up to this point.
        assert!(
            !sandbox.capture.exists(),
            "save/delete must not invoke the CLI"
        );
        let list = service.list_security_profiles().expect("list");
        assert!(list.users.is_empty(), "user profile deleted");
        assert_eq!(
            sandbox.captured(),
            vec![vec!["config", "get", "tools", "--json"]],
            "list performs exactly one policy read"
        );
    });
}

#[test]
fn security_profile_list_and_applied_state_over_fake_cli() {
    // a) A policy matching a user profile → currentApplied is that profile.
    scenario("profile-list-applied", None, |sandbox| {
        sandbox.seed(serde_json::json!({
            "tools": {
                "profile": "messaging",
                "deny": ["group:automation"],
                "exec": {"mode": "ask"}
            }
        }));
        let service = security_service(sandbox);
        service
            .save_security_profile(&user_profile("my-profile", "내 프로필"))
            .expect("save");
        let list = service.list_security_profiles().expect("list");
        assert_eq!(list.builtins.len(), 2);
        assert_eq!(list.users.len(), 1);
        assert_eq!(list.users[0].id, "my-profile");
        assert_eq!(list.current_applied.as_deref(), Some("my-profile"));
        assert!(!list.policy_read_failed);
    });

    // b) Builtin priority: a policy matching `default` reports `default`
    //    even when an identical user profile exists.
    scenario("profile-list-builtin-priority", None, |sandbox| {
        sandbox.seed(serde_json::json!({
            "tools": { "profile": "coding" }
        }));
        let service = security_service(sandbox);
        let mut copy = clawdesk_lib::domain::models::builtin_profiles()[0].clone();
        copy.id = "default-copy".into();
        service.save_security_profile(&copy).expect("save copy");
        let list = service.list_security_profiles().expect("list");
        assert_eq!(list.current_applied.as_deref(), Some("default"));
    });

    // c) Custom policy → no applied profile.
    scenario("profile-list-custom", None, |sandbox| {
        sandbox.seed(serde_json::json!({
            "tools": { "profile": "full", "deny": ["web_search"] }
        }));
        let service = security_service(sandbox);
        let list = service.list_security_profiles().expect("list");
        assert_eq!(list.current_applied, None);
        assert!(!list.policy_read_failed);
    });

    // d) Policy read failure (corrupt config state → CLI failure) →
    //    policyReadFailed, list still shown.
    scenario("profile-list-policy-read-failed", None, |sandbox| {
        sandbox.seed(serde_json::json!({}));
        let service = security_service(sandbox);
        service
            .save_security_profile(&user_profile("my-profile", "x"))
            .expect("save");
        // Corrupt the config state so `config get tools --json` fails: the
        // profile list must still render, only the applied-state part
        // degrades (no clean-state guess).
        fs::write(sandbox.dir.join("openclaw.json"), "{ corrupt state").expect("corrupt the state");
        let list = service.list_security_profiles().expect("list");
        assert!(list.policy_read_failed, "read failure must be surfaced");
        assert_eq!(list.current_applied, None, "no applied guess");
        assert_eq!(list.builtins.len(), 2, "builtins still listed");
        assert_eq!(list.users.len(), 1, "user store still listed");
    });
}

#[test]
fn dry_run_rejection_writes_nothing() {
    scenario("tool-dry-run-reject", Some("config-invalid"), |sandbox| {
        sandbox.seed(serde_json::json!({}));
        let service = tools_service();
        let err = service
            .set_tool_profile("coding")
            .expect_err("dry-run reject");
        assert_eq!(err.code, "openclaw-config-invalid");
        assert!(err.message.contains("simulated schema rejection"));
        let lines = sandbox.captured();
        assert_eq!(lines.len(), 1, "only the dry-run ran, no real write");
        assert!(lines[0].contains(&"--dry-run".to_string()));
        let state = sandbox.state();
        assert!(state.get("tools").is_none(), "state must be unchanged");
    });
}

#[test]
fn security_audit_over_fake_cli() {
    // a) Happy path: exact argv, findings parsing (checkId-less rows
    //    dropped), suppressed count, no forbidden flags.
    scenario("audit-read", None, |sandbox| {
        sandbox.seed(serde_json::json!({
            "securityAudit": {
                "findings": [
                    {"checkId": "gateway.exposure.open", "severity": "critical",
                     "title": "Gateway reachable", "detail": "port 18789"},
                    {"severity": "warn", "title": "dropped: no checkId"}
                ],
                "suppressedFindings": [{"checkId": "s1"}, {"checkId": "s2"}]
            }
        }));
        let service = security_service(sandbox);
        let result = service.run_security_audit().expect("audit");
        assert_eq!(result.findings.len(), 2, "checkId-less row dropped");
        assert_eq!(
            result.findings[0].check_id, "tools.exec.security_full_configured",
            "derived row (exec.mode unset) comes first"
        );
        assert_eq!(result.findings[0].severity.as_deref(), Some("warn"));
        assert_eq!(result.findings[1].check_id, "gateway.exposure.open");
        assert_eq!(result.findings[1].severity.as_deref(), Some("critical"));
        assert_eq!(result.findings[1].detail.as_deref(), Some("port 18789"));
        assert_eq!(result.suppressed_count, 2);
        assert!(result.summary.get("total").is_some(), "summary object");
        let lines = sandbox.captured();
        assert_eq!(lines, vec![vec!["security", "audit", "--json"]]);
        sandbox.assert_no_forbidden_audit_flags();
    });

    // b) Nonzero exit (cli-error envelope) → openclaw-security-audit-failed.
    scenario("audit-cli-error", Some("cli-error"), |sandbox| {
        sandbox.seed(serde_json::json!({}));
        let service = security_service(sandbox);
        let err = service.run_security_audit().expect_err("cli error");
        assert_eq!(err.code, "openclaw-security-audit-failed");
        assert!(err.message.contains("simulated cli error"));
    });

    // c) Malformed output → same stable code.
    scenario("audit-malformed", Some("malformed"), |sandbox| {
        sandbox.seed(serde_json::json!({}));
        let service = security_service(sandbox);
        let err = service.run_security_audit().expect_err("malformed output");
        assert_eq!(err.code, "openclaw-security-audit-failed");
    });

    // d) Not-json output → same stable code.
    scenario("audit-not-json", Some("not-json"), |sandbox| {
        sandbox.seed(serde_json::json!({}));
        let service = security_service(sandbox);
        let err = service.run_security_audit().expect_err("not json");
        assert_eq!(err.code, "openclaw-security-audit-failed");
    });

    // e) Failing CLI: the error message stays masked (S3/S8).
    scenario("audit-fail-masked", Some("fail"), |sandbox| {
        sandbox.seed(serde_json::json!({}));
        let service = security_service(sandbox);
        let err = service.run_security_audit().expect_err("simulated failure");
        assert_eq!(err.code, "openclaw-security-audit-failed");
        assert!(
            !err.message.contains("sk-fake123456789"),
            "secret must not leak into the error: {}",
            err.message
        );
        assert!(err.message.contains("sk-****"), "stderr should be masked");
    });
}

#[test]
fn tools_security_missing_executable() {
    // Service level: detection failure → the reused Phase 1 code, 0 CLI calls.
    let tools = tools_service_no_openclaw();
    assert_eq!(
        tools.get_tool_policy().unwrap_err().code,
        "openclaw-not-found"
    );
    // A valid value passes validation, so detection failure must surface.
    assert_eq!(
        tools.set_tool_profile("coding").unwrap_err().code,
        "openclaw-not-found"
    );
    assert_eq!(
        tools
            .set_tool_allow(&["web_search".into()])
            .unwrap_err()
            .code,
        "openclaw-not-found"
    );
    assert_eq!(
        tools.set_tool_deny(&["group:fs".into()]).unwrap_err().code,
        "openclaw-not-found"
    );
    assert_eq!(
        tools.set_exec_mode("deny").unwrap_err().code,
        "openclaw-not-found"
    );

    let sandbox = Sandbox::new("no-openclaw-security");
    let security = SecurityProfileService::new(
        Arc::new(NoOpenClaw),
        Arc::new(OpenClawSecurityAdapter::new(Arc::new(ProcessRunner))),
        Arc::new(OpenClawConfigAdapter::new(Arc::new(ProcessRunner))),
        Arc::new(SecurityProfileStore::new(
            sandbox.dir.join("security-profiles.json"),
        )),
    );
    assert_eq!(
        security.run_security_audit().unwrap_err().code,
        "openclaw-not-found"
    );
    assert_eq!(
        security
            .apply_security_profile("hardened")
            .unwrap_err()
            .code,
        "openclaw-not-found",
        "builtin resolves, then detection fails — no config write"
    );
    // The store side is independent of OpenClaw: list still works and only
    // the policy part degrades.
    let list = security.list_security_profiles().expect("list");
    assert!(list.policy_read_failed);
    assert_eq!(list.builtins.len(), 2);
}

// --- fake-level runner --------------------------------------------------------------

fn run_fake(
    argv: &[&str],
    sandbox: &Sandbox,
    extra_envs: &[(&str, &str)],
) -> Result<ProcessOutput, ProcessError> {
    let argv: Vec<String> = argv.iter().map(|arg| arg.to_string()).collect();
    let mut request = ProcessRequest::new(FAKE_OPENCLAW, argv, TIMEOUT);
    request.env.push((
        "CLAWDESK_FAKE_STATE".to_string(),
        sandbox.dir.to_string_lossy().into_owned(),
    ));
    request.env.push((
        "CLAWDESK_FAKE_CAPTURE".to_string(),
        sandbox.capture.to_string_lossy().into_owned(),
    ));
    for (key, value) in extra_envs {
        request.env.push((key.to_string(), value.to_string()));
    }
    ProcessRunner.run(&request)
}
