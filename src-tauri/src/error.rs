//! Unified application error with stable, machine-readable codes.
//!
//! Architecture §5: all errors surfaced across the stack use `AppError`.
//! Messages must never contain secrets (S3/S8).

use std::fmt;

use crate::infrastructure::masking::mask_secrets;

/// Unified application error. `code` is stable and safe to compare/log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppError {
    /// Stable error code, e.g. `"openclaw-not-found"`.
    pub code: &'static str,
    /// Human-readable message (already masked).
    pub message: String,
}

impl AppError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        // S8: every error message passes the masking pipeline exactly once
        // at construction, so no secret can reach logs, the UI, or IPC.
        Self {
            code,
            message: mask_secrets(&message.into()),
        }
    }

    pub fn unsupported_architecture(found: impl Into<String>) -> Self {
        Self::new(
            "unsupported-architecture",
            format!(
                "unsupported architecture: {} (ClawDesk supports x64 only)",
                found.into()
            ),
        )
    }

    pub fn unsupported_os_version(build: u32) -> Self {
        Self::new(
            "unsupported-os-version",
            format!(
                "unsupported Windows build {build}: ClawDesk requires Windows 10/11 (build >= 10240)"
            ),
        )
    }

    pub fn os_info_unavailable(message: impl Into<String>) -> Self {
        Self::new("os-info-unavailable", message)
    }

    pub fn node_not_found() -> Self {
        Self::new(
            "node-not-found",
            "Node.js executable was not found on this machine",
        )
    }

    pub fn node_version_unavailable(message: impl Into<String>) -> Self {
        Self::new("node-version-unavailable", message)
    }

    pub fn openclaw_not_found() -> Self {
        Self::new(
            "openclaw-not-found",
            "OpenClaw executable was not found on this machine",
        )
    }

    pub fn process_timeout(command: &str) -> Self {
        Self::new("process-timeout", format!("{command} timed out"))
    }

    pub fn process_failed(command: &str, detail: impl Into<String>) -> Self {
        Self::new(
            "process-failed",
            format!("{command} failed: {}", detail.into()),
        )
    }

    pub fn openclaw_version_parse(detail: impl Into<String>) -> Self {
        Self::new(
            "openclaw-version-parse",
            format!("cannot parse OpenClaw version: {}", detail.into()),
        )
    }

    pub fn openclaw_gateway_parse(detail: impl Into<String>) -> Self {
        Self::new(
            "openclaw-gateway-parse",
            format!("cannot parse OpenClaw gateway status: {}", detail.into()),
        )
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for AppError {}
