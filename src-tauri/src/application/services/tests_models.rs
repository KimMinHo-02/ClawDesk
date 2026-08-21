use super::*;
use crate::domain::models::openclaw::{GatewayStatus, OpenClawVersion, UpdateState};
use crate::domain::ports::openclaw::OpenClawPort;
use crate::domain::ports::openclaw_config::{OpenClawConfigPort, WriteMode};
use crate::domain::ports::secrets::SecretStorePort;
use crate::error::AppError;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const EXE: &str = "C:\\fake\\openclaw.exe";

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

/// Recording config fake. `write`/`unset` honor `failure` when set;
/// every call is appended to `log` as a descriptive line.
struct FakeConfig {
    providers: Mutex<Vec<ProviderDetail>>,
    raw: Mutex<std::collections::HashMap<String, String>>,
    log: Arc<Mutex<Vec<String>>>,
    last_write: Mutex<Option<(String, WriteMode, String)>>,
    failure: Mutex<Option<AppError>>,
    thinking_default: Mutex<Option<ThinkingLevel>>,
    default_model: Mutex<Option<String>>,
}

impl FakeConfig {
    fn new(providers: Vec<ProviderDetail>) -> (Arc<FakeConfig>, Arc<Mutex<Vec<String>>>) {
        let log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let inner = Arc::new(FakeConfig {
            providers: Mutex::new(providers),
            raw: Mutex::new(std::collections::HashMap::new()),
            log: Arc::clone(&log),
            last_write: Mutex::new(None),
            failure: Mutex::new(None),
            thinking_default: Mutex::new(None),
            default_model: Mutex::new(None),
        });
        (inner, log)
    }

