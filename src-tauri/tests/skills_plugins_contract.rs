//! Phase 4 contract tests: real `ProcessRunner` + fake `openclaw` CLI with a
//! per-test state sandbox (S5: fake CLI only, no real OpenClaw, no system
//! mutation — sandboxes live under the cargo target dir).
//!
//! Adapter/service-driven scenarios run inside a single serialized section
//! because the fake receives its sandbox via inherited process environment.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use clawdesk_lib::application::{PluginsService, SkillsService};
use clawdesk_lib::domain::models::openclaw::{GatewayStatus, OpenClawVersion, UpdateState};
use clawdesk_lib::domain::models::{ExecutableDetection, SkillRow};
use clawdesk_lib::domain::ports::openclaw::OpenClawPort;
use clawdesk_lib::domain::ports::process::{
    ProcessError, ProcessOutput, ProcessPort, ProcessRequest,
};
use clawdesk_lib::domain::ports::{OpenClawPluginsPort, OpenClawSkillsPort};
use clawdesk_lib::error::AppError;
use clawdesk_lib::infrastructure::openclaw::{
    OpenClawConfigAdapter, OpenClawPluginsAdapter, OpenClawSkillsAdapter,
};
use clawdesk_lib::infrastructure::process::ProcessRunner;

const FAKE_OPENCLAW: &str = env!("CARGO_BIN_EXE_clawdesk-fake-openclaw");
const TIMEOUT: Duration = Duration::from_secs(10);

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

fn exe() -> &'static Path {
    Path::new(FAKE_OPENCLAW)
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

fn skills_service() -> SkillsService {
    SkillsService::new(
        Arc::new(DetectsFakeCli),
        Arc::new(OpenClawSkillsAdapter::new(Arc::new(ProcessRunner))),
        Arc::new(OpenClawConfigAdapter::new(Arc::new(ProcessRunner))),
    )
}

fn plugins_service() -> PluginsService {
    PluginsService::new(
        Arc::new(DetectsFakeCli),
        Arc::new(OpenClawPluginsAdapter::new(Arc::new(ProcessRunner))),
    )
}

// --- fake-level behavior (per-request env; parallel-safe) -------------------------

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

#[test]
fn skills_list_emits_rows_from_state() {
    let sandbox = Sandbox::new("fake-skills-list");
    sandbox.seed(serde_json::json!({
        "skills": {
            "catalog": {
                "weather": {"description": "Weather forecast", "source": "bundled"},
                "github": {"eligible": false}
            },
            "entries": { "github": { "enabled": false } }
        }
    }));
    let output =
        run_fake(&["skills", "list", "--json"], &sandbox, &[]).expect("skills list should run");
    assert_eq!(output.exit_code, 0);
    let value: serde_json::Value =
        serde_json::from_str(&output.stdout).expect("payload should parse");
    let rows = value["skills"].as_array().expect("rows array");
    assert_eq!(rows.len(), 2);
    let github = &rows[0];
    assert_eq!(github["name"], "github");
    assert_eq!(github["enabled"], false, "entries override wins");
    assert_eq!(github["eligible"], false);
    let weather = &rows[1];
    assert_eq!(weather["name"], "weather");
    assert_eq!(weather["enabled"], true, "default enabled");
    assert_eq!(weather["description"], "Weather forecast");
    assert_eq!(weather["source"], "bundled");
    assert_eq!(sandbox.captured(), vec![vec!["skills", "list", "--json"]]);
}

#[test]
fn skills_info_unknown_name_is_nonzero() {
    let sandbox = Sandbox::new("fake-skills-info-unknown");
    sandbox.seed(serde_json::json!({
        "skills": { "catalog": { "weather": {} } }
    }));
    let output = run_fake(&["skills", "info", "ghost", "--json"], &sandbox, &[])
        .expect("skills info should run");
    assert_eq!(output.exit_code, 2);
    assert!(output.stderr.contains("unknown skill: ghost"));
}

