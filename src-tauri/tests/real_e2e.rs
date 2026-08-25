//! Real OpenClaw install E2E — opt-in only (S9).
//!
//! Real system mutation happens ONLY when all three conditions hold:
//!
//! 1. dedicated test target: `cargo test --test real_e2e`
//! 2. cargo feature: `real-e2e`
//! 3. environment: `CLAWDESK_REAL_E2E=1`
//!
//! Run with:
//!
//! ```text
//! CLAWDESK_REAL_E2E=1 cargo test --features real-e2e --test real_e2e
//! ```
//!
//! Without all three, the test self-skips as a non-mutating NOT-RUN.
//!
//! Cleanup ownership: a pre-existing OpenClaw install is never touched or
//! uninstalled; only an installation created by this test run is cleaned up
//! (test teardown only — there is no product uninstall feature in Phase 2).

#[test]
fn real_openclaw_install_flow() {
    if !real_e2e_enabled() {
        eprintln!("real E2E: NOT-RUN (CLAWDESK_REAL_E2E=1 not set)");
        return;
    }
    #[cfg(feature = "real-e2e")]
    {
        real::run();
    }
    #[cfg(not(feature = "real-e2e"))]
    {
        eprintln!("real E2E: NOT-RUN (real-e2e feature not enabled)");
    }
}

/// Phase 3 config CRUD round-trip against the real OpenClaw — same triple
/// gate as the install flow. Only a test-owned provider
/// (`clawdesk-e2e-<timestamp>`) is created and removed; user providers,
/// models, keys, and defaults are never touched.
#[test]
fn real_openclaw_config_flow() {
    if !real_e2e_enabled() {
        eprintln!("real E2E (config): NOT-RUN (CLAWDESK_REAL_E2E=1 not set)");
        return;
    }
    #[cfg(feature = "real-e2e")]
    {
        real::config_flow();
    }
    #[cfg(not(feature = "real-e2e"))]
    {
        eprintln!("real E2E (config): NOT-RUN (real-e2e feature not enabled)");
    }
}

/// Phase 4 skills/plugins flow against the real OpenClaw — same triple gate.
///
/// Read-only for the user's state: only a test-owned skill entry
/// (`clawdesk-e2e-<timestamp>`) is created and removed. No existing skill
/// or plugin is toggled, installed, or updated; `plugins inspect --runtime`
/// is a read-only live probe of one installed plugin.
#[test]
fn real_openclaw_skills_plugins_flow() {
    if !real_e2e_enabled() {
        eprintln!("real E2E (skills/plugins): NOT-RUN (CLAWDESK_REAL_E2E=1 not set)");
        return;
    }
    #[cfg(feature = "real-e2e")]
    {
        real::skills_plugins_flow();
    }
    #[cfg(not(feature = "real-e2e"))]
    {
        eprintln!("real E2E (skills/plugins): NOT-RUN (real-e2e feature not enabled)");
    }
}

/// Phase 5 tools/security flow against the real OpenClaw — same triple gate.
///
/// Read-only except for the single test-owned `tools.profile` round-trip:
/// current value recorded (possibly unset) → set `messaging` → read back →
/// restored (unset when it was unset) → restoration confirmed. No other
/// tool policy field, profile, or security surface is touched.
#[test]
fn real_openclaw_tools_security_flow() {
    if !real_e2e_enabled() {
        eprintln!("real E2E (tools/security): NOT-RUN (CLAWDESK_REAL_E2E=1 not set)");
        return;
    }
    #[cfg(feature = "real-e2e")]
    {
        real::tools_security_flow();
    }
    #[cfg(not(feature = "real-e2e"))]
    {
        eprintln!("real E2E (tools/security): NOT-RUN (real-e2e feature not enabled)");
    }
}

