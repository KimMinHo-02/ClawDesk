//! Parsing OpenClaw CLI output.
//!
//! Payload shapes follow the fake CLI contract fixtures in
//! `tests/fixtures/openclaw/` (latest stable based).

use crate::domain::models::openclaw::{GatewayStatus, OpenClawVersion, UpdateState};
use crate::error::AppError;
use crate::infrastructure::masking::mask_secrets;

/// Parses `openclaw --version` stdout.
///
/// The real CLI (npm `openclaw` launcher) prints `OpenClaw <version>`, or
/// `OpenClaw <version> (<commit>)` for git-source builds. The first
/// non-empty line must therefore start with `OpenClaw` followed by a version
/// string; the optional `(<commit>)` tail is dropped and the version is kept
/// exactly as printed.
pub fn parse_version_output(stdout: &str) -> Result<OpenClawVersion, AppError> {
    let line = stdout
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    if line.is_empty() {
        return Err(AppError::openclaw_version_parse("output was empty"));
    }
    let Some(after_prefix) = line.strip_prefix("OpenClaw") else {
        return Err(AppError::openclaw_version_parse(format!(
            "unexpected version line: {}",
            mask_secrets(line)
        )));
    };
    // Drop the optional `(<commit>)` suffix that git-source builds print.
    let version = after_prefix
        .split_once('(')
        .map_or(after_prefix, |(version, _)| version)
        .trim();
    if !is_version_string(version) {
        return Err(AppError::openclaw_version_parse(format!(
            "unexpected version format: {}",
            mask_secrets(line)
        )));
    }
    Ok(OpenClawVersion {
        raw: version.to_string(),
    })
}

/// Returns true when `value` is exactly a version string.
pub fn is_version_string(value: &str) -> bool {
    let value = value.trim();
    let (core, suffix) = match value.split_once(['-', '+']) {
        Some((core, suffix)) => (core, Some(suffix)),
        None => (value, None),
    };
    let mut parts = core.split('.');
    let (major, minor, patch) = (parts.next(), parts.next(), parts.next());
    if parts.next().is_some() {
        return false;
    }
    let digits = |part: Option<&str>| matches!(part, Some(part) if !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()));
    match (digits(major), digits(minor), digits(patch)) {
        (true, true, true) => matches_suffix(suffix),
        _ => false,
    }
}

fn matches_suffix(suffix: Option<&str>) -> bool {
    match suffix {
        Some(suffix) => {
            let pre_release = suffix.split('+').next().unwrap_or(suffix);
            !pre_release.is_empty()
                && pre_release
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
        }
        None => true,
    }
}

/// Payload for `openclaw gateway status --json` (latest stable CLI contract).
///
/// `ok` is true only when at least one gateway target is reachable — the CLI
/// still emits this JSON and exits with status 1 when nothing is reachable.
/// `targets` always carries at least one probe target.
#[derive(serde::Deserialize)]
struct GatewayPayload {
    ok: bool,
    #[serde(rename = "primaryTargetId", default)]
    primary_target_id: Option<String>,
    targets: Vec<GatewayTargetPayload>,
}

#[derive(serde::Deserialize)]
struct GatewayTargetPayload {
    id: Option<String>,
    url: Option<String>,
    connect: Option<GatewayConnectPayload>,
    server: Option<GatewayServerPayload>,
}

#[derive(serde::Deserialize)]
struct GatewayConnectPayload {
    ok: Option<bool>,
}

#[derive(serde::Deserialize)]
struct GatewayServerPayload {
    version: Option<String>,
}

/// Picks the target `primaryTargetId` refers to, else the first reachable
/// target, else the first target.
fn select_primary_target(payload: &GatewayPayload) -> Option<&GatewayTargetPayload> {
    let by_id = payload
        .primary_target_id
        .as_deref()
        .filter(|id| !id.trim().is_empty())
        .and_then(|id| {
            payload
                .targets
                .iter()
                .find(|target| target.id.as_deref() == Some(id))
        });
    if let Some(target) = by_id {
        return Some(target);
    }
    let reachable = payload
        .targets
        .iter()
        .find(|target| target.connect.as_ref().and_then(|connect| connect.ok) == Some(true));
    reachable.or_else(|| payload.targets.first())
}

/// Extracts the port from a gateway url such as `ws://127.0.0.1:18789`.
fn port_from_gateway_url(url: &str) -> Option<u16> {
    let authority = url.split_once("://")?.1.split(['/', '?', '#']).next()?;
    let port = if let Some((_, port)) = authority.rsplit_once("]:") {
        port
    } else {
        authority.rsplit_once(':')?.1
    };
    port.trim().parse::<u16>().ok().filter(|port| *port > 0)
}

