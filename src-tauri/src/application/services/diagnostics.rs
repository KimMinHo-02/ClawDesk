//! Diagnostics use case (Phase 8): read-only profile/update/gateway/logs.
//!
//! Orchestration: validate the log limit (S2, 0 CLI on violation,
//! fail-closed) → detect the OpenClaw executable → delegate to the Phase 1
//! `OpenClawPort` (gateway status — reuse) or the Phase 8
//! `OpenClawDiagnosticsPort` (agents/update detail/logs).

use std::path::PathBuf;
use std::sync::Arc;

use super::environment::default_openclaw_search_dirs;
use crate::domain::models::diagnostics::{AgentRow, LogsResult, UpdateStatusDetail};
use crate::domain::models::openclaw::GatewayStatus;
use crate::domain::ports::openclaw::OpenClawPort;
use crate::domain::ports::openclaw_diagnostics::OpenClawDiagnosticsPort;
use crate::error::AppError;
use crate::infrastructure::openclaw::{OpenClawAdapter, OpenClawDiagnosticsAdapter};
use crate::infrastructure::process::ProcessRunner;

/// One-shot log tail bounds enforced before any CLI call (contract §5).
pub const LOGS_LIMIT_MIN: u32 = 1;
pub const LOGS_LIMIT_MAX: u32 = 1000;
/// The default tail length (the CLI default; the UI preselects it).
pub const LOGS_LIMIT_DEFAULT: u32 = 200;

/// Use case layer: composes the OpenClaw executable port with the Phase 8
/// diagnostics port.
pub struct DiagnosticsService {
    openclaw: Arc<dyn OpenClawPort>,
    diagnostics: Arc<dyn OpenClawDiagnosticsPort>,
}

impl DiagnosticsService {
    pub fn new(
        openclaw: Arc<dyn OpenClawPort>,
        diagnostics: Arc<dyn OpenClawDiagnosticsPort>,
    ) -> Self {
        Self {
            openclaw,
            diagnostics,
        }
    }

    /// Production wiring: real adapters over the single `ProcessRunner`.
    pub fn production() -> Self {
        let process: Arc<dyn crate::domain::ports::process::ProcessPort> = Arc::new(ProcessRunner);
        let openclaw = Arc::new(OpenClawAdapter::new(
            Arc::clone(&process),
            default_openclaw_search_dirs(),
        ));
        let diagnostics = Arc::new(OpenClawDiagnosticsAdapter::new(process));
        Self::new(openclaw, diagnostics)
    }

    fn executable(&self) -> Result<PathBuf, AppError> {
        match self.openclaw.detect_executable() {
            crate::domain::models::ExecutableDetection::Found { path } => Ok(path),
            crate::domain::models::ExecutableDetection::NotFound => {
                Err(AppError::openclaw_not_found())
            }
        }
    }

    /// `openclaw gateway status --json` (Phase 1 port reuse — 0 re-implementation).
    pub fn gateway_status(&self) -> Result<GatewayStatus, AppError> {
        let exe = self.executable()?;
        self.openclaw.gateway_status(&exe)
    }

    /// `openclaw update status --json` with version detail (fail-soft).
    pub fn update_status(&self) -> Result<UpdateStatusDetail, AppError> {
        let exe = self.executable()?;
        self.diagnostics.update_detail(&exe)
    }

    /// `openclaw agents list --json` (read-only display).
    pub fn agents(&self) -> Result<Vec<AgentRow>, AppError> {
        let exe = self.executable()?;
        self.diagnostics.list_agents(&exe)
    }

    /// `openclaw logs --limit <n> --json` one-shot tail.
    ///
    /// Fail-closed order: limit validation (violation → `logs-limit-invalid`,
    /// 0 executable detection, 0 CLI) → executable detection → tail.
    pub fn logs(&self, limit: u32) -> Result<LogsResult, AppError> {
        if !(LOGS_LIMIT_MIN..=LOGS_LIMIT_MAX).contains(&limit) {
            return Err(AppError::invalid_input(
                "logs-limit-invalid",
                "log limit",
                &limit.to_string(),
            ));
        }
        let exe = self.executable()?;
        self.diagnostics.tail_logs(&exe, limit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::openclaw::{OpenClawVersion, UpdateState};
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    const EXE: &str = "C:\\fake\\openclaw.exe";

    struct FixedOpenClaw {
        gateway: Mutex<Option<GatewayStatus>>,
    }

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
            Ok(self
                .gateway
                .lock()
                .unwrap()
                .clone()
                .unwrap_or(GatewayStatus {
                    state: "running".into(),
                    version: Some("2026.7.1-2".into()),
                    port: Some(18789),
                }))
        }
        fn update_state(&self, _exe: &Path) -> Result<UpdateState, AppError> {
            unimplemented!()
        }
    }

