//! `OpenClawConfigAdapter` — non-interactive `openclaw config` /
//! `openclaw models` CLI invocations (Phase 3).
//!
//! Every invocation is a structured `executable + argv` request through the
//! `ProcessPort` (S1/S2). Writes are two-step: `--dry-run --json` first;
//! when the dry run is not `ok`, nothing is written and
//! `openclaw-config-invalid` is returned. ClawDesk never touches
//! `openclaw.json` directly.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::domain::models::models::{
    ModelEntry, ModelRow, ProviderApiKey, ProviderDetail, ThinkingLevel, CLAWDESK_SECRET_ALIAS,
};
use crate::domain::ports::openclaw_config::{OpenClawConfigPort, WriteMode};
use crate::domain::ports::process::{ProcessError, ProcessOutput, ProcessPort, ProcessRequest};
use crate::error::AppError;

/// Contract timeout for config CLI calls (Phase 3: 30s).
const CONFIG_TIMEOUT: Duration = Duration::from_secs(30);

/// OpenClaw config adapter. All CLI invocations go through the `ProcessPort`.
pub struct OpenClawConfigAdapter {
    process: Arc<dyn ProcessPort>,
}

impl OpenClawConfigAdapter {
    pub fn new(process: Arc<dyn ProcessPort>) -> Self {
        Self { process }
    }

    fn run_cli(
        &self,
        executable: &Path,
        argv: Vec<String>,
        label: &str,
    ) -> Result<ProcessOutput, AppError> {
        let request = ProcessRequest::new(executable.to_path_buf(), argv, CONFIG_TIMEOUT);
        match self.process.run(&request) {
            Ok(output) => Ok(output),
            Err(ProcessError::NotFound { .. }) => Err(AppError::openclaw_not_found()),
            Err(ProcessError::Timeout { .. }) => Err(AppError::process_timeout(label)),
            Err(ProcessError::SpawnFailed { message }) => {
                Err(AppError::process_failed(label, message))
            }
        }
    }

    /// Parses the CLI's JSON output, mapping parse failures to the stable
    /// read-failed code (output is already masked).
    fn parse_json(output: &ProcessOutput, label: &str) -> Result<serde_json::Value, AppError> {
        if output.exit_code != 0 {
            return Err(AppError::openclaw_config_write_failed(format!(
                "{label}: exit code {}: {}",
                output.exit_code,
                output.stderr.trim()
            )));
        }
        serde_json::from_str::<serde_json::Value>(&output.stdout).map_err(|err| {
            AppError::openclaw_config_read_failed(format!(
                "{label}: unparseable output: {} ({err})",
                output.stdout.trim()
            ))
        })
    }

    /// Parses a `config set/unset --dry-run --json` envelope.
    fn parse_dry_run(output: &ProcessOutput) -> Result<bool, AppError> {
        let value = Self::parse_json(output, "openclaw config dry-run")?;
        let ok = value
            .get("ok")
            .and_then(|ok| ok.as_bool())
            .ok_or_else(|| AppError::openclaw_config_invalid("dry-run output has no `ok` flag"))?;
        if ok {
            return Ok(true);
        }
        let detail = value
            .get("errors")
            .and_then(|errors| errors.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.get("message").and_then(|m| m.as_str()))
                    .collect::<Vec<_>>()
                    .join("; ")
            })
            .filter(|joined| !joined.is_empty())
            .unwrap_or_else(|| "schema validation failed".to_string());
        Err(AppError::openclaw_config_invalid(detail))
    }

    fn mode_flag(mode: WriteMode) -> Option<&'static str> {
        match mode {
            WriteMode::Merge => Some("--merge"),
            WriteMode::Replace => Some("--replace"),
            WriteMode::Plain => None,
        }
    }
}

impl OpenClawConfigPort for OpenClawConfigAdapter {
    fn config_path(&self, executable: &Path) -> Result<PathBuf, AppError> {
        const LABEL: &str = "openclaw config file --json";
        let output = self.run_cli(
            executable,
            vec!["config".into(), "file".into(), "--json".into()],
            LABEL,
        )?;
        let value = Self::parse_json(&output, LABEL)?;
        value
            .get("path")
            .and_then(|p| p.as_str())
            .map(PathBuf::from)
            .ok_or_else(|| AppError::openclaw_config_read_failed(format!("{LABEL}: no path")))
    }

