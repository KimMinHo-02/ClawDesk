//! Domain types for the Phase 8 profile/update/diagnostics feature.
//!
//! All types are read-only display shapes (PRODUCT_CONTRACT §4.7). Wire
//! shapes are camelCase (architecture §5); fields the CLI may omit are
//! `Option` (fail-soft — the display degrades, it never errors per row).

use crate::domain::models::openclaw::UpdateState;

/// `get-update-status` wire type: the Phase 1 `UpdateState` plus the
/// version strings, which is what PRODUCT_CONTRACT §4.7 calls "버전 차이".
///
/// `current`/`latest` are `None` whenever the state could not be
/// determined (fail-soft — same policy as Phase 1 `update_state`).
///
/// Output-only wire type (Serialize, like Phase 1 `GatewayStatus`): the
/// Phase 1 `UpdateState` is serialize-only and never crosses the wire
/// inbound.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct UpdateStatusDetail {
    pub state: UpdateState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest: Option<String>,
}

impl UpdateStatusDetail {
    /// The structured "cannot be determined" value (no error).
    pub fn unknown() -> Self {
        Self {
            state: UpdateState::Unknown,
            current: None,
            latest: None,
        }
    }
}

/// One row of `openclaw agents list --json` (read-only display).
///
/// `id` is the only required field; everything else may be `None`
/// (fail-soft, no row drop beyond missing `id`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentRow {
    pub id: String,
    /// `true` for the default agent (`main`).
    pub default: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emoji: Option<String>,
    /// The agent's workspace directory, when the CLI reports one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// Channel binding count, when the CLI reports one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bindings: Option<u64>,
}

/// One type-tagged event of `openclaw logs --limit <n> --json`
/// (line-delimited; the tag is `kind` on the wire).
///
/// Parsing is fail-soft per line: any line that is not a JSON object with a
/// recognized `type` (`log`/`meta`/`notice`/`raw`) becomes `Raw`, so a
/// partial/unknown payload always yields a viewable tail. `error`-typed
/// lines (stderr-only per the CLI docs) are kept as `Raw` too.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum LogEvent {
    /// A parsed log entry (optional identity fields fail soft).
    #[serde(rename_all = "camelCase")]
    Log {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        time: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        level: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        subsystem: Option<String>,
        message: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hostname: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        agent_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        channel: Option<String>,
    },
    /// Stream metadata (typically the first event of a tail).
    #[serde(rename_all = "camelCase")]
    Meta {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        file: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        source_kind: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        service: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cursor: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        size: Option<u64>,
    },
    /// A truncation/rotation hint.
    Notice {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        truncated: Option<bool>,
    },
    /// A line the parser could not classify (or an `error`-typed line).
    Raw { line: String },
}

/// `get-logs` wire type: the one-shot tail result.
///
/// An empty stdout (no log lines) is a successful zero-line result, not an
/// error. `source` is the log file from the first `meta` event, if any;
/// `truncated` is true when a `notice` event reports truncation.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogsResult {
    pub lines: Vec<LogEvent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub truncated: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_detail_wire_is_snake_single_words_and_omits_none() {
        let detail = UpdateStatusDetail {
            state: UpdateState::UpdateAvailable,
            current: Some("2026.7.1".into()),
            latest: Some("2026.7.1-2".into()),
        };
        let json = serde_json::to_value(&detail).expect("serialize");
        assert_eq!(json["state"], "update-available");
        assert_eq!(json["current"], "2026.7.1");
        assert_eq!(json["latest"], "2026.7.1-2");
        let unknown = UpdateStatusDetail::unknown();
        let json = serde_json::to_value(&unknown).expect("serialize");
        assert_eq!(json["state"], "unknown");
        assert!(json.get("current").is_none(), "None omitted on the wire");
        assert!(json.get("latest").is_none(), "None omitted on the wire");
    }

    #[test]
    fn agent_row_wire_is_camel_case_and_omits_none() {
        let row = AgentRow {
            id: "main".into(),
            default: true,
            name: Some("Main Agent".into()),
            emoji: None,
            workspace: None,
            bindings: Some(2),
        };
        let json = serde_json::to_value(&row).expect("serialize");
        assert_eq!(json["id"], "main");
        assert_eq!(json["default"], true);
        assert_eq!(json["name"], "Main Agent");
        assert_eq!(json["bindings"], 2);
        assert!(json.get("emoji").is_none(), "None omitted on the wire");
        assert!(json.get("workspace").is_none(), "None omitted on the wire");
        let row: AgentRow = serde_json::from_value(json).expect("deserialize");
        assert_eq!(
            row,
            AgentRow {
                id: "main".into(),
                default: true,
                name: Some("Main Agent".into()),
                emoji: None,
                workspace: None,
                bindings: Some(2),
            }
        );
    }

    #[test]
    fn log_event_wire_uses_kind_tag_and_camel_fields() {
        let event = LogEvent::Log {
            time: Some("2026-08-26T10:00:00Z".into()),
            level: Some("info".into()),
            subsystem: None,
            message: "gateway started".into(),
            hostname: None,
            agent_id: Some("main".into()),
            session_id: None,
            channel: None,
        };
        let json = serde_json::to_value(&event).expect("serialize");
        assert_eq!(json["kind"], "log");
        assert_eq!(json["agentId"], "main");
        assert!(json.get("subsystem").is_none());
        assert_eq!(
            serde_json::to_value(&LogEvent::Raw { line: "x".into() }).expect("serialize")["kind"],
            "raw"
        );
        assert_eq!(
            serde_json::to_value(&LogEvent::Notice {
                message: None,
                truncated: Some(true)
            })
            .expect("serialize")["kind"],
            "notice"
        );
    }

    #[test]
    fn logs_result_wire_shape() {
        let result = LogsResult {
            lines: vec![LogEvent::Raw {
                line: "hello".into(),
            }],
            source: Some("openclaw-2026-08-26.log".into()),
            truncated: true,
        };
        let json = serde_json::to_value(&result).expect("serialize");
        assert_eq!(json["lines"][0]["kind"], "raw");
        assert_eq!(json["source"], "openclaw-2026-08-26.log");
        assert_eq!(json["truncated"], true);
    }
}
