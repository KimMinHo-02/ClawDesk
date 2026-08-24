//! Domain types for the Phase 5 tools / security feature.
//!
//! Wire shapes are camelCase (architecture §5). Policy fields the config may
//! omit are `Option`/empty arrays (fail-soft, contract §1): `tools.profile`
//! and `tools.exec.mode` read as `null` when unset, `tools.allow`/`tools.deny`
//! read as empty arrays.
//!
//! The builtin security profiles are read-only presets defined in code; they
//! are never persisted to the ClawDesk profile store.

use crate::error::AppError;

/// The current OpenClaw tool policy (`openclaw config get tools --json`,
/// redacted snapshot).
///
/// Fail-soft: missing fields are `null`/empty — the policy object itself is
/// never dropped because of a missing field.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ToolPolicy {
    /// `tools.profile` (`minimal`/`coding`/`messaging`/`full`), `null` when
    /// unset (unset behaves as `full`).
    #[serde(default)]
    pub profile: Option<String>,
    /// `tools.allow` entries (tool id / `group:*` / wildcard pattern).
    #[serde(default)]
    pub allow: Vec<String>,
    /// `tools.deny` entries — deny wins over allow.
    #[serde(default)]
    pub deny: Vec<String>,
    /// `tools.exec.mode` (`deny`/`allowlist`/`ask`/`auto`/`full`), `null`
    /// when unset (host default: no approval gate).
    #[serde(default)]
    pub exec_mode: Option<String>,
    /// `tools.elevated.enabled` — read-only display (no write surface).
    #[serde(default)]
    pub elevated_enabled: Option<bool>,
    /// `tools.fs.workspaceOnly` — read-only display (no write surface).
    #[serde(default)]
    pub fs_workspace_only: Option<bool>,
}

/// One finding of `openclaw security audit --json`.
///
/// `checkId` is the only required field (rows without it are dropped);
/// `severity`/`title`/`detail` are fail-soft → `null`. Unknown severity
/// values keep the raw string (the UI maps them to "unknown").
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityFinding {
    pub check_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// The result of a cold, read-only `openclaw security audit --json`.
///
/// `summary` has no asserted schema (informational display only — the live
/// shape is confirmed by the real-E2E baseline). `suppressed_count` is a
/// display-only count; suppressed finding details are never surfaced.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityAuditResult {
    pub summary: serde_json::Value,
    #[serde(default)]
    pub findings: Vec<SecurityFinding>,
    pub suppressed_count: u64,
}

/// A named tool-policy preset: two builtin profiles (read-only, in code)
/// plus user profiles stored in `%APPDATA%\ClawDesk\security-profiles.json`.
///
/// The store file holds tool policy only — no secrets (S3/S7).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityProfile {
    pub id: String,
    /// Display-only name (1–50 chars, no control characters).
    pub name: String,
    /// `tools.profile` enum: `minimal` | `coding` | `messaging` | `full`.
    pub base_profile: String,
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
    /// `tools.exec.mode` enum: `deny` | `allowlist` | `ask` | `auto` | `full`.
    pub exec_mode: String,
}

/// The two builtin security profiles (read-only, not persisted).
///
/// `default` mirrors the OpenClaw onboarding default (local config gets
/// `tools.profile: "coding"`, exec default no-approval). `hardened` mirrors
/// the docs Hardened baseline (messaging base, automation/runtime/fs
/// groups + session tools denied, exec `deny`).
pub fn builtin_profiles() -> Vec<SecurityProfile> {
    vec![
        SecurityProfile {
            id: "default".into(),
            name: "기본".into(),
            base_profile: "coding".into(),
            allow: Vec::new(),
            deny: Vec::new(),
            exec_mode: "full".into(),
        },
        SecurityProfile {
            id: "hardened".into(),
            name: "보안 강화".into(),
            base_profile: "messaging".into(),
            allow: Vec::new(),
            deny: vec![
                "group:automation".into(),
                "group:runtime".into(),
                "group:fs".into(),
                "sessions_spawn".into(),
                "sessions_send".into(),
            ],
            exec_mode: "deny".into(),
        },
    ]
}

