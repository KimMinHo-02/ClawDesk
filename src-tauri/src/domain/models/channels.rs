//! Domain types for the Phase 6 channels feature (Discord / Telegram).
//!
//! Wire shapes mirror the latest-stable OpenClaw config format (camelCase).
//! Parsers are fail-soft: a malformed/absent field degrades to
//! `null`/empty/`absent` — it never panics and never surfaces the token
//! value (redacted snapshots only, S3/S7).
//!
//! The live CLI output schemas for `channels list/status` and
//! `pairing list` are fixture-confirmed per contract (Phase 6 unverified
//! items); the fake CLI in `fixtures/fake-openclaw` is the contract source.

use crate::domain::models::models::{SecretRef, CLAWDESK_SECRET_ALIAS};
use crate::error::AppError;

/// The channels ClawDesk manages (contract: Discord / Telegram only).
pub const SUPPORTED_CHANNELS: [&str; 2] = ["discord", "telegram"];

/// The Discord official plugin npm id (the only plugin ClawDesk may install).
pub const DISCORD_PLUGIN_ID: &str = "@openclaw/discord";

/// S2: the channel id must be exactly `discord` or `telegram` before any
/// argv/config-path use.
pub fn validate_channel_id(channel: &str) -> Result<(), AppError> {
    if SUPPORTED_CHANNELS.contains(&channel) {
        Ok(())
    } else {
        Err(AppError::channel_id_invalid(channel))
    }
}

/// The ClawDesk-managed key id for a channel token (exec id pattern,
/// passes `is_valid_key_id`):
/// `channels/discord/token` | `channels/telegram/botToken`.
pub fn channel_secret_key_id(channel: &str) -> String {
    match channel {
        "discord" => "channels/discord/token".to_string(),
        "telegram" => "channels/telegram/botToken".to_string(),
        _ => unreachable!("channel id is validated by validate_channel_id"),
    }
}

/// The ClawDesk exec SecretRef for a channel token (reuses the Phase 3
/// `clawdesk` provider alias; the token value is never part of the ref).
pub fn channel_secret_ref(channel: &str) -> SecretRef {
    SecretRef {
        source: "exec".to_string(),
        provider: CLAWDESK_SECRET_ALIAS.to_string(),
        id: channel_secret_key_id(channel),
    }
}

/// The config path of the token field:
/// `channels.discord.token` | `channels.telegram.botToken`.
pub fn channel_token_path(channel: &str) -> String {
    match channel {
        "discord" => "channels.discord.token".to_string(),
        "telegram" => "channels.telegram.botToken".to_string(),
        _ => unreachable!("channel id is validated by validate_channel_id"),
    }
}

/// The config section path: `channels.<channel>`.
pub fn channel_section_path(channel: &str) -> String {
    format!("channels.{channel}")
}

/// `channels.<channel>.enabled`
pub fn channel_enabled_path(channel: &str) -> String {
    format!("channels.{channel}.enabled")
}

/// `channels.<channel>.dmPolicy`
pub fn channel_dm_policy_path(channel: &str) -> String {
    format!("channels.{channel}.dmPolicy")
}

/// `channels.<channel>.allowFrom`
pub fn channel_allow_from_path(channel: &str) -> String {
    format!("channels.{channel}.allowFrom")
}

/// `channels.<channel>.groupPolicy`
pub fn channel_group_policy_path(channel: &str) -> String {
    format!("channels.{channel}.groupPolicy")
}

/// S2: the token must be non-empty after trimming. The value itself is
/// never echoed into the error (S3) — only a fact message.
pub fn validate_channel_token(token: &str) -> Result<(), AppError> {
    if token.trim().is_empty() {
        return Err(AppError::channel_token_invalid());
    }
    Ok(())
}

/// S2: `dmPolicy` enum (both channels): `pairing` | `allowlist` | `open` |
/// `disabled`.
pub fn validate_dm_policy(policy: &str) -> Result<(), AppError> {
    if matches!(policy, "pairing" | "allowlist" | "open" | "disabled") {
        Ok(())
    } else {
        Err(AppError::dm_policy_invalid(policy))
    }
}

/// S2: `groupPolicy` enum: `open` | `allowlist` | `disabled`.
pub fn validate_group_policy(policy: &str) -> Result<(), AppError> {
    if matches!(policy, "open" | "allowlist" | "disabled") {
        Ok(())
    } else {
        Err(AppError::group_policy_invalid(policy))
    }
}

