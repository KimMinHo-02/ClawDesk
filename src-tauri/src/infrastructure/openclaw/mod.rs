//! OpenClaw infrastructure adapters.

pub mod adapter;
pub mod installer;
pub mod parse;

pub use adapter::OpenClawAdapter;
pub use installer::{node_version_supported, npm_install_policy, OpenClawInstaller};
