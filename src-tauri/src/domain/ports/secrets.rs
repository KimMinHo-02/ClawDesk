//! OS secret store port (Phase 3, S7).
//!
//! API keys/credentials persist **only** through the OS secret store
//! (DPAPI on Windows). Key ids are non-secret strings (e.g.
//! `providers/<providerId>/apiKey`); values never appear in index files,
//! logs, or errors.

use crate::error::AppError;

pub trait SecretStorePort: Send + Sync {
    /// Stores (or overwrites) the value for `key_id`.
    fn set(&self, key_id: &str, value: &str) -> Result<(), AppError>;

    /// Reads the value for `key_id` (`None` when the key does not exist).
    fn get(&self, key_id: &str) -> Result<Option<String>, AppError>;

    /// Deletes the key. Deleting a missing key is an error (fail-closed).
    fn delete(&self, key_id: &str) -> Result<(), AppError>;

    /// Whether the key exists (index-based, no value access).
    fn contains(&self, key_id: &str) -> bool;

    /// All stored key ids (non-secret).
    fn list_key_ids(&self) -> Vec<String>;
}

/// Key id shape (mirrors the OpenClaw exec-id pattern minus `..` segments):
/// `[A-Za-z0-9]` start, then `[A-Za-z0-9._:/#-]`, max 256 chars.
pub fn is_valid_key_id(key_id: &str) -> bool {
    let bytes = key_id.as_bytes();
    let ok = bytes.first().is_some_and(|c| c.is_ascii_alphanumeric())
        && bytes.len() <= 256
        && bytes.iter().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b':' | b'/' | b'#' | b'-')
        })
        && !bytes.windows(2).any(|pair| pair == b"..");
    ok
}

#[cfg(test)]
mod tests {
    use super::is_valid_key_id;

    #[test]
    fn key_id_accepts_clawdesk_shape() {
        assert!(is_valid_key_id("providers/acme/apiKey"));
        assert!(is_valid_key_id("providers/a-b.c_1:x#y/apiKey"));
    }

    #[test]
    fn key_id_rejects_traversal_and_bad_chars() {
        for bad in ["", "..", "a/../b", "a b", "a\\b", "a\nb", &"x".repeat(257)] {
            assert!(!is_valid_key_id(bad), "{bad:?}");
        }
    }
}
