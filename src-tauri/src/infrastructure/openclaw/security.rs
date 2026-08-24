//! `OpenClawSecurityAdapter` — read-only `openclaw security audit` CLI
//! invocation (Phase 5).
//!
//! Every invocation is a structured `executable + argv` request through the
//! `ProcessPort` (S1/S2). The audit is cold (no live Gateway probe): the
//! argv is exactly `security audit --json` — never `--deep`/`--fix`
//! (non-goals) and never `--token`/`--password` (no credentials, S2/S3).
//! Finding rows are fail-soft: `checkId` required, the rest `null`.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use crate::domain::models::tools::{parse_audit_document, SecurityAuditResult};
use crate::domain::ports::openclaw_security::OpenClawSecurityPort;
use crate::domain::ports::process::{ProcessError, ProcessOutput, ProcessPort, ProcessRequest};
use crate::error::AppError;

/// Contract timeout for the cold security audit (Phase 5: 60s — a cold
/// filesystem/ACL scan is slower than the 30s config calls).
const AUDIT_TIMEOUT: Duration = Duration::from_secs(60);

/// OpenClaw security adapter. All CLI invocations go through the
/// `ProcessPort`.
pub struct OpenClawSecurityAdapter {
    process: Arc<dyn ProcessPort>,
}

impl OpenClawSecurityAdapter {
    pub fn new(process: Arc<dyn ProcessPort>) -> Self {
        Self { process }
    }

