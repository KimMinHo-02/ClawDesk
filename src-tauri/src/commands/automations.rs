//! Tauri IPC commands for the Phase 7 automations feature.
//!
//! Request bodies are camelCase at the IPC boundary (`jobId`,
//! `scheduleKind`, `scheduleValue`, `scheduleTz`, `payloadKind`, `text`,
//! `wake`); errors are the unified `AppError` (stable code + masked
//! message). All input is format-validated in the service layer before any
//! CLI call (S2); the session pairing is fixed in Rust (the wire carries no
//! session field).

use crate::application::AutomationService;
use crate::domain::models::automations::{AutomationCreated, AutomationJob, AutomationJobList};
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

/// `get-automations`: all job rows including disabled (read-only).
#[tauri::command(rename = "get-automations")]
pub async fn get_automations() -> Result<AutomationJobList, AppError> {
    let service = AutomationService::production();
    run_blocking(move || {
        Ok(AutomationJobList {
            jobs: service.list_automations()?,
        })
    })
    .await
}

/// `get-automation`: one job detail (id pre-validated, fail-closed).
#[tauri::command(rename = "get-automation")]
pub async fn get_automation(job_id: String) -> Result<AutomationJob, AppError> {
    let service = AutomationService::production();
    run_blocking(move || service.get_automation(&job_id)).await
}

/// `create-automation`: reminder (`--system-event` + `main` session) or
/// task (`--message` + `isolated` session); the pairing is fixed in Rust.
/// Returns the new job id (fail-soft extraction — a missing id is a
/// structured error).
#[tauri::command(rename = "create-automation")]
pub async fn create_automation(
    name: String,
    schedule_kind: String,
    schedule_value: String,
    schedule_tz: Option<String>,
    payload_kind: String,
    text: String,
    wake: Option<String>,
) -> Result<AutomationCreated, AppError> {
    let service = AutomationService::production();
    run_blocking(move || {
        let job_id = service.create_automation(
            &name,
            &schedule_kind,
            &schedule_value,
            schedule_tz.as_deref(),
            &payload_kind,
            &text,
            wake.as_deref(),
        )?;
        Ok(AutomationCreated { job_id })
    })
    .await
}

/// `update-automation`: same field set as create; the payload kind cannot
/// change (kind change = delete + recreate, blocked by the UI).
#[tauri::command(rename = "update-automation")]
#[allow(clippy::too_many_arguments)] // the Phase 7 contract fixes this field set
pub async fn update_automation(
    job_id: String,
    name: String,
    schedule_kind: String,
    schedule_value: String,
    schedule_tz: Option<String>,
    payload_kind: String,
    text: String,
    wake: Option<String>,
) -> Result<(), AppError> {
    let service = AutomationService::production();
    run_blocking(move || {
        service.update_automation(
            &job_id,
            &name,
            &schedule_kind,
            &schedule_value,
            schedule_tz.as_deref(),
            &payload_kind,
            &text,
            wake.as_deref(),
        )
    })
    .await
}

/// `set-automation-enabled`: `automations enable|disable <jobId> --json`.
#[tauri::command(rename = "set-automation-enabled")]
pub async fn set_automation_enabled(job_id: String, enabled: bool) -> Result<(), AppError> {
    let service = AutomationService::production();
    run_blocking(move || service.set_automation_enabled(&job_id, enabled)).await
}

/// `delete-automation`: `automations remove <jobId> --json`.
#[tauri::command(rename = "delete-automation")]
pub async fn delete_automation(job_id: String) -> Result<(), AppError> {
    let service = AutomationService::production();
    run_blocking(move || service.remove_automation(&job_id)).await
}
