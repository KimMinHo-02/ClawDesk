//! OpenClaw automations port (Phase 7).
//!
//! Every method maps to a non-interactive `openclaw automations` CLI
//! invocation (structured argv via `ProcessPort` — S1/S2). Job ids, names,
//! schedule values, and payload text are validated before use and passed as
//! single argv elements. No `run`/`runs` (manual execution is a non-goal —
//! the Gateway scheduler is the execution surface), no command/script
//! payloads, no delivery routing, no per-job model/fallback/thinking
//! overrides.

use std::path::Path;

use crate::domain::models::automations::{AutomationJob, AutomationJobRow};
use crate::error::AppError;

pub trait OpenClawAutomationsPort: Send + Sync {
    /// `openclaw automations list --all --json` → all job rows including
    /// disabled (read-only, 30s).
    fn list_automations(&self, executable: &Path) -> Result<Vec<AutomationJobRow>, AppError>;

    /// `openclaw automations get <job_id> --json` → the job detail
    /// (read-only, 30s).
    fn get_automation(&self, executable: &Path, job_id: &str) -> Result<AutomationJob, AppError>;

    /// `openclaw automations add --name <name> <schedule flags>
    /// --session main --system-event <text> --wake <wake> --json` (reminder)
    /// or `--session isolated --message <text> --json` (task) → the new job
    /// id (30s). The session pairing is fixed by the payload kind.
    #[allow(clippy::too_many_arguments)] // the Phase 7 contract fixes this field set
    fn add_automation(
        &self,
        executable: &Path,
        name: &str,
        schedule_kind: &str,
        schedule_value: &str,
        schedule_tz: Option<&str>,
        payload_kind: &str,
        text: &str,
        wake: Option<&str>,
    ) -> Result<String, AppError>;

    /// `openclaw automations edit <job_id> --name <name> <schedule flags>
    /// --system-event <text> [--wake <wake>] | --message <text> --json`
    /// (30s). The payload kind is the current job's kind (kind change =
    /// delete + recreate, blocked by the UI).
    #[allow(clippy::too_many_arguments)] // the Phase 7 contract fixes this field set
    fn edit_automation(
        &self,
        executable: &Path,
        job_id: &str,
        name: &str,
        schedule_kind: &str,
        schedule_value: &str,
        schedule_tz: Option<&str>,
        payload_kind: &str,
        text: &str,
        wake: Option<&str>,
    ) -> Result<(), AppError>;

    /// `openclaw automations enable <job_id> --json` |
    /// `openclaw automations disable <job_id> --json` (30s).
    fn set_automation_enabled(
        &self,
        executable: &Path,
        job_id: &str,
        enabled: bool,
    ) -> Result<(), AppError>;

    /// `openclaw automations remove <job_id> --json` (30s). The `rm`/`delete`
    /// aliases are never used.
    fn remove_automation(&self, executable: &Path, job_id: &str) -> Result<(), AppError>;
}
