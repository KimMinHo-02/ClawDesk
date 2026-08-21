//! `OpenClawAdapter` — OpenClaw executable/version/gateway/update detection.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::domain::models::openclaw::{
    ExecutableDetection, GatewayStatus, OpenClawVersion, UpdateState,
};
use crate::domain::ports::openclaw::OpenClawPort;
use crate::domain::ports::process::{ProcessError, ProcessOutput, ProcessPort, ProcessRequest};
use crate::error::AppError;
use crate::infrastructure::openclaw::parse::{
    parse_gateway_json, parse_update_json, parse_version_output,
};

const VERSION_TIMEOUT: Duration = Duration::from_secs(10);
const GATEWAY_TIMEOUT: Duration = Duration::from_secs(10);
const UPDATE_TIMEOUT: Duration = Duration::from_secs(15);

/// Candidate filenames for the OpenClaw CLI (npm shims + PATH executables).
const OPENCLAW_FILENAMES: [&str; 4] = ["openclaw.exe", "openclaw.cmd", "openclaw.ps1", "openclaw"];

/// OpenClaw adapter. All CLI invocations go through the `ProcessPort`.
pub struct OpenClawAdapter {
    process: Arc<dyn ProcessPort>,
    search_dirs: Vec<PathBuf>,
}

impl OpenClawAdapter {
    pub fn new(process: Arc<dyn ProcessPort>, search_dirs: Vec<PathBuf>) -> Self {
        Self {
            process,
            search_dirs,
        }
    }
}

impl OpenClawPort for OpenClawAdapter {
    fn detect_executable(&self) -> ExecutableDetection {
        for dir in &self.search_dirs {
            for name in OPENCLAW_FILENAMES {
                let candidate = dir.join(name);
                if candidate.is_file() {
                    return ExecutableDetection::Found { path: candidate };
                }
            }
        }
        ExecutableDetection::NotFound
    }

    fn version(&self, executable: &Path) -> Result<OpenClawVersion, AppError> {
        const LABEL: &str = "openclaw --version";
        let output = self.run_cli(executable, &["--version"], VERSION_TIMEOUT, LABEL)?;
        Self::require_success(&output, LABEL)?;
        parse_version_output(&output.stdout)
    }

    fn version_from_entry(&self, node: &Path, entry: &Path) -> Result<OpenClawVersion, AppError> {
        const LABEL: &str = "openclaw (package entry) --version";
        let argv = vec![
            entry.to_string_lossy().into_owned(),
            "--version".to_string(),
        ];
        let request = ProcessRequest::new(node.to_path_buf(), argv, VERSION_TIMEOUT);
        let output = match self.process.run(&request) {
            Ok(output) => output,
            Err(ProcessError::NotFound { .. }) => return Err(AppError::node_not_found()),
            Err(ProcessError::Timeout { .. }) => return Err(AppError::process_timeout(LABEL)),
            Err(ProcessError::SpawnFailed { message }) => {
                return Err(AppError::process_failed(LABEL, message))
            }
        };
        Self::require_success(&output, LABEL)?;
        parse_version_output(&output.stdout)
    }

    fn gateway_status(&self, executable: &Path) -> Result<GatewayStatus, AppError> {
        const LABEL: &str = "openclaw gateway status --json";
        let output = self.run_cli(
            executable,
            &["gateway", "status", "--json"],
            GATEWAY_TIMEOUT,
            LABEL,
        )?;
        match parse_gateway_json(&output.stdout) {
            // The real CLI exits non-zero when no gateway is reachable while
            // still printing a valid status payload — a parseable payload wins.
            Ok(status) => Ok(status),
            Err(parse_error) => {
                if output.exit_code == 0 {
                    Err(parse_error)
                } else {
                    Err(AppError::process_failed(
                        LABEL,
                        format!("exit code {}: {}", output.exit_code, output.stderr.trim()),
                    ))
                }
            }
        }
    }

    fn update_state(&self, executable: &Path) -> Result<UpdateState, AppError> {
        let output = match self.run_cli(
            executable,
            &["update", "status", "--json"],
            UPDATE_TIMEOUT,
            "openclaw update status --json",
        ) {
            Ok(output) => output,
            // The state simply cannot be determined; report Unknown.
            Err(_) => return Ok(UpdateState::Unknown),
        };
        Ok(parse_update_json(&output.stdout))
    }
}

