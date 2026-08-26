//! OpenClaw infrastructure adapters.

pub mod adapter;
pub mod automations;
pub mod channels;
pub mod config;
pub mod diagnostics;
pub mod installer;
pub mod parse;
pub mod plugin_install;
pub mod plugins;
pub mod security;
pub mod skills;

pub use adapter::OpenClawAdapter;
pub use automations::OpenClawAutomationsAdapter;
pub use channels::OpenClawChannelsAdapter;
pub use config::OpenClawConfigAdapter;
pub use diagnostics::OpenClawDiagnosticsAdapter;
pub use installer::{node_version_supported, npm_install_policy, OpenClawInstaller};
pub use plugin_install::OpenClawPluginInstallAdapter;
pub use plugins::OpenClawPluginsAdapter;
pub use security::OpenClawSecurityAdapter;
pub use skills::OpenClawSkillsAdapter;
