use super::*;
use crate::domain::models::openclaw::{GatewayStatus, OpenClawVersion, UpdateState};
use crate::domain::models::{ModelEntry, ProviderApiKey, ProviderDetail};
use crate::domain::ports::openclaw::OpenClawPort;
use crate::domain::ports::openclaw_config::{OpenClawConfigPort, WriteMode};
use crate::domain::ports::secrets::SecretStorePort;
use crate::error::AppError;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const EXE: &str = "C:\\fake\\openclaw.exe";
const RESOLVER: &str = "C:\\tools\\clawdesk-secret-resolver.exe";

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
        if let Some(rest) = path.strip_prefix("models.providers.") {
            if rest.contains('.') {
                let (id, field) = rest.rsplit_once('.').expect("subpath");
                let mut map = self.providers.lock().unwrap();
                if let Some(p) = map.iter_mut().find(|p| p.id == id) {
                    if field == "apiKey" {
                        p.api_key = ProviderApiKey::Managed;
                    }
                }
            }
        }
        Ok(())
    }
    fn unset(&self, _exe: &Path, path: &str) -> Result<(), AppError> {
        self.log.lock().unwrap().push(format!("unset:{path}"));
        if let Some(rest) = path.strip_prefix("models.providers.") {
            if let Some((id, field)) = rest.split_once('.') {
                if field == "apiKey" {
                    let mut map = self.providers.lock().unwrap();
                    if let Some(p) = map.iter_mut().find(|p| p.id == id) {
                        p.api_key = ProviderApiKey::Absent;
                    }
                }
            } else if !rest.contains('.') {
                self.providers.lock().unwrap().retain(|p| p.id != rest);
            }
        }
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

type ServiceFixture = (
    ApiKeyService,
    Arc<FakeConfig>,
    Arc<FakeStore>,
    Arc<Mutex<Vec<String>>>,
);

fn service(providers: Vec<ProviderDetail>) -> ServiceFixture {
    let (config, log) = FakeConfig::new(providers);
    let secrets = Arc::new(FakeStore::default());
    let service = ApiKeyService::new(
        Arc::new(FixedOpenClaw),
        config.clone(),
        secrets.clone(),
        PathBuf::from(RESOLVER),
    );
    (service, config, secrets, log)
}

// --- set_api_key ------------------------------------------------------------------

#[test]
fn set_key_full_sequence_registers_declaration_ref_and_value() {
    let (service, config, store, log) = service(vec![provider("acme", ProviderApiKey::Absent)]);
    service
        .set_api_key("acme", "sk-fake123456789")
        .expect("set");
    let lines = log.lock().unwrap().clone();
    assert_eq!(
        lines,
        vec![
            "read-providers".to_string(),
            "read-raw:secrets.providers.clawdesk".to_string(),
            "write:secrets.providers.clawdesk:plain".to_string(),
            "write:models.providers.acme.apiKey:plain".to_string(),
        ]
    );
    let writes = config.writes.lock().unwrap().clone();
    // Declaration carries the resolver absolute path (no shell).
    let (_decl_path, _mode, decl_body) = writes
        .iter()
        .find(|(path, _, _)| path == "secrets.providers.clawdesk")
        .cloned()
        .expect("declaration write");
    let decl: serde_json::Value = serde_json::from_str(&decl_body).unwrap();
    assert_eq!(decl["source"], "exec");
    assert_eq!(decl["command"], RESOLVER);
    // The ref write targets the provider's apiKey subpath.
    let (_ref_path, _mode, ref_body) = writes
        .iter()
        .find(|(path, _, _)| path == "models.providers.acme.apiKey")
        .cloned()
        .expect("ref write");
    let reference: serde_json::Value = serde_json::from_str(&ref_body).unwrap();
    assert_eq!(reference["source"], "exec");
    assert_eq!(reference["provider"], "clawdesk");
    assert_eq!(reference["id"], "providers/acme/apiKey");
    // DPAPI holds the value (only in the store).
    assert_eq!(
        store.get("providers/acme/apiKey").unwrap(),
        Some("sk-fake123456789".to_string())
    );
    // No plaintext in any log line or write path.
    assert!(lines.iter().all(|line| !line.contains("sk-fake123456789")));
}

#[test]
fn set_key_with_matching_declaration_skips_declaration_write() {
    let (service, _config, store, log) =
        service_with_declaration(vec![provider("acme", ProviderApiKey::Absent)], RESOLVER);
    service
        .set_api_key("acme", "sk-fake123456789")
        .expect("set");
    let lines = log.lock().unwrap().clone();
    assert!(
        !lines
            .iter()
            .any(|line| line.starts_with("write:secrets.providers.clawdesk")),
        "idempotent: no declaration rewrite. {lines:?}"
    );
    assert!(lines.contains(&"write:models.providers.acme.apiKey:plain".to_string()));
    assert!(store.contains("providers/acme/apiKey"));
}

fn service_with_declaration(providers: Vec<ProviderDetail>, command: &str) -> ServiceFixture {
    let (service, config, store, log) = service(providers);
    config.raw.lock().unwrap().insert(
        "secrets.providers.clawdesk".to_string(),
        declaration_for(command),
    );
    (service, config, store, log)
}

#[test]
fn set_key_with_stale_declaration_rewrites_it() {
    let (service, config, _store, log) = service_with_declaration(
        vec![provider("acme", ProviderApiKey::Absent)],
        "C:\\old\\resolver.exe",
    );
    service
        .set_api_key("acme", "sk-fake123456789")
        .expect("set");
    let lines = log.lock().unwrap().clone();
    assert!(lines.contains(&"write:secrets.providers.clawdesk:plain".to_string()));
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
fn set_empty_key_is_rejected_without_any_run() {
    let (service, _config, store, log) = service(vec![provider("acme", ProviderApiKey::Absent)]);
    let err = service.set_api_key("acme", "   ").expect_err("empty key");
    assert_eq!(err.code, "openclaw-config-invalid");
    assert!(log.lock().unwrap().is_empty());
    assert!(!store.contains("providers/acme/apiKey"));
}

#[test]
fn set_key_missing_provider_is_read_failed() {
    let (service, _config, store, log) = service(vec![]);
    let err = service
        .set_api_key("gone", "sk-fake123456789")
        .expect_err("missing provider");
    assert_eq!(err.code, "openclaw-config-read-failed");
    assert!(log
        .lock()
        .unwrap()
        .iter()
        .all(|line| !line.starts_with("write")));
    assert!(!store.contains("providers/gone/apiKey"));
}

#[test]
fn set_key_with_external_key_is_rejected_without_writes() {
    let (service, _config, store, log) = service(vec![provider("acme", ProviderApiKey::Other)]);
    let err = service
        .set_api_key("acme", "sk-fake123456789")
        .expect_err("external key must be rejected");
    assert_eq!(err.code, "secret-ref-registration-failed");
    assert!(log
        .lock()
        .unwrap()
        .iter()
        .all(|line| !line.starts_with("write")));
    assert!(!store.contains("providers/acme/apiKey"));
}

#[test]
fn set_key_dpapi_failure_writes_no_config() {
    let (service, _config, store, log) = service(vec![provider("acme", ProviderApiKey::Absent)]);
    *store.set_fails.lock().unwrap() = true;
    let err = service
        .set_api_key("acme", "sk-fake123456789")
        .expect_err("injected DPAPI failure");
    assert_eq!(err.code, "secret-store-unavailable");
    // DPAPI first (contract order): a DPAPI failure leaves zero config
    // writes — only the read for the provider check may have run.
    let lines = log.lock().unwrap().clone();
    assert!(
        lines
            .iter()
            .all(|line| !line.starts_with("write") && !line.starts_with("unset")),
        "zero config writes on DPAPI failure: {lines:?}"
    );
    assert!(!store.contains("providers/acme/apiKey"));
}

#[test]
fn set_key_invalid_provider_id_is_rejected_without_any_run() {
    let (service, _config, _store, log) = service(vec![]);
    let err = service
        .set_api_key("../evil", "sk-fake123456789")
        .expect_err("bad id");
    assert_eq!(err.code, "provider-id-invalid");
    assert!(log.lock().unwrap().is_empty());
}

// --- delete_api_key -------------------------------------------------------------------

#[test]
fn delete_key_removes_ref_value_and_last_declaration() {
    let (service, config, store, log) = service(vec![provider("acme", ProviderApiKey::Managed)]);
    store
        .set("providers/acme/apiKey", "sk-fake123456789")
        .unwrap();
    service.delete_api_key("acme").expect("delete");
    let lines = log.lock().unwrap().clone();
    assert_eq!(
        lines,
        vec![
            "read-providers".to_string(),
            "unset:models.providers.acme.apiKey".to_string(),
            "read-providers".to_string(),
            "unset:secrets.providers.clawdesk".to_string(),
        ]
    );
    assert!(!store.contains("providers/acme/apiKey"));
    let remaining = config.read_providers(Path::new(EXE)).unwrap();
    assert_eq!(remaining[0].api_key, ProviderApiKey::Absent);
}

#[test]
fn delete_key_keeps_declaration_when_other_managed_key_remains() {
    let (service, _config, store, log) = service(vec![
        provider("acme", ProviderApiKey::Managed),
        provider("beta", ProviderApiKey::Managed),
    ]);
    store.set("providers/acme/apiKey", "k1").unwrap();
    store.set("providers/beta/apiKey", "k2").unwrap();
    service.delete_api_key("acme").expect("delete");
    let lines = log.lock().unwrap().clone();
    assert!(!lines.contains(&"unset:secrets.providers.clawdesk".to_string()));
    assert!(!store.contains("providers/acme/apiKey"));
    assert!(store.contains("providers/beta/apiKey"));
}

#[test]
fn delete_key_without_managed_ref_or_store_entry_is_read_failed() {
    let (service, _config, store, log) = service(vec![provider("acme", ProviderApiKey::Absent)]);
    let err = service
        .delete_api_key("acme")
        .expect_err("nothing registered");
    assert_eq!(err.code, "openclaw-config-read-failed");
    assert!(log
        .lock()
        .unwrap()
        .iter()
        .all(|line| !line.starts_with("write") && !line.starts_with("unset")));
    assert!(!store.contains("providers/acme/apiKey"));
}

#[test]
fn delete_orphan_store_entry_cleans_up_without_api_key_writes() {
    // A `set_api_key` whose config write failed after the DPAPI step leaves a
    // store entry without a config ref (orphan).
    let (service, _config, store, log) = service(vec![provider("acme", ProviderApiKey::Absent)]);
    store.set("providers/acme/apiKey", "orphaned").unwrap();
    service.delete_api_key("acme").expect("orphan cleanup");
    assert!(!store.contains("providers/acme/apiKey"));
    let lines = log.lock().unwrap().clone();
    // No apiKey subpath write at all; with no managed key left, only the
    // declaration unset may happen.
    assert!(lines
        .iter()
        .all(|line| !line.contains("models.providers.acme.apiKey")));
    assert!(!lines.iter().any(|line| line.starts_with("write")));
    assert!(lines.contains(&"unset:secrets.providers.clawdesk".to_string()));
}

// --- list_api_keys -----------------------------------------------------------------------

#[test]
fn list_api_keys_parses_managed_key_ids_only() {
    let (service, _config, store, _log) = service(vec![]);
    store.set("providers/acme/apiKey", "a").unwrap();
    store.set("providers/beta-x/apiKey", "b").unwrap();
    store.set("unrelated/key", "x").unwrap();
    store.set("providers//apiKey", "y").unwrap();
    let statuses = service.list_api_keys().expect("list");
    let ids: Vec<&str> = statuses.iter().map(|s| s.provider_id.as_str()).collect();
    assert_eq!(ids, vec!["acme", "beta-x"]);
    assert!(statuses.iter().all(|s| s.registered));
}
