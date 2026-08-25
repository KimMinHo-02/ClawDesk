//! Channels use case (Phase 6).
//!
//! Orchestration over the channels/plugin-install/config ports:
//! - `get_channels`: `channels list --all --json` + `channels status --json`
//!   (both read-only) merged into discord/telegram rows (absent row →
//!   `installed:false, configured:false` — fail-soft, no "connected" guess
//!   when `gatewayReachable` is false).
//! - `connect_channel`: token ref precondition (ClawDesk-managed, otherwise
//!   `channel-token-not-found` with zero mutation CLI calls) → for Discord,
//!   ensure `@openclaw/discord` is installed (idempotent: `plugins list` →
//!   `plugins install` → `plugins list` post-check) → `enabled=true` write.
//!   Fixed order, first failure stops (no partial connect).
//! - policy writes: pre-validation (S2, 0 process runs on failure) then
//!   the fixed 2-path `set_dm_access` order (dmPolicy → allowFrom).
//!
//! No optimistic updates: every finished mutation (success OR failure) is
//! followed by a UI re-query of the actual state.

use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use super::environment::default_openclaw_search_dirs;
use crate::domain::models::channels::{
    channel_allow_from_path, channel_dm_policy_path, channel_enabled_path,
    channel_group_policy_path, channel_section_path, channel_token_path,
    classify_channel_token_state, validate_allow_from_entry, validate_channel_id,
    validate_dm_access, validate_dm_policy, validate_group_policy, validate_pairing_code,
    ChannelConfig, ChannelSummary, ChannelTokenState, ChannelsOverview, PairingRequest,
    DISCORD_PLUGIN_ID, SUPPORTED_CHANNELS,
};
use crate::domain::models::skills::PluginRow;
use crate::domain::ports::openclaw::OpenClawPort;
use crate::domain::ports::openclaw_channels::OpenClawChannelsPort;
use crate::domain::ports::openclaw_config::{OpenClawConfigPort, WriteMode};
use crate::domain::ports::openclaw_plugin_install::OpenClawPluginInstallPort;
use crate::domain::ports::openclaw_plugins::OpenClawPluginsPort;
use crate::error::AppError;
use crate::infrastructure::openclaw::{
    OpenClawAdapter, OpenClawChannelsAdapter, OpenClawConfigAdapter, OpenClawPluginInstallAdapter,
    OpenClawPluginsAdapter,
};
use crate::infrastructure::process::ProcessRunner;

/// Use case layer: composes the OpenClaw executable, config, plugins,
/// plugin-install, and channels ports.
pub struct ChannelService {
    openclaw: Arc<dyn OpenClawPort>,
    config: Arc<dyn OpenClawConfigPort>,
    plugins: Arc<dyn OpenClawPluginsPort>,
    plugin_install: Arc<dyn OpenClawPluginInstallPort>,
    channels: Arc<dyn OpenClawChannelsPort>,
}

impl ChannelService {
    pub fn new(
        openclaw: Arc<dyn OpenClawPort>,
        config: Arc<dyn OpenClawConfigPort>,
        plugins: Arc<dyn OpenClawPluginsPort>,
        plugin_install: Arc<dyn OpenClawPluginInstallPort>,
        channels: Arc<dyn OpenClawChannelsPort>,
    ) -> Self {
        Self {
            openclaw,
            config,
            plugins,
            plugin_install,
            channels,
        }
    }

    /// Production wiring: real adapters over the single `ProcessRunner`.
    pub fn production() -> Self {
        let process: Arc<dyn crate::domain::ports::process::ProcessPort> = Arc::new(ProcessRunner);
        let openclaw = Arc::new(OpenClawAdapter::new(
            Arc::clone(&process),
            default_openclaw_search_dirs(),
        ));
        let config = Arc::new(OpenClawConfigAdapter::new(Arc::clone(&process)));
        let plugins = Arc::new(OpenClawPluginsAdapter::new(Arc::clone(&process)));
        let plugin_install = Arc::new(OpenClawPluginInstallAdapter::new(Arc::clone(&process)));
        let channels = Arc::new(OpenClawChannelsAdapter::new(process));
        Self::new(openclaw, config, plugins, plugin_install, channels)
    }

