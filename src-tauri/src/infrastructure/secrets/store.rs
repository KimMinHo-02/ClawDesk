//! `SecretStore` — DPAPI-backed OS secret store (Phase 3, S7).
//!
//! Layout under the store root (default `%APPDATA%\ClawDesk\secrets`):
//! - `<hex(key_id)>.blob` — DPAPI-protected value (the only plaintext-free
//!   persistence; a hex filename keeps arbitrary key ids filesystem-safe
//!   and collision-free)
//! - `index.json` — non-secret index (key id + updated timestamp only;
//!   values are never stored here)
//!
//! The DPAPI primitive sits behind `DpapiPort` so unit tests can substitute
//! a fake; the production binding is `WindowsDpapi`.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::domain::ports::secrets::{is_valid_key_id, SecretStorePort};
use crate::error::AppError;
use crate::infrastructure::secrets::dpapi::{DpapiPort, WindowsDpapi};

/// Environment override for the store root (test hook; production uses the
/// default `%APPDATA%\ClawDesk\secrets`).
pub const SECRETS_ROOT_ENV: &str = "CLAWDESK_SECRETS_ROOT";

#[derive(Debug)]
pub struct SecretStore {
    root: PathBuf,
    dpapi: Arc<dyn DpapiPort>,
}

#[derive(Debug, Serialize, Deserialize)]
struct IndexEntry {
    id: String,
    /// Unix seconds of the last set/update.
    updated: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Index {
    keys: Vec<IndexEntry>,
}

impl SecretStore {
    /// Store at an explicit root with an explicit DPAPI implementation
    /// (test wiring).
    pub fn new(root: PathBuf, dpapi: Arc<dyn DpapiPort>) -> Self {
        Self { root, dpapi }
    }

    /// Production store: default root + real DPAPI. `SECRETS_ROOT_ENV`
    /// overrides the root (used by the secret resolver's tests).
    pub fn production() -> Self {
        let root = match std::env::var(SECRETS_ROOT_ENV) {
            Ok(root) if !root.is_empty() => PathBuf::from(root),
            _ => default_root(),
        };
        Self::new(root, Arc::new(WindowsDpapi::new()))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn blob_path(&self, key_id: &str) -> PathBuf {
        self.root.join(format!("{}.blob", hex(key_id)))
    }

    fn index_path(&self) -> PathBuf {
        self.root.join("index.json")
    }

    fn load_index(&self) -> Result<Index, AppError> {
        match fs::read_to_string(self.index_path()) {
            Ok(body) => serde_json::from_str(&body).map_err(|err| {
                AppError::secret_store_unavailable(format!("index is corrupted: {err}"))
            }),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Index::default()),
            Err(err) => Err(io_error("read index", &err)),
        }
    }

    fn save_index(&self, index: &Index) -> Result<(), AppError> {
        fs::create_dir_all(&self.root).map_err(|err| io_error("create store dir", &err))?;
        let body = serde_json::to_string_pretty(index)
            .map_err(|err| AppError::secret_store_unavailable(format!("encode index: {err}")))?;
        write_atomic(&self.index_path(), &body)
    }

    fn require_key(&self, key_id: &str) -> Result<(), AppError> {
        let index = self.load_index()?;
        if !index.keys.iter().any(|entry| entry.id == key_id) {
            return Err(AppError::secret_store_unavailable(
                "the requested key is not registered",
            ));
        }
        Ok(())
    }
}

fn default_root() -> PathBuf {
    let appdata = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    appdata.join("ClawDesk").join("secrets")
}

fn hex(bytes: &str) -> String {
    bytes.bytes().map(|b| format!("{b:02x}")).collect()
}

/// Writes `bytes` to `path` atomically (temp file + rename), replacing any
/// existing file (app-owned state file management).
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
    AppError::secret_store_unavailable(format!("{what}: {err}"))
}

