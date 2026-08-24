//! Skills use case (Phase 4).
//!
//! Orchestration: validate the skill name (S2, 0 process runs on failure) →
//! detect the OpenClaw executable → existence check via `skills list` (no
//! write for unknown skills) → toggle with the Phase 3 config port's
//! built-in dry-run → commit on the `skills.entries.<name>.enabled` leaf.
//!
//! The `enabled` leaf is the only skills config surface Phase 4 touches
//! (contract §6: `env`/`apiKey`/`config` contact is 0).

use std::path::PathBuf;
use std::sync::Arc;

use super::environment::default_openclaw_search_dirs;
use crate::domain::models::skills::{validate_skill_name, SkillRow};
use crate::domain::ports::openclaw::OpenClawPort;
use crate::domain::ports::openclaw_config::{OpenClawConfigPort, WriteMode};
use crate::domain::ports::openclaw_skills::OpenClawSkillsPort;
use crate::error::AppError;
use crate::infrastructure::openclaw::{
    OpenClawAdapter, OpenClawConfigAdapter, OpenClawSkillsAdapter,
};
use crate::infrastructure::process::ProcessRunner;

/// Use case layer: composes the OpenClaw executable, skills, and config
/// ports.
pub struct SkillsService {
    openclaw: Arc<dyn OpenClawPort>,
    skills: Arc<dyn OpenClawSkillsPort>,
    config: Arc<dyn OpenClawConfigPort>,
}

impl SkillsService {
    pub fn new(
        openclaw: Arc<dyn OpenClawPort>,
        skills: Arc<dyn OpenClawSkillsPort>,
        config: Arc<dyn OpenClawConfigPort>,
    ) -> Self {
        Self {
            openclaw,
            skills,
            config,
        }
    }

    /// Production wiring: real adapters over the single `ProcessRunner`.
    pub fn production() -> Self {
        let process: Arc<dyn crate::domain::ports::process::ProcessPort> = Arc::new(ProcessRunner);
        let openclaw = Arc::new(OpenClawAdapter::new(
            Arc::clone(&process),
            default_openclaw_search_dirs(),
        ));
        let skills = Arc::new(OpenClawSkillsAdapter::new(Arc::clone(&process)));
        let config = Arc::new(OpenClawConfigAdapter::new(process));
        Self::new(openclaw, skills, config)
    }

    fn executable(&self) -> Result<PathBuf, AppError> {
        match self.openclaw.detect_executable() {
            crate::domain::models::ExecutableDetection::Found { path } => Ok(path),
            crate::domain::models::ExecutableDetection::NotFound => {
                Err(AppError::openclaw_not_found())
            }
        }
    }

    /// All skill rows (`openclaw skills list --json`, read-only).
    pub fn list_skills(&self) -> Result<Vec<SkillRow>, AppError> {
        let exe = self.executable()?;
        self.skills.list_skills(&exe)
    }

