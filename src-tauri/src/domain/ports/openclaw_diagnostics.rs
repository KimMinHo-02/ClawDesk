//! OpenClaw diagnostics port (Phase 8).
//!
//! Read-only profile/update/log surface (PRODUCT_CONTRACT §4.7). Every call
//! is a structured `executable + argv` request through the `ProcessPort`
//! (S1/S2); log tails are one-shot (`logs --limit <n> --json`) — `--follow`
//! streaming is a non-goal and must never be emitted.
//!
//! `gateway_status` stays on the Phase 1 `OpenClawPort` (reuse, 0 moves).

use std::path::Path;

use crate::domain::models::diagnostics::{AgentRow, LogsResult, UpdateStatusDetail};
use crate::error::AppError;

pub trait OpenClawDiagnosticsPort: Send + Sync {
    /// `openclaw agents list --json` → all agent rows (read-only).
    ///
    /// Rows missing optional fields are kept with `null` (fail-soft); only a
    /// missing/empty `id` drops a row.
    fn list_agents(&self, executable: &Path) -> Result<Vec<AgentRow>, AppError>;

    /// `openclaw update status --json` → state plus current/latest versions.
    ///
    /// Fail-soft (Phase 1 policy): whenever the state cannot be determined
    /// (process failure, timeout, missing data, malformed payload) this
    /// resolves to `Ok(UpdateStatusDetail::unknown())`, never an error.
    fn update_detail(&self, executable: &Path) -> Result<UpdateStatusDetail, AppError>;

    /// `openclaw logs --limit <n> --json` → one-shot tail (no `--follow`).
    ///
    /// An empty stdout is a successful zero-line result.
    fn tail_logs(&self, executable: &Path, limit: u32) -> Result<LogsResult, AppError>;
}
