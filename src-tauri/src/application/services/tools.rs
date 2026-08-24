//! Tool policy use case (Phase 5).
//!
//! Orchestration: validate user input (S2, 0 process runs on failure) →
//! detect the OpenClaw executable → read/write via the Phase 3
//! `OpenClawConfigPort` (structured argv, dry-run → commit inside the port).
//!
//! The global `tools.*` config surface is the only tool-policy surface
//! touched: `tools.profile`, `tools.allow`, `tools.deny`, `tools.exec.mode`.
//! `tools.elevated.enabled`/`tools.fs.workspaceOnly` are read-only display.

use std::path::PathBuf;
use std::sync::Arc;

use super::environment::default_openclaw_search_dirs;
use crate::domain::models::tools::{
    parse_tool_policy, validate_exec_mode, validate_tool_entry, validate_tool_profile, ToolPolicy,
};
use crate::domain::ports::openclaw::OpenClawPort;
use crate::domain::ports::openclaw_config::{OpenClawConfigPort, WriteMode};
use crate::error::AppError;
use crate::infrastructure::openclaw::{OpenClawAdapter, OpenClawConfigAdapter};
use crate::infrastructure::process::ProcessRunner;

/// Use case layer: composes the OpenClaw executable and config ports. No
/// dedicated tool-policy port exists (contract §4): the Phase 3 config port
/// is the write path.
pub struct ToolPolicyService {
    openclaw: Arc<dyn OpenClawPort>,
    config: Arc<dyn OpenClawConfigPort>,
}

impl ToolPolicyService {
    pub fn new(openclaw: Arc<dyn OpenClawPort>, config: Arc<dyn OpenClawConfigPort>) -> Self {
        Self { openclaw, config }
    }

    /// Production wiring: real adapters over the single `ProcessRunner`.
    pub fn production() -> Self {
        let process: Arc<dyn crate::domain::ports::process::ProcessPort> = Arc::new(ProcessRunner);
        let openclaw = Arc::new(OpenClawAdapter::new(
            Arc::clone(&process),
            default_openclaw_search_dirs(),
        ));
        let config = Arc::new(OpenClawConfigAdapter::new(process));
        Self::new(openclaw, config)
    }

    fn executable(&self) -> Result<PathBuf, AppError> {
        match self.openclaw.detect_executable() {
            crate::domain::models::ExecutableDetection::Found { path } => Ok(path),
            crate::domain::models::ExecutableDetection::NotFound => {
                Err(AppError::openclaw_not_found())
            }
        }
    }