pub const BUILTIN_PROFILE_IDS: [&str; 2] = ["default", "hardened"];

/// Whether `id` is a builtin profile id (builtins are immutable).
pub fn is_builtin_profile_id(id: &str) -> bool {
    BUILTIN_PROFILE_IDS.contains(&id)
}

/// Whether the current policy matches the given profile's four fields.
///
/// Normalization (docs: `full` = unset for both knobs):
/// - `tools.profile` unset ≡ `full`
/// - `tools.exec.mode` unset ≡ `full`
///
/// All four fields must match; any partial mismatch means "custom".
pub fn profile_applied(policy: &ToolPolicy, profile: &SecurityProfile) -> bool {
    let effective_profile = policy.profile.as_deref().unwrap_or("full");
    let effective_exec_mode = policy.exec_mode.as_deref().unwrap_or("full");
    effective_profile == profile.base_profile
        && effective_exec_mode == profile.exec_mode
        && policy.allow == profile.allow
        && policy.deny == profile.deny
}

/// Finds the first profile (in the given order) whose four fields match the
/// current policy; `None` means the policy is custom.
pub fn find_matching_profile(policy: &ToolPolicy, profiles: &[SecurityProfile]) -> Option<String> {
    profiles
        .iter()
        .find(|profile| profile_applied(policy, profile))
        .map(|profile| profile.id.clone())
}

// --- Input validation (S2: validate before any argv/config-path use) ---------

/// Validates the `tools.profile` enum value.
pub fn validate_tool_profile(profile: &str) -> Result<(), AppError> {
    match profile {
        "minimal" | "coding" | "messaging" | "full" => Ok(()),
        other => Err(AppError::tool_profile_invalid(other)),
    }
}

/// Validates one `tools.allow`/`tools.deny` entry.
///
/// Rules (contract §1): non-empty, ≤128 chars, character set
/// `[A-Za-z0-9_:.*-]` (no whitespace, no `/`, no `..` traversal); a
/// `group:` prefix must be exactly `group:[A-Za-z0-9-]{1,32}`.
pub fn validate_tool_entry(entry: &str) -> Result<(), AppError> {
    if is_valid_tool_entry(entry) {
        Ok(())
    } else {
        Err(AppError::tool_entry_invalid(entry))
    }
}

fn is_valid_tool_entry(entry: &str) -> bool {
    if entry.is_empty() || entry.len() > 128 {
        return false;
    }
    if let Some(rest) = entry.strip_prefix("group:") {
        return !rest.is_empty()
            && rest.len() <= 32
            && rest.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
    }
    entry
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b':' | b'.' | b'*' | b'-'))
        && !entry.contains("..")
}

/// Validates the `tools.exec.mode` enum value.
pub fn validate_exec_mode(mode: &str) -> Result<(), AppError> {
    match mode {
        "deny" | "allowlist" | "ask" | "auto" | "full" => Ok(()),
        other => Err(AppError::exec_mode_invalid(other)),
    }
}

/// Validates a user security profile id: `^[a-z][a-z0-9_-]{0,63}$`.
pub fn validate_profile_slug(id: &str) -> Result<(), AppError> {
    let bytes = id.as_bytes();
    let ok = matches!(bytes.first(), Some(c) if c.is_ascii_lowercase())
        && bytes.len() <= 64
        && bytes[1..]
            .iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'_' | b'-'));
    if ok {
        Ok(())
    } else {
        Err(AppError::security_profile_id_invalid(id))
    }
}

/// Validates a user security profile display name: 1–50 chars, no control
/// characters.
pub fn validate_profile_name(name: &str) -> Result<(), AppError> {
    let ok = !name.is_empty() && name.chars().count() <= 50 && !name.chars().any(char::is_control);
    if ok {
        Ok(())
    } else {
        Err(AppError::security_profile_name_invalid(name))
    }
}

/// Validates every field of a user security profile (before any store or
/// config I/O).
pub fn validate_profile(profile: &SecurityProfile) -> Result<(), AppError> {
    validate_profile_slug(&profile.id)?;
    validate_profile_name(&profile.name)?;
    validate_tool_profile(&profile.base_profile)?;
    validate_exec_mode(&profile.exec_mode)?;
    for entry in profile.allow.iter().chain(profile.deny.iter()) {
        validate_tool_entry(entry)?;
    }
    Ok(())
}

