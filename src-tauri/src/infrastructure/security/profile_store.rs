//! `SecurityProfileStore` — ClawDesk-owned user security profile store
//! (Phase 5, S4).
//!
//! File: `%APPDATA%\ClawDesk\security-profiles.json` with the shape
//! `{"version":1,"profiles":[...]}`. The file holds tool policy only —
//! no secrets (S3/S7). Builtin profiles are never persisted.
//!
//! Fail-closed: a missing file is a fresh empty list; a corrupt file is a
//! stable error and is NEVER rewritten. Writes are atomic (temp + rename).
//! The path is a constructor parameter so unit/contract tests can inject a
//! sandbox directory (Phase 3 `SecretStore` pattern).

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::domain::models::tools::SecurityProfile;
use crate::domain::ports::security_profile_store::SecurityProfileStorePort;
use crate::error::AppError;

/// Environment override for the store file (test hook; production uses
/// `%APPDATA%\ClawDesk\security-profiles.json`).
pub const PROFILE_STORE_PATH_ENV: &str = "CLAWDESK_SECURITY_PROFILES_PATH";

const STORE_VERSION: u32 = 1;

#[derive(Debug)]
pub struct SecurityProfileStore {
    path: PathBuf,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoreFile {
    version: u32,
    profiles: Vec<SecurityProfile>,
}

impl SecurityProfileStore {
    /// Store at an explicit file path (test wiring / production wiring).
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Production store: `%APPDATA%\ClawDesk\security-profiles.json`.
    pub fn production() -> Self {
        let path = match std::env::var(PROFILE_STORE_PATH_ENV) {
            Ok(path) if !path.is_empty() => PathBuf::from(path),
            _ => default_path(),
        };
        Self::new(path)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn load(&self) -> Result<Vec<SecurityProfile>, AppError> {
        match fs::read_to_string(&self.path) {
            Ok(body) => {
                let file: StoreFile = serde_json::from_str(&body).map_err(|err| {
                    AppError::security_profile_store_failed(format!(
                        "store file is corrupted: {err}"
                    ))
                })?;
                if file.version != STORE_VERSION {
                    return Err(AppError::security_profile_store_failed(format!(
                        "unsupported store version {} (expected {STORE_VERSION})",
                        file.version
                    )));
                }
                Ok(file.profiles)
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
            Err(err) => Err(io_error("read store file", &err)),
        }
    }

    fn persist(&self, profiles: &[SecurityProfile]) -> Result<(), AppError> {
        let file = StoreFile {
            version: STORE_VERSION,
            profiles: profiles.to_vec(),
        };
        let body = serde_json::to_string_pretty(&file).map_err(|err| {
            AppError::security_profile_store_failed(format!("encode store file: {err}"))
        })?;
        write_atomic(&self.path, &body)
    }
}

fn default_path() -> PathBuf {
    let appdata = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    appdata.join("ClawDesk").join("security-profiles.json")
}

/// Writes `bytes` to `path` atomically (temp file + rename), replacing any
/// existing file (app-owned state file management, Phase 3 pattern).
fn write_atomic(path: &Path, bytes: &str) -> Result<(), AppError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| io_error("create dir", &err))?;
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("file");
    let temp = path.with_file_name(format!("{file_name}.tmp.{}", std::process::id()));
    fs::write(&temp, bytes).map_err(|err| io_error("write temp", &err))?;
    match fs::rename(&temp, path) {
        Ok(()) => Ok(()),
        Err(_) => {
            // Windows rename does not overwrite: remove our own previous
            // state file and retry once.
            let _ = fs::remove_file(path);
            fs::rename(&temp, path).map_err(|err| io_error("rename", &err))
        }
    }
}

fn io_error(what: &str, err: &std::io::Error) -> AppError {
    AppError::security_profile_store_failed(format!("{what}: {err}"))
}

impl SecurityProfileStorePort for SecurityProfileStore {
    fn list(&self) -> Result<Vec<SecurityProfile>, AppError> {
        self.load()
    }

    fn get(&self, id: &str) -> Result<Option<SecurityProfile>, AppError> {
        Ok(self.load()?.into_iter().find(|profile| profile.id == id))
    }

    fn save(&self, profile: &SecurityProfile) -> Result<(), AppError> {
        let mut profiles = self.load()?;
        if let Some(existing) = profiles.iter_mut().find(|p| p.id == profile.id) {
            *existing = profile.clone();
        } else {
            profiles.push(profile.clone());
        }
        self.persist(&profiles)
    }