    fn with_failure(inner: &Arc<FakeConfig>, err: AppError) {
        *inner.failure.lock().unwrap() = Some(err);
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
    fn read_models(&self, _exe: &Path) -> Result<Vec<ModelRow>, AppError> {
        Ok(vec![ModelRow {
            provider: "acme".into(),
            model: "m1".into(),
            full: "acme/m1".into(),
            name: None,
            reasoning: true,
            context_tokens: Some(100),
            supported_reasoning_efforts: None,
        }])
    }
    fn read_default_model(&self, _exe: &Path) -> Result<Option<String>, AppError> {
        Ok(self.default_model.lock().unwrap().clone())
    }
    fn read_thinking_default(&self, _exe: &Path) -> Result<Option<ThinkingLevel>, AppError> {
        Ok(*self.thinking_default.lock().unwrap())
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
        if let Some(failure) = self.failure.lock().unwrap().clone() {
            self.log
                .lock()
                .unwrap()
                .push(format!("write:{path}:REJECTED"));
            return Err(failure);
        }
        self.last_write
            .lock()
            .unwrap()
            .replace((path.to_string(), mode, value_json.to_string()));
        let mode_label = match mode {
            WriteMode::Merge => "merge",
            WriteMode::Replace => "replace",
            WriteMode::Plain => "plain",
        };
        self.log
            .lock()
            .unwrap()
            .push(format!("write:{path}:{mode_label}"));
        self.apply_write(path, value_json);
        Ok(())
    }
    fn unset(&self, _exe: &Path, path: &str) -> Result<(), AppError> {
        if let Some(failure) = self.failure.lock().unwrap().clone() {
            self.log
                .lock()
                .unwrap()
                .push(format!("unset:{path}:REJECTED"));
            return Err(failure);
        }
        self.log.lock().unwrap().push(format!("unset:{path}"));
        if let Some(id) = path.strip_prefix("models.providers.") {
            if !id.contains('.') {
                self.providers.lock().unwrap().retain(|p| p.id != id);
            }
        }
        Ok(())
    }
    fn set_default_model(&self, _exe: &Path, model_ref: &str) -> Result<(), AppError> {
        self.log
            .lock()
            .unwrap()
            .push(format!("models-set:{model_ref}"));
        Ok(())
    }
}

impl FakeConfig {
    fn apply_write(&self, path: &str, value_json: &str) {
        if let Some(rest) = path.strip_prefix("models.providers.") {
            if rest.contains('.') {
                let (id, field) = rest.rsplit_once('.').expect("subpath");
                let mut map = self.providers.lock().unwrap();
                if let Some(p) = map.iter_mut().find(|p| p.id == id) {
                    match field {
                        "models" => p.models = serde_json::from_str(value_json).unwrap(),
                        "baseUrl" => p.base_url = Some(serde_json::from_str(value_json).unwrap()),
                        "api" => p.api = Some(serde_json::from_str(value_json).unwrap()),
                        "apiKey" => p.api_key = ProviderApiKey::Managed,
                        _ => {}
                    }
                }
            } else {
                let mut value: serde_json::Value = serde_json::from_str(value_json).unwrap();
                value["id"] = serde_json::Value::String(rest.to_string());
                let parsed: ProviderDetail = serde_json::from_value(value).unwrap();
                self.providers.lock().unwrap().push(parsed);
            }
        }
    }
}

/// In-memory secret store fake.
#[derive(Debug, Default)]
struct FakeStore {
    map: Mutex<std::collections::HashMap<String, String>>,
}

impl SecretStorePort for FakeStore {
    fn set(&self, key_id: &str, value: &str) -> Result<(), AppError> {
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
        base_url: Some(format!("https://{id}.test/v1")),
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

fn input() -> ProviderInput {
    ProviderInput {
        id: "acme".to_string(),
        base_url: Some("https://api.acme.test/v1".to_string()),
        api: "openai-completions".to_string(),
        models: vec![ModelInput {
            id: "m1".to_string(),
            name: Some("M1".to_string()),
            reasoning: true,
            input: vec!["text".to_string(), "image".to_string()],
            context_window: Some(128000),
            max_tokens: None,
            supports_reasoning_effort: true,
            supported_reasoning_efforts: Some(vec!["low".into(), "high".into()]),
        }],
    }
}

type ServiceFixture = (
    ModelService,
    Arc<FakeConfig>,
    Arc<FakeStore>,
    Arc<Mutex<Vec<String>>>,
);

fn service(providers: Vec<ProviderDetail>) -> ServiceFixture {
    let (config, log) = FakeConfig::new(providers);
    let secrets = Arc::new(FakeStore::default());
    let service = ModelService::new(Arc::new(FixedOpenClaw), config.clone(), secrets.clone());
    (service, config, secrets, log)
}

// --- list_providers -----------------------------------------------------------

#[test]
fn list_providers_computes_registration_state() {
    let (service, _config, store, _log) = service(vec![
        provider("managed", ProviderApiKey::Managed),
        provider("absent", ProviderApiKey::Absent),
        provider("other", ProviderApiKey::Other),
    ]);
    store
        .set(&secret_key_id("managed"), "sk-fake123456789")
        .unwrap();
    let summaries = service.list_providers().expect("list");
    let by_id = |id: &str| summaries.iter().find(|s| s.id == id).unwrap();
    assert!(by_id("managed").api_key_registered);
    assert!(!by_id("absent").api_key_registered);
    assert!(!by_id("other").api_key_registered);
    assert_eq!(by_id("managed").model_count, 1);
}

#[test]
fn list_providers_managed_without_store_key_is_not_registered() {
    let (service, _config, _store, _log) = service(vec![provider("acme", ProviderApiKey::Managed)]);
    let summaries = service.list_providers().expect("list");
    assert!(!summaries[0].api_key_registered);
}

// --- get_provider ---------------------------------------------------------------

#[test]
fn get_provider_returns_detail_and_rejects_missing() {
    let (service, _config, _store, log) = service(vec![provider("acme", ProviderApiKey::Absent)]);
    let detail = service.get_provider("acme").expect("detail");
    assert_eq!(detail.models.len(), 1);
    assert_eq!(detail.api_key, ProviderApiKey::Absent);

    let err = service.get_provider("missing").expect_err("not found");
    assert_eq!(err.code, "openclaw-config-read-failed");
    assert!(log
        .lock()
        .unwrap()
        .iter()
        .all(|line| !line.contains("write")));
}

// --- save_provider ----------------------------------------------------------------

#[test]
fn save_new_provider_is_single_merge_write_without_api_key() {
    let (service, config, _store, log) = service(vec![]);
    service.save_provider(&input()).expect("save");
    let lines = log.lock().unwrap().clone();
    assert_eq!(
        lines,
        vec![
            "read-providers".to_string(),
            "write:models.providers.acme:merge".to_string()
        ]
    );
    let (path, mode, body) = config.last_write.lock().unwrap().clone().unwrap();
    assert_eq!(path, "models.providers.acme");
    assert_eq!(mode, WriteMode::Merge);
    let value: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert!(body.contains("\"api\""), "api type must be written");
    assert!(body.contains("m1"), "models must be written");
    assert!(
        !body.contains("apiKey"),
        "apiKey must never be in the payload"
    );
    assert_eq!(value["models"][0]["reasoning"], true);
    assert_eq!(
        value["models"][0]["compat"]["supportedReasoningEfforts"][0],
        "low"
    );
}

#[test]
fn save_existing_provider_uses_subpath_writes_and_preserves_api_key() {
    let (service, config, _store, log) = service(vec![provider("acme", ProviderApiKey::Managed)]);
    service.save_provider(&input()).expect("save");
    let lines = log.lock().unwrap().clone();
    assert_eq!(
        lines,
        vec![
            "read-providers".to_string(),
            "write:models.providers.acme.baseUrl:plain".to_string(),
            "write:models.providers.acme.api:plain".to_string(),
            "write:models.providers.acme.models:replace".to_string(),
        ]
    );
    // The provider node itself is never rewritten → its apiKey survives.
    assert!(!log
        .lock()
        .unwrap()
        .iter()
        .any(|line| line == "write:models.providers.acme:merge"
            || line == "write:models.providers.acme:replace"));
    let updated = config
        .read_providers(Path::new(EXE))
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(
        updated.base_url.as_deref(),
        Some("https://api.acme.test/v1")
    );
    assert_eq!(updated.api_key, ProviderApiKey::Managed);
    assert_eq!(updated.models[0].context_window, Some(128000));
}

#[test]
fn save_invalid_input_performs_no_process_runs() {
    let (service, _config, _store, log) = service(vec![]);
    for (id, url, api, effort) in [
        (
            "../evil",
            Some("https://x.test"),
            "openai-completions",
            None,
        ),
        ("ok", Some("ftp://x.test"), "openai-completions", None),
        ("ok", None, "not-an-api", None),
        ("ok", None, "openai-completions", Some("very-high")),
    ] {
        let mut bad = input();
        bad.id = id.to_string();
        bad.base_url = url.map(str::to_string);
        bad.api = api.to_string();
        if let Some(effort) = effort {
            bad.models[0].supported_reasoning_efforts = Some(vec![effort.to_string()]);
        }
        let err = service.save_provider(&bad).expect_err("must reject");
        assert!(
            matches!(err.code, "provider-id-invalid" | "thinking-level-invalid"),
            "{id} {url:?} {api} {effort:?} → {} ",
            err.code
        );
    }
    assert!(
        log.lock().unwrap().is_empty(),
        "0 process runs on validation failure"
    );
}

#[test]
fn save_update_stops_after_first_write_failure() {
    let (service, config, _store, log) = service(vec![provider("acme", ProviderApiKey::Absent)]);
    FakeConfig::with_failure(
        &config,
        AppError::openclaw_config_invalid("schema rejected"),
    );
    let err = service
        .save_provider(&input())
        .expect_err("injected failure");
    assert_eq!(err.code, "openclaw-config-invalid");
    let lines = log.lock().unwrap().clone();
    assert_eq!(
        lines,
        vec![
            "read-providers".to_string(),
            "write:models.providers.acme.baseUrl:REJECTED".to_string(),
        ]
    );
}

// --- delete_provider -----------------------------------------------------------------

#[test]
fn delete_provider_with_managed_key_cleans_key_and_last_declaration() {
    let (service, config, store, log) = service(vec![provider("acme", ProviderApiKey::Managed)]);
    store
        .set(&secret_key_id("acme"), "sk-fake123456789")
        .unwrap();
    service.delete_provider("acme").expect("delete");
    let lines = log.lock().unwrap().clone();
    assert!(
        lines.contains(&"unset:models.providers.acme".to_string()),
        "{lines:?}"
    );
    assert!(
        lines.contains(&"unset:secrets.providers.clawdesk".to_string()),
        "last managed key must remove the declaration"
    );
    assert!(!store.contains(&secret_key_id("acme")));
    assert!(config.read_providers(Path::new(EXE)).unwrap().is_empty());
}

#[test]
fn delete_provider_keeps_declaration_when_another_managed_key_remains() {
    let (service, _config, store, log) = service(vec![
        provider("acme", ProviderApiKey::Managed),
        provider("beta", ProviderApiKey::Managed),
    ]);
    store.set(&secret_key_id("acme"), "k1").unwrap();
    store.set(&secret_key_id("beta"), "k2").unwrap();
    service.delete_provider("acme").expect("delete");
    let lines = log.lock().unwrap().clone();
    assert!(!lines.contains(&"unset:secrets.providers.clawdesk".to_string()));
    assert!(store.contains(&secret_key_id("beta")));
    assert!(!store.contains(&secret_key_id("acme")));
}

#[test]
fn delete_provider_without_key_does_not_touch_secrets() {
    let (service, _config, store, log) = service(vec![provider("acme", ProviderApiKey::Absent)]);
    service.delete_provider("acme").expect("delete");
    let lines = log.lock().unwrap().clone();
    assert_eq!(
        lines,
        vec![
            "read-providers".to_string(),
            "unset:models.providers.acme".to_string(),
        ]
    );
    assert!(store.list_key_ids().is_empty());
}

#[test]
fn delete_missing_provider_is_read_failed() {
    let (service, _config, _store, log) = service(vec![]);
    let err = service.delete_provider("gone").expect_err("must fail");
    assert_eq!(err.code, "openclaw-config-read-failed");
    assert!(log
        .lock()
        .unwrap()
        .iter()
        .all(|line| !line.starts_with("unset")));
}

// --- default model / thinking default --------------------------------------------------

#[test]
fn set_default_model_validates_before_running() {
    let (service, _config, _store, log) = service(vec![]);
    for bad in ["acme", "acme/", "../gpt", "a/b/c"] {
        assert!(service.set_default_model(bad).is_err(), "{bad}");
    }
    assert!(log.lock().unwrap().is_empty());
    service.set_default_model("acme/m1").expect("set");
    assert_eq!(
        log.lock().unwrap().clone(),
        vec!["models-set:acme/m1".to_string()]
    );
}

#[test]
fn set_thinking_default_writes_json_string_level() {
    let (service, config, _store, log) = service(vec![]);
    assert_eq!(
        service.set_thinking_default("very-high").unwrap_err().code,
        "thinking-level-invalid"
    );
    assert!(log.lock().unwrap().is_empty());
    service.set_thinking_default("high").expect("set");
    let lines = log.lock().unwrap().clone();
    assert_eq!(lines, vec!["write:agents.defaults.thinkingDefault:plain"]);
    let (path, _mode, body) = config.last_write.lock().unwrap().clone().unwrap();
    assert_eq!(path, "agents.defaults.thinkingDefault");
    assert_eq!(body, r#""high""#);
}

#[test]
fn reads_pass_through() {
    let (service, config, _store, _log) = service(vec![]);
    *config.thinking_default.lock().unwrap() = Some(ThinkingLevel::Low);
    *config.default_model.lock().unwrap() = Some("acme/m1".to_string());
    assert_eq!(
        service.get_thinking_default().unwrap(),
        Some(ThinkingLevel::Low)
    );
    assert_eq!(service.get_default_model().unwrap(), Some("acme/m1".into()));
    assert_eq!(service.list_models().unwrap()[0].full, "acme/m1");
}
