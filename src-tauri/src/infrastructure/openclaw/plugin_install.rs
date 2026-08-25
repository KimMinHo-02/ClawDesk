//! `OpenClawPluginInstallAdapter` — `openclaw plugins install <npm-id>`
//! CLI invocation (Phase 6).
//!
//! Every invocation is a structured `executable + argv` request through the
//! `ProcessPort` (S1/S2). The npm id is a single argv element. Timeout is
//! the 300s contract (installs are slower than the 30s config calls).
//! `plugins update/remove` and any other plugin mutation are non-goals.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use crate::domain::ports::openclaw_plugin_install::OpenClawPluginInstallPort;
use crate::domain::ports::process::{ProcessError, ProcessOutput, ProcessPort, ProcessRequest};
use crate::error::AppError;

/// Contract timeout for `plugins install` (Phase 6: 300s).
const PLUGIN_INSTALL_TIMEOUT: Duration = Duration::from_secs(300);

/// OpenClaw plugin-install adapter. All CLI invocations go through the
/// `ProcessPort`.
pub struct OpenClawPluginInstallAdapter {
    process: Arc<dyn ProcessPort>,
}

impl OpenClawPluginInstallAdapter {
    pub fn new(process: Arc<dyn ProcessPort>) -> Self {
        Self { process }
    }

    fn run_cli(
        &self,
        executable: &Path,
        argv: Vec<String>,
        label: &str,
    ) -> Result<ProcessOutput, AppError> {
        let request = ProcessRequest::new(executable.to_path_buf(), argv, PLUGIN_INSTALL_TIMEOUT);
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

/// Extracts the failure message from a CLI JSON failure envelope
/// (`{"ok":false,"error":...}`) — the Phase 1/5 convention.
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

impl OpenClawPluginInstallPort for OpenClawPluginInstallAdapter {
    fn install_plugin(&self, executable: &Path, npm_id: &str) -> Result<(), AppError> {
        const LABEL: &str = "openclaw plugins install";
        let output = self.run_cli(
            executable,
            vec!["plugins".into(), "install".into(), npm_id.to_string()],
            LABEL,
        )?;
        if output.exit_code != 0 {
            let stderr = output.stderr.trim();
            let detail = match (parse_failure_envelope(&output.stdout), stderr.is_empty()) {
                (Some(envelope), true) => envelope,
                (Some(envelope), false) => format!("{envelope} ({stderr})"),
                (None, false) => format!("exit code {} ({stderr})", output.exit_code),
                (None, true) => format!("exit code {}", output.exit_code),
            };
            return Err(AppError::openclaw_plugin_install_failed(npm_id, detail));
        }
        Ok(())
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
    ) -> (
        OpenClawPluginInstallAdapter,
        Arc<Mutex<Vec<ProcessRequest>>>,
    ) {
        let requests: Arc<Mutex<Vec<ProcessRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let fake = Arc::new(ScriptedProcess {
            responses: Arc::new(Mutex::new(responses)),
            requests: Arc::clone(&requests),
        });
        (OpenClawPluginInstallAdapter::new(fake), requests)
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
    fn install_exact_argv_and_timeout() {
        let (adapter, requests) = scripted(vec![Ok(output(0, r#"{"ok":true}"#, ""))]);
        adapter
            .install_plugin(Path::new(EXE), "@openclaw/discord")
            .expect("install");
        let argv = requests.lock().unwrap()[0].argv.clone();
        assert_eq!(argv, vec!["plugins", "install", "@openclaw/discord"]);
        // The 300s contract timeout applies to installs.
        assert_eq!(
            requests.lock().unwrap()[0].timeout,
            Duration::from_secs(300)
        );
    }

    #[test]
    fn install_nonzero_with_envelope_is_install_failed() {
        let (adapter, _) = scripted(vec![Ok(output(
            1,
            r#"{"ok":false,"error":{"type":"cli_error","message":"npm registry unreachable"}}"#,
            "",
        ))]);
        let err = adapter
            .install_plugin(Path::new(EXE), "@openclaw/discord")
            .expect_err("nonzero exit");
        assert_eq!(err.code, "openclaw-plugin-install-failed");
        assert!(err.message.contains("@openclaw/discord"));
        assert!(err.message.contains("npm registry unreachable"));
    }

    #[test]
    fn install_nonzero_without_envelope_keeps_stderr_masked() {
        let (adapter, _) = scripted(vec![Ok(output(2, "", "boom: token sk-fake123456789"))]);
        let err = adapter
            .install_plugin(Path::new(EXE), "@openclaw/discord")
            .expect_err("nonzero exit");
        assert_eq!(err.code, "openclaw-plugin-install-failed");
        // S3: the secret in stderr must be masked in the error message.
        assert!(!err.message.contains("sk-fake123456789"));
        assert!(err.message.contains("sk-****"));
    }

    #[test]
    fn install_process_errors_map_to_stable_codes() {
        let (adapter, _) = scripted(vec![Err(ProcessError::NotFound {
            executable: EXE.into(),
        })]);
        assert_eq!(
            adapter
                .install_plugin(Path::new(EXE), "@openclaw/discord")
                .unwrap_err()
                .code,
            "openclaw-not-found"
        );
        let (adapter, _) = scripted(vec![Err(ProcessError::Timeout {
            executable: EXE.into(),
        })]);
        assert_eq!(
            adapter
                .install_plugin(Path::new(EXE), "@openclaw/discord")
                .unwrap_err()
                .code,
            "process-timeout"
        );
        let (adapter, _) = scripted(vec![Err(ProcessError::SpawnFailed {
            message: "spawn failed: denied".into(),
        })]);
        assert_eq!(
            adapter
                .install_plugin(Path::new(EXE), "@openclaw/discord")
                .unwrap_err()
                .code,
            "process-failed"
        );
    }
}
