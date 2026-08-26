//! Tauri IPC commands for the Phase 8 profile/update/diagnostics feature.
//!
//! All four commands are read-only (PRODUCT_CONTRACT §4.7). Errors are the
//! unified `AppError` (stable code + masked message); the log limit is
//! format-validated in the service layer before any CLI call (S2).

use crate::application::DiagnosticsService;
use crate::domain::models::diagnostics::{AgentRow, LogsResult, UpdateStatusDetail};
use crate::domain::models::openclaw::GatewayStatus;
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

/// `get-gateway-status`: Phase 1 gateway status (read-only reuse).
#[tauri::command(rename = "get-gateway-status")]
pub async fn get_gateway_status() -> Result<GatewayStatus, AppError> {
    let service = DiagnosticsService::production();
    run_blocking(move || service.gateway_status()).await
}

/// `get-update-status`: update state plus current/latest versions.
///
/// Fail-soft: an undeterminable state is `state:"unknown"` with no versions
/// (a value, not an error — Phase 1 policy).
#[tauri::command(rename = "get-update-status")]
pub async fn get_update_status() -> Result<UpdateStatusDetail, AppError> {
    let service = DiagnosticsService::production();
    run_blocking(move || service.update_status()).await
}

/// `get-agents`: all agent rows (read-only display).
#[tauri::command(rename = "get-agents")]
pub async fn get_agents() -> Result<Vec<AgentRow>, AppError> {
    let service = DiagnosticsService::production();
    run_blocking(move || service.agents()).await
}

/// `get-logs`: one-shot tail of at most `limit` lines (1..=1000, validated
/// before any CLI call; never `--follow`).
#[tauri::command(rename = "get-logs")]
pub async fn get_logs(limit: u32) -> Result<LogsResult, AppError> {
    let service = DiagnosticsService::production();
    run_blocking(move || service.logs(limit)).await
}
