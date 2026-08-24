//! OpenClaw skills port (Phase 4).
//!
//! Read-only skill inventory via `openclaw skills list --json` (structured
//! argv through `ProcessPort` — S1/S2). Skill activation is NOT a CLI
//! command: it is a config override (`skills.entries.<name>.enabled`) and
//! therefore goes through the Phase 3 `OpenClawConfigPort` in the service
//! layer, not this port.

use std::path::Path;

use crate::domain::models::skills::SkillRow;
use crate::error::AppError;

pub trait OpenClawSkillsPort: Send + Sync {
    /// `openclaw skills list --json` → all skill rows (read-only).
    ///
    /// Rows missing optional fields are kept with `null` (fail-soft).
    fn list_skills(&self, executable: &Path) -> Result<Vec<SkillRow>, AppError>;
}
