//! Domain types for the Phase 7 automations feature.
//!
//! ClawDesk manages automation job definitions only (list/detail/create/
//! edit/enable/remove through the `openclaw automations` CLI). Scheduling is
//! delegated to the OpenClaw Gateway; the command/script payload surface and
//! manual execution (`run`/`runs`) are non-goals.
//!
//! Parsers are fail-soft (contract §2): `id` is required (rows without it
//! are dropped), everything else degrades to `null` and unknown raw values
//! are kept — the live row/detail schemas are unverified items 1/2.

use crate::error::AppError;

/// The schedule kinds ClawDesk manages (contract: `at`/`every`/`cron` only —
/// `stream`/`on-exit` are non-goals).
pub const SCHEDULE_KINDS: [&str; 3] = ["at", "every", "cron"];

/// The payload kinds ClawDesk manages (contract: `reminder`/`task` only —
/// command/script payloads are non-goals).
pub const PAYLOAD_KINDS: [&str; 2] = ["reminder", "task"];

/// The reminder wake values.
pub const WAKE_VALUES: [&str; 2] = ["now", "next-heartbeat"];

/// Fixed session pairing per payload kind (contract §1: the IPC wire carries
/// no session field — the server fixes it):
/// reminder → `main` + `--system-event`, task → `isolated` + `--message`.
pub const REMINDER_SESSION: &str = "main";
pub const TASK_SESSION: &str = "isolated";

/// S2: a job id must be 1–64 chars of `[A-Za-z0-9._:-]` before any argv use.
pub fn validate_automation_id(id: &str) -> Result<(), AppError> {
    let bytes = id.as_bytes();
    let ok = (1..=64).contains(&bytes.len())
        && bytes
            .iter()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b':' | b'-'));
    if ok {
        Ok(())
    } else {
        Err(AppError::automation_id_invalid(id))
    }
}

/// S2: a job name must be non-empty after trimming, ≤128 chars, and contain
/// no control characters.
pub fn validate_automation_name(name: &str) -> Result<(), AppError> {
    let trimmed = name.trim();
    let ok = !trimmed.is_empty()
        && trimmed.chars().count() <= 128
        && trimmed.chars().all(|c| !c.is_control());
    if ok {
        Ok(())
    } else {
        Err(AppError::automation_name_invalid(name))
    }
}

/// S2: a schedule `{kind, value, tz}` before any argv use (contract §1):
/// - `kind` ∈ {at, every, cron}
/// - `at`: explicit UTC ISO 8601 only (`Z` or a `±offset`) — offset-less
///   datetimes are rejected (timezone interpretation is not delegated)
/// - `every`: `^[1-9][0-9]*[mhd]$`
/// - `cron`: 5/6 whitespace fields, each `^[\d*,/-]+$` (semantics are the
///   CLI's to validate)
/// - `tz`: cron-only, `^[A-Za-z0-9+/_-]{1,64}$`; at/every + tz → reject
pub fn validate_schedule(kind: &str, value: &str, tz: Option<&str>) -> Result<(), AppError> {
    let tz = tz.map(str::trim).filter(|t| !t.is_empty());
    match kind {
        "at" => {
            if tz.is_some() {
                return Err(AppError::automation_schedule_invalid(
                    "at schedule does not accept a timezone",
                ));
            }
            if !is_explicit_utc_iso8601(value) {
                return Err(AppError::automation_schedule_invalid(value));
            }
        }
        "every" => {
            if tz.is_some() {
                return Err(AppError::automation_schedule_invalid(
                    "every schedule does not accept a timezone",
                ));
            }
            if !is_every_interval(value) {
                return Err(AppError::automation_schedule_invalid(value));
            }
        }
        "cron" => {
            if !is_cron_expression(value) {
                return Err(AppError::automation_schedule_invalid(value));
            }
            if let Some(tz) = tz {
                if !is_iana_timezone(tz) {
                    return Err(AppError::automation_schedule_invalid(tz));
                }
            }
        }
        other => {
            return Err(AppError::automation_schedule_invalid(other));
        }
    }
    Ok(())
}

