use super::*;
use crate::domain::models::models::{ModelEntry, ProviderApiKey, ProviderDetail};
use crate::domain::models::openclaw::{GatewayStatus, OpenClawVersion, UpdateState};
use crate::domain::ports::openclaw::OpenClawPort;
use crate::domain::ports::openclaw_config::{OpenClawConfigPort, WriteMode};
use crate::domain::ports::secrets::SecretStorePort;
use crate::error::AppError;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const EXE: &str = "C:\\fake\\openclaw.exe";
const RESOLVER: &str = "C:\\tools\\clawdesk-secret-resolver.exe";
const TEST_TOKEN: &str = "clawdesk-test-channel-token-1234567890";

// --- fakes ----------------------------------------------------------------

struct FixedOpenClaw;

impl OpenClawPort for FixedOpenClaw {
    fn detect_executable(&self) -> crate::domain::models::ExecutableDetection {
        crate::domain::models::ExecutableDetection::Found {
            path: PathBuf::from(EXE),
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

struct FakeConfig {
    providers: Mutex<Vec<ProviderDetail>>,
    raw: Mutex<std::collections::HashMap<String, String>>,
    log: Arc<Mutex<Vec<String>>>,
    writes: Mutex<Vec<(String, WriteMode, String)>>,
}

impl FakeConfig {
    fn new(providers: Vec<ProviderDetail>) -> (Arc<FakeConfig>, Arc<Mutex<Vec<String>>>) {
        let log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let inner = Arc::new(FakeConfig {
            providers: Mutex::new(providers),
            raw: Mutex::new(std::collections::HashMap::new()),
            log: Arc::clone(&log),
            writes: Mutex::new(Vec::new()),
        });
        (inner, log)
    }
}

impl OpenClawConfigPort for FakeConfig {
    fn config_path(&self, _exe: &Path) -> Result<PathBuf, AppError> {
        unimplemented!()
    }
    fn read_providers(&self, _exe: &Path) -> Result<Vec<ProviderDetail>, AppError> {
        self.log.lock().unwrap().push("read-providers".to_string());
        Ok(self.providers.lock().unwrap().clone())
    }
    fn read_models(&self, _exe: &Path) -> Result<Vec<crate::domain::models::ModelRow>, AppError> {
        Ok(Vec::new())
    }
    fn read_default_model(&self, _exe: &Path) -> Result<Option<String>, AppError> {
        Ok(None)
    }
    fn read_thinking_default(
        &self,
        _exe: &Path,
    ) -> Result<Option<crate::domain::models::ThinkingLevel>, AppError> {
        Ok(None)
    }
    fn read_raw(&self, _exe: &Path, path: &str) -> Result<Option<String>, AppError> {
        self.log.lock().unwrap().push(format!("read-raw:{path}"));
        Ok(self.raw.lock().unwrap().get(path).cloned())
    }
    fn write(
        &self,
        _exe: &Path,
        path: &str,
        value_json: &str,
        mode: WriteMode,
    ) -> Result<(), AppError> {
        self.writes
            .lock()
            .unwrap()
            .push((path.to_string(), mode, value_json.to_string()));
        let mode_label = match mode {
            WriteMode::Merge => "merge",
            WriteMode::Replace => "replace",
            WriteMode::Plain => "plain",
        };
        self.log
            .lock()
            .unwrap()
            .push(format!("write:{path}:{mode_label}"));
        self.raw
            .lock()
            .unwrap()
            .insert(path.to_string(), value_json.to_string());
        Ok(())
    }
    fn unset(&self, _exe: &Path, path: &str) -> Result<(), AppError> {
        self.log.lock().unwrap().push(format!("unset:{path}"));
        self.raw.lock().unwrap().remove(path);
        Ok(())
    }
    fn set_default_model(&self, _exe: &Path, _model_ref: &str) -> Result<(), AppError> {
        unimplemented!()
    }
}

#[derive(Debug, Default)]
struct FakeStore {
    map: Mutex<std::collections::HashMap<String, String>>,
    set_fails: Mutex<bool>,
}

impl SecretStorePort for FakeStore {
    fn set(&self, key_id: &str, value: &str) -> Result<(), AppError> {
        if *self.set_fails.lock().unwrap() {
            return Err(AppError::secret_store_unavailable("injected DPAPI failure"));
        }
        self.map
            .lock()
            .unwrap()
            .insert(key_id.to_string(), value.to_string());
        Ok(())
    }
    fn get(&self, key_id: &str) -> Result<Option<String>, AppError> {
        Ok(self.map.lock().unwrap().get(key_id).cloned())
    }
    fn delete(&self, key_id: &str) -> Result<(), AppError> {
        if self.map.lock().unwrap().remove(key_id).is_none() {
            return Err(AppError::secret_store_unavailable("not registered"));
        }
        Ok(())
    }
    fn contains(&self, key_id: &str) -> bool {
        self.map.lock().unwrap().contains_key(key_id)
    }
    fn list_key_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.map.lock().unwrap().keys().cloned().collect();
        ids.sort();
        ids
    }
}

// --- helpers ----------------------------------------------------------------

fn provider(id: &str, key: ProviderApiKey) -> ProviderDetail {
    ProviderDetail {
        id: id.to_string(),
        base_url: None,
        api: Some("openai-completions".to_string()),
        api_key: key,
        models: vec![ModelEntry {
            id: "m1".into(),
            name: None,
            reasoning: false,
            input: vec!["text".into()],
            context_window: None,
            max_tokens: None,
            compat: None,
        }],
    }
}

fn declaration_for(resolver: &str) -> String {
    serde_json::json!({
        "source": "exec",
        "command": resolver,
        "timeoutMs": 5000,
        "jsonOnly": true,
    })
    .to_string()
}

fn discord_ref() -> String {
    serde_json::to_string(&channel_secret_ref("discord")).unwrap()
}

fn telegram_ref() -> String {
    serde_json::to_string(&channel_secret_ref("telegram")).unwrap()
}

type ServiceFixture = (
    ChannelTokenService,
    Arc<FakeConfig>,
    Arc<FakeStore>,
    Arc<Mutex<Vec<String>>>,
);

fn service(providers: Vec<ProviderDetail>) -> ServiceFixture {
    let (config, log) = FakeConfig::new(providers);
    let secrets = Arc::new(FakeStore::default());
    let service = ChannelTokenService::new(
        Arc::new(FixedOpenClaw),
        config.clone(),
        secrets.clone(),
        PathBuf::from(RESOLVER),
    );
    (service, config, secrets, log)
}

fn service_with_declaration(providers: Vec<ProviderDetail>, command: &str) -> ServiceFixture {
    let (service, config, store, log) = service(providers);
    config.raw.lock().unwrap().insert(
        "secrets.providers.clawdesk".to_string(),
        declaration_for(command),
    );
    (service, config, store, log)
}

// --- set_channel_token ---------------------------------------------------------

#[test]
fn set_token_full_sequence_stores_dpapi_first_then_ref() {
    let (service, config, store, log) = service(vec![]);
    service
        .set_channel_token("discord", TEST_TOKEN)
        .expect("set");
    let lines = log.lock().unwrap().clone();
    assert_eq!(
        lines,
        vec![
            "read-raw:channels.discord.token".to_string(),
            "read-raw:secrets.providers.clawdesk".to_string(),
            "write:secrets.providers.clawdesk:plain".to_string(),
            "write:channels.discord.token:plain".to_string(),
        ]
    );
    let writes = config.writes.lock().unwrap().clone();
    let (_decl_path, _mode, decl_body) = writes
        .iter()
        .find(|(path, _, _)| path == "secrets.providers.clawdesk")
        .cloned()
        .expect("declaration write");
    let decl: serde_json::Value = serde_json::from_str(&decl_body).unwrap();
    assert_eq!(decl["source"], "exec");
    assert_eq!(decl["command"], RESOLVER);
    let (_ref_path, _mode, ref_body) = writes
        .iter()
        .find(|(path, _, _)| path == "channels.discord.token")
        .cloned()
        .expect("ref write");
    let reference: serde_json::Value = serde_json::from_str(&ref_body).unwrap();
    assert_eq!(reference["source"], "exec");
    assert_eq!(reference["provider"], "clawdesk");
    assert_eq!(reference["id"], "channels/discord/token");
    // DPAPI holds the value (only in the store).
    assert_eq!(
        store.get("channels/discord/token").unwrap(),
        Some(TEST_TOKEN.to_string())
    );
    // No plaintext in any log line, write path, or write body.
    for line in &lines {
        assert!(!line.contains(TEST_TOKEN), "{line}");
    }
    for (_path, _mode, body) in &writes {
        assert!(!body.contains(TEST_TOKEN));
    }
}

#[test]
fn set_telegram_token_uses_bot_token_paths() {
    let (service, config, store, _log) = service(vec![]);
    service
        .set_channel_token("telegram", TEST_TOKEN)
        .expect("set");
    assert_eq!(
        store.get("channels/telegram/botToken").unwrap(),
        Some(TEST_TOKEN.to_string())
    );
    let writes = config.writes.lock().unwrap().clone();
    let (_ref_path, _mode, ref_body) = writes
        .iter()
        .find(|(path, _, _)| path == "channels.telegram.botToken")
        .cloned()
        .expect("telegram ref write");
    let reference: serde_json::Value = serde_json::from_str(&ref_body).unwrap();
    assert_eq!(reference["id"], "channels/telegram/botToken");
}

#[test]
fn set_token_with_matching_declaration_skips_declaration_write() {
    let (service, _config, store, log) = service_with_declaration(vec![], RESOLVER);
    service
        .set_channel_token("discord", TEST_TOKEN)
        .expect("set");
    let lines = log.lock().unwrap().clone();
    assert!(
        !lines
            .iter()
            .any(|line| line.starts_with("write:secrets.providers.clawdesk")),
        "idempotent: no declaration rewrite. {lines:?}"
    );
    assert!(lines.contains(&"write:channels.discord.token:plain".to_string()));
    assert!(store.contains("channels/discord/token"));
}

#[test]
fn set_token_with_stale_declaration_rewrites_it() {
    let (service, config, _store, _log) = service_with_declaration(vec![], "C:\\old\\resolver.exe");
    service
        .set_channel_token("discord", TEST_TOKEN)
        .expect("set");
    let writes = config.writes.lock().unwrap().clone();
    let (_, _, body) = writes
        .iter()
        .find(|(path, _, _)| path == "secrets.providers.clawdesk")
        .cloned()
        .expect("declaration write");
    let decl: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(decl["command"], RESOLVER);
}

#[test]
fn set_token_with_external_plaintext_is_rejected_without_writes() {
    let (service, config, store, log) = service(vec![]);
    config.raw.lock().unwrap().insert(
        "channels.discord.token".to_string(),
        r#""external-token-value""#.into(),
    );
    let err = service
        .set_channel_token("discord", TEST_TOKEN)
        .expect_err("external token must be rejected");
    assert_eq!(err.code, "secret-ref-registration-failed");
    assert!(
        log.lock()
            .unwrap()
            .iter()
            .all(|line| !line.starts_with("write") && !line.starts_with("unset")),
        "zero config writes on external token"
    );
    assert!(
        !store.contains("channels/discord/token"),
        "DPAPI set 0 times"
    );
}

#[test]
fn set_token_with_foreign_ref_is_rejected_without_writes() {
    let (service, config, store, log) = service(vec![]);
    config.raw.lock().unwrap().insert(
        "channels.discord.token".to_string(),
        r#"{"source":"exec","provider":"env","id":"DISCORD_BOT_TOKEN"}"#.into(),
    );
    let err = service
        .set_channel_token("discord", TEST_TOKEN)
        .expect_err("foreign ref must be rejected");
    assert_eq!(err.code, "secret-ref-registration-failed");
    assert!(log
        .lock()
        .unwrap()
        .iter()
        .all(|line| !line.starts_with("write")));
    assert!(!store.contains("channels/discord/token"));
}

#[test]
fn set_token_reattaches_when_already_managed() {
    let (service, config, store, _log) = service(vec![]);
    config
        .raw
        .lock()
        .unwrap()
        .insert("channels.discord.token".to_string(), discord_ref());
    service
        .set_channel_token("discord", TEST_TOKEN)
        .expect("re-register");
    assert_eq!(
        store.get("channels/discord/token").unwrap(),
        Some(TEST_TOKEN.to_string()),
        "value re-stored"
    );
    let writes = config.writes.lock().unwrap().clone();
    assert!(
        writes
            .iter()
            .any(|(path, _, _)| path == "channels.discord.token"),
        "ref re-written"
    );
}

#[test]
fn set_token_invalid_channel_or_empty_token_has_zero_runs() {
    let (service, _config, store, log) = service(vec![]);
    for bad in ["slack", "Discord", ""] {
        let err = service
            .set_channel_token(bad, TEST_TOKEN)
            .expect_err("bad channel");
        assert_eq!(err.code, "channel-id-invalid", "{bad:?}");
    }
    for bad in ["", "   "] {
        let err = service
            .set_channel_token("discord", bad)
            .expect_err("empty token");
        assert_eq!(err.code, "channel-token-invalid", "{bad:?}");
    }
    assert!(log.lock().unwrap().is_empty(), "no config run at all");
    assert!(!store.contains("channels/discord/token"));
}

#[test]
fn set_token_dpapi_failure_writes_no_config() {
    let (service, _config, store, log) = service(vec![]);
    *store.set_fails.lock().unwrap() = true;
    let err = service
        .set_channel_token("discord", TEST_TOKEN)
        .expect_err("injected DPAPI failure");
    assert_eq!(err.code, "secret-store-unavailable");
    // DPAPI first (contract order): a DPAPI failure leaves zero config
    // writes — only the token-field read may have run.
    let lines = log.lock().unwrap().clone();
    assert!(
        lines
            .iter()
            .all(|line| !line.starts_with("write") && !line.starts_with("unset")),
        "zero config writes on DPAPI failure: {lines:?}"
    );
    assert!(!store.contains("channels/discord/token"));
}

// --- delete_channel_token ------------------------------------------------------

#[test]
fn delete_token_removes_ref_value_and_last_declaration() {
    let (service, config, store, log) = service(vec![]);
    config
        .raw
        .lock()
        .unwrap()
        .insert("channels.discord.token".to_string(), discord_ref());
    config.raw.lock().unwrap().insert(
        "secrets.providers.clawdesk".to_string(),
        declaration_for(RESOLVER),
    );
    store.set("channels/discord/token", TEST_TOKEN).unwrap();
    service.delete_channel_token("discord").expect("delete");
    let lines = log.lock().unwrap().clone();
    assert_eq!(
        lines,
        vec![
            "read-raw:channels.discord.token".to_string(),
            "unset:channels.discord.token".to_string(),
            "read-providers".to_string(),
            "read-raw:channels.discord.token".to_string(),
            "read-raw:channels.telegram.botToken".to_string(),
            "unset:secrets.providers.clawdesk".to_string(),
        ]
    );
    assert!(!store.contains("channels/discord/token"));
}

#[test]
fn delete_token_keeps_declaration_when_provider_key_remains() {
    let (service, config, store, log) = service(vec![provider("acme", ProviderApiKey::Managed)]);
    config
        .raw
        .lock()
        .unwrap()
        .insert("channels.discord.token".to_string(), discord_ref());
    store.set("providers/acme/apiKey", "k1").unwrap();
    store.set("channels/discord/token", TEST_TOKEN).unwrap();
    service.delete_channel_token("discord").expect("delete");
    let lines = log.lock().unwrap().clone();
    assert!(
        !lines.contains(&"unset:secrets.providers.clawdesk".to_string()),
        "provider surface still managed: declaration must stay. {lines:?}"
    );
    assert!(!store.contains("channels/discord/token"));
}

#[test]
fn delete_token_keeps_declaration_when_other_channel_ref_remains() {
    let (service, config, store, log) = service(vec![]);
    config
        .raw
        .lock()
        .unwrap()
        .insert("channels.telegram.botToken".to_string(), telegram_ref());
    config
        .raw
        .lock()
        .unwrap()
        .insert("channels.discord.token".to_string(), discord_ref());
    store.set("channels/telegram/botToken", "tg").unwrap();
    store.set("channels/discord/token", TEST_TOKEN).unwrap();
    service.delete_channel_token("discord").expect("delete");
    let lines = log.lock().unwrap().clone();
    assert!(
        !lines.contains(&"unset:secrets.providers.clawdesk".to_string()),
        "channel surface still managed: declaration must stay. {lines:?}"
    );
    assert!(!store.contains("channels/discord/token"));
    assert!(
        store.contains("channels/telegram/botToken"),
        "the other channel entry is untouched"
    );
}

#[test]
fn delete_orphan_store_entry_cleans_up_without_ref_writes() {
    // A `set_channel_token` whose config write failed after the DPAPI step
    // leaves a store entry without a config ref (orphan).
    let (service, _config, store, log) = service(vec![]);
    store.set("channels/discord/token", "orphaned").unwrap();
    service
        .delete_channel_token("discord")
        .expect("orphan cleanup");
    assert!(!store.contains("channels/discord/token"));
    let lines = log.lock().unwrap().clone();
    assert!(
        lines.iter().all(
            |line| !line.starts_with("write") && !line.contains("unset:channels.discord.token")
        ),
        "no token ref writes at all: {lines:?}"
    );
    // No managed refs on either surface left → the shared declaration is
    // cleaned up.
    assert!(lines.contains(&"unset:secrets.providers.clawdesk".to_string()));
}

#[test]
fn delete_token_without_ref_or_store_entry_is_not_found() {
    let (service, _config, store, log) = service(vec![]);
    let err = service
        .delete_channel_token("discord")
        .expect_err("nothing registered");
    assert_eq!(err.code, "channel-token-not-found");
    assert!(
        log.lock()
            .unwrap()
            .iter()
            .all(|line| !line.starts_with("write") && !line.starts_with("unset")),
        "no mutation of any kind"
    );
    assert!(!store.contains("channels/discord/token"));
}

#[test]
fn delete_token_external_value_without_store_entry_is_not_found() {
    let (service, config, store, log) = service(vec![]);
    config.raw.lock().unwrap().insert(
        "channels.telegram.botToken".to_string(),
        r#""external-tg""#.into(),
    );
    let err = service
        .delete_channel_token("telegram")
        .expect_err("external token is not ours");
    assert_eq!(err.code, "channel-token-not-found");
    assert!(
        log.lock()
            .unwrap()
            .iter()
            .all(|line| !line.starts_with("unset")),
        "the external config value must not be touched"
    );
    assert!(
        config
            .raw
            .lock()
            .unwrap()
            .contains_key("channels.telegram.botToken"),
        "external value still present"
    );
    assert!(!store.contains("channels/telegram/botToken"));
}

#[test]
fn delete_token_external_value_with_orphan_entry_cleans_store_only() {
    let (service, config, store, _log) = service(vec![]);
    config
        .raw
        .lock()
        .unwrap()
        .insert("channels.discord.token".to_string(), r#""external""#.into());
    store.set("channels/discord/token", "orphaned").unwrap();
    service
        .delete_channel_token("discord")
        .expect("store-only cleanup");
    assert!(!store.contains("channels/discord/token"));
    assert!(
        config
            .raw
            .lock()
            .unwrap()
            .contains_key("channels.discord.token"),
        "external config value preserved"
    );
}

#[test]
fn delete_token_invalid_channel_has_zero_runs() {
    let (service, _config, _store, log) = service(vec![]);
    let err = service
        .delete_channel_token("slack")
        .expect_err("bad channel");
    assert_eq!(err.code, "channel-id-invalid");
    assert!(log.lock().unwrap().is_empty());
}

// --- list_channel_tokens ---------------------------------------------------------

#[test]
fn list_channel_tokens_parses_channel_key_ids_only() {
    let (service, _config, store, _log) = service(vec![]);
    store.set("channels/discord/token", "a").unwrap();
    store.set("channels/telegram/botToken", "b").unwrap();
    store.set("providers/acme/apiKey", "x").unwrap();
    store.set("channels/unknown/field", "y").unwrap();
    let statuses = service.list_channel_tokens().expect("list");
    assert_eq!(statuses.len(), 2);
    assert_eq!(statuses[0].channel, "discord");
    assert!(statuses[0].registered);
    assert_eq!(statuses[1].channel, "telegram");
    assert!(statuses[1].registered);
}