/// Phase 6 channels flow against the real OpenClaw — same triple gate.
///
/// Read-only baseline (list/status/config) plus a Discord token round-trip
/// executed ONLY when the current Discord token state is `absent` (a
/// user-managed token is never touched). No real `plugins install` and no
/// Telegram mutation.
#[test]
fn real_openclaw_channels_flow() {
    if !real_e2e_enabled() {
        eprintln!("real E2E (channels): NOT-RUN (CLAWDESK_REAL_E2E=1 not set)");
        return;
    }
    #[cfg(feature = "real-e2e")]
    {
        real::channels_flow();
    }
    #[cfg(not(feature = "real-e2e"))]
    {
        eprintln!("real E2E (channels): NOT-RUN (real-e2e feature not enabled)");
    }
}

fn real_e2e_enabled() -> bool {
    std::env::var("CLAWDESK_REAL_E2E").is_ok_and(|value| value == "1")
}

#[cfg(feature = "real-e2e")]
mod real {
    use std::path::Path;
    use std::sync::Arc;
    use std::time::Duration;

    use clawdesk_lib::application::{
        ChannelService, ChannelTokenService, EnvironmentReport, EnvironmentService, InstallResult,
        InstallService, ModelInput, ModelService, ProviderInput,
    };
    use clawdesk_lib::domain::models::channels::ChannelTokenState;
    use clawdesk_lib::domain::models::OpenClawStatus;
    use clawdesk_lib::domain::ports::process::{ProcessPort, ProcessRequest};
    use clawdesk_lib::domain::ports::{OpenClawInstallerPort, WindowsSystemPort};
    use clawdesk_lib::infrastructure::openclaw::OpenClawInstaller;
    use clawdesk_lib::infrastructure::process::ProcessRunner;
    use clawdesk_lib::infrastructure::windows::WindowsSystemAdapter;

    pub fn run() {
        let environment = EnvironmentService::production();
        let pre_report = environment
            .detect_environment()
            .expect("pre-install environment detection");
        let pre_installed = !matches!(pre_report.openclaw, OpenClawStatus::NotFound);

        // Node/npm preconditions are exercised implicitly by the install
        // service (structured errors otherwise).
        let service = InstallService::production();
        let result = service
            .install_openclaw()
            .expect("real OpenClaw install flow should succeed");

        let version = match &result {
            InstallResult::AlreadyInstalled { version } | InstallResult::Installed { version } => {
                version.clone()
            }
        };
        assert!(!version.is_empty(), "install must report a version");

        // Post-install verification via the Phase 1 detect/version features.
        let post_report: EnvironmentReport = environment
            .detect_environment()
            .expect("post-install environment detection");
        match post_report.openclaw {
            OpenClawStatus::Detected {
                version: Some(post_version),
                ..
            } => assert_eq!(post_version, version, "installed version must verify"),
            other => panic!("OpenClaw should be detected with a version, got {other:?}"),
        }

        // Cleanup: only a test-owned installation (created by this run) is
        // uninstalled. A pre-existing install is never touched.
        let test_owned = !pre_installed && matches!(result, InstallResult::Installed { .. });
        if test_owned {
            match cleanup_uninstall() {
                Ok(()) => eprintln!("real E2E: cleaned up test-owned installation"),
                Err(err) => eprintln!("real E2E: cleanup uninstall failed: {err}"),
            }
        } else {
            eprintln!("real E2E: pre-existing install untouched (no cleanup)");
        }
    }