    fn executable(&self) -> Result<PathBuf, AppError> {
        match self.openclaw.detect_executable() {
            crate::domain::models::ExecutableDetection::Found { path } => Ok(path),
            crate::domain::models::ExecutableDetection::NotFound => {
                Err(AppError::openclaw_not_found())
            }
        }
    }

    /// `get-channels`: list rows + status rows merged for discord/telegram.
    /// Read-only (no config mutation).
    pub fn get_channels(&self) -> Result<ChannelsOverview, AppError> {
        let exe = self.executable()?;
        let rows = self.channels.list_channels(&exe)?;
        let status = self.channels.channel_status(&exe)?;
        let channels = SUPPORTED_CHANNELS
            .iter()
            .map(|id| {
                let list_row = rows.iter().find(|row| row.id == *id);
                let status_row = status.rows.iter().find(|row| row.id == *id);
                ChannelSummary {
                    id: (*id).to_string(),
                    installed: list_row.map(|row| row.installed).unwrap_or(false),
                    configured: list_row.map(|row| row.configured).unwrap_or(false),
                    enabled: list_row.map(|row| row.enabled).unwrap_or(false),
                    runtime_state: status_row.and_then(|row| row.state.clone()),
                }
            })
            .collect();
        Ok(ChannelsOverview {
            gateway_reachable: status.gateway_reachable,
            channels,
        })
    }

    /// Redacted `channels.<channel>` snapshot (fail-soft parse).
    pub fn get_channel_config(&self, channel: &str) -> Result<ChannelConfig, AppError> {
        validate_channel_id(channel)?;
        let exe = self.executable()?;
        let raw = self.config.read_raw(&exe, &channel_section_path(channel))?;
        Ok(parse_channel_config_safe(raw.as_deref()))
    }

    /// `connect-channel`: token ref → (Discord: plugin install) →
    /// `enabled=true`. Fixed order, first failure stops.
    pub fn connect_channel(&self, channel: &str) -> Result<(), AppError> {
        validate_channel_id(channel)?;
        let exe = self.executable()?;
        self.require_managed_token(channel, &exe)?;
        if channel == "discord" {
            self.ensure_discord_plugin(&exe)?;
        }
        self.config.write(
            &exe,
            &channel_enabled_path(channel),
            "true",
            WriteMode::Plain,
        )?;
        Ok(())
    }

    /// `set-channel-enabled`: scalar write (disable keeps token/policy).
    pub fn set_channel_enabled(&self, channel: &str, enabled: bool) -> Result<(), AppError> {
        validate_channel_id(channel)?;
        let exe = self.executable()?;
        let value = if enabled { "true" } else { "false" };
        self.config.write(
            &exe,
            &channel_enabled_path(channel),
            value,
            WriteMode::Plain,
        )
    }

    /// `set-dm-access`: pre-validation (fail-closed, 0 process runs) then
    /// the fixed 2-path order: `dmPolicy` (scalar) → `allowFrom`
    /// (array `--replace`). First failure stops.
    pub fn set_dm_access(
        &self,
        channel: &str,
        dm_policy: &str,
        allow_from: &[String],
    ) -> Result<(), AppError> {
        validate_channel_id(channel)?;
        validate_dm_policy(dm_policy)?;
        for entry in allow_from {
            validate_allow_from_entry(entry)?;
        }
        validate_dm_access(dm_policy, allow_from)?;
        let exe = self.executable()?;
        let policy_json = serde_json::to_string(dm_policy)
            .map_err(|err| AppError::openclaw_config_invalid(format!("encode dmPolicy: {err}")))?;
        self.config.write(
            &exe,
            &channel_dm_policy_path(channel),
            &policy_json,
            WriteMode::Plain,
        )?;
        let entries_json = serde_json::to_string(allow_from)
            .map_err(|err| AppError::openclaw_config_invalid(format!("encode allowFrom: {err}")))?;
        self.config.write(
            &exe,
            &channel_allow_from_path(channel),
            &entries_json,
            WriteMode::Replace,
        )
    }

    /// `set-group-policy`: enum-validated scalar write.
    pub fn set_group_policy(&self, channel: &str, group_policy: &str) -> Result<(), AppError> {
        validate_channel_id(channel)?;
        validate_group_policy(group_policy)?;
        let exe = self.executable()?;
        let value_json = serde_json::to_string(group_policy).map_err(|err| {
            AppError::openclaw_config_invalid(format!("encode groupPolicy: {err}"))
        })?;
        self.config.write(
            &exe,
            &channel_group_policy_path(channel),
            &value_json,
            WriteMode::Plain,
        )
    }

