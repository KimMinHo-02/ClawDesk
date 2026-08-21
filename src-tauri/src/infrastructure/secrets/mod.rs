//! OS secret store adapter (Phase 3, S7).

pub mod dpapi;
pub mod store;

pub use dpapi::WindowsDpapi;
pub use store::SecretStore;
