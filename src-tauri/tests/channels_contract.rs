//! Phase 6 contract tests: real `ProcessRunner` + fake `openclaw` CLI with a
//! per-test state sandbox (S5: fake CLI only, no real OpenClaw, no system
//! mutation — sandboxes live under the cargo target dir).
//!
//! Adapter/service-driven scenarios run inside serialized sub-scenarios
//! because the fake receives its sandbox via inherited process environment.
//! The token value (S3/S7) is asserted to appear 0 times in the captured
//! argv and in the config state file.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use clawdesk_lib::application::{ChannelService, ChannelTokenService};
use clawdesk_lib::domain::models::channels::ChannelTokenState;
use clawdesk_lib::domain::models::openclaw::{GatewayStatus, OpenClawVersion, UpdateState};
use clawdesk_lib::domain::models::ExecutableDetection;
use clawdesk_lib::domain::ports::openclaw::OpenClawPort;
use clawdesk_lib::domain::ports::openclaw_channels::OpenClawChannelsPort;
use clawdesk_lib::domain::ports::openclaw_plugin_install::OpenClawPluginInstallPort;
use clawdesk_lib::domain::ports::process::{
    ProcessError, ProcessOutput, ProcessPort, ProcessRequest,
};
use clawdesk_lib::domain::ports::secrets::SecretStorePort;
use clawdesk_lib::error::AppError;
use clawdesk_lib::infrastructure::openclaw::{
    OpenClawChannelsAdapter, OpenClawConfigAdapter, OpenClawPluginInstallAdapter,
    OpenClawPluginsAdapter,
};
use clawdesk_lib::infrastructure::process::ProcessRunner;