impl SecretStorePort for SecretStore {
    fn set(&self, key_id: &str, value: &str) -> Result<(), AppError> {
        if !is_valid_key_id(key_id) {
            return Err(AppError::secret_store_unavailable("invalid key id"));
        }
        // The root may not exist on a fresh install; the blob write below
        // needs its parent.
        fs::create_dir_all(&self.root).map_err(|err| io_error("create store dir", &err))?;
        let protected = self.dpapi.protect(value.as_bytes())?;
        fs::write(self.blob_path(key_id), &protected)
            .map_err(|err| io_error("write blob", &err))?;

        let mut index = self.load_index()?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if let Some(entry) = index.keys.iter_mut().find(|e| e.id == key_id) {
            entry.updated = now;
        } else {
            index.keys.push(IndexEntry {
                id: key_id.to_string(),
                updated: now,
            });
        }
        self.save_index(&index)
    }

    fn get(&self, key_id: &str) -> Result<Option<String>, AppError> {
        if !is_valid_key_id(key_id) {
            return Ok(None);
        }
        let blob_path = self.blob_path(key_id);
        let protected = match fs::read(&blob_path) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(io_error("read blob", &err)),
        };
        let value = self.dpapi.unprotect(&protected)?;
        String::from_utf8(value)
            .map(Some)
            .map_err(|_| AppError::secret_store_unavailable("stored value is not UTF-8"))
    }

    fn delete(&self, key_id: &str) -> Result<(), AppError> {
        if !is_valid_key_id(key_id) {
            return Err(AppError::secret_store_unavailable("invalid key id"));
        }
        self.require_key(key_id)?;

        let mut index = self.load_index()?;
        index.keys.retain(|entry| entry.id != key_id);
        self.save_index(&index)?;

        match fs::remove_file(self.blob_path(key_id)) {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(io_error("remove blob", &err)),
        }
    }

    fn contains(&self, key_id: &str) -> bool {
        if !is_valid_key_id(key_id) {
            return false;
        }
        self.load_index()
            .map(|index| index.keys.iter().any(|entry| entry.id == key_id))
            .unwrap_or(false)
    }

    fn list_key_ids(&self) -> Vec<String> {
        self.load_index()
            .map(|index| index.keys.into_iter().map(|entry| entry.id).collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// In-memory DPAPI fake: marks bytes so round-trips are verifiable.
    #[derive(Debug)]
    struct FakeDpapi {
        failures: Arc<AtomicUsize>,
    }

    impl DpapiPort for FakeDpapi {
        fn protect(&self, data: &[u8]) -> Result<Vec<u8>, AppError> {
            if self.failures.load(Ordering::SeqCst) > 0 {
                return Err(AppError::secret_store_unavailable("injected DPAPI failure"));
            }
            let mut out = Vec::from("FAKEENC:");
            out.extend_from_slice(data);
            Ok(out)
        }
        fn unprotect(&self, data: &[u8]) -> Result<Vec<u8>, AppError> {
            if self.failures.load(Ordering::SeqCst) > 0 {
                return Err(AppError::secret_store_unavailable("injected DPAPI failure"));
            }
            let bytes = data
                .strip_prefix(b"FAKEENC:")
                .ok_or_else(|| AppError::secret_store_unavailable("not DPAPI-protected"))?;
            Ok(bytes.to_vec())
        }
    }

    /// Unique per-process scratch root under the cargo target dir.
    fn scratch_root(tag: &str) -> PathBuf {
        let target = std::env::var("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target"));
        let root = target
            .join("clawdesk-secret-store")
            .join(format!("{tag}-{}", std::process::id()));
        let _ = fs::create_dir_all(&root);
        root
    }

    fn store(root: PathBuf) -> (SecretStore, Arc<AtomicUsize>) {
        let failures = Arc::new(AtomicUsize::new(0));
        (
            SecretStore::new(
                root,
                Arc::new(FakeDpapi {
                    failures: Arc::clone(&failures),
                }),
            ),
            failures,
        )
    }

    #[test]
    fn set_get_roundtrip_and_index_has_no_value() {
        let root = scratch_root("roundtrip");
        let (store, failures) = store(root.clone());
        failures.store(0, Ordering::SeqCst);
        store
            .set("providers/acme/apiKey", "sk-fake123456789")
            .expect("set should work");
        assert_eq!(
            store.get("providers/acme/apiKey").expect("get"),
            Some("sk-fake123456789".to_string())
        );
        assert!(store.contains("providers/acme/apiKey"));
        assert_eq!(
            store.list_key_ids(),
            vec!["providers/acme/apiKey".to_string()]
        );

        // The index must be value-free (non-secret metadata only).
        let index_body = fs::read_to_string(root.join("index.json")).expect("index exists");
        assert!(
            !index_body.contains("sk-fake123456789"),
            "index must be value-free"
        );
        // The blob must be a transformed (protected) form, not the raw value.
        let blob = fs::read(root.join(format!("{}.blob", hex("providers/acme/apiKey"))))
            .expect("blob exists");
        assert!(
            blob.starts_with(b"FAKEENC:"),
            "blob must be DPAPI-protected (fake marker present)"
        );
    }

    #[test]
    fn overwrite_updates_value_and_index_entry_count() {
        let root = scratch_root("overwrite");
        let (store, failures) = store(root);
        failures.store(0, Ordering::SeqCst);
        store.set("providers/a/apiKey", "v1").unwrap();
        store.set("providers/a/apiKey", "v2").unwrap();
        assert_eq!(store.get("providers/a/apiKey").unwrap(), Some("v2".into()));
        assert_eq!(store.list_key_ids(), vec!["providers/a/apiKey".to_string()]);
    }

    #[test]
    fn get_missing_is_none_delete_missing_is_error() {
        let root = scratch_root("missing");
        let (store, failures) = store(root);
        failures.store(0, Ordering::SeqCst);
        assert_eq!(store.get("providers/x/apiKey").unwrap(), None);
        assert!(!store.contains("providers/x/apiKey"));
        let err = store.delete("providers/x/apiKey").expect_err("must fail");
        assert_eq!(err.code, "secret-store-unavailable");
    }

    #[test]
    fn delete_removes_index_and_blob() {
        let root = scratch_root("delete");
        let (store, failures) = store(root);
        failures.store(0, Ordering::SeqCst);
        store.set("providers/a/apiKey", "v").unwrap();
        store.delete("providers/a/apiKey").unwrap();
        assert_eq!(store.get("providers/a/apiKey").unwrap(), None);
        assert!(!store.contains("providers/a/apiKey"));
        assert!(store.list_key_ids().is_empty());
    }

    #[test]
    fn dpapi_failure_maps_to_secret_store_unavailable() {
        let root = scratch_root("dpapi-fail");
        let (store, failures) = store(root);
        failures.store(1, Ordering::SeqCst);
        let err = store
            .set("providers/a/apiKey", "v")
            .expect_err("protect failure");
        assert_eq!(err.code, "secret-store-unavailable");
        // No blob was written; get resolves to None without touching DPAPI.
        assert_eq!(store.get("providers/a/apiKey").unwrap(), None);
    }

    #[test]
    fn invalid_key_ids_are_rejected() {
        let root = scratch_root("bad-key");
        let (store, failures) = store(root);
        failures.store(0, Ordering::SeqCst);
        assert!(store.set("../evil", "v").is_err());
        assert!(store.set("a b", "v").is_err());
        assert_eq!(store.get("a/b/c").unwrap(), None);
    }

    /// Real DPAPI round-trip (Windows only): proves the production binding
    /// works. DPAPI calls are pure byte transforms — no persistent OS state
    /// is created or modified.
    #[cfg(windows)]
    #[test]
    fn real_dpapi_roundtrip() {
        let dpapi = WindowsDpapi::new();
        let secret = b"sk-real-dpapi-test-12345";
        let protected = dpapi.protect(secret).expect("protect");
        assert_ne!(protected, secret.to_vec(), "value must be transformed");
        let restored = dpapi.unprotect(&protected).expect("unprotect");
        assert_eq!(restored, secret.to_vec());
    }

    /// Garbage input to unprotect must fail cleanly (no panic).
    #[cfg(windows)]
    #[test]
    fn real_dpapi_unprotect_garbage_fails() {
        let dpapi = WindowsDpapi::new();
        let err = dpapi.unprotect(b"not a dpapi blob").expect_err("must fail");
        assert_eq!(err.code, "secret-store-unavailable");
    }
}
