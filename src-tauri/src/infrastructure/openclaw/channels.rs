//! `OpenClawChannelsAdapter` — non-interactive `openclaw channels` /
//! `openclaw pairing` CLI invocations (Phase 6).
//!
//! Every invocation is a structured `executable + argv` request through the
//! `ProcessPort` (S1/S2) with the exact contract argv:
//!
//! - `channels list --all --json`
//! - `channels status --json`
//! - `pairing list <channel> --json`
//! - `pairing approve <channel> <code>`
//!
//! No `--probe`/`capabilities`/`resolve`/`logs`/`dead-letters` (non-goals),
//! no credentials in argv (S2/S3). Row parsing is fail-soft (contract §4):
//! `id`/`code`-less rows are dropped, unknown raw values are kept.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use crate::domain::models::channels::{
    parse_channel_list_row, parse_channel_status, parse_pairing_requests, ChannelRow,
    ChannelStatus, PairingRequest,
};
use crate::domain::ports::openclaw_channels::OpenClawChannelsPort;
use crate::domain::ports::process::{ProcessError, ProcessOutput, ProcessPort, ProcessRequest};
use crate::error::AppError;

/// Contract timeout for channels list/status and pairing calls (Phase 6:
/// 30s, same class as the config calls).
const CHANNELS_TIMEOUT: Duration = Duration::from_secs(30);

/// OpenClaw channels adapter. All CLI invocations go through the
/// `ProcessPort`.
pub struct OpenClawChannelsAdapter {
    process: Arc<dyn ProcessPort>,
}

impl OpenClawChannelsAdapter {
    pub fn new(process: Arc<dyn ProcessPort>) -> Self {
        Self { process }
    }

