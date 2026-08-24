//! OpenClaw security port (Phase 5).
//!
//! Read-only cold security audit via `openclaw security audit --json`
//! (structured argv through `ProcessPort` — S1/S2). No `--deep`/`--fix`
//! (non-goal), no credentials in argv/env/UI (S2/S3). Audit results are
//! display-only; ClawDesk never acts on findings.

use std::path::Path;

use crate::domain::models::tools::SecurityAuditResult;
use crate::error::AppError;

pub trait OpenClawSecurityPort: Send + Sync {
    /// `openclaw security audit --json` → parsed audit result (read-only).
    ///
    /// Non-zero exit or unparseable output is a stable structured error;
    /// the UI must not assume a clean state on failure (fail-closed).
    fn run_security_audit(&self, executable: &Path) -> Result<SecurityAuditResult, AppError>;
}