    /// `list-pairing-requests`: channel-validated read of the pending
    /// pairing requests.
    pub fn list_pairing_requests(&self, channel: &str) -> Result<Vec<PairingRequest>, AppError> {
        validate_channel_id(channel)?;
        let exe = self.executable()?;
        self.channels.pairing_list(&exe, channel)
    }

    /// `approve-pairing`: channel + code validated (S2, 0 process runs on
    /// failure) before the CLI call. The first approval may bootstrap the
    /// command owner (UI notice only — no extra CLI call).
    pub fn approve_pairing(&self, channel: &str, code: &str) -> Result<(), AppError> {
        validate_channel_id(channel)?;
        validate_pairing_code(code)?;
        let exe = self.executable()?;
        self.channels.pairing_approve(&exe, channel, code)
    }

    /// The connect precondition: the token field must hold a ClawDesk-managed
    /// exec ref. Anything else (absent, external) is a structured error with
    /// zero mutation CLI calls.
    fn require_managed_token(&self, channel: &str, exe: &Path) -> Result<(), AppError> {
        let current = self.config.read_raw(exe, &channel_token_path(channel))?;
        let state = classify_channel_token_state(current.as_deref());
        if state == ChannelTokenState::Managed {
            Ok(())
        } else {
            Err(AppError::channel_token_not_found(channel))
        }
    }

    /// Idempotent Discord plugin install: `plugins list` → (missing)
    /// `plugins install @openclaw/discord` → `plugins list` post-check.
    fn ensure_discord_plugin(&self, exe: &Path) -> Result<(), AppError> {
        let is_installed = |rows: &[PluginRow]| rows.iter().any(|row| row.id == DISCORD_PLUGIN_ID);
        let rows = self.plugins.list_plugins(exe)?;
        if is_installed(&rows) {
            return Ok(());
        }
        self.plugin_install.install_plugin(exe, DISCORD_PLUGIN_ID)?;
        let rows = self.plugins.list_plugins(exe)?;
        if !is_installed(&rows) {
            return Err(AppError::openclaw_plugin_install_failed(
                DISCORD_PLUGIN_ID,
                "the plugin is still missing after install",
            ));
        }
        Ok(())
    }
}

