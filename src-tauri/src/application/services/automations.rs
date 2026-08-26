//! Automations use case (Phase 7).
//!
//! Orchestration over the openclaw-detect + automations ports:
//! - every job id / name / schedule / payload input is validated first
//!   (S2, fail-closed — zero CLI calls on any validation failure);
//! - the session pairing is fixed by the payload kind (reminder →
//!   `main`/`--system-event`, task → `isolated`/`--message`); the IPC wire
//!   carries no session field;
//! - `create` returns the new job id extracted from the CLI result
//!   (fail-soft — a missing id is a structured error, never a silent
//!   success).
//!
//! No optimistic updates: the CLI is the source of truth; the UI re-queries
//! after every finished mutation (success OR failure).

use std::path::PathBuf;
use std::sync::Arc;

use super::environment::default_openclaw_search_dirs;
use crate::domain::models::automations::{
    validate_automation_id, validate_automation_name, validate_automation_payload,
    validate_schedule, AutomationJob, AutomationJobRow,
};
use crate::domain::ports::openclaw::OpenClawPort;
use crate::domain::ports::openclaw_automations::OpenClawAutomationsPort;
use crate::error::AppError;
use crate::infrastructure::openclaw::{OpenClawAdapter, OpenClawAutomationsAdapter};
use crate::infrastructure::process::ProcessRunner;

/// Use case layer: composes the OpenClaw executable detection and the
/// automations port.
pub struct AutomationService {
    openclaw: Arc<dyn OpenClawPort>,
    automations: Arc<dyn OpenClawAutomationsPort>,
}

impl AutomationService {
    pub fn new(
        openclaw: Arc<dyn OpenClawPort>,
        automations: Arc<dyn OpenClawAutomationsPort>,
    ) -> Self {
        Self {
            openclaw,
            automations,
        }
    }

    /// Production wiring: real adapters over the single `ProcessRunner`.
    pub fn production() -> Self {
        let process: Arc<dyn crate::domain::ports::process::ProcessPort> = Arc::new(ProcessRunner);
        let openclaw = Arc::new(OpenClawAdapter::new(
            Arc::clone(&process),
            default_openclaw_search_dirs(),
        ));
        let automations = Arc::new(OpenClawAutomationsAdapter::new(process));
        Self::new(openclaw, automations)
    }

    fn executable(&self) -> Result<PathBuf, AppError> {
        match self.openclaw.detect_executable() {
            crate::domain::models::ExecutableDetection::Found { path } => Ok(path),
            crate::domain::models::ExecutableDetection::NotFound => {
                Err(AppError::openclaw_not_found())
            }
        }
    }

