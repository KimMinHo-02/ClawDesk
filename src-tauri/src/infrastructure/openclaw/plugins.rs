//! `OpenClawPluginsAdapter` — non-interactive `openclaw plugins` CLI
//! invocations (Phase 4).
//!
//! Every invocation is a structured `executable + argv` request through the
//! `ProcessPort` (S1/S2). The plugin id is a single argv element (never
//! interpolated into a shell or config path). Row parsing is fail-soft:
//! optional fields become `null` / empty arrays (contract §2).

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use crate::domain::models::skills::{PluginRow, PluginRuntime};
use crate::domain::ports::openclaw_plugins::OpenClawPluginsPort;
use crate::domain::ports::process::{ProcessError, ProcessOutput, ProcessPort, ProcessRequest};
use crate::error::AppError;

/// Contract timeout for plugins list/enable/disable (Phase 4: 30s).
const PLUGINS_TIMEOUT: Duration = Duration::from_secs(30);

/// Runtime inspect loads plugin modules, so it gets a longer timeout
/// (Phase 4: 60s).
const RUNTIME_INSPECT_TIMEOUT: Duration = Duration::from_secs(60);

/// OpenClaw plugins adapter. All CLI invocations go through the `ProcessPort`.
pub struct OpenClawPluginsAdapter {
    process: Arc<dyn ProcessPort>,
}

impl OpenClawPluginsAdapter {
    pub fn new(process: Arc<dyn ProcessPort>) -> Self {
        Self { process }
    }

    fn run_cli(
        &self,
        executable: &Path,
        argv: Vec<String>,
        label: &str,
        timeout: Duration,
    ) -> Result<ProcessOutput, AppError> {
        let request = ProcessRequest::new(executable.to_path_buf(), argv, timeout);
        match self.process.run(&request) {
            Ok(output) => Ok(output),
            Err(ProcessError::NotFound { .. }) => Err(AppError::openclaw_not_found()),
            Err(ProcessError::Timeout { .. }) => Err(AppError::process_timeout(label)),
            Err(ProcessError::SpawnFailed { message }) => {
                Err(AppError::process_failed(label, message))
            }
        }
    }

    fn parse_json(
        output: &ProcessOutput,
        label: &str,
        read_failed: bool,
    ) -> Result<serde_json::Value, AppError> {
        if output.exit_code != 0 {
            let detail = format!(
                "{label}: exit code {}: {}",
                output.exit_code,
                output.stderr.trim()
            );
            return if read_failed {
                Err(AppError::openclaw_plugins_read_failed(detail))
            } else {
                Err(AppError::openclaw_plugin_toggle_failed("plugin", detail))
            };
        }
        serde_json::from_str::<serde_json::Value>(&output.stdout).map_err(|err| {
            if read_failed {
                AppError::openclaw_plugins_read_failed(format!(
                    "{label}: unparseable output: {} ({err})",
                    output.stdout.trim()
                ))
            } else {
                AppError::openclaw_plugin_toggle_failed(
                    "plugin",
                    format!(
                        "{label}: unparseable output: {} ({err})",
                        output.stdout.trim()
                    ),
                )
            }
        })
    }
}

impl OpenClawPluginsPort for OpenClawPluginsAdapter {
    fn list_plugins(&self, executable: &Path) -> Result<Vec<PluginRow>, AppError> {
        const LABEL: &str = "openclaw plugins list --json";
        let output = self.run_cli(
            executable,
            vec!["plugins".into(), "list".into(), "--json".into()],
            LABEL,
            PLUGINS_TIMEOUT,
        )?;
        let value = Self::parse_json(&output, LABEL, true)?;
        // Rows live at the top level (array) or under a `plugins` key
        // (object envelope) — accept both (fail-soft).
        let rows_value = match &value {
            serde_json::Value::Array(_) => value,
            serde_json::Value::Object(map) => map
                .get("plugins")
                .cloned()
                .unwrap_or(serde_json::Value::Array(vec![])),
            other => {
                return Err(AppError::openclaw_plugins_read_failed(format!(
                    "{LABEL}: expected array/object, got {}",
                    other
                )))
            }
        };
        let rows = match &rows_value {
            serde_json::Value::Array(items) => items,
            other => {
                return Err(AppError::openclaw_plugins_read_failed(format!(
                    "{LABEL}: expected a row array, got {}",
                    other
                )))
            }
        };
        Ok(rows.iter().filter_map(parse_plugin_row).collect())
    }