/// S2: one `allowFrom` entry — `*` or a numeric user id of 1–32 digits
/// (Discord/Telegram common; ClawDesk accepts numeric only, prefix forms
/// are rejected).
pub fn validate_allow_from_entry(entry: &str) -> Result<(), AppError> {
    let ok = entry == "*"
        || (entry.as_bytes().first().is_some_and(|c| c.is_ascii_digit())
            && entry.len() <= 32
            && entry.bytes().all(|c| c.is_ascii_digit()));
    if ok {
        Ok(())
    } else {
        Err(AppError::allow_from_entry_invalid(entry))
    }
}

/// Cross-rule for a DM access change: `allowlist` requires at least one
/// entry, `open` requires `*` to be present.
pub fn validate_dm_access(dm_policy: &str, allow_from: &[String]) -> Result<(), AppError> {
    match dm_policy {
        "allowlist" if allow_from.is_empty() => Err(AppError::dm_access_inconsistent(
            "dmPolicy=allowlist requires at least one allowFrom entry",
        )),
        "open" if !allow_from.iter().any(|entry| entry == "*") => Err(
            AppError::dm_access_inconsistent("dmPolicy=open requires `*` in allowFrom"),
        ),
        _ => Ok(()),
    }
}

/// S2: a pairing code — 4–64 chars of `[A-Za-z0-9_-]` (single argv element,
/// never shell-interpolated).
pub fn validate_pairing_code(code: &str) -> Result<(), AppError> {
    let bytes = code.as_bytes();
    let ok = bytes.len() >= 4
        && bytes.len() <= 64
        && bytes
            .iter()
            .all(|c| c.is_ascii_alphanumeric() || *c == b'_' || *c == b'-');
    if ok {
        Ok(())
    } else {
        Err(AppError::pairing_code_invalid(code))
    }
}

// --- Token state (shape-based classification, value never exposed) ----------------

/// How the channel token field is populated (redacted snapshot only).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ChannelTokenState {
    /// No token field (null/absent).
    Absent,
    /// A ClawDesk exec SecretRef (`source=exec`, `provider=clawdesk`).
    Managed,
    /// Any other present value (plaintext string or a foreign ref — never
    /// exposed).
    External,
}

/// Classifies a raw (redacted) token field value without ever exposing the
/// value. Unparseable input is conservatively `External` (fail-closed: a
/// ClawDesk write must not overwrite an unknown existing value).
pub fn classify_channel_token_state(raw: Option<&str>) -> ChannelTokenState {
    match raw {
        None => ChannelTokenState::Absent,
        Some(text) => {
            let text = text.trim();
            if text.is_empty() || text == "null" {
                return ChannelTokenState::Absent;
            }
            match serde_json::from_str::<serde_json::Value>(text) {
                Ok(serde_json::Value::Object(map)) => {
                    let is_clawdesk_ref = map.get("source").and_then(|v| v.as_str())
                        == Some("exec")
                        && map.get("provider").and_then(|v| v.as_str())
                            == Some(CLAWDESK_SECRET_ALIAS)
                        && map
                            .get("id")
                            .and_then(|v| v.as_str())
                            .is_some_and(|id| !id.is_empty());
                    if is_clawdesk_ref {
                        ChannelTokenState::Managed
                    } else {
                        ChannelTokenState::External
                    }
                }
                Ok(_) => ChannelTokenState::External,
                Err(_) => {
                    // A non-JSON string is a (redacted) plaintext token.
                    ChannelTokenState::External
                }
            }
        }
    }
}

// --- Channel config snapshot (fail-soft) ------------------------------------------

/// A redacted snapshot of `channels.<channel>` (contract §3 policy read).
/// The token value itself is never a field (S7).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelConfig {
    /// `enabled` (null when the section is absent).
    pub enabled: Option<bool>,
    /// Token field state: `managed` | `external` | `absent`.
    #[serde(rename = "tokenState")]
    pub token_state: ChannelTokenState,
    /// `dmPolicy` (unknown raw values are kept, null when absent).
    pub dm_policy: Option<String>,
    /// `allowFrom` (non-array/absent → empty; non-string elements skipped).
    pub allow_from: Vec<String>,
    /// `groupPolicy` (unknown raw values are kept, null when absent).
    pub group_policy: Option<String>,
}