    /// Toggles `skills.entries.<name>.enabled` through the config port's
    /// two-step write.
    ///
    /// Fail-closed order: name validation (0 runs) → executable detection →
    /// existence check via `skills list` (`skill-not-found`, 0 writes) →
    /// dry-run → commit. The value is a single JSON `true`/`false` argv
    /// element; no other skills config surface is touched.
    pub fn set_skill_enabled(&self, name: &str, enabled: bool) -> Result<(), AppError> {
        validate_skill_name(name)?;
        let exe = self.executable()?;
        let exists = self
            .skills
            .list_skills(&exe)?
            .iter()
            .any(|row| row.name == name);
        if !exists {
            return Err(AppError::skill_not_found(name));
        }
        let value = if enabled { "true" } else { "false" };
        self.config.write(
            &exe,
            &format!("skills.entries.{name}.enabled"),
            value,
            WriteMode::Plain,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::models::{ModelRow, ProviderDetail, ThinkingLevel};
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

    struct FakeSkills {
        rows: Mutex<Vec<SkillRow>>,
        calls: Arc<Mutex<u32>>,
    }

    impl OpenClawSkillsPort for FakeSkills {
        fn list_skills(&self, _exe: &Path) -> Result<Vec<SkillRow>, AppError> {
            *self.calls.lock().unwrap() += 1;
            Ok(self.rows.lock().unwrap().clone())
        }
    }

    struct FakeConfig {
        log: Arc<Mutex<Vec<String>>>,
        failure: Mutex<Option<AppError>>,
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
        fn read_raw(&self, _exe: &Path, _path: &str) -> Result<Option<String>, AppError> {
            unimplemented!()
        }
    }

    fn skill(name: &str, enabled: bool) -> SkillRow {
        SkillRow {
            name: name.into(),
            enabled: Some(enabled),
            eligible: Some(true),
            description: None,
            source: None,
        }
    }

    #[test]
    fn set_skill_enabled_writes_enabled_leaf() {
        let skills = Arc::new(FakeSkills {
            rows: Mutex::new(vec![skill("weather", true)]),
            calls: Arc::new(Mutex::new(0)),
        });
        let log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let config = Arc::new(FakeConfig {
            log: Arc::clone(&log),
            failure: Mutex::new(None),
        });
        let cloned = Arc::clone(&skills);
        let skills_port: Arc<dyn OpenClawSkillsPort> = cloned;
        let service = SkillsService::new(Arc::new(FixedOpenClaw), skills_port, config);
        service
            .set_skill_enabled("weather", false)
            .expect("toggle should succeed");
        assert_eq!(
            log.lock().unwrap()[0],
            "write:skills.entries.weather.enabled=false:Plain"
        );
        assert_eq!(*skills.calls.lock().unwrap(), 1, "one list for the check");
    }

    #[test]
    fn set_skill_enabled_writes_true_value() {
        let skills = Arc::new(FakeSkills {
            rows: Mutex::new(vec![skill("github", false)]),
            calls: Arc::new(Mutex::new(0)),
        });
        let log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let config = Arc::new(FakeConfig {
            log: Arc::clone(&log),
            failure: Mutex::new(None),
        });
        let service = SkillsService::new(Arc::new(FixedOpenClaw), skills, config);
        service
            .set_skill_enabled("github", true)
            .expect("toggle should succeed");
        assert_eq!(
            log.lock().unwrap()[0],
            "write:skills.entries.github.enabled=true:Plain"
        );
    }

    #[test]
    fn unknown_skill_is_not_found_with_zero_writes() {
        let skills = Arc::new(FakeSkills {
            rows: Mutex::new(vec![skill("weather", true)]),
            calls: Arc::new(Mutex::new(0)),
        });
        let log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let config = Arc::new(FakeConfig {
            log: Arc::clone(&log),
            failure: Mutex::new(None),
        });
        let service = SkillsService::new(Arc::new(FixedOpenClaw), skills, config);
        let err = service
            .set_skill_enabled("ghost", true)
            .expect_err("unknown skill");
        assert_eq!(err.code, "skill-not-found");
        assert!(log.lock().unwrap().is_empty(), "no config write");
    }

    #[test]
    fn invalid_name_has_zero_cli_calls() {
        let skills = Arc::new(FakeSkills {
            rows: Mutex::new(vec![skill("weather", true)]),
            calls: Arc::new(Mutex::new(0)),
        });
        let log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let config = Arc::new(FakeConfig {
            log: Arc::clone(&log),
            failure: Mutex::new(None),
        });
        let cloned = Arc::clone(&skills);
        let skills_port: Arc<dyn OpenClawSkillsPort> = cloned;
        let service = SkillsService::new(Arc::new(FixedOpenClaw), skills_port, config);
        for name in ["../evil", "a/b", "a b", "", ".hidden"] {
            let err = service
                .set_skill_enabled(name, true)
                .expect_err("must be rejected");
            assert_eq!(err.code, "skill-name-invalid", "{name:?}");
        }
        assert_eq!(*skills.calls.lock().unwrap(), 0, "no CLI calls at all");
        assert!(log.lock().unwrap().is_empty());
    }

    #[test]
    fn config_rejection_maps_through_unwrapped() {
        let skills = Arc::new(FakeSkills {
            rows: Mutex::new(vec![skill("weather", true)]),
            calls: Arc::new(Mutex::new(0)),
        });
        let log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let config = Arc::new(FakeConfig {
            log: Arc::clone(&log),
            failure: Mutex::new(None),
        });
        *config.failure.lock().unwrap() = Some(AppError::openclaw_config_invalid("rejected"));
        let service = SkillsService::new(Arc::new(FixedOpenClaw), skills, config);
        let err = service
            .set_skill_enabled("weather", true)
            .expect_err("config rejection");
        assert_eq!(err.code, "openclaw-config-invalid");
    }

    #[test]
    fn list_skills_delegates_to_port() {
        let skills = Arc::new(FakeSkills {
            rows: Mutex::new(vec![skill("weather", true)]),
            calls: Arc::new(Mutex::new(0)),
        });
        let log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let config = Arc::new(FakeConfig {
            log: Arc::clone(&log),
            failure: Mutex::new(None),
        });
        let service = SkillsService::new(Arc::new(FixedOpenClaw), skills, config);
        let rows = service.list_skills().expect("list");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "weather");
    }
}
