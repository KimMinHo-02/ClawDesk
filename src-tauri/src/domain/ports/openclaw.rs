//! OpenClaw state port.

use std::path::Path;

use crate::domain::models::openclaw::{
    ExecutableDetection, GatewayStatus, OpenClawVersion, UpdateState,
};
use crate::error::AppError;

pub trait OpenClawPort: Send + Sync {
    /// Locate the OpenClaw executable among the configured search locations.
    fn detect_executable(&self) -> ExecutableDetection;

    /// Run `openclaw --version` and parse the version.
    fn version(&self, executable: &Path) -> Result<OpenClawVersion, AppError>;

    /// Run `openclaw gateway status --json` and parse the payload.
    fn gateway_status(&self, executable: &Path) -> Result<GatewayStatus, AppError>;

    /// Run `openclaw update status --json` and compare against latest stable.
    ///
    /// When the state cannot be determined, this resolves to
    /// `Ok(UpdateState::Unknown)` rather than an error.
    fn update_state(&self, executable: &Path) -> Result<UpdateState, AppError>;
}