    /// Phase 3: config schema baseline + test-owned provider round-trip.
    ///
    /// Read-only for everything except the single test-owned provider
    /// (`clawdesk-e2e-<timestamp>`), which is created and then deleted.
    /// User providers/models/keys/defaults are never modified.
    pub fn config_flow() {
        let environment = EnvironmentService::production();
        let report = environment
            .detect_environment()
            .expect("environment detection for config E2E");
        let OpenClawStatus::Detected {
            executable,
            version,
            ..
        } = &report.openclaw
        else {
            eprintln!("real E2E (config): NOT-RUN (OpenClaw not detected)");
            return;
        };
        eprintln!(
            "real E2E (config): OpenClaw detected (version: {:?})",
            version
        );

        // Field baseline: `openclaw config schema` is the code ground truth
        // for config fields (especially the `secrets.providers.*.command`
        // path format on Windows). Informational: a CLI shape change must
        // not fail the round-trip, so failures are reported, not asserted.
        schema_baseline(Path::new(executable));

        // Test-owned provider round-trip: save → read → delete.
        let service = ModelService::production();
        let provider_id = format!(
            "clawdesk-e2e-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
        );
        let input = ProviderInput {
            id: provider_id.clone(),
            base_url: Some("https://clawdesk-e2e.invalid/v1".to_string()),
            api: "openai-completions".to_string(),
            models: vec![ModelInput {
                id: "e2e-model".to_string(),
                name: Some("E2E model".to_string()),
                reasoning: true,
                input: vec!["text".to_string()],
                context_window: Some(128000),
                max_tokens: None,
                supports_reasoning_effort: false,
                supported_reasoning_efforts: None,
            }],
        };
        service
            .save_provider(&input)
            .unwrap_or_else(|err| panic!("test-owned provider save failed: {err}"));

        let saved = service
            .get_provider(&provider_id)
            .unwrap_or_else(|err| panic!("test-owned provider read failed: {err}"));
        assert!(
            saved.models.iter().any(|model| model.id == "e2e-model"),
            "saved provider must contain the e2e model"
        );

        service
            .delete_provider(&provider_id)
            .unwrap_or_else(|err| panic!("test-owned provider delete failed: {err}"));
        assert!(
            service
                .list_providers()
                .expect("list after delete")
                .iter()
                .all(|summary| summary.id != provider_id),
            "test-owned provider must be gone after delete"
        );
        eprintln!("real E2E (config): test-owned provider round-trip OK");
    }

