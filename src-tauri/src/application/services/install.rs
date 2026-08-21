//! OpenClaw install use case (Phase 2).
//!
//! Orchestration:
//! existing OpenClaw detect → already installed: return existing version
//! → not installed: Node precondition → npm entry → npm version → npm version
//! policy → npm install → OpenClaw package/spawn entry resolve → post-install
//! detect/version → result.
//!
//! Fail-closed: every precondition failure returns a stable `AppError` before
//! any npm install spawn.

use std::sync::Arc;

use super::environment::default_openclaw_search_dirs;
use crate::domain::models::{ExecutableDetection, NodeDetection};
use crate::domain::ports::installer::OpenClawInstallerPort;
use crate::domain::ports::openclaw::OpenClawPort;
use crate::domain::ports::process::ProcessPort;
use crate::domain::ports::windows_system::WindowsSystemPort;
use crate::error::AppError;
use crate::infrastructure::openclaw::{
    node_version_supported, npm_install_policy, OpenClawAdapter, OpenClawInstaller,
};
use crate::infrastructure::process::ProcessRunner;
use crate::infrastructure::windows::WindowsSystemAdapter;

/// Structured install result (IPC wire type).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum InstallResult {
    /// OpenClaw was already installed; the existing version is returned
    /// unchanged (Phase 2 never updates an existing install).
    AlreadyInstalled { version: String },
    /// OpenClaw was installed by this call; the installed version is returned.
    Installed { version: String },
}

/// Use case layer: composes the Windows, OpenClaw, and installer ports.
pub struct InstallService {
    windows: Arc<dyn WindowsSystemPort>,
    openclaw: Arc<dyn OpenClawPort>,
    installer: Arc<dyn OpenClawInstallerPort>,
}

impl InstallService {
    pub fn new(
        windows: Arc<dyn WindowsSystemPort>,
        openclaw: Arc<dyn OpenClawPort>,
        installer: Arc<dyn OpenClawInstallerPort>,
    ) -> Self {
        Self {
            windows,
            openclaw,
            installer,
        }
    }

    /// Production wiring: real adapters over the single `ProcessRunner`.
    pub fn production() -> Self {
        let process: Arc<dyn ProcessPort> = Arc::new(ProcessRunner);
        let windows = Arc::new(WindowsSystemAdapter::new(Arc::clone(&process)));
        let openclaw = Arc::new(OpenClawAdapter::new(
            Arc::clone(&process),
            default_openclaw_search_dirs(),
        ));
        let installer = Arc::new(OpenClawInstaller::production(Arc::clone(&process)));
        Self::new(windows, openclaw, installer)
    }

    /// Installs `openclaw@latest`, or returns the existing install as-is.
    pub fn install_openclaw(&self) -> Result<InstallResult, AppError> {
        // Idempotency: an existing install is returned unchanged; no npm
        // version/install spawn happens on this path.
        if let ExecutableDetection::Found { .. } = self.openclaw.detect_executable() {
            return self.existing_install_version();
        }

        // Node preconditions (0 npm spawns on failure).
        match self.windows.detect_node()? {
            NodeDetection::NotFound => return Err(AppError::node_not_found()),
            NodeDetection::Found { version } => {
                if !node_version_supported(&version) {
                    return Err(AppError::unsupported_node_version(version));
                }
            }
        }
        let node_exe = self.windows.node_executable()?;

        // npm spawn entry + version policy (install spawn still 0 here).
        let npm = self.installer.resolve_npm_entry(&node_exe)?;
        let npm_version = self.installer.npm_version(&npm)?;
        let allow_scripts = npm_install_policy(&npm_version)?;

        // Install. Target is always `openclaw@latest`; no user input.
        self.installer
            .install_openclaw_latest(&npm, allow_scripts)?;

        // Post-install verification: package entry, detect, --version.
        let entry = self.installer.resolve_openclaw_entry()?;
        match self.openclaw.detect_executable() {
            ExecutableDetection::Found { .. } => {}
            ExecutableDetection::NotFound => {
                return Err(AppError::openclaw_install_verify_failed(
                    "OpenClaw executable was not found after install",
                ));
            }
        }
        let version = self
            .openclaw
            .version_from_entry(&node_exe, &entry.entry)
            .map_err(|err| verify_failed(&err))?;
        Ok(InstallResult::Installed {
            version: version.raw,
        })
    }

    /// Version of an existing install via the resolved package entry.
    ///
    /// Never spawns npm (0 npm version/install spawns).
    fn existing_install_version(&self) -> Result<InstallResult, AppError> {
        let node_exe = self.windows.node_executable()?;
        let entry = self.installer.resolve_openclaw_entry()?;
        let version = self
            .openclaw
            .version_from_entry(&node_exe, &entry.entry)
            .map_err(|err| verify_failed(&err))?;
        Ok(InstallResult::AlreadyInstalled {
            version: version.raw,
        })
    }
}

