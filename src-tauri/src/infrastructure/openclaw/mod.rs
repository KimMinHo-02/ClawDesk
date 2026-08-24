//! OpenClaw infrastructure adapters.

pub mod adapter;
pub mod config;
pub mod installer;
pub mod parse;
pub mod plugins;
pub mod skills;

pub use adapter::OpenClawAdapter;
pub use config::OpenClawConfigAdapter;
pub use installer::{node_version_supported, npm_install_policy, OpenClawInstaller};
pub use plugins::OpenClawPluginsAdapter;
pub use skills::OpenClawSkillsAdapter;
