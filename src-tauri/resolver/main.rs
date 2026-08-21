//! `clawdesk-secret-resolver` — the OpenClaw exec secret provider binary.
//!
//! OpenClaw runs this absolute binary (no shell) with the exec-protocol v1
//! JSON on stdin and expects the resolved values on stdout (jsonOnly). The
//! binary reads values from the ClawDesk secret store (DPAPI) and never
//! accepts secrets via argv/environment; per-id failures are reported in the
//! `errors` map with codes only (S3/S8).
//!
//! Exit codes: 0 = protocol response written (even with per-id errors),
//! 65 = protocol/usage error (bad request shape).

use std::io::{Read, Write};

use clawdesk_lib::domain::ports::secrets::{is_valid_key_id, SecretStorePort};
use clawdesk_lib::infrastructure::secrets::SecretStore;

const EXPECTED_PROVIDER: &str = "clawdesk";

/// Wire fields are camelCase (`protocolVersion`) per the exec protocol.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct Request {
    protocol_version: u32,
    provider: String,
    ids: Vec<String>,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct Response {
    protocol_version: u32,
    values: std::collections::BTreeMap<String, String>,
    #[serde(skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    errors: std::collections::BTreeMap<String, ErrorEntry>,
}

#[derive(serde::Serialize)]
struct ErrorEntry {
    code: &'static str,
}

fn main() {
    // The protocol request arrives on stdin; `CLAWDESK_RESOLVER_REQUEST` is
    // an equivalent test hook (lets contract tests run the binary without a
    // piped stdin through the shared ProcessRunner).
    let input = match std::env::var("CLAWDESK_RESOLVER_REQUEST") {
        Ok(request) if !request.is_empty() => request,
        _ => {
            let mut input = String::new();
            if let Err(err) = std::io::stdin().read_to_string(&mut input) {
                eprintln!("secret-resolver: cannot read stdin: {err:?}");
                std::process::exit(65);
            }
            input
        }
    };
    let request: Request = match serde_json::from_str(&input) {
        Ok(request) => request,
        Err(err) => {
            eprintln!("secret-resolver: invalid request: {err}");
            std::process::exit(65);
        }
    };
    if request.protocol_version != 1 {
        eprintln!("secret-resolver: unsupported protocolVersion");
        std::process::exit(65);
    }
    if request.provider != EXPECTED_PROVIDER {
        eprintln!("secret-resolver: unknown provider");
        std::process::exit(65);
    }

    let store = SecretStore::production();
    let mut values = std::collections::BTreeMap::new();
    let mut errors = std::collections::BTreeMap::new();
    for id in &request.ids {
        if !is_valid_key_id(id) {
            errors.insert(id.clone(), ErrorEntry { code: "INVALID_ID" });
            continue;
        }
        match store.get(id) {
            Ok(Some(value)) => {
                values.insert(id.clone(), value);
            }
            Ok(None) => {
                errors.insert(id.clone(), ErrorEntry { code: "NOT_FOUND" });
            }
            Err(err) => {
                // Code only — never the (masked) message or a value (S3).
                eprintln!("secret-resolver: store lookup failed: {}", err.code);
                errors.insert(
                    id.clone(),
                    ErrorEntry {
                        code: "STORE_UNAVAILABLE",
                    },
                );
            }
        }
    }

    let response = Response {
        protocol_version: 1,
        values,
        errors,
    };
    let mut stdout = std::io::stdout();
    match serde_json::to_string(&response) {
        Ok(body) => {
            let _ = writeln!(stdout, "{body}");
        }
        Err(err) => {
            eprintln!("secret-resolver: cannot encode response: {err}");
            std::process::exit(65);
        }
    }
}