#[test]
fn plugins_list_emits_rows_from_state() {
    let sandbox = Sandbox::new("fake-plugins-list");
    sandbox.seed(serde_json::json!({
        "plugins": {
            "catalog": {
                "@openclaw/discord": {
                    "name": "Discord", "format": "module", "version": "1.2.3",
                    "origin": "npm", "dependencyStatus": "ok"
                },
                "local-plugin": {"format": "local"}
            },
            "entries": { "@openclaw/discord": { "enabled": false } }
        }
    }));
    let output =
        run_fake(&["plugins", "list", "--json"], &sandbox, &[]).expect("plugins list should run");
    assert_eq!(output.exit_code, 0);
    let value: serde_json::Value =
        serde_json::from_str(&output.stdout).expect("payload should parse");
    let rows = value["plugins"].as_array().expect("rows array");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["id"], "@openclaw/discord");
    assert_eq!(rows[0]["enabled"], false, "entries override wins");
    assert_eq!(rows[0]["name"], "Discord");
    assert_eq!(rows[0]["dependencyStatus"], "ok");
    assert_eq!(rows[1]["id"], "local-plugin");
    assert_eq!(rows[1]["enabled"], true, "default enabled");
    assert_eq!(sandbox.captured(), vec![vec!["plugins", "list", "--json"]]);
}

#[test]
fn plugins_enable_unknown_id_is_nonzero_and_state_unchanged() {
    let sandbox = Sandbox::new("fake-plugins-enable-unknown");
    sandbox.seed(serde_json::json!({
        "plugins": { "catalog": { "known": {} } }
    }));
    let output =
        run_fake(&["plugins", "enable", "ghost"], &sandbox, &[]).expect("enable should run");
    assert_eq!(output.exit_code, 2);
    assert!(output.stderr.contains("unknown plugin: ghost"));
    let state = sandbox.state();
    assert!(
        state["plugins"].get("entries").is_none(),
        "state must be unchanged"
    );
}

#[test]
fn plugins_inspect_unknown_id_is_nonzero() {
    let sandbox = Sandbox::new("fake-plugins-inspect-unknown");
    sandbox.seed(serde_json::json!({
        "plugins": { "catalog": { "known": {} } }
    }));
    let output = run_fake(
        &["plugins", "inspect", "ghost", "--runtime", "--json"],
        &sandbox,
        &[],
    )
    .expect("inspect should run");
    assert_eq!(output.exit_code, 2);
    assert!(output.stderr.contains("unknown plugin: ghost"));
}

#[test]
fn plugins_toggle_rejected_in_nix_mode() {
    let sandbox = Sandbox::new("fake-plugins-nix");
    sandbox.seed(serde_json::json!({
        "plugins": { "catalog": { "known": {} } }
    }));
    let output = run_fake(
        &["plugins", "enable", "known"],
        &sandbox,
        &[("OPENCLAW_NIX_MODE", "1")],
    )
    .expect("enable should run");
    assert_eq!(output.exit_code, 2);
    assert!(output.stderr.contains("nix mode"));
    let state = sandbox.state();
    assert!(state["plugins"].get("entries").is_none(), "state unchanged");
}

