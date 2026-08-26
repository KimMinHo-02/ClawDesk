//! Node update use case (Phase 8.1): one-shot Node.js update for an
//! unsupported detected version.
//!
//! Fail-closed order (0 OS mutation unless every precondition holds):
//!
//! 1. `detect_node()` → `NotFound` ⇒ `node-not-found` (Phase 2 contract:
//!    guidance only — 8.1 does not auto-install a missing Node)
//! 2. `Found` + supported ⇒ `node-update-not-needed` (0 winget)
//! 3. `Found` + unsupported ⇒ `NodeUpdatePort::update_node()`
//! 4. Result validation: only `Found` + supported succeeds; anything else
//!    (still unsupported, `NotFound`) ⇒ `node-update-failed`

use std::sync::Arc;

use crate::domain::models::windows::NodeDetection;
use crate::domain::ports::node_update::NodeUpdatePort;
use crate::domain::ports::process::ProcessPort;
use crate::domain::ports::windows_system::WindowsSystemPort;
use crate::error::AppError;
use crate::infrastructure::openclaw::node_version_supported;
use crate::infrastructure::process::ProcessRunner;
use crate::infrastructure::windows::{NodeUpdateAdapter, WindowsSystemAdapter};

/// Use case layer: composes the Windows system port (detection) with the
/// Node update port (winget + re-detection).
pub struct NodeUpdateService {
    windows: Arc<dyn WindowsSystemPort>,
    node_update: Arc<dyn NodeUpdatePort>,
}

impl NodeUpdateService {
    pub fn new(windows: Arc<dyn WindowsSystemPort>, node_update: Arc<dyn NodeUpdatePort>) -> Self {
        Self {
            windows,
            node_update,
        }
    }

    /// Production wiring: real adapters over the single `ProcessRunner`.
    pub fn production() -> Self {
        let process: Arc<dyn ProcessPort> = Arc::new(ProcessRunner);
        let windows = Arc::new(WindowsSystemAdapter::new(Arc::clone(&process)));
        let node_update = Arc::new(NodeUpdateAdapter::new(process));
        Self::new(windows, node_update)
    }

    /// One-shot Node.js update, guarded by the Phase 2 support policy.
    pub fn update_node(&self) -> Result<NodeDetection, AppError> {
        match self.windows.detect_node()? {
            NodeDetection::NotFound => Err(AppError::node_not_found()),
            NodeDetection::Found { version } if node_version_supported(&version) => {
                Err(AppError::node_update_not_needed(version))
            }
            NodeDetection::Found { version } => {
                let detected = self.node_update.update_node()?;
                match &detected {
                    NodeDetection::Found {
                        version: new_version,
                    } if node_version_supported(new_version) => Ok(detected),
                    NodeDetection::Found {
                        version: new_version,
                    } => Err(AppError::node_update_failed(format!(
                        "Node.js is still unsupported after the update: {new_version} (was {version})"
                    ))),
                    NodeDetection::NotFound => Err(AppError::node_update_failed(
                        "Node.js was not detectable after the update",
                    )),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::windows::{Architecture, WindowsVersion};
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    struct FixedWindows {
        node: NodeDetection,
    }

    impl WindowsSystemPort for FixedWindows {
        fn os_version(&self) -> Result<WindowsVersion, AppError> {
            unimplemented!()
        }
        fn architecture(&self) -> Result<Architecture, AppError> {
            unimplemented!()
        }
        fn detect_node(&self) -> Result<NodeDetection, AppError> {
            Ok(self.node.clone())
        }
        fn node_executable(&self) -> Result<PathBuf, AppError> {
            unimplemented!()
        }
    }

    struct FakeNodeUpdate {
        calls: Arc<Mutex<u32>>,
        result: Result<NodeDetection, AppError>,
    }

    impl NodeUpdatePort for FakeNodeUpdate {
        fn update_node(&self) -> Result<NodeDetection, AppError> {
            *self.calls.lock().unwrap() += 1;
            self.result.clone()
        }
    }

    fn service(
        node: NodeDetection,
        result: Result<NodeDetection, AppError>,
    ) -> (NodeUpdateService, Arc<Mutex<u32>>) {
        let calls: Arc<Mutex<u32>> = Arc::new(Mutex::new(0));
        let node_update: Arc<dyn NodeUpdatePort> = Arc::new(FakeNodeUpdate {
            calls: Arc::clone(&calls),
            result,
        });
        let windows: Arc<dyn WindowsSystemPort> = Arc::new(FixedWindows { node });
        (NodeUpdateService::new(windows, node_update), calls)
    }

    #[test]
    fn missing_node_is_node_not_found_with_zero_update() {
        let (service, calls) = service(
            NodeDetection::NotFound,
            Ok(NodeDetection::Found {
                version: "24.15.0".into(),
            }),
        );
        let err = service.update_node().expect_err("missing node must fail");
        assert_eq!(err.code, "node-not-found");
        assert_eq!(*calls.lock().unwrap(), 0, "0 winget attempts");
    }

    #[test]
    fn supported_node_is_not_needed_with_zero_update() {
        let (service, calls) = service(
            NodeDetection::Found {
                version: "24.15.0".into(),
            },
            Ok(NodeDetection::Found {
                version: "26.0.0".into(),
            }),
        );
        let err = service
            .update_node()
            .expect_err("already supported must fail");
        assert_eq!(err.code, "node-update-not-needed");
        assert!(err.message.contains("24.15.0"));
        assert_eq!(*calls.lock().unwrap(), 0, "0 winget attempts");
    }

    #[test]
    fn unsupported_node_updates_to_supported() {
        let (service, calls) = service(
            NodeDetection::Found {
                version: "18.19.0".into(),
            },
            Ok(NodeDetection::Found {
                version: "24.15.0".into(),
            }),
        );
        let detected = service.update_node().expect("update must succeed");
        assert_eq!(
            detected,
            NodeDetection::Found {
                version: "24.15.0".into()
            }
        );
        assert_eq!(*calls.lock().unwrap(), 1);
    }

    #[test]
    fn update_still_unsupported_is_failed() {
        let (service, _) = service(
            NodeDetection::Found {
                version: "18.19.0".into(),
            },
            Ok(NodeDetection::Found {
                version: "20.11.0".into(),
            }),
        );
        let err = service
            .update_node()
            .expect_err("still unsupported must fail");
        assert_eq!(err.code, "node-update-failed");
        assert!(err.message.contains("20.11.0"));
        assert!(err.message.contains("18.19.0"));
    }

    #[test]
    fn update_to_not_found_is_failed() {
        let (service, _) = service(
            NodeDetection::Found {
                version: "18.19.0".into(),
            },
            Ok(NodeDetection::NotFound),
        );
        let err = service.update_node().expect_err("not detectable must fail");
        assert_eq!(err.code, "node-update-failed");
    }

    #[test]
    fn port_error_passes_through() {
        let (service, _) = service(
            NodeDetection::Found {
                version: "18.19.0".into(),
            },
            Err(AppError::winget_not_found()),
        );
        let err = service
            .update_node()
            .expect_err("port error must pass through");
        assert_eq!(err.code, "winget-not-found");
    }
}