impl OpenClawAdapter {
    /// Runs an OpenClaw CLI command through the process port.
    ///
    /// Returns the (already masked) process output whenever the process ran
    /// to completion — even with a non-zero exit code. The real
    /// `openclaw gateway status --json` exits 1 when no gateway is reachable,
    /// so each caller decides how strictly to treat the exit code.
    /// Spawn/timeout/not-found failures are mapped to structured errors here.
    fn run_cli(
        &self,
        executable: &Path,
        argv: &[&str],
        timeout: Duration,
        label: &str,
    ) -> Result<ProcessOutput, AppError> {
        let request = ProcessRequest::new(
            executable.to_path_buf(),
            argv.iter().map(|arg| arg.to_string()).collect(),
            timeout,
        );
        match self.process.run(&request) {
            Ok(output) => Ok(output),
            Err(ProcessError::NotFound { .. }) => Err(AppError::openclaw_not_found()),
            Err(ProcessError::Timeout { .. }) => Err(AppError::process_timeout(label)),
            Err(ProcessError::SpawnFailed { message }) => {
                Err(AppError::process_failed(label, message))
            }
        }
    }

    fn require_success(output: &ProcessOutput, label: &str) -> Result<(), AppError> {
        if output.exit_code == 0 {
            Ok(())
        } else {
            Err(AppError::process_failed(
                label,
                format!("exit code {}: {}", output.exit_code, output.stderr.trim()),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic in-memory ProcessPort for unit testing adapter logic.
    struct ScriptedProcess {
        response: Result<ProcessOutput, ProcessError>,
        last_request: Arc<std::sync::Mutex<Option<ProcessRequest>>>,
    }

    impl ProcessPort for ScriptedProcess {
        fn run(&self, request: &ProcessRequest) -> Result<ProcessOutput, ProcessError> {
            *self.last_request.lock().unwrap() = Some(request.clone());
            self.response.clone()
        }
    }

    fn scripted(
        response: Result<ProcessOutput, ProcessError>,
    ) -> (
        Arc<ScriptedProcess>,
        Arc<std::sync::Mutex<Option<ProcessRequest>>>,
    ) {
        let last: Arc<std::sync::Mutex<Option<ProcessRequest>>> =
            Arc::new(std::sync::Mutex::new(None));
        let fake = Arc::new(ScriptedProcess {
            response,
            last_request: Arc::clone(&last),
        });
        (fake, last)
    }

    fn output(exit_code: i32, stdout: &str, stderr: &str) -> ProcessOutput {
        ProcessOutput {
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
            exit_code,
        }
    }

    #[test]
    fn version_maps_process_output() {
        let (fake, _) = scripted(Ok(output(0, "OpenClaw 2026.7.1-2\n", "")));
        let adapter = OpenClawAdapter::new(fake, Vec::new());
        let version = adapter
            .version(Path::new("C:\\x\\openclaw.exe"))
            .expect("version should parse");
        assert_eq!(version.raw, "2026.7.1-2");
    }

    #[test]
    fn version_uses_structured_argv() {
        let (fake, last_request) = scripted(Ok(output(0, "OpenClaw 2026.7.1-2\n", "")));
        let adapter = OpenClawAdapter::new(fake, Vec::new());
        adapter
            .version(Path::new("C:\\x\\openclaw.exe"))
            .expect("should run");
        let request = last_request
            .lock()
            .unwrap()
            .take()
            .expect("request should be recorded");
        assert_eq!(request.executable, PathBuf::from("C:\\x\\openclaw.exe"));
        assert_eq!(request.argv, vec!["--version".to_string()]);
        assert!(request.env.is_empty(), "no shell, no injected commands");
    }

    #[test]
    fn version_non_zero_exit_is_structured_error() {
        let (fake, _) = scripted(Ok(output(1, "", "boom")));
        let adapter = OpenClawAdapter::new(fake, Vec::new());
        let err = adapter
            .version(Path::new("C:\\x\\openclaw.exe"))
            .expect_err("non-zero exit must be an error");
        assert_eq!(err.code, "process-failed");
        assert!(err.message.contains("boom"));
    }

    #[test]
    fn version_not_found_maps_to_openclaw_not_found() {
        let (fake, _) = scripted(Err(ProcessError::NotFound {
            executable: "C:\\x\\openclaw.exe".to_string(),
        }));
        let adapter = OpenClawAdapter::new(fake, Vec::new());
        let err = adapter
            .version(Path::new("C:\\x\\openclaw.exe"))
            .expect_err("not found must be an error");
        assert_eq!(err.code, "openclaw-not-found");
    }

    #[test]
    fn version_timeout_maps_to_process_timeout() {
        let (fake, _) = scripted(Err(ProcessError::Timeout {
            executable: "C:\\x\\openclaw.exe".to_string(),
        }));
        let adapter = OpenClawAdapter::new(fake, Vec::new());
        let err = adapter
            .version(Path::new("C:\\x\\openclaw.exe"))
            .expect_err("timeout must be an error");
        assert_eq!(err.code, "process-timeout");
    }

    const GATEWAY_STOPPED: &str = r#"{"ok":false,"primaryTargetId":null,"targets":[{"id":"localLoopback","url":"ws://127.0.0.1:18789","connect":{"ok":false},"server":null}]}"#;

    #[test]
    fn gateway_parses_and_maps_errors() {
        let (fake, _) = scripted(Ok(output(0, GATEWAY_STOPPED, "")));
        let adapter = OpenClawAdapter::new(fake, Vec::new());
        let status = adapter
            .gateway_status(Path::new("C:\\x\\openclaw.exe"))
            .expect("gateway should parse");
        assert_eq!(status.state, "stopped");
        assert_eq!(status.version, None);
        assert_eq!(status.port, Some(18789));
    }

    #[test]
    fn gateway_non_zero_exit_with_stopped_payload_is_ok() {
        // The real CLI exits 1 when no gateway is reachable.
        let (fake, _) = scripted(Ok(output(1, GATEWAY_STOPPED, "")));
        let adapter = OpenClawAdapter::new(fake, Vec::new());
        let status = adapter
            .gateway_status(Path::new("C:\\x\\openclaw.exe"))
            .expect("stopped payload must parse despite exit 1");
        assert_eq!(status.state, "stopped");
    }

    #[test]
    fn gateway_zero_exit_with_unparseable_output_is_parse_error() {
        let (fake, _) = scripted(Ok(output(0, "gateway is not json", "")));
        let adapter = OpenClawAdapter::new(fake, Vec::new());
        let err = adapter
            .gateway_status(Path::new("C:\\x\\openclaw.exe"))
            .expect_err("unparseable payload must be an error");
        assert_eq!(err.code, "openclaw-gateway-parse");
    }

    #[test]
    fn gateway_non_zero_exit_with_unparseable_output_is_process_failed() {
        let (fake, _) = scripted(Ok(output(1, "gateway is not json", "boom")));
        let adapter = OpenClawAdapter::new(fake, Vec::new());
        let err = adapter
            .gateway_status(Path::new("C:\\x\\openclaw.exe"))
            .expect_err("failed run with bad output must be an error");
        assert_eq!(err.code, "process-failed");
        assert!(err.message.contains("boom"));
    }

    #[test]
    fn update_state_unknown_on_process_failure() {
        let (fake, _) = scripted(Err(ProcessError::Timeout {
            executable: "openclaw".to_string(),
        }));
        let adapter = OpenClawAdapter::new(fake, Vec::new());
        let state = adapter
            .update_state(Path::new("C:\\x\\openclaw.exe"))
            .expect("update failure should be Unknown, not an error");
        assert_eq!(state, UpdateState::Unknown);
    }

    #[test]
    fn update_state_parses_payload() {
        let (fake, _) = scripted(Ok(output(
            0,
            r#"{"current":"2026.7.1","latest":"2026.7.1-2","updateAvailable":true}"#,
            "",
        )));
        let adapter = OpenClawAdapter::new(fake, Vec::new());
        let state = adapter
            .update_state(Path::new("C:\\x\\openclaw.exe"))
            .expect("update should parse");
        assert_eq!(state, UpdateState::UpdateAvailable);
    }
}