    fn run_cli(
        &self,
        executable: &Path,
        argv: Vec<String>,
        label: &str,
    ) -> Result<ProcessOutput, AppError> {
        let request = ProcessRequest::new(executable.to_path_buf(), argv, CHANNELS_TIMEOUT);
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
/// (`{"ok":false,"error":...}`) — the Phase 1/5 convention. Returns `None`
/// when the stdout carries no envelope.
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

/// Builds the failure detail for a non-zero exit (envelope + stderr).
fn failure_detail(label: &str, output: &ProcessOutput) -> String {
    let stderr = output.stderr.trim();
    let detail = match (parse_failure_envelope(&output.stdout), stderr.is_empty()) {
        (Some(envelope), true) => envelope,
        (Some(envelope), false) => format!("{envelope} ({stderr})"),
        (None, false) => format!("exit code {} ({stderr})", output.exit_code),
        (None, true) => format!("exit code {}", output.exit_code),
    };
    format!("{label}: {detail}")
}

impl OpenClawChannelsPort for OpenClawChannelsAdapter {
    fn list_channels(&self, executable: &Path) -> Result<Vec<ChannelRow>, AppError> {
        const LABEL: &str = "openclaw channels list --all --json";
        let output = self.run_cli(
            executable,
            vec![
                "channels".into(),
                "list".into(),
                "--all".into(),
                "--json".into(),
            ],
            LABEL,
        )?;
        if output.exit_code != 0 {
            return Err(AppError::openclaw_channels_failed(failure_detail(
                LABEL, &output,
            )));
        }
        let value = serde_json::from_str::<serde_json::Value>(&output.stdout).map_err(|err| {
            AppError::openclaw_channels_failed(format!(
                "{LABEL}: unparseable output: {} ({err})",
                output.stdout.trim()
            ))
        })?;
        // Rows live at the top level (array) or under a `channels` key
        // (object envelope) — accept both (fail-soft).
        let items = match &value {
            serde_json::Value::Array(items) => items.clone(),
            serde_json::Value::Object(map) => map
                .get("channels")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        Ok(items.iter().filter_map(parse_channel_list_row).collect())
    }

    fn channel_status(&self, executable: &Path) -> Result<ChannelStatus, AppError> {
        const LABEL: &str = "openclaw channels status --json";
        let output = self.run_cli(
            executable,
            vec!["channels".into(), "status".into(), "--json".into()],
            LABEL,
        )?;
        if output.exit_code != 0 {
            return Err(AppError::openclaw_channels_failed(failure_detail(
                LABEL, &output,
            )));
        }
        let value = serde_json::from_str::<serde_json::Value>(&output.stdout).map_err(|err| {
            AppError::openclaw_channels_failed(format!(
                "{LABEL}: unparseable output: {} ({err})",
                output.stdout.trim()
            ))
        })?;
        Ok(parse_channel_status(&value))
    }

    fn pairing_list(
        &self,
        executable: &Path,
        channel: &str,
    ) -> Result<Vec<PairingRequest>, AppError> {
        const LABEL: &str = "openclaw pairing list";
        let output = self.run_cli(
            executable,
            vec![
                "pairing".into(),
                "list".into(),
                channel.to_string(),
                "--json".into(),
            ],
            LABEL,
        )?;
        if output.exit_code != 0 {
            return Err(AppError::openclaw_pairing_failed(failure_detail(
                LABEL, &output,
            )));
        }
        let value = serde_json::from_str::<serde_json::Value>(&output.stdout).map_err(|err| {
            AppError::openclaw_pairing_failed(format!(
                "{LABEL}: unparseable output: {} ({err})",
                output.stdout.trim()
            ))
        })?;
        Ok(parse_pairing_requests(&value))
    }

    fn pairing_approve(
        &self,
        executable: &Path,
        channel: &str,
        code: &str,
    ) -> Result<(), AppError> {
        const LABEL: &str = "openclaw pairing approve";
        let output = self.run_cli(
            executable,
            vec![
                "pairing".into(),
                "approve".into(),
                channel.to_string(),
                code.to_string(),
            ],
            LABEL,
        )?;
        if output.exit_code != 0 {
            return Err(AppError::openclaw_pairing_failed(failure_detail(
                LABEL, &output,
            )));
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
    ) -> (OpenClawChannelsAdapter, Arc<Mutex<Vec<ProcessRequest>>>) {
        let requests: Arc<Mutex<Vec<ProcessRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let fake = Arc::new(ScriptedProcess {
            responses: Arc::new(Mutex::new(responses)),
            requests: Arc::clone(&requests),
        });
        (OpenClawChannelsAdapter::new(fake), requests)
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
    fn list_channels_exact_argv_and_rows() {
        let body = r#"{"ok":true,"channels":[
            {"id":"discord","installed":true,"configured":true,"enabled":false},
            {"id":"telegram","installed":true,"configured":false,"enabled":true},
            {"installed":true},
            {"id":""}
        ]}"#;
        let (adapter, requests) = scripted(vec![Ok(output(0, body, ""))]);
        let rows = adapter.list_channels(Path::new(EXE)).expect("parse");
        assert_eq!(rows.len(), 2, "id-less/empty-id rows dropped");
        assert_eq!(rows[0].id, "discord");
        assert!(rows[0].installed && rows[0].configured);
        assert!(!rows[0].enabled);
        assert_eq!(rows[1].id, "telegram");
        let argv = requests.lock().unwrap()[0].argv.clone();
        assert_eq!(argv, vec!["channels", "list", "--all", "--json"]);
        // The 30s contract timeout applies.
        assert_eq!(requests.lock().unwrap()[0].timeout, CHANNELS_TIMEOUT);
    }

    #[test]
    fn list_channels_accepts_bare_array_shape() {
        let body = r#"[{"id":"discord","enabled":true}]"#;
        let (adapter, _) = scripted(vec![Ok(output(0, body, ""))]);
        let rows = adapter.list_channels(Path::new(EXE)).expect("parse");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "discord");
        assert!(rows[0].enabled);
    }

    #[test]
    fn list_channels_nonzero_exit_is_channels_failed_with_envelope() {
        let body =
            r#"{"ok":false,"error":{"type":"cli_error","message":"channels config unreadable"}}"#;
        let (adapter, _) = scripted(vec![Ok(output(1, body, ""))]);
        let err = adapter
            .list_channels(Path::new(EXE))
            .expect_err("nonzero exit");
        assert_eq!(err.code, "openclaw-channels-failed");
        assert!(err.message.contains("channels config unreadable"));
    }

    #[test]
    fn list_channels_nonzero_exit_without_envelope_keeps_stderr_masked() {
        let (adapter, _) = scripted(vec![Ok(output(2, "", "boom: token sk-fake123456789"))]);
        let err = adapter
            .list_channels(Path::new(EXE))
            .expect_err("nonzero exit");
        assert_eq!(err.code, "openclaw-channels-failed");
        // S3: the secret in stderr must be masked in the error message.
        assert!(!err.message.contains("sk-fake123456789"));
        assert!(err.message.contains("sk-****"));
    }

    #[test]
    fn list_channels_malformed_output_is_channels_failed() {
        let (adapter, _) = scripted(vec![Ok(output(0, "channels truncated {", ""))]);
        let err = adapter
            .list_channels(Path::new(EXE))
            .expect_err("malformed");
        assert_eq!(err.code, "openclaw-channels-failed");
    }

    #[test]
    fn channel_status_exact_argv_and_config_only_fallback() {
        let body = r#"{"ok":true,"gatewayReachable":true,"channels":[{"id":"discord","state":"connected"},{"id":"telegram"},{"state":"no-id"}]}"#;
        let (adapter, requests) = scripted(vec![Ok(output(0, body, ""))]);
        let status = adapter.channel_status(Path::new(EXE)).expect("parse");
        assert!(status.gateway_reachable);
        assert_eq!(status.rows.len(), 2, "id-less row dropped");
        assert_eq!(status.rows[0].state.as_deref(), Some("connected"));
        assert_eq!(status.rows[1].state, None);
        let argv = requests.lock().unwrap()[0].argv.clone();
        assert_eq!(argv, vec!["channels", "status", "--json"]);

        // Missing/false gatewayReachable → config-only fallback.
        let (adapter, _) = scripted(vec![Ok(output(0, r#"{"ok":true}"#, ""))]);
        let status = adapter.channel_status(Path::new(EXE)).expect("parse");
        assert!(!status.gateway_reachable);
        assert!(status.rows.is_empty());
    }

    #[test]
    fn channel_status_nonzero_is_channels_failed() {
        let (adapter, _) = scripted(vec![Ok(output(
            1,
            r#"{"ok":false,"error":"gateway stopped"}"#,
            "",
        ))]);
        let err = adapter
            .channel_status(Path::new(EXE))
            .expect_err("nonzero exit");
        assert_eq!(err.code, "openclaw-channels-failed");
        assert!(err.message.contains("gateway stopped"));
    }

    #[test]
    fn pairing_list_exact_argv_and_rows() {
        let body = r#"{"ok":true,"requests":[{"code":"AB12CD34","sender":"user-1"},{"sender":"dropped"}]}"#;
        let (adapter, requests) = scripted(vec![Ok(output(0, body, ""))]);
        let requests_parsed = adapter
            .pairing_list(Path::new(EXE), "discord")
            .expect("parse");
        assert_eq!(requests_parsed.len(), 1, "code-less row dropped");
        assert_eq!(requests_parsed[0].code, "AB12CD34");
        assert_eq!(requests_parsed[0].sender.as_deref(), Some("user-1"));
        let argv = requests.lock().unwrap()[0].argv.clone();
        assert_eq!(argv, vec!["pairing", "list", "discord", "--json"]);
    }

    #[test]
    fn pairing_list_nonzero_is_pairing_failed() {
        let (adapter, _) = scripted(vec![Ok(output(
            1,
            r#"{"ok":false,"error":{"type":"cli_error","message":"unknown channel: slack"}}"#,
            "",
        ))]);
        let err = adapter
            .pairing_list(Path::new(EXE), "slack")
            .expect_err("nonzero exit");
        assert_eq!(err.code, "openclaw-pairing-failed");
        assert!(err.message.contains("unknown channel: slack"));
    }

    #[test]
    fn pairing_list_malformed_is_pairing_failed() {
        let (adapter, _) = scripted(vec![Ok(output(0, "pairing rows {", ""))]);
        let err = adapter
            .pairing_list(Path::new(EXE), "discord")
            .expect_err("malformed");
        assert_eq!(err.code, "openclaw-pairing-failed");
    }

    #[test]
    fn pairing_approve_exact_single_element_code() {
        let (adapter, requests) = scripted(vec![Ok(output(0, "", ""))]);
        adapter
            .pairing_approve(Path::new(EXE), "telegram", "AB12CD34")
            .expect("approve");
        let argv = requests.lock().unwrap()[0].argv.clone();
        assert_eq!(argv, vec!["pairing", "approve", "telegram", "AB12CD34"]);
        // The code is one argv element, never interpolated.
        assert_eq!(requests.lock().unwrap()[0].timeout, CHANNELS_TIMEOUT);
    }

    #[test]
    fn pairing_approve_nonzero_is_pairing_failed() {
        let (adapter, _) = scripted(vec![Ok(output(
            1,
            r#"{"ok":false,"error":{"type":"cli_error","message":"unknown pairing code: ZZ"}}"#,
            "",
        ))]);
        let err = adapter
            .pairing_approve(Path::new(EXE), "discord", "ZZ")
            .expect_err("unknown code");
        assert_eq!(err.code, "openclaw-pairing-failed");
        assert!(err.message.contains("unknown pairing code: ZZ"));
    }

    #[test]
    fn process_errors_map_to_stable_codes() {
        let (adapter, _) = scripted(vec![Err(ProcessError::NotFound {
            executable: EXE.into(),
        })]);
        assert_eq!(
            adapter.list_channels(Path::new(EXE)).unwrap_err().code,
            "openclaw-not-found"
        );
        let (adapter, _) = scripted(vec![Err(ProcessError::Timeout {
            executable: EXE.into(),
        })]);
        assert_eq!(
            adapter.channel_status(Path::new(EXE)).unwrap_err().code,
            "process-timeout"
        );
        let (adapter, _) = scripted(vec![Err(ProcessError::SpawnFailed {
            message: "spawn failed: denied".into(),
        })]);
        assert_eq!(
            adapter
                .pairing_approve(Path::new(EXE), "discord", "ABCD")
                .unwrap_err()
                .code,
            "process-failed"
        );
    }
}
