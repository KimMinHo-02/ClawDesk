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

fn real_e2e_enabled() -> bool {
    std::env::var("CLAWDESK_REAL_E2E").is_ok_and(|value| value == "1")
}

#[cfg(feature = "real-e2e")]
mod real {
    use std::path::Path;
    use std::sync::Arc;
    use std::time::Duration;

    use clawdesk_lib::application::{
        EnvironmentReport, EnvironmentService, InstallResult, InstallService, ModelInput,
        ModelService, ProviderInput,
    };
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
