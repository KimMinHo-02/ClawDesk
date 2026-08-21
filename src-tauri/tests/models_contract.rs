//! Phase 3 contract tests: real `ProcessRunner` + fake `openclaw` CLI with a
//! per-test config state sandbox (S5: fake CLI only, no real OpenClaw, no
//! system mutation — sandboxes live under the cargo target dir).
//!
//! Adapter-driven scenarios run inside a single serialized test because the
//! fake receives its sandbox via inherited process environment (the shared
//! `ProcessRunner` has no per-request env injection for adapter calls).
//!
//! The secret-resolver section runs the real resolver binary against a
//! sandboxed secret-store root (DPAPI calls are pure byte transforms).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use clawdesk_lib::application::services::api_key::ApiKeyService;
use clawdesk_lib::domain::models::models::ThinkingLevel;
use clawdesk_lib::domain::models::openclaw::{GatewayStatus, OpenClawVersion, UpdateState};
use clawdesk_lib::domain::models::{ExecutableDetection, ProviderApiKey};
use clawdesk_lib::domain::ports::openclaw::OpenClawPort;
use clawdesk_lib::domain::ports::openclaw_config::{OpenClawConfigPort, WriteMode};
use clawdesk_lib::domain::ports::process::{
    ProcessError, ProcessOutput, ProcessPort, ProcessRequest,
};
use clawdesk_lib::domain::ports::secrets::SecretStorePort;
use clawdesk_lib::error::AppError;
use clawdesk_lib::infrastructure::openclaw::OpenClawConfigAdapter;
use clawdesk_lib::infrastructure::process::ProcessRunner;
use clawdesk_lib::infrastructure::secrets::dpapi::WindowsDpapi;
use clawdesk_lib::infrastructure::secrets::SecretStore;

const FAKE_OPENCLAW: &str = env!("CARGO_BIN_EXE_clawdesk-fake-openclaw");
const RESOLVER: &str = env!("CARGO_BIN_EXE_clawdesk-secret-resolver");
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
        // Directory name avoids `sk-` adjacency with longer tokens: the
        // process-output masking pipeline (S8) masks anything after `sk-`.
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

/// Runs one serialized adapter-driven sub-scenario in its own sandbox.
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

fn adapter() -> OpenClawConfigAdapter {
    OpenClawConfigAdapter::new(Arc::new(ProcessRunner))
}

fn exe() -> &'static Path {
    Path::new(FAKE_OPENCLAW)
}

// --- fake-level behavior (per-request env; parallel-safe) -------------------------

#[test]
fn config_file_reports_sandbox_path() {
    let sandbox = Sandbox::new("config-file");
    let output =
        run_fake(&["config", "file", "--json"], &sandbox, &[]).expect("config file should run");
    assert_eq!(output.exit_code, 0);
    let value: serde_json::Value =
        serde_json::from_str(&output.stdout).expect("payload should parse");
    let expected = sandbox
        .dir
        .join("openclaw.json")
        .to_string_lossy()
        .to_string();
    assert_eq!(value["path"].as_str(), Some(expected.as_str()));
}

#[test]
fn config_get_redacts_secret_strings_but_keeps_refs() {
    let sandbox = Sandbox::new("redaction");
    sandbox.seed(serde_json::json!({
        "models": {"providers": {
            "plain": {
                "baseUrl": "https://plain.test",
                "apiKey": "sk-fake123456789"
            },
            "managed": {
                "baseUrl": "https://managed.test",
                "apiKey": {"source": "exec", "provider": "clawdesk", "id": "providers/managed/apiKey"}
            }
        }},
        "agents": {"defaults": {}},
        "secrets": {"providers": {}}
    }));
    let output = run_fake(
        &["config", "get", "models.providers", "--json"],
        &sandbox,
        &[],
    )
    .expect("config get should run");
    assert_eq!(output.exit_code, 0);
    assert!(
        !output.stdout.contains("sk-fake123456789"),
        "plaintext key must be redacted: {}",
        output.stdout
    );
    let value: serde_json::Value =
        serde_json::from_str(&output.stdout).expect("payload should parse");
    // The fake redacts to "***"; the S8 output mask may re-mask it under the
    // secret-named key — either way the plaintext must not survive.
    let redacted = value["plain"]["apiKey"].as_str().expect("string value");
    assert_ne!(redacted, "sk-fake123456789");
    assert!(
        redacted.chars().count() <= 8,
        "value must stay short: {redacted}"
    );
    assert_eq!(value["managed"]["apiKey"]["provider"], "clawdesk");
}

