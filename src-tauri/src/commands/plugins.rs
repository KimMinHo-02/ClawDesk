//! Tauri IPC commands for the Phase 4 plugins feature.
//!
//! Request bodies are camelCase at the IPC boundary (`pluginId`); the
//! response rows and runtime surfaces use the camelCase wire shapes from
//! the domain models. Errors are the unified `AppError` (stable code +
//! masked message).

use crate::application::PluginsService;
use crate::domain::models::skills::{PluginRow, PluginRuntime};
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

/// All plugins (`openclaw plugins list --json`, cold read).
#[tauri::command(rename = "list-plugins")]
pub async fn list_plugins() -> Result<Vec<PluginRow>, AppError> {
    let service = PluginsService::production();
    run_blocking(move || service.list_plugins()).await
}

/// Enables/disables a plugin (`openclaw plugins enable/disable <id>`).
///
/// On failure the frontend re-queries `list-plugins` and shows the actual
/// state (no optimistic updates).
#[tauri::command(rename = "set-plugin-enabled")]
pub async fn set_plugin_enabled(plugin_id: String, enabled: bool) -> Result<(), AppError> {
    let service = PluginsService::production();
    run_blocking(move || service.set_plugin_enabled(&plugin_id, enabled)).await
}

/// The live runtime surface of one plugin
/// (`openclaw plugins inspect <id> --runtime --json`).
///
/// On-demand only: this loads plugin modules, so the UI must not call it
/// while loading the list.
#[tauri::command(rename = "get-plugin-runtime")]
pub async fn get_plugin_runtime(plugin_id: String) -> Result<PluginRuntime, AppError> {
    let service = PluginsService::production();
    run_blocking(move || service.get_plugin_runtime(&plugin_id)).await
}