    /// Phase 4: skills/plugins row schema baseline + a test-owned skill
    /// entry round-trip + a read-only runtime inspect of one installed
    /// plugin.
    ///
    /// Only the test-owned entry `skills.entries.clawdesk-e2e-<timestamp>`
    /// is created and removed. Existing skills/plugins are never modified.
    pub fn skills_plugins_flow() {
        use clawdesk_lib::domain::ports::openclaw_config::{OpenClawConfigPort, WriteMode};
        use clawdesk_lib::domain::ports::openclaw_plugins::OpenClawPluginsPort;
        use clawdesk_lib::domain::ports::openclaw_skills::OpenClawSkillsPort;
        use clawdesk_lib::infrastructure::openclaw::{
            OpenClawConfigAdapter, OpenClawPluginsAdapter, OpenClawSkillsAdapter,
        };

        let environment = EnvironmentService::production();
        let report = environment
            .detect_environment()
            .expect("environment detection for skills/plugins E2E");
        let OpenClawStatus::Detected {
            executable,
            version,
            ..
        } = &report.openclaw
        else {
            eprintln!("real E2E (skills/plugins): NOT-RUN (OpenClaw not detected)");
            return;
        };
        eprintln!(
            "real E2E (skills/plugins): OpenClaw detected (version: {:?})",
            version
        );

        let exe: &Path = Path::new(executable);
        let skills = OpenClawSkillsAdapter::new(Arc::new(ProcessRunner));
        let plugins = OpenClawPluginsAdapter::new(Arc::new(ProcessRunner));
        let config = OpenClawConfigAdapter::new(Arc::new(ProcessRunner));

        // Row schema baselines (informational — a live schema change must
        // not fail the flow, so failures are reported, not asserted).
        match skills.list_skills(exe) {
            Ok(rows) => eprintln!(
                "real E2E (skills): skills list parsed rows={} first={:?}",
                rows.len(),
                rows.first().map(|row| &row.name)
            ),
            Err(err) => eprintln!("real E2E (skills): skills list NOT-VERIFIED ({})", err),
        }
        match plugins.list_plugins(exe) {
            Ok(rows) => eprintln!(
                "real E2E (plugins): plugins list parsed rows={} first={:?}",
                rows.len(),
                rows.first().map(|row| &row.id)
            ),
            Err(err) => eprintln!("real E2E (plugins): plugins list NOT-VERIFIED ({})", err),
        }

        // Test-owned skill entry round-trip: set → verify → unset.
        let entry = format!(
            "clawdesk-e2e-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
        );
        let leaf = format!("skills.entries.{entry}.enabled");
        config
            .write(exe, &leaf, "true", WriteMode::Plain)
            .unwrap_or_else(|err| panic!("test-owned skill entry set failed: {err}"));
        let read_back = config
            .read_raw(exe, &leaf)
            .expect("test-owned skill entry read")
            .unwrap_or_else(|| panic!("test-owned skill entry missing after set"));
        assert!(
            read_back == "true",
            "test-owned skill entry must read back as true, got {read_back}"
        );
        config
            .unset(exe, &leaf)
            .unwrap_or_else(|err| panic!("test-owned skill entry unset failed: {err}"));
        // The entry object may remain empty or be cleaned up by the CLI —
        // remove it explicitly when it still exists.
        if config
            .read_raw(exe, &format!("skills.entries.{entry}"))
            .expect("test-owned entry read after unset")
            .is_some()
        {
            config
                .unset(exe, &format!("skills.entries.{entry}"))
                .unwrap_or_else(|err| panic!("test-owned entry object unset failed: {err}"));
        }
        assert!(
            config
                .read_raw(exe, &format!("skills.entries.{entry}"))
                .expect("final test-owned entry read")
                .is_none(),
            "test-owned skill entry must be gone after cleanup"
        );
        eprintln!("real E2E (skills): test-owned entry round-trip OK");

        // Read-only runtime inspect of one installed plugin (state 0 change).
        match plugins.list_plugins(exe) {
            Ok(rows) if !rows.is_empty() => {
                let target = rows[0].id.clone();
                match plugins.inspect_plugin_runtime(exe, &target) {
                    Ok(runtime) => eprintln!(
                        "real E2E (plugins): runtime inspect OK id={} tools={} hooks={} services={} cliCommands={} gatewayMethods={} routes={}",
                        runtime.id,
                        runtime.tools.len(),
                        runtime.hooks.len(),
                        runtime.services.len(),
                        runtime.cli_commands.len(),
                        runtime.gateway_methods.len(),
                        runtime.routes.len()
                    ),
                    Err(err) => eprintln!(
                        "real E2E (plugins): runtime inspect NOT-VERIFIED ({err})"
                    ),
                }
            }
            Ok(_) => {
                eprintln!("real E2E (plugins): no installed plugins (inspect NOT-VERIFIED)");
            }
            Err(err) => {
                eprintln!("real E2E (plugins): plugins list NOT-VERIFIED ({err}); inspect skipped")
            }
        }
    }

