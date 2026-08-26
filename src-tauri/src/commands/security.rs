//! Tauri IPC commands for the Phase 5 security profile + audit feature.
//!
//! Request bodies are camelCase at the IPC boundary (`profile`,
//! `profileId`); errors are the unified `AppError` (stable code + masked
//! message).

use crate::application::{SecurityProfileList, SecurityProfileService};
use crate::domain::models::tools::{SecurityAuditResult, SecurityProfile};
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

/// Builtins + user profiles + applied-state determination. A tool-policy
/// read failure degrades to `policyReadFailed: true` (the list is still
/// shown); a corrupt profile store is a stable error.
#[tauri::command(rename = "list-security-profiles")]
pub async fn list_security_profiles() -> Result<SecurityProfileList, AppError> {
    let service = SecurityProfileService::production();
    run_blocking(move || service.list_security_profiles()).await
}

/// Inserts or replaces a user profile (upsert). Builtins are immutable
/// (`security-profile-conflict`). No config write happens on save.
#[tauri::command(rename = "save-security-profile")]
pub async fn save_security_profile(profile: SecurityProfile) -> Result<(), AppError> {
    let service = SecurityProfileService::production();
    run_blocking(move || service.save_security_profile(&profile)).await
}

/// Deletes a user profile. Builtin ids and unknown ids are
/// `security-profile-not-found`.
#[tauri::command(rename = "delete-security-profile")]
pub async fn delete_security_profile(profile_id: String) -> Result<(), AppError> {
    let service = SecurityProfileService::production();
    run_blocking(move || service.delete_security_profile(&profile_id)).await
}

/// Applies the profile's four fields to the OpenClaw config, in a fixed
/// order (each dry-run → commit). On failure the UI re-queries the actual
/// state (no optimistic updates).
#[tauri::command(rename = "apply-security-profile")]
pub async fn apply_security_profile(profile_id: String) -> Result<(), AppError> {
    let service = SecurityProfileService::production();
    run_blocking(move || service.apply_security_profile(&profile_id)).await
}

/// Cold, read-only security audit (`openclaw security audit --json`).
/// Never `--deep`/`--fix`/`--token`/`--password`. On failure the UI shows
/// "audit failed" — it never assumes a clean state.
#[tauri::command(rename = "run-security-audit")]
pub async fn run_security_audit() -> Result<SecurityAuditResult, AppError> {
    let service = SecurityProfileService::production();
    run_blocking(move || service.run_security_audit()).await
}
