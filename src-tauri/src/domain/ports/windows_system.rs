//! Windows system detection port.

use std::path::PathBuf;

use crate::domain::models::windows::{Architecture, NodeDetection, WindowsVersion};
use crate::error::AppError;

pub trait WindowsSystemPort: Send + Sync {
    /// Detect Windows build/version.
    ///
    /// Returns a structured error for unsupported OS builds.
    fn os_version(&self) -> Result<WindowsVersion, AppError>;

    /// Detect the CPU architecture.
    ///
    /// Non-x64 is a structured error (ClawDesk is x64-only).
    fn architecture(&self) -> Result<Architecture, AppError>;

    /// Detect Node.js.
    ///
    /// Absence is `Ok(NodeDetection::NotFound)`, not an error.
    fn detect_node(&self) -> Result<NodeDetection, AppError>;

    /// Resolve the absolute path of the Node.js executable used for spawning.
    ///
    /// Absence is a structured `node-not-found` error.
    fn node_executable(&self) -> Result<PathBuf, AppError>;
}
