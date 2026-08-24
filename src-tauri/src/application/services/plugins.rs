//! Plugins use case (Phase 4).
//!
//! Orchestration: validate the plugin id (S2, 0 process runs on failure) →
//! detect the OpenClaw executable → dedicated CLI invocation via the plugins
//! port. Toggles are fail-closed: on failure the UI re-queries the list and
//! shows the actual state (no optimistic updates).

use std::path::PathBuf;
use std::sync::Arc;

use super::environment::default_openclaw_search_dirs;
use crate::domain::models::skills::{validate_plugin_id, PluginRow, PluginRuntime};
use crate::domain::ports::openclaw::OpenClawPort;
use crate::domain::ports::openclaw_plugins::OpenClawPluginsPort;
use crate::error::AppError;
use crate::infrastructure::openclaw::{OpenClawAdapter, OpenClawPluginsAdapter};
use crate::infrastructure::process::ProcessRunner;

/// Use case layer: composes the OpenClaw executable and plugins ports.
pub struct PluginsService {
    openclaw: Arc<dyn OpenClawPort>,
    plugins: Arc<dyn OpenClawPluginsPort>,
}

impl PluginsService {
    pub fn new(openclaw: Arc<dyn OpenClawPort>, plugins: Arc<dyn OpenClawPluginsPort>) -> Self {
        Self { openclaw, plugins }
    }

    /// Production wiring: real adapters over the single `ProcessRunner`.
    pub fn production() -> Self {
        let process: Arc<dyn crate::domain::ports::process::ProcessPort> = Arc::new(ProcessRunner);
        let openclaw = Arc::new(OpenClawAdapter::new(
            Arc::clone(&process),
            default_openclaw_search_dirs(),
        ));
        let plugins = Arc::new(OpenClawPluginsAdapter::new(process));
        Self::new(openclaw, plugins)
    }

    fn executable(&self) -> Result<PathBuf, AppError> {
        match self.openclaw.detect_executable() {
            crate::domain::models::ExecutableDetection::Found { path } => Ok(path),
            crate::domain::models::ExecutableDetection::NotFound => {
                Err(AppError::openclaw_not_found())
            }
        }
    }

    /// All plugin rows (`openclaw plugins list --json`, cold read).
    pub fn list_plugins(&self) -> Result<Vec<PluginRow>, AppError> {
        let exe = self.executable()?;
        self.plugins.list_plugins(&exe)
    }

    /// Enables/disables a plugin (`openclaw plugins enable/disable <id>`).
    ///
    /// The id is validated before any process run and passed as a single
    /// argv element. Non-zero exit is a structured error; the caller must
    /// re-query the list for the actual state (the CLI may have already
    /// changed it).
    pub fn set_plugin_enabled(&self, id: &str, enabled: bool) -> Result<(), AppError> {
        validate_plugin_id(id)?;
        let exe = self.executable()?;
        if enabled {
            self.plugins.enable_plugin(&exe, id)
        } else {
            self.plugins.disable_plugin(&exe, id)
        }
    }