    /// Phase 5: cold audit read-only baseline + the test-owned
    /// `tools.profile` round-trip (restoration guaranteed).
    pub fn tools_security_flow() {
        use clawdesk_lib::domain::models::tools::SecurityAuditResult;
        use clawdesk_lib::domain::ports::openclaw_config::{OpenClawConfigPort, WriteMode};
        use clawdesk_lib::domain::ports::openclaw_security::OpenClawSecurityPort;
        use clawdesk_lib::infrastructure::openclaw::{
            OpenClawConfigAdapter, OpenClawSecurityAdapter,
        };

        let environment = EnvironmentService::production();
        let report = environment
            .detect_environment()
            .expect("environment detection for tools/security E2E");
        let OpenClawStatus::Detected {
            executable,
            version,
            ..
        } = &report.openclaw
        else {
            eprintln!("real E2E (tools/security): NOT-RUN (OpenClaw not detected)");
            return;
        };
        eprintln!(
            "real E2E (tools/security): OpenClaw detected (version: {:?})",
            version
        );

        let exe: &Path = Path::new(executable);
        let security = OpenClawSecurityAdapter::new(Arc::new(ProcessRunner));
        let config = OpenClawConfigAdapter::new(Arc::new(ProcessRunner));

        // Read-only cold audit: the findings row schema baseline (live
        // shape changes are reported, not asserted — contract §7).
        match security.run_security_audit(exe) {
            Ok(SecurityAuditResult {
                findings,
                suppressed_count,
                ..
            }) => {
                eprintln!(
                    "real E2E (tools/security): audit OK findings={} first={:?} suppressed={}",
                    findings.len(),
                    findings.first().map(|f| &f.check_id),
                    suppressed_count
                );
            }
            Err(err) => {
                eprintln!("real E2E (tools/security): audit NOT-VERIFIED ({err})")
            }
        }

        // Test-owned `tools.profile` round-trip. If the current value
        // cannot be read, the round-trip is skipped (no mutation on an
        // unknown original).
        let original = match config.read_raw(exe, "tools.profile") {
            Ok(value) => value,
            Err(err) => {
                eprintln!(
                    "real E2E (tools/security): tools.profile round-trip NOT-RUN (read failed: {err})"
                );
                return;
            }
        };
        match &original {
            Some(text) => eprintln!("real E2E (tools/security): original tools.profile={text}"),
            None => eprintln!("real E2E (tools/security): original tools.profile=<unset>"),
        }
        config
            .write(exe, "tools.profile", "\"messaging\"", WriteMode::Plain)
            .unwrap_or_else(|err| panic!("tools.profile round-trip set failed: {err}"));
        let read_back = config
            .read_raw(exe, "tools.profile")
            .expect("read back tools.profile");
        assert_eq!(
            read_back.as_deref(),
            Some("\"messaging\""),
            "the set value must read back as \"messaging\""
        );
        match &original {
            Some(text) => config
                .write(exe, "tools.profile", text, WriteMode::Plain)
                .unwrap_or_else(|err| panic!("tools.profile restore failed: {err}")),
            None => config
                .unset(exe, "tools.profile")
                .unwrap_or_else(|err| panic!("tools.profile unset failed: {err}")),
        }
        let restored = config
            .read_raw(exe, "tools.profile")
            .expect("final tools.profile read");
        assert_eq!(
            restored, original,
            "tools.profile must be restored to its original value"
        );
        eprintln!("real E2E (tools/security): tools.profile round-trip OK");
    }

    /// Phase 6: channels read-only baseline (list/status/config) + a Discord
    /// token round-trip executed only when the current Discord token state
    /// is `absent`. No real `plugins install`, no Telegram mutation, and a
    /// user-managed token is never touched.
    pub fn channels_flow() {
        let environment = EnvironmentService::production();
        let report = environment
            .detect_environment()
            .expect("environment detection for channels E2E");
        let OpenClawStatus::Detected { version, .. } = &report.openclaw else {
            eprintln!("real E2E (channels): NOT-RUN (OpenClaw not detected)");
            return;
        };
        eprintln!(
            "real E2E (channels): OpenClaw detected (version: {:?})",
            version
        );

        let channels = ChannelService::production();
        let tokens = ChannelTokenService::production();

        // Read-only baselines (informational — a live CLI shape change must
        // not fail the flow, so failures are reported, not asserted).
        match channels.get_channels() {
            Ok(overview) => eprintln!(
                "real E2E (channels): overview gatewayReachable={} channels={:?}",
                overview.gateway_reachable, overview.channels
            ),
            Err(err) => eprintln!("real E2E (channels): overview NOT-VERIFIED ({err})"),
        }
        for channel in ["discord", "telegram"] {
            match channels.get_channel_config(channel) {
                Ok(config) => eprintln!(
                    "real E2E (channels): config {channel} tokenState={:?} dmPolicy={:?} groupPolicy={:?}",
                    config.token_state, config.dm_policy, config.group_policy
                ),
                Err(err) => eprintln!(
                    "real E2E (channels): config {channel} NOT-VERIFIED ({err})"
                ),
            }
        }

        // Discord token round-trip: only when the token state is absent.
        let state = match channels.get_channel_config("discord") {
            Ok(config) => config.token_state,
            Err(err) => {
                eprintln!(
                    "real E2E (channels): token round-trip NOT-RUN (discord config read failed: {err})"
                );
                return;
            }
        };
        if state != ChannelTokenState::Absent {
            eprintln!(
                "real E2E (channels): token round-trip NOT-RUN (discord token state is {state:?})"
            );
            return;
        }
        let test_token = "clawdesk-real-e2e-discord-4242424242";
        tokens
            .set_channel_token("discord", test_token)
            .unwrap_or_else(|err| panic!("discord token set failed: {err}"));
        let after_set = channels
            .get_channel_config("discord")
            .expect("discord config read after set");
        assert_eq!(
            after_set.token_state,
            ChannelTokenState::Managed,
            "discord token must classify as managed after set"
        );
        tokens
            .delete_channel_token("discord")
            .unwrap_or_else(|err| panic!("discord token delete failed: {err}"));
        let after_delete = channels
            .get_channel_config("discord")
            .expect("discord config read after delete");
        assert_eq!(
            after_delete.token_state,
            ChannelTokenState::Absent,
            "discord token must be absent after delete"
        );
        eprintln!("real E2E (channels): discord token round-trip OK");
    }