    fn delete(&self, id: &str) -> Result<Option<SecurityProfile>, AppError> {
        let mut profiles = self.load()?;
        let position = profiles.iter().position(|profile| profile.id == id);
        let Some(index) = position else {
            return Ok(None);
        };
        let removed = profiles.remove(index);
        self.persist(&profiles)?;
        Ok(Some(removed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unique per-test scratch file (own subdirectory, so parallel tests
    /// cannot see each other's in-flight temp files).
    fn scratch_path(tag: &str) -> PathBuf {
        let target = std::env::var("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target"));
        let dir = target
            .join("clawdesk-profile-store")
            .join(format!("{tag}-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        dir.join("security-profiles.json")
    }

    fn profile(id: &str, name: &str) -> SecurityProfile {
        SecurityProfile {
            id: id.into(),
            name: name.into(),
            base_profile: "coding".into(),
            allow: vec!["web_search".into()],
            deny: Vec::new(),
            exec_mode: "ask".into(),
        }
    }

    #[test]
    fn missing_file_is_fresh_empty_list() {
        let store = SecurityProfileStore::new(scratch_path("missing"));
        assert!(!store.path().exists());
        assert!(store.list().expect("list").is_empty());
        assert_eq!(store.get("any").expect("get"), None);
    }

    #[test]
    fn save_get_list_roundtrip_and_shape() {
        let path = scratch_path("roundtrip");
        let store = SecurityProfileStore::new(path.clone());
        let a = profile("alpha", "알파");
        let b = profile("beta", "베타");
        store.save(&a).expect("save a");
        store.save(&b).expect("save b");

        let listed = store.list().expect("list");
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].id, "alpha");
        assert_eq!(listed[1].id, "beta");
        assert_eq!(
            store.get("beta").expect("get").map(|p| p.name),
            Some("베타".to_string())
        );
        assert_eq!(store.get("gamma").expect("get"), None);

        // File shape: version 1 + profiles array, no secrets possible
        // (the profile struct has no secret fields).
        let body = fs::read_to_string(&path).expect("file exists");
        let value: serde_json::Value = serde_json::from_str(&body).expect("json");
        assert_eq!(value["version"], 1);
        assert_eq!(value["profiles"].as_array().expect("array").len(), 2);
    }

    #[test]
    fn save_same_id_replaces_in_place() {
        let store = SecurityProfileStore::new(scratch_path("upsert"));
        let first = profile("alpha", "v1");
        store.save(&first).expect("save v1");
        let mut updated = first.clone();
        updated.name = "v2".into();
        updated.exec_mode = "deny".into();
        store.save(&updated).expect("save v2");

        let listed = store.list().expect("list");
        assert_eq!(listed.len(), 1, "upsert must not duplicate");
        assert_eq!(listed[0].name, "v2");
        assert_eq!(listed[0].exec_mode, "deny");
    }

    #[test]
    fn delete_removes_and_reports_removed() {
        let store = SecurityProfileStore::new(scratch_path("delete"));
        let a = profile("alpha", "알파");
        let b = profile("beta", "베타");
        store.save(&a).expect("save a");
        store.save(&b).expect("save b");

        let removed = store.delete("alpha").expect("delete");
        assert_eq!(removed.as_ref().map(|p| p.id.as_str()), Some("alpha"));
        assert_eq!(store.list().expect("list").len(), 1);
        assert_eq!(
            store.delete("alpha").expect("delete again"),
            None,
            "missing → None"
        );
    }

    #[test]
    fn corrupt_file_is_store_failed_and_not_rewritten() {
        let path = scratch_path("corrupt");
        fs::create_dir_all(path.parent().expect("parent")).expect("dir");
        let corrupt = "{ not json !!";
        fs::write(&path, corrupt).expect("seed corrupt file");
        let store = SecurityProfileStore::new(path.clone());

        assert_eq!(
            store.list().unwrap_err().code,
            "security-profile-store-failed"
        );
        assert_eq!(
            store.get("x").unwrap_err().code,
            "security-profile-store-failed"
        );
        // Delete on a corrupt file fails too (the store is unreadable).
        assert_eq!(
            store.delete("x").unwrap_err().code,
            "security-profile-store-failed"
        );
        // The corrupt file must be left untouched (no rewrite).
        assert_eq!(fs::read_to_string(&path).expect("still there"), corrupt);
    }

    #[test]
    fn wrong_version_is_store_failed() {
        let path = scratch_path("version");
        fs::create_dir_all(path.parent().expect("parent")).expect("dir");
        fs::write(&path, r#"{"version":2,"profiles":[]}"#).expect("seed v2 file");
        let store = SecurityProfileStore::new(path);
        assert_eq!(
            store.list().unwrap_err().code,
            "security-profile-store-failed"
        );
    }

    #[test]
    fn write_is_atomic_no_temp_leftovers() {
        let path = scratch_path("atomic");
        let store = SecurityProfileStore::new(path.clone());
        store.save(&profile("alpha", "알파")).expect("save");
        store.save(&profile("beta", "베타")).expect("save again");

        let dir = path.parent().expect("parent");
        let leftovers: Vec<String> = fs::read_dir(dir)
            .expect("read dir")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".tmp."))
            .collect();
        assert!(leftovers.is_empty(), "temp files must be renamed away");
        assert_eq!(store.list().expect("list").len(), 2);
    }

    #[test]
    fn stored_profiles_roundtrip_camel_case_wire() {
        let store = SecurityProfileStore::new(scratch_path("wire"));
        let mut profile = profile("my-profile", "내 프로필");
        profile.deny = vec!["group:fs".into()];
        store.save(&profile).expect("save");
        let body = fs::read_to_string(store.path()).expect("file");
        let value: serde_json::Value = serde_json::from_str(&body).expect("json");
        let row = &value["profiles"][0];
        assert_eq!(row["id"], "my-profile");
        assert_eq!(row["baseProfile"], "coding");
        assert_eq!(row["allow"], serde_json::json!(["web_search"]));
        assert_eq!(row["deny"], serde_json::json!(["group:fs"]));
        assert_eq!(row["execMode"], "ask");
    }
}
