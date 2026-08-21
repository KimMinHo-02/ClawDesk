//! Model/provider/default/reasoning use case (Phase 3).
//!
//! Orchestration: validate user input (S2, 0 process runs on failure) →
//! detect the OpenClaw executable → read via the config port → write with
//! the port's built-in dry-run → commit. Field-level subpath writes are used
//! for provider updates so the (redacted) `apiKey` field is never touched.

use std::path::PathBuf;
use std::sync::Arc;

use super::environment::default_openclaw_search_dirs;
use crate::domain::models::models::{
    secret_key_id, validate_model_ref, validate_provider, validate_provider_id, ModelEntry,
    ModelRow, ProviderApiKey, ProviderDetail, ProviderSummary, ThinkingLevel,
};
use crate::domain::ports::openclaw::OpenClawPort;
use crate::domain::ports::openclaw_config::{OpenClawConfigPort, WriteMode};
use crate::domain::ports::secrets::SecretStorePort;
use crate::error::AppError;
use crate::infrastructure::openclaw::{OpenClawAdapter, OpenClawConfigAdapter};
use crate::infrastructure::process::ProcessRunner;
use crate::infrastructure::secrets::SecretStore;

/// One model as submitted by the UI (command-layer DTO shape).
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInput {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub reasoning: bool,
    #[serde(default)]
    pub input: Vec<String>,
    #[serde(default)]
    pub context_window: Option<u64>,
    #[serde(default)]
    pub max_tokens: Option<u64>,
    #[serde(default)]
    pub supports_reasoning_effort: bool,
    #[serde(default)]
    pub supported_reasoning_efforts: Option<Vec<String>>,
}

/// A provider as submitted by the UI (command-layer DTO shape).
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInput {
    pub id: String,
    #[serde(default)]
    pub base_url: Option<String>,
    pub api: String,
    #[serde(default)]
    pub models: Vec<ModelInput>,
}

/// Use case layer: composes the OpenClaw executable, config, and secret
/// store ports.
pub struct ModelService {
    openclaw: Arc<dyn OpenClawPort>,
    config: Arc<dyn OpenClawConfigPort>,
    secrets: Arc<dyn SecretStorePort>,
}

impl ModelService {
    pub fn new(
        openclaw: Arc<dyn OpenClawPort>,
        config: Arc<dyn OpenClawConfigPort>,
        secrets: Arc<dyn SecretStorePort>,
    ) -> Self {
        Self {
            openclaw,
            config,
            secrets,
        }
    }

    /// Production wiring: real adapters over the single `ProcessRunner` and
    /// the DPAPI-backed `SecretStore`.
    pub fn production() -> Self {
        let process: Arc<dyn crate::domain::ports::process::ProcessPort> = Arc::new(ProcessRunner);
        let openclaw = Arc::new(OpenClawAdapter::new(
            Arc::clone(&process),
            default_openclaw_search_dirs(),
        ));
        let config = Arc::new(OpenClawConfigAdapter::new(Arc::clone(&process)));
        let secrets: Arc<dyn SecretStorePort> = Arc::new(SecretStore::production());
        Self::new(openclaw, config, secrets)
    }

    fn executable(&self) -> Result<PathBuf, AppError> {
        match self.openclaw.detect_executable() {
            crate::domain::models::ExecutableDetection::Found { path } => Ok(path),
            crate::domain::models::ExecutableDetection::NotFound => {
                Err(AppError::openclaw_not_found())
            }
        }
    }

    /// All providers with computed API-key registration state.
    pub fn list_providers(&self) -> Result<Vec<ProviderSummary>, AppError> {
        let exe = self.executable()?;
        let providers = self.config.read_providers(&exe)?;
        Ok(providers
            .into_iter()
            .map(|provider| {
                let registered = provider.api_key == ProviderApiKey::Managed
                    && self.secrets.contains(&secret_key_id(&provider.id));
                ProviderSummary {
                    api_key_registered: registered,
                    id: provider.id,
                    base_url: provider.base_url,
                    api: provider.api,
                    model_count: provider.models.len(),
                }
            })
            .collect())
    }

    /// Full detail of one provider (redacted; never includes key values).
    pub fn get_provider(&self, provider_id: &str) -> Result<ProviderDetail, AppError> {
        validate_provider_id(provider_id)?;
        let exe = self.executable()?;
        self.config
            .read_providers(&exe)?
            .into_iter()
            .find(|provider| provider.id == provider_id)
            .ok_or_else(|| {
                AppError::openclaw_config_read_failed(format!("provider not found: {provider_id}"))
            })
    }