/// Parses the raw `config get channels.<channel> --json` snapshot
/// (fail-soft: missing section → null/empty/absent).
pub fn parse_channel_config(raw: Option<&str>) -> ChannelConfig {
    let default = || ChannelConfig {
        enabled: None,
        token_state: classify_channel_token_state(None),
        dm_policy: None,
        allow_from: Vec::new(),
        group_policy: None,
    };
    let Some(text) = raw else {
        return default();
    };
    let value = match serde_json::from_str::<serde_json::Value>(text) {
        Ok(value) => value,
        Err(_) => return default(),
    };
    match value {
        serde_json::Value::Object(map) => ChannelConfig {
            enabled: map.get("enabled").and_then(|v| v.as_bool()),
            // `token` (Discord) or `botToken` (Telegram) — the section is
            // channel-agnostic at parse time, so accept either field name.
            token_state: map
                .get("botToken")
                .or_else(|| map.get("token"))
                .map(classify_token_value)
                .unwrap_or(ChannelTokenState::Absent),
            dm_policy: map
                .get("dmPolicy")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            allow_from: map
                .get("allowFrom")
                .and_then(|v| v.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            group_policy: map
                .get("groupPolicy")
                .and_then(|v| v.as_str())
                .map(str::to_string),
        },
        // A non-object section shape is not a channel config: fail-soft.
        _ => default(),
    }
}

/// Classifies the `token` field of a channel section object.
fn classify_token_value(value: &serde_json::Value) -> ChannelTokenState {
    match value {
        serde_json::Value::Null => ChannelTokenState::Absent,
        serde_json::Value::Object(map) => {
            let is_clawdesk_ref = map.get("source").and_then(|v| v.as_str()) == Some("exec")
                && map.get("provider").and_then(|v| v.as_str()) == Some(CLAWDESK_SECRET_ALIAS)
                && map
                    .get("id")
                    .and_then(|v| v.as_str())
                    .is_some_and(|id| !id.is_empty());
            if is_clawdesk_ref {
                ChannelTokenState::Managed
            } else {
                ChannelTokenState::External
            }
        }
        // A present non-ref value (plaintext string, number, ...) — never
        // exposed, never overwritten by ClawDesk.
        _ => ChannelTokenState::External,
    }
}

// --- channels list / status rows (fail-soft) ----------------------------------------

/// One row of `openclaw channels list --all --json`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelRow {
    pub id: String,
    pub installed: bool,
    pub configured: bool,
    pub enabled: bool,
}

/// Parses one channel list row (fail-soft: `id` required, row dropped
/// otherwise; boolean fields default to false).
pub fn parse_channel_list_row(raw: &serde_json::Value) -> Option<ChannelRow> {
    let id = raw.get("id")?.as_str()?.to_string();
    if id.is_empty() {
        return None;
    }
    Some(ChannelRow {
        id,
        installed: raw
            .get("installed")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        configured: raw
            .get("configured")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        enabled: raw
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
    })
}

/// One per-channel runtime row of `openclaw channels status --json`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelStatusRow {
    pub id: String,
    /// Raw runtime state string (unknown values kept; null when absent).
    pub state: Option<String>,
}

/// The parsed `channels status --json` document.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelStatus {
    /// `false` when the field is absent or false (config-only fallback).
    pub gateway_reachable: bool,
    pub rows: Vec<ChannelStatusRow>,
}

