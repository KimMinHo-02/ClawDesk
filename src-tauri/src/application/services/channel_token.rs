//! Channel token use case (Phase 6, S7).
//!
//! Token lifecycle (contract order, `PHASE_06.md` §1 — mirrors the Phase 3
//! `ApiKeyService`):
//! - set: validate channel/token → the current token field must be absent or
//!   a ClawDesk ref (an externally managed value is rejected with zero
//!   writes) → store the value in DPAPI first → ensure the shared
//!   `clawdesk` exec provider declaration (idempotent) → write the exec
//!   SecretRef to `channels.discord.token` / `channels.telegram.botToken`.
//!   A DPAPI failure leaves the config untouched (zero writes); a later
//!   config-write failure leaves an orphan store entry that
//!   `delete_channel_token` cleans up.
//! - delete: managed ref present → unset the ref → delete the DPAPI entry.
//!   Orphan (ref absent but store entry present) → store-only cleanup.
//!   Then remove the shared exec provider declaration only when **neither**
//!   surface holds a ClawDesk-managed ref anymore: the provider surface
//!   (`models.providers.*.apiKey`) or the channel surface (both channel
//!   token paths).
//!
//! The token value never appears in config, argv, logs, or errors
//! (S3/S7/S8) — only the exec SecretRef and the DPAPI entry carry it.

use std::path::PathBuf;
use std::sync::Arc;

use super::api_key::ensure_clawdesk_secret_provider;
use super::environment::default_openclaw_search_dirs;
use crate::domain::models::channels::{
    channel_secret_key_id, channel_secret_ref, channel_token_path, classify_channel_token_state,
    validate_channel_id, validate_channel_token, ChannelTokenState, ChannelTokenStatus,
    SUPPORTED_CHANNELS,
};
use crate::domain::models::models::ProviderApiKey;
use crate::domain::ports::openclaw::OpenClawPort;
use crate::domain::ports::openclaw_config::{OpenClawConfigPort, WriteMode};
use crate::domain::ports::secrets::SecretStorePort;
use crate::error::AppError;
use crate::infrastructure::openclaw::{OpenClawAdapter, OpenClawConfigAdapter};
use crate::infrastructure::process::ProcessRunner;
use crate::infrastructure::secrets::SecretStore;

/// Use case layer: composes the OpenClaw executable, config, and secret
/// store ports plus the resolver binary location.
pub struct ChannelTokenService {
    openclaw: Arc<dyn OpenClawPort>,
    config: Arc<dyn OpenClawConfigPort>,
    secrets: Arc<dyn SecretStorePort>,
    resolver: PathBuf,
}

impl ChannelTokenService {
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
    /// executable (same binary the Phase 3 API-key lifecycle uses).
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

    /// Registers (or re-registers) the channel token.
    pub fn set_channel_token(&self, channel: &str, token: &str) -> Result<(), AppError> {
        validate_channel_id(channel)?;
        validate_channel_token(token)?;
        let exe = self.executable()?;
        let token_path = channel_token_path(channel);
        // An externally managed token (plaintext or a foreign ref) must be
        // changed in OpenClaw first — DPAPI and config writes stay at zero.
        if let Some(current) = self.config.read_raw(&exe, &token_path)? {
            if classify_channel_token_state(Some(&current)) != ChannelTokenState::Managed {
                return Err(AppError::secret_ref_registration_failed(
                    "the channel token is managed outside ClawDesk; change it in OpenClaw first",
                ));
            }
        }

        // DPAPI first (contract order): on failure the config receives zero
        // writes. A later config-write failure leaves an orphan store entry,
        // which `delete_channel_token` cleans up.
        self.secrets.set(&channel_secret_key_id(channel), token)?;

        ensure_clawdesk_secret_provider(&*self.config, &exe, &self.resolver)?;

        let reference = channel_secret_ref(channel);
        let reference_json = serde_json::to_string(&reference).map_err(|err| {
            AppError::secret_ref_registration_failed(format!("encode SecretRef: {err}"))
        })?;
        self.config
            .write(&exe, &token_path, &reference_json, WriteMode::Plain)
            .map_err(|err| AppError::secret_ref_registration_failed(err.message))
    }

    /// Deletes the channel's managed token (config ref + DPAPI entry), or
    /// cleans up an orphan store entry (ref absent, value present — left by
    /// a `set_channel_token` whose config write failed after the DPAPI
    /// step). Then the shared exec provider declaration is removed only
    /// when neither the provider surface nor the channel surface holds a
    /// ClawDesk-managed ref anymore.
    pub fn delete_channel_token(&self, channel: &str) -> Result<(), AppError> {
        validate_channel_id(channel)?;
        let key_id = channel_secret_key_id(channel);
        let exe = self.executable()?;
        let token_path = channel_token_path(channel);
        let state =
            classify_channel_token_state(self.config.read_raw(&exe, &token_path)?.as_deref());
        match state {
            ChannelTokenState::Managed => {
                self.config.unset(&exe, &token_path)?;
                self.secrets.delete(&key_id)?;
            }
            // External: the config value is not ours. Only an orphan store
            // entry (ours) is cleaned up; the external value stays.
            ChannelTokenState::External | ChannelTokenState::Absent => {
                if self.secrets.contains(&key_id) {
                    self.secrets.delete(&key_id)?;
                } else {
                    return Err(AppError::channel_token_not_found(channel));
                }
            }
        }

        // Dual-surface declaration cleanup: unset `secrets.providers.clawdesk`
        // only when no provider key AND no channel token ref is managed.
        let provider_managed = self
            .config
            .read_providers(&exe)?
            .iter()
            .any(|provider| provider.api_key == ProviderApiKey::Managed);
        let mut channel_managed = false;
        for other in SUPPORTED_CHANNELS {
            let current = self.config.read_raw(&exe, &channel_token_path(other))?;
            if classify_channel_token_state(current.as_deref()) == ChannelTokenState::Managed {
                channel_managed = true;
                break;
            }
        }
        if !provider_managed && !channel_managed {
            self.config.unset(&exe, "secrets.providers.clawdesk")?;
        }
        Ok(())
    }

    /// Registration state from the ClawDesk secret store index (non-secret).
    pub fn list_channel_tokens(&self) -> Result<Vec<ChannelTokenStatus>, AppError> {
        Ok(self
            .secrets
            .list_key_ids()
            .into_iter()
            .filter_map(|key_id| {
                let rest = key_id.strip_prefix("channels/")?;
                let (channel, field) = rest.rsplit_once('/')?;
                if !matches!(field, "token" | "botToken") || channel.is_empty() {
                    return None;
                }
                Some(ChannelTokenStatus {
                    channel: channel.to_string(),
                    registered: true,
                })
            })
            .collect())
    }
}

#[cfg(test)]
#[path = "tests_channel_token.rs"]
mod tests;
