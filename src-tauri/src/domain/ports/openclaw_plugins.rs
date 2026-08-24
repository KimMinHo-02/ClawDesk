//! OpenClaw plugins port (Phase 4).
//!
//! Every method maps to a non-interactive `openclaw plugins` CLI invocation
//! (structured argv via `ProcessPort` — S1/S2). Plugin ids are always taken
//! from `plugins list` rows (never free text) and validated before use.

use std::path::Path;

use crate::domain::models::skills::{PluginRow, PluginRuntime};
use crate::error::AppError;

pub trait OpenClawPluginsPort: Send + Sync {
    /// `openclaw plugins list --json` → the cold plugin inventory
    /// (persisted local registry + manifest fallback — not a live gateway
    /// probe). Read-only.
    fn list_plugins(&self, executable: &Path) -> Result<Vec<PluginRow>, AppError>;

    /// `openclaw plugins enable <id>` — updates config + cold registry
    /// (no gateway restart). Non-zero exit is a structured error; the
    /// caller must re-query the list for the actual state.
    fn enable_plugin(&self, executable: &Path, id: &str) -> Result<(), AppError>;

    /// `openclaw plugins disable <id>` — same contract as `enable_plugin`.
    fn disable_plugin(&self, executable: &Path, id: &str) -> Result<(), AppError>;

    /// `openclaw plugins inspect <id> --runtime --json` → the live runtime
    /// surface (registered tools/hooks/services/commands/gateway methods/
    /// routes). This loads plugin modules, so it uses a longer timeout.
    fn inspect_plugin_runtime(
        &self,
        executable: &Path,
        id: &str,
    ) -> Result<PluginRuntime, AppError>;
}
