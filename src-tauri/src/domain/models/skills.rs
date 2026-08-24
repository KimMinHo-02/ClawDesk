//! Domain types for the Phase 4 skills/plugins feature.
//!
//! Wire shapes are camelCase (architecture §5). Row fields that the CLI may
//! omit are `Option` (fail-soft): a row is kept as long as it has its
//! identifying key, everything else may be `null` (contract §1/§2).

use crate::error::AppError;

/// One row of `openclaw skills list --json` (read-only).
///
/// `enabled`/`eligible` are the skill's configured state and its load-time
/// eligibility (`metadata.openclaw.requires` gating). Both are `Option` so a
/// row missing them is still displayed (fail-soft, no row drop).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillRow {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eligible: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Load source (`workspace`, `bundled`, ...), when the CLI reports one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// One row of `openclaw plugins list --json` (cold read of the local
/// plugin registry + manifest fallback).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginRow {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Plugin format as reported by the CLI (e.g. `module`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
    /// Origin/source of the plugin, when reported.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Package dependency install state, when reported.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependency_status: Option<String>,
}

/// The live runtime surface of one plugin
/// (`openclaw plugins inspect <id> --runtime --json`).
///
/// Surface arrays are the registered names; a missing surface is an empty
/// array (never an error). `diagnostics` is optional.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PluginRuntime {
    pub id: String,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub hooks: Vec<String>,
    #[serde(default)]
    pub services: Vec<String>,
    #[serde(default)]
    pub cli_commands: Vec<String>,
    #[serde(default)]
    pub gateway_methods: Vec<String>,
    #[serde(default)]
    pub routes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<Vec<String>>,
}

// --- Input validation (S2: validate before any argv/config-path use) ---------

/// Validates a skill name. The name is interpolated into a config path
/// (`skills.entries.<name>.enabled`), so it must satisfy the Phase 3 entry
/// id rules: starts alphanumeric, then `[A-Za-z0-9._-]`, max 128 chars,
/// no `/`, `:`, whitespace, or `..` traversal.
pub fn validate_skill_name(name: &str) -> Result<(), AppError> {
    let bytes = name.as_bytes();
    let ok = matches!(bytes.first(), Some(c) if c.is_ascii_alphanumeric())
        && bytes.len() <= 128
        && bytes
            .iter()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
        && !name.contains("..");
    if ok {
        Ok(())
    } else {
        Err(AppError::skill_name_invalid(name))
    }
}

/// Validates a plugin id (npm style, Phase 4 contract pattern
/// `^(@[A-Za-z0-9][A-Za-z0-9._-]*/)?[A-Za-z0-9][A-Za-z0-9._-]{0,255}$`).
///
/// The id is only ever used as a single argv element (never in a config
/// path), but the same safe character set is enforced: no whitespace,
/// no leading dot, no `..` traversal, no extra `/`.
pub fn validate_plugin_id(id: &str) -> Result<(), AppError> {
    let ok = if let Some(rest) = id.strip_prefix('@') {
        // Scoped form: `@<scope>/<name>`.
        match rest.split_once('/') {
            Some((scope, name)) => validate_npm_segment(scope) && validate_npm_name(name),
            None => false,
        }
    } else {
        validate_npm_name(id)
    };
    if ok {
        Ok(())
    } else {
        Err(AppError::plugin_id_invalid(id))
    }
}

/// A scope segment: alphanumeric start, then `[A-Za-z0-9._-]`, no `..`.
fn validate_npm_segment(segment: &str) -> bool {
    let bytes = segment.as_bytes();
    matches!(bytes.first(), Some(c) if c.is_ascii_alphanumeric())
        && bytes
            .iter()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
        && !segment.contains("..")
}

/// A plugin name: alphanumeric start, then `[A-Za-z0-9._-]{0,255}`,
/// no `/`, no `..`.
fn validate_npm_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    matches!(bytes.first(), Some(c) if c.is_ascii_alphanumeric())
        && bytes.len() <= 256
        && !name.contains('/')
        && bytes
            .iter()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
        && !name.contains("..")
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- skill name -----------------------------------------------------------

    #[test]
    fn skill_name_accepts_normal_names() {
        for name in [
            "weather",
            "a",
            "My-Skill",
            "my.skill",
            "skill_1",
            "s1234567890123456789",
            &"a".repeat(128),
        ] {
            assert!(validate_skill_name(name).is_ok(), "{name}");
        }
    }

    #[test]
    fn skill_name_rejects_traversal_and_injection() {
        for name in [
            "",
            "..",
            "../evil",
            "a/b",
            "a:b",
            "a b",
            "a;b",
            "$(rm -rf)",
            "a\nb",
            ".hidden",
            "-dash",
            &"x".repeat(129),
        ] {
            let err = validate_skill_name(name).expect_err(&format!("{name:?} must be rejected"));
            assert_eq!(err.code, "skill-name-invalid", "{name:?}");
        }
    }

    // --- plugin id -------------------------------------------------------------

    #[test]
    fn plugin_id_accepts_npm_style_ids() {
        for id in [
            "discord",
            "a",
            "My-Plugin",
            "my.plugin",
            "p_1",
            "@openclaw/discord",
            "@openclaw/skill-hub",
            "@my.scope/plugin",
            &"a".repeat(256),
        ] {
            assert!(validate_plugin_id(id).is_ok(), "{id}");
        }
    }

    #[test]
    fn plugin_id_rejects_traversal_and_injection() {
        for id in [
            "",
            "..",
            "../evil",
            "a b",
            "a;b",
            "@scope",
            "@scope/",
            "@.scope/plugin",
            "@scope/.hidden",
            "@a/b/c",
            "scope/plugin",
            "a\nb",
            ".hidden",
            "-dash",
            "@sc_ope/",
            &"x".repeat(257),
        ] {
            let err = validate_plugin_id(id).expect_err(&format!("{id:?} must be rejected"));
            assert_eq!(err.code, "plugin-id-invalid", "{id:?}");
        }
    }

    // --- wire shapes -------------------------------------------------------------

    #[test]
    fn skill_row_wire_is_camel_case_with_optional_fields() {
        let row = SkillRow {
            name: "weather".into(),
            enabled: Some(true),
            eligible: Some(true),
            description: None,
            source: None,
        };
        let json = serde_json::to_value(&row).expect("serialize");
        assert_eq!(json["name"], "weather");
        assert_eq!(json["enabled"], true);
        assert!(json.get("description").is_none(), "null omitted on wire");
        let row: SkillRow = serde_json::from_value(json).expect("deserialize");
        assert!(row.description.is_none());
    }

    #[test]
    fn plugin_runtime_surfaces_default_to_empty() {
        let value = serde_json::json!({ "id": "@openclaw/discord" });
        let runtime: PluginRuntime = serde_json::from_value(value).expect("parse");
        assert_eq!(runtime.id, "@openclaw/discord");
        assert!(runtime.tools.is_empty());
        assert!(runtime.routes.is_empty());
        assert!(runtime.diagnostics.is_none());
    }
}
