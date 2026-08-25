//! Unified application error with stable, machine-readable codes.
//!
//! Architecture §5: all errors surfaced across the stack use `AppError`.
//! Messages must never contain secrets (S3/S8).

use std::fmt;

use crate::infrastructure::masking::mask_secrets;

/// Unified application error. `code` is stable and safe to compare/log.
///
/// Serialized across Tauri IPC (Phase 2 commands layer): the frontend maps
/// the stable `code` to a user-facing message, never the raw `message`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AppError {
    /// Stable error code, e.g. `"openclaw-not-found"`.
    pub code: &'static str,
    /// Human-readable message (already masked).
    pub message: String,
}

impl AppError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        // S8: every error message passes the masking pipeline exactly once
        // at construction, so no secret can reach logs, the UI, or IPC.
        Self {
            code,
            message: mask_secrets(&message.into()),
        }
    }

    pub fn unsupported_architecture(found: impl Into<String>) -> Self {
        Self::new(
            "unsupported-architecture",
            format!(
                "unsupported architecture: {} (ClawDesk supports x64 only)",
                found.into()
            ),
        )
    }

    pub fn unsupported_os_version(build: u32) -> Self {
        Self::new(
            "unsupported-os-version",
            format!(
                "unsupported Windows build {build}: ClawDesk requires Windows 10/11 (build >= 10240)"
            ),
        )
    }

    pub fn os_info_unavailable(message: impl Into<String>) -> Self {
        Self::new("os-info-unavailable", message)
    }

    pub fn node_not_found() -> Self {
        Self::new(
            "node-not-found",
            "Node.js executable was not found on this machine",
        )
    }

    pub fn node_version_unavailable(message: impl Into<String>) -> Self {
        Self::new("node-version-unavailable", message)
    }

    pub fn unsupported_node_version(version: impl Into<String>) -> Self {
        Self::new(
            "unsupported-node-version",
            format!(
                "unsupported Node.js version: {} (supported: 22.22.3+, 24.15+, 25.9+, 26+)",
                version.into()
            ),
        )
    }

    pub fn npm_not_found() -> Self {
        Self::new(
            "npm-not-found",
            "npm was not found alongside the detected Node.js installation",
        )
    }

    pub fn unsupported_npm_version(version: impl Into<String>) -> Self {
        Self::new(
            "unsupported-npm-version",
            format!(
                "unsupported npm version: {} (npm 11.13-11.15 cannot install OpenClaw)",
                version.into()
            ),
        )
    }

    pub fn openclaw_install_failed(detail: impl Into<String>) -> Self {
        Self::new(
            "openclaw-install-failed",
            format!("OpenClaw installation failed: {}", detail.into()),
        )
    }

    pub fn openclaw_install_verify_failed(detail: impl Into<String>) -> Self {
        Self::new(
            "openclaw-install-verify-failed",
            format!("OpenClaw install verification failed: {}", detail.into()),
        )
    }

    pub fn openclaw_not_found() -> Self {
        Self::new(
            "openclaw-not-found",
            "OpenClaw executable was not found on this machine",
        )
    }

    pub fn process_timeout(command: &str) -> Self {
        Self::new("process-timeout", format!("{command} timed out"))
    }

    pub fn process_failed(command: &str, detail: impl Into<String>) -> Self {
        Self::new(
            "process-failed",
            format!("{command} failed: {}", detail.into()),
        )
    }

    pub fn openclaw_version_parse(detail: impl Into<String>) -> Self {
        Self::new(
            "openclaw-version-parse",
            format!("cannot parse OpenClaw version: {}", detail.into()),
        )
    }

    pub fn openclaw_gateway_parse(detail: impl Into<String>) -> Self {
        Self::new(
            "openclaw-gateway-parse",
            format!("cannot parse OpenClaw gateway status: {}", detail.into()),
        )
    }

    /// S2: user input failed format validation before any process run.
    /// The offending value is included only as a masked diagnostic.
    pub fn invalid_input(code: &'static str, field: &str, value: &str) -> Self {
        Self::new(code, format!("{field} is invalid: {value}"))
    }

    pub fn openclaw_config_read_failed(detail: impl Into<String>) -> Self {
        Self::new(
            "openclaw-config-read-failed",
            format!("reading the OpenClaw config failed: {}", detail.into()),
        )
    }

    pub fn openclaw_config_write_failed(detail: impl Into<String>) -> Self {
        Self::new(
            "openclaw-config-write-failed",
            format!("writing the OpenClaw config failed: {}", detail.into()),
        )
    }

    pub fn openclaw_config_invalid(detail: impl Into<String>) -> Self {
        Self::new(
            "openclaw-config-invalid",
            format!("the OpenClaw config change was rejected: {}", detail.into()),
        )
    }

    /// S2: skill name failed format validation before any process run.
    pub fn skill_name_invalid(name: &str) -> Self {
        Self::invalid_input("skill-name-invalid", "skill name", name)
    }

    /// The toggle target skill is not in the `skills list` output; no config
    /// write is attempted.
    pub fn skill_not_found(name: &str) -> Self {
        Self::new("skill-not-found", format!("skill not found: {name}"))
    }

    /// S2: plugin id failed format validation before any process run.
    pub fn plugin_id_invalid(id: &str) -> Self {
        Self::invalid_input("plugin-id-invalid", "plugin id", id)
    }

    pub fn openclaw_skills_read_failed(detail: impl Into<String>) -> Self {
        Self::new(
            "openclaw-skills-read-failed",
            format!("reading the OpenClaw skills failed: {}", detail.into()),
        )
    }

    pub fn openclaw_plugins_read_failed(detail: impl Into<String>) -> Self {
        Self::new(
            "openclaw-plugins-read-failed",
            format!("reading the OpenClaw plugins failed: {}", detail.into()),
        )
    }

    /// `openclaw plugins enable/disable <id>` exited non-zero (unknown id,
    /// Nix-mode rejection, ...). The actual state must be re-queried.
    pub fn openclaw_plugin_toggle_failed(id: &str, detail: impl Into<String>) -> Self {
        Self::new(
            "openclaw-plugin-toggle-failed",
            format!("enabling/disabling plugin {id} failed: {}", detail.into()),
        )
    }

    pub fn secret_store_unavailable(detail: impl Into<String>) -> Self {
        Self::new(
            "secret-store-unavailable",
            format!(
                "the OS secret store (DPAPI) is unavailable: {}",
                detail.into()
            ),
        )
    }

    pub fn secret_ref_registration_failed(detail: impl Into<String>) -> Self {
        Self::new(
            "secret-ref-registration-failed",
            format!(
                "registering the API key reference failed: {}",
                detail.into()
            ),
        )
    }

    // --- Phase 5: tools / security -------------------------------------------

    /// S2: the tool profile value failed the enum validation before any
    /// process run.
    pub fn tool_profile_invalid(profile: &str) -> Self {
        Self::invalid_input("tool-profile-invalid", "tool profile", profile)
    }

    /// S2: an allow/deny entry failed format validation before any process
    /// run.
    pub fn tool_entry_invalid(entry: &str) -> Self {
        Self::invalid_input("tool-entry-invalid", "tool entry", entry)
    }

    /// S2: the exec mode value failed the enum validation before any
    /// process run.
    pub fn exec_mode_invalid(mode: &str) -> Self {
        Self::invalid_input("exec-mode-invalid", "exec mode", mode)
    }

    /// S2: a security profile id failed the slug validation.
    pub fn security_profile_id_invalid(id: &str) -> Self {
        Self::invalid_input("security-profile-id-invalid", "profile id", id)
    }

    /// S2: a security profile display name failed validation (1–50 chars,
    /// no control characters).
    pub fn security_profile_name_invalid(name: &str) -> Self {
        Self::invalid_input("security-profile-name-invalid", "profile name", name)
    }

    /// The referenced profile id is not a builtin or a stored user profile
    /// (includes delete/edit attempts against builtin ids).
    pub fn security_profile_not_found(id: &str) -> Self {
        Self::new(
            "security-profile-not-found",
            format!("profile not found: {id}"),
        )
    }

    /// A new profile id collides with a builtin profile id.
    pub fn security_profile_conflict(id: &str) -> Self {
        Self::new(
            "security-profile-conflict",
            format!("profile id already exists: {id}"),
        )
    }

    /// The ClawDesk-owned security profile store failed to read/write.
    pub fn security_profile_store_failed(detail: impl Into<String>) -> Self {
        Self::new(
            "security-profile-store-failed",
            format!(
                "reading/writing the security profile store failed: {}",
                detail.into()
            ),
        )
    }

    /// `openclaw security audit --json` failed (run or parse). The audit
    /// result is unknown — the UI must not assume a clean state.
    pub fn openclaw_security_audit_failed(detail: impl Into<String>) -> Self {
        Self::new(
            "openclaw-security-audit-failed",
            format!("the security audit failed: {}", detail.into()),
        )
    }

    // --- Phase 6: channels -------------------------------------------------------

    /// S2: the channel id failed the `{discord, telegram}` validation before
    /// any process run.
    pub fn channel_id_invalid(channel: &str) -> Self {
        Self::invalid_input("channel-id-invalid", "channel", channel)
    }

    /// S2: the channel token is empty after trimming. The value is never
    /// echoed (S3).
    pub fn channel_token_invalid() -> Self {
        Self::new("channel-token-invalid", "the channel token is empty")
    }

    /// The channel has no ClawDesk-managed token (delete target missing, or
    /// connect precondition unmet).
    pub fn channel_token_not_found(channel: &str) -> Self {
        Self::new(
            "channel-token-not-found",
            format!("no ClawDesk-managed token for channel {channel}"),
        )
    }

    /// S2: the dmPolicy value failed the enum validation before any process
    /// run.
    pub fn dm_policy_invalid(policy: &str) -> Self {
        Self::invalid_input("dm-policy-invalid", "dmPolicy", policy)
    }

    /// S2: the groupPolicy value failed the enum validation before any
    /// process run.
    pub fn group_policy_invalid(policy: &str) -> Self {
        Self::invalid_input("group-policy-invalid", "groupPolicy", policy)
    }

    /// S2: an allowFrom entry failed format validation (`*` or numeric id).
    pub fn allow_from_entry_invalid(entry: &str) -> Self {
        Self::invalid_input("allow-from-entry-invalid", "allowFrom entry", entry)
    }

    /// The dmPolicy/allowFrom pair violates the cross-rule (allowlist needs
    /// at least one entry, open needs `*`).
    pub fn dm_access_inconsistent(detail: impl Into<String>) -> Self {
        Self::new(
            "dm-access-inconsistent",
            format!("the DM access settings are inconsistent: {}", detail.into()),
        )
    }

    /// S2: the pairing code failed format validation before any process run.
    pub fn pairing_code_invalid(code: &str) -> Self {
        Self::invalid_input("pairing-code-invalid", "pairing code", code)
    }

    /// `openclaw channels list/status` failed (run or parse).
    pub fn openclaw_channels_failed(detail: impl Into<String>) -> Self {
        Self::new(
            "openclaw-channels-failed",
            format!("reading the OpenClaw channels failed: {}", detail.into()),
        )
    }

    /// `openclaw pairing list/approve` failed (run or parse).
    pub fn openclaw_pairing_failed(detail: impl Into<String>) -> Self {
        Self::new(
            "openclaw-pairing-failed",
            format!("the OpenClaw pairing operation failed: {}", detail.into()),
        )
    }

    /// `openclaw plugins install <npm-id>` failed (run or post-check).
    pub fn openclaw_plugin_install_failed(npm_id: &str, detail: impl Into<String>) -> Self {
        Self::new(
            "openclaw-plugin-install-failed",
            format!("installing plugin {npm_id} failed: {}", detail.into()),
        )
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for AppError {}