    /// The current tool policy (`openclaw config get tools --json`,
    /// read-only, redacted). A missing `tools` section reads as the empty
    /// policy (fail-soft: unset fields are `null`/empty).
    pub fn get_tool_policy(&self) -> Result<ToolPolicy, AppError> {
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

    /// Sets `tools.profile` (scalar, dry-run → commit).
    ///
    /// Fail-closed: enum validation (0 CLI calls) → executable detection →
    /// two-step config write.
    pub fn set_tool_profile(&self, profile: &str) -> Result<(), AppError> {
        validate_tool_profile(profile)?;
        let exe = self.executable()?;
        let value_json = serde_json::to_string(profile).expect("a &str always serializes to JSON");
        self.config
            .write(&exe, "tools.profile", &value_json, WriteMode::Plain)
    }

    /// Replaces the whole `tools.allow` array (dry-run → commit, `--replace`).
    ///
    /// Fail-closed: every entry is validated before any CLI call (0 calls on
    /// the first invalid entry); the array is a single JSON argv element
    /// (S1/S2).
    pub fn set_tool_allow(&self, entries: &[String]) -> Result<(), AppError> {
        self.set_tool_entries("tools.allow", entries)
    }

    /// Replaces the whole `tools.deny` array (deny wins over allow).
    pub fn set_tool_deny(&self, entries: &[String]) -> Result<(), AppError> {
        self.set_tool_entries("tools.deny", entries)
    }

    fn set_tool_entries(&self, path: &str, entries: &[String]) -> Result<(), AppError> {
        for entry in entries {
            validate_tool_entry(entry)?;
        }
        let exe = self.executable()?;
        let value_json =
            serde_json::to_string(entries).expect("a &[String] always serializes to JSON");
        self.config
            .write(&exe, path, &value_json, WriteMode::Replace)
    }

    /// Sets `tools.exec.mode` (scalar, dry-run → commit).
    pub fn set_exec_mode(&self, mode: &str) -> Result<(), AppError> {
        validate_exec_mode(mode)?;
        let exe = self.executable()?;
        let value_json = serde_json::to_string(mode).expect("a &str always serializes to JSON");
        self.config
            .write(&exe, "tools.exec.mode", &value_json, WriteMode::Plain)
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

    /// Records every write; `read_raw` returns the seeded JSON text.
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
            if let Some(failure) = self.failure.lock().unwrap().clone() {
                return Err(failure);
            }
            self.log
                .lock()
                .unwrap()
                .push(format!("write:{path}={value_json}:{mode:?}"));
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

    fn service(
        openclaw: Arc<dyn OpenClawPort>,
        tools_raw: Option<String>,
        log: Arc<Mutex<Vec<String>>>,
    ) -> (ToolPolicyService, Arc<Mutex<Option<AppError>>>) {
        let failure: Arc<Mutex<Option<AppError>>> = Arc::new(Mutex::new(None));
        let config = Arc::new(FakeConfig {
            tools_raw: Mutex::new(tools_raw),
            log,
            failure: Arc::clone(&failure),
        });
        (ToolPolicyService::new(openclaw, config), failure)
    }

    #[test]
    fn get_tool_policy_parses_snapshot() {
        let log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let (service, _failure) = service(
            Arc::new(FixedOpenClaw),
            Some(r#"{"profile":"coding","allow":["web_search"],"deny":["group:fs"],"exec":{"mode":"ask"},"elevated":{"enabled":true},"fs":{"workspaceOnly":true}}"#.into()),
            Arc::clone(&log),
        );
        let policy = service.get_tool_policy().expect("read");
        assert_eq!(policy.profile.as_deref(), Some("coding"));
        assert_eq!(policy.allow, vec!["web_search"]);
        assert_eq!(policy.deny, vec!["group:fs"]);
        assert_eq!(policy.exec_mode.as_deref(), Some("ask"));
        assert_eq!(policy.elevated_enabled, Some(true));
        assert_eq!(policy.fs_workspace_only, Some(true));
        assert!(log.lock().unwrap().is_empty(), "read is write-free");
    }

    #[test]
    fn get_tool_policy_missing_section_is_empty_policy() {
        let log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let (service, _failure) = service(Arc::new(FixedOpenClaw), None, log);
        let policy = service.get_tool_policy().expect("read");
        assert_eq!(policy, ToolPolicy::default());
    }

    #[test]
    fn set_tool_profile_writes_json_scalar_plain() {
        let log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let (service, _failure) = service(Arc::new(FixedOpenClaw), None, Arc::clone(&log));
        service.set_tool_profile("messaging").expect("write");
        assert_eq!(
            log.lock().unwrap()[0],
            "write:tools.profile=\"messaging\":Plain"
        );
    }

    #[test]
    fn set_tool_profile_invalid_enum_has_zero_cli_calls() {
        let log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let (service, _failure) = service(Arc::new(FixedOpenClaw), None, Arc::clone(&log));
        for profile in ["", "Coding", "default", "coding "] {
            let err = service
                .set_tool_profile(profile)
                .expect_err("must be rejected");
            assert_eq!(err.code, "tool-profile-invalid", "{profile:?}");
        }
        assert!(log.lock().unwrap().is_empty(), "no CLI call at all");
    }

    #[test]
    fn set_tool_allow_writes_json_array_replace() {
        let log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let (service, _failure) = service(Arc::new(FixedOpenClaw), None, Arc::clone(&log));
        service
            .set_tool_allow(&["web_search".into(), "image*".into()])
            .expect("write");
        assert_eq!(
            log.lock().unwrap()[0],
            "write:tools.allow=[\"web_search\",\"image*\"]:Replace"
        );
    }

    #[test]
    fn set_tool_deny_writes_json_array_replace() {
        let log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let (service, _failure) = service(Arc::new(FixedOpenClaw), None, Arc::clone(&log));
        service.set_tool_deny(&["group:fs".into()]).expect("write");
        assert_eq!(
            log.lock().unwrap()[0],
            "write:tools.deny=[\"group:fs\"]:Replace"
        );
    }

    #[test]
    fn set_tool_entries_invalid_entry_has_zero_cli_calls() {
        let log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let (service, _failure) = service(Arc::new(FixedOpenClaw), None, Arc::clone(&log));
        for entries in [
            vec!["ok".to_string(), "../evil".to_string()],
            vec!["a/b".to_string()],
            vec!["a b".to_string()],
            vec![String::new()],
            vec!["x".repeat(129)],
            vec!["group:".to_string()],
        ] {
            let err = service
                .set_tool_allow(&entries)
                .expect_err("must be rejected");
            assert_eq!(err.code, "tool-entry-invalid", "{entries:?}");
            let err = service
                .set_tool_deny(&entries)
                .expect_err("must be rejected");
            assert_eq!(err.code, "tool-entry-invalid", "{entries:?}");
        }
        assert!(log.lock().unwrap().is_empty(), "no CLI call at all");
    }

    #[test]
    fn set_exec_mode_writes_json_scalar_plain() {
        let log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let (service, _failure) = service(Arc::new(FixedOpenClaw), None, Arc::clone(&log));
        service.set_exec_mode("ask").expect("write");
        assert_eq!(
            log.lock().unwrap()[0],
            "write:tools.exec.mode=\"ask\":Plain"
        );
    }

    #[test]
    fn set_exec_mode_invalid_enum_has_zero_cli_calls() {
        let log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let (service, _failure) = service(Arc::new(FixedOpenClaw), None, Arc::clone(&log));
        for mode in ["", "Full", "deny-all"] {
            let err = service.set_exec_mode(mode).expect_err("must be rejected");
            assert_eq!(err.code, "exec-mode-invalid", "{mode:?}");
        }
        assert!(log.lock().unwrap().is_empty(), "no CLI call at all");
    }

    #[test]
    fn missing_executable_is_openclaw_not_found() {
        let log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let (service, _failure) = service(Arc::new(NoOpenClaw), None, Arc::clone(&log));
        assert_eq!(
            service.get_tool_policy().unwrap_err().code,
            "openclaw-not-found"
        );
        assert_eq!(
            service.set_tool_profile("coding").unwrap_err().code,
            "openclaw-not-found"
        );
        assert_eq!(
            service.set_exec_mode("deny").unwrap_err().code,
            "openclaw-not-found"
        );
        assert!(log.lock().unwrap().is_empty());
    }

    #[test]
    fn config_rejection_maps_through_unwrapped() {
        let log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let config = Arc::new(FakeConfig {
            tools_raw: Mutex::new(None),
            log: Arc::clone(&log),
            failure: Arc::new(Mutex::new(Some(AppError::openclaw_config_invalid(
                "alsoAllow conflict",
            )))),
        });
        let service = ToolPolicyService::new(Arc::new(FixedOpenClaw), config);
        let err = service
            .set_tool_allow(&["web_search".into()])
            .expect_err("rejected");
        assert_eq!(err.code, "openclaw-config-invalid");
        assert!(err.message.contains("alsoAllow conflict"));
    }
}
