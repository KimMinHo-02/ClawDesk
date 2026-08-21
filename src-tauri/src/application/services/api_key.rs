//! Provider API key use case (Phase 3, S7).
//!
//! Key lifecycle (contract order, `PHASE_03.md` §1 API key):
//! - set: validate → provider exists → store the value in DPAPI first →
//!   ensure the `clawdesk` exec provider declaration (idempotent) → write
//!   the exec SecretRef to `models.providers.<id>.apiKey`. A DPAPI failure
//!   leaves the config untouched (zero writes); a later config-write failure
//!   leaves an orphan store entry that `delete_api_key` cleans up.
//! - delete: managed ref present → unset the ref → delete the DPAPI entry.
//!   Orphan (ref absent but store entry present) → store-only cleanup.
//!   Then remove the exec provider declaration when no provider holds a
//!   managed key anymore.
//!
//! The key value never appears in config, argv, logs, or errors (S3/S7/S8).

use std::path::PathBuf;
use std::sync::Arc;

use super::environment::default_openclaw_search_dirs;
use crate::domain::models::models::{
    clawdesk_secret_ref, secret_key_id, validate_provider_id, ApiKeyStatus, ProviderApiKey,
};
use crate::domain::ports::openclaw::OpenClawPort;
use crate::domain::ports::openclaw_config::{OpenClawConfigPort, WriteMode};
use crate::domain::ports::secrets::SecretStorePort;
use crate::error::AppError;
use crate::infrastructure::openclaw::{OpenClawAdapter, OpenClawConfigAdapter};
use crate::infrastructure::process::ProcessRunner;
use crate::infrastructure::secrets::SecretStore;

/// Use case layer: composes the OpenClaw executable, config, and secret
/// store ports plus the resolver binary location.
pub struct ApiKeyService {
    openclaw: Arc<dyn OpenClawPort>,
    config: Arc<dyn OpenClawConfigPort>,
    secrets: Arc<dyn SecretStorePort>,
    resolver: PathBuf,
}

impl ApiKeyService {
    pub fn new(
        openclaw: Arc<dyn OpenClawPort>,
        config: Arc<dyn OpenClawConfigPort>,
        secrets: Arc<dyn SecretStorePort>,
        resolver: PathBuf,
    ) -> Self {
        Self {
            openclaw,
            config,
            secrets,
            resolver,
        }
    }