    struct FakeDiagnostics {
        calls: Arc<Mutex<Vec<String>>>,
        logs_error: Mutex<Option<AppError>>,
    }

    impl OpenClawDiagnosticsPort for FakeDiagnostics {
        fn list_agents(&self, _exe: &Path) -> Result<Vec<AgentRow>, AppError> {
            self.calls.lock().unwrap().push("list_agents".into());
            Ok(vec![AgentRow {
                id: "main".into(),
                default: true,
                name: None,
                emoji: None,
                workspace: None,
                bindings: None,
            }])
        }
        fn update_detail(&self, _exe: &Path) -> Result<UpdateStatusDetail, AppError> {
            self.calls.lock().unwrap().push("update_detail".into());
            Ok(UpdateStatusDetail {
                state: UpdateState::Updated,
                current: Some("2026.7.1-2".into()),
                latest: Some("2026.7.1-2".into()),
            })
        }
        fn tail_logs(&self, _exe: &Path, limit: u32) -> Result<LogsResult, AppError> {
            self.calls
                .lock()
                .unwrap()
                .push(format!("tail_logs:{limit}"));
            if let Some(failure) = self.logs_error.lock().unwrap().clone() {
                return Err(failure);
            }
            Ok(LogsResult {
                lines: vec![],
                source: None,
                truncated: false,
            })
        }
    }

    fn service() -> (DiagnosticsService, Arc<Mutex<Vec<String>>>) {
        let calls: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let diagnostics: Arc<dyn OpenClawDiagnosticsPort> = Arc::new(FakeDiagnostics {
            calls: Arc::clone(&calls),
            logs_error: Mutex::new(None),
        });
        let openclaw: Arc<dyn OpenClawPort> = Arc::new(FixedOpenClaw {
            gateway: Mutex::new(None),
        });
        let service = DiagnosticsService::new(openclaw, diagnostics);
        (service, calls)
    }

    #[test]
    fn gateway_status_delegates_to_phase1_port() {
        let (service, calls) = service();
        let status = service.gateway_status().expect("gateway");
        assert_eq!(status.state, "running");
        assert_eq!(status.port, Some(18789));
        // The Phase 1 port handles it — the diagnostics port is untouched.
        assert!(calls.lock().unwrap().is_empty());
    }

    #[test]
    fn update_status_delegates_to_diagnostics_port() {
        let (service, calls) = service();
        let detail = service.update_status().expect("update");
        assert_eq!(detail.state, UpdateState::Updated);
        assert_eq!(detail.current.as_deref(), Some("2026.7.1-2"));
        assert_eq!(*calls.lock().unwrap(), vec!["update_detail".to_string()]);
    }

    #[test]
    fn agents_delegates_to_diagnostics_port() {
        let (service, calls) = service();
        let rows = service.agents().expect("agents");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "main");
        assert!(rows[0].default);
        assert_eq!(*calls.lock().unwrap(), vec!["list_agents".to_string()]);
    }

    #[test]
    fn logs_passes_valid_limits_through() {
        for limit in [LOGS_LIMIT_MIN, LOGS_LIMIT_MAX, LOGS_LIMIT_DEFAULT] {
            let (service, calls) = service();
            service.logs(limit).expect("valid limit");
            assert_eq!(
                *calls.lock().unwrap(),
                vec![format!("tail_logs:{limit}")],
                "limit {limit} must reach the port"
            );
        }
    }

    #[test]
    fn logs_rejects_out_of_range_limits_with_zero_cli() {
        for limit in [0u32, 1001, u32::MAX] {
            let (service, calls) = service();
            let err = service.logs(limit).expect_err("must be rejected");
            assert_eq!(err.code, "logs-limit-invalid", "{limit}");
            assert!(calls.lock().unwrap().is_empty(), "0 CLI calls");
        }
    }

    #[test]
    fn logs_port_failure_passes_through() {
        let calls: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let diagnostics: Arc<dyn OpenClawDiagnosticsPort> = Arc::new(FakeDiagnostics {
            calls: Arc::clone(&calls),
            logs_error: Mutex::new(Some(AppError::openclaw_logs_read_failed("exit 1"))),
        });
        let openclaw: Arc<dyn OpenClawPort> = Arc::new(FixedOpenClaw {
            gateway: Mutex::new(None),
        });
        let service = DiagnosticsService::new(openclaw, diagnostics);
        let err = service.logs(50).expect_err("port failure");
        assert_eq!(err.code, "openclaw-logs-read-failed");
        assert_eq!(*calls.lock().unwrap(), vec!["tail_logs:50".to_string()]);
    }
}
