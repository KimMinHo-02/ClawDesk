//! Domain types for the OpenClaw install flow (Phase 2).

use std::path::PathBuf;

/// Resolved npm spawn entry: the Node runtime plus its `npm-cli.js`.
///
/// Per S1 and the Phase 2 contract, npm is always launched as the structured
/// pair `node.exe + npm-cli.js` — never via `npm.cmd` / `npm.ps1` /
/// extension-less shims.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NpmEntry {
    /// Absolute path of the `node.exe` runtime.
    pub node: PathBuf,
    /// Absolute path of the `npm-cli.js` entry belonging to that runtime.
    pub npm_cli: PathBuf,
}

/// The installed OpenClaw package's JS spawn entry, boundary-validated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedOpenClawEntry {
    /// Canonical package root (`<npm global prefix>/node_modules/openclaw`).
    pub package_root: PathBuf,
    /// Canonical JS entry (e.g. `.../openclaw.mjs`), guaranteed to be inside
    /// `package_root` (absolute/canonical path boundary check).
    pub entry: PathBuf,
}