/// Runs the fake CLI with explicit per-request sandbox env.
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

// --- adapter config lifecycle (serialized sub-scenarios) ---------------------------

#[test]
fn adapter_config_lifecycle_over_fake_cli() {
    // a) provider add: exact argv (dry-run + commit) and state.
    scenario("add-provider", None, |sandbox| {
        let payload = r#"{"baseUrl":"https://api.acme.test/v1","api":"openai-completions","models":[{"id":"m1","name":"M1","reasoning":true}]}"#;
        adapter()
            .write(exe(), "models.providers.acme", payload, WriteMode::Merge)
            .expect("write should succeed");
        let lines = sandbox.captured();
        assert_eq!(lines.len(), 2, "dry-run then commit");
        assert_eq!(
            lines[0],
            vec![
                "config",
                "set",
                "models.providers.acme",
                payload,
                "--strict-json",
                "--merge",
                "--dry-run",
                "--json",
            ]
        );
        assert_eq!(
            lines[1],
            vec![
                "config",
                "set",
                "models.providers.acme",
                payload,
                "--strict-json",
                "--merge"
            ]
        );
        let state = sandbox.state();
        assert_eq!(
            state["models"]["providers"]["acme"]["baseUrl"],
            "https://api.acme.test/v1"
        );
        assert_eq!(
            state["models"]["providers"]["acme"]["models"][0]["id"],
            "m1"
        );
        assert!(
            state["models"]["providers"]["acme"].get("apiKey").is_none(),
            "no apiKey field in a created provider"
        );
    });

    // b) provider update: subpath writes preserve the apiKey + siblings.
    scenario("update-provider", None, |sandbox| {
        sandbox.seed(serde_json::json!({
            "models": {"providers": {
                "acme": {
                    "baseUrl": "https://old.test",
                    "api": "anthropic-messages",
                    "apiKey": "sk-fake123456789",
                    "models": [{"id": "old-model"}]
                },
                "beta": {"baseUrl": "https://beta.test"}
            }},
            "agents": {"defaults": {}},
            "secrets": {"providers": {}}
        }));
        adapter()
            .write(
                exe(),
                "models.providers.acme.baseUrl",
                "\"https://new.test/v1\"",
                WriteMode::Plain,
            )
            .expect("baseUrl write");
        adapter()
            .write(
                exe(),
                "models.providers.acme.api",
                "\"openai-completions\"",
                WriteMode::Plain,
            )
            .expect("api write");
        adapter()
            .write(
                exe(),
                "models.providers.acme.models",
                r#"[{"id":"m1","reasoning":true}]"#,
                WriteMode::Replace,
            )
            .expect("models write");

        let state = sandbox.state();
        let acme = &state["models"]["providers"]["acme"];
        assert_eq!(acme["baseUrl"], "https://new.test/v1");
        assert_eq!(acme["api"], "openai-completions");
        assert_eq!(acme["models"][0]["id"], "m1");
        assert_eq!(
            acme["apiKey"], "sk-fake123456789",
            "apiKey subpath preserved"
        );
        assert_eq!(
            state["models"]["providers"]["beta"]["baseUrl"],
            "https://beta.test"
        );

        let lines = sandbox.captured();
        let models_write = lines
            .iter()
            .find(|line| line.get(2).map(|p| p.as_str()) == Some("models.providers.acme.models"))
            .expect("models write captured");
        assert!(models_write.contains(&"--replace".to_string()));
        let base_url_write = lines
            .iter()
            .find(|line| line.get(2).map(|p| p.as_str()) == Some("models.providers.acme.baseUrl"))
            .expect("baseUrl write captured");
        assert!(!base_url_write.contains(&"--merge".to_string()));
        assert!(!base_url_write.contains(&"--replace".to_string()));
    });

    // c) merge mode merges into the existing provider object.
    scenario("merge-provider", None, |sandbox| {
        sandbox.seed(serde_json::json!({
            "models": {"providers": {"acme": {"api": "anthropic-messages", "models": [{"id": "keep"}]}}},
            "agents": {"defaults": {}},
            "secrets": {"providers": {}}
        }));
        adapter()
            .write(
                exe(),
                "models.providers.acme",
                r#"{"baseUrl":"https://api.acme.test"}"#,
                WriteMode::Merge,
            )
            .expect("merge write");
        let state = sandbox.state();
        let acme = &state["models"]["providers"]["acme"];
        assert_eq!(acme["baseUrl"], "https://api.acme.test");
        assert_eq!(
            acme["api"], "anthropic-messages",
            "merge keeps existing fields"
        );
        assert_eq!(acme["models"][0]["id"], "keep");
    });

    // d) dry-run schema rejection → config-invalid, no real write.
    scenario("dryrun-reject", Some("config-invalid"), |sandbox| {
        let err = adapter()
            .write(
                exe(),
                "models.providers.acme",
                r#"{"api":"openai-completions"}"#,
                WriteMode::Merge,
            )
            .expect_err("simulated schema rejection");
        assert_eq!(err.code, "openclaw-config-invalid");
        assert!(err.message.contains("simulated schema rejection"));
        let lines = sandbox.captured();
        assert_eq!(lines.len(), 1, "only the dry-run ran");
        assert!(lines[0].contains(&"--dry-run".to_string()));
        assert!(
            !sandbox.dir.join("openclaw.json").is_file(),
            "no state file may be created"
        );
    });

    // e) invalid JSON value → config-invalid, no real write.
    scenario("invalid-json", None, |sandbox| {
        let err = adapter()
            .write(
                exe(),
                "models.providers.acme",
                "{not json",
                WriteMode::Merge,
            )
            .expect_err("invalid JSON");
        assert_eq!(err.code, "openclaw-config-invalid");
        let lines = sandbox.captured();
        assert_eq!(lines.len(), 1, "only the dry-run ran");
        assert!(
            !sandbox.dir.join("openclaw.json").is_file(),
            "no state file may be created"
        );
    });

    // f) provider delete: exact unset argv, sibling preserved.
    scenario("delete-provider", None, |sandbox| {
        sandbox.seed(serde_json::json!({
            "models": {"providers": {"acme": {"api": "x"}, "beta": {"api": "y"}}},
            "agents": {"defaults": {}},
            "secrets": {"providers": {}}
        }));
        adapter()
            .unset(exe(), "models.providers.acme")
            .expect("unset");
        let lines = sandbox.captured();
        assert_eq!(lines.len(), 2, "dry-run then commit");
        assert_eq!(
            lines[0],
            vec![
                "config",
                "unset",
                "models.providers.acme",
                "--dry-run",
                "--json"
            ]
        );
        assert_eq!(lines[1], vec!["config", "unset", "models.providers.acme"]);
        let state = sandbox.state();
        assert!(state["models"]["providers"].get("acme").is_none());
        assert!(state["models"]["providers"].get("beta").is_some());
    });

    // g) unset of a missing target → read-failed, no real unset.
    scenario("unset-missing", None, |sandbox| {
        sandbox.seed(serde_json::json!({
            "models": {"providers": {"beta": {"api": "y"}}},
            "agents": {"defaults": {}},
            "secrets": {"providers": {}}
        }));
        let err = adapter()
            .unset(exe(), "models.providers.gone")
            .expect_err("missing target");
        assert_eq!(err.code, "openclaw-config-read-failed");
        let lines = sandbox.captured();
        assert_eq!(lines.len(), 1, "no real unset after a failed dry-run");
        assert!(lines[0].contains(&"--dry-run".to_string()));
        assert!(sandbox.state()["models"]["providers"].get("beta").is_some());
    });

    // h) models list reports the configured models.
    scenario("models-list", None, |sandbox| {
        sandbox.seed(serde_json::json!({
            "models": {"providers": {"acme": {
                "models": [
                    {"id": "m1", "name": "M1", "reasoning": true, "contextWindow": 128000,
                     "compat": {"supportsReasoningEffort": true, "supportedReasoningEfforts": ["low", "high"]}},
                    {"id": "m2", "reasoning": false}
                ]
            }}},
            "agents": {"defaults": {}},
            "secrets": {"providers": {}}
        }));
        let rows = adapter().read_models(exe()).expect("models list");
        assert_eq!(rows.len(), 2);
        let m1 = rows.iter().find(|row| row.model == "m1").unwrap();
        assert_eq!(m1.full, "acme/m1");
        assert!(m1.reasoning);
        assert_eq!(m1.context_tokens, Some(128000));
        assert_eq!(
            m1.supported_reasoning_efforts.as_deref(),
            Some([ThinkingLevel::Low, ThinkingLevel::High].as_slice())
        );
        let m2 = rows.iter().find(|row| row.model == "m2").unwrap();
        assert!(!m2.reasoning);
    });

    // i) models set: valid ref updates agents.defaults.model.
    scenario("models-set", None, |sandbox| {
        sandbox.seed(serde_json::json!({
            "models": {"providers": {"acme": {"models": [{"id": "m1"}]}}},
            "agents": {"defaults": {}},
            "secrets": {"providers": {}}
        }));
        adapter()
            .set_default_model(exe(), "acme/m1")
            .expect("models set");
        let lines = sandbox.captured();
        assert_eq!(lines, vec![vec!["models", "set", "acme/m1"]]);
        assert_eq!(
            sandbox.state()["agents"]["defaults"]["model"]["primary"],
            "acme/m1"
        );
        assert_eq!(
            adapter().read_default_model(exe()).expect("read back"),
            Some("acme/m1".to_string())
        );
    });

    // j) models set: unknown ref fails, config unchanged.
    scenario("models-set-bad", None, |sandbox| {
        sandbox.seed(serde_json::json!({
            "models": {"providers": {"acme": {"models": [{"id": "m1"}]}}},
            "agents": {"defaults": {}},
            "secrets": {"providers": {}}
        }));
        let err = adapter()
            .set_default_model(exe(), "acme/nope")
            .expect_err("unknown ref");
        assert_eq!(err.code, "openclaw-config-write-failed");
        assert!(sandbox.state()["agents"]["defaults"].get("model").is_none());
    });

    // k) thinking default round-trip.
    scenario("thinking-default", None, |sandbox| {
        sandbox.seed(serde_json::json!({
            "models": {"providers": {}},
            "agents": {"defaults": {"thinkingDefault": "high"}},
            "secrets": {"providers": {}}
        }));
        assert_eq!(
            adapter().read_thinking_default(exe()).expect("read"),
            Some(ThinkingLevel::High)
        );
        adapter()
            .write(
                exe(),
                "agents.defaults.thinkingDefault",
                r#""xhigh""#,
                WriteMode::Plain,
            )
            .expect("write");
        assert_eq!(
            adapter().read_thinking_default(exe()).expect("read again"),
            Some(ThinkingLevel::XHigh)
        );
        let lines = sandbox.captured();
        assert!(lines.iter().any(|line| {
            line.get(2).map(|p| p.as_str()) == Some("agents.defaults.thinkingDefault")
                && line.get(3).map(|v| v.as_str()) == Some(r#""xhigh""#)
        }));
    });

    // l) API key SecretRef + declaration round-trips.
    scenario("secret-ref", None, |_sandbox| {
        adapter()
            .write(
                exe(),
                "models.providers.acme",
                r#"{"api":"openai-completions","models":[{"id":"m1"}]}"#,
                WriteMode::Merge,
            )
            .expect("provider create");
        adapter()
            .write(
                exe(),
                "models.providers.acme.apiKey",
                r#"{"source":"exec","provider":"clawdesk","id":"providers/acme/apiKey"}"#,
                WriteMode::Plain,
            )
            .expect("ref write");
        let providers = adapter().read_providers(exe()).expect("providers");
        assert_eq!(providers[0].api_key, ProviderApiKey::Managed);
        let raw = adapter()
            .read_raw(exe(), "models.providers.acme.apiKey")
            .expect("raw read")
            .expect("ref present");
        let value: serde_json::Value = serde_json::from_str(&raw).expect("ref json");
        assert_eq!(value["provider"], "clawdesk");

        let declaration = r#"{"source":"exec","command":"C:\\tools\\resolver.exe","timeoutMs":5000,"jsonOnly":true}"#;
        adapter()
            .write(
                exe(),
                "secrets.providers.clawdesk",
                declaration,
                WriteMode::Plain,
            )
            .expect("declaration write");
        let raw = adapter()
            .read_raw(exe(), "secrets.providers.clawdesk")
            .expect("declaration read")
            .expect("declaration present");
        let value: serde_json::Value = serde_json::from_str(&raw).expect("declaration json");
        assert_eq!(value["source"], "exec");

        // Clearing the ref subpath returns the provider to Absent.
        adapter()
            .unset(exe(), "models.providers.acme.apiKey")
            .expect("unset ref");
        let providers = adapter().read_providers(exe()).expect("providers again");
        assert_eq!(providers[0].api_key, ProviderApiKey::Absent);
    });

    // m) protected path: an entry-removing replacement without `--replace`
    // is rejected (state unchanged, no real write).
    scenario("protected-path-reject", None, |sandbox| {
        sandbox.seed(serde_json::json!({
            "models": {"providers": {"acme": {"api": "x"}, "beta": {"api": "y"}}},
            "agents": {"defaults": {}},
            "secrets": {"providers": {}}
        }));
        let err = adapter()
            .write(
                exe(),
                "models.providers",
                r#"{"acme":{"api":"x"}}"#,
                WriteMode::Plain,
            )
            .expect_err("protected path must reject without --replace");
        assert_eq!(err.code, "openclaw-config-invalid");
        assert!(err.message.contains("protected path"));
        let lines = sandbox.captured();
        assert_eq!(lines.len(), 1, "only the dry-run ran");
        let state = sandbox.state();
        assert!(
            state["models"]["providers"].get("beta").is_some(),
            "state must be unchanged"
        );
    });

    // n) protected path: the same removal is honored with `--replace`.
    scenario("protected-path-replace", None, |sandbox| {
        sandbox.seed(serde_json::json!({
            "models": {"providers": {"acme": {"api": "x"}, "beta": {"api": "y"}}},
            "agents": {"defaults": {}},
            "secrets": {"providers": {}}
        }));
        adapter()
            .write(
                exe(),
                "models.providers",
                r#"{"acme":{"api":"x"}}"#,
                WriteMode::Replace,
            )
            .expect("--replace must be honored");
        let state = sandbox.state();
        assert!(state["models"]["providers"].get("acme").is_some());
        assert!(state["models"]["providers"].get("beta").is_none());
    });
}

/// Full `set_api_key` flow over the real config adapter + fake CLI: the key
/// value must never reach argv (capture verification, contract §4), the
/// config receives only the exec SecretRef, and the value lands in the
/// ClawDesk secret store only.
#[test]
fn api_key_set_flow_keeps_key_out_of_argv() {
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
        fn version_from_entry(
            &self,
            _node: &Path,
            _entry: &Path,
        ) -> Result<OpenClawVersion, AppError> {
            unimplemented!()
        }
        fn gateway_status(&self, _exe: &Path) -> Result<GatewayStatus, AppError> {
            unimplemented!()
        }
        fn update_state(&self, _exe: &Path) -> Result<UpdateState, AppError> {
            unimplemented!()
        }
    }

    scenario("api-key-argv", None, |sandbox| {
        sandbox.seed(serde_json::json!({
            "models": {"providers": {"acme": {"api": "openai-completions", "models": [{"id": "m1"}]}}},
            "agents": {"defaults": {}},
            "secrets": {"providers": {}}
        }));
        let store = SecretStore::new(sandbox.dir.join("secrets"), Arc::new(WindowsDpapi::new()));
        let service = ApiKeyService::new(
            Arc::new(DetectsFakeCli),
            Arc::new(OpenClawConfigAdapter::new(Arc::new(ProcessRunner))),
            Arc::new(store),
            PathBuf::from(RESOLVER),
        );
        service
            .set_api_key("acme", "sk-fake-argv-12345")
            .expect("set should succeed");

        // No captured argv row may contain the key value (S3/S7).
        for row in sandbox.captured() {
            assert!(!row.join(" ").contains("sk-fake-argv-12345"));
        }
        // The config carries only the exec SecretRef (never the value).
        let state = sandbox.state();
        assert_eq!(
            state["models"]["providers"]["acme"]["apiKey"]["provider"],
            "clawdesk"
        );
        assert_eq!(
            state["models"]["providers"]["acme"]["apiKey"]["id"],
            "providers/acme/apiKey"
        );
        // The value exists only in the ClawDesk secret store.
        let value = SecretStore::new(sandbox.dir.join("secrets"), Arc::new(WindowsDpapi::new()))
            .get("providers/acme/apiKey")
            .expect("store read")
            .expect("key registered");
        assert_eq!(value, "sk-fake-argv-12345");
    });
}