    /// Pre-validates all user input (fail-closed, 0 CLI calls on failure).
    /// The normalized (trimmed, non-blank) schedule tz is returned.
    fn validate_create_input<'a>(
        name: &'a str,
        schedule_kind: &'a str,
        schedule_value: &'a str,
        schedule_tz: Option<&'a str>,
        payload_kind: &'a str,
        text: &'a str,
        wake: Option<&'a str>,
    ) -> Result<Option<&'a str>, AppError> {
        validate_automation_name(name)?;
        validate_schedule(schedule_kind, schedule_value, schedule_tz)?;
        validate_automation_payload(payload_kind, text, wake)?;
        Ok(schedule_tz.map(str::trim).filter(|t| !t.is_empty()))
    }

    /// `get-automations`: all job rows including disabled (read-only).
    pub fn list_automations(&self) -> Result<Vec<AutomationJobRow>, AppError> {
        let exe = self.executable()?;
        self.automations.list_automations(&exe)
    }

    /// `get-automation`: id validated (fail-closed) → job detail.
    pub fn get_automation(&self, job_id: &str) -> Result<AutomationJob, AppError> {
        validate_automation_id(job_id)?;
        let exe = self.executable()?;
        self.automations.get_automation(&exe, job_id)
    }

    /// `create-automation`: full pre-validation → `automations add` →
    /// the new job id.
    #[allow(clippy::too_many_arguments)] // the Phase 7 contract fixes this 7-field wire
    pub fn create_automation(
        &self,
        name: &str,
        schedule_kind: &str,
        schedule_value: &str,
        schedule_tz: Option<&str>,
        payload_kind: &str,
        text: &str,
        wake: Option<&str>,
    ) -> Result<String, AppError> {
        let tz = Self::validate_create_input(
            name,
            schedule_kind,
            schedule_value,
            schedule_tz,
            payload_kind,
            text,
            wake,
        )?;
        let exe = self.executable()?;
        self.automations.add_automation(
            &exe,
            name.trim(),
            schedule_kind,
            schedule_value.trim(),
            tz,
            payload_kind,
            text.trim(),
            wake,
        )
    }

    /// `update-automation`: id + full field pre-validation → `automations
    /// edit`. The payload kind is the input's kind (kind change = delete +
    /// recreate, blocked by the UI).
    #[allow(clippy::too_many_arguments)] // the Phase 7 contract fixes this 7-field wire
    pub fn update_automation(
        &self,
        job_id: &str,
        name: &str,
        schedule_kind: &str,
        schedule_value: &str,
        schedule_tz: Option<&str>,
        payload_kind: &str,
        text: &str,
        wake: Option<&str>,
    ) -> Result<(), AppError> {
        validate_automation_id(job_id)?;
        let tz = Self::validate_create_input(
            name,
            schedule_kind,
            schedule_value,
            schedule_tz,
            payload_kind,
            text,
            wake,
        )?;
        let exe = self.executable()?;
        self.automations.edit_automation(
            &exe,
            job_id,
            name.trim(),
            schedule_kind,
            schedule_value.trim(),
            tz,
            payload_kind,
            text.trim(),
            wake,
        )
    }

    /// `set-automation-enabled`: id validated (fail-closed) →
    /// `automations enable|disable`.
    pub fn set_automation_enabled(&self, job_id: &str, enabled: bool) -> Result<(), AppError> {
        validate_automation_id(job_id)?;
        let exe = self.executable()?;
        self.automations
            .set_automation_enabled(&exe, job_id, enabled)
    }

    /// `delete-automation`: id validated (fail-closed) → `automations remove`.
    pub fn remove_automation(&self, job_id: &str) -> Result<(), AppError> {
        validate_automation_id(job_id)?;
        let exe = self.executable()?;
        self.automations.remove_automation(&exe, job_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::openclaw::{GatewayStatus, OpenClawVersion, UpdateState};
    use crate::domain::ports::openclaw_automations::OpenClawAutomationsPort;
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

    struct FakeAutomations {
        log: Arc<Mutex<Vec<String>>>,
        add_result: Mutex<Option<AppError>>,
    }

    impl FakeAutomations {
        fn new(log: Arc<Mutex<Vec<String>>>) -> Arc<Self> {
            Arc::new(Self {
                log,
                add_result: Mutex::new(None),
            })
        }
    }

    impl OpenClawAutomationsPort for FakeAutomations {
        fn list_automations(&self, _exe: &Path) -> Result<Vec<AutomationJobRow>, AppError> {
            self.log.lock().unwrap().push("list".to_string());
            Ok(Vec::new())
        }
        fn get_automation(&self, _exe: &Path, job_id: &str) -> Result<AutomationJob, AppError> {
            self.log.lock().unwrap().push(format!("get:{job_id}"));
            Ok(AutomationJob {
                id: job_id.to_string(),
                name: None,
                enabled: None,
                status: None,
                schedule: None,
                payload: None,
            })
        }
        fn add_automation(
            &self,
            _exe: &Path,
            name: &str,
            schedule_kind: &str,
            schedule_value: &str,
            schedule_tz: Option<&str>,
            payload_kind: &str,
            text: &str,
            wake: Option<&str>,
        ) -> Result<String, AppError> {
            if let Some(failure) = self.add_result.lock().unwrap().clone() {
                return Err(failure);
            }
            self.log.lock().unwrap().push(format!(
                "add:{name}|{schedule_kind}|{schedule_value}|tz={schedule_tz:?}|{payload_kind}|{text}|wake={wake:?}"
            ));
            Ok("job-1".to_string())
        }
        fn edit_automation(
            &self,
            _exe: &Path,
            job_id: &str,
            name: &str,
            schedule_kind: &str,
            schedule_value: &str,
            _schedule_tz: Option<&str>,
            payload_kind: &str,
            text: &str,
            _wake: Option<&str>,
        ) -> Result<(), AppError> {
            self.log.lock().unwrap().push(format!(
                "edit:{job_id}:{name}|{schedule_kind}|{schedule_value}|{payload_kind}|{text}"
            ));
            Ok(())
        }
        fn set_automation_enabled(
            &self,
            _exe: &Path,
            job_id: &str,
            enabled: bool,
        ) -> Result<(), AppError> {
            self.log
                .lock()
                .unwrap()
                .push(format!("toggle:{job_id}:{enabled}"));
            Ok(())
        }
        fn remove_automation(&self, _exe: &Path, job_id: &str) -> Result<(), AppError> {
            self.log.lock().unwrap().push(format!("remove:{job_id}"));
            Ok(())
        }
    }

    fn service() -> (
        AutomationService,
        Arc<Mutex<Vec<String>>>,
        Arc<FakeAutomations>,
    ) {
        let log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let automations = FakeAutomations::new(Arc::clone(&log));
        let service = AutomationService::new(Arc::new(FixedOpenClaw), automations.clone());
        (service, log, automations)
    }

    #[test]
    fn list_delegates_to_port() {
        let (service, log, _) = service();
        assert!(service.list_automations().expect("list").is_empty());
        assert_eq!(log.lock().unwrap().clone(), vec!["list".to_string()]);
    }

    #[test]
    fn get_validates_id_before_any_cli_call() {
        let (service, log, _) = service();
        for bad in ["", "bad id", &"x".repeat(65)] {
            assert_eq!(
                service.get_automation(bad).unwrap_err().code,
                "automation-id-invalid",
                "{bad:?}"
            );
        }
        assert!(log.lock().unwrap().is_empty(), "0 CLI calls");
        let job = service.get_automation("job-1").expect("get");
        assert_eq!(job.id, "job-1");
        assert_eq!(log.lock().unwrap().clone(), vec!["get:job-1".to_string()]);
    }

    #[test]
    fn create_validates_fail_closed_with_zero_cli() {
        let (service, log, _) = service();
        #[allow(clippy::type_complexity)] // positional case table; fields destructured below
        let cases: Vec<(
            &str,
            &str,
            &str,
            Option<&str>,
            &str,
            &str,
            Option<&str>,
            &str,
        )> = vec![
            (
                "",
                "at",
                "2027-02-01T16:00:00Z",
                None,
                "reminder",
                "text",
                None,
                "name",
            ),
            (
                "   ",
                "at",
                "2027-02-01T16:00:00Z",
                None,
                "reminder",
                "text",
                None,
                "name",
            ),
            (
                "a\u{01}b",
                "at",
                "2027-02-01T16:00:00Z",
                None,
                "reminder",
                "text",
                None,
                "name",
            ),
            (
                "name",
                "at",
                "2027-02-01T16:00:00",
                None,
                "reminder",
                "text",
                None,
                "schedule",
            ),
            (
                "name",
                "at",
                "2027-02-01T16:00:00Z",
                Some("Asia/Seoul"),
                "reminder",
                "text",
                None,
                "schedule",
            ),
            (
                "name", "every", "0m", None, "reminder", "text", None, "schedule",
            ),
            (
                "name", "cron", "0 9 * *", None, "reminder", "text", None, "schedule",
            ),
            (
                "name", "stream", "x", None, "reminder", "text", None, "schedule",
            ),
            (
                "name",
                "at",
                "2027-02-01T16:00:00Z",
                None,
                "reminder",
                "",
                None,
                "payload",
            ),
            (
                "name",
                "at",
                "2027-02-01T16:00:00Z",
                None,
                "command",
                "ls",
                None,
                "payload",
            ),
            (
                "name",
                "at",
                "2027-02-01T16:00:00Z",
                None,
                "task",
                "text",
                Some("now"),
                "payload",
            ),
            (
                "name",
                "at",
                "2027-02-01T16:00:00Z",
                None,
                "reminder",
                "text",
                Some("later"),
                "payload",
            ),
        ];
        for (name, kind, value, tz, pkind, text, wake, label) in &cases {
            let expected = match *label {
                "name" => "automation-name-invalid",
                "schedule" => "automation-schedule-invalid",
                _ => "automation-payload-invalid",
            };
            let err = service
                .create_automation(name, kind, value, *tz, pkind, text, *wake)
                .unwrap_err();
            assert_eq!(err.code, expected, "{label}");
        }
        assert!(
            log.lock().unwrap().is_empty(),
            "0 CLI calls on validation failure"
        );
    }

    #[test]
    fn create_delegates_trimmed_values_and_returns_job_id() {
        let (service, log, _) = service();
        let job_id = service
            .create_automation(
                "  name  ",
                "cron",
                " 0 9 * * 1 ",
                Some("  Asia/Seoul "),
                "reminder",
                "  text  ",
                Some("now"),
            )
            .expect("create");
        assert_eq!(job_id, "job-1");
        assert_eq!(
            log.lock().unwrap().clone(),
            vec![
                "add:name|cron|0 9 * * 1|tz=Some(\"Asia/Seoul\")|reminder|text|wake=Some(\"now\")"
                    .to_string()
            ]
        );
    }

    #[test]
    fn create_add_failure_maps_to_stable_code() {
        let (service, _log, automations) = service();
        *automations.add_result.lock().unwrap() = Some(AppError::openclaw_automations_failed(
            "no job id in the add result",
        ));
        let err = service
            .create_automation(
                "name",
                "at",
                "2027-02-01T16:00:00Z",
                None,
                "reminder",
                "text",
                None,
            )
            .expect_err("add failure");
        assert_eq!(err.code, "openclaw-automations-failed");
    }

    #[test]
    fn update_validates_id_then_fields_fail_closed() {
        let (service, log, _) = service();
        let err = service
            .update_automation(
                "bad id",
                "name",
                "at",
                "2027-02-01T16:00:00Z",
                None,
                "reminder",
                "text",
                None,
            )
            .expect_err("bad id");
        assert_eq!(err.code, "automation-id-invalid");
        let err = service
            .update_automation(
                "job-1",
                "name",
                "cron",
                "a 9 * * *",
                None,
                "reminder",
                "text",
                None,
            )
            .expect_err("bad schedule");
        assert_eq!(err.code, "automation-schedule-invalid");
        assert!(log.lock().unwrap().is_empty(), "0 CLI calls");
        service
            .update_automation(
                "job-1", "name", "every", "10m", None, "reminder", "text", None,
            )
            .expect("update");
        assert_eq!(
            log.lock().unwrap().clone(),
            vec!["edit:job-1:name|every|10m|reminder|text".to_string()]
        );
    }

    #[test]
    fn toggle_and_remove_validate_id_then_delegate() {
        let (service, log, _) = service();
        for bad in ["", "bad id"] {
            assert_eq!(
                service.set_automation_enabled(bad, true).unwrap_err().code,
                "automation-id-invalid",
                "{bad:?}"
            );
            assert_eq!(
                service.remove_automation(bad).unwrap_err().code,
                "automation-id-invalid",
                "{bad:?}"
            );
        }
        service
            .set_automation_enabled("job-1", false)
            .expect("disable");
        service.remove_automation("job-1").expect("remove");
        assert_eq!(
            log.lock().unwrap().clone(),
            vec!["toggle:job-1:false".to_string(), "remove:job-1".to_string()]
        );
    }
}
