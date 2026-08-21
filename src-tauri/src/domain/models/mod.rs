//! Domain models (value types shared across layers).

pub mod install;
pub mod openclaw;
pub mod windows;

pub use install::{NpmEntry, ResolvedOpenClawEntry};
pub use openclaw::{
    ExecutableDetection, GatewayStatus, OpenClawStatus, OpenClawVersion, UpdateState,
};
pub use windows::{Architecture, NodeDetection, WindowsVersion};
