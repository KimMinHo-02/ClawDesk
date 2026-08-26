//! Tauri IPC commands for the Phase 5 tool policy feature.
//!
//! Request bodies are camelCase at the IPC boundary (`profile`, `entries`,
//! `mode`); errors are the unified `AppError` (stable code + masked
//! message).

use crate::application::ToolPolicyService;
use crate::domain::models::tools::ToolPolicy;
use crate::error::AppError;

/// Runs a blocking task on Tauri's blocking thread pool (CLI calls) so the
/// UI thread is never blocked.
async fn run_blocking<T, F>(f: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, AppError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|_| AppError::new("process-failed", "command task failed unexpectedly"))?
}

/// The current tool policy (`openclaw config get tools --json`, read-only,
/// redacted). Unset fields read as `null`/empty (fail-soft).
#[tauri::command(rename = "get-tool-policy")]
pub async fn get_tool_policy() -> Result<ToolPolicy, AppError> {
    let service = ToolPolicyService::production();
    run_blocking(move || service.get_tool_policy()).await
}

/// Sets `tools.profile` (enum-validated; two-step config write).
#[tauri::command(rename = "set-tool-profile")]
pub async fn set_tool_profile(profile: String) -> Result<(), AppError> {
    let service = ToolPolicyService::production();
    run_blocking(move || service.set_tool_profile(&profile)).await
}

/// Replaces the whole `tools.allow` array (entry-validated, `--replace`).
#[tauri::command(rename = "set-tool-allow")]
pub async fn set_tool_allow(entries: Vec<String>) -> Result<(), AppError> {
    let service = ToolPolicyService::production();
    run_blocking(move || service.set_tool_allow(&entries)).await
}

/// Replaces the whole `tools.deny` array (entry-validated, `--replace`).
/// Deny wins over allow.
#[tauri::command(rename = "set-tool-deny")]
pub async fn set_tool_deny(entries: Vec<String>) -> Result<(), AppError> {
    let service = ToolPolicyService::production();
    run_blocking(move || service.set_tool_deny(&entries)).await
}

/// Sets `tools.exec.mode` (enum-validated; two-step config write).
#[tauri::command(rename = "set-exec-mode")]
pub async fn set_exec_mode(mode: String) -> Result<(), AppError> {
    let service = ToolPolicyService::production();
    run_blocking(move || service.set_exec_mode(&mode)).await
}
