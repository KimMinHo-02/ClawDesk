//! Security profile store port (Phase 5).
//!
//! ClawDesk-owned persistence for user security profiles
//! (`%APPDATA%\ClawDesk\security-profiles.json`, S4). The file holds tool
//! policy only — no secrets (S3/S7).
//!
//! Builtin profiles are NOT stored; they live in code
//! (`domain::models::tools::builtin_profiles`).

use crate::domain::models::tools::SecurityProfile;
use crate::error::AppError;

pub trait SecurityProfileStorePort: Send + Sync {
    /// All stored user profiles. File absent → empty list; a corrupt file is
    /// a structured error (fail-closed, the file is never rewritten).
    fn list(&self) -> Result<Vec<SecurityProfile>, AppError>;

    /// One stored profile by id (`None` when absent).
    fn get(&self, id: &str) -> Result<Option<SecurityProfile>, AppError>;

    /// Inserts or replaces the profile (upsert). The profile must already be
    /// validated by the service layer.
    fn save(&self, profile: &SecurityProfile) -> Result<(), AppError>;

    /// Deletes a stored profile. `Ok(None)` when the id is not stored;
    /// `Ok(Some(profile))` when it was removed.
    fn delete(&self, id: &str) -> Result<Option<SecurityProfile>, AppError>;
}
