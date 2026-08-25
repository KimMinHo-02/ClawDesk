//! OpenClaw plugin-install port (Phase 6).
//!
//! `openclaw plugins install <npm-id>` — structured argv via `ProcessPort`
//! (S1/S2). The npm id is a single argv element. This is the **only** plugin
//! mutation ClawDesk performs (`@openclaw/discord` for the Discord channel);
//! `plugins update/remove` and any other plugin install are non-goals.

use std::path::Path;

use crate::error::AppError;

pub trait OpenClawPluginInstallPort: Send + Sync {
    /// `openclaw plugins install <npm-id>` (300s — installs are slower than
    /// the 30s config calls). The real CLI is idempotent for an already
    /// installed plugin (exit 0); the caller re-checks `plugins list` for
    /// the actual state.
    fn install_plugin(&self, executable: &Path, npm_id: &str) -> Result<(), AppError>;
}