/// Parses `openclaw gateway status --json` stdout.
///
/// `state` is derived from the payload `ok` flag (`"running"` / `"stopped"`);
/// version and port come from the primary target.
pub fn parse_gateway_json(stdout: &str) -> Result<GatewayStatus, AppError> {
    let payload: GatewayPayload = serde_json::from_str(stdout)
        .map_err(|err| AppError::openclaw_gateway_parse(mask_secrets(&err.to_string())))?;
    let (version, port) = select_primary_target(&payload)
        .map(|target| {
            (
                target
                    .server
                    .as_ref()
                    .and_then(|server| server.version.clone())
                    .filter(|version| !version.trim().is_empty()),
                target.url.as_deref().and_then(port_from_gateway_url),
            )
        })
        .unwrap_or((None, None));
    Ok(GatewayStatus {
        state: if payload.ok { "running" } else { "stopped" }.to_string(),
        version,
        port,
    })
}

/// Payload for `openclaw update status --json`.
#[derive(serde::Deserialize)]
struct UpdatePayload {
    current: Option<String>,
    latest: Option<String>,
    #[serde(rename = "updateAvailable", default)]
    update_available: Option<bool>,
}

/// Parses `openclaw update status --json` stdout.
///
/// Any parse failure or missing data resolves to `UpdateState::Unknown`
/// (the state cannot be determined, which is a valid structured answer).
pub fn parse_update_json(stdout: &str) -> UpdateState {
    let payload: UpdatePayload = match serde_json::from_str(stdout) {
        Ok(payload) => payload,
        Err(_) => return UpdateState::Unknown,
    };
    let (Some(current), Some(latest)) = (payload.current, payload.latest) else {
        return UpdateState::Unknown;
    };
    let available = payload.update_available.unwrap_or(current != latest);
    if available {
        UpdateState::UpdateAvailable
    } else {
        UpdateState::Updated
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_parses_simple() {
        let version = parse_version_output("OpenClaw 2026.7.1-2\n").unwrap();
        assert_eq!(version.raw, "2026.7.1-2");
    }

    #[test]
    fn version_parses_with_commit_suffix() {
        let version = parse_version_output("OpenClaw 2026.8.1 (abc1234)\n").unwrap();
        assert_eq!(version.raw, "2026.8.1");
    }

    #[test]
    fn version_parses_with_noise_before() {
        let version = parse_version_output("  OpenClaw 2026.7.1-2\n").unwrap();
        assert_eq!(version.raw, "2026.7.1-2");
    }

    #[test]
    fn version_parses_prerelease() {
        assert!(is_version_string("2026.8.0-beta.1"));
        assert!(is_version_string("2026.8.0+build5"));
    }

    #[test]
    fn version_rejects_bare_version_without_launcher_prefix() {
        let err = parse_version_output("2026.7.1-2\n").unwrap_err();
        assert_eq!(err.code, "openclaw-version-parse");
    }

    #[test]
    fn version_rejects_malformed() {
        let err = parse_version_output("OpenClaw (build 8765) channel=stable revision=abc123\n")
            .unwrap_err();
        assert_eq!(err.code, "openclaw-version-parse");
    }

    #[test]
    fn version_rejects_empty() {
        let err = parse_version_output("").unwrap_err();
        assert_eq!(err.code, "openclaw-version-parse");
    }

    #[test]
    fn version_string_validation() {
        assert!(is_version_string("1.0.0"));
        assert!(!is_version_string("1.0"));
        assert!(!is_version_string("1.0.0.0"));
        assert!(!is_version_string("one.2.3"));
        assert!(!is_version_string("v1.0.0"));
        assert!(!is_version_string(""));
    }

    /// Official-shape payload with the local loopback gateway reachable.
    const GATEWAY_RUNNING: &str = r#"{"ok":true,"degraded":false,"capability":"read_only","ts":1784000000000,"durationMs":12,"timeoutMs":3000,"primaryTargetId":"localLoopback","warnings":[],"network":{},"discovery":{"timeoutMs":1200,"count":0,"beacons":[]},"targets":[{"id":"localLoopback","kind":"localLoopback","url":"ws://127.0.0.1:18789","active":true,"tunnel":null,"connect":{"ok":true,"rpcOk":true,"scopeLimited":false,"latencyMs":12,"error":null,"close":null},"auth":{"role":"operator","scopes":["operator.read"],"capability":"read_only"},"server":{"version":"2026.7.1-2","connId":"c3f1e2a4"},"self":null,"config":null,"health":null,"summary":null,"presence":null}]}"#;

    /// Official-shape payload when no gateway target is reachable.
    const GATEWAY_STOPPED: &str = r#"{"ok":false,"degraded":false,"capability":"unknown","ts":1784000000000,"durationMs":3,"timeoutMs":3000,"primaryTargetId":null,"warnings":[{"code":"no_gateway_reachable","message":"no gateway target is reachable"}],"network":{},"discovery":{"timeoutMs":1200,"count":0,"beacons":[]},"targets":[{"id":"localLoopback","kind":"localLoopback","url":"ws://127.0.0.1:18789","active":false,"tunnel":null,"connect":{"ok":false,"rpcOk":false,"scopeLimited":false,"latencyMs":null,"error":"connect ECONNREFUSED 127.0.0.1:18789","close":null},"auth":null,"server":null,"self":null,"config":null,"health":null,"summary":null,"presence":null}]}"#;

    #[test]
    fn gateway_parses_running_payload() {
        let status = parse_gateway_json(GATEWAY_RUNNING).unwrap();
        assert_eq!(status.state, "running");
        assert_eq!(status.version.as_deref(), Some("2026.7.1-2"));
        assert_eq!(status.port, Some(18789));
    }

    #[test]
    fn gateway_parses_stopped_payload() {
        let status = parse_gateway_json(GATEWAY_STOPPED).unwrap();
        assert_eq!(status.state, "stopped");
        assert_eq!(status.version, None);
        assert_eq!(status.port, Some(18789));
    }

    #[test]
    fn gateway_falls_back_to_reachable_target_when_primary_id_unknown() {
        let payload = r#"{"ok":true,"primaryTargetId":"ghost","targets":[{"id":"a","url":"ws://127.0.0.1:1","connect":{"ok":false}},{"id":"b","url":"ws://127.0.0.1:2","connect":{"ok":true},"server":{"version":"1.0.0"}}]}"#;
        let status = parse_gateway_json(payload).unwrap();
        assert_eq!(status.state, "running");
        assert_eq!(status.version.as_deref(), Some("1.0.0"));
        assert_eq!(status.port, Some(2));
    }

    #[test]
    fn gateway_is_empty_when_payload_has_no_targets() {
        let status =
            parse_gateway_json(r#"{"ok":true,"primaryTargetId":null,"targets":[]}"#).unwrap();
        assert_eq!(status.state, "running");
        assert_eq!(status.version, None);
        assert_eq!(status.port, None);
    }

    #[test]
    fn gateway_rejects_non_json() {
        let err = parse_gateway_json("gateway status: ok (human text)").unwrap_err();
        assert_eq!(err.code, "openclaw-gateway-parse");
    }

    #[test]
    fn gateway_rejects_cli_error_envelope() {
        let err = parse_gateway_json(
            r#"{"ok":false,"error":{"type":"cli_error","message":"gateway status command failed"}}"#,
        )
        .unwrap_err();
        assert_eq!(err.code, "openclaw-gateway-parse");
    }

    #[test]
    fn gateway_url_port_extraction() {
        assert_eq!(port_from_gateway_url("ws://127.0.0.1:18789"), Some(18789));
        assert_eq!(
            port_from_gateway_url("wss://host.example:8443/x"),
            Some(8443)
        );
        assert_eq!(port_from_gateway_url("ws://[::1]:18789"), Some(18789));
        assert_eq!(port_from_gateway_url("ws://host.example"), None);
        assert_eq!(port_from_gateway_url("ws://host.example:0"), None);
        assert_eq!(port_from_gateway_url("not-a-gateway-url"), None);
    }

    #[test]
    fn update_parses_updated() {
        let state = parse_update_json(
            r#"{"current":"2026.7.1-2","latest":"2026.7.1-2","updateAvailable":false}"#,
        );
        assert_eq!(state, UpdateState::Updated);
    }

    #[test]
    fn update_parses_available() {
        let state = parse_update_json(
            r#"{"current":"2026.7.1","latest":"2026.7.1-2","updateAvailable":true}"#,
        );
        assert_eq!(state, UpdateState::UpdateAvailable);
    }

    #[test]
    fn update_infers_available_from_version_diff() {
        let state = parse_update_json(r#"{"current":"2026.7.1","latest":"2026.7.1-2"}"#);
        assert_eq!(state, UpdateState::UpdateAvailable);
    }

    #[test]
    fn update_is_unknown_on_non_json() {
        let state = parse_update_json("totally not json");
        assert_eq!(state, UpdateState::Unknown);
    }

    #[test]
    fn update_is_unknown_on_missing_fields() {
        let state = parse_update_json(r#"{"status":"ok"}"#);
        assert_eq!(state, UpdateState::Unknown);
    }
}