/// Maps any post-install verification error to the stable
/// `openclaw-install-verify-failed` code (detail preserved, already masked).
fn verify_failed(err: &AppError) -> AppError {
    AppError::openclaw_install_verify_failed(err.message.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::install::{NpmEntry, ResolvedOpenClawEntry};
    use crate::domain::models::openclaw::{GatewayStatus, OpenClawVersion, UpdateState};
    use crate::domain::models::{Architecture, WindowsVersion};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    const NODE: &str = "C:\\fake\\node.exe";
    const NPM_CLI: &str = "C:\\fake\\npm-cli.js";
    const OPENCLAW_ENTRY: &str = "C:\\fake\\npm\\node_modules\\openclaw\\openclaw.mjs";

    /// Shared call counters proving spawn-free failure paths.
    #[derive(Default)]
    struct Counters {
        npm_entry: AtomicUsize,
        npm_version: AtomicUsize,
        install: AtomicUsize,
        entry: AtomicUsize,
    }

    struct FakeWindows {
        node: Result<NodeDetection, AppError>,
        node_exe: Result<PathBuf, AppError>,
    }

    impl WindowsSystemPort for FakeWindows {
        fn os_version(&self) -> Result<WindowsVersion, AppError> {
            unimplemented!("not used by InstallService")
        }
        fn architecture(&self) -> Result<Architecture, AppError> {
            unimplemented!("not used by InstallService")
        }
        fn detect_node(&self) -> Result<NodeDetection, AppError> {
            self.node.clone()
        }
        fn node_executable(&self) -> Result<PathBuf, AppError> {
            self.node_exe.clone()
        }
    }

    fn windows_ok() -> FakeWindows {
        FakeWindows {
            node: Ok(NodeDetection::Found {
                version: "22.22.3".into(),
            }),
            node_exe: Ok(PathBuf::from(NODE)),
        }
    }

    struct FakeOpenClaw {
        detect: ExecutableDetection,
        version_from_entry: Result<String, AppError>,
    }

    impl OpenClawPort for FakeOpenClaw {
        fn detect_executable(&self) -> ExecutableDetection {
            self.detect.clone()
        }
        fn version(&self, _exe: &Path) -> Result<OpenClawVersion, AppError> {
            unimplemented!("InstallService uses version_from_entry")
        }
        fn version_from_entry(
            &self,
            _node: &Path,
            _entry: &Path,
        ) -> Result<OpenClawVersion, AppError> {
            self.version_from_entry
                .clone()
                .map(|raw| OpenClawVersion { raw })
        }
        fn gateway_status(&self, _exe: &Path) -> Result<GatewayStatus, AppError> {
            unimplemented!("not used by InstallService")
        }
        fn update_state(&self, _exe: &Path) -> Result<UpdateState, AppError> {
            unimplemented!("not used by InstallService")
        }
    }

    struct FakeInstaller {
        npm_entry: Result<NpmEntry, AppError>,
        npm_version: Result<String, AppError>,
        install_result: Result<(), AppError>,
        entry: Result<ResolvedOpenClawEntry, AppError>,
        counters: Arc<Counters>,
        allow_scripts: Mutex<Option<bool>>,
    }

    impl FakeInstaller {
        fn new(
            npm_entry: Result<NpmEntry, AppError>,
            npm_version: Result<String, AppError>,
            install_result: Result<(), AppError>,
            entry: Result<ResolvedOpenClawEntry, AppError>,
        ) -> Self {
            Self {
                npm_entry,
                npm_version,
                install_result,
                entry,
                counters: Arc::new(Counters::default()),
                allow_scripts: Mutex::new(None),
            }
        }
    }

    impl OpenClawInstallerPort for FakeInstaller {
        fn resolve_npm_entry(&self, _node: &Path) -> Result<NpmEntry, AppError> {
            self.counters.npm_entry.fetch_add(1, Ordering::SeqCst);
            self.npm_entry.clone()
        }
        fn npm_version(&self, _entry: &NpmEntry) -> Result<String, AppError> {
            self.counters.npm_version.fetch_add(1, Ordering::SeqCst);
            self.npm_version.clone()
        }
        fn install_openclaw_latest(
            &self,
            _entry: &NpmEntry,
            allow_scripts: bool,
        ) -> Result<(), AppError> {
            self.counters.install.fetch_add(1, Ordering::SeqCst);
            *self.allow_scripts.lock().unwrap() = Some(allow_scripts);
            self.install_result.clone()
        }
        fn resolve_openclaw_entry(&self) -> Result<ResolvedOpenClawEntry, AppError> {
            self.counters.entry.fetch_add(1, Ordering::SeqCst);
            self.entry.clone()
        }
    }

    fn npm_entry() -> NpmEntry {
        NpmEntry {
            node: PathBuf::from(NODE),
            npm_cli: PathBuf::from(NPM_CLI),
        }
    }

    fn resolved_entry() -> ResolvedOpenClawEntry {
        ResolvedOpenClawEntry {
            package_root: PathBuf::from("C:\\fake\\npm\\node_modules\\openclaw"),
            entry: PathBuf::from(OPENCLAW_ENTRY),
        }
    }

    fn not_found() -> ExecutableDetection {
        ExecutableDetection::NotFound
    }

    fn found() -> ExecutableDetection {
        ExecutableDetection::Found {
            path: PathBuf::from("C:\\fake\\npm\\openclaw.cmd"),
        }
    }

    fn ok_installer() -> FakeInstaller {
        FakeInstaller::new(
            Ok(npm_entry()),
            Ok("11.16.0".to_string()),
            Ok(()),
            Ok(resolved_entry()),
        )
    }

    /// Service with a supported Node ("22.22.3") and the given fakes.
    fn service(
        detect: ExecutableDetection,
        version: Result<String, AppError>,
        installer: FakeInstaller,
    ) -> (InstallService, Arc<Counters>) {
        let counters = Arc::clone(&installer.counters);
        let openclaw = FakeOpenClaw {
            detect,
            version_from_entry: version,
        };
        let service = InstallService::new(
            Arc::new(windows_ok()),
            Arc::new(openclaw),
            Arc::new(installer),
        );
        (service, counters)
    }

    // --- idempotency -----------------------------------------------------------

    #[test]
    fn already_installed_returns_existing_version_without_npm_spawns() {
        let (service, counters) = service(found(), Ok("2026.7.1-2".to_string()), ok_installer());
        let result = service.install_openclaw().expect("should succeed");
        assert_eq!(
            result,
            InstallResult::AlreadyInstalled {
                version: "2026.7.1-2".to_string()
            }
        );
        assert_eq!(counters.npm_entry.load(Ordering::SeqCst), 0);
        assert_eq!(counters.npm_version.load(Ordering::SeqCst), 0);
        assert_eq!(counters.install.load(Ordering::SeqCst), 0);
        assert_eq!(counters.entry.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn already_installed_unverifiable_entry_is_verify_failed() {
        let installer = FakeInstaller::new(
            Ok(npm_entry()),
            Ok("11.16.0".to_string()),
            Ok(()),
            Err(AppError::openclaw_install_verify_failed(
                "package root not found",
            )),
        );
        let (service, _) = service(found(), Ok("2026.7.1-2".to_string()), installer);
        let err = service.install_openclaw().expect_err("must fail");
        assert_eq!(err.code, "openclaw-install-verify-failed");
    }

    #[test]
    fn already_installed_version_failure_is_verify_failed() {
        let (service, _) = service(
            found(),
            Err(AppError::process_failed(
                "openclaw (package entry) --version",
                "exit code 2: bad",
            )),
            ok_installer(),
        );
        let err = service.install_openclaw().expect_err("must fail");
        assert_eq!(err.code, "openclaw-install-verify-failed");
        assert!(err.message.contains("exit code 2: bad"));
    }

    // --- node preconditions ------------------------------------------------------

    #[test]
    fn missing_node_is_structured_error_without_npm_spawns() {
        let windows = FakeWindows {
            node: Ok(NodeDetection::NotFound),
            node_exe: Err(AppError::node_not_found()),
        };
        let openclaw = FakeOpenClaw {
            detect: not_found(),
            version_from_entry: Ok("2026.7.1-2".to_string()),
        };
        let installer = ok_installer();
        let counters = Arc::clone(&installer.counters);
        let service =
            InstallService::new(Arc::new(windows), Arc::new(openclaw), Arc::new(installer));
        let err = service.install_openclaw().expect_err("must fail");
        assert_eq!(err.code, "node-not-found");
        assert_eq!(counters.npm_entry.load(Ordering::SeqCst), 0);
        assert_eq!(counters.npm_version.load(Ordering::SeqCst), 0);
        assert_eq!(counters.install.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn unsupported_node_is_structured_error_without_npm_spawns() {
        for version in ["23.0.0", "22.22.2", "24.14.9", "25.8.9"] {
            let windows = FakeWindows {
                node: Ok(NodeDetection::Found {
                    version: version.to_string(),
                }),
                node_exe: Ok(PathBuf::from(NODE)),
            };
            let openclaw = FakeOpenClaw {
                detect: not_found(),
                version_from_entry: Ok("2026.7.1-2".to_string()),
            };
            let installer = ok_installer();
            let counters = Arc::clone(&installer.counters);
            let service =
                InstallService::new(Arc::new(windows), Arc::new(openclaw), Arc::new(installer));
            let err = service
                .install_openclaw()
                .expect_err(&format!("{version} must be rejected"));
            assert_eq!(err.code, "unsupported-node-version", "node {version}");
            assert_eq!(
                counters.npm_entry.load(Ordering::SeqCst),
                0,
                "node {version}"
            );
            assert_eq!(
                counters.npm_version.load(Ordering::SeqCst),
                0,
                "node {version}"
            );
            assert_eq!(counters.install.load(Ordering::SeqCst), 0, "node {version}");
        }
    }

    // --- npm stage ------------------------------------------------------------------

    #[test]
    fn npm_not_found_propagates_without_install_spawn() {
        let installer = FakeInstaller::new(
            Err(AppError::npm_not_found()),
            Ok("11.16.0".to_string()),
            Ok(()),
            Ok(resolved_entry()),
        );
        let (service, counters) = service(not_found(), Ok("2026.7.1-2".to_string()), installer);
        let err = service.install_openclaw().expect_err("must fail");
        assert_eq!(err.code, "npm-not-found");
        assert_eq!(counters.install.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn blocked_npm_range_blocks_install_spawn() {
        for version in ["11.13.0", "11.14.2", "11.15.9"] {
            let installer = FakeInstaller::new(
                Ok(npm_entry()),
                Ok(version.to_string()),
                Ok(()),
                Ok(resolved_entry()),
            );
            let (service, counters) = service(not_found(), Ok("2026.7.1-2".to_string()), installer);
            let err = service.install_openclaw().expect_err("must fail");
            assert_eq!(err.code, "unsupported-npm-version", "npm {version}");
            assert_eq!(counters.install.load(Ordering::SeqCst), 0, "npm {version}");
        }
    }

    // --- install + post-install verification -----------------------------------------

    /// OpenClaw fake whose detect flips NotFound → Found after install.
    struct FlippingOpenClaw {
        installed: Arc<AtomicUsize>,
        version: String,
    }

    impl OpenClawPort for FlippingOpenClaw {
        fn detect_executable(&self) -> ExecutableDetection {
            if self.installed.load(Ordering::SeqCst) == 0 {
                not_found()
            } else {
                found()
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
            Ok(OpenClawVersion {
                raw: self.version.clone(),
            })
        }
        fn gateway_status(&self, _exe: &Path) -> Result<GatewayStatus, AppError> {
            unimplemented!()
        }
        fn update_state(&self, _exe: &Path) -> Result<UpdateState, AppError> {
            unimplemented!()
        }
    }

    struct FlippingInstaller {
        npm_version: String,
        installed: Arc<AtomicUsize>,
        counters: Arc<Counters>,
    }

    impl FlippingInstaller {
        fn new(npm_version: &str, installed: Arc<AtomicUsize>) -> Self {
            Self {
                npm_version: npm_version.to_string(),
                installed,
                counters: Arc::new(Counters::default()),
            }
        }
    }

    impl OpenClawInstallerPort for FlippingInstaller {
        fn resolve_npm_entry(&self, _node: &Path) -> Result<NpmEntry, AppError> {
            self.counters.npm_entry.fetch_add(1, Ordering::SeqCst);
            Ok(npm_entry())
        }
        fn npm_version(&self, _entry: &NpmEntry) -> Result<String, AppError> {
            self.counters.npm_version.fetch_add(1, Ordering::SeqCst);
            Ok(self.npm_version.clone())
        }
        fn install_openclaw_latest(
            &self,
            _entry: &NpmEntry,
            _allow_scripts: bool,
        ) -> Result<(), AppError> {
            self.counters.install.fetch_add(1, Ordering::SeqCst);
            self.installed.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn resolve_openclaw_entry(&self) -> Result<ResolvedOpenClawEntry, AppError> {
            self.counters.entry.fetch_add(1, Ordering::SeqCst);
            Ok(resolved_entry())
        }
    }

    /// Records the allow-scripts decision while delegating to `inner`.
    struct Recorder {
        inner: FlippingInstaller,
        last: Arc<Mutex<Option<bool>>>,
    }

    impl OpenClawInstallerPort for Recorder {
        fn resolve_npm_entry(&self, node: &Path) -> Result<NpmEntry, AppError> {
            self.inner.resolve_npm_entry(node)
        }
        fn npm_version(&self, entry: &NpmEntry) -> Result<String, AppError> {
            self.inner.npm_version(entry)
        }
        fn install_openclaw_latest(
            &self,
            entry: &NpmEntry,
            allow_scripts: bool,
        ) -> Result<(), AppError> {
            *self.last.lock().unwrap() = Some(allow_scripts);
            self.inner.install_openclaw_latest(entry, allow_scripts)
        }
        fn resolve_openclaw_entry(&self) -> Result<ResolvedOpenClawEntry, AppError> {
            self.inner.resolve_openclaw_entry()
        }
    }

    fn flipping_service(npm_version: &str) -> (InstallService, Arc<Counters>) {
        let installed = Arc::new(AtomicUsize::new(0));
        let openclaw = FlippingOpenClaw {
            installed: Arc::clone(&installed),
            version: "2026.7.1-2".to_string(),
        };
        let installer = FlippingInstaller::new(npm_version, installed);
        let counters = Arc::clone(&installer.counters);
        let service = InstallService::new(
            Arc::new(windows_ok()),
            Arc::new(openclaw),
            Arc::new(installer),
        );
        (service, counters)
    }

    #[test]
    fn install_success_returns_installed_version() {
        for version in ["11.12.0", "11.16.0", "12.0.0"] {
            let (service, counters) = flipping_service(version);
            let result = service.install_openclaw().expect("should succeed");
            assert_eq!(
                result,
                InstallResult::Installed {
                    version: "2026.7.1-2".to_string()
                },
                "npm {version}"
            );
            assert_eq!(counters.install.load(Ordering::SeqCst), 1, "npm {version}");
        }
    }

    #[test]
    fn allow_scripts_flag_decision_per_npm_version() {
        for (version, expected) in [
            ("9.0.0", false),
            ("10.9.0", false),
            ("11.12.0", false),
            ("11.16.0", true),
            ("12.0.0", true),
        ] {
            let installed = Arc::new(AtomicUsize::new(0));
            let openclaw = FlippingOpenClaw {
                installed: Arc::clone(&installed),
                version: "2026.7.1-2".to_string(),
            };
            let last = Arc::new(Mutex::new(None));
            let recorder = Recorder {
                inner: FlippingInstaller::new(version, installed),
                last: Arc::clone(&last),
            };
            let service = InstallService::new(
                Arc::new(windows_ok()),
                Arc::new(openclaw),
                Arc::new(recorder),
            );
            service.install_openclaw().expect("should succeed");
            assert_eq!(*last.lock().unwrap(), Some(expected), "npm {version}");
        }
    }

    #[test]
    fn npm_install_failure_maps_to_openclaw_install_failed() {
        let installer = FakeInstaller::new(
            Ok(npm_entry()),
            Ok("11.16.0".to_string()),
            Err(AppError::openclaw_install_failed("exit code 1: boom")),
            Ok(resolved_entry()),
        );
        let (service, _) = service(not_found(), Ok("2026.7.1-2".to_string()), installer);
        let err = service.install_openclaw().expect_err("must fail");
        assert_eq!(err.code, "openclaw-install-failed");
    }

    #[test]
    fn npm_timeout_maps_to_process_timeout() {
        let installer = FakeInstaller::new(
            Ok(npm_entry()),
            Ok("11.16.0".to_string()),
            Err(AppError::process_timeout("npm install -g openclaw@latest")),
            Ok(resolved_entry()),
        );
        let (service, _) = service(not_found(), Ok("2026.7.1-2".to_string()), installer);
        let err = service.install_openclaw().expect_err("must fail");
        assert_eq!(err.code, "process-timeout");
    }

    #[test]
    fn post_install_entry_failure_is_verify_failed() {
        let installer = FakeInstaller::new(
            Ok(npm_entry()),
            Ok("11.16.0".to_string()),
            Ok(()),
            Err(AppError::openclaw_install_verify_failed(
                "package root not found",
            )),
        );
        let (service, _) = service(not_found(), Ok("2026.7.1-2".to_string()), installer);
        let err = service.install_openclaw().expect_err("must fail");
        assert_eq!(err.code, "openclaw-install-verify-failed");
    }

    #[test]
    fn post_install_detect_failure_is_verify_failed() {
        // detect stays NotFound after install → verification must fail.
        let (service, _) = service(not_found(), Ok("2026.7.1-2".to_string()), ok_installer());
        let err = service.install_openclaw().expect_err("must fail");
        assert_eq!(err.code, "openclaw-install-verify-failed");
    }
}