const FAKE_OPENCLAW: &str = env!("CARGO_BIN_EXE_clawdesk-fake-openclaw");
const TIMEOUT: Duration = Duration::from_secs(10);
/// Distinctive, non-`sk-` values so masking cannot hide a leak: if the token
/// ever reaches argv/config/state, the assertions below still see it.
const TEST_TOKEN: &str = "clawdesk-test-channel-token-1234567890";
const TEST_BOT_TOKEN: &str = "clawdesk-test-telegram-bot-9876543210";

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

    /// S3/S7: the token value must never appear in the captured argv.
    fn assert_no_token_in_capture(&self, token: &str) {
        let body = fs::read_to_string(&self.capture).expect("capture file exists");
        assert!(!body.contains(token), "token leaked into argv");
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

fn channels_service() -> ChannelService {
    let process: Arc<dyn ProcessPort> = Arc::new(ProcessRunner);
    ChannelService::new(
        Arc::new(DetectsFakeCli),
        Arc::new(OpenClawConfigAdapter::new(Arc::clone(&process))),
        Arc::new(OpenClawPluginsAdapter::new(Arc::clone(&process))),
        Arc::new(OpenClawPluginInstallAdapter::new(Arc::clone(&process))),
        Arc::new(OpenClawChannelsAdapter::new(process)),
    )
}

fn channels_service_no_openclaw() -> ChannelService {
    let process: Arc<dyn ProcessPort> = Arc::new(ProcessRunner);
    ChannelService::new(
        Arc::new(NoOpenClaw),
        Arc::new(OpenClawConfigAdapter::new(Arc::clone(&process))),
        Arc::new(OpenClawPluginsAdapter::new(Arc::clone(&process))),
        Arc::new(OpenClawPluginInstallAdapter::new(Arc::clone(&process))),
        Arc::new(OpenClawChannelsAdapter::new(process)),
    )
}

// --- secret store fakes (sandbox-local) -----------------------------------------

/// File-backed secret store fake (sandbox-local): one blob per key id plus an
/// op log. Values live only in the sandbox blob files (never in argv/config).
struct FileSecretStore {
    dir: PathBuf,
}

impl FileSecretStore {
    fn new(dir: PathBuf) -> Self {
        let store = Self { dir };
        let _ = fs::create_dir_all(&store.dir);
        store
    }

    fn blob(&self, key_id: &str) -> PathBuf {
        self.dir
            .join(format!("secrets-{}.bin", key_id.replace('/', "-")))
    }

    fn log_op(&self, op: &str) {
        let path = self.dir.join("store-ops.jsonl");
        let _ = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .and_then(|mut f| f.write_all(format!("{op}\n").as_bytes()));
    }

    fn ops(&self) -> Vec<String> {
        fs::read_to_string(self.dir.join("store-ops.jsonl"))
            .map(|body| body.lines().map(str::to_string).collect())
            .unwrap_or_default()
    }
}

impl SecretStorePort for FileSecretStore {
    fn set(&self, key_id: &str, value: &str) -> Result<(), AppError> {
        self.log_op(&format!("set:{key_id}"));
        fs::write(self.blob(key_id), value)
            .map_err(|err| AppError::secret_store_unavailable(err.to_string()))
    }
    fn get(&self, key_id: &str) -> Result<Option<String>, AppError> {
        match fs::read_to_string(self.blob(key_id)) {
            Ok(body) => Ok(Some(body)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(AppError::secret_store_unavailable(err.to_string())),
        }
    }
    fn delete(&self, key_id: &str) -> Result<(), AppError> {
        let path = self.blob(key_id);
        if !path.exists() {
            return Err(AppError::secret_store_unavailable("not registered"));
        }
        self.log_op(&format!("delete:{key_id}"));
        fs::remove_file(&path).map_err(|err| AppError::secret_store_unavailable(err.to_string()))
    }
    fn contains(&self, key_id: &str) -> bool {
        self.blob(key_id).exists()
    }
    fn list_key_ids(&self) -> Vec<String> {
        fs::read_dir(&self.dir)
            .map(|entries| {
                entries
                    .filter_map(|entry| entry.ok())
                    .filter(|entry| {
                        let file_name = entry.file_name();
                        let name = file_name.to_string_lossy();
                        name.starts_with("secrets-") && name.ends_with(".bin")
                    })
                    .map(|entry| {
                        let name = entry.file_name().to_string_lossy().into_owned();
                        name["secrets-".len()..name.len() - ".bin".len()].replace('-', "/")
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// A secret store that fails on every write (injects the DPAPI failure).
struct FailingSecretStore;

impl SecretStorePort for FailingSecretStore {
    fn set(&self, _key_id: &str, _value: &str) -> Result<(), AppError> {
        Err(AppError::secret_store_unavailable("injected DPAPI failure"))
    }
    fn get(&self, _key_id: &str) -> Result<Option<String>, AppError> {
        Ok(None)
    }
    fn delete(&self, _key_id: &str) -> Result<(), AppError> {
        Err(AppError::secret_store_unavailable("injected DPAPI failure"))
    }
    fn contains(&self, _key_id: &str) -> bool {
        false
    }
    fn list_key_ids(&self) -> Vec<String> {
        Vec::new()
    }
}

fn token_service(sandbox: &Sandbox) -> ChannelTokenService {
    let process: Arc<dyn ProcessPort> = Arc::new(ProcessRunner);
    ChannelTokenService::new(
        Arc::new(DetectsFakeCli),
        Arc::new(OpenClawConfigAdapter::new(Arc::clone(&process))),
        Arc::new(FileSecretStore::new(sandbox.dir.join("secrets"))),
        sandbox.dir.join("clawdesk-secret-resolver.exe"),
    )
}

fn token_service_failing_store(sandbox: &Sandbox) -> ChannelTokenService {
    let process: Arc<dyn ProcessPort> = Arc::new(ProcessRunner);
    ChannelTokenService::new(
        Arc::new(DetectsFakeCli),
        Arc::new(OpenClawConfigAdapter::new(Arc::clone(&process))),
        Arc::new(FailingSecretStore),
        sandbox.dir.join("clawdesk-secret-resolver.exe"),
    )
}

/// The ClawDesk exec SecretRef (the only token artifact allowed in config
/// state; the value itself lives only in the secret store).
fn clawdesk_ref(channel: &str) -> serde_json::Value {
    let id = match channel {
        "discord" => "channels/discord/token",
        "telegram" => "channels/telegram/botToken",
        _ => unreachable!("supported channels only"),
    };
    serde_json::json!({ "source": "exec", "provider": "clawdesk", "id": id })
}

// --- get-channels / get-channel-config ---------------------------------------------

#[test]
fn get_channels_over_fake_cli() {
    // a) Explicit list rows + reachable gateway: supported channels only,
    //    status merged, exact argv.
    scenario("channels-read-rows", None, |sandbox| {
        sandbox.seed(serde_json::json!({
            "channels": [
                {"id":"discord","installed":true,"configured":true,"enabled":false},
                {"id":"telegram","installed":true,"configured":true,"enabled":true},
                {"id":"slack","installed":true,"configured":true,"enabled":true}
            ],
            "channelsStatus": {
                "gatewayReachable": true,
                "channels": [
                    {"id":"discord","state":"connected"},
                    {"id":"telegram"}
                ]
            }
        }));
        let service = channels_service();
        let overview = service.get_channels().expect("overview");
        assert!(overview.gateway_reachable);
        assert_eq!(overview.channels.len(), 2, "only supported channels");
        let discord = &overview.channels[0];
        assert_eq!(discord.id, "discord");
        assert!(discord.installed && discord.configured && !discord.enabled);
        assert_eq!(discord.runtime_state.as_deref(), Some("connected"));
        let telegram = &overview.channels[1];
        assert_eq!(telegram.id, "telegram");
        assert!(telegram.installed && telegram.configured && telegram.enabled);
        assert_eq!(telegram.runtime_state, None);
        assert_eq!(
            sandbox.captured(),
            vec![
                vec!["channels", "list", "--all", "--json"],
                vec!["channels", "status", "--json"],
            ]
        );
    });

    // b) Derived rows (config sections) without a status section:
    //    config-only state (no "connected" guess).
    scenario("channels-read-derived", None, |sandbox| {
        sandbox.seed(serde_json::json!({
            "channels": {
                "telegram": {"enabled": true, "botToken": clawdesk_ref("telegram")}
            },
            "plugins": {"catalog": {"@openclaw/discord": {"name":"discord"}}}
        }));
        let overview = channels_service().get_channels().expect("overview");
        assert!(!overview.gateway_reachable, "absent status → config-only");
        let discord = &overview.channels[0];
        assert!(discord.installed && !discord.configured && !discord.enabled);
        let telegram = &overview.channels[1];
        assert!(telegram.installed && telegram.configured && telegram.enabled);
        assert_eq!(discord.runtime_state, None);
        assert_eq!(telegram.runtime_state, None);
    });
}

#[test]
fn get_channel_config_over_fake_cli() {
    scenario("channel-config-read", None, |sandbox| {
        sandbox.seed(serde_json::json!({
            "channels": {
                "discord": {
                    "enabled": true,
                    "token": clawdesk_ref("discord"),
                    "dmPolicy": "pairing",
                    "allowFrom": ["1234567890"],
                    "groupPolicy": "allowlist"
                },
                "telegram": {"botToken": clawdesk_ref("telegram"), "enabled": false}
            }
        }));
        let service = channels_service();
        let view = service.get_channel_config("discord").expect("discord view");
        assert_eq!(view.enabled, Some(true));
        assert_eq!(view.token_state, ChannelTokenState::Managed);
        assert_eq!(view.dm_policy.as_deref(), Some("pairing"));
        assert_eq!(view.allow_from, vec!["1234567890".to_string()]);
        assert_eq!(view.group_policy.as_deref(), Some("allowlist"));
        // Telegram uses the `botToken` field name — same classification.
        let view = service
            .get_channel_config("telegram")
            .expect("telegram view");
        assert_eq!(view.token_state, ChannelTokenState::Managed);
        assert_eq!(view.enabled, Some(false));
        assert_eq!(view.dm_policy, None);
        assert!(view.allow_from.is_empty());
        assert_eq!(view.group_policy, None);
        assert_eq!(
            sandbox.captured(),
            vec![
                vec!["config", "get", "channels.discord", "--json"],
                vec!["config", "get", "channels.telegram", "--json"],
            ]
        );
    });
}

// --- connect-channel ------------------------------------------------------------------

#[test]
fn connect_discord_over_fake_cli() {
    scenario("connect-discord", None, |sandbox| {
        sandbox.seed(serde_json::json!({
            "channels": {"discord": {"token": clawdesk_ref("discord")}}
        }));
        let service = channels_service();
        service.connect_channel("discord").expect("connect");
        let lines = sandbox.captured();
        assert_eq!(
            lines,
            vec![
                vec!["config", "get", "channels.discord.token", "--json"],
                vec!["plugins", "list", "--json"],
                vec!["plugins", "install", "@openclaw/discord"],
                vec!["plugins", "list", "--json"],
                vec![
                    "config",
                    "set",
                    "channels.discord.enabled",
                    "true",
                    "--strict-json",
                    "--dry-run",
                    "--json"
                ],
                vec![
                    "config",
                    "set",
                    "channels.discord.enabled",
                    "true",
                    "--strict-json"
                ],
            ]
        );
        let state = sandbox.state();
        assert!(
            state["plugins"]["catalog"]["@openclaw/discord"].is_object(),
            "plugin installed"
        );
        assert_eq!(state["channels"]["discord"]["enabled"], true);
        // Reconnect: the plugin is now installed → no second install.
        service.connect_channel("discord").expect("second connect");
        let lines = sandbox.captured();
        let installs = lines
            .iter()
            .filter(|line| {
                line.first().map(|s| s.as_str()) == Some("plugins")
                    && line.get(1).map(|s| s.as_str()) == Some("install")
            })
            .count();
        assert_eq!(
            installs, 1,
            "install runs exactly once across both connects"
        );
        // The derived list now reports configured + enabled + installed.
        let overview = service.get_channels().expect("overview");
        let discord = overview
            .channels
            .iter()
            .find(|row| row.id == "discord")
            .expect("discord row");
        assert!(discord.installed && discord.configured && discord.enabled);
    });
}

#[test]
fn connect_telegram_over_fake_cli() {
    scenario("connect-telegram", None, |sandbox| {
        sandbox.seed(serde_json::json!({
            "channels": {"telegram": {"botToken": clawdesk_ref("telegram")}}
        }));
        let service = channels_service();
        service.connect_channel("telegram").expect("connect");
        // No plugin step for Telegram — token check + enabled write only.
        assert_eq!(
            sandbox.captured(),
            vec![
                vec!["config", "get", "channels.telegram.botToken", "--json"],
                vec![
                    "config",
                    "set",
                    "channels.telegram.enabled",
                    "true",
                    "--strict-json",
                    "--dry-run",
                    "--json"
                ],
                vec![
                    "config",
                    "set",
                    "channels.telegram.enabled",
                    "true",
                    "--strict-json"
                ],
            ]
        );
        assert_eq!(sandbox.state()["channels"]["telegram"]["enabled"], true);
    });
}

#[test]
fn connect_without_managed_token_fails_closed() {
    // a) No token at all.
    scenario("connect-no-token", None, |sandbox| {
        sandbox.seed(serde_json::json!({}));
        let err = channels_service()
            .connect_channel("discord")
            .expect_err("no token");
        assert_eq!(err.code, "channel-token-not-found");
        assert_eq!(
            sandbox.captured(),
            vec![vec!["config", "get", "channels.discord.token", "--json"]],
            "only the token check ran — no install/enable"
        );
    });

    // b) An externally managed token (plaintext): same stable error, zero
    //    mutation calls, and the external value never leaks into the error.
    scenario("connect-external-token", None, |sandbox| {
        sandbox.seed(serde_json::json!({
            "channels": {"discord": {"token": "external-plaintext-token"}}
        }));
        let err = channels_service()
            .connect_channel("discord")
            .expect_err("external token");
        assert_eq!(err.code, "channel-token-not-found");
        assert!(
            !err.message.contains("external-plaintext-token"),
            "external value must not leak: {}",
            err.message
        );
        assert_eq!(
            sandbox.captured(),
            vec![vec!["config", "get", "channels.discord.token", "--json"]]
        );
    });
}

// --- policy writes -------------------------------------------------------------------

#[test]
fn set_dm_access_over_fake_cli() {
    // a) Happy path: dmPolicy → allowFrom, exact argv with --replace.
    scenario("dm-access-set", None, |sandbox| {
        sandbox.seed(serde_json::json!({}));
        let service = channels_service();
        service
            .set_dm_access("discord", "allowlist", &["1234567890".into()])
            .expect("set dm access");
        assert_eq!(
            sandbox.captured(),
            vec![
                vec![
                    "config",
                    "set",
                    "channels.discord.dmPolicy",
                    "\"allowlist\"",
                    "--strict-json",
                    "--dry-run",
                    "--json"
                ],
                vec![
                    "config",
                    "set",
                    "channels.discord.dmPolicy",
                    "\"allowlist\"",
                    "--strict-json"
                ],
                vec![
                    "config",
                    "set",
                    "channels.discord.allowFrom",
                    r#"["1234567890"]"#,
                    "--strict-json",
                    "--replace",
                    "--dry-run",
                    "--json"
                ],
                vec![
                    "config",
                    "set",
                    "channels.discord.allowFrom",
                    r#"["1234567890"]"#,
                    "--strict-json",
                    "--replace"
                ],
            ]
        );
        let state = sandbox.state();
        assert_eq!(state["channels"]["discord"]["dmPolicy"], "allowlist");
        assert_eq!(
            state["channels"]["discord"]["allowFrom"],
            serde_json::json!(["1234567890"])
        );
    });

    // b) Invalid combinations: the stable validation codes, 0 CLI calls.
    scenario("dm-access-invalid", None, |sandbox| {
        sandbox.seed(serde_json::json!({}));
        let service = channels_service();
        let cases: Vec<(&str, &str, Vec<String>, &str)> = vec![
            ("slack", "pairing", vec!["*".into()], "channel-id-invalid"),
            ("discord", "", vec!["*".into()], "dm-policy-invalid"),
            ("discord", "Pairing", vec!["*".into()], "dm-policy-invalid"),
            ("discord", "allowlist", vec![], "dm-access-inconsistent"),
            (
                "discord",
                "open",
                vec!["123".into()],
                "dm-access-inconsistent",
            ),
            (
                "discord",
                "pairing",
                vec!["abc".into()],
                "allow-from-entry-invalid",
            ),
            (
                "discord",
                "pairing",
                vec!["".into()],
                "allow-from-entry-invalid",
            ),
        ];
        for (channel, policy, entries, expected) in &cases {
            let err = service
                .set_dm_access(channel, policy, entries)
                .expect_err("must be rejected");
            assert_eq!(err.code, *expected, "{channel}/{policy:?}/{entries:?}");
        }
        assert!(!sandbox.capture.exists(), "no CLI call may be captured");
        let state = sandbox.state();
        assert!(state.get("channels").is_none(), "state must be unchanged");
    });
}

#[test]
fn set_group_policy_over_fake_cli() {
    scenario("group-policy-set", None, |sandbox| {
        sandbox.seed(serde_json::json!({}));
        let service = channels_service();
        service
            .set_group_policy("telegram", "allowlist")
            .expect("set");
        assert_eq!(
            sandbox.captured(),
            vec![
                vec![
                    "config",
                    "set",
                    "channels.telegram.groupPolicy",
                    "\"allowlist\"",
                    "--strict-json",
                    "--dry-run",
                    "--json"
                ],
                vec![
                    "config",
                    "set",
                    "channels.telegram.groupPolicy",
                    "\"allowlist\"",
                    "--strict-json"
                ],
            ]
        );
        assert_eq!(
            sandbox.state()["channels"]["telegram"]["groupPolicy"],
            "allowlist"
        );
        let err = service
            .set_group_policy("discord", "Open")
            .expect_err("invalid enum");
        assert_eq!(err.code, "group-policy-invalid");
        let err = service
            .set_group_policy("slack", "open")
            .expect_err("invalid channel");
        assert_eq!(err.code, "channel-id-invalid");
    });
}

#[test]
fn set_channel_enabled_over_fake_cli() {
    scenario("channel-enabled-toggle", None, |sandbox| {
        sandbox.seed(serde_json::json!({
            "channels": {"discord": {"token": clawdesk_ref("discord"), "enabled": true}}
        }));
        let service = channels_service();
        service
            .set_channel_enabled("discord", false)
            .expect("disable");
        assert_eq!(
            sandbox.captured(),
            vec![
                vec![
                    "config",
                    "set",
                    "channels.discord.enabled",
                    "false",
                    "--strict-json",
                    "--dry-run",
                    "--json"
                ],
                vec![
                    "config",
                    "set",
                    "channels.discord.enabled",
                    "false",
                    "--strict-json"
                ],
            ]
        );
        let state = sandbox.state();
        assert_eq!(state["channels"]["discord"]["enabled"], false);
        assert_eq!(
            state["channels"]["discord"]["token"],
            clawdesk_ref("discord"),
            "disable keeps the token"
        );
        let err = service
            .set_channel_enabled("slack", true)
            .expect_err("invalid channel");
        assert_eq!(err.code, "channel-id-invalid");
    });
}

// --- pairing ---------------------------------------------------------------------------

#[test]
fn pairing_over_fake_cli() {
    scenario("pairing-list-approve", None, |sandbox| {
        sandbox.seed(serde_json::json!({
            "pairing": {"discord": [
                {"code":"AB12CD34","sender":"user-1"},
                {"sender":"row without code is dropped"}
            ]}
        }));
        let service = channels_service();
        let requests = service.list_pairing_requests("discord").expect("list");
        assert_eq!(requests.len(), 1, "code-less row dropped");
        assert_eq!(requests[0].code, "AB12CD34");
        assert_eq!(requests[0].sender.as_deref(), Some("user-1"));
        service
            .approve_pairing("discord", "AB12CD34")
            .expect("approve");
        assert_eq!(
            sandbox.captured(),
            vec![
                vec!["pairing", "list", "discord", "--json"],
                vec!["pairing", "approve", "discord", "AB12CD34"],
            ]
        );
        // Only the approved (code-bearing) row is removed.
        assert_eq!(
            sandbox.state()["pairing"]["discord"],
            serde_json::json!([{"sender": "row without code is dropped"}])
        );
        // Approving again: the code is gone → the CLI failure envelope.
        let err = service
            .approve_pairing("discord", "AB12CD34")
            .expect_err("unknown code");
        assert_eq!(err.code, "openclaw-pairing-failed");
        assert!(err.message.contains("pairing code not found"));
    });

    // Invalid channel/code: validation before any process run (0 CLI calls).
    scenario("pairing-validation", None, |sandbox| {
        sandbox.seed(serde_json::json!({}));
        let service = channels_service();
        for channel in ["slack", "", "Discord"] {
            assert_eq!(
                service.list_pairing_requests(channel).unwrap_err().code,
                "channel-id-invalid",
                "{channel:?}"
            );
            assert_eq!(
                service
                    .approve_pairing(channel, "ABCD1234")
                    .unwrap_err()
                    .code,
                "channel-id-invalid",
                "{channel:?}"
            );
        }
        let long_code = "x".repeat(65);
        for code in ["", "abc", "1 2", long_code.as_str()] {
            assert_eq!(
                service.approve_pairing("discord", code).unwrap_err().code,
                "pairing-code-invalid",
                "{code:?}"
            );
        }
        assert!(!sandbox.capture.exists(), "no CLI call may be captured");
    });
}

// --- channel token lifecycle (DPAPI-first, dual-surface) --------------------------------

#[test]
fn set_channel_token_over_fake_cli() {
    scenario("channel-token-set", None, |sandbox| {
        sandbox.seed(serde_json::json!({}));
        let service = token_service(sandbox);
        service
            .set_channel_token("discord", TEST_TOKEN)
            .expect("register");
        let lines = sandbox.captured();
        assert_eq!(
            lines.len(),
            6,
            "token read + declaration (read, dry, commit) + ref (dry, commit)"
        );
        assert_eq!(
            lines[0],
            vec!["config", "get", "channels.discord.token", "--json"]
        );
        assert_eq!(
            lines[1],
            vec!["config", "get", "secrets.providers.clawdesk", "--json"]
        );
        assert_eq!(
            &lines[2][..3],
            &["config", "set", "secrets.providers.clawdesk"]
        );
        assert!(lines[2].contains(&"--dry-run".to_string()));
        assert_eq!(
            &lines[3][..3],
            &["config", "set", "secrets.providers.clawdesk"]
        );
        assert!(!lines[3].contains(&"--dry-run".to_string()));
        assert_eq!(&lines[4][..3], &["config", "set", "channels.discord.token"]);
        assert!(lines[4].contains(&"--dry-run".to_string()));
        assert_eq!(&lines[5][..3], &["config", "set", "channels.discord.token"]);
        // The shared declaration shape (exec resolver).
        let declaration: serde_json::Value =
            serde_json::from_str(&lines[3][3]).expect("declaration json");
        assert_eq!(declaration["source"], "exec");
        assert_eq!(declaration["timeoutMs"], 5000);
        assert_eq!(declaration["jsonOnly"], true);
        assert!(declaration["command"]
            .as_str()
            .unwrap()
            .ends_with("clawdesk-secret-resolver.exe"));
        // The config holds only the exec SecretRef — never the value.
        let reference: serde_json::Value = serde_json::from_str(&lines[5][3]).expect("ref json");
        assert_eq!(
            reference,
            serde_json::json!({"source":"exec","provider":"clawdesk","id":"channels/discord/token"})
        );
        let state = sandbox.state();
        assert_eq!(state["channels"]["discord"]["token"], reference);
        assert_eq!(state["secrets"]["providers"]["clawdesk"], declaration);
        // The value lives only in the (fake) secret store.
        let store = FileSecretStore::new(sandbox.dir.join("secrets"));
        assert!(store.contains("channels/discord/token"));
        assert_eq!(
            store.get("channels/discord/token").expect("read blob"),
            Some(TEST_TOKEN.to_string())
        );
        assert_eq!(store.ops(), vec!["set:channels/discord/token".to_string()]);
        // S3/S7: zero token leaks in argv or config state.
        sandbox.assert_no_token_in_capture(TEST_TOKEN);
        let state_body = fs::read_to_string(sandbox.dir.join("openclaw.json")).unwrap();
        assert!(
            !state_body.contains(TEST_TOKEN),
            "token leaked into config state"
        );

        // Second channel: the declaration already matches → no declaration
        // write (only the botToken ref write).
        service
            .set_channel_token("telegram", TEST_BOT_TOKEN)
            .expect("register telegram");
        let lines = sandbox.captured();
        let telegram_lines = &lines[6..];
        assert_eq!(
            telegram_lines.len(),
            4,
            "token read + declaration read + ref (dry, commit): {telegram_lines:?}"
        );
        assert_eq!(
            telegram_lines[0],
            vec!["config", "get", "channels.telegram.botToken", "--json"]
        );
        assert!(
            !telegram_lines.iter().any(|line| {
                line.get(1).map(|s| s.as_str()) == Some("set")
                    && line.get(2).map(|s| s.as_str()) == Some("secrets.providers.clawdesk")
            }),
            "declaration must not be rewritten"
        );
        assert_eq!(
            sandbox.state()["channels"]["telegram"]["botToken"],
            clawdesk_ref("telegram")
        );
        let store = FileSecretStore::new(sandbox.dir.join("secrets"));
        assert!(store.contains("channels/telegram/botToken"));
        sandbox.assert_no_token_in_capture(TEST_TOKEN);
        sandbox.assert_no_token_in_capture(TEST_BOT_TOKEN);
        let state_body = fs::read_to_string(sandbox.dir.join("openclaw.json")).unwrap();
        assert!(!state_body.contains(TEST_BOT_TOKEN), "bot token leaked");
    });

    // b) Invalid channel / empty token: 0 process runs.
    scenario("channel-token-invalid", None, |sandbox| {
        sandbox.seed(serde_json::json!({}));
        let service = token_service(sandbox);
        let err = service
            .set_channel_token("discord", "   ")
            .expect_err("empty token");
        assert_eq!(err.code, "channel-token-invalid");
        let err = service
            .set_channel_token("slack", TEST_TOKEN)
            .expect_err("bad channel");
        assert_eq!(err.code, "channel-id-invalid");
        assert!(!sandbox.capture.exists(), "no CLI call may be captured");
    });

    // c) An externally managed token value: rejected with zero writes.
    scenario("channel-token-external-rejected", None, |sandbox| {
        sandbox.seed(serde_json::json!({
            "channels": {"discord": {"token": "external-plaintext-token"}}
        }));
        let service = token_service(sandbox);
        let err = service
            .set_channel_token("discord", TEST_TOKEN)
            .expect_err("external value");
        assert_eq!(err.code, "secret-ref-registration-failed");
        assert!(
            !err.message.contains("external-plaintext-token"),
            "external value must not leak: {}",
            err.message
        );
        assert_eq!(
            sandbox.captured(),
            vec![vec!["config", "get", "channels.discord.token", "--json"]],
            "zero store/config writes on rejection"
        );
        let state = sandbox.state();
        assert_eq!(
            state["channels"]["discord"]["token"], "external-plaintext-token",
            "the external value stays untouched"
        );
    });

    // d) DPAPI failure: zero config writes.
    scenario("channel-token-dpapi-failure", None, |sandbox| {
        sandbox.seed(serde_json::json!({}));
        let service = token_service_failing_store(sandbox);
        let err = service
            .set_channel_token("discord", TEST_TOKEN)
            .expect_err("store down");
        assert_eq!(err.code, "secret-store-unavailable");
        assert!(!err.message.contains(TEST_TOKEN), "token must not leak");
        assert_eq!(
            sandbox.captured(),
            vec![vec!["config", "get", "channels.discord.token", "--json"]],
            "only the pre-check read ran"
        );
        let state = sandbox.state();
        assert!(state.get("channels").is_none(), "zero config writes");
        assert!(
            state["secrets"]["providers"].get("clawdesk").is_none(),
            "zero declaration writes"
        );
    });
}

#[test]
fn delete_channel_token_over_fake_cli() {
    // a) Dual-surface: deleting discord keeps the shared declaration while
    //    telegram is still managed; deleting the last ref removes it.
    scenario("channel-token-delete-dual-surface", None, |sandbox| {
        sandbox.seed(serde_json::json!({}));
        let service = token_service(sandbox);
        service
            .set_channel_token("discord", TEST_TOKEN)
            .expect("set discord");
        service
            .set_channel_token("telegram", TEST_BOT_TOKEN)
            .expect("set telegram");
        service
            .delete_channel_token("discord")
            .expect("delete discord");
        let state = sandbox.state();
        assert!(
            state["secrets"]["providers"]["clawdesk"].is_object(),
            "declaration stays: telegram is still managed"
        );
        assert!(
            state["channels"]
                .get("discord")
                .and_then(|s| s.get("token"))
                .is_none(),
            "discord token ref removed"
        );
        assert!(state["channels"]["telegram"]["botToken"].is_object());
        let store = FileSecretStore::new(sandbox.dir.join("secrets"));
        assert!(!store.contains("channels/discord/token"));
        assert!(store.contains("channels/telegram/botToken"));
        assert!(!store
            .ops()
            .contains(&"delete:channels/telegram/botToken".to_string()));

        service
            .delete_channel_token("telegram")
            .expect("delete telegram");
        let state = sandbox.state();
        assert!(
            state["secrets"]["providers"].get("clawdesk").is_none(),
            "last declaration removed"
        );
        let store = FileSecretStore::new(sandbox.dir.join("secrets"));
        assert!(!store.contains("channels/telegram/botToken"));
        assert_eq!(
            store
                .ops()
                .iter()
                .filter(|op| op.starts_with("delete:"))
                .count(),
            2
        );
        sandbox.assert_no_token_in_capture(TEST_TOKEN);
        sandbox.assert_no_token_in_capture(TEST_BOT_TOKEN);
    });

    // b) Nothing registered for the channel: stable not-found, read-only.
    scenario("channel-token-delete-missing", None, |sandbox| {
        sandbox.seed(serde_json::json!({}));
        let service = token_service(sandbox);
        let err = service
            .delete_channel_token("discord")
            .expect_err("nothing registered");
        assert_eq!(err.code, "channel-token-not-found");
        assert_eq!(
            sandbox.captured(),
            vec![vec!["config", "get", "channels.discord.token", "--json"]]
        );
        let err = service
            .delete_channel_token("slack")
            .expect_err("bad channel");
        assert_eq!(err.code, "channel-id-invalid");
    });
}

#[test]
fn list_channel_tokens_reflects_store_index() {
    scenario("token-list", None, |sandbox| {
        let service = token_service(sandbox);
        assert!(service.list_channel_tokens().expect("list").is_empty());
        service
            .set_channel_token("discord", TEST_TOKEN)
            .expect("set");
        let statuses = service.list_channel_tokens().expect("list");
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].channel, "discord");
        assert!(statuses[0].registered);
    });
}

// --- failure behaviors / masking ---------------------------------------------------------

#[test]
fn channels_adapter_failures_over_fake_cli() {
    // a) cli-error envelope on every channels/pairing/install call.
    scenario("channels-adapter-cli-error", Some("cli-error"), |sandbox| {
        sandbox.seed(serde_json::json!({}));
        let exe = Path::new(FAKE_OPENCLAW);
        let channels = OpenClawChannelsAdapter::new(Arc::new(ProcessRunner));
        assert_eq!(
            channels.list_channels(exe).unwrap_err().code,
            "openclaw-channels-failed"
        );
        assert_eq!(
            channels.channel_status(exe).unwrap_err().code,
            "openclaw-channels-failed"
        );
        assert_eq!(
            channels.pairing_list(exe, "discord").unwrap_err().code,
            "openclaw-pairing-failed"
        );
        assert_eq!(
            channels
                .pairing_approve(exe, "discord", "AB12CD34")
                .unwrap_err()
                .code,
            "openclaw-pairing-failed"
        );
        let install = OpenClawPluginInstallAdapter::new(Arc::new(ProcessRunner));
        let err = install
            .install_plugin(exe, "@openclaw/discord")
            .unwrap_err();
        assert_eq!(err.code, "openclaw-plugin-install-failed");
        assert!(
            sandbox.state().get("plugins").is_none(),
            "a failed install must not mutate state"
        );
    });

    // b) Malformed / not-json output → the stable parse-failure code.
    scenario("channels-adapter-malformed", Some("malformed"), |sandbox| {
        sandbox.seed(serde_json::json!({}));
        let channels = OpenClawChannelsAdapter::new(Arc::new(ProcessRunner));
        assert_eq!(
            channels
                .list_channels(Path::new(FAKE_OPENCLAW))
                .unwrap_err()
                .code,
            "openclaw-channels-failed"
        );
    });
    scenario("channels-adapter-not-json", Some("not-json"), |sandbox| {
        sandbox.seed(serde_json::json!({}));
        let channels = OpenClawChannelsAdapter::new(Arc::new(ProcessRunner));
        assert_eq!(
            channels
                .channel_status(Path::new(FAKE_OPENCLAW))
                .unwrap_err()
                .code,
            "openclaw-channels-failed"
        );
    });

    // c) Failing CLI: the error message stays masked (S3/S8).
    scenario("channels-adapter-fail-masked", Some("fail"), |sandbox| {
        sandbox.seed(serde_json::json!({}));
        let channels = OpenClawChannelsAdapter::new(Arc::new(ProcessRunner));
        let err = channels
            .list_channels(Path::new(FAKE_OPENCLAW))
            .unwrap_err();
        assert_eq!(err.code, "openclaw-channels-failed");
        assert!(
            !err.message.contains("sk-fake123456789"),
            "secret must not leak into the error: {}",
            err.message
        );
        assert!(err.message.contains("sk-****"), "stderr should be masked");
    });
}

#[test]
fn channels_dry_run_rejection_writes_nothing() {
    scenario(
        "channels-dry-run-reject",
        Some("config-invalid"),
        |sandbox| {
            sandbox.seed(serde_json::json!({}));
            let service = channels_service();
            let err = service
                .set_channel_enabled("discord", true)
                .expect_err("dry-run reject");
            assert_eq!(err.code, "openclaw-config-invalid");
            let lines = sandbox.captured();
            assert_eq!(lines.len(), 1, "only the dry-run ran, no real write");
            assert!(lines[0].contains(&"--dry-run".to_string()));
            let state = sandbox.state();
            assert!(state.get("channels").is_none(), "state must be unchanged");
        },
    );
}

#[test]
fn channels_missing_executable() {
    // Service level: detection failure → the reused Phase 1 code, 0 CLI calls.
    let service = channels_service_no_openclaw();
    let failures = [
        service.get_channels().unwrap_err(),
        service.get_channel_config("discord").unwrap_err(),
        service.connect_channel("discord").unwrap_err(),
        service.set_channel_enabled("discord", true).unwrap_err(),
        service
            .set_dm_access("discord", "pairing", &["*".into()])
            .unwrap_err(),
        service.set_group_policy("discord", "open").unwrap_err(),
        service.list_pairing_requests("discord").unwrap_err(),
        service.approve_pairing("discord", "ABCD1234").unwrap_err(),
    ];
    for err in failures {
        assert_eq!(err.code, "openclaw-not-found");
    }
    let token = ChannelTokenService::new(
        Arc::new(NoOpenClaw),
        Arc::new(OpenClawConfigAdapter::new(Arc::new(ProcessRunner))),
        Arc::new(FailingSecretStore),
        PathBuf::from("resolver.exe"),
    );
    assert_eq!(
        token
            .set_channel_token("discord", TEST_TOKEN)
            .unwrap_err()
            .code,
        "openclaw-not-found"
    );
    assert_eq!(
        token.delete_channel_token("discord").unwrap_err().code,
        "openclaw-not-found"
    );
}

// --- fake-level behavior (per-request env; parallel-safe) --------------------------------

#[test]
fn fake_plugins_install_updates_catalog() {
    let sandbox = Sandbox::new("fake-plugin-install");
    sandbox.seed(serde_json::json!({}));
    let output = run_fake(&["plugins", "install", "@openclaw/discord"], &sandbox, &[])
        .expect("install should run");
    assert_eq!(output.exit_code, 0);
    let state = sandbox.state();
    assert!(state["plugins"]["catalog"]["@openclaw/discord"].is_object());
    // Idempotent: a second install leaves the catalog unchanged.
    let output = run_fake(&["plugins", "install", "@openclaw/discord"], &sandbox, &[])
        .expect("second install");
    assert_eq!(output.exit_code, 0);
    assert_eq!(
        sandbox.captured(),
        vec![
            vec!["plugins", "install", "@openclaw/discord"],
            vec!["plugins", "install", "@openclaw/discord"],
        ]
    );
}

#[test]
fn fake_never_accepts_a_channel_token_in_argv() {
    // S2: the fake has no token-carrying command — `channels add --token`
    // must be rejected exactly like an unknown command, so a regression that
    // starts passing tokens in argv is caught at the contract layer.
    let sandbox = Sandbox::new("fake-no-token-argv");
    sandbox.seed(serde_json::json!({}));
    let output = run_fake(
        &["channels", "add", "discord", "--token", TEST_TOKEN],
        &sandbox,
        &[],
    )
    .expect("invocation should run");
    assert_eq!(output.exit_code, 2, "token-carrying command unsupported");
    assert!(
        output.stderr.contains("unsupported command"),
        "{:?}",
        output.stderr
    );
}

// --- fake-level runner ---------------------------------------------------------------------

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
    // Fake-level tests run in parallel with the serialized scenario tests,
    // which set `CLAWDESK_FAKE_BEHAVIOR` as inherited global env. Pin the
    // behavior per request (default: normal) so a scenario's override can
    // never leak into a fake-level invocation.
    let behavior_pinned = extra_envs
        .iter()
        .any(|(key, _)| *key == "CLAWDESK_FAKE_BEHAVIOR");
    if !behavior_pinned {
        request
            .env
            .push(("CLAWDESK_FAKE_BEHAVIOR".to_string(), "normal".to_string()));
    }
    for (key, value) in extra_envs {
        request.env.push((key.to_string(), value.to_string()));
    }
    ProcessRunner.run(&request)
}