    /// Adds a new provider or updates an existing one.
    ///
    /// New: one whole-entry write (`--merge`). Existing: field-level subpath
    /// writes (`baseUrl`, `api`, `models`) so the provider's `apiKey`
    /// (redacted on read) is never rewritten or lost.
    pub fn save_provider(&self, input: &ProviderInput) -> Result<(), AppError> {
        // 1. Validate everything before any process run (0 runs on failure).
        for model in &input.models {
            if let Some(levels) = &model.supported_reasoning_efforts {
                for level in levels {
                    ThinkingLevel::parse(level).ok_or_else(|| {
                        AppError::invalid_input(
                            "thinking-level-invalid",
                            "supportedReasoningEfforts",
                            level,
                        )
                    })?;
                }
            }
        }
        let models: Vec<ModelEntry> = input.models.iter().map(to_model_entry).collect();
        validate_provider(&input.id, input.base_url.as_deref(), &input.api, &models)?;

        let exe = self.executable()?;
        let existing = self
            .config
            .read_providers(&exe)?
            .into_iter()
            .find(|provider| provider.id == input.id);

        if existing.is_some() {
            // Update: subpath writes; the apiKey field is left untouched.
            if let Some(base_url) = &input.base_url {
                self.config.write(
                    &exe,
                    &format!("models.providers.{}.baseUrl", input.id),
                    &format!("{:?}", base_url),
                    WriteMode::Plain,
                )?;
            }
            self.config.write(
                &exe,
                &format!("models.providers.{}.api", input.id),
                &format!("{:?}", input.api),
                WriteMode::Plain,
            )?;
            let models_json = serde_json::to_string(&models).map_err(|err| {
                AppError::openclaw_config_write_failed(format!("encode models: {err}"))
            })?;
            self.config.write(
                &exe,
                &format!("models.providers.{}.models", input.id),
                &models_json,
                WriteMode::Replace,
            )?;
        } else {
            // Create: whole entry (the apiKey field is never part of the
            // payload — S7). `null` fields are omitted.
            let mut payload = serde_json::Map::new();
            if let Some(base_url) = &input.base_url {
                payload.insert(
                    "baseUrl".to_string(),
                    serde_json::Value::String(base_url.clone()),
                );
            }
            payload.insert(
                "api".to_string(),
                serde_json::Value::String(input.api.clone()),
            );
            let models_value = serde_json::to_value(&models).map_err(|err| {
                AppError::openclaw_config_write_failed(format!("encode models: {err}"))
            })?;
            payload.insert("models".to_string(), models_value);
            let payload = serde_json::Value::Object(payload).to_string();
            self.config.write(
                &exe,
                &format!("models.providers.{}", input.id),
                &payload,
                WriteMode::Merge,
            )?;
        }
        Ok(())
    }

    /// Deletes a provider and, with it, any ClawDesk-managed API key
    /// (DPAPI entry + exec provider declaration when it is the last).
    pub fn delete_provider(&self, provider_id: &str) -> Result<(), AppError> {
        validate_provider_id(provider_id)?;
        let exe = self.executable()?;
        let existing = self
            .config
            .read_providers(&exe)?
            .into_iter()
            .find(|provider| provider.id == provider_id)
            .ok_or_else(|| {
                AppError::openclaw_config_read_failed(format!("provider not found: {provider_id}"))
            })?;
        let had_managed_key = existing.api_key == ProviderApiKey::Managed;

        self.config
            .unset(&exe, &format!("models.providers.{provider_id}"))?;

        if had_managed_key {
            self.secrets.delete(&secret_key_id(provider_id))?;
            if !self
                .config
                .read_providers(&exe)?
                .iter()
                .any(|provider| provider.api_key == ProviderApiKey::Managed)
            {
                self.config.unset(&exe, "secrets.providers.clawdesk")?;
            }
        }
        Ok(())
    }

    /// All model rows (`openclaw models list`, read-only).
    pub fn list_models(&self) -> Result<Vec<ModelRow>, AppError> {
        let exe = self.executable()?;
        self.config.read_models(&exe)
    }

    pub fn get_default_model(&self) -> Result<Option<String>, AppError> {
        let exe = self.executable()?;
        self.config.read_default_model(&exe)
    }

    pub fn set_default_model(&self, model_ref: &str) -> Result<(), AppError> {
        validate_model_ref(model_ref)?;
        let exe = self.executable()?;
        self.config.set_default_model(&exe, model_ref)
    }

    pub fn get_thinking_default(&self) -> Result<Option<ThinkingLevel>, AppError> {
        let exe = self.executable()?;
        self.config.read_thinking_default(&exe)
    }

    /// Sets the global thinking default. The level is validated (stable
    /// `thinking-level-invalid`) before any process run.
    pub fn set_thinking_default(&self, level: &str) -> Result<(), AppError> {
        let level = ThinkingLevel::parse(level).ok_or_else(|| {
            AppError::invalid_input("thinking-level-invalid", "thinking level", level)
        })?;
        let exe = self.executable()?;
        self.config.write(
            &exe,
            "agents.defaults.thinkingDefault",
            &format!("{:?}", level.wire_id()),
            WriteMode::Plain,
        )
    }
}

/// Converts a UI model input into the config-format entry.
fn to_model_entry(input: &ModelInput) -> ModelEntry {
    let modalities = if input.input.is_empty() {
        vec!["text".to_string()]
    } else {
        input.input.clone()
    };
    let compat = input.supports_reasoning_effort.then(|| {
        crate::domain::models::models::ModelCompat {
            supports_reasoning_effort: true,
            supported_reasoning_efforts: input.supported_reasoning_efforts.as_ref().map(|levels| {
                levels
                    .iter()
                    // Validated in `save_provider` before this conversion.
                    .map(|level| ThinkingLevel::parse(level).expect("validated in save_provider"))
                    .collect()
            }),
        }
    });
    ModelEntry {
        id: input.id.clone(),
        name: input.name.clone(),
        reasoning: input.reasoning,
        input: modalities,
        context_window: input.context_window,
        max_tokens: input.max_tokens,
        compat,
    }
}

#[cfg(test)]
#[path = "tests_models.rs"]
mod tests;