/// Local alias to avoid repeating the long path in `get_channel_config`.
fn parse_channel_config_safe(raw: Option<&str>) -> ChannelConfig {
    crate::domain::models::channels::parse_channel_config(raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::models::channels::{
        channel_secret_ref, ChannelRow, ChannelStatus, ChannelStatusRow, ChannelTokenState,
    };
    use crate::domain::models::openclaw::{GatewayStatus, OpenClawVersion, UpdateState};
    use crate::domain::ports::openclaw::OpenClawPort;
    use crate::domain::ports::openclaw_channels::OpenClawChannelsPort;
    use crate::domain::ports::openclaw_config::{OpenClawConfigPort, WriteMode};
    use crate::domain::ports::openclaw_plugin_install::OpenClawPluginInstallPort;
    use crate::domain::ports::openclaw_plugins::OpenClawPluginsPort;
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

    struct FakeConfig {
        raw: Mutex<std::collections::HashMap<String, String>>,
        log: Arc<Mutex<Vec<String>>>,
        writes: Mutex<Vec<(String, WriteMode, String)>>,
    }

    impl FakeConfig {
        fn new() -> (Arc<FakeConfig>, Arc<Mutex<Vec<String>>>) {
            let log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
            let inner = Arc::new(FakeConfig {
                raw: Mutex::new(std::collections::HashMap::new()),
                log: Arc::clone(&log),
                writes: Mutex::new(Vec::new()),
            });
            (inner, log)
        }
    }

    impl OpenClawConfigPort for FakeConfig {
        fn config_path(&self, _exe: &Path) -> Result<PathBuf, AppError> {
            unimplemented!()
        }
        fn read_providers(
            &self,
            _exe: &Path,
        ) -> Result<Vec<crate::domain::models::models::ProviderDetail>, AppError> {
            Ok(Vec::new())
        }
        fn read_models(
            &self,
            _exe: &Path,
        ) -> Result<Vec<crate::domain::models::ModelRow>, AppError> {
            Ok(Vec::new())
        }
        fn read_default_model(&self, _exe: &Path) -> Result<Option<String>, AppError> {
            Ok(None)
        }
        fn read_thinking_default(
            &self,
            _exe: &Path,
        ) -> Result<Option<crate::domain::models::ThinkingLevel>, AppError> {
            Ok(None)
        }
        fn read_raw(&self, _exe: &Path, path: &str) -> Result<Option<String>, AppError> {
            self.log.lock().unwrap().push(format!("read-raw:{path}"));
            Ok(self.raw.lock().unwrap().get(path).cloned())
        }
        fn write(
            &self,
            _exe: &Path,
            path: &str,
            value_json: &str,
            mode: WriteMode,
        ) -> Result<(), AppError> {
            self.writes
                .lock()
                .unwrap()
                .push((path.to_string(), mode, value_json.to_string()));
            let mode_label = match mode {
                WriteMode::Merge => "merge",
                WriteMode::Replace => "replace",
                WriteMode::Plain => "plain",
            };
            self.log
                .lock()
                .unwrap()
                .push(format!("write:{path}:{mode_label}"));
            Ok(())
        }
        fn unset(&self, _exe: &Path, path: &str) -> Result<(), AppError> {
            self.log.lock().unwrap().push(format!("unset:{path}"));
            Ok(())
        }
        fn set_default_model(&self, _exe: &Path, _model_ref: &str) -> Result<(), AppError> {
            unimplemented!()
        }
    }

    struct FakePlugins {
        /// The plugin ids reported by `plugins list` (scripted, replaced on
        /// demand via `set_ids`).
        ids: Mutex<Vec<String>>,
        log: Arc<Mutex<Vec<String>>>,
    }

    impl FakePlugins {
        fn new(ids: Vec<String>) -> (Arc<FakePlugins>, Arc<Mutex<Vec<String>>>) {
            let log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
            (Self::with_log(ids, Arc::clone(&log)), log)
        }

        /// Shares an external log so the exact operation order (plugins
        /// list / install interleaving) is observable in one place.
        fn with_log(ids: Vec<String>, log: Arc<Mutex<Vec<String>>>) -> Arc<FakePlugins> {
            Arc::new(FakePlugins {
                ids: Mutex::new(ids),
                log: Arc::clone(&log),
            })
        }
    }

    impl OpenClawPluginsPort for FakePlugins {
        fn list_plugins(&self, _exe: &Path) -> Result<Vec<PluginRow>, AppError> {
            self.log.lock().unwrap().push("plugins-list".to_string());
            let ids = self.ids.lock().unwrap().clone();
            Ok(ids
                .into_iter()
                .map(|id| PluginRow {
                    id,
                    enabled: Some(true),
                    name: None,
                    format: None,
                    origin: None,
                    version: None,
                    dependency_status: None,
                })
                .collect())
        }
        fn enable_plugin(&self, _exe: &Path, _id: &str) -> Result<(), AppError> {
            unimplemented!()
        }
        fn disable_plugin(&self, _exe: &Path, _id: &str) -> Result<(), AppError> {
            unimplemented!()
        }
        fn inspect_plugin_runtime(
            &self,
            _exe: &Path,
            _id: &str,
        ) -> Result<crate::domain::models::skills::PluginRuntime, AppError> {
            unimplemented!()
        }
    }

    struct FakeInstall {
        log: Arc<Mutex<Vec<String>>>,
        failure: Mutex<Option<AppError>>,
        /// Simulates the install taking effect in the plugin list.
        plugins: Arc<FakePlugins>,
    }

    impl OpenClawPluginInstallPort for FakeInstall {
        fn install_plugin(&self, _exe: &Path, npm_id: &str) -> Result<(), AppError> {
            if let Some(failure) = self.failure.lock().unwrap().clone() {
                return Err(failure);
            }
            self.log.lock().unwrap().push(format!("install:{npm_id}"));
            // Idempotent real CLI: the plugin becomes visible after install.
            let mut ids = self.plugins.ids.lock().unwrap();
            if !ids.contains(&npm_id.to_string()) {
                ids.push(npm_id.to_string());
            }
            Ok(())
        }
    }

    struct FakeChannels {
        rows: Vec<ChannelRow>,
        status: ChannelStatus,
        pairing: Vec<PairingRequest>,
        log: Arc<Mutex<Vec<String>>>,
    }

    impl OpenClawChannelsPort for FakeChannels {
        fn list_channels(&self, _exe: &Path) -> Result<Vec<ChannelRow>, AppError> {
            self.log.lock().unwrap().push("channels-list".to_string());
            Ok(self.rows.clone())
        }
        fn channel_status(&self, _exe: &Path) -> Result<ChannelStatus, AppError> {
            self.log.lock().unwrap().push("channels-status".to_string());
            Ok(self.status.clone())
        }
        fn pairing_list(
            &self,
            _exe: &Path,
            channel: &str,
        ) -> Result<Vec<PairingRequest>, AppError> {
            self.log
                .lock()
                .unwrap()
                .push(format!("pairing-list:{channel}"));
            Ok(self.pairing.clone())
        }
        fn pairing_approve(&self, _exe: &Path, channel: &str, code: &str) -> Result<(), AppError> {
            self.log
                .lock()
                .unwrap()
                .push(format!("pairing-approve:{channel}:{code}"));
            Ok(())
        }
    }

    type ServiceFixture = (
        ChannelService,
        Arc<FakeConfig>,
        Arc<FakePlugins>,
        Arc<FakeInstall>,
        Arc<FakeChannels>,
        Arc<Mutex<Vec<String>>>,
    );

    fn service() -> ServiceFixture {
        let (config, config_log) = FakeConfig::new();
        let plugins = FakePlugins::with_log(Vec::new(), Arc::clone(&config_log));
        let install = Arc::new(FakeInstall {
            log: Arc::clone(&config_log),
            failure: Mutex::new(None),
            plugins: Arc::clone(&plugins),
        });
        let channels = Arc::new(FakeChannels {
            rows: vec![
                ChannelRow {
                    id: "discord".into(),
                    installed: true,
                    configured: false,
                    enabled: false,
                },
                ChannelRow {
                    id: "telegram".into(),
                    installed: true,
                    configured: true,
                    enabled: true,
                },
            ],
            status: ChannelStatus {
                gateway_reachable: true,
                rows: vec![
                    ChannelStatusRow {
                        id: "discord".into(),
                        state: Some("connected".into()),
                    },
                    ChannelStatusRow {
                        id: "telegram".into(),
                        state: None,
                    },
                ],
            },
            pairing: vec![PairingRequest {
                code: "AB12CD34".into(),
                sender: Some("user-1".into()),
            }],
            log: Arc::clone(&config_log),
        });
        let service = ChannelService::new(
            Arc::new(FixedOpenClaw),
            config.clone(),
            plugins.clone(),
            install.clone(),
            channels.clone(),
        );
        (service, config, plugins, install, channels, config_log)
    }

    fn seed_discord_ref(config: &FakeConfig) {
        config.raw.lock().unwrap().insert(
            "channels.discord.token".to_string(),
            serde_json::to_string(&channel_secret_ref("discord")).unwrap(),
        );
    }

    fn seed_telegram_ref(config: &FakeConfig) {
        config.raw.lock().unwrap().insert(
            "channels.telegram.botToken".to_string(),
            serde_json::to_string(&channel_secret_ref("telegram")).unwrap(),
        );
    }

    // --- get_channels -----------------------------------------------------------

    #[test]
    fn get_channels_merges_list_and_status_rows() {
        let (service, _config, _plugins, _install, _channels, log) = service();
        let overview = service.get_channels().expect("get channels");
        assert!(overview.gateway_reachable);
        assert_eq!(overview.channels.len(), 2, "discord/telegram only");
        let discord = &overview.channels[0];
        assert_eq!(discord.id, "discord");
        assert!(discord.installed && !discord.configured && !discord.enabled);
        assert_eq!(discord.runtime_state.as_deref(), Some("connected"));
        let telegram = &overview.channels[1];
        assert_eq!(telegram.id, "telegram");
        assert!(telegram.installed && telegram.configured && telegram.enabled);
        assert_eq!(telegram.runtime_state, None);
        let lines = log.lock().unwrap().clone();
        assert_eq!(
            lines,
            vec!["channels-list".to_string(), "channels-status".to_string()]
        );
    }

    #[test]
    fn get_channels_absent_rows_are_fail_soft_false() {
        let (config, log) = FakeConfig::new();
        let (plugins, _plugins_log) = FakePlugins::new(Vec::new());
        let install = Arc::new(FakeInstall {
            log: Arc::clone(&log),
            failure: Mutex::new(None),
            plugins: Arc::clone(&plugins),
        });
        let channels = Arc::new(FakeChannels {
            rows: Vec::new(),
            status: ChannelStatus {
                gateway_reachable: false,
                rows: Vec::new(),
            },
            pairing: Vec::new(),
            log: Arc::clone(&log),
        });
        let service =
            ChannelService::new(Arc::new(FixedOpenClaw), config, plugins, install, channels);
        let overview = service.get_channels().expect("get channels");
        assert!(!overview.gateway_reachable);
        assert_eq!(overview.channels.len(), 2, "discord/telegram only");
        for summary in &overview.channels {
            assert!(
                !summary.installed && !summary.configured && !summary.enabled,
                "absent rows fail soft to false"
            );
            assert_eq!(summary.runtime_state, None);
        }
    }

    // --- connect_channel -----------------------------------------------------------

    #[test]
    fn connect_discord_full_order_token_install_enabled() {
        let (service, config, plugins, _install, _channels, log) = service();
        seed_discord_ref(&config);
        service.connect_channel("discord").expect("connect");
        let lines = log.lock().unwrap().clone();
        assert_eq!(
            lines,
            vec![
                "read-raw:channels.discord.token".to_string(),
                "plugins-list".to_string(),
                "install:@openclaw/discord".to_string(),
                "plugins-list".to_string(),
                "write:channels.discord.enabled:plain".to_string(),
            ]
        );
        assert!(plugins
            .ids
            .lock()
            .unwrap()
            .contains(&DISCORD_PLUGIN_ID.to_string()));
    }

    #[test]
    fn connect_discord_install_idempotent_second_run_skips_install() {
        let (service, config, _plugins, _install, _channels, log) = service();
        seed_discord_ref(&config);
        service.connect_channel("discord").expect("first connect");
        let mut lines = log.lock().unwrap().clone();
        assert_eq!(lines.iter().filter(|l| l.starts_with("install")).count(), 1);
        // Second run: the plugin is now installed → no second install.
        service.connect_channel("discord").expect("second connect");
        lines = log.lock().unwrap().clone();
        assert_eq!(
            lines.iter().filter(|l| l.starts_with("install")).count(),
            1,
            "install must run exactly once across both connects: {lines:?}"
        );
    }

    #[test]
    fn connect_discord_already_installed_skips_install() {
        let (service, config, plugins, _install, _channels, log) = service();
        seed_discord_ref(&config);
        plugins
            .ids
            .lock()
            .unwrap()
            .push(DISCORD_PLUGIN_ID.to_string());
        service.connect_channel("discord").expect("connect");
        let lines = log.lock().unwrap().clone();
        assert_eq!(
            lines,
            vec![
                "read-raw:channels.discord.token".to_string(),
                "plugins-list".to_string(),
                "write:channels.discord.enabled:plain".to_string(),
            ]
        );
    }

    #[test]
    fn connect_telegram_has_no_install_step() {
        let (service, config, _plugins, _install, _channels, log) = service();
        seed_telegram_ref(&config);
        service.connect_channel("telegram").expect("connect");
        let lines = log.lock().unwrap().clone();
        assert_eq!(
            lines,
            vec![
                "read-raw:channels.telegram.botToken".to_string(),
                "write:channels.telegram.enabled:plain".to_string(),
            ]
        );
    }

    #[test]
    fn connect_without_token_stops_before_any_mutation() {
        let (service, _config, _plugins, _install, _channels, log) = service();
        let err = service
            .connect_channel("discord")
            .expect_err("no token registered");
        assert_eq!(err.code, "channel-token-not-found");
        let lines = log.lock().unwrap().clone();
        assert_eq!(
            lines,
            vec!["read-raw:channels.discord.token".to_string()],
            "only the token check ran — no install/enable CLI: {lines:?}"
        );
    }

    #[test]
    fn connect_with_external_token_stops_before_any_mutation() {
        let (service, config, _plugins, _install, _channels, log) = service();
        config.raw.lock().unwrap().insert(
            "channels.discord.token".to_string(),
            r#""external-token""#.into(),
        );
        let err = service
            .connect_channel("discord")
            .expect_err("external token");
        assert_eq!(err.code, "channel-token-not-found");
        let lines = log.lock().unwrap().clone();
        assert_eq!(lines.len(), 1, "no mutation CLI: {lines:?}");
    }

    #[test]
    fn connect_install_fails_stops_before_enabled_write() {
        let (service, config, _plugins, install, _channels, log) = service();
        seed_discord_ref(&config);
        *install.failure.lock().unwrap() = Some(AppError::openclaw_plugin_install_failed(
            DISCORD_PLUGIN_ID,
            "npm registry unreachable",
        ));
        let err = service
            .connect_channel("discord")
            .expect_err("install failure");
        assert_eq!(err.code, "openclaw-plugin-install-failed");
        let lines = log.lock().unwrap().clone();
        assert!(
            !lines
                .iter()
                .any(|line| line.starts_with("write:channels.discord.enabled")),
            "no enabled write after install failure: {lines:?}"
        );
    }

    #[test]
    fn connect_install_post_check_missing_is_install_failed() {
        // The install "succeeds" but the post-check still sees no plugin:
        // use a plugin list that never gains the id.
        let (config, config_log) = FakeConfig::new();
        let (plugins, _plugins_log) = FakePlugins::new(Vec::new());
        let install = Arc::new(FakeInstall {
            log: Arc::clone(&config_log),
            failure: Mutex::new(None),
            // Point at a separate plugin list the service never reads from,
            // so the post-check list stays empty after the "install".
            plugins: FakePlugins::new(Vec::new()).0,
        });
        let channels = Arc::new(FakeChannels {
            rows: Vec::new(),
            status: ChannelStatus {
                gateway_reachable: false,
                rows: Vec::new(),
            },
            pairing: Vec::new(),
            log: Arc::clone(&config_log),
        });
        seed_discord_ref(&config);
        let plugins_port: Arc<dyn OpenClawPluginsPort> = plugins.clone();
        let service = ChannelService::new(
            Arc::new(FixedOpenClaw),
            config,
            plugins_port,
            install,
            channels,
        );
        let err = service
            .connect_channel("discord")
            .expect_err("post-check must fail");
        assert_eq!(err.code, "openclaw-plugin-install-failed");
        let lines = config_log.lock().unwrap().clone();
        assert!(
            !lines
                .iter()
                .any(|line| line.starts_with("write:channels.discord.enabled")),
            "no enabled write after a failed post-check: {lines:?}"
        );
    }

    // --- policy writes ----------------------------------------------------------------

    #[test]
    fn set_dm_access_writes_policy_then_allow_from_in_order() {
        let (service, _config, _plugins, _install, _channels, log) = service();
        service
            .set_dm_access("discord", "allowlist", &["1234567890".into()])
            .expect("set dm access");
        let lines = log.lock().unwrap().clone();
        assert_eq!(
            lines,
            vec![
                "write:channels.discord.dmPolicy:plain".to_string(),
                "write:channels.discord.allowFrom:replace".to_string(),
            ]
        );
    }

    #[test]
    fn set_dm_access_validation_fails_closed_with_zero_cli() {
        let (service, _config, _plugins, _install, _channels, log) = service();
        let star = vec!["*".to_string()];
        let one = vec!["123".to_string()];
        let short = vec!["12a".to_string()];
        let blank = vec!["".to_string()];
        let cases: Vec<(&str, &[String], &str)> = vec![
            ("", &star, "dm-policy-invalid"),
            ("Pairing", &star, "dm-policy-invalid"),
            ("discord", &[], ""),
            ("allowlist", &[], "dm-access-inconsistent"),
            ("open", &one, "dm-access-inconsistent"),
            ("pairing", &short, "allow-from-entry-invalid"),
            ("pairing", &blank, "allow-from-entry-invalid"),
        ];
        for (policy, entries, expected) in &cases {
            if expected.is_empty() {
                continue;
            }
            let err = service
                .set_dm_access("discord", policy, entries)
                .expect_err("must be rejected");
            assert_eq!(err.code, *expected, "{policy:?}/{entries:?}");
        }
        // Invalid channel id is rejected first.
        let err = service
            .set_dm_access("slack", "pairing", &["*".into()])
            .expect_err("bad channel");
        assert_eq!(err.code, "channel-id-invalid");
        assert!(
            log.lock().unwrap().is_empty(),
            "no config write or read on validation failure"
        );
    }

    #[test]
    fn set_group_policy_scalar_write_and_validation() {
        let (service, _config, _plugins, _install, _channels, log) = service();
        service
            .set_group_policy("telegram", "allowlist")
            .expect("set group policy");
        assert_eq!(
            log.lock().unwrap().clone(),
            vec!["write:channels.telegram.groupPolicy:plain".to_string()]
        );
        let err = service
            .set_group_policy("discord", "Pairing")
            .expect_err("invalid enum");
        assert_eq!(err.code, "group-policy-invalid");
    }

    #[test]
    fn set_channel_enabled_scalar_write() {
        let (service, _config, _plugins, _install, _channels, log) = service();
        service
            .set_channel_enabled("discord", false)
            .expect("disable");
        assert_eq!(
            log.lock().unwrap().clone(),
            vec!["write:channels.discord.enabled:plain".to_string()]
        );
        let err = service
            .set_channel_enabled("slack", true)
            .expect_err("bad channel");
        assert_eq!(err.code, "channel-id-invalid");
    }

    // --- pairing ------------------------------------------------------------------------

    #[test]
    fn pairing_commands_validate_before_any_call() {
        let (service, _config, _plugins, _install, _channels, log) = service();
        for bad in ["slack", "", "Discord"] {
            let err = service
                .list_pairing_requests(bad)
                .expect_err("bad channel list");
            assert_eq!(err.code, "channel-id-invalid", "{bad:?}");
            let err = service
                .approve_pairing(bad, "ABCD1234")
                .expect_err("bad channel approve");
            assert_eq!(err.code, "channel-id-invalid", "{bad:?}");
        }
        for bad in ["", "abc", "1 2", &"x".repeat(65)] {
            let err = service
                .approve_pairing("discord", bad)
                .expect_err("bad code");
            assert_eq!(err.code, "pairing-code-invalid", "{bad:?}");
        }
        assert!(log.lock().unwrap().is_empty(), "no CLI call at all");
    }

    #[test]
    fn pairing_commands_delegate_to_port() {
        let (service, _config, _plugins, _install, _channels, log) = service();
        let requests = service.list_pairing_requests("discord").expect("list");
        assert_eq!(requests[0].code, "AB12CD34");
        service
            .approve_pairing("telegram", "ABCD1234")
            .expect("approve");
        let lines = log.lock().unwrap().clone();
        assert_eq!(
            lines,
            vec![
                "pairing-list:discord".to_string(),
                "pairing-approve:telegram:ABCD1234".to_string()
            ]
        );
    }

    // --- config read -----------------------------------------------------------------------

    #[test]
    fn get_channel_config_parses_redacted_section() {
        let (service, config, _plugins, _install, _channels, _log) = service();
        config
            .raw
            .lock()
            .unwrap()
            .insert(
                "channels.telegram".to_string(),
                serde_json::json!({
                    "enabled": false,
                    "botToken": {"source":"exec","provider":"clawdesk","id":"channels/telegram/botToken"},
                    "dmPolicy": "pairing",
                    "allowFrom": ["*"],
                    "groupPolicy": "allowlist"
                })
                .to_string(),
            );
        let config_view = service.get_channel_config("telegram").expect("read config");
        assert_eq!(config_view.enabled, Some(false));
        assert_eq!(config_view.token_state, ChannelTokenState::Managed);
        assert_eq!(config_view.dm_policy.as_deref(), Some("pairing"));
        assert_eq!(config_view.allow_from, vec!["*".to_string()]);
        assert_eq!(config_view.group_policy.as_deref(), Some("allowlist"));

        // Missing section → fail-soft defaults.
        let config_view = service.get_channel_config("discord").expect("read config");
        assert_eq!(config_view.enabled, None);
        assert_eq!(config_view.token_state, ChannelTokenState::Absent);
        assert_eq!(config_view.dm_policy, None);
        assert!(config_view.allow_from.is_empty());
        assert_eq!(config_view.group_policy, None);
    }
}
