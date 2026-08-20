//! Domain ports (adapter interfaces).

pub mod openclaw;
pub mod process;
pub mod windows_system;

pub use openclaw::OpenClawPort;
pub use process::{ProcessError, ProcessOutput, ProcessPort, ProcessRequest};
pub use windows_system::WindowsSystemPort;
