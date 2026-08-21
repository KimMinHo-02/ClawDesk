//! Tauri IPC commands for the Phase 3 model/provider/API-key/reasoning
//! feature.
//!
//! Request bodies are camelCase (JS convention) at the IPC boundary;
//! responses use the OpenClaw config-format wire shapes (also camelCase).
//! Every command runs blocking work on Tauri's blocking pool (config CLI
//! calls, DPAPI calls) so the UI thread is never blocked.

use crate::application::{ApiKeyService, ModelService, ProviderInput};
use crate::domain::models::models::{ApiKeyStatus, ModelRow, ProviderDetail, ProviderSummary};
use crate::error::AppError;

/// Runs a blocking task on Tauri's blocking thread pool (config CLI calls,
/// DPAPI calls) so the UI thread is never blocked.
async fn run_blocking<T, F>(f: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, AppError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|_| AppError::new("process-failed", "command task failed unexpectedly"))?
}

/// Lists all providers with computed API-key registration state.
#[tauri::command]
pub async fn list_providers() -> Result<Vec<ProviderSummary>, AppError> {
    let service = ModelService::production();
    run_blocking(move || service.list_providers()).await
}

/// Full detail of one provider (redacted; never includes key values).
#[tauri::command]
pub async fn get_provider(provider_id: String) -> Result<ProviderDetail, AppError> {
    let service = ModelService::production();
    run_blocking(move || service.get_provider(&provider_id)).await
}

/// Adds a new provider or updates an existing one.
///
/// The body never contains an API key (S7); key management is a separate
/// command pair.
#[tauri::command]
pub async fn save_provider(provider: ProviderInput) -> Result<(), AppError> {
    let service = ModelService::production();
    run_blocking(move || service.save_provider(&provider)).await
}

/// Deletes a provider (and its ClawDesk-managed API key, if any).
#[tauri::command]
pub async fn delete_provider(provider_id: String) -> Result<(), AppError> {
    let service = ModelService::production();
    run_blocking(move || service.delete_provider(&provider_id)).await
}

/// All models (`openclaw models list`, read-only).
#[tauri::command]
pub async fn list_models() -> Result<Vec<ModelRow>, AppError> {
    let service = ModelService::production();
    run_blocking(move || service.list_models()).await
}

/// The current default `provider/model` reference, if set.
#[tauri::command]
pub async fn get_default_model() -> Result<Option<String>, AppError> {
    let service = ModelService::production();
    run_blocking(move || service.get_default_model()).await
}

/// Sets the default model (`openclaw models set <provider/model>`).
#[tauri::command]
pub async fn set_default_model(model_ref: String) -> Result<(), AppError> {
    let service = ModelService::production();
    run_blocking(move || service.set_default_model(&model_ref)).await
}

/// The current global thinking (reasoning effort) default, if set.
#[tauri::command]
pub async fn get_reasoning_default() -> Result<Option<String>, AppError> {
    let service = ModelService::production();
    run_blocking(move || {
        service
            .get_thinking_default()
            .map(|level| level.map(|level| level.wire_id().to_string()))
    })
    .await
}

/// Sets the global thinking (reasoning effort) default.
///
/// `level` is a wire id (`off|minimal|low|medium|high|xhigh|adaptive|max|
/// ultra`); unknown values are rejected with `thinking-level-invalid`.
#[tauri::command]
pub async fn set_reasoning_default(level: String) -> Result<(), AppError> {
    let service = ModelService::production();
    run_blocking(move || service.set_thinking_default(&level)).await
}

/// Registers (or re-registers) the provider's API key.
///
/// The key travels to DPAPI only (S7); OpenClaw receives an exec SecretRef.
#[tauri::command]
pub async fn set_provider_api_key(provider_id: String, api_key: String) -> Result<(), AppError> {
    let service = ApiKeyService::production();
    run_blocking(move || service.set_api_key(&provider_id, &api_key)).await
}

/// Deletes the provider's managed API key (config ref + DPAPI entry).
#[tauri::command]
pub async fn delete_provider_api_key(provider_id: String) -> Result<(), AppError> {
    let service = ApiKeyService::production();
    run_blocking(move || service.delete_api_key(&provider_id)).await
}

/// Registration state of all ClawDesk-managed API keys (non-secret).
#[tauri::command]
pub async fn list_api_keys() -> Result<Vec<ApiKeyStatus>, AppError> {
    let service = ApiKeyService::production();
    run_blocking(move || service.list_api_keys()).await
}
