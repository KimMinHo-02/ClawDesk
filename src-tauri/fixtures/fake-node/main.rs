//! A deterministic, std+serde_json fake "node" for Phase 8.1 tests.
//!
//! `node --version` prints `v<version>` where `<version>` comes from the
//! state file (`node.json` in `CLAWDESK_FAKE_STATE`) — the same file
//! `fake-winget` updates on a successful install, so an update scenario
//! flips the reported version end to end. Default (no state): `18.19.0`
//! (an unsupported version). Any other argv → exit 2.

use std::env;
use std::fs;
use std::path::Path;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.len() == 1 && args[0] == "--version" {
        let version = read_node_version().unwrap_or_else(|| "18.19.0".to_string());
        println!("v{version}");
        return ExitCode::SUCCESS;
    }
    eprintln!("fake-node: unsupported arguments: {}", args.join(" "));
    ExitCode::from(2)
}

fn read_node_version() -> Option<String> {
    let dir = env::var("CLAWDESK_FAKE_STATE")
        .ok()
        .filter(|dir| !dir.is_empty())?;
    let body = fs::read_to_string(Path::new(&dir).join("node.json")).ok()?;
    let value: serde_json::Value = serde_json::from_str(&body).ok()?;
    value["node"]["version"].as_str().map(str::to_string)
}
