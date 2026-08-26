//! Node.js update port (Phase 8.1).
//!
//! One-shot Node.js update via `winget` (PRODUCT_CONTRACT §3 — no terminal):
//! availability probe → `winget install OpenJS.NodeJS.LTS` (upsert) →
//! post-update re-detection. Every invocation is a structured
//! `executable + argv` request through the `ProcessPort` (S1/S2).
//!
//! The post-update re-detection is part of the port because the update is
//! only *verified* by re-detecting: the caller (service) must never trust
//! the winget exit code alone.

use crate::domain::models::windows::NodeDetection;
use crate::error::AppError;

pub trait NodeUpdatePort: Send + Sync {
    /// Performs the one-shot Node.js update and returns the post-update
    /// detection.
    ///
    /// - winget absent ⇒ `winget-not-found` (0 install attempts)
    /// - winget non-zero / timeout ⇒ structured error
    /// - Node no longer detectable after the update ⇒
    ///   `Ok(NodeDetection::NotFound)` (the caller validates)
    fn update_node(&self) -> Result<NodeDetection, AppError>;
}
