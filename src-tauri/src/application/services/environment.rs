//! Environment detection: the single use case that composes the Windows and
//! OpenClaw ports into one structured report.

use std::path::PathBuf;
use std::sync::Arc;

use crate::domain::models::windows::{Architecture, NodeDetection, WindowsVersion};
use crate::domain::models::OpenClawStatus;
use crate::domain::ports::openclaw::OpenClawPort;
use crate::domain::ports::process::ProcessPort;
use crate::domain::ports::windows_system::WindowsSystemPort;
use crate::error::AppError;
use crate::infrastructure::openclaw::OpenClawAdapter;
use crate::infrastructure::process::ProcessRunner;
use crate::infrastructure::windows::WindowsSystemAdapter;

/// Structured snapshot of the host environment and OpenClaw state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvironmentReport {
    pub windows_version: WindowsVersion,
    pub architecture: Architecture,
    pub node: NodeDetection,
    pub openclaw: OpenClawStatus,
}

/// Use case layer: composes `WindowsSystemPort` and `OpenClawPort`.
pub struct EnvironmentService {
    windows: Arc<dyn WindowsSystemPort>,
    openclaw: Arc<dyn OpenClawPort>,
}

impl EnvironmentService {
    pub fn new(windows: Arc<dyn WindowsSystemPort>, openclaw: Arc<dyn OpenClawPort>) -> Self {
        Self { windows, openclaw }
    }

    /// Production wiring: real adapters over the single `ProcessRunner`.
    pub fn production() -> Self {
        let process: Arc<dyn ProcessPort> = Arc::new(ProcessRunner);
        let windows = Arc::new(WindowsSystemAdapter::new(Arc::clone(&process)));
        let openclaw = Arc::new(OpenClawAdapter::new(
            Arc::clone(&process),
            default_openclaw_search_dirs(),
        ));
        Self::new(windows, openclaw)
    }

    /// Detect everything.
    ///
    /// Unsupported OS/architecture is a structured `AppError`. "Not found"
    /// conditions are represented as values inside the report.
    pub fn detect_environment(&self) -> Result<EnvironmentReport, AppError> {
        let windows_version = self.windows.os_version()?;
        let architecture = self.windows.architecture()?;
        let node = self.windows.detect_node()?;
        let openclaw = self.detect_openclaw();

        Ok(EnvironmentReport {
            windows_version,
            architecture,
            node,
            openclaw,
        })
    }

    fn detect_openclaw(&self) -> OpenClawStatus {
        let path = match self.openclaw.detect_executable() {
            crate::domain::models::ExecutableDetection::Found { path } => path,
            crate::domain::models::ExecutableDetection::NotFound => {
                return OpenClawStatus::NotFound;
            }
        };

        let version = self.openclaw.version(&path).map(|v| v.raw).ok();
        let gateway = self.openclaw.gateway_status(&path).ok();
        let update = self
            .openclaw
            .update_state(&path)
            .unwrap_or(crate::domain::models::UpdateState::Unknown);

        OpenClawStatus::Detected {
            executable: path,
            version,
            gateway,
            update,
        }
    }
}

