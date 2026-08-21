//! OpenClaw config port (Phase 3).
//!
//! Every method maps to a non-interactive `openclaw config` / `openclaw
//! models` CLI invocation (structured argv via `ProcessPort` — S1/S2).
//! ClawDesk never edits `openclaw.json` directly; the CLI's schema
//! validation, protected-path rules, and atomic writes are the safety
//! mechanism (dry-run → commit inside `write`/`unset`).

use std::path::{Path, PathBuf};

use crate::domain::models::models::{ModelRow, ProviderDetail, ThinkingLevel};
use crate::error::AppError;

pub trait OpenClawConfigPort: Send + Sync {
    /// `openclaw config file --json` → the active config file path.
    fn config_path(&self, executable: &Path) -> Result<PathBuf, AppError>;

    /// `openclaw config get models.providers --json` → all providers
    /// (redacted snapshot; secret values never appear).
    fn read_providers(&self, executable: &Path) -> Result<Vec<ProviderDetail>, AppError>;

    /// `openclaw models list --json` → all model rows (read-only).
    fn read_models(&self, executable: &Path) -> Result<Vec<ModelRow>, AppError>;

    /// `openclaw config get agents.defaults.model --json` → the current
    /// default `provider/model` reference, if set.
    fn read_default_model(&self, executable: &Path) -> Result<Option<String>, AppError>;

    /// `openclaw config get agents.defaults.thinkingDefault --json` → the
    /// current global thinking level, if set.
    fn read_thinking_default(&self, executable: &Path) -> Result<Option<ThinkingLevel>, AppError>;

    /// Commits a config write. Runs `--dry-run --json` first; when the dry
    /// run is not `ok`, nothing is written and
    /// `openclaw-config-invalid` is returned. `replace` selects
    /// `--replace` (full node replacement for protected paths); otherwise
    /// `--merge` when the path targets a provider entry node, plain set for
    /// scalars — the caller passes the exact mode.
    fn write(
        &self,
        executable: &Path,
        path: &str,
        value_json: &str,
        mode: WriteMode,
    ) -> Result<(), AppError>;

    /// Deletes a config path (`config unset <path>`, dry-run first).
    /// A missing target is a structured error and leaves the config unchanged.
    fn unset(&self, executable: &Path, path: &str) -> Result<(), AppError>;

    /// `openclaw models set <provider/model>` → writes
    /// `agents.defaults.model.primary`. Unknown refs are rejected by the CLI
    /// (non-zero exit, config unchanged).
    fn set_default_model(&self, executable: &Path, model_ref: &str) -> Result<(), AppError>;

    /// Reads one raw config value as JSON text (redacted).
    fn read_raw(&self, executable: &Path, path: &str) -> Result<Option<String>, AppError>;
}

/// The `config set` write mode for a target path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteMode {
    /// `--merge`: merge into an existing map/entry (new provider entries).
    Merge,
    /// `--replace`: replace the whole node (updates of protected nodes).
    Replace,
    /// Plain set (scalar values, non-protected nodes).
    Plain,
}