    fn enable_plugin(&self, executable: &Path, id: &str) -> Result<(), AppError> {
        const LABEL: &str = "openclaw plugins enable";
        let output = self.run_cli(
            executable,
            vec!["plugins".into(), "enable".into(), id.to_string()],
            LABEL,
            PLUGINS_TIMEOUT,
        )?;
        if output.exit_code != 0 {
            return Err(AppError::openclaw_plugin_toggle_failed(
                id,
                format!("exit code {}: {}", output.exit_code, output.stderr.trim()),
            ));
        }
        Ok(())
    }

    fn disable_plugin(&self, executable: &Path, id: &str) -> Result<(), AppError> {
        const LABEL: &str = "openclaw plugins disable";
        let output = self.run_cli(
            executable,
            vec!["plugins".into(), "disable".into(), id.to_string()],
            LABEL,
            PLUGINS_TIMEOUT,
        )?;
        if output.exit_code != 0 {
            return Err(AppError::openclaw_plugin_toggle_failed(
                id,
                format!("exit code {}: {}", output.exit_code, output.stderr.trim()),
            ));
        }
        Ok(())
    }

    fn inspect_plugin_runtime(
        &self,
        executable: &Path,
        id: &str,
    ) -> Result<PluginRuntime, AppError> {
        const LABEL: &str = "openclaw plugins inspect --runtime --json";
        let output = self.run_cli(
            executable,
            vec![
                "plugins".into(),
                "inspect".into(),
                id.to_string(),
                "--runtime".into(),
                "--json".into(),
            ],
            LABEL,
            RUNTIME_INSPECT_TIMEOUT,
        )?;
        let value = Self::parse_json(&output, LABEL, true)?;
        let id = value
            .get("id")
            .or_else(|| value.get("pluginId"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                AppError::openclaw_plugins_read_failed(format!("{LABEL}: no plugin id"))
            })?
            .to_string();
        // Surface names may live at the top level or under a `runtime`
        // object; the live schema is fixture-confirmed per contract.
        let surface =
            |key: &str| -> Vec<String> { parse_surface(&value[key], &value["runtime"][key]) };
        let tools = surface("tools");
        let hooks = surface("hooks");
        let services = surface("services");
        let cli_commands = surface("cliCommands");
        let gateway_methods = surface("gatewayMethods");
        let routes = surface("routes");
        let diagnostics = match &value["diagnostics"] {
            serde_json::Value::Array(items) => {
                let parsed: Vec<String> = items
                    .iter()
                    .filter_map(|item| {
                        item.as_str()
                            .or_else(|| item.get("message").and_then(|m| m.as_str()))
                            .map(str::to_string)
                    })
                    .collect();
                if parsed.is_empty() && !items.is_empty() {
                    None
                } else {
                    Some(parsed)
                }
            }
            serde_json::Value::String(message) => Some(vec![message.clone()]),
            serde_json::Value::Null => None,
            other => {
                return Err(AppError::openclaw_plugins_read_failed(format!(
                    "{LABEL}: unexpected diagnostics shape {}",
                    other
                )))
            }
        };
        Ok(PluginRuntime {
            id,
            tools,
            hooks,
            services,
            cli_commands,
            gateway_methods,
            routes,
            diagnostics,
        })
    }
}

