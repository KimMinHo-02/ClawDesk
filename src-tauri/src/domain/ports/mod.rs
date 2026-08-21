//! Domain ports (adapter interfaces).

pub mod installer;
pub mod openclaw;
pub mod process;
pub mod windows_system;

pub use installer::OpenClawInstallerPort;
pub use openclaw::OpenClawPort;
pub use process::{ProcessError, ProcessOutput, ProcessPort, ProcessRequest};
pub use windows_system::WindowsSystemPort;
