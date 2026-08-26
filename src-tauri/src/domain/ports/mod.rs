//! Domain ports (adapter interfaces).

pub mod installer;
pub mod openclaw;
pub mod openclaw_automations;
pub mod openclaw_channels;
pub mod openclaw_config;
pub mod openclaw_plugin_install;
pub mod openclaw_plugins;
pub mod openclaw_security;
pub mod openclaw_skills;
pub mod process;
pub mod secrets;
pub mod security_profile_store;
pub mod windows_system;

pub use installer::OpenClawInstallerPort;
pub use openclaw::OpenClawPort;
pub use openclaw_automations::OpenClawAutomationsPort;
pub use openclaw_channels::OpenClawChannelsPort;
pub use openclaw_config::{OpenClawConfigPort, WriteMode};
pub use openclaw_plugin_install::OpenClawPluginInstallPort;
pub use openclaw_plugins::OpenClawPluginsPort;
pub use openclaw_security::OpenClawSecurityPort;
pub use openclaw_skills::OpenClawSkillsPort;
pub use process::{ProcessError, ProcessOutput, ProcessPort, ProcessRequest};
pub use secrets::{is_valid_key_id, SecretStorePort};
pub use security_profile_store::SecurityProfileStorePort;
pub use windows_system::WindowsSystemPort;