/// S2: a payload `{kind, text, wake}` before any argv use (contract §1):
/// text non-empty after trimming and ≤8000 chars; `kind` ∈ {reminder, task};
/// wake is reminder-only (task + wake → reject) and must be
/// `now`/`next-heartbeat`.
pub fn validate_automation_payload(
    kind: &str,
    text: &str,
    wake: Option<&str>,
) -> Result<(), AppError> {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.chars().count() > 8000 {
        return Err(AppError::automation_payload_invalid(
            "text must be 1-8000 characters",
        ));
    }
    match kind {
        "reminder" => match wake {
            None => Ok(()),
            Some(wake) if WAKE_VALUES.contains(&wake) => Ok(()),
            Some(wake) => Err(AppError::automation_payload_invalid(wake)),
        },
        "task" => match wake {
            None => Ok(()),
            Some(wake) => Err(AppError::automation_payload_invalid(&format!(
                "task payload does not accept --wake ({wake})"
            ))),
        },
        other => Err(AppError::automation_payload_invalid(other)),
    }
}

// --- schedule value checks (no external date/cron libraries — std only) ----------

/// Parses `YYYY-MM-DDTHH:MM:SS(.frac)?(Z|±HH:MM|±HHMM)` — explicit UTC ISO
/// 8601 only (offset-less datetimes are rejected).
fn is_explicit_utc_iso8601(value: &str) -> bool {
    let bytes = value.as_bytes();
    // Minimum layout: `YYYY-MM-DDTHH:MM:SSZ` (20 bytes, the zone starts at
    // index 19).
    if bytes.len() < 20 {
        return false;
    }
    let digits = |range: std::ops::Range<usize>| bytes[range].iter().all(|c| c.is_ascii_digit());
    let number = |range: std::ops::Range<usize>| -> u32 {
        std::str::from_utf8(&bytes[range])
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0)
    };
    if !digits(0..4) || bytes[4] != b'-' || !digits(5..7) || bytes[7] != b'-' || !digits(8..10) {
        return false;
    }
    if bytes[10] != b'T'
        || !digits(11..13)
        || bytes[13] != b':'
        || !digits(14..16)
        || bytes[16] != b':'
        || !digits(17..19)
    {
        return false;
    }
    let (year, month, day) = (number(0..4), number(5..7), number(8..10));
    let (hour, minute, second) = (number(11..13), number(14..16), number(17..19));
    if !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) {
        return false;
    }
    if hour > 23 || minute > 59 || second > 59 {
        return false;
    }
    let mut index = 19usize;
    if index < bytes.len() && bytes[index] == b'.' {
        index += 1;
        let start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        if index == start || index - start > 9 {
            return false;
        }
    }
    if index >= bytes.len() {
        return false;
    }
    match bytes[index] {
        b'Z' => index + 1 == bytes.len(),
        b'+' | b'-' => {
            index += 1;
            if bytes.len() - index < 2 || !digits(index..index + 2) {
                return false;
            }
            index += 2;
            // The colon between offset hours and minutes is optional
            // (`±HH:MM` or `±HHMM`).
            if index < bytes.len() && bytes[index] == b':' {
                index += 1;
            }
            if bytes.len() - index < 2 || !digits(index..index + 2) {
                return false;
            }
            index += 2;
            index == bytes.len()
        }
        _ => false,
    }
}