// --- secret resolver (real binary, sandboxed store) --------------------------------

fn run_resolver(store_root: &Path, request_body: &str) -> Result<ProcessOutput, ProcessError> {
    let mut request = ProcessRequest::new(RESOLVER, Vec::new(), TIMEOUT);
    request.env.push((
        "CLAWDESK_SECRETS_ROOT".to_string(),
        store_root.to_string_lossy().into_owned(),
    ));
    request.env.push((
        "CLAWDESK_RESOLVER_REQUEST".to_string(),
        request_body.to_string(),
    ));
    ProcessRunner.run(&request)
}

#[test]
fn resolver_returns_stored_value() {
    let root = target_dir().join("clawdesk_test_resolver").join(format!(
        "value-{}-{}",
        std::process::id(),
        stamp()
    ));
    fs::create_dir_all(&root).expect("create resolver sandbox");
    let store = SecretStore::new(root.clone(), Arc::new(WindowsDpapi::new()));
    store
        .set("providers/acme/apiKey", "sk-fake-resolver-12345")
        .expect("seed store");
    let request =
        "{\"protocolVersion\":1,\"provider\":\"clawdesk\",\"ids\":[\"providers/acme/apiKey\"]}";
    let output = run_resolver(&root, request).expect("resolver should run");
    assert_eq!(output.exit_code, 0, "stderr: {}", output.stderr);
    let response: serde_json::Value = serde_json::from_str(&output.stdout).expect("response json");
    assert_eq!(response["protocolVersion"], 1);
    // The resolver returned the stored secret (protocol design), and the
    // S8 output mask redacted it in transit — the masked form proves the
    // round-trip without ever printing the value.
    assert_eq!(response["values"]["providers/acme/apiKey"], "****");
    assert!(!output.stdout.contains("sk-fake-resolver-12345"));
    assert!(!output.stderr.contains("sk-fake-resolver-12345"));
}

