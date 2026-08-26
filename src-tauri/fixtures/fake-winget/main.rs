//! A deterministic, std+serde_json fake "winget" for Phase 8.1 tests.
//!
//! Per-test configuration is read from the process environment (same
//! convention as `fake-openclaw`; adapter-driven scenario tests run
//! serialized because the adapter's spawns inherit it):
//!
//! - `CLAWDESK_FAKE_STATE`   sandbox dir holding `node.json`
//!   (`{"node":{"version":"..."}}`)
//! - `CLAWDESK_FAKE_CAPTURE` file receiving one JSON argv line per command
//! - `CLAWDESK_FAKE_BEHAVIOR` `normal` (default) | `noop` | `fail` | `sleep`
//!
//! Commands:
//! - `--version` → fake winget version (availability probe)
//! - the exact `install --id OpenJS.NodeJS.LTS ...` argv:
//!   - `normal`: sets `node.version` to the supported LTS in state, exit 0
//!   - `noop`:   exit 0, state unchanged (update ran but nothing changed)
//!   - `fail`:   stderr with a fake `sk-` token (masking check), exit 1
//!   - `sleep`:  sleeps 3s, exit 0 (runner-timeout scenarios)
//! - anything else (unknown flag, missing `--id`, wrong package id,
//!   `upgrade`, ...) → exit 2, exactly like real winget rejects bad input.

use std::env;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;
use std::thread;
use std::time::Duration;

/// The exact install argv the adapter must use (byte-match contract).
const INSTALL_ARGS: [&str; 8] = [
    "install",
    "--id",
    "OpenJS.NodeJS.LTS",
    "--exact",
    "--silent",
    "--disable-interactivity",
    "--accept-source-agreements",
    "--accept-package-agreements",
];

/// The Node version a successful fake install "provides" (supported LTS).
const UPDATED_NODE_VERSION: &str = "24.15.0";

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.len() == 1 && args[0] == "--version" {
        capture_argv(&args);
        println!("v1.8.2000");
        return ExitCode::SUCCESS;
    }
    if args.first().map(|arg| arg.as_str()) == Some("install") {
        return handle_install(&args);
    }
    capture_argv(&args);
    eprintln!("fake-winget: unsupported command: {}", args.join(" "));
    ExitCode::from(2)
}

fn handle_install(args: &[String]) -> ExitCode {
    capture_argv(args);
    // Argument validation runs FIRST — a concurrent scenario's unrelated
    // `CLAWDESK_FAKE_BEHAVIOR` must never turn a bad invocation into a
    // success (real winget rejects unknown flags before any work).
    if !matches_exact(args, &INSTALL_ARGS) {
        eprintln!(
            "fake-winget: unsupported install arguments: {}",
            args.join(" ")
        );
        return ExitCode::from(2);
    }
    match env::var("CLAWDESK_FAKE_BEHAVIOR").as_deref() {
        Ok("sleep") => {
            thread::sleep(Duration::from_secs(3));
            println!("fake-winget: simulated install finished (after delay)");
            ExitCode::SUCCESS
        }
        Ok("fail") => {
            eprintln!("fake-winget: simulated winget failure referencing token sk-fake123456789");
            ExitCode::from(1)
        }
        Ok("noop") => {
            println!("fake-winget: install requested (no state change)");
            ExitCode::SUCCESS
        }
        _ => match set_node_version(UPDATED_NODE_VERSION) {
            Ok(()) => {
                println!("Successfully installed OpenJS.NodeJS.LTS {UPDATED_NODE_VERSION}");
                ExitCode::SUCCESS
            }
            Err(message) => {
                eprintln!("fake-winget: {message}");
                ExitCode::from(64)
            }
        },
    }
}

/// True when `args` equals `expected` element by element.
fn matches_exact(args: &[String], expected: &[&str]) -> bool {
    if args.len() != expected.len() {
        return false;
    }
    args.iter()
        .zip(expected.iter())
        .all(|(arg, exp)| arg.as_str() == *exp)
}

/// Appends the exact argv as one JSON array line (contract assertion aid).
fn capture_argv(args: &[String]) {
    if let Ok(file) = env::var("CLAWDESK_FAKE_CAPTURE") {
        if file.is_empty() {
            return;
        }
        let line = serde_json::to_string(args).unwrap_or_default();
        let _ = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file)
            .and_then(|mut f| f.write_all(format!("{line}\n").as_bytes()));
    }
}

fn state_path() -> Option<PathBuf> {
    env::var("CLAWDESK_FAKE_STATE")
        .ok()
        .filter(|dir| !dir.is_empty())
        .map(|dir| PathBuf::from(dir).join("node.json"))
}

/// Sets `node.version` in the simulated state (creates the file when
/// absent, like a fresh machine).
fn set_node_version(version: &str) -> Result<(), String> {
    let path = state_path().ok_or("CLAWDESK_FAKE_STATE is not set")?;
    let mut state: serde_json::Value = match fs::read_to_string(&path) {
        Ok(body) => serde_json::from_str(&body).map_err(|err| err.to_string())?,
        Err(_) => serde_json::json!({}),
    };
    state["node"]["version"] = serde_json::Value::String(version.to_string());
    fs::write(&path, state.to_string()).map_err(|err| err.to_string())?;
    Ok(())
}
