//! Application layer: use cases that compose ports.

pub mod services;

pub use services::environment::{
    default_openclaw_search_dirs, EnvironmentReport, EnvironmentService,
};