/// Parses one plugin row (fail-soft: only `id` is required).
fn parse_plugin_row(raw: &serde_json::Value) -> Option<PluginRow> {
    let id = raw.get("id")?.as_str()?.to_string();
    if id.is_empty() {
        return None;
    }
    let enabled = raw.get("enabled").and_then(|v| v.as_bool());
    let name = raw.get("name").and_then(|v| v.as_str()).map(str::to_string);
    let format = raw
        .get("format")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    // The origin key name is not fixed by the docs; accept both.
    let origin = raw
        .get("origin")
        .or_else(|| raw.get("source"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let version = raw
        .get("version")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let dependency_status = raw
        .get("dependencyStatus")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    Some(PluginRow {
        id,
        enabled,
        name,
        format,
        origin,
        version,
        dependency_status,
    })
}

/// Parses one registered-surface array: missing/null → empty; elements are
/// coerced to names (string as-is, object → `name`/`id` field, else skipped).
fn parse_surface(top: &serde_json::Value, nested: &serde_json::Value) -> Vec<String> {
    let value = if top.is_null() { nested } else { top };
    let items = match value.as_array() {
        Some(items) => items,
        None => return Vec::new(),
    };
    items
        .iter()
        .filter_map(|item| {
            if let Some(name) = item.as_str() {
                Some(name.to_string())
            } else {
                item.get("name")
                    .or_else(|| item.get("id"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            }
        })
        .collect()
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
    ) -> (OpenClawPluginsAdapter, Arc<Mutex<Vec<ProcessRequest>>>) {
        let requests: Arc<Mutex<Vec<ProcessRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let fake = Arc::new(ScriptedProcess {
            responses: Arc::new(Mutex::new(responses)),
            requests: Arc::clone(&requests),
        });
        (OpenClawPluginsAdapter::new(fake), requests)
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
    fn list_plugins_exact_argv_and_rows() {
        let body = r#"{"ok":true,"plugins":[
            {"id":"@openclaw/discord","enabled":true,"name":"Discord","format":"module","version":"1.2.3","dependencyStatus":"ok"},
            {"id":"local-plugin","enabled":false},
            {"id":"minimal"}
        ]}"#;
        let (adapter, requests) = scripted(vec![Ok(output(0, body, ""))]);
        let rows = adapter.list_plugins(Path::new(EXE)).expect("parse");
        assert_eq!(rows.len(), 3, "no row may be dropped");
        assert_eq!(rows[0].id, "@openclaw/discord");
        assert_eq!(rows[0].enabled, Some(true));
        assert_eq!(rows[0].name.as_deref(), Some("Discord"));
        assert_eq!(rows[0].format.as_deref(), Some("module"));
        assert_eq!(rows[0].version.as_deref(), Some("1.2.3"));
        assert_eq!(rows[0].dependency_status.as_deref(), Some("ok"));
        assert_eq!(rows[1].enabled, Some(false));
        assert_eq!(rows[1].name, None);
        // Fail-soft: a row with only `id` is kept, the rest is null.
        assert_eq!(rows[2].id, "minimal");
        assert_eq!(rows[2].enabled, None);
        let argv = requests.lock().unwrap()[0].argv.clone();
        assert_eq!(argv, vec!["plugins", "list", "--json"]);
    }

    #[test]
    fn list_plugins_accepts_bare_array_shape() {
        let body = r#"[{"id":"a","enabled":true}]"#;
        let (adapter, _) = scripted(vec![Ok(output(0, body, ""))]);
        let rows = adapter.list_plugins(Path::new(EXE)).expect("parse");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "a");
    }

    #[test]
    fn list_plugins_malformed_json_is_read_failed() {
        let (adapter, _) = scripted(vec![Ok(output(0, "plugins are fine", ""))]);
        let err = adapter.list_plugins(Path::new(EXE)).expect_err("must fail");
        assert_eq!(err.code, "openclaw-plugins-read-failed");
    }

    #[test]
    fn enable_plugin_exact_single_argv() {
        let (adapter, requests) = scripted(vec![Ok(output(0, "", ""))]);
        adapter
            .enable_plugin(Path::new(EXE), "@openclaw/discord")
            .expect("enable");
        let argv = requests.lock().unwrap()[0].argv.clone();
        assert_eq!(argv, vec!["plugins", "enable", "@openclaw/discord"]);
    }

    #[test]
    fn disable_plugin_exact_single_argv() {
        let (adapter, requests) = scripted(vec![Ok(output(0, "", ""))]);
        adapter
            .disable_plugin(Path::new(EXE), "local-plugin")
            .expect("disable");
        let argv = requests.lock().unwrap()[0].argv.clone();
        assert_eq!(argv, vec!["plugins", "disable", "local-plugin"]);
    }

    #[test]
    fn enable_plugin_nonzero_is_toggle_failed() {
        let (adapter, _) = scripted(vec![Ok(output(2, "", "unknown plugin: nope"))]);
        let err = adapter
            .enable_plugin(Path::new(EXE), "nope")
            .expect_err("unknown id");
        assert_eq!(err.code, "openclaw-plugin-toggle-failed");
        assert!(err.message.contains("unknown plugin: nope"));
    }

    #[test]
    fn disable_plugin_nonzero_is_toggle_failed() {
        let (adapter, _) = scripted(vec![Ok(output(1, "", "nix mode: rejected"))]);
        let err = adapter
            .disable_plugin(Path::new(EXE), "nope")
            .expect_err("nix rejection");
        assert_eq!(err.code, "openclaw-plugin-toggle-failed");
    }

    #[test]
    fn inspect_runtime_exact_argv_and_surfaces() {
        let body = r#"{"ok":true,"id":"@openclaw/discord","runtime":{
            "tools":["discord_send","discord_read"],
            "hooks":[{"name":"on-message"}],
            "services":"disc",
            "cliCommands":[],
            "gatewayMethods":["discord.connect"],
            "routes":["/discord/events"]
        },
        "diagnostics":["loaded in 12ms"]}"#;
        let (adapter, requests) = scripted(vec![Ok(output(0, body, ""))]);
        let runtime = adapter
            .inspect_plugin_runtime(Path::new(EXE), "@openclaw/discord")
            .expect("inspect");
        assert_eq!(runtime.id, "@openclaw/discord");
        assert_eq!(runtime.tools, vec!["discord_send", "discord_read"]);
        assert_eq!(runtime.hooks, vec!["on-message"], "objects coerce to name");
        assert!(
            runtime.services.is_empty(),
            "non-array surface degrades to empty (fail-soft)"
        );
        assert!(runtime.cli_commands.is_empty());
        assert_eq!(runtime.gateway_methods, vec!["discord.connect"]);
        assert_eq!(runtime.routes, vec!["/discord/events"]);
        assert_eq!(
            runtime.diagnostics.as_deref(),
            Some(vec!["loaded in 12ms".to_string()].as_slice())
        );
        let argv = requests.lock().unwrap()[0].argv.clone();
        assert_eq!(
            argv,
            vec![
                "plugins",
                "inspect",
                "@openclaw/discord",
                "--runtime",
                "--json"
            ]
        );
        // The longer 60s contract timeout applies to runtime inspect.
        assert_eq!(requests.lock().unwrap()[0].timeout, Duration::from_secs(60));
    }

    #[test]
    fn inspect_runtime_missing_surfaces_default_to_empty() {
        let body = r#"{"ok":true,"id":"minimal-plugin"}"#;
        let (adapter, _) = scripted(vec![Ok(output(0, body, ""))]);
        let runtime = adapter
            .inspect_plugin_runtime(Path::new(EXE), "minimal-plugin")
            .expect("inspect");
        assert!(runtime.tools.is_empty());
        assert!(runtime.diagnostics.is_none());
    }

    #[test]
    fn inspect_runtime_nonzero_is_read_failed() {
        let (adapter, _) = scripted(vec![Ok(output(2, "", "unknown plugin: nope"))]);
        let err = adapter
            .inspect_plugin_runtime(Path::new(EXE), "nope")
            .expect_err("unknown id");
        assert_eq!(err.code, "openclaw-plugins-read-failed");
    }

    #[test]
    fn inspect_runtime_timeout_is_process_timeout() {
        let (adapter, _) = scripted(vec![Err(ProcessError::Timeout {
            executable: EXE.into(),
        })]);
        let err = adapter
            .inspect_plugin_runtime(Path::new(EXE), "nope")
            .expect_err("timeout");
        assert_eq!(err.code, "process-timeout");
    }

    #[test]
    fn process_errors_map_to_stable_codes() {
        let (adapter, _) = scripted(vec![Err(ProcessError::NotFound {
            executable: EXE.into(),
        })]);
        assert_eq!(
            adapter.list_plugins(Path::new(EXE)).unwrap_err().code,
            "openclaw-not-found"
        );
    }
}