    fn read_providers(&self, executable: &Path) -> Result<Vec<ProviderDetail>, AppError> {
        const LABEL: &str = "openclaw config get models.providers --json";
        let output = self.run_cli(
            executable,
            vec![
                "config".into(),
                "get".into(),
                "models.providers".into(),
                "--json".into(),
            ],
            LABEL,
        )?;
        let value = Self::parse_json(&output, LABEL)?;
        match value {
            serde_json::Value::Null => Ok(Vec::new()),
            serde_json::Value::Object(map) => {
                let mut providers = Vec::with_capacity(map.len());
                for (id, raw) in map {
                    let provider = parse_provider(&id, &raw).ok_or_else(|| {
                        AppError::openclaw_config_read_failed(format!(
                            "{LABEL}: invalid provider entry {id}"
                        ))
                    })?;
                    providers.push(provider);
                }
                Ok(providers)
            }
            other => Err(AppError::openclaw_config_read_failed(format!(
                "{LABEL}: expected object, got {}",
                other
            ))),
        }
    }

    fn read_models(&self, executable: &Path) -> Result<Vec<ModelRow>, AppError> {
        const LABEL: &str = "openclaw models list --json";
        let output = self.run_cli(
            executable,
            vec!["models".into(), "list".into(), "--json".into()],
            LABEL,
        )?;
        let value = Self::parse_json(&output, LABEL)?;
        let rows_value = match &value {
            serde_json::Value::Array(_) => value,
            serde_json::Value::Object(map) => map
                .get("models")
                .cloned()
                .unwrap_or(serde_json::Value::Array(vec![])),
            other => {
                return Err(AppError::openclaw_config_read_failed(format!(
                    "{LABEL}: expected array/object, got {}",
                    other
                )))
            }
        };
        serde_json::from_value::<Vec<ModelRow>>(rows_value).map_err(|err| {
            AppError::openclaw_config_read_failed(format!("{LABEL}: invalid rows ({err})"))
        })
    }

    fn read_default_model(&self, executable: &Path) -> Result<Option<String>, AppError> {
        const LABEL: &str = "openclaw config get agents.defaults.model --json";
        let output = self.run_cli(
            executable,
            vec![
                "config".into(),
                "get".into(),
                "agents.defaults.model".into(),
                "--json".into(),
            ],
            LABEL,
        )?;
        let value = Self::parse_json(&output, LABEL)?;
        Ok(match value {
            serde_json::Value::Null => None,
            serde_json::Value::String(ref_) => Some(ref_),
            serde_json::Value::Object(map) => map
                .get("primary")
                .and_then(|p| p.as_str())
                .map(str::to_string),
            other => {
                return Err(AppError::openclaw_config_read_failed(format!(
                    "{LABEL}: unexpected shape {}",
                    other
                )))
            }
        })
    }

    fn read_thinking_default(&self, executable: &Path) -> Result<Option<ThinkingLevel>, AppError> {
        const LABEL: &str = "openclaw config get agents.defaults.thinkingDefault --json";
        let output = self.run_cli(
            executable,
            vec![
                "config".into(),
                "get".into(),
                "agents.defaults.thinkingDefault".into(),
                "--json".into(),
            ],
            LABEL,
        )?;
        let value = Self::parse_json(&output, LABEL)?;
        match value {
            serde_json::Value::Null => Ok(None),
            serde_json::Value::String(raw) => Ok(ThinkingLevel::parse(&raw)),
            other => Err(AppError::openclaw_config_read_failed(format!(
                "{LABEL}: unexpected shape {}",
                other
            ))),
        }
    }

    fn read_raw(&self, executable: &Path, path: &str) -> Result<Option<String>, AppError> {
        let label = format!("openclaw config get {path} --json");
        let output = self.run_cli(
            executable,
            vec![
                "config".into(),
                "get".into(),
                path.to_string(),
                "--json".into(),
            ],
            &label,
        )?;
        let value = Self::parse_json(&output, &label)?;
        Ok(match value {
            serde_json::Value::Null => None,
            other => Some(other.to_string()),
        })
    }

    fn write(
        &self,
        executable: &Path,
        path: &str,
        value_json: &str,
        mode: WriteMode,
    ) -> Result<(), AppError> {
        const LABEL: &str = "openclaw config set";
        let mut dry_argv = vec![
            "config".to_string(),
            "set".to_string(),
            path.to_string(),
            value_json.to_string(),
            "--strict-json".to_string(),
        ];
        if let Some(flag) = Self::mode_flag(mode) {
            dry_argv.push(flag.to_string());
        }
        dry_argv.push("--dry-run".to_string());
        dry_argv.push("--json".to_string());

        let dry = self.run_cli(executable, dry_argv, LABEL)?;
        Self::parse_dry_run(&dry)?;

        let mut real_argv = vec![
            "config".to_string(),
            "set".to_string(),
            path.to_string(),
            value_json.to_string(),
            "--strict-json".to_string(),
        ];
        if let Some(flag) = Self::mode_flag(mode) {
            real_argv.push(flag.to_string());
        }
        let output = self.run_cli(executable, real_argv, LABEL)?;
        if output.exit_code != 0 {
            return Err(AppError::openclaw_config_write_failed(format!(
                "exit code {}: {}",
                output.exit_code,
                output.stderr.trim()
            )));
        }
        Ok(())
    }

