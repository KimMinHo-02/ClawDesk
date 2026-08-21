//! OpenClaw install port (Phase 2).

use std::path::Path;

use crate::domain::models::install::{NpmEntry, ResolvedOpenClawEntry};
use crate::error::AppError;

/// Installs OpenClaw through npm. All process execution goes through the
/// `ProcessPort` (S1); no shell, no npm shims, no user-controlled input.
pub trait OpenClawInstallerPort: Send + Sync {
    /// Resolves the npm spawn entry (node.exe + npm-cli.js) for the Node
    /// runtime at `node_executable`.
    ///
    /// `npm.cmd` / `npm.ps1` / extension-less shims are never resolved or
    /// spawned (Phase 2 contract).
    fn resolve_npm_entry(&self, node_executable: &Path) -> Result<NpmEntry, AppError>;

    /// Runs `node.exe npm-cli.js --version` and returns the parsed version.
    fn npm_version(&self, entry: &NpmEntry) -> Result<String, AppError>;

    /// Runs `npm install -g openclaw@latest`, including
    /// `--allow-scripts=openclaw` only when `allow_scripts` is true.
    fn install_openclaw_latest(
        &self,
        entry: &NpmEntry,
        allow_scripts: bool,
    ) -> Result<(), AppError>;

    /// Resolves the installed OpenClaw package's JS spawn entry from the npm
    /// global prefix (`node_modules/openclaw/package.json` → `bin.openclaw`),
    /// boundary-validated against the package root.
    fn resolve_openclaw_entry(&self) -> Result<ResolvedOpenClawEntry, AppError>;
}
