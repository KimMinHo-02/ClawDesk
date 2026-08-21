//! Tauri IPC command layer (created in Phase 2).
//!
//! The frontend command names (kebab-case) live in exactly one place:
//! `src/lib/tauri/` on the frontend (architecture §5). Rust command names are
//! snake_case; Tauri maps `detect_environment` → `detect-environment` and
//! `install_openclaw` → `install-openclaw` automatically.
//!
//! Errors are returned as the unified `AppError` (stable `code` + masked
//! `message`); infrastructure details never reach the frontend raw.

pub mod models;

use crate::application::{EnvironmentReport, EnvironmentService, InstallResult, InstallService};
use crate::error::AppError;

/// Runs a blocking task on Tauri's blocking thread pool so long process
/// work (e.g. a 15-minute npm install) never blocks the UI thread.
async fn run_blocking<T, F>(f: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, AppError> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|_| AppError::new("process-failed", "command task failed unexpectedly"))?
}

/// Exposes the Phase 1 `EnvironmentService` over IPC: current environment
/// state before install, re-checkable after install completes.
#[tauri::command]
pub async fn detect_environment() -> Result<EnvironmentReport, AppError> {
    let service = EnvironmentService::production();
    run_blocking(move || service.detect_environment()).await
}

/// Installs `openclaw@latest` (or returns the existing install as-is).
///
/// No arguments: the install has zero user-controlled input — the target is
/// always `openclaw@latest`.
#[tauri::command]
pub async fn install_openclaw() -> Result<InstallResult, AppError> {
    let service = InstallService::production();
    run_blocking(move || service.install_openclaw()).await
}