    /// Production wiring. The resolver binary lives next to the running
    /// executable (cargo target dir in dev; the app dir in the NSIS install,
    /// Phase 10).
    pub fn production() -> Self {
        let process: Arc<dyn crate::domain::ports::process::ProcessPort> = Arc::new(ProcessRunner);
        let openclaw = Arc::new(OpenClawAdapter::new(
            Arc::clone(&process),
            default_openclaw_search_dirs(),
        ));
        let config = Arc::new(OpenClawConfigAdapter::new(Arc::clone(&process)));
        let secrets: Arc<dyn SecretStorePort> = Arc::new(SecretStore::production());
        let resolver = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|dir| dir.to_path_buf()))
            .unwrap_or_default()
            .join("clawdesk-secret-resolver.exe");
        Self::new(openclaw, config, secrets, resolver)
    }

    fn executable(&self) -> Result<PathBuf, AppError> {
        match self.openclaw.detect_executable() {
            crate::domain::models::ExecutableDetection::Found { path } => Ok(path),
            crate::domain::models::ExecutableDetection::NotFound => {
                Err(AppError::openclaw_not_found())
            }
        }
    }

    /// Registers (or re-registers) the provider's API key.
    pub fn set_api_key(&self, provider_id: &str, api_key: &str) -> Result<(), AppError> {
        validate_provider_id(provider_id)?;
        if api_key.trim().is_empty() {
            return Err(AppError::openclaw_config_invalid("API key is empty"));
        }
        let exe = self.executable()?;
        let provider = self
            .config
            .read_providers(&exe)?
            .into_iter()
            .find(|provider| provider.id == provider_id)
            .ok_or_else(|| {
                AppError::openclaw_config_read_failed(format!("provider not found: {provider_id}"))
            })?;
        if provider.api_key != ProviderApiKey::Managed && provider.api_key != ProviderApiKey::Absent
        {
            return Err(AppError::secret_ref_registration_failed(
                "the provider has an externally managed apiKey; change it in OpenClaw first",
            ));
        }

        // DPAPI first (contract order): on failure the config receives zero
        // writes. A later config-write failure leaves an orphan store entry,
        // which `delete_api_key` cleans up.
        self.secrets.set(&secret_key_id(provider_id), api_key)?;

        self.ensure_secret_provider(&exe)?;

        let reference = clawdesk_secret_ref(provider_id);
        let reference_json = serde_json::to_string(&reference).map_err(|err| {
            AppError::secret_ref_registration_failed(format!("encode SecretRef: {err}"))
        })?;
        self.config
            .write(
                &exe,
                &format!("models.providers.{provider_id}.apiKey"),
                &reference_json,
                WriteMode::Plain,
            )
            .map_err(|err| AppError::secret_ref_registration_failed(err.message))
    }

    /// Deletes the provider's managed API key (config ref + DPAPI entry), or
    /// cleans up an orphan store entry (ref absent, value present — left by
    /// a `set_api_key` whose config write failed after the DPAPI step).
    pub fn delete_api_key(&self, provider_id: &str) -> Result<(), AppError> {
        validate_provider_id(provider_id)?;
        let key_id = secret_key_id(provider_id);
        let exe = self.executable()?;
        let provider = self
            .config
            .read_providers(&exe)?
            .into_iter()
            .find(|provider| provider.id == provider_id)
            .ok_or_else(|| {
                AppError::openclaw_config_read_failed(format!("provider not found: {provider_id}"))
            })?;
        if provider.api_key == ProviderApiKey::Managed {
            self.config
                .unset(&exe, &format!("models.providers.{provider_id}.apiKey"))?;
            self.secrets.delete(&key_id)?;
        } else if self.secrets.contains(&key_id) {
            // Orphan cleanup: the config ref is absent, so no apiKey write
            // happens — only the store entry (and possibly the declaration)
            // is removed.
            self.secrets.delete(&key_id)?;
        } else {
            return Err(AppError::openclaw_config_read_failed(
                "the provider has no ClawDesk-managed API key",
            ));
        }

        if !self
            .config
            .read_providers(&exe)?
            .iter()
            .any(|provider| provider.api_key == ProviderApiKey::Managed)
        {
            self.config.unset(&exe, "secrets.providers.clawdesk")?;
        }
        Ok(())
    }

    /// Registration state from the ClawDesk secret store index (non-secret).
    pub fn list_api_keys(&self) -> Result<Vec<ApiKeyStatus>, AppError> {
        Ok(self
            .secrets
            .list_key_ids()
            .into_iter()
            .filter_map(|key_id| {
                let rest = key_id.strip_prefix("providers/")?;
                let provider_id = rest.strip_suffix("/apiKey")?;
                (!provider_id.is_empty()).then_some(ApiKeyStatus {
                    provider_id: provider_id.to_string(),
                    registered: true,
                })
            })
            .collect())
    }

    /// Ensures `secrets.providers.clawdesk` points at the current resolver
    /// binary (idempotent: no write when the declaration already matches).
    fn ensure_secret_provider(&self, exe: &std::path::Path) -> Result<(), AppError> {
        const DECLARATION_PATH: &str = "secrets.providers.clawdesk";
        let resolver_display = self.resolver.display().to_string();
        if let Some(current) = self.config.read_raw(exe, DECLARATION_PATH)? {
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&current) {
                if value.get("command").and_then(|command| command.as_str())
                    == Some(resolver_display.as_str())
                {
                    return Ok(());
                }
            }
        }
        let declaration = serde_json::json!({
            "source": "exec",
            "command": resolver_display,
            "timeoutMs": 5000,
            "jsonOnly": true,
        });
        self.config
            .write(
                exe,
                DECLARATION_PATH,
                &declaration.to_string(),
                WriteMode::Plain,
            )
            .map_err(|err| AppError::secret_ref_registration_failed(err.message))
    }
}

#[cfg(test)]
#[path = "tests_api_key.rs"]
mod tests;