    fn unset(&self, executable: &Path, path: &str) -> Result<(), AppError> {
        const LABEL: &str = "openclaw config unset";
        let dry_argv = vec![
            "config".to_string(),
            "unset".to_string(),
            path.to_string(),
            "--dry-run".to_string(),
            "--json".to_string(),
        ];
        let dry = self.run_cli(executable, dry_argv, LABEL)?;
        Self::parse_dry_run(&dry).map_err(|err| {
            // An unset dry-run failure means the target is missing/rejected;
            // the config is unchanged.
            AppError::openclaw_config_read_failed(err.message)
        })?;

        let output = self.run_cli(
            executable,
            vec!["config".to_string(), "unset".to_string(), path.to_string()],
            LABEL,
        )?;
        if output.exit_code != 0 {
            return Err(AppError::openclaw_config_write_failed(format!(
                "exit code {}: {}",
                output.exit_code,
                output.stderr.trim()
            )));
        }
        Ok(())
    }

    fn set_default_model(&self, executable: &Path, model_ref: &str) -> Result<(), AppError> {
        const LABEL: &str = "openclaw models set";
        let output = self.run_cli(
            executable,
            vec![
                "models".to_string(),
                "set".to_string(),
                model_ref.to_string(),
            ],
            LABEL,
        )?;
        if output.exit_code != 0 {
            return Err(AppError::openclaw_config_write_failed(format!(
                "exit code {}: {}",
                output.exit_code,
                output.stderr.trim()
            )));
        }
        Ok(())
    }
}

/// Parses one provider entry from the redacted config snapshot.
fn parse_provider(id: &str, raw: &serde_json::Value) -> Option<ProviderDetail> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct ProviderRead {
        #[serde(default)]
        base_url: Option<String>,
        #[serde(default)]
        api: Option<String>,
        #[serde(default)]
        api_key: Option<serde_json::Value>,
        #[serde(default)]
        models: Option<Vec<ModelEntry>>,
    }
    let read: ProviderRead = serde_json::from_value(raw.clone()).ok()?;
    Some(ProviderDetail {
        id: id.to_string(),
        base_url: read.base_url,
        api: read.api,
        api_key: parse_api_key_state(read.api_key.as_ref()),
        models: read.models.unwrap_or_default(),
    })
}