    fn run_cli(
        &self,
        executable: &Path,
        argv: Vec<String>,
        label: &str,
    ) -> Result<ProcessOutput, AppError> {
        let request = ProcessRequest::new(executable.to_path_buf(), argv, AUDIT_TIMEOUT);
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

impl OpenClawSecurityPort for OpenClawSecurityAdapter {
    fn run_security_audit(&self, executable: &Path) -> Result<SecurityAuditResult, AppError> {
        const LABEL: &str = "openclaw security audit --json";
        let output = self.run_cli(
            executable,
            vec!["security".into(), "audit".into(), "--json".into()],
            LABEL,
        )?;
        if output.exit_code != 0 {
            // Reuse the Phase 1 JSON failure envelope convention: the
            // failure detail (if any) lives in `{"ok":false,"error":...}`.
            let stderr = output.stderr.trim();
            let detail = match (parse_failure_envelope(&output.stdout), stderr.is_empty()) {
                (Some(envelope), true) => envelope,
                (Some(envelope), false) => format!("{envelope} ({stderr})"),
                (None, false) => format!("exit code {} ({stderr})", output.exit_code),
                (None, true) => format!("exit code {}", output.exit_code),
            };
            return Err(AppError::openclaw_security_audit_failed(format!(
                "{LABEL}: {detail}"
            )));
        }
        let value = serde_json::from_str::<serde_json::Value>(&output.stdout).map_err(|err| {
            AppError::openclaw_security_audit_failed(format!(
                "{LABEL}: unparseable output: {} ({err})",
                output.stdout.trim()
            ))
        })?;
        Ok(parse_audit_document(&value))
    }
}

/// Extracts the failure message from a CLI JSON failure envelope
/// (`{"ok":false,"error":{"type":"cli_error","message":...}}` or a plain
/// string error). Returns `None` when the stdout carries no envelope.
fn parse_failure_envelope(stdout: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(stdout.trim()).ok()?;
    if value.get("ok").and_then(|ok| ok.as_bool()) != Some(false) {
        return None;
    }
    match &value["error"] {
        serde_json::Value::String(message) => Some(message.clone()),
        serde_json::Value::Object(map) => map
            .get("message")
            .and_then(|m| m.as_str())
            .map(str::to_string),
        other if other.is_null() => None,
        _ => None,
    }
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
                    stdout: "{}".into(),
                    stderr: String::new(),
                    exit_code: 0,
                }),
            }
        }
    }

    fn scripted(
        responses: Vec<Result<ProcessOutput, ProcessError>>,
    ) -> (OpenClawSecurityAdapter, Arc<Mutex<Vec<ProcessRequest>>>) {
        let requests: Arc<Mutex<Vec<ProcessRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let fake = Arc::new(ScriptedProcess {
            responses: Arc::new(Mutex::new(responses)),
            requests: Arc::clone(&requests),
        });
        (OpenClawSecurityAdapter::new(fake), requests)
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
    fn audit_exact_argv_no_forbidden_flags() {
        let body = r#"{"ok":true,"findings":[{"checkId":"tools.exec.security_full_configured","severity":"warn"}],"summary":{"total":1},"suppressedFindings":[]}"#;
        let (adapter, requests) = scripted(vec![Ok(output(0, body, ""))]);
        let result = adapter.run_security_audit(Path::new(EXE)).expect("audit");
        let argv = requests.lock().unwrap()[0].argv.clone();
        assert_eq!(argv, vec!["security", "audit", "--json"]);
        for forbidden in ["--deep", "--fix", "--token", "--password"] {
            assert!(
                !argv.contains(&forbidden.to_string()),
                "{forbidden} forbidden"
            );
        }
        // The 60s contract timeout applies to the cold audit.
        assert_eq!(requests.lock().unwrap()[0].timeout, AUDIT_TIMEOUT);
        assert_eq!(result.findings.len(), 1);
        assert_eq!(
            result.findings[0].check_id,
            "tools.exec.security_full_configured"
        );
        assert_eq!(result.suppressed_count, 0);
        assert_eq!(result.summary, serde_json::json!({"total": 1}));
    }

    #[test]
    fn audit_nonzero_exit_is_audit_failed_with_envelope() {
        let body =
            r#"{"ok":false,"error":{"type":"cli_error","message":"audit config unreadable"}}"#;
        let (adapter, _) = scripted(vec![Ok(output(1, body, ""))]);
        let err = adapter
            .run_security_audit(Path::new(EXE))
            .expect_err("nonzero exit");
        assert_eq!(err.code, "openclaw-security-audit-failed");
        assert!(err.message.contains("audit config unreadable"));
    }

    #[test]
    fn audit_nonzero_exit_without_envelope() {
        let (adapter, _) = scripted(vec![Ok(output(2, "", "boom: token sk-fake123456789"))]);
        let err = adapter
            .run_security_audit(Path::new(EXE))
            .expect_err("nonzero exit");
        assert_eq!(err.code, "openclaw-security-audit-failed");
        // S3: the secret in stderr must be masked in the error message.
        assert!(!err.message.contains("sk-fake123456789"));
        assert!(err.message.contains("sk-****"));
    }

    #[test]
    fn audit_malformed_output_is_audit_failed() {
        let (adapter, _) = scripted(vec![Ok(output(0, "audit results truncated {", ""))]);
        let err = adapter
            .run_security_audit(Path::new(EXE))
            .expect_err("malformed");
        assert_eq!(err.code, "openclaw-security-audit-failed");
    }

    #[test]
    fn audit_findings_rows_are_fail_soft() {
        let body = r#"{"ok":true,"findings":[
            {"checkId":"fs.config.perms_world_readable","severity":"critical","title":"World readable","detail":"file is 0644"},
            {"severity":"warn"},
            {"checkId":"gateway.exposure.open","severity":"unknown-level"}
        ],"summary":{"checked":3},"suppressedFindings":[{"checkId":"s1"},{"checkId":"s2"}]}"#;
        let (adapter, _) = scripted(vec![Ok(output(0, body, ""))]);
        let result = adapter.run_security_audit(Path::new(EXE)).expect("audit");
        assert_eq!(result.findings.len(), 2, "checkId-less row dropped");
        assert_eq!(result.findings[0].severity.as_deref(), Some("critical"));
        assert_eq!(
            result.findings[1].severity.as_deref(),
            Some("unknown-level"),
            "raw kept"
        );
        assert_eq!(result.suppressed_count, 2);
        assert_eq!(result.summary, serde_json::json!({"checked": 3}));
    }

    #[test]
    fn audit_process_errors_map_to_stable_codes() {
        let (adapter, _) = scripted(vec![Err(ProcessError::NotFound {
            executable: EXE.into(),
        })]);
        assert_eq!(
            adapter.run_security_audit(Path::new(EXE)).unwrap_err().code,
            "openclaw-not-found"
        );
        let (adapter, _) = scripted(vec![Err(ProcessError::Timeout {
            executable: EXE.into(),
        })]);
        assert_eq!(
            adapter.run_security_audit(Path::new(EXE)).unwrap_err().code,
            "process-timeout"
        );
        let (adapter, _) = scripted(vec![Err(ProcessError::SpawnFailed {
            message: "spawn failed: denied".into(),
        })]);
        assert_eq!(
            adapter.run_security_audit(Path::new(EXE)).unwrap_err().code,
            "process-failed"
        );
    }
}
