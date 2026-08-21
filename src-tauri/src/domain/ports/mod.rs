//! Domain ports (adapter interfaces).

pub mod installer;
pub mod openclaw;
pub mod openclaw_config;
pub mod process;
pub mod secrets;
pub mod windows_system;

pub use installer::OpenClawInstallerPort;
pub use openclaw::OpenClawPort;
pub use openclaw_config::{OpenClawConfigPort, WriteMode};
pub use process::{ProcessError, ProcessOutput, ProcessPort, ProcessRequest};
pub use secrets::{is_valid_key_id, SecretStorePort};
pub use windows_system::WindowsSystemPort;
