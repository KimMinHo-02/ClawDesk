//! Domain types for OpenClaw state.

/// Result of locating the OpenClaw executable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutableDetection {
    /// No OpenClaw executable found on this machine (structured "not found").
    NotFound,
    /// Executable found, with its resolved path.
    Found { path: std::path::PathBuf },
}

/// Gateway status reported by OpenClaw.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct GatewayStatus {
    /// High level state derived from the payload `ok` flag: "running" / "stopped".
    pub state: String,
    /// Gateway version reported by the primary target, when it reports one.
    pub version: Option<String>,
    /// Port the primary target listens on (from its ws:// url), when any.
    pub port: Option<u16>,
}

/// Parsed `--version` output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenClawVersion {
    /// Version extracted from the CLI's `OpenClaw <version>` output,
    /// e.g. "2026.7.1-2".
    pub raw: String,
}

/// OpenClaw presence plus the states that could be collected from it.
///
/// `NotFound` is a structured "not found" value. When detected, individual
/// sub-states that could not be collected are `None` / `Unknown`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum OpenClawStatus {
    NotFound,
    Detected {
        executable: std::path::PathBuf,
        version: Option<String>,
        gateway: Option<GatewayStatus>,
        update: UpdateState,
    },
}

/// Update state relative to latest stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum UpdateState {
    /// Installed version matches latest stable.
    Updated,
    /// A newer stable version is available.
    UpdateAvailable,
    /// Could not determine (missing data, error, or unknown payload).
    Unknown,
}
