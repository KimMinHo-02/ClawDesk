//! Tauri IPC command for the Phase 8.1 one-shot Node.js update.
//!
//! Read the detection-first guard chain in `NodeUpdateService`: no OS
//! mutation happens unless the detected Node version is unsupported.
//! Errors are the unified `AppError` (stable code + masked message).

use crate::application::NodeUpdateService;
use crate::domain::models::windows::NodeDetection;
use crate::error::AppError;

/// Runs a blocking task on Tauri's blocking thread pool (winget can run
/// for up to 15 minutes) so the UI thread is never blocked.
async fn run_blocking<T, F>(f: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, AppError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|_| AppError::new("process-failed", "command task failed unexpectedly"))?
}

/// `update-node`: one-shot Node.js update (winget) for an unsupported
/// detected version. Returns the post-update detection.
#[tauri::command(rename = "update-node")]
pub async fn update_node() -> Result<NodeDetection, AppError> {
    let service = NodeUpdateService::production();
    run_blocking(move || service.update_node()).await
}