/// Parses the `channels status --json` document (fail-soft: missing
/// `gatewayReachable`/`false` → false; `id`-less rows dropped).
pub fn parse_channel_status(value: &serde_json::Value) -> ChannelStatus {
    let object = value.as_object();
    let gateway_reachable = object
        .and_then(|map| map.get("gatewayReachable"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let rows = object
        .and_then(|map| map.get("channels"))
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|raw| {
                    let id = raw.get("id")?.as_str()?.to_string();
                    if id.is_empty() {
                        return None;
                    }
                    Some(ChannelStatusRow {
                        id,
                        state: raw
                            .get("state")
                            .and_then(|v| v.as_str())
                            .map(str::to_string),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    ChannelStatus {
        gateway_reachable,
        rows,
    }
}

// --- Pairing rows (fail-soft) --------------------------------------------------------

/// One pending pairing request (`openclaw pairing list <channel> --json`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingRequest {
    /// The pairing code (required — rows without it are dropped).
    pub code: String,
    /// The requesting sender (fail-soft, null when absent).
    pub sender: Option<String>,
}

/// Parses the pairing request rows (fail-soft: `code` required, row dropped
/// otherwise; sender is optional).
pub fn parse_pairing_requests(value: &serde_json::Value) -> Vec<PairingRequest> {
    let items: Vec<&serde_json::Value> = match value {
        serde_json::Value::Array(items) => items.iter().collect(),
        serde_json::Value::Object(map) => map
            .get("requests")
            .and_then(|v| v.as_array())
            .map(|items| items.iter().collect())
            .unwrap_or_default(),
        _ => return Vec::new(),
    };
    items
        .into_iter()
        .filter_map(|raw| {
            let code = raw.get("code")?.as_str()?.to_string();
            if code.is_empty() {
                return None;
            }
            Some(PairingRequest {
                code,
                sender: raw
                    .get("sender")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
            })
        })
        .collect()
}

/// A merged row of `get-channels` (list row + status row + config flag).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelSummary {
    pub id: String,
    pub installed: bool,
    pub configured: bool,
    pub enabled: bool,
    /// Raw runtime state from `channels status` (null when the gateway is
    /// unreachable / the row is absent — config-only state).
    pub runtime_state: Option<String>,
}

/// The `get-channels` response.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelsOverview {
    /// `false` → the UI must present config-based state only (no "connected"
    /// guess).
    pub gateway_reachable: bool,
    pub channels: Vec<ChannelSummary>,
}

/// API-key-style registration state of a channel token (store index view).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelTokenStatus {
    pub channel: String,
    pub registered: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- id / field validation -------------------------------------------------

    #[test]
    fn channel_id_accepts_supported_channels_only() {
        assert!(validate_channel_id("discord").is_ok());
        assert!(validate_channel_id("telegram").is_ok());
        for bad in [
            "slack",
            "Discord",
            "DISCORD",
            "",
            "discord ",
            " discord",
            "discord/telegram",
            "discord..",
        ] {
            let err = validate_channel_id(bad).expect_err(bad);
            assert_eq!(err.code, "channel-id-invalid", "{bad:?}");
        }
    }

    #[test]
    fn channel_key_ids_follow_exec_id_pattern() {
        use crate::domain::ports::secrets::is_valid_key_id;
        assert_eq!(channel_secret_key_id("discord"), "channels/discord/token");
        assert_eq!(
            channel_secret_key_id("telegram"),
            "channels/telegram/botToken"
        );
        assert!(is_valid_key_id(&channel_secret_key_id("discord")));
        assert!(is_valid_key_id(&channel_secret_key_id("telegram")));
    }

    #[test]
    fn channel_secret_ref_uses_clawdesk_alias() {
        let reference = channel_secret_ref("discord");
        assert_eq!(reference.source, "exec");
        assert_eq!(reference.provider, "clawdesk");
        assert_eq!(reference.id, "channels/discord/token");
        let reference = channel_secret_ref("telegram");
        assert_eq!(reference.id, "channels/telegram/botToken");
    }

    #[test]
    fn channel_config_paths() {
        assert_eq!(channel_token_path("discord"), "channels.discord.token");
        assert_eq!(channel_token_path("telegram"), "channels.telegram.botToken");
        assert_eq!(channel_section_path("discord"), "channels.discord");
        assert_eq!(
            channel_enabled_path("telegram"),
            "channels.telegram.enabled"
        );
        assert_eq!(
            channel_dm_policy_path("discord"),
            "channels.discord.dmPolicy"
        );
        assert_eq!(
            channel_allow_from_path("discord"),
            "channels.discord.allowFrom"
        );
        assert_eq!(
            channel_group_policy_path("telegram"),
            "channels.telegram.groupPolicy"
        );
    }

    #[test]
    fn token_validation_requires_non_empty_after_trim() {
        assert!(validate_channel_token("  token123  ").is_ok());
        for bad in ["", "   "] {
            let err = validate_channel_token(bad).expect_err(bad);
            assert_eq!(err.code, "channel-token-invalid", "{bad:?}");
        }
    }

    #[test]
    fn dm_policy_validation() {
        for ok in ["pairing", "allowlist", "open", "disabled"] {
            assert!(validate_dm_policy(ok).is_ok(), "{ok}");
        }
        for bad in ["", "Pairing", "openx", "allow-list"] {
            let err = validate_dm_policy(bad).expect_err(bad);
            assert_eq!(err.code, "dm-policy-invalid", "{bad:?}");
        }
    }

    #[test]
    fn group_policy_validation() {
        for ok in ["open", "allowlist", "disabled"] {
            assert!(validate_group_policy(ok).is_ok(), "{ok}");
        }
        for bad in ["", "Open", "allow-list", "pairing"] {
            let err = validate_group_policy(bad).expect_err(bad);
            assert_eq!(err.code, "group-policy-invalid", "{bad:?}");
        }
    }

    #[test]
    fn allow_from_entry_validation() {
        for ok in ["*", "1", "1234567890", &"1".repeat(32)] {
            assert!(validate_allow_from_entry(ok).is_ok(), "{ok:?}");
        }
        for bad in [
            "",
            "12a",
            "a1",
            &"1".repeat(33),
            "12 34",
            "-1",
            "+1",
            "1.5",
            "1e3",
            "1_2",
            "../x",
        ] {
            let err = validate_allow_from_entry(bad).expect_err(bad);
            assert_eq!(err.code, "allow-from-entry-invalid", "{bad:?}");
        }
    }

    #[test]
    fn dm_access_cross_rules() {
        assert!(validate_dm_access("pairing", &[]).is_ok());
        assert!(validate_dm_access("disabled", &[]).is_ok());
        assert!(validate_dm_access("allowlist", &["123".into()]).is_ok());
        assert!(validate_dm_access("open", &["*".into()]).is_ok());
        assert!(validate_dm_access("open", &["*".into(), "123".into()]).is_ok());

        let err = validate_dm_access("allowlist", &[]).expect_err("allowlist + empty");
        assert_eq!(err.code, "dm-access-inconsistent");
        let err = validate_dm_access("open", &["123".into()]).expect_err("open without *");
        assert_eq!(err.code, "dm-access-inconsistent");
    }

    #[test]
    fn pairing_code_validation() {
        for ok in ["abcd", "AB12CD34", "a_b-c9", &"x".repeat(64)] {
            assert!(validate_pairing_code(ok).is_ok(), "{ok:?}");
        }
        for bad in ["", "abc", "1 2", "a.b", &"x".repeat(65)] {
            let err = validate_pairing_code(bad).expect_err(bad);
            assert_eq!(err.code, "pairing-code-invalid", "{bad:?}");
        }
    }

    // --- token state classification ---------------------------------------------

    #[test]
    fn token_state_classification_three_way() {
        assert_eq!(
            classify_channel_token_state(None),
            ChannelTokenState::Absent
        );
        assert_eq!(
            classify_channel_token_state(Some("null")),
            ChannelTokenState::Absent
        );
        let managed = serde_json::json!({
            "source": "exec",
            "provider": "clawdesk",
            "id": "channels/discord/token"
        })
        .to_string();
        assert_eq!(
            classify_channel_token_state(Some(&managed)),
            ChannelTokenState::Managed
        );
        // External: plaintext (redacted by the CLI), foreign ref, other refs.
        for raw in [
            r#""***""#,
            r#""sk-external""#,
            r#"{"source":"exec","provider":"env","id":"DISCORD_BOT_TOKEN"}"#,
            r#"{"source":"file","provider":"x","id":"y"}"#,
            r#"{"source":"exec","provider":"clawdesk"}"#,
            "not json at all",
        ] {
            assert_eq!(
                classify_channel_token_state(Some(raw)),
                ChannelTokenState::External,
                "{raw}"
            );
        }
    }

    // --- channel config parser ----------------------------------------------------

    #[test]
    fn channel_config_section_absent_is_fail_soft_default() {
        let config = parse_channel_config(None);
        assert_eq!(config.enabled, None);
        assert_eq!(config.token_state, ChannelTokenState::Absent);
        assert_eq!(config.dm_policy, None);
        assert!(config.allow_from.is_empty());
        assert_eq!(config.group_policy, None);
    }

    #[test]
    fn channel_config_parses_full_shape() {
        let raw = serde_json::json!({
            "enabled": true,
            "token": {"source":"exec","provider":"clawdesk","id":"channels/discord/token"},
            "dmPolicy": "pairing",
            "allowFrom": ["1234567890", "*"],
            "groupPolicy": "allowlist",
            "applicationId": "999"
        })
        .to_string();
        let config = parse_channel_config(Some(&raw));
        assert_eq!(config.enabled, Some(true));
        assert_eq!(config.token_state, ChannelTokenState::Managed);
        assert_eq!(config.dm_policy.as_deref(), Some("pairing"));
        assert_eq!(config.allow_from, vec!["1234567890", "*"]);
        assert_eq!(config.group_policy.as_deref(), Some("allowlist"));
    }

    #[test]
    fn channel_config_external_token_classified_without_value() {
        let raw = serde_json::json!({
            "token": "***",
            "dmPolicy": "unknown-future-value",
            "allowFrom": "not-an-array",
            "groupPolicy": 5
        })
        .to_string();
        let config = parse_channel_config(Some(&raw));
        assert_eq!(config.token_state, ChannelTokenState::External);
        // Unknown raw values are kept (dmPolicy), non-matching shapes
        // degrade (allowFrom → empty, groupPolicy → null).
        assert_eq!(config.dm_policy.as_deref(), Some("unknown-future-value"));
        assert!(config.allow_from.is_empty());
        assert_eq!(config.group_policy, None);
        assert_eq!(config.enabled, None);
    }

    #[test]
    fn channel_config_non_object_section_fails_soft() {
        for raw in [r#""a string""#, "42", "true", "broken {"] {
            let config = parse_channel_config(Some(raw));
            assert_eq!(config.token_state, ChannelTokenState::Absent, "{raw}");
            assert_eq!(config.enabled, None, "{raw}");
        }
    }

    // --- list / status / pairing row parsers ----------------------------------------

    #[test]
    fn channel_list_row_parser_fail_soft() {
        let row = parse_channel_list_row(&serde_json::json!({
            "id": "discord",
            "installed": true,
            "configured": false,
            "enabled": true
        }))
        .expect("row");
        assert_eq!(row.id, "discord");
        assert!(row.installed);
        assert!(!row.configured);
        assert!(row.enabled);

        // Missing booleans default to false; id-less/empty rows drop.
        let row = parse_channel_list_row(&serde_json::json!({"id": "telegram"})).expect("row");
        assert!(!row.installed && !row.configured && !row.enabled);
        assert_eq!(
            parse_channel_list_row(&serde_json::json!({"installed": true})),
            None
        );
        assert_eq!(parse_channel_list_row(&serde_json::json!({"id": ""})), None);
        assert_eq!(parse_channel_list_row(&serde_json::json!({"id": 5})), None);
    }

    #[test]
    fn channel_status_parser_gateway_and_rows_fail_soft() {
        let status = parse_channel_status(&serde_json::json!({
            "ok": true,
            "gatewayReachable": true,
            "channels": [
                {"id": "discord", "state": "connected"},
                {"id": "telegram"},
                {"state": "no-id-row"}
            ]
        }));
        assert!(status.gateway_reachable);
        assert_eq!(status.rows.len(), 2, "id-less row dropped");
        assert_eq!(status.rows[0].id, "discord");
        assert_eq!(status.rows[0].state.as_deref(), Some("connected"));
        assert_eq!(status.rows[1].id, "telegram");
        assert_eq!(status.rows[1].state, None);
    }

    #[test]
    fn channel_status_parser_missing_gateway_is_config_only() {
        for doc in [
            serde_json::json!({}),
            serde_json::json!({"gatewayReachable": false}),
        ] {
            let status = parse_channel_status(&doc);
            assert!(!status.gateway_reachable, "{doc}");
            assert!(status.rows.is_empty());
        }
        // Unknown raw state strings are kept verbatim.
        let status = parse_channel_status(&serde_json::json!({
            "gatewayReachable": true,
            "channels": [{"id": "discord", "state": "some-future-state"}]
        }));
        assert_eq!(status.rows[0].state.as_deref(), Some("some-future-state"));
    }

    #[test]
    fn pairing_request_parser_requires_code() {
        let requests = parse_pairing_requests(&serde_json::json!({
            "ok": true,
            "requests": [
                {"code": "AB12CD34", "sender": "user-1"},
                {"sender": "no code — dropped"},
                {"code": ""},
                "not-an-object"
            ]
        }));
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].code, "AB12CD34");
        assert_eq!(requests[0].sender.as_deref(), Some("user-1"));

        // Bare-array shape also works; sender-less rows are kept.
        let requests = parse_pairing_requests(&serde_json::json!([
            {"code": "EF56GH78"}
        ]));
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].sender, None);

        assert!(parse_pairing_requests(&serde_json::json!("nope")).is_empty());
    }
}