#[test]
fn skills_list_slow_process_times_out() {
    let sandbox = Sandbox::new("fake-skills-timeout");
    sandbox.seed(serde_json::json!({
        "skills": { "catalog": { "weather": {} } }
    }));
    let argv = vec![
        "skills".to_string(),
        "list".to_string(),
        "--json".to_string(),
    ];
    let mut request = ProcessRequest::new(FAKE_OPENCLAW, argv, Duration::from_millis(400));
    request.env.push((
        "CLAWDESK_FAKE_STATE".to_string(),
        sandbox.dir.to_string_lossy().into_owned(),
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

#[test]
fn skills_list_failure_output_is_masked() {
    let sandbox = Sandbox::new("fake-skills-masking");
    sandbox.seed(serde_json::json!({
        "skills": { "catalog": { "weather": {} } }
    }));
    let output = run_fake(
        &["skills", "list", "--json"],
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
fn skills_list_over_fake_cli() {
    scenario("skills-list", None, |sandbox| {
        sandbox.seed(serde_json::json!({
            "skills": {
                "catalog": {
                    "weather": {"description": "Weather forecast", "source": "bundled"},
                    "github": {"eligible": false},
                    "minimal": {}
                },
                "entries": { "github": { "enabled": false } }
            }
        }));
        let rows = OpenClawSkillsAdapter::new(Arc::new(ProcessRunner))
            .list_skills(exe())
            .expect("skills list");
        assert_eq!(rows.len(), 3, "no row may be dropped");
        let github = rows.iter().find(|row| row.name == "github").expect("row");
        assert_eq!(github.enabled, Some(false));
        assert_eq!(github.eligible, Some(false));
        assert_eq!(github.description, None);
        let weather = rows.iter().find(|row| row.name == "weather").expect("row");
        assert_eq!(weather.enabled, Some(true));
        assert_eq!(weather.eligible, Some(true));
        assert_eq!(weather.description.as_deref(), Some("Weather forecast"));
        assert_eq!(weather.source.as_deref(), Some("bundled"));
        // Fail-soft: a catalog entry with no optional fields still yields
        // a full row with nulls.
        let minimal = rows.iter().find(|row| row.name == "minimal").expect("row");
        assert_eq!(minimal.enabled, Some(true));
        assert_eq!(minimal.description, None);
        assert_eq!(sandbox.captured(), vec![vec!["skills", "list", "--json"]]);
    });
}

#[test]
fn skill_toggle_over_fake_cli() {
    // a) toggle to disabled: exact argv (list check + dry-run + commit),
    // state update, re-list confirmation.
    scenario("skill-toggle-off", None, |sandbox| {
        sandbox.seed(serde_json::json!({
            "skills": { "catalog": { "weather": {} } }
        }));
        let service = skills_service();
        service
            .set_skill_enabled("weather", false)
            .expect("toggle off");
        let lines = sandbox.captured();
        assert_eq!(lines.len(), 3, "list check, dry-run, commit");
        assert_eq!(lines[0], vec!["skills", "list", "--json"]);
        assert_eq!(
            lines[1],
            vec![
                "config",
                "set",
                "skills.entries.weather.enabled",
                "false",
                "--strict-json",
                "--dry-run",
                "--json"
            ]
        );
        assert_eq!(
            lines[2],
            vec![
                "config",
                "set",
                "skills.entries.weather.enabled",
                "false",
                "--strict-json"
            ]
        );
        let state = sandbox.state();
        assert_eq!(state["skills"]["entries"]["weather"]["enabled"], false);
        // Re-list (what the UI does after the response) shows the new state.
        let rows = service.list_skills().expect("re-list");
        let weather: &SkillRow = rows.iter().find(|row| row.name == "weather").expect("row");
        assert_eq!(weather.enabled, Some(false));
    });

    // b) toggle to enabled: the same flow with `true`.
    scenario("skill-toggle-on", None, |sandbox| {
        sandbox.seed(serde_json::json!({
            "skills": {
                "catalog": { "weather": {} },
                "entries": { "weather": { "enabled": false } }
            }
        }));
        let service = skills_service();
        service
            .set_skill_enabled("weather", true)
            .expect("toggle on");
        let lines = sandbox.captured();
        assert_eq!(
            lines[2],
            vec![
                "config",
                "set",
                "skills.entries.weather.enabled",
                "true",
                "--strict-json"
            ]
        );
        let rows = service.list_skills().expect("re-list");
        let weather: &SkillRow = rows.iter().find(|row| row.name == "weather").expect("row");
        assert_eq!(weather.enabled, Some(true));
    });

    // c) unknown skill: skill-not-found, config write 0 times.
    scenario("skill-toggle-unknown", None, |sandbox| {
        sandbox.seed(serde_json::json!({
            "skills": { "catalog": { "weather": {} } }
        }));
        let service = skills_service();
        let err = service
            .set_skill_enabled("ghost", true)
            .expect_err("unknown skill");
        assert_eq!(err.code, "skill-not-found");
        let lines = sandbox.captured();
        assert_eq!(lines.len(), 1, "only the existence-check list ran");
        assert_eq!(lines[0], vec!["skills", "list", "--json"]);
        let state = sandbox.state();
        assert!(
            state["skills"].get("entries").is_none(),
            "no entries section may be created"
        );
    });

    // d) invalid skill name: skill-name-invalid, CLI calls 0 times.
    scenario("skill-toggle-invalid-name", None, |sandbox| {
        sandbox.seed(serde_json::json!({
            "skills": { "catalog": { "weather": {} } }
        }));
        let service = skills_service();
        for name in ["../evil", "a/b", "a b"] {
            let err = service
                .set_skill_enabled(name, true)
                .expect_err("must be rejected");
            assert_eq!(err.code, "skill-name-invalid", "{name:?}");
        }
        assert!(!sandbox.capture.exists(), "no CLI call may be captured");
    });
}

#[test]
fn plugins_list_over_fake_cli() {
    scenario("plugins-list", None, |sandbox| {
        sandbox.seed(serde_json::json!({
            "plugins": {
                "catalog": {
                    "@openclaw/discord": {
                        "name": "Discord", "format": "module", "version": "1.2.3",
                        "origin": "npm", "dependencyStatus": "ok"
                    },
                    "local-plugin": {"format": "local"}
                },
                "entries": { "@openclaw/discord": { "enabled": false } }
            }
        }));
        let rows = OpenClawPluginsAdapter::new(Arc::new(ProcessRunner))
            .list_plugins(exe())
            .expect("plugins list");
        assert_eq!(rows.len(), 2);
        let discord = rows
            .iter()
            .find(|row| row.id == "@openclaw/discord")
            .expect("row");
        assert_eq!(discord.enabled, Some(false));
        assert_eq!(discord.name.as_deref(), Some("Discord"));
        assert_eq!(discord.format.as_deref(), Some("module"));
        assert_eq!(discord.origin.as_deref(), Some("npm"));
        assert_eq!(discord.version.as_deref(), Some("1.2.3"));
        assert_eq!(discord.dependency_status.as_deref(), Some("ok"));
        let local = rows
            .iter()
            .find(|row| row.id == "local-plugin")
            .expect("row");
        assert_eq!(local.enabled, Some(true));
        assert_eq!(local.name, None, "fail-soft: missing optional → null");
        assert_eq!(sandbox.captured(), vec![vec!["plugins", "list", "--json"]]);
    });
}

#[test]
fn plugin_toggle_over_fake_cli() {
    // a) enable: exact single argv row, state update, re-list.
    scenario("plugin-enable", None, |sandbox| {
        sandbox.seed(serde_json::json!({
            "plugins": {
                "catalog": { "@openclaw/discord": {} },
                "entries": { "@openclaw/discord": { "enabled": false } }
            }
        }));
        let service = plugins_service();
        service
            .set_plugin_enabled("@openclaw/discord", true)
            .expect("enable");
        let lines = sandbox.captured();
        assert_eq!(lines, vec![vec!["plugins", "enable", "@openclaw/discord"]]);
        let state = sandbox.state();
        assert_eq!(
            state["plugins"]["entries"]["@openclaw/discord"]["enabled"],
            true
        );
        let rows = service.list_plugins().expect("re-list");
        assert_eq!(rows[0].enabled, Some(true));
    });

    // b) disable: exact single argv row, state update, re-list.
    scenario("plugin-disable", None, |sandbox| {
        sandbox.seed(serde_json::json!({
            "plugins": { "catalog": { "@openclaw/discord": {} } }
        }));
        let service = plugins_service();
        service
            .set_plugin_enabled("@openclaw/discord", false)
            .expect("disable");
        let lines = sandbox.captured();
        assert_eq!(lines, vec![vec!["plugins", "disable", "@openclaw/discord"]]);
        let rows = service.list_plugins().expect("re-list");
        assert_eq!(rows[0].enabled, Some(false));
    });

    // c) unknown id: nonzero → toggle-failed, state unchanged.
    scenario("plugin-toggle-unknown", None, |sandbox| {
        sandbox.seed(serde_json::json!({
            "plugins": { "catalog": { "known": {} } }
        }));
        let service = plugins_service();
        let err = service
            .set_plugin_enabled("ghost", true)
            .expect_err("unknown id");
        assert_eq!(err.code, "openclaw-plugin-toggle-failed");
        assert!(err.message.contains("unknown plugin: ghost"));
        let state = sandbox.state();
        assert!(state["plugins"].get("entries").is_none(), "state unchanged");
    });

    // d) Nix-mode rejection: same error path, no special handling.
    scenario("plugin-toggle-nix", None, |sandbox| {
        sandbox.seed(serde_json::json!({
            "plugins": { "catalog": { "known": {} } }
        }));
        let service = plugins_service();
        std::env::set_var("OPENCLAW_NIX_MODE", "1");
        let result = service.set_plugin_enabled("known", true);
        std::env::remove_var("OPENCLAW_NIX_MODE");
        let err = result.expect_err("nix mode must reject");
        assert_eq!(err.code, "openclaw-plugin-toggle-failed");
        assert!(err.message.contains("nix mode"));
    });

    // e) invalid plugin id: plugin-id-invalid, CLI calls 0 times.
    scenario("plugin-toggle-invalid-id", None, |sandbox| {
        sandbox.seed(serde_json::json!({
            "plugins": { "catalog": { "known": {} } }
        }));
        let service = plugins_service();
        for id in ["a b", "..", "a/b"] {
            let err = service
                .set_plugin_enabled(id, true)
                .expect_err("must be rejected");
            assert_eq!(err.code, "plugin-id-invalid", "{id:?}");
            let err = service
                .get_plugin_runtime(id)
                .expect_err("must be rejected");
            assert_eq!(err.code, "plugin-id-invalid", "{id:?}");
        }
        assert!(!sandbox.capture.exists(), "no CLI call may be captured");
    });
}

#[test]
fn plugin_runtime_inspect_over_fake_cli() {
    // a) happy path: exact argv, surface parsing, diagnostics.
    scenario("plugin-runtime", None, |sandbox| {
        sandbox.seed(serde_json::json!({
            "plugins": {
                "catalog": {
                    "@openclaw/discord": {
                        "runtime": {
                            "tools": ["discord_send", "discord_read"],
                            "hooks": [{"name": "on-message"}],
                            "services": ["discord"],
                            "gatewayMethods": ["discord.connect"],
                            "routes": ["/discord/events"]
                        },
                        "diagnostics": ["loaded in 12ms"]
                    }
                }
            }
        }));
        let service = plugins_service();
        let runtime = service
            .get_plugin_runtime("@openclaw/discord")
            .expect("inspect");
        assert_eq!(runtime.id, "@openclaw/discord");
        assert_eq!(runtime.tools, vec!["discord_send", "discord_read"]);
        assert_eq!(runtime.hooks, vec!["on-message"], "object coerces to name");
        assert_eq!(runtime.services, vec!["discord"]);
        assert!(runtime.cli_commands.is_empty(), "absent → empty");
        assert_eq!(runtime.gateway_methods, vec!["discord.connect"]);
        assert_eq!(runtime.routes, vec!["/discord/events"]);
        assert_eq!(
            runtime.diagnostics.as_deref(),
            Some(vec!["loaded in 12ms".to_string()].as_slice())
        );
        let lines = sandbox.captured();
        assert_eq!(
            lines,
            vec![vec![
                "plugins",
                "inspect",
                "@openclaw/discord",
                "--runtime",
                "--json"
            ]]
        );
    });

    // b) plugin without runtime data: all surfaces empty, no diagnostics.
    scenario("plugin-runtime-empty", None, |sandbox| {
        sandbox.seed(serde_json::json!({
            "plugins": { "catalog": { "minimal": {} } }
        }));
        let service = plugins_service();
        let runtime = service.get_plugin_runtime("minimal").expect("inspect");
        assert!(runtime.tools.is_empty());
        assert!(runtime.routes.is_empty());
        assert!(runtime.diagnostics.is_none());
    });

    // c) unknown id: nonzero → plugins-read-failed.
    scenario("plugin-runtime-unknown", None, |sandbox| {
        sandbox.seed(serde_json::json!({
            "plugins": { "catalog": { "known": {} } }
        }));
        let service = plugins_service();
        let err = service.get_plugin_runtime("ghost").expect_err("unknown id");
        assert_eq!(err.code, "openclaw-plugins-read-failed");
    });
}

#[test]
fn skills_plugins_read_failures() {
    // a) malformed JSON output → skills read-failed.
    scenario("skills-malformed", Some("malformed"), |sandbox| {
        sandbox.seed(serde_json::json!({
            "skills": { "catalog": { "weather": {} } }
        }));
        let err = OpenClawSkillsAdapter::new(Arc::new(ProcessRunner))
            .list_skills(exe())
            .expect_err("malformed output");
        assert_eq!(err.code, "openclaw-skills-read-failed");
    });

    // b) malformed JSON output → plugins read-failed.
    scenario("plugins-malformed", Some("malformed"), |sandbox| {
        sandbox.seed(serde_json::json!({
            "plugins": { "catalog": { "known": {} } }
        }));
        let err = OpenClawPluginsAdapter::new(Arc::new(ProcessRunner))
            .list_plugins(exe())
            .expect_err("malformed output");
        assert_eq!(err.code, "openclaw-plugins-read-failed");
    });

    // c) not-json output → both read-failed codes.
    scenario("skills-not-json", Some("not-json"), |_sandbox| {
        let err = OpenClawSkillsAdapter::new(Arc::new(ProcessRunner))
            .list_skills(exe())
            .expect_err("not json");
        assert_eq!(err.code, "openclaw-skills-read-failed");
    });
    scenario("plugins-not-json", Some("not-json"), |_sandbox| {
        let err = OpenClawPluginsAdapter::new(Arc::new(ProcessRunner))
            .list_plugins(exe())
            .expect_err("not json");
        assert_eq!(err.code, "openclaw-plugins-read-failed");
    });

    // d) cli-error (exit 1) → read-failed for both.
    scenario("skills-cli-error", Some("cli-error"), |_sandbox| {
        let err = OpenClawSkillsAdapter::new(Arc::new(ProcessRunner))
            .list_skills(exe())
            .expect_err("cli error");
        assert_eq!(err.code, "openclaw-skills-read-failed");
    });
    scenario("plugins-cli-error", Some("cli-error"), |_sandbox| {
        let err = OpenClawPluginsAdapter::new(Arc::new(ProcessRunner))
            .list_plugins(exe())
            .expect_err("cli error");
        assert_eq!(err.code, "openclaw-plugins-read-failed");
    });

    // e) failing CLI: the error message must stay masked (S3/S8).
    scenario("skills-fail-masked", Some("fail"), |_sandbox| {
        let err = OpenClawSkillsAdapter::new(Arc::new(ProcessRunner))
            .list_skills(exe())
            .expect_err("simulated failure");
        assert_eq!(err.code, "openclaw-skills-read-failed");
        assert!(
            !err.message.contains("sk-fake123456789"),
            "secret must not leak into the error: {}",
            err.message
        );
    });
}

#[test]
fn skills_plugins_missing_executable() {
    // Service level: detection failure → the reused Phase 1 code.
    let skills = SkillsService::new(
        Arc::new(NoOpenClaw),
        Arc::new(OpenClawSkillsAdapter::new(Arc::new(ProcessRunner))),
        Arc::new(OpenClawConfigAdapter::new(Arc::new(ProcessRunner))),
    );
    assert_eq!(skills.list_skills().unwrap_err().code, "openclaw-not-found");
    // A valid name passes validation, so detection failure must surface.
    assert_eq!(
        skills.set_skill_enabled("weather", true).unwrap_err().code,
        "openclaw-not-found",
        "validation passes, detection fails"
    );

    let plugins = PluginsService::new(
        Arc::new(NoOpenClaw),
        Arc::new(OpenClawPluginsAdapter::new(Arc::new(ProcessRunner))),
    );
    assert_eq!(
        plugins.list_plugins().unwrap_err().code,
        "openclaw-not-found"
    );
    assert_eq!(
        plugins
            .set_plugin_enabled("discord", true)
            .unwrap_err()
            .code,
        "openclaw-not-found"
    );
}
