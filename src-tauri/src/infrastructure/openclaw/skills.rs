//! `OpenClawSkillsAdapter` — read-only `openclaw skills` CLI invocations
//! (Phase 4).
//!
//! Every invocation is a structured `executable + argv` request through the
//! `ProcessPort` (S1/S2). Row parsing is fail-soft: a row is kept as long as
//! it has a `name`; missing optional fields become `null` (contract §1).
//! Skill activation is a config write and stays in the service layer
//! (Phase 3 `OpenClawConfigPort`).

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use crate::domain::models::skills::SkillRow;
use crate::domain::ports::openclaw_skills::OpenClawSkillsPort;
use crate::domain::ports::process::{ProcessError, ProcessOutput, ProcessPort, ProcessRequest};
use crate::error::AppError;

/// Contract timeout for skills CLI calls (Phase 4: 30s).
const SKILLS_TIMEOUT: Duration = Duration::from_secs(30);

/// OpenClaw skills adapter. All CLI invocations go through the `ProcessPort`.
pub struct OpenClawSkillsAdapter {
    process: Arc<dyn ProcessPort>,
}

impl OpenClawSkillsAdapter {
    pub fn new(process: Arc<dyn ProcessPort>) -> Self {
        Self { process }
    }

    fn run_cli(
        &self,
        executable: &Path,
        argv: Vec<String>,
        label: &str,
    ) -> Result<ProcessOutput, AppError> {
        let request = ProcessRequest::new(executable.to_path_buf(), argv, SKILLS_TIMEOUT);
        match self.process.run(&request) {
            Ok(output) => Ok(output),
            Err(ProcessError::NotFound { .. }) => Err(AppError::openclaw_not_found()),
            Err(ProcessError::Timeout { .. }) => Err(AppError::process_timeout(label)),
            Err(ProcessError::SpawnFailed { message }) => {
                Err(AppError::process_failed(label, message))
            }
        }
    }
}

impl OpenClawSkillsPort for OpenClawSkillsAdapter {
    fn list_skills(&self, executable: &Path) -> Result<Vec<SkillRow>, AppError> {
        const LABEL: &str = "openclaw skills list --json";
        let output = self.run_cli(
            executable,
            vec!["skills".into(), "list".into(), "--json".into()],
            LABEL,
        )?;
        if output.exit_code != 0 {
            return Err(AppError::openclaw_skills_read_failed(format!(
                "{LABEL}: exit code {}: {}",
                output.exit_code,
                output.stderr.trim()
            )));
        }
        let value = serde_json::from_str::<serde_json::Value>(&output.stdout).map_err(|err| {
            AppError::openclaw_skills_read_failed(format!(
                "{LABEL}: unparseable output: {} ({err})",
                output.stdout.trim()
            ))
        })?;
        // The rows live either at the top level (array) or under a
        // `skills` key (object envelope) — accept both (fail-soft).
        let rows_value = match &value {
            serde_json::Value::Array(_) => value,
            serde_json::Value::Object(map) => map
                .get("skills")
                .cloned()
                .unwrap_or(serde_json::Value::Array(vec![])),
            other => {
                return Err(AppError::openclaw_skills_read_failed(format!(
                    "{LABEL}: expected array/object, got {}",
                    other
                )))
            }
        };
        let rows = match &rows_value {
            serde_json::Value::Array(items) => items,
            other => {
                return Err(AppError::openclaw_skills_read_failed(format!(
                    "{LABEL}: expected a row array, got {}",
                    other
                )))
            }
        };
        Ok(rows.iter().filter_map(parse_skill_row).collect())
    }
}

