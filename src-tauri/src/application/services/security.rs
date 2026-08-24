//! Security profile + audit use case (Phase 5).
//!
//! Orchestration:
//! - `save`: validate every field (S2, 0 store/CLI I/O on failure) → builtin
//!   id collision check (`security-profile-conflict`) → store upsert.
//! - `delete`: slug validation → builtin id → `security-profile-not-found` →
//!   store delete (missing id → `security-profile-not-found`).
//! - `apply`: resolve the profile (builtin or user; unknown →
//!   `security-profile-not-found`, 0 CLI calls) → executable detection →
//!   four ordered config writes (`tools.profile` → `tools.allow` →
//!   `tools.deny` → `tools.exec.mode`, each dry-run → commit). The first
//!   failure stops the sequence (no partial continue); the UI re-queries
//!   the actual state (no optimistic updates).
//! - `list`: builtins (code) + user profiles (store) + applied-state
//!   determination. A tool-policy read failure degrades to
//!   `policyReadFailed: true` (the store side still lists); a corrupt store
//!   is a hard error (fail-closed, no rewrite).
//! - `audit`: cold read-only `openclaw security audit --json` (never
//!   `--deep`/`--fix`/`--token`/`--password`).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::environment::default_openclaw_search_dirs;
use crate::domain::models::tools::{
    builtin_profiles, find_matching_profile, is_builtin_profile_id, parse_tool_policy,
    validate_profile, validate_profile_slug, SecurityAuditResult, SecurityProfile,
};
use crate::domain::ports::openclaw::OpenClawPort;
use crate::domain::ports::openclaw_config::{OpenClawConfigPort, WriteMode};
use crate::domain::ports::openclaw_security::OpenClawSecurityPort;
use crate::domain::ports::security_profile_store::SecurityProfileStorePort;
use crate::error::AppError;
use crate::infrastructure::openclaw::{
    OpenClawAdapter, OpenClawConfigAdapter, OpenClawSecurityAdapter,
};
use crate::infrastructure::process::ProcessRunner;
use crate::infrastructure::security::SecurityProfileStore;

/// `list-security-profiles` response: builtin + user profiles plus the
/// applied-state determination (contract §4).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityProfileList {
    pub builtins: Vec<SecurityProfile>,
    pub users: Vec<SecurityProfile>,
    /// The id of the profile whose four fields match the current policy
    /// (builtins take priority); `null` when the policy is custom or the
    /// policy read failed.
    pub current_applied: Option<String>,
    /// True when the current tool policy could not be read (OpenClaw
    /// missing/CLI failure). The list itself is still shown.
    pub policy_read_failed: bool,
}

/// Use case layer: composes the OpenClaw executable, security, config, and
/// profile-store ports.
pub struct SecurityProfileService {
    openclaw: Arc<dyn OpenClawPort>,
    security: Arc<dyn OpenClawSecurityPort>,
    config: Arc<dyn OpenClawConfigPort>,
    store: Arc<dyn SecurityProfileStorePort>,
}

