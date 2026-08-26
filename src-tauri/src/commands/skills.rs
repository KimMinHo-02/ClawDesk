//! Tauri IPC commands for the Phase 4 skills feature.
//!
//! Request bodies are camelCase at the IPC boundary (`skillName`); the
//! response rows use the camelCase wire shapes from the domain models.
//! Errors are the unified `AppError` (stable code + masked message).

use crate::application::SkillsService;
use crate::domain::models::skills::SkillRow;
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

/// All skills (`openclaw skills list --json`, read-only).
#[tauri::command(rename = "list-skills")]
pub async fn list_skills() -> Result<Vec<SkillRow>, AppError> {
    let service = SkillsService::production();
    run_blocking(move || service.list_skills()).await
}

/// Toggles `skills.entries.<skillName>.enabled` (two-step config write).
///
/// Unknown names fail with `skill-not-found` and perform zero writes; the
/// change applies from the next new session (UI notice, not a CLI concern).
#[tauri::command(rename = "set-skill-enabled")]
pub async fn set_skill_enabled(skill_name: String, enabled: bool) -> Result<(), AppError> {
    let service = SkillsService::production();
    run_blocking(move || service.set_skill_enabled(&skill_name, enabled)).await
}