/// Default OpenClaw search locations on Windows: npm global dir + PATH dirs.
pub fn default_openclaw_search_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();

    if let Ok(appdata) = std::env::var("APPDATA") {
        let npm_dir = PathBuf::from(appdata).join("npm");
        if npm_dir.is_dir() {
            dirs.push(npm_dir);
        }
    }

    if let Ok(path_var) = std::env::var("PATH") {
        for entry in std::env::split_paths(&path_var) {
            if entry.is_dir() && !dirs.contains(&entry) {
                dirs.push(entry);
            }
        }
    }

    dirs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::openclaw::{ExecutableDetection, GatewayStatus, UpdateState};
    use crate::domain::models::windows::{Architecture, NodeDetection, WindowsVersion};
    use crate::domain::ports::{OpenClawPort, WindowsSystemPort};

    /// Fake Windows port with fixed results.
    struct FakeWindows {
        os_version: Result<WindowsVersion, AppError>,
        architecture: Result<Architecture, AppError>,
        node: Result<NodeDetection, AppError>,
    }

    impl WindowsSystemPort for FakeWindows {
        fn os_version(&self) -> Result<WindowsVersion, AppError> {
            self.os_version.clone()
        }
        fn architecture(&self) -> Result<Architecture, AppError> {
            self.architecture.clone()
        }
        fn detect_node(&self) -> Result<NodeDetection, AppError> {
            self.node.clone()
        }
    }

    /// Fake OpenClaw port with fixed results.
    struct FakeOpenClaw {
        detection: ExecutableDetection,
        version: Result<String, AppError>,
        gateway: Result<GatewayStatus, AppError>,
        update: Result<UpdateState, AppError>,
    }

    impl OpenClawPort for FakeOpenClaw {
        fn detect_executable(&self) -> ExecutableDetection {
            self.detection.clone()
        }
        fn version(
            &self,
            _exe: &std::path::Path,
        ) -> Result<crate::domain::models::OpenClawVersion, AppError> {
            self.version
                .clone()
                .map(|raw| crate::domain::models::OpenClawVersion { raw })
        }
        fn gateway_status(&self, _exe: &std::path::Path) -> Result<GatewayStatus, AppError> {
            self.gateway.clone()
        }
        fn update_state(&self, _exe: &std::path::Path) -> Result<UpdateState, AppError> {
            self.update.clone()
        }
    }

    fn windows_ok() -> FakeWindows {
        FakeWindows {
            os_version: Ok(WindowsVersion {
                major_version: 11,
                build: 26100,
                ubr: 1,
                product_name: Some("Windows 11 Pro".to_string()),
            }),
            architecture: Ok(Architecture::X64),
            node: Ok(NodeDetection::Found {
                version: "22.14.0".into(),
            }),
        }
    }

    #[test]
    fn report_aggregates_detected_environment() {
        let openclaw = FakeOpenClaw {
            detection: ExecutableDetection::Found {
                path: PathBuf::from("C:\\fake\\openclaw.exe"),
            },
            version: Ok("2026.7.1-2".to_string()),
            gateway: Ok(GatewayStatus {
                state: "running".into(),
                version: Some("2026.7.1-2".into()),
                port: Some(18789),
            }),
            update: Ok(UpdateState::Updated),
        };
        let service = EnvironmentService::new(Arc::new(windows_ok()), Arc::new(openclaw));
        let report = service.detect_environment().unwrap();

        assert_eq!(report.windows_version.build, 26100);
        assert_eq!(report.architecture, Architecture::X64);
        assert_eq!(
            report.node,
            NodeDetection::Found {
                version: "22.14.0".into()
            }
        );
        match report.openclaw {
            OpenClawStatus::Detected {
                version: Some(version),
                update: UpdateState::Updated,
                ..
            } => assert_eq!(version, "2026.7.1-2"),
            other => panic!("expected detected OpenClaw, got {other:?}"),
        }
    }

    #[test]
    fn report_is_structured_when_openclaw_missing() {
        let openclaw = FakeOpenClaw {
            detection: ExecutableDetection::NotFound,
            version: Err(AppError::openclaw_not_found()),
            gateway: Err(AppError::openclaw_not_found()),
            update: Err(AppError::openclaw_not_found()),
        };
        let service = EnvironmentService::new(Arc::new(windows_ok()), Arc::new(openclaw));
        let report = service.detect_environment().unwrap();
        assert_eq!(report.openclaw, OpenClawStatus::NotFound);
    }

    #[test]
    fn unsupported_architecture_is_structured_error() {
        let windows = FakeWindows {
            os_version: Ok(WindowsVersion {
                major_version: 11,
                build: 26100,
                ubr: 0,
                product_name: None,
            }),
            architecture: Err(AppError::unsupported_architecture("arm64")),
            node: Ok(NodeDetection::NotFound),
        };
        let openclaw = FakeOpenClaw {
            detection: ExecutableDetection::NotFound,
            version: Ok(String::new()),
            gateway: Err(AppError::openclaw_not_found()),
            update: Ok(UpdateState::Unknown),
        };
        let service = EnvironmentService::new(Arc::new(windows), Arc::new(openclaw));
        let err = service.detect_environment().unwrap_err();
        assert_eq!(err.code, "unsupported-architecture");
    }
}