fn days_in_month(year: u32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

/// `^[1-9][0-9]*[mhd]$` — fixed interval without a leading zero.
fn is_every_interval(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 2
        && bytes[0].is_ascii_digit()
        && bytes[0] != b'0'
        && bytes[..bytes.len() - 1].iter().all(|c| c.is_ascii_digit())
        && matches!(bytes[bytes.len() - 1], b'm' | b'h' | b'd')
}

/// 5/6 whitespace fields, each `^[\d*,/-]+$` (semantics are the CLI's).
fn is_cron_expression(value: &str) -> bool {
    let fields = value.split_whitespace().collect::<Vec<_>>();
    matches!(fields.len(), 5 | 6)
        && fields.iter().all(|field| {
            !field.is_empty()
                && field
                    .bytes()
                    .all(|c| c.is_ascii_digit() || matches!(c, b'*' | b',' | b'/' | b'-'))
        })
}

/// `^[A-Za-z0-9+/_-]{1,64}$` with alphanumeric first/last characters —
/// IANA timezone id (existence is the CLI's).
fn is_iana_timezone(value: &str) -> bool {
    let bytes = value.as_bytes();
    (1..=64).contains(&bytes.len())
        && bytes[0].is_ascii_alphanumeric()
        && bytes[bytes.len() - 1].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'/' | b'_' | b'-'))
}

// --- row / detail parsers (fail-soft) ------------------------------------------------

/// Best-effort schedule view of a job (contract: kind/value/tz). Unknown
/// shapes (including non-goal kinds like `stream`) degrade to absent.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationScheduleView {
    pub kind: String,
    pub value: Option<String>,
    pub tz: Option<String>,
}

/// Best-effort payload view of a job (contract: kind/text).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationPayloadView {
    pub kind: String,
    pub text: Option<String>,
}

/// Best-effort schedule parse from a raw `schedule` field (fail-soft).
pub fn parse_schedule_view(raw: Option<&serde_json::Value>) -> Option<AutomationScheduleView> {
    let raw = raw?.as_object()?;
    let tz = raw.get("tz").and_then(|v| v.as_str()).map(str::to_string);
    let (kind, value_key) = match raw.get("kind").and_then(|v| v.as_str()) {
        Some("at") => ("at", "at"),
        Some("every") => ("every", "every"),
        Some("cron") => ("cron", "cron"),
        Some(_) => return None,
        None if raw.contains_key("at") => ("at", "at"),
        None if raw.contains_key("every") => ("every", "every"),
        None if raw.contains_key("cron") => ("cron", "cron"),
        None => return None,
    };
    let value = raw
        .get(value_key)
        .and_then(|v| v.as_str())
        .map(str::to_string);
    Some(AutomationScheduleView {
        kind: kind.to_string(),
        value,
        tz,
    })
}

/// Best-effort payload parse from a raw `payload` field (fail-soft).
pub fn parse_payload_view(raw: Option<&serde_json::Value>) -> Option<AutomationPayloadView> {
    let raw = raw?.as_object()?;
    let text = raw
        .get("text")
        .or_else(|| raw.get("message"))
        .or_else(|| raw.get("systemEvent"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let kind = match raw.get("kind").and_then(|v| v.as_str()) {
        Some("reminder") => "reminder",
        Some("task") => "task",
        Some(_) => return None,
        None if raw.contains_key("systemEvent") => "reminder",
        None if raw.contains_key("message") => "task",
        None => return None,
    };
    Some(AutomationPayloadView {
        kind: kind.to_string(),
        text,
    })
}

/// One row of `openclaw automations list --all --json` (fail-soft: `id`
/// required — rows without it are dropped; unknown values kept raw).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationJobRow {
    pub id: String,
    pub name: Option<String>,
    pub enabled: Option<bool>,
    /// Top-level `status` — unknown raw values are kept (UI "미확인").
    pub status: Option<String>,
    pub next_run_at_ms: Option<u64>,
    pub schedule: Option<AutomationScheduleView>,
    pub payload: Option<AutomationPayloadView>,
}

/// Parses one list row (fail-soft, `id` required).
pub fn parse_automation_job_row(raw: &serde_json::Value) -> Option<AutomationJobRow> {
    let raw = raw.as_object()?;
    let id = raw.get("id")?.as_str()?.to_string();
    if id.is_empty() {
        return None;
    }
    Some(AutomationJobRow {
        id,
        name: raw.get("name").and_then(|v| v.as_str()).map(str::to_string),
        enabled: raw.get("enabled").and_then(|v| v.as_bool()),
        status: raw
            .get("status")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        next_run_at_ms: raw.get("nextRunAtMs").and_then(|v| v.as_u64()),
        schedule: parse_schedule_view(raw.get("schedule")),
        payload: parse_payload_view(raw.get("payload")),
    })
}

/// The `get-automations` response wrapper (contract §5: `{jobs[]}`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationJobList {
    pub jobs: Vec<AutomationJobRow>,
}