impl SecurityProfileService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        openclaw: Arc<dyn OpenClawPort>,
        security: Arc<dyn OpenClawSecurityPort>,
        config: Arc<dyn OpenClawConfigPort>,
        store: Arc<dyn SecurityProfileStorePort>,
    ) -> Self {
        Self {
            openclaw,
            security,
            config,
            store,
        }
    }

    /// Production wiring: real adapters over the single `ProcessRunner` and
    /// the ClawDesk-owned profile store file.
    pub fn production() -> Self {
        let process: Arc<dyn crate::domain::ports::process::ProcessPort> = Arc::new(ProcessRunner);
        let openclaw = Arc::new(OpenClawAdapter::new(
            Arc::clone(&process),
            default_openclaw_search_dirs(),
        ));
        let security = Arc::new(OpenClawSecurityAdapter::new(Arc::clone(&process)));
        let config = Arc::new(OpenClawConfigAdapter::new(Arc::clone(&process)));
        let store: Arc<dyn SecurityProfileStorePort> = Arc::new(SecurityProfileStore::production());
        Self::new(openclaw, security, config, store)
    }

    fn executable(&self) -> Result<PathBuf, AppError> {
        match self.openclaw.detect_executable() {
            crate::domain::models::ExecutableDetection::Found { path } => Ok(path),
            crate::domain::models::ExecutableDetection::NotFound => {
                Err(AppError::openclaw_not_found())
            }
        }
    }

    /// Builtins + user profiles + the applied-state determination.
    ///
    /// A tool-policy read failure is non-fatal (`policyReadFailed: true`);
    /// a corrupt profile store is a hard error (fail-closed).
    pub fn list_security_profiles(&self) -> Result<SecurityProfileList, AppError> {
        let builtins = builtin_profiles();
        let users = self.store.list()?;

        // Builtins are checked first so a user copy of a builtin still
        // reports the builtin id (deterministic).
        let mut all: Vec<SecurityProfile> = builtins.clone();
        all.extend(users.iter().cloned());
        let (current_applied, policy_read_failed) = match self.current_policy() {
            Ok(policy) => (find_matching_profile(&policy, &all), false),
            Err(_) => (None, true),
        };
        Ok(SecurityProfileList {
            current_applied,
            policy_read_failed,
            builtins,
            users,
        })
    }

    fn current_policy(&self) -> Result<crate::domain::models::tools::ToolPolicy, AppError> {
        let exe = self.executable()?;
        let raw = self.config.read_raw(&exe, "tools")?;
        let value: serde_json::Value = match raw {
            None => serde_json::Value::Null,
            Some(text) => serde_json::from_str(&text).map_err(|err| {
                AppError::openclaw_config_read_failed(format!(
                    "the tools policy snapshot is not valid JSON: {err}"
                ))
            })?,
        };
        Ok(parse_tool_policy(&value))
    }

    /// Inserts or replaces a user profile (upsert).
    ///
    /// Fail-closed order: full field validation (0 I/O) → builtin id
    /// collision (`security-profile-conflict`, builtins are immutable) →
    /// store write. Config is never touched by a save.
    pub fn save_security_profile(&self, profile: &SecurityProfile) -> Result<(), AppError> {
        validate_profile(profile)?;
        if is_builtin_profile_id(&profile.id) {
            return Err(AppError::security_profile_conflict(&profile.id));
        }
        self.store.save(profile)
    }

    /// Deletes a user profile. Builtin ids are not stored, so delete
    /// attempts against them are `security-profile-not-found` (contract §2).
    pub fn delete_security_profile(&self, id: &str) -> Result<(), AppError> {
        validate_profile_slug(id)?;
        if is_builtin_profile_id(id) {
            return Err(AppError::security_profile_not_found(id));
        }
        match self.store.delete(id)? {
            Some(_) => Ok(()),
            None => Err(AppError::security_profile_not_found(id)),
        }
    }

    /// Resolves a profile by id: builtin first, then the user store.
    fn resolve_profile(&self, id: &str) -> Result<SecurityProfile, AppError> {
        if let Some(builtin) = builtin_profiles().into_iter().find(|p| p.id == id) {
            return Ok(builtin);
        }
        if is_builtin_profile_id(id) {
            // Unreachable (all builtin ids are in the list above); kept as a
            // defensive fail-closed branch.
            return Err(AppError::security_profile_not_found(id));
        }
        match self.store.get(id)? {
            Some(profile) => Ok(profile),
            None => Err(AppError::security_profile_not_found(id)),
        }
    }

    /// Applies the profile's four fields to the config, in a fixed order:
    /// `tools.profile` → `tools.allow` → `tools.deny` → `tools.exec.mode`
    /// (each dry-run → commit). The first failure stops the sequence; the
    /// caller must re-query the actual state (no optimistic updates).
    ///
    /// Builtin profiles apply through this same path (no special case).
    pub fn apply_security_profile(&self, id: &str) -> Result<(), AppError> {
        validate_profile_slug(id)?;
        let profile = self.resolve_profile(id)?;
        let exe = self.executable()?;
        self.write_field(
            &exe,
            "tools.profile",
            &profile.base_profile,
            WriteMode::Plain,
        )?;
        self.write_field(&exe, "tools.allow", &profile.allow, WriteMode::Replace)?;
        self.write_field(&exe, "tools.deny", &profile.deny, WriteMode::Replace)?;
        self.write_field(
            &exe,
            "tools.exec.mode",
            &profile.exec_mode,
            WriteMode::Plain,
        )
    }

    fn write_field<T: serde::Serialize>(
        &self,
        exe: &Path,
        path: &str,
        value: &T,
        mode: WriteMode,
    ) -> Result<(), AppError> {
        let value_json =
            serde_json::to_string(value).expect("validated policy values always serialize");
        self.config.write(exe, path, &value_json, mode)
    }

    /// Cold, read-only security audit (no `--deep`/`--fix`, no credentials).
    /// Failure leaves the result unknown — the UI shows "audit failed" and
    /// never assumes a clean state.
    pub fn run_security_audit(&self) -> Result<SecurityAuditResult, AppError> {
        let exe = self.executable()?;
        self.security.run_security_audit(&exe)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::models::{ModelRow, ProviderDetail, ThinkingLevel};
    use crate::domain::models::openclaw::{GatewayStatus, OpenClawVersion, UpdateState};
    use std::path::Path;
    use std::sync::Mutex;

    const EXE: &str = "C:\\fake\\openclaw.exe";

    struct FixedOpenClaw;

    impl OpenClawPort for FixedOpenClaw {
        fn detect_executable(&self) -> crate::domain::models::ExecutableDetection {
            crate::domain::models::ExecutableDetection::Found {
                path: PathBuf::from(EXE),
            }
        }
        fn version(&self, _exe: &Path) -> Result<OpenClawVersion, AppError> {
            unimplemented!()
        }
        fn version_from_entry(
            &self,
            _node: &Path,
            _entry: &Path,
        ) -> Result<OpenClawVersion, AppError> {
            unimplemented!()
        }
        fn gateway_status(&self, _exe: &Path) -> Result<GatewayStatus, AppError> {
            unimplemented!()
        }
        fn update_state(&self, _exe: &Path) -> Result<UpdateState, AppError> {
            unimplemented!()
        }
    }

    struct NoOpenClaw;

    impl OpenClawPort for NoOpenClaw {
        fn detect_executable(&self) -> crate::domain::models::ExecutableDetection {
            crate::domain::models::ExecutableDetection::NotFound
        }
        fn version(&self, _exe: &Path) -> Result<OpenClawVersion, AppError> {
            unimplemented!()
        }
        fn version_from_entry(
            &self,
            _node: &Path,
            _entry: &Path,
        ) -> Result<OpenClawVersion, AppError> {
            unimplemented!()
        }
        fn gateway_status(&self, _exe: &Path) -> Result<GatewayStatus, AppError> {
            unimplemented!()
        }
        fn update_state(&self, _exe: &Path) -> Result<UpdateState, AppError> {
            unimplemented!()
        }
    }

    struct FakeSecurity {
        result: Mutex<Result<SecurityAuditResult, AppError>>,
        calls: Arc<Mutex<u32>>,
    }

    impl OpenClawSecurityPort for FakeSecurity {
        fn run_security_audit(&self, _exe: &Path) -> Result<SecurityAuditResult, AppError> {
            *self.calls.lock().unwrap() += 1;
            self.result.lock().unwrap().clone()
        }
    }

    struct FakeConfig {
        tools_raw: Mutex<Option<String>>,
        log: Arc<Mutex<Vec<String>>>,
        failure: Arc<Mutex<Option<AppError>>>,
    }

    impl OpenClawConfigPort for FakeConfig {
        fn config_path(&self, _exe: &Path) -> Result<PathBuf, AppError> {
            unimplemented!()
        }
        fn read_providers(&self, _exe: &Path) -> Result<Vec<ProviderDetail>, AppError> {
            unimplemented!()
        }
        fn read_models(&self, _exe: &Path) -> Result<Vec<ModelRow>, AppError> {
            unimplemented!()
        }
        fn read_default_model(&self, _exe: &Path) -> Result<Option<String>, AppError> {
            unimplemented!()
        }
        fn read_thinking_default(&self, _exe: &Path) -> Result<Option<ThinkingLevel>, AppError> {
            unimplemented!()
        }
        fn write(
            &self,
            _exe: &Path,
            path: &str,
            value_json: &str,
            mode: WriteMode,
        ) -> Result<(), AppError> {
            // Log the attempt first so a rejected write is still counted as
            // "the sequence reached this field and stopped".
            self.log
                .lock()
                .unwrap()
                .push(format!("write:{path}={value_json}:{mode:?}"));
            if let Some(failure) = self.failure.lock().unwrap().clone() {
                return Err(failure);
            }
            Ok(())
        }
        fn unset(&self, _exe: &Path, _path: &str) -> Result<(), AppError> {
            unimplemented!()
        }
        fn set_default_model(&self, _exe: &Path, _model_ref: &str) -> Result<(), AppError> {
            unimplemented!()
        }
        fn read_raw(&self, _exe: &Path, path: &str) -> Result<Option<String>, AppError> {
            assert_eq!(path, "tools", "the service reads only `tools`");
            Ok(self.tools_raw.lock().unwrap().clone())
        }
    }

    struct FakeStore {
        profiles: Arc<Mutex<Vec<SecurityProfile>>>,
        corrupt: Arc<Mutex<bool>>,
    }

    impl SecurityProfileStorePort for FakeStore {
        fn list(&self) -> Result<Vec<SecurityProfile>, AppError> {
            if *self.corrupt.lock().unwrap() {
                return Err(AppError::security_profile_store_failed("injected"));
            }
            Ok(self.profiles.lock().unwrap().clone())
        }
        fn get(&self, id: &str) -> Result<Option<SecurityProfile>, AppError> {
            if *self.corrupt.lock().unwrap() {
                return Err(AppError::security_profile_store_failed("injected"));
            }
            Ok(self
                .profiles
                .lock()
                .unwrap()
                .iter()
                .find(|p| p.id == id)
                .cloned())
        }
        fn save(&self, profile: &SecurityProfile) -> Result<(), AppError> {
            if *self.corrupt.lock().unwrap() {
                return Err(AppError::security_profile_store_failed("injected"));
            }
            let mut profiles = self.profiles.lock().unwrap();
            if let Some(existing) = profiles.iter_mut().find(|p| p.id == profile.id) {
                *existing = profile.clone();
            } else {
                profiles.push(profile.clone());
            }
            Ok(())
        }
        fn delete(&self, id: &str) -> Result<Option<SecurityProfile>, AppError> {
            if *self.corrupt.lock().unwrap() {
                return Err(AppError::security_profile_store_failed("injected"));
            }
            let mut profiles = self.profiles.lock().unwrap();
            Ok(profiles
                .iter()
                .position(|p| p.id == id)
                .map(|index| profiles.remove(index)))
        }
    }

    fn user_profile(id: &str, name: &str) -> SecurityProfile {
        SecurityProfile {
            id: id.into(),
            name: name.into(),
            base_profile: "messaging".into(),
            allow: Vec::new(),
            deny: vec!["group:automation".into()],
            exec_mode: "ask".into(),
        }
    }

    #[allow(clippy::type_complexity)]
    fn service(
        openclaw: Arc<dyn OpenClawPort>,
        security_result: Result<SecurityAuditResult, AppError>,
        tools_raw: Option<String>,
        profiles: Vec<SecurityProfile>,
    ) -> (
        SecurityProfileService,
        Arc<Mutex<Vec<String>>>,
        Arc<Mutex<Option<AppError>>>,
        Arc<Mutex<bool>>,
        Arc<Mutex<u32>>,
        Arc<Mutex<Vec<SecurityProfile>>>,
    ) {
        let security_calls: Arc<Mutex<u32>> = Arc::new(Mutex::new(0));
        let security = Arc::new(FakeSecurity {
            result: Mutex::new(security_result),
            calls: Arc::clone(&security_calls),
        });
        let config_log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let config_failure: Arc<Mutex<Option<AppError>>> = Arc::new(Mutex::new(None));
        let config = Arc::new(FakeConfig {
            tools_raw: Mutex::new(tools_raw),
            log: Arc::clone(&config_log),
            failure: Arc::clone(&config_failure),
        });
        let store_corrupt: Arc<Mutex<bool>> = Arc::new(Mutex::new(false));
        let store_profiles: Arc<Mutex<Vec<SecurityProfile>>> = Arc::new(Mutex::new(profiles));
        let store = Arc::new(FakeStore {
            profiles: Arc::clone(&store_profiles),
            corrupt: Arc::clone(&store_corrupt),
        });
        (
            SecurityProfileService::new(openclaw, security, config, store),
            config_log,
            config_failure,
            store_corrupt,
            security_calls,
            store_profiles,
        )
    }

    fn audit_ok() -> Result<SecurityAuditResult, AppError> {
        Ok(SecurityAuditResult {
            summary: serde_json::json!({"total": 0}),
            findings: Vec::new(),
            suppressed_count: 0,
        })
    }

    // --- save -------------------------------------------------------------

    #[test]
    fn save_inserts_valid_profile() {
        let (service, log, _failure, _corrupt, _calls, profiles) =
            service(Arc::new(FixedOpenClaw), audit_ok(), None, Vec::new());
        let profile = user_profile("my-profile", "내 프로필");
        service.save_security_profile(&profile).expect("save");
        assert!(
            log.lock().unwrap().is_empty(),
            "save never touches the config"
        );
        let stored = profiles.lock().unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].id, "my-profile");
        // Upsert: saving the same id replaces in place.
        drop(stored);
        let mut updated = profile.clone();
        updated.name = "업데이트".into();
        service.save_security_profile(&updated).expect("upsert");
        let stored = profiles.lock().unwrap();
        assert_eq!(stored.len(), 1, "upsert must not duplicate");
        assert_eq!(stored[0].name, "업데이트");
    }

    #[test]
    fn save_builtin_id_is_conflict_with_zero_io() {
        let (service, log, _failure, _corrupt, _calls, _profiles) =
            service(Arc::new(FixedOpenClaw), audit_ok(), None, Vec::new());
        for id in ["default", "hardened"] {
            let mut profile = user_profile("", "");
            profile.id = id.into();
            profile.name = "x".into();
            let err = service
                .save_security_profile(&profile)
                .expect_err("builtin collision");
            assert_eq!(err.code, "security-profile-conflict");
        }
        assert!(log.lock().unwrap().is_empty());
    }

    #[test]
    fn save_invalid_fields_have_zero_io() {
        let (service, log, _failure, _corrupt, _calls, _profiles) =
            service(Arc::new(FixedOpenClaw), audit_ok(), None, Vec::new());
        let cases: Vec<(SecurityProfile, &str)> = vec![
            (user_profile("Bad-ID", "x"), "security-profile-id-invalid"),
            (user_profile("ok-id", ""), "security-profile-name-invalid"),
            (
                {
                    let mut p = user_profile("ok-id", "x");
                    p.base_profile = "nope".into();
                    p
                },
                "tool-profile-invalid",
            ),
            (
                {
                    let mut p = user_profile("ok-id", "x");
                    p.exec_mode = "nope".into();
                    p
                },
                "exec-mode-invalid",
            ),
            (
                {
                    let mut p = user_profile("ok-id", "x");
                    p.allow.push("../evil".into());
                    p
                },
                "tool-entry-invalid",
            ),
        ];
        for (profile, expected_code) in cases {
            let err = service
                .save_security_profile(&profile)
                .expect_err("must be rejected");
            assert_eq!(err.code, expected_code, "{:?}", profile.id);
        }
        assert!(log.lock().unwrap().is_empty());
    }

    #[test]
    fn save_store_failure_propagates() {
        let (service, _log, _failure, corrupt, _calls, _profiles) =
            service(Arc::new(FixedOpenClaw), audit_ok(), None, Vec::new());
        *corrupt.lock().unwrap() = true;
        let err = service
            .save_security_profile(&user_profile("my-profile", "x"))
            .expect_err("store failure");
        assert_eq!(err.code, "security-profile-store-failed");
    }

    // --- delete -------------------------------------------------------------

    #[test]
    fn delete_builtin_id_is_not_found_with_zero_io() {
        let (service, log, _failure, _corrupt, _calls, _profiles) =
            service(Arc::new(FixedOpenClaw), audit_ok(), None, Vec::new());
        let err = service
            .delete_security_profile("default")
            .expect_err("builtin delete");
        assert_eq!(err.code, "security-profile-not-found");
        assert!(log.lock().unwrap().is_empty());
    }

    #[test]
    fn delete_unknown_id_is_not_found() {
        let (service, _log, _failure, _corrupt, _calls, _profiles) = service(
            Arc::new(FixedOpenClaw),
            audit_ok(),
            None,
            vec![user_profile("alpha", "x")],
        );
        let err = service
            .delete_security_profile("ghost")
            .expect_err("unknown id");
        assert_eq!(err.code, "security-profile-not-found");
    }

    #[test]
    fn delete_invalid_slug_is_id_invalid() {
        let (service, _log, _failure, _corrupt, _calls, _profiles) =
            service(Arc::new(FixedOpenClaw), audit_ok(), None, Vec::new());
        let err = service
            .delete_security_profile("Bad ID")
            .expect_err("bad slug");
        assert_eq!(err.code, "security-profile-id-invalid");
    }

    // --- apply --------------------------------------------------------------

    #[test]
    fn apply_user_profile_writes_four_fields_in_order() {
        let (service, log, _failure, _corrupt, _calls, _profiles) = service(
            Arc::new(FixedOpenClaw),
            audit_ok(),
            None,
            vec![user_profile("alpha", "x")],
        );
        service.apply_security_profile("alpha").expect("apply");
        let writes = log.lock().unwrap().clone();
        assert_eq!(
            writes,
            vec![
                "write:tools.profile=\"messaging\":Plain",
                "write:tools.allow=[]:Replace",
                "write:tools.deny=[\"group:automation\"]:Replace",
                "write:tools.exec.mode=\"ask\":Plain",
            ]
        );
    }

    #[test]
    fn apply_builtin_profile_uses_same_path() {
        let (service, log, _failure, _corrupt, _calls, _profiles) =
            service(Arc::new(FixedOpenClaw), audit_ok(), None, Vec::new());
        service
            .apply_security_profile("hardened")
            .expect("apply builtin");
        let writes = log.lock().unwrap().clone();
        assert_eq!(writes.len(), 4);
        assert_eq!(writes[0], "write:tools.profile=\"messaging\":Plain");
        assert_eq!(
            writes[2],
            "write:tools.deny=[\"group:automation\",\"group:runtime\",\"group:fs\",\"sessions_spawn\",\"sessions_send\"]:Replace"
        );
        assert_eq!(writes[3], "write:tools.exec.mode=\"deny\":Plain");
    }

    #[test]
    fn apply_unknown_id_is_not_found_with_zero_cli_calls() {
        let (service, log, _failure, _corrupt, _calls, _profiles) =
            service(Arc::new(FixedOpenClaw), audit_ok(), None, Vec::new());
        let err = service
            .apply_security_profile("ghost")
            .expect_err("unknown id");
        assert_eq!(err.code, "security-profile-not-found");
        assert!(log.lock().unwrap().is_empty(), "no config write");
    }

    #[test]
    fn apply_stops_on_first_failure_no_partial_continue() {
        let (service, log, failure, _corrupt, _calls, _profiles) = service(
            Arc::new(FixedOpenClaw),
            audit_ok(),
            None,
            vec![user_profile("alpha", "x")],
        );
        *failure.lock().unwrap() = Some(AppError::openclaw_config_invalid("schema reject"));
        let err = service
            .apply_security_profile("alpha")
            .expect_err("first write rejected");
        assert_eq!(err.code, "openclaw-config-invalid");
        let writes = log.lock().unwrap().clone();
        assert_eq!(
            writes.len(),
            1,
            "sequence must stop after the first failure"
        );
        assert!(writes[0].starts_with("write:tools.profile="));
    }

    #[test]
    fn apply_missing_executable_is_openclaw_not_found() {
        let (service, log, _failure, _corrupt, _calls, _profiles) = service(
            Arc::new(NoOpenClaw),
            audit_ok(),
            None,
            vec![user_profile("alpha", "x")],
        );
        let err = service
            .apply_security_profile("alpha")
            .expect_err("not found");
        assert_eq!(err.code, "openclaw-not-found");
        assert!(log.lock().unwrap().is_empty());
    }

    // --- list ------------------------------------------------------------------

    #[test]
    fn list_combines_builtins_users_and_applied_state() {
        let (service, _log, _failure, _corrupt, _calls, _profiles) = service(
            Arc::new(FixedOpenClaw),
            audit_ok(),
            Some(
                r#"{"profile":"messaging","deny":["group:automation"],"exec":{"mode":"ask"}}"#
                    .into(),
            ),
            vec![user_profile("alpha", "x")],
        );
        let list = service.list_security_profiles().expect("list");
        assert_eq!(list.builtins.len(), 2);
        assert_eq!(list.users.len(), 1);
        assert_eq!(list.users[0].id, "alpha");
        // The seeded policy matches user profile `alpha` exactly.
        assert_eq!(list.current_applied.as_deref(), Some("alpha"));
        assert!(!list.policy_read_failed);
    }

    #[test]
    fn list_builtin_priority_for_matching_policy() {
        // A user profile identical to a builtin: the builtin id wins.
        let mut copy = builtin_profiles()[0].clone();
        copy.id = "default-copy".into();
        let (service, _log, _failure, _corrupt, _calls, _profiles) = service(
            Arc::new(FixedOpenClaw),
            audit_ok(),
            Some(r#"{"profile":"coding"}"#.into()),
            vec![copy],
        );
        let list = service.list_security_profiles().expect("list");
        assert_eq!(list.current_applied.as_deref(), Some("default"));
    }

    #[test]
    fn list_custom_policy_has_no_applied_profile() {
        let (service, _log, _failure, _corrupt, _calls, _profiles) = service(
            Arc::new(FixedOpenClaw),
            audit_ok(),
            Some(r#"{"profile":"full","deny":["web_search"]}"#.into()),
            Vec::new(),
        );
        let list = service.list_security_profiles().expect("list");
        assert_eq!(list.current_applied, None);
        assert!(!list.policy_read_failed);
    }

    #[test]
    fn list_policy_read_failure_degrades_not_fails() {
        let (service, _log, _failure, _corrupt, _calls, _profiles) = service(
            Arc::new(NoOpenClaw),
            audit_ok(),
            None,
            vec![user_profile("alpha", "x")],
        );
        let list = service.list_security_profiles().expect("list");
        assert!(list.policy_read_failed);
        assert_eq!(list.current_applied, None);
        assert_eq!(list.builtins.len(), 2, "builtins still shown");
        assert_eq!(list.users.len(), 1, "user store still listed");
    }

    #[test]
    fn list_corrupt_store_is_hard_error() {
        let (service, _log, _failure, corrupt, _calls, _profiles) =
            service(Arc::new(FixedOpenClaw), audit_ok(), None, Vec::new());
        *corrupt.lock().unwrap() = true;
        let err = service.list_security_profiles().expect_err("corrupt store");
        assert_eq!(err.code, "security-profile-store-failed");
    }

    // --- audit -------------------------------------------------------------------

    #[test]
    fn run_audit_delegates_to_port() {
        let (service, _log, _failure, _corrupt, calls, _profiles) =
            service(Arc::new(FixedOpenClaw), audit_ok(), None, Vec::new());
        let result = service.run_security_audit().expect("audit");
        assert_eq!(result.findings.len(), 0);
        assert_eq!(*calls.lock().unwrap(), 1);
    }

    #[test]
    fn run_audit_failure_is_stable_and_no_retry() {
        let (service, _log, _failure, _corrupt, calls, _profiles) = service(
            Arc::new(FixedOpenClaw),
            Err(AppError::openclaw_security_audit_failed("injected")),
            None,
            Vec::new(),
        );
        let err = service.run_security_audit().expect_err("audit failure");
        assert_eq!(err.code, "openclaw-security-audit-failed");
        assert_eq!(*calls.lock().unwrap(), 1, "no silent retry");
    }

    #[test]
    fn run_audit_missing_executable_is_openclaw_not_found() {
        let (service, _log, _failure, _corrupt, calls, _profiles) =
            service(Arc::new(NoOpenClaw), audit_ok(), None, Vec::new());
        let err = service.run_security_audit().expect_err("not found");
        assert_eq!(err.code, "openclaw-not-found");
        assert_eq!(*calls.lock().unwrap(), 0, "no CLI call");
    }
}