    /// Prints the config-schema baseline observations (informational).
    fn schema_baseline(executable: &Path) {
        let process: Arc<dyn ProcessPort> = Arc::new(ProcessRunner);
        for argv in [
            vec![
                "config".to_string(),
                "schema".to_string(),
                "--json".to_string(),
            ],
            vec!["config".to_string(), "schema".to_string()],
        ] {
            let output = match process.run(&ProcessRequest::new(
                executable.to_path_buf(),
                argv,
                Duration::from_secs(30),
            )) {
                Ok(output) if output.exit_code == 0 => output,
                _ => continue,
            };
            let body = output.stdout.trim();
            if body.is_empty() {
                continue;
            }
            let parsed_ok = serde_json::from_str::<serde_json::Value>(body).is_ok();
            eprintln!("real E2E (config): schema baseline parsed_json={parsed_ok}");
            eprintln!(
                "real E2E (config): schema mentions secrets.providers={}",
                body.contains("secrets.providers")
            );
            eprintln!(
                "real E2E (config): schema mentions models.providers={}",
                body.contains("models.providers")
            );
            eprintln!(
                "real E2E (config): schema mentions agents.defaults.thinkingDefault={}",
                body.contains("agents.defaults.thinkingDefault")
            );
            return;
        }
        eprintln!("real E2E (config): schema baseline NOT-VERIFIED (command unsupported)");
    }

    /// Test-teardown-only uninstall of the test-owned installation, executed
    /// as structured `node npm-cli.js uninstall -g openclaw` through the
    /// single ProcessRunner boundary.
    fn cleanup_uninstall() -> Result<(), String> {
        let process: Arc<dyn ProcessPort> = Arc::new(ProcessRunner);
        let windows = WindowsSystemAdapter::new(Arc::clone(&process));
        let node_exe = windows.node_executable().map_err(|err| err.to_string())?;
        let installer = OpenClawInstaller::production(Arc::clone(&process));
        let npm = installer
            .resolve_npm_entry(&node_exe)
            .map_err(|err| err.to_string())?;
        let request = ProcessRequest::new(
            npm.node,
            vec![
                npm.npm_cli.to_string_lossy().into_owned(),
                "uninstall".to_string(),
                "-g".to_string(),
                "openclaw".to_string(),
            ],
            Duration::from_secs(5 * 60),
        );
        let output = process.run(&request).map_err(|err| err.to_string())?;
        if output.exit_code == 0 {
            Ok(())
        } else {
            Err(format!(
                "uninstall exit code {}: {}",
                output.exit_code,
                output.stderr.trim()
            ))
        }
    }
}