/// Classifies the `apiKey` field without ever exposing its value:
/// ClawDesk exec ref → `Managed`, any other present value → `Other`.
fn parse_api_key_state(api_key: Option<&serde_json::Value>) -> ProviderApiKey {
    match api_key {
        None | Some(serde_json::Value::Null) => ProviderApiKey::Absent,
        Some(serde_json::Value::Object(map)) => {
            let is_clawdesk_ref = map.get("source").and_then(|v| v.as_str()) == Some("exec")
                && map.get("provider").and_then(|v| v.as_str()) == Some(CLAWDESK_SECRET_ALIAS);
            if is_clawdesk_ref {
                ProviderApiKey::Managed
            } else {
                ProviderApiKey::Other
            }
        }
        Some(_) => ProviderApiKey::Other,
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
                    stdout: "null".into(),
                    stderr: String::new(),
                    exit_code: 0,
                }),
            }
        }
    }

    fn scripted(
        responses: Vec<Result<ProcessOutput, ProcessError>>,
    ) -> (OpenClawConfigAdapter, Arc<Mutex<Vec<ProcessRequest>>>) {
        let requests: Arc<Mutex<Vec<ProcessRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let fake = Arc::new(ScriptedProcess {
            responses: Arc::new(Mutex::new(responses)),
            requests: Arc::clone(&requests),
        });
        (OpenClawConfigAdapter::new(fake), requests)
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
    fn config_path_parses_path() {
        let (adapter, requests) = scripted(vec![Ok(output(
            0,
            r#"{"path":"C:\\Users\\u\\.openclaw\\openclaw.json"}"#,
            "",
        ))]);
        let path = adapter.config_path(Path::new(EXE)).expect("should parse");
        assert_eq!(
            path,
            PathBuf::from("C:\\Users\\u\\.openclaw\\openclaw.json")
        );
        let argv = requests.lock().unwrap()[0].argv.clone();
        assert_eq!(argv, vec!["config", "file", "--json"]);
    }

    #[test]
    fn read_providers_classifies_api_key_states() {
        let body = r#"{
            "acme": {
                "baseUrl": "https://api.acme.test/v1",
                "api": "openai-completions",
                "apiKey": {"source":"exec","provider":"clawdesk","id":"providers/acme/apiKey"},
                "models": [{"id":"m1","name":"M1","reasoning":true}]
            },
            "plain": {
                "baseUrl": "https://plain.test",
                "apiKey": "sk-fake123456789"
            }
        }"#;
        let (adapter, requests) = scripted(vec![Ok(output(0, body, ""))]);
        let providers = adapter.read_providers(Path::new(EXE)).expect("parse");
        assert_eq!(providers.len(), 2);
        let acme = providers.iter().find(|p| p.id == "acme").unwrap();
        assert_eq!(acme.api_key, ProviderApiKey::Managed);
        assert_eq!(acme.models.len(), 1);
        assert!(acme.models[0].reasoning);
        let plain = providers.iter().find(|p| p.id == "plain").unwrap();
        assert_eq!(plain.api_key, ProviderApiKey::Other);
        // The redacted snapshot must not leak into parsed types anywhere.
        assert_eq!(
            requests.lock().unwrap()[0].argv,
            vec!["config", "get", "models.providers", "--json"]
        );
    }

    #[test]
    fn read_providers_null_is_empty() {
        let (adapter, _) = scripted(vec![Ok(output(0, "null", ""))]);
        assert!(adapter.read_providers(Path::new(EXE)).unwrap().is_empty());
    }

    #[test]
    fn read_providers_malformed_is_read_failed() {
        let (adapter, _) = scripted(vec![Ok(output(0, "providers are fine", ""))]);
        let err = adapter
            .read_providers(Path::new(EXE))
            .expect_err("must fail");
        assert_eq!(err.code, "openclaw-config-read-failed");
    }

    #[test]
    fn read_models_parses_object_and_array_shapes() {
        let body = r#"{"ok":true,"models":[{"provider":"acme","model":"m1","full":"acme/m1","reasoning":true,"supportedReasoningEfforts":["low","high"]}]}"#;
        let (adapter, _) = scripted(vec![Ok(output(0, body, ""))]);
        let rows = adapter.read_models(Path::new(EXE)).expect("parse");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].full, "acme/m1");
        assert!(rows[0].reasoning);
        assert_eq!(
            rows[0].supported_reasoning_efforts.as_deref(),
            Some([ThinkingLevel::Low, ThinkingLevel::High].as_slice())
        );
    }

    #[test]
    fn read_default_model_handles_string_and_object() {
        let (adapter, _) = scripted(vec![
            Ok(output(0, r#""acme/m1""#, "")),
            Ok(output(0, r#"{"primary":"acme/m1","fallbacks":[]}"#, "")),
            Ok(output(0, "null", "")),
        ]);
        let exe = Path::new(EXE);
        assert_eq!(
            adapter.read_default_model(exe).unwrap(),
            Some("acme/m1".into())
        );
        assert_eq!(
            adapter.read_default_model(exe).unwrap(),
            Some("acme/m1".into())
        );
        assert_eq!(adapter.read_default_model(exe).unwrap(), None);
    }

    #[test]
    fn read_thinking_default_parses_known_and_unknown() {
        let (adapter, _) = scripted(vec![
            Ok(output(0, r#""high""#, "")),
            Ok(output(0, r#""future-level""#, "")),
            Ok(output(0, "null", "")),
        ]);
        let exe = Path::new(EXE);
        assert_eq!(
            adapter.read_thinking_default(exe).unwrap(),
            Some(ThinkingLevel::High)
        );
        assert_eq!(adapter.read_thinking_default(exe).unwrap(), None);
        assert_eq!(adapter.read_thinking_default(exe).unwrap(), None);
    }

    #[test]
    fn write_runs_dry_run_then_commit_with_mode_flag() {
        let (adapter, requests) = scripted(vec![
            Ok(output(0, r#"{"ok":true,"operations":1,"errors":[]}"#, "")),
            Ok(output(0, "", "")),
        ]);
        adapter
            .write(
                Path::new(EXE),
                "models.providers.acme",
                r#"{"baseUrl":"https://x.test"}"#,
                WriteMode::Merge,
            )
            .expect("write should succeed");
        let all = requests.lock().unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(
            all[0].argv,
            vec![
                "config",
                "set",
                "models.providers.acme",
                r#"{"baseUrl":"https://x.test"}"#,
                "--strict-json",
                "--merge",
                "--dry-run",
                "--json"
            ]
        );
        assert_eq!(
            all[1].argv,
            vec![
                "config",
                "set",
                "models.providers.acme",
                r#"{"baseUrl":"https://x.test"}"#,
                "--strict-json",
                "--merge"
            ]
        );
    }

    #[test]
    fn write_replace_mode_uses_replace_flag() {
        let (adapter, requests) = scripted(vec![
            Ok(output(0, r#"{"ok":true,"errors":[]}"#, "")),
            Ok(output(0, "", "")),
        ]);
        adapter
            .write(
                Path::new(EXE),
                "models.providers.acme",
                "{}",
                WriteMode::Replace,
            )
            .expect("write");
        let all = requests.lock().unwrap();
        assert!(all[0].argv.contains(&"--replace".to_string()));
        assert!(all[1].argv.contains(&"--replace".to_string()));
        assert!(!all[0].argv.contains(&"--merge".to_string()));
    }

    #[test]
    fn write_dry_run_failure_writes_nothing() {
        let (adapter, requests) = scripted(vec![Ok(output(
            0,
            r#"{"ok":false,"errors":[{"kind":"schema","message":"unknown provider id"}]}"#,
            "",
        ))]);
        let err = adapter
            .write(Path::new(EXE), "models.providers.x", "{}", WriteMode::Plain)
            .expect_err("dry-run reject must be an error");
        assert_eq!(err.code, "openclaw-config-invalid");
        assert!(err.message.contains("unknown provider id"));
        let all = requests.lock().unwrap();
        assert_eq!(all.len(), 1, "no real write after dry-run failure");
        assert!(all[0].argv.contains(&"--dry-run".to_string()));
    }

    #[test]
    fn write_commit_failure_is_write_failed() {
        let (adapter, requests) = scripted(vec![
            Ok(output(0, r#"{"ok":true,"errors":[]}"#, "")),
            Ok(output(1, "", "disk full")),
        ]);
        let err = adapter
            .write(Path::new(EXE), "models.providers.x", "{}", WriteMode::Plain)
            .expect_err("commit failure");
        assert_eq!(err.code, "openclaw-config-write-failed");
        assert!(err.message.contains("disk full"));
        assert_eq!(requests.lock().unwrap().len(), 2);
    }

    #[test]
    fn unset_is_two_step_and_maps_missing_target() {
        let (adapter, requests) = scripted(vec![Ok(output(
            0,
            r#"{"ok":false,"errors":[{"kind":"not-found","message":"path does not exist"}]}"#,
            "",
        ))]);
        let err = adapter
            .unset(Path::new(EXE), "models.providers.gone")
            .expect_err("missing target");
        assert_eq!(err.code, "openclaw-config-read-failed");
        assert_eq!(requests.lock().unwrap().len(), 1);
    }

    #[test]
    fn unset_success_is_two_step() {
        let (adapter, requests) = scripted(vec![
            Ok(output(0, r#"{"ok":true,"errors":[]}"#, "")),
            Ok(output(0, "", "")),
        ]);
        adapter
            .unset(Path::new(EXE), "models.providers.acme")
            .expect("unset");
        let all = requests.lock().unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(
            all[0].argv,
            vec![
                "config",
                "unset",
                "models.providers.acme",
                "--dry-run",
                "--json"
            ]
        );
        assert_eq!(
            all[1].argv,
            vec!["config", "unset", "models.providers.acme"]
        );
    }

    #[test]
    fn set_default_model_nonzero_is_write_failed() {
        let (adapter, requests) = scripted(vec![Ok(output(2, "", "unknown model: acme/nope"))]);
        let err = adapter
            .set_default_model(Path::new(EXE), "acme/nope")
            .expect_err("unknown ref");
        assert_eq!(err.code, "openclaw-config-write-failed");
        assert_eq!(
            requests.lock().unwrap()[0].argv,
            vec!["models", "set", "acme/nope"]
        );
    }

    #[test]
    fn process_errors_map_to_stable_codes() {
        let (adapter, _) = scripted(vec![Err(ProcessError::NotFound {
            executable: EXE.into(),
        })]);
        assert_eq!(
            adapter.read_providers(Path::new(EXE)).unwrap_err().code,
            "openclaw-not-found"
        );
        let (adapter, _) = scripted(vec![Err(ProcessError::Timeout {
            executable: EXE.into(),
        })]);
        assert_eq!(
            adapter.read_models(Path::new(EXE)).unwrap_err().code,
            "process-timeout"
        );
    }
}