/// Parses the redacted `tools` section snapshot (`config get tools --json`)
/// into a `ToolPolicy` (fail-soft, contract §1): missing or non-matching
/// fields degrade to `null`/empty — the policy object is never dropped.
/// Unknown enum strings are kept raw (the write-side validators gate user
/// input, not the read).
pub fn parse_tool_policy(value: &serde_json::Value) -> ToolPolicy {
    let string_field = |v: &serde_json::Value| v.as_str().map(str::to_string);
    let string_list = |v: &serde_json::Value| -> Vec<String> {
        v.as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    let bool_field = |v: &serde_json::Value| v.as_bool();
    ToolPolicy {
        profile: string_field(&value["profile"]),
        allow: string_list(&value["allow"]),
        deny: string_list(&value["deny"]),
        exec_mode: string_field(&value["exec"]["mode"]),
        elevated_enabled: bool_field(&value["elevated"]["enabled"]),
        fs_workspace_only: bool_field(&value["fs"]["workspaceOnly"]),
    }
}

// --- `security audit --json` parsing (fail-soft, contract §3) ----------------

/// Parses the `findings` array: `checkId` is required (rows without it are
/// dropped); `severity`/`title`/`detail` fail soft to `null` (unknown
/// severity values keep the raw string).
pub fn parse_findings(value: &serde_json::Value) -> Vec<SecurityFinding> {
    let Some(items) = value.as_array() else {
        return Vec::new();
    };
    items.iter().filter_map(parse_finding).collect()
}

fn parse_finding(raw: &serde_json::Value) -> Option<SecurityFinding> {
    let check_id = raw.get("checkId")?.as_str()?.to_string();
    if check_id.is_empty() {
        return None;
    }
    let severity = raw
        .get("severity")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let title = raw
        .get("title")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let detail = raw
        .get("detail")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    Some(SecurityFinding {
        check_id,
        severity,
        title,
        detail,
    })
}

/// Computes the display-only suppressed-finding count (details are never
/// surfaced). Array → length; object → `count` field when numeric; else 0.
pub fn suppressed_count(value: &serde_json::Value) -> u64 {
    match value {
        serde_json::Value::Array(items) => items.len() as u64,
        serde_json::Value::Object(map) => map.get("count").and_then(|c| c.as_u64()).unwrap_or(0),
        _ => 0,
    }
}

/// Builds the audit result from the parsed JSON document (fail-soft):
/// missing `findings` → empty list, missing `summary` → `null`, missing
/// `suppressedFindings` → 0.
pub fn parse_audit_document(value: &serde_json::Value) -> SecurityAuditResult {
    SecurityAuditResult {
        summary: value
            .get("summary")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
        findings: parse_findings(&value["findings"]),
        suppressed_count: suppressed_count(&value["suppressedFindings"]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- ToolPolicy parsing ---------------------------------------------------

    #[test]
    fn tool_policy_parses_full_shape() {
        let body = r#"{
            "profile": "coding",
            "allow": ["web_search", "image*"],
            "deny": ["group:automation"],
            "exec": {"mode": "ask"},
            "elevated": {"enabled": true},
            "fs": {"workspaceOnly": true}
        }"#;
        let value: serde_json::Value = serde_json::from_str(body).expect("json");
        let policy = parse_tool_policy(&value);
        assert_eq!(policy.profile.as_deref(), Some("coding"));
        assert_eq!(policy.allow, vec!["web_search", "image*"]);
        assert_eq!(policy.deny, vec!["group:automation"]);
        assert_eq!(policy.exec_mode.as_deref(), Some("ask"));
        assert_eq!(policy.elevated_enabled, Some(true));
        assert_eq!(policy.fs_workspace_only, Some(true));
    }

    #[test]
    fn tool_policy_missing_fields_are_fail_soft() {
        // Nothing set: every field is null/empty (no drop).
        let policy = parse_tool_policy(&serde_json::Value::Null);
        assert_eq!(policy, ToolPolicy::default());
        let policy = parse_tool_policy(&serde_json::json!({}));
        assert_eq!(policy.profile, None);
        assert!(policy.allow.is_empty());
        assert!(policy.deny.is_empty());
        assert_eq!(policy.exec_mode, None);
        assert_eq!(policy.elevated_enabled, None);
        assert_eq!(policy.fs_workspace_only, None);
    }

    #[test]
    fn tool_policy_unknown_values_are_kept_raw() {
        // Unknown profile/mode strings are not rejected (fail-soft); the
        // write-side validators are the gate for user input.
        let value = serde_json::json!({
            "profile": "future-profile",
            "exec": {"mode": "future-mode"},
            "elevated": "unexpected",
            "fs": {"workspaceOnly": "yes"}
        });
        let policy = parse_tool_policy(&value);
        assert_eq!(policy.profile.as_deref(), Some("future-profile"));
        assert_eq!(policy.exec_mode.as_deref(), Some("future-mode"));
        assert_eq!(policy.elevated_enabled, None, "non-bool → null");
        assert_eq!(policy.fs_workspace_only, None, "non-bool → null");
    }

    #[test]
    fn tool_policy_non_array_lists_degrade_to_empty() {
        let value = serde_json::json!({"allow": "web_search", "deny": {"a": 1}});
        let policy = parse_tool_policy(&value);
        assert!(policy.allow.is_empty());
        assert!(policy.deny.is_empty());
    }

    #[test]
    fn tool_policy_wire_is_camel_case() {
        let policy = ToolPolicy {
            profile: Some("coding".into()),
            allow: vec!["web_search".into()],
            deny: Vec::new(),
            exec_mode: Some("ask".into()),
            elevated_enabled: Some(true),
            fs_workspace_only: None,
        };
        let json = serde_json::to_value(&policy).expect("serialize");
        assert_eq!(json["profile"], "coding");
        assert_eq!(json["allow"], serde_json::json!(["web_search"]));
        assert_eq!(json["execMode"], "ask");
        assert_eq!(json["elevatedEnabled"], true);
        // Unset read-only fields are explicit `null` on the wire (the UI
        // shows them as "not set", not as a missing field).
        assert_eq!(json["fsWorkspaceOnly"], serde_json::Value::Null);
        let round: ToolPolicy = serde_json::from_value(json).expect("deserialize");
        assert_eq!(round, policy);
    }

    // --- validators -------------------------------------------------------------

    #[test]
    fn tool_profile_accepts_the_four_enum_values() {
        for profile in ["minimal", "coding", "messaging", "full"] {
            assert!(validate_tool_profile(profile).is_ok(), "{profile}");
        }
    }

    #[test]
    fn tool_profile_rejects_unknown_and_empty() {
        for profile in [
            "", "Coding", "FULL", "default", "coding ", "cod ing", "../evil",
        ] {
            let err = validate_tool_profile(profile).expect_err(&format!("{profile:?}"));
            assert_eq!(err.code, "tool-profile-invalid", "{profile:?}");
        }
    }

    #[test]
    fn tool_entry_accepts_tool_ids_groups_and_wildcards() {
        for entry in [
            "web_search",
            "session_status",
            "image*",
            "outlook__*",
            "group:fs",
            "group:automation",
            "group:my-group",
            &"a".repeat(128),
            &format!("group:{}", "g".repeat(32)),
        ] {
            assert!(validate_tool_entry(entry).is_ok(), "{entry}");
        }
    }

    #[test]
    fn tool_entry_rejects_traversal_whitespace_and_bad_groups() {
        for entry in [
            "",
            "../evil",
            "a/b",
            "a b",
            "a\nb",
            "a;b",
            "$(rm)",
            "a*b c",
            &"a".repeat(129),
            "group:",
            "group:has space",
            &format!("group:{}", "g".repeat(33)),
        ] {
            let err = validate_tool_entry(entry).expect_err(&format!("{entry:?}"));
            assert_eq!(err.code, "tool-entry-invalid", "{entry:?}");
        }
    }

    #[test]
    fn exec_mode_accepts_the_five_enum_values() {
        for mode in ["deny", "allowlist", "ask", "auto", "full"] {
            assert!(validate_exec_mode(mode).is_ok(), "{mode}");
        }
    }

    #[test]
    fn exec_mode_rejects_unknown_and_empty() {
        for mode in ["", "Full", "deny-all", "ask ", "deny/all"] {
            let err = validate_exec_mode(mode).expect_err(&format!("{mode:?}"));
            assert_eq!(err.code, "exec-mode-invalid", "{mode:?}");
        }
    }

    #[test]
    fn profile_slug_accepts_lower_case_slugs() {
        for id in ["a", "my-profile", "profile_1", "x-2_z", &"a".repeat(64)] {
            assert!(validate_profile_slug(id).is_ok(), "{id}");
        }
    }

    #[test]
    fn profile_slug_rejects_bad_shapes() {
        for id in [
            "",
            "A",
            "My-Profile",
            "1abc",
            "-abc",
            "_abc",
            "a b",
            "a/b",
            "a.b",
            &"a".repeat(65),
        ] {
            let err = validate_profile_slug(id).expect_err(&format!("{id:?}"));
            assert_eq!(err.code, "security-profile-id-invalid", "{id:?}");
        }
    }

    #[test]
    fn profile_name_accepts_display_names() {
        for name in ["기본", "My Profile 1", &"한".repeat(50)] {
            assert!(validate_profile_name(name).is_ok(), "{name:?}");
        }
    }

    #[test]
    fn profile_name_rejects_empty_long_and_control() {
        for name in ["", &"한".repeat(51), "bad\nname", "tab\there"] {
            let err = validate_profile_name(name).expect_err(&format!("{name:?}"));
            assert_eq!(err.code, "security-profile-name-invalid", "{name:?}");
        }
    }

    #[test]
    fn validate_profile_checks_every_field() {
        let base = SecurityProfile {
            id: "my-profile".into(),
            name: "내 프로필".into(),
            base_profile: "coding".into(),
            allow: vec!["web_search".into()],
            deny: Vec::new(),
            exec_mode: "ask".into(),
        };
        assert!(validate_profile(&base).is_ok());

        let mut bad_id = base.clone();
        bad_id.id = "Bad-ID".into();
        assert_eq!(
            validate_profile(&bad_id).unwrap_err().code,
            "security-profile-id-invalid"
        );

        let mut bad_name = base.clone();
        bad_name.name = String::new();
        assert_eq!(
            validate_profile(&bad_name).unwrap_err().code,
            "security-profile-name-invalid"
        );

        let mut bad_base = base.clone();
        bad_base.base_profile = "nope".into();
        assert_eq!(
            validate_profile(&bad_base).unwrap_err().code,
            "tool-profile-invalid"
        );

        let mut bad_mode = base.clone();
        bad_mode.exec_mode = "nope".into();
        assert_eq!(
            validate_profile(&bad_mode).unwrap_err().code,
            "exec-mode-invalid"
        );

        let mut bad_allow = base.clone();
        bad_allow.allow.push("../evil".into());
        assert_eq!(
            validate_profile(&bad_allow).unwrap_err().code,
            "tool-entry-invalid"
        );

        let mut bad_deny = base.clone();
        bad_deny.deny.push("a b".into());
        assert_eq!(
            validate_profile(&bad_deny).unwrap_err().code,
            "tool-entry-invalid"
        );
    }

    // --- applied-state determination ---------------------------------------------

    fn policy(profile: Option<&str>, exec_mode: Option<&str>) -> ToolPolicy {
        ToolPolicy {
            profile: profile.map(str::to_string),
            allow: Vec::new(),
            deny: Vec::new(),
            exec_mode: exec_mode.map(str::to_string),
            elevated_enabled: None,
            fs_workspace_only: None,
        }
    }

    #[test]
    fn builtin_default_matches_unset_policy() {
        // unset profile ≡ full, unset exec.mode ≡ full — but `default` has
        // base `coding`, so a fully unset policy is NOT `default`.
        let default = builtin_profiles()[0].clone();
        assert!(!profile_applied(&policy(None, None), &default));
        let coding_full = policy(Some("coding"), None);
        assert!(profile_applied(&coding_full, &default));
        assert!(profile_applied(
            &policy(Some("coding"), Some("full")),
            &default
        ));
        // Explicit full profile + full exec mode matches `hardened`-less? No:
        // hardened denies groups, so it never matches an empty deny list.
        let hardened = builtin_profiles()[1].clone();
        assert!(!profile_applied(
            &policy(Some("messaging"), Some("deny")),
            &hardened
        ));
    }

    #[test]
    fn hardened_matches_only_its_exact_deny_set() {
        let hardened = builtin_profiles()[1].clone();
        let mut matching = policy(Some("messaging"), Some("deny"));
        matching.deny = hardened.deny.clone();
        assert!(profile_applied(&matching, &hardened));
        let mut partial = matching.clone();
        partial.deny.pop();
        assert!(
            !profile_applied(&partial, &hardened),
            "partial mismatch → custom"
        );
    }

    #[test]
    fn find_matching_profile_returns_first_match() {
        let profiles = builtin_profiles();
        let matched = policy(Some("coding"), None);
        assert_eq!(
            find_matching_profile(&matched, &profiles),
            Some("default".to_string())
        );
        let custom = policy(Some("messaging"), Some("deny"));
        assert_eq!(find_matching_profile(&custom, &profiles), None);
    }

    // --- audit findings parser ----------------------------------------------------

    #[test]
    fn findings_parser_drops_rows_without_check_id() {
        let value = serde_json::json!([
            {"checkId": "tools.exec.security_full_configured", "severity": "warn"},
            {"severity": "warn", "title": "no check id"},
            {"checkId": "", "title": "empty id"},
            {"checkId": 42},
            "not-an-object",
            {"checkId": "fs.config.perms_world_readable", "severity": "critical",
             "title": "World readable", "detail": "config file is 0644"}
        ]);
        let findings = parse_findings(&value);
        assert_eq!(findings.len(), 2, "only checkId rows survive");
        assert_eq!(findings[0].check_id, "tools.exec.security_full_configured");
        assert_eq!(findings[0].severity.as_deref(), Some("warn"));
        assert_eq!(findings[0].title, None);
        assert_eq!(findings[0].detail, None);
        assert_eq!(findings[1].severity.as_deref(), Some("critical"));
        assert_eq!(findings[1].title.as_deref(), Some("World readable"));
        assert_eq!(findings[1].detail.as_deref(), Some("config file is 0644"));
    }

    #[test]
    fn findings_parser_keeps_unknown_severity_raw() {
        let value = serde_json::json!([
            {"checkId": "tools.web.future", "severity": "experimental"}
        ]);
        let findings = parse_findings(&value);
        assert_eq!(findings[0].severity.as_deref(), Some("experimental"));
    }

    #[test]
    fn findings_parser_non_array_is_empty() {
        assert!(parse_findings(&serde_json::Value::Null).is_empty());
        assert!(parse_findings(&serde_json::json!({})).is_empty());
        assert!(parse_findings(&serde_json::json!("oops")).is_empty());
    }

    #[test]
    fn suppressed_count_handles_array_object_and_absent() {
        assert_eq!(suppressed_count(&serde_json::json!([1, 2, 3])), 3);
        assert_eq!(suppressed_count(&serde_json::json!({"count": 7})), 7);
        assert_eq!(suppressed_count(&serde_json::json!({"count": "x"})), 0);
        assert_eq!(suppressed_count(&serde_json::Value::Null), 0);
    }

    #[test]
    fn audit_document_parsing_is_fail_soft() {
        let value = serde_json::json!({
            "ok": true,
            "findings": [
                {"checkId": "gateway.exposure.open", "severity": "critical"},
                {"no": "id"}
            ],
            "summary": {"checked": 42},
            "suppressedFindings": [{"checkId": "x"}, {"checkId": "y"}]
        });
        let result = parse_audit_document(&value);
        assert_eq!(result.summary, serde_json::json!({"checked": 42}));
        assert_eq!(result.findings.len(), 1);
        assert_eq!(result.suppressed_count, 2);

        let empty = parse_audit_document(&serde_json::json!({}));
        assert_eq!(empty.summary, serde_json::Value::Null);
        assert!(empty.findings.is_empty());
        assert_eq!(empty.suppressed_count, 0);
    }
}