    /// The live runtime surface of one plugin (on-demand; it loads plugin
    /// modules, so it is intentionally not run during list loading).
    pub fn get_plugin_runtime(&self, id: &str) -> Result<PluginRuntime, AppError> {
        validate_plugin_id(id)?;
        let exe = self.executable()?;
        self.plugins.inspect_plugin_runtime(&exe, id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::openclaw::{GatewayStatus, OpenClawVersion, UpdateState};
    use std::path::{Path, PathBuf};
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

    struct FakePlugins {
        log: Arc<Mutex<Vec<String>>>,
        failure: Mutex<Option<AppError>>,
    }

    impl OpenClawPluginsPort for FakePlugins {
        fn list_plugins(&self, _exe: &Path) -> Result<Vec<PluginRow>, AppError> {
            if let Some(failure) = self.failure.lock().unwrap().clone() {
                return Err(failure);
            }
            self.log.lock().unwrap().push("list".to_string());
            Ok(vec![PluginRow {
                id: "discord".into(),
                enabled: Some(true),
                name: None,
                format: None,
                origin: None,
                version: None,
                dependency_status: None,
            }])
        }
        fn enable_plugin(&self, _exe: &Path, id: &str) -> Result<(), AppError> {
            if let Some(failure) = self.failure.lock().unwrap().clone() {
                return Err(failure);
            }
            self.log.lock().unwrap().push(format!("enable:{id}"));
            Ok(())
        }
        fn disable_plugin(&self, _exe: &Path, id: &str) -> Result<(), AppError> {
            if let Some(failure) = self.failure.lock().unwrap().clone() {
                return Err(failure);
            }
            self.log.lock().unwrap().push(format!("disable:{id}"));
            Ok(())
        }
        fn inspect_plugin_runtime(&self, _exe: &Path, id: &str) -> Result<PluginRuntime, AppError> {
            if let Some(failure) = self.failure.lock().unwrap().clone() {
                return Err(failure);
            }
            self.log.lock().unwrap().push(format!("inspect:{id}"));
            Ok(PluginRuntime {
                id: id.to_string(),
                ..Default::default()
            })
        }
    }

    fn fake_plugins() -> (Arc<FakePlugins>, Arc<Mutex<Vec<String>>>) {
        let log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let fake = Arc::new(FakePlugins {
            log: Arc::clone(&log),
            failure: Mutex::new(None),
        });
        (fake, log)
    }

    fn service(fake: Arc<FakePlugins>) -> PluginsService {
        PluginsService::new(Arc::new(FixedOpenClaw), fake)
    }

    #[test]
    fn set_plugin_enabled_dispatches_enable() {
        let (fake, log) = fake_plugins();
        let service = service(fake);
        service
            .set_plugin_enabled("@openclaw/discord", true)
            .expect("enable");
        assert_eq!(log.lock().unwrap()[0], "enable:@openclaw/discord");
    }

    #[test]
    fn set_plugin_enabled_dispatches_disable() {
        let (fake, log) = fake_plugins();
        let service = service(fake);
        service
            .set_plugin_enabled("local-plugin", false)
            .expect("disable");
        assert_eq!(log.lock().unwrap()[0], "disable:local-plugin");
    }

    #[test]
    fn invalid_id_has_zero_cli_calls() {
        let (fake, log) = fake_plugins();
        let service = service(fake);
        for id in ["a b", "..", "a/b", "", ".hidden"] {
            let err = service
                .set_plugin_enabled(id, true)
                .expect_err("must be rejected");
            assert_eq!(err.code, "plugin-id-invalid", "{id:?}");
            let err = service
                .get_plugin_runtime(id)
                .expect_err("must be rejected");
            assert_eq!(err.code, "plugin-id-invalid", "{id:?}");
        }
        assert!(log.lock().unwrap().is_empty(), "no CLI calls at all");
    }

    #[test]
    fn toggle_failure_maps_to_toggle_failed() {
        let (fake, _log) = fake_plugins();
        let service = service(fake.clone());
        *fake.failure.lock().unwrap() = Some(AppError::openclaw_plugin_toggle_failed(
            "nope",
            "exit code 2: unknown",
        ));
        let err = service
            .set_plugin_enabled("nope", true)
            .expect_err("toggle failure");
        assert_eq!(err.code, "openclaw-plugin-toggle-failed");
    }

    #[test]
    fn list_and_runtime_delegate_to_port() {
        let (fake, log) = fake_plugins();
        let service = service(fake);
        let rows = service.list_plugins().expect("list");
        assert_eq!(rows[0].id, "discord");
        let runtime = service
            .get_plugin_runtime("@openclaw/discord")
            .expect("runtime");
        assert_eq!(runtime.id, "@openclaw/discord");
        assert_eq!(
            &*log.lock().unwrap(),
            &["list".to_string(), "inspect:@openclaw/discord".to_string()]
        );
    }
}