#[test]
fn resolver_missing_id_is_not_found_error() {
    let root = target_dir().join("clawdesk_test_resolver").join(format!(
        "missing-{}-{}",
        std::process::id(),
        stamp()
    ));
    fs::create_dir_all(&root).expect("create resolver sandbox");
    let request =
        "{\"protocolVersion\":1,\"provider\":\"clawdesk\",\"ids\":[\"providers/gone/apiKey\"]}";
    let output = run_resolver(&root, request).expect("resolver should run");
    assert_eq!(output.exit_code, 0, "per-id errors still exit 0");
    let response: serde_json::Value = serde_json::from_str(&output.stdout).expect("response json");
    assert_eq!(
        response["errors"]["providers/gone/apiKey"]["code"],
        "NOT_FOUND"
    );
    assert!(response["values"].as_object().unwrap().is_empty());
}

#[test]
fn resolver_rejects_unknown_provider_and_bad_protocol() {
    let root = target_dir().join("clawdesk_test_resolver").join(format!(
        "reject-{}-{}",
        std::process::id(),
        stamp()
    ));
    fs::create_dir_all(&root).expect("create resolver sandbox");
    let request = "{\"protocolVersion\":1,\"provider\":\"other\",\"ids\":[\"providers/a/apiKey\"]}";
    let output = run_resolver(&root, request).expect("resolver should run");
    assert_eq!(output.exit_code, 65);
    assert!(output.stdout.trim().is_empty(), "no protocol response");

    let request =
        "{\"protocolVersion\":2,\"provider\":\"clawdesk\",\"ids\":[\"providers/a/apiKey\"]}";
    let output = run_resolver(&root, request).expect("resolver should run");
    assert_eq!(output.exit_code, 65);

    let output = run_resolver(&root, "not json at all").expect("resolver should run");
    assert_eq!(output.exit_code, 65);
}