/// The `create-automation` response (contract §5: `{jobId}`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationCreated {
    pub job_id: String,
}

/// The `openclaw automations get <id> --json` detail (fail-soft: unknown
/// fields/status kept raw, contract §2).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AutomationJob {
    pub id: String,
    pub name: Option<String>,
    pub enabled: Option<bool>,
    pub status: Option<String>,
    pub schedule: Option<AutomationScheduleView>,
    pub payload: Option<AutomationPayloadView>,
}

/// Locates the job object in a `get` response (fail-soft: the job may be the
/// document itself or nested under `job`/`automation`/`data` — unverified
/// item 2).
fn extract_job_object(value: &serde_json::Value) -> &serde_json::Value {
    if let Some(map) = value.as_object() {
        for key in ["job", "automation", "data"] {
            if let Some(inner) = map.get(key) {
                if inner.as_object().is_some() {
                    return inner;
                }
            }
        }
    }
    value
}

/// Parses the `get` detail (fail-soft: a missing `id` falls back to the
/// requested id — the CLI accepted it).
pub fn parse_automation_job(value: &serde_json::Value, requested_id: &str) -> AutomationJob {
    let object = extract_job_object(value);
    AutomationJob {
        id: object
            .get("id")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .filter(|id| !id.is_empty())
            .unwrap_or_else(|| requested_id.to_string()),
        name: object
            .get("name")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        enabled: object.get("enabled").and_then(|v| v.as_bool()),
        status: object
            .get("status")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        schedule: parse_schedule_view(object.get("schedule")),
        payload: parse_payload_view(object.get("payload")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- id validation -------------------------------------------------------------

    #[test]
    fn automation_id_accepts_slug_like_ids() {
        for ok in ["job-1", "a", "A.b_c:d-e", &"x".repeat(64)] {
            assert!(validate_automation_id(ok).is_ok(), "{ok:?}");
        }
    }

    #[test]
    fn automation_id_rejects_invalid_shapes() {
        for bad in [
            "",
            " ",
            "bad id",
            "id/",
            "id\\x",
            "id|",
            "id?",
            "id:space ",
            &"x".repeat(65),
        ] {
            let err = validate_automation_id(bad).expect_err(bad);
            assert_eq!(err.code, "automation-id-invalid", "{bad:?}");
        }
    }

    // --- name validation -------------------------------------------------------------

    #[test]
    fn automation_name_validation() {
        for ok in [
            "name",
            "한글 이름 with spaces  and “quotes”",
            &"x".repeat(128),
        ] {
            assert!(validate_automation_name(ok).is_ok(), "{ok:?}");
        }
        for bad in ["", "   ", "a\u{01}b", "a\u{007F}b", &"x".repeat(129)] {
            let err = validate_automation_name(bad).expect_err(bad);
            assert_eq!(err.code, "automation-name-invalid", "{bad:?}");
        }
    }

    // --- schedule validation ---------------------------------------------------------

    #[test]
    fn schedule_at_requires_explicit_utc_offset() {
        for ok in [
            "2027-02-01T16:00:00Z",
            "2027-02-01T16:00:00+09:00",
            "2027-02-01T16:00:00-0500",
            "2027-02-01T16:00:00.123Z",
            "2027-02-01T16:00:00.123456789Z",
        ] {
            assert!(validate_schedule("at", ok, None).is_ok(), "{ok}");
        }
        for bad in [
            "2027-02-01T16:00:00",       // offset-less
            "2027-02-01 16:00:00Z",      // space instead of T
            "2027-02-01",                // date only
            "2027-02-01T16:00Z",         // minute precision
            "2027-13-01T16:00:00Z",      // month 13
            "2027-02-31T16:00:00Z",      // Feb 31
            "2027-04-31T16:00:00Z",      // Apr 31
            "2027-02-29T16:00:00Z",      // non-leap Feb 29
            "2027-02-01T24:00:00Z",      // hour 24
            "2027-02-01T16:60:00Z",      // minute 60
            "2027-02-01T16:00:60Z",      // second 60
            "2027-02-01T16:00:00z",      // lowercase z
            "2027-02-01T16:00:00+9:00",  // 1-digit offset hour
            "2027-02-01T16:00:00Zextra", // trailing junk
            "2027-02-01T16:00:00.",      // empty fraction
        ] {
            let err = validate_schedule("at", bad, None).expect_err(bad);
            assert_eq!(err.code, "automation-schedule-invalid", "{bad:?}");
        }
        assert!(
            validate_schedule("at", "2028-02-29T16:00:00Z", None).is_ok(),
            "leap year"
        );
    }

    #[test]
    fn schedule_at_rejects_timezone() {
        for tz in ["Asia/Seoul", "UTC"] {
            let err = validate_schedule("at", "2027-02-01T16:00:00Z", Some(tz)).expect_err(tz);
            assert_eq!(err.code, "automation-schedule-invalid", "{tz:?}");
        }
        // An empty/blank tz counts as absent.
        for tz in ["", "   "] {
            assert!(
                validate_schedule("at", "2027-02-01T16:00:00Z", Some(tz)).is_ok(),
                "{tz:?}"
            );
        }
    }

    #[test]
    fn schedule_every_pattern() {
        for ok in ["1m", "10m", "1h", "24h", "1d", "30d"] {
            assert!(validate_schedule("every", ok, None).is_ok(), "{ok}");
        }
        for bad in [
            "", "0m", "01m", "1x", "m", "1 m", "+1m", "1.5m", "10M", "1 m",
        ] {
            let err = validate_schedule("every", bad, None).expect_err(bad);
            assert_eq!(err.code, "automation-schedule-invalid", "{bad:?}");
        }
        let err = validate_schedule("every", "10m", Some("Asia/Seoul")).expect_err("every + tz");
        assert_eq!(err.code, "automation-schedule-invalid");
    }

    #[test]
    fn schedule_cron_fields_and_tz() {
        for ok in [
            "0 9 * * 1",
            "*/5 * * * *",
            "0 9 * * 1 2027",
            "5,10 4,16 1,15 * 3-5",
            "0-30/5 * * * *",
        ] {
            assert!(validate_schedule("cron", ok, None).is_ok(), "{ok}");
        }
        for bad in [
            "",                // no fields
            "0 9 * *",         // 4 fields
            "0 9 * * 1 2 3",   // 7 fields
            "a 9 * * *",       // illegal char
            "0 9 * * * (mon)", // illegal chars
        ] {
            let err = validate_schedule("cron", bad, None).expect_err(bad);
            assert_eq!(err.code, "automation-schedule-invalid", "{bad:?}");
        }
        for tz in ["Asia/Seoul", "America/New_York", "UTC", "Etc/GMT+8"] {
            assert!(
                validate_schedule("cron", "0 9 * * 1", Some(tz)).is_ok(),
                "{tz:?}"
            );
        }
        // An empty/blank tz counts as absent (the same rule as `at`).
        for tz in ["", "   "] {
            assert!(
                validate_schedule("cron", "0 9 * * 1", Some(tz)).is_ok(),
                "{tz:?}"
            );
        }
        for bad_tz in ["Asia Seoul", "Asia/Seoul/", &"x".repeat(65)] {
            let err = validate_schedule("cron", "0 9 * * 1", Some(bad_tz)).expect_err(bad_tz);
            assert_eq!(err.code, "automation-schedule-invalid", "{bad_tz:?}");
        }
        let err = validate_schedule("stream", "whatever", None).expect_err("unknown kind");
        assert_eq!(err.code, "automation-schedule-invalid");
        let err = validate_schedule("on-exit", "x", None).expect_err("unknown kind");
        assert_eq!(err.code, "automation-schedule-invalid");
    }

    // --- payload validation ------------------------------------------------------------

    #[test]
    fn payload_validation_kinds_text_wake() {
        for ok in [
            (None, "now"),
            (Some("now"), "now"),
            (Some("next-heartbeat"), "next-heartbeat"),
        ] {
            assert!(
                validate_automation_payload("reminder", "약속", ok.0).is_ok(),
                "{:?}",
                ok.1
            );
        }
        assert!(validate_automation_payload("task", "보고서", None).is_ok());
        assert!(
            validate_automation_payload("reminder", &"x".repeat(8000), None).is_ok(),
            "exactly 8000 chars"
        );
        let too_long = "x".repeat(8001);
        let cases: Vec<(&str, &str, Option<&str>, &str)> = vec![
            ("reminder", "", None, "empty text"),
            ("reminder", "   ", Some("now"), "blank text"),
            ("reminder", &too_long, None, "too long"),
            ("task", "보고서", Some("now"), "task + wake"),
            (
                "task",
                "보고서",
                Some("next-heartbeat"),
                "task + wake (next-heartbeat)",
            ),
            ("command", "ls -la", None, "command kind"),
            ("script", "x", None, "script kind"),
            ("reminder", "text", Some("later"), "invalid wake"),
            ("", "text", None, "empty kind"),
        ];
        for (kind, text, wake, label) in cases {
            let err = validate_automation_payload(kind, text, wake).expect_err(label);
            assert_eq!(err.code, "automation-payload-invalid", "{label}");
        }
    }

    // --- schedule / payload views --------------------------------------------------------

    #[test]
    fn schedule_view_parses_explicit_and_inferred_kind() {
        let view = parse_schedule_view(Some(&serde_json::json!({
            "kind": "at",
            "at": "2027-02-01T16:00:00Z"
        })))
        .expect("view");
        assert_eq!(view.kind, "at");
        assert_eq!(view.value.as_deref(), Some("2027-02-01T16:00:00Z"));
        assert_eq!(view.tz, None);

        let view = parse_schedule_view(Some(&serde_json::json!({
            "cron": "0 9 * * 1",
            "tz": "Asia/Seoul"
        })))
        .expect("view");
        assert_eq!(view.kind, "cron");
        assert_eq!(view.value.as_deref(), Some("0 9 * * 1"));
        assert_eq!(view.tz.as_deref(), Some("Asia/Seoul"));

        let view = parse_schedule_view(Some(&serde_json::json!({
            "kind": "every",
            "every": "30m"
        })))
        .expect("view");
        assert_eq!(view.kind, "every");
        assert_eq!(view.value.as_deref(), Some("30m"));

        // Non-goal kinds / unknown shapes → absent (fail-soft).
        assert_eq!(
            parse_schedule_view(Some(&serde_json::json!({"kind": "stream"}))),
            None
        );
        assert_eq!(
            parse_schedule_view(Some(&serde_json::json!({"foo": "bar"}))),
            None
        );
        assert_eq!(
            parse_schedule_view(Some(&serde_json::json!("a string"))),
            None
        );
        assert_eq!(parse_schedule_view(None), None);
    }

    #[test]
    fn payload_view_parses_explicit_and_inferred_kind() {
        let view = parse_payload_view(Some(&serde_json::json!({
            "kind": "reminder",
            "text": "약속"
        })))
        .expect("view");
        assert_eq!(view.kind, "reminder");
        assert_eq!(view.text.as_deref(), Some("약속"));

        let view = parse_payload_view(Some(&serde_json::json!({
            "message": "보고서"
        })))
        .expect("view");
        assert_eq!(view.kind, "task");
        assert_eq!(view.text.as_deref(), Some("보고서"));

        let view = parse_payload_view(Some(&serde_json::json!({
            "systemEvent": "wake up"
        })))
        .expect("view");
        assert_eq!(view.kind, "reminder");
        assert_eq!(view.text.as_deref(), Some("wake up"));

        assert_eq!(
            parse_payload_view(Some(&serde_json::json!({"kind": "command"}))),
            None
        );
        assert_eq!(
            parse_payload_view(Some(&serde_json::json!({"foo": 1}))),
            None
        );
        assert_eq!(parse_payload_view(None), None);
    }

    // --- row / detail parsers --------------------------------------------------------------

    #[test]
    fn job_row_parser_fail_soft() {
        let row = parse_automation_job_row(&serde_json::json!({
            "id": "job-1",
            "name": "알림",
            "enabled": true,
            "status": "ok",
            "nextRunAtMs": 1798761600000u64,
            "schedule": {"kind": "at", "at": "2099-01-01T00:00:00Z"},
            "payload": {"kind": "reminder", "text": "약속", "wake": "now"},
            "unknownField": 42
        }))
        .expect("row");
        assert_eq!(row.id, "job-1");
        assert_eq!(row.name.as_deref(), Some("알림"));
        assert_eq!(row.enabled, Some(true));
        assert_eq!(row.status.as_deref(), Some("ok"));
        assert_eq!(row.next_run_at_ms, Some(1798761600000));
        assert_eq!(row.schedule.expect("schedule").kind, "at");
        assert_eq!(row.payload.expect("payload").kind, "reminder");

        // id-less / empty-id / non-string-id rows drop.
        assert_eq!(
            parse_automation_job_row(&serde_json::json!({"name": "n"})),
            None
        );
        assert_eq!(
            parse_automation_job_row(&serde_json::json!({"id": ""})),
            None
        );
        assert_eq!(
            parse_automation_job_row(&serde_json::json!({"id": 5})),
            None
        );
        assert_eq!(
            parse_automation_job_row(&serde_json::json!("not an object")),
            None
        );

        // Missing optional fields → null (fail-soft), unknown status raw kept.
        let row = parse_automation_job_row(&serde_json::json!({
            "id": "job-2",
            "status": "weird-future-status"
        }))
        .expect("row");
        assert_eq!(row.name, None);
        assert_eq!(row.enabled, None);
        assert_eq!(row.status.as_deref(), Some("weird-future-status"));
        assert_eq!(row.next_run_at_ms, None);
        assert_eq!(row.schedule, None);
        assert_eq!(row.payload, None);
    }

    #[test]
    fn job_detail_parser_fail_soft() {
        let job = parse_automation_job(
            &serde_json::json!({
                "ok": true,
                "job": {
                    "id": "job-1",
                    "name": "알림",
                    "enabled": true,
                    "status": "ok",
                    "schedule": {"kind": "cron", "cron": "0 9 * * 1", "tz": "Asia/Seoul"},
                    "payload": {"kind": "task", "text": "보고서"}
                }
            }),
            "job-1",
        );
        assert_eq!(job.id, "job-1");
        assert_eq!(job.name.as_deref(), Some("알림"));
        assert_eq!(job.enabled, Some(true));
        assert_eq!(job.status.as_deref(), Some("ok"));
        assert_eq!(
            job.schedule.expect("schedule").tz.as_deref(),
            Some("Asia/Seoul")
        );
        assert_eq!(job.payload.expect("payload").kind, "task");

        // Bare job object.
        let job = parse_automation_job(&serde_json::json!({"id": "job-2"}), "job-2");
        assert_eq!(job.id, "job-2");
        assert_eq!(job.name, None);

        // No id anywhere → the requested id (the CLI accepted it).
        let job = parse_automation_job(&serde_json::json!({"name": "x"}), "job-3");
        assert_eq!(job.id, "job-3");
    }
}