/// Parses one skill row (fail-soft: only `name` is required).
fn parse_skill_row(raw: &serde_json::Value) -> Option<SkillRow> {
    let name = raw.get("name")?.as_str()?.to_string();
    if name.is_empty() {
        return None;
    }
    let enabled = raw.get("enabled").and_then(|v| v.as_bool());
    let eligible = raw.get("eligible").and_then(|v| v.as_bool());
    let description = raw
        .get("description")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    // The load-source key name is not fixed by the docs; accept both.
    let source = raw
        .get("source")
        .or_else(|| raw.get("origin"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    Some(SkillRow {
        name,
        enabled,
        eligible,
        description,
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct ScriptedProcess {
        responses: Arc<std::sync::Mutex<Vec<Result<ProcessOutput, ProcessError>>>>,
        requests: Arc<Mutex<Vec<ProcessRequest>>>,
    }

    impl ProcessPort for ScriptedProcess {
        fn run(&self, request: &ProcessRequest) -> Result<ProcessOutput, ProcessError> {
            self.requests.lock().unwrap().push(request.clone());
            let mut queue = self.responses.lock().unwrap();
            match queue.first().cloned() {
                Some(response) => {
                    let _ = queue.remove(0);
                    response
                }
                None => Ok(ProcessOutput {
                    stdout: "[]".into(),
                    stderr: String::new(),
                    exit_code: 0,
                }),
            }
        }
    }

    fn scripted(
        responses: Vec<Result<ProcessOutput, ProcessError>>,
    ) -> (OpenClawSkillsAdapter, Arc<Mutex<Vec<ProcessRequest>>>) {
        let requests: Arc<Mutex<Vec<ProcessRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let fake = Arc::new(ScriptedProcess {
            responses: Arc::new(Mutex::new(responses)),
            requests: Arc::clone(&requests),
        });
        (OpenClawSkillsAdapter::new(fake), requests)
    }

    fn output(exit_code: i32, stdout: &str, stderr: &str) -> ProcessOutput {
        ProcessOutput {
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
            exit_code,
        }
    }

    const EXE: &str = "C:\\fake\\openclaw.exe";

    #[test]
    fn list_skills_exact_argv_and_rows() {
        let body = r#"{"ok":true,"skills":[
            {"name":"weather","enabled":true,"eligible":true,"description":"Weather skill"},
            {"name":"github","enabled":false,"eligible":true,"source":"bundled"},
            {"name":"minimal"}
        ]}"#;
        let (adapter, requests) = scripted(vec![Ok(output(0, body, ""))]);
        let rows = adapter.list_skills(Path::new(EXE)).expect("parse");
        assert_eq!(rows.len(), 3, "no row may be dropped");
        assert_eq!(rows[0].name, "weather");
        assert_eq!(rows[0].enabled, Some(true));
        assert_eq!(rows[0].eligible, Some(true));
        assert_eq!(rows[0].description.as_deref(), Some("Weather skill"));
        assert_eq!(rows[0].source.as_deref(), None);
        assert_eq!(rows[1].enabled, Some(false));
        assert_eq!(rows[1].source.as_deref(), Some("bundled"));
        // Fail-soft: a row with only `name` is kept, the rest is null.
        assert_eq!(rows[2].name, "minimal");
        assert_eq!(rows[2].enabled, None);
        assert_eq!(rows[2].eligible, None);
        let argv = requests.lock().unwrap()[0].argv.clone();
        assert_eq!(argv, vec!["skills", "list", "--json"]);
    }

    #[test]
    fn list_skills_accepts_bare_array_shape() {
        let body = r#"[{"name":"a","enabled":true,"eligible":true}]"#;
        let (adapter, _) = scripted(vec![Ok(output(0, body, ""))]);
        let rows = adapter.list_skills(Path::new(EXE)).expect("parse");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "a");
    }

    #[test]
    fn list_skills_malformed_json_is_read_failed() {
        let (adapter, _) = scripted(vec![Ok(output(0, "skills are fine", ""))]);
        let err = adapter.list_skills(Path::new(EXE)).expect_err("must fail");
        assert_eq!(err.code, "openclaw-skills-read-failed");
    }

    #[test]
    fn list_skills_nonzero_exit_is_read_failed() {
        let (adapter, _) = scripted(vec![Ok(output(2, "", "boom"))]);
        let err = adapter.list_skills(Path::new(EXE)).expect_err("must fail");
        assert_eq!(err.code, "openclaw-skills-read-failed");
    }

    #[test]
    fn list_skills_rows_without_name_are_skipped() {
        let body = r#"{"skills":[{"enabled":true},{"name":""},{"name":"ok"}]}"#;
        let (adapter, _) = scripted(vec![Ok(output(0, body, ""))]);
        let rows = adapter.list_skills(Path::new(EXE)).expect("parse");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "ok");
    }

    #[test]
    fn process_errors_map_to_stable_codes() {
        let (adapter, _) = scripted(vec![Err(ProcessError::NotFound {
            executable: EXE.into(),
        })]);
        assert_eq!(
            adapter.list_skills(Path::new(EXE)).unwrap_err().code,
            "openclaw-not-found"
        );
        let (adapter, _) = scripted(vec![Err(ProcessError::Timeout {
            executable: EXE.into(),
        })]);
        assert_eq!(
            adapter.list_skills(Path::new(EXE)).unwrap_err().code,
            "process-timeout"
        );
        let (adapter, _) = scripted(vec![Err(ProcessError::SpawnFailed {
            message: "spawn failed: denied".into(),
        })]);
        assert_eq!(
            adapter.list_skills(Path::new(EXE)).unwrap_err().code,
            "process-failed"
        );
    }
}
