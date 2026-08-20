//! Domain models (value types shared across layers).

pub mod openclaw;
pub mod windows;

pub use openclaw::{
    ExecutableDetection, GatewayStatus, OpenClawStatus, OpenClawVersion, UpdateState,
};
pub use windows::{Architecture, NodeDetection, WindowsVersion};
