use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::thread;
use std::time::Duration;

use serde_json::{Map, Value};

/// A deterministic, std-only stand-in for the real `openclaw` CLI.
///
/// Phase 1 commands (`--version`, `gateway status`, `update status`) emit
/// fixture payloads chosen by child-process environment variables.
///
/// Phase 3 adds a **config state simulation**: when `CLAWDESK_FAKE_STATE`
/// points at a sandbox directory, the fake maintains a JSON state file
/// (`openclaw.json`) and implements the non-interactive
/// `config`/`models` commands the adapter uses:
///
/// - `config file --json`
/// - `config get <path> --json` (redacted snapshot)
/// - `config set <path> <json> [--strict-json] [--merge|--replace] [--dry-run --json]`
/// - `config unset <path> [--dry-run --json]`
/// - `models list --json`
/// - `models set <provider/model>`
///
/// Phase 4 adds the skills/plugins simulation (state `skills`/`plugins`
/// sections):
///
/// - `skills list --json`, `skills info <name> --json`
/// - `plugins list --json`
/// - `plugins enable <id>`, `plugins disable <id>` (Nix mode rejects)
/// - `plugins inspect <id> [--runtime] --json`
///
/// Phase 5 adds the cold security audit (read-only, no credentials):
///
/// - `security audit --json` (state-derived findings +
///   `securityAudit.findings`/`suppressedFindings` passthrough;
///   `--deep`/`--fix`/`--token`/`--password` are rejected)
///
/// Phase 6 adds the channels/pairing simulation (state `channels` /
/// `channelsStatus` / `pairing` sections; the fake never receives a token
/// value — `channels add --token` is intentionally unsupported):
///
/// - `channels list --all --json` — explicit `channels` array rows are
///   passed through; otherwise rows are derived from the `channels.<id>`
///   config sections (Discord `installed` ⇔ `plugins.catalog` holds
///   `@openclaw/discord`)
/// - `channels status --json` — `channelsStatus` section passthrough
///   (`gatewayReachable` + `channels` rows)
/// - `pairing list <channel> --json` — `pairing.<channel>` rows
/// - `pairing approve <channel> <code>` — removes the matching row
///   (unknown code → failure envelope, exit 1)
/// - `plugins install <npm-id>` — dedup into `plugins.catalog` (the
///   `plugins list` re-check then sees it)
///
/// `CLAWDESK_FAKE_CAPTURE` (file) receives one JSON array line per
/// invocation with the exact argv, so contract tests can assert the
/// structured command line byte-for-byte.
fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    capture_argv(&args);

    if matches_command(&args, &["--version"]) {
        return handle_version();
    }
    if matches_command(&args, &["gateway", "status", "--json"]) {
        return handle_gateway();
    }
    if matches_command(&args, &["update", "status", "--json"]) {
        return handle_update();
    }
    if matches_command(&args, &["agents", "list", "--json"]) {
        return handle_agents_list();
    }
    if args.first().map(|a| a.as_str()) == Some("logs") {
        return handle_logs(&args[1..]);
    }
    if matches_command(&args, &["config", "file", "--json"]) {
        return handle_config_file();
    }
    if args.first().map(|a| a.as_str()) == Some("config")
        && args.get(1).map(|a| a.as_str()) == Some("get")
        && args.last().map(|a| a.as_str()) == Some("--json")
        && args.len() >= 4
    {
        let path = args[2..args.len() - 1].join(".");
        return handle_config_get(&path);
    }
    if args.first().map(|a| a.as_str()) == Some("config")
        && args.get(1).map(|a| a.as_str()) == Some("set")
        && args.len() >= 4
    {
        return handle_config_set(&args[2..]);
    }
    if args.first().map(|a| a.as_str()) == Some("config")
        && args.get(1).map(|a| a.as_str()) == Some("unset")
        && args.len() >= 3
    {
        let rest = &args[2..];
        let dry_run = rest.iter().any(|arg| arg == "--dry-run");
        let path = if rest.last().map(|a| a.as_str()) == Some("--json") {
            rest[..rest.len() - 2].join(".")
        } else {
            rest.join(".")
        };
        return handle_config_unset(&path, dry_run);
    }
    if matches_command(&args, &["models", "list", "--json"]) {
        return handle_models_list();
    }
    if args.len() == 3 && args[0] == "models" && args[1] == "set" {
        return handle_models_set(&args[2]);
    }
    if matches_command(&args, &["skills", "list", "--json"]) {
        return handle_skills_list();
    }
    if args.len() >= 3
        && args[0] == "skills"
        && args[1] == "info"
        && args.last().map(|a| a.as_str()) == Some("--json")
    {
        return handle_skills_info(&args[2]);
    }
    if args.len() >= 3
        && args[0] == "security"
        && args[1] == "audit"
        && args.iter().any(|a| a.as_str() == "--json")
    {
        // The cold audit never takes `--deep`/`--fix` (non-goals) or
        // credentials; reject them like the real CLI would.
        if args
            .iter()
            .any(|arg| matches!(arg.as_str(), "--deep" | "--fix" | "--token" | "--password"))
        {
            eprintln!("fake-openclaw: security audit rejects --deep/--fix/--token/--password");
            return ExitCode::from(2);
        }
        return handle_security_audit();
    }
    if matches_command(&args, &["plugins", "list", "--json"]) {
        return handle_plugins_list();
    }
    if args.len() == 3 && args[0] == "plugins" && matches!(args[1].as_str(), "enable" | "disable") {
        return handle_plugins_toggle(&args[1], &args[2]);
    }
    if args.len() >= 4
        && args[0] == "plugins"
        && args[1] == "inspect"
        && args.last().map(|a| a.as_str()) == Some("--json")
    {
        let runtime = args.iter().any(|arg| arg.as_str() == "--runtime");
        return handle_plugins_inspect(&args[2], runtime);
    }
    if args.len() == 3 && args[0] == "plugins" && args[1] == "install" {
        return handle_plugins_install(&args[2]);
    }
    if matches_command(&args, &["channels", "list", "--all", "--json"]) {
        return handle_channels_list();
    }
    if matches_command(&args, &["channels", "status", "--json"]) {
        return handle_channels_status();
    }
    if args.len() >= 4
        && args[0] == "pairing"
        && args[1] == "list"
        && args.last().map(|a| a.as_str()) == Some("--json")
    {
        return handle_pairing_list(&args[2]);
    }
    if args.len() == 4 && args[0] == "pairing" && args[1] == "approve" {
        return handle_pairing_approve(&args[2], &args[3]);
    }
    if args.first().map(|a| a.as_str()) == Some("automations") {
        let sub = args.get(1).map(|a| a.as_str()).unwrap_or("");
        match sub {
            "list" => return handle_automations_list(),
            "get" => return handle_automations_get(args.get(2).map(|a| a.as_str()).unwrap_or("")),
            "add" => return handle_automations_add(&args[2..]),
            "edit" => return handle_automations_edit(&args[2..]),
            "enable" => {
                return handle_automations_toggle(
                    "enable",
                    args.get(2).map(|a| a.as_str()).unwrap_or(""),
                )
            }
            "disable" => {
                return handle_automations_toggle(
                    "disable",
                    args.get(2).map(|a| a.as_str()).unwrap_or(""),
                )
            }
            "remove" => {
                return handle_automations_remove(args.get(2).map(|a| a.as_str()).unwrap_or(""))
            }
            // `run`/`runs` (and anything else): intentionally unhandled —
            // the non-goal regression gate falls through to the
            // unsupported exit 2 below.
            _ => {}
        }
    }
    eprintln!("fake-openclaw: unsupported command: {}", args.join(" "));
    ExitCode::from(2)
}

fn matches_command(args: &[String], expected: &[&str]) -> bool {
    if args.len() != expected.len() {
        return false;
    }
    for (arg, exp) in args.iter().zip(expected.iter()) {
        if arg.as_str() != *exp {
            return false;
        }
    }
    true
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

enum Behavior {
    Normal,
    Malformed,
    NotJson,
    CliError,
    Stopped,
    Fail,
    Sleep,
    ConfigInvalid,
}

fn behavior() -> Behavior {
    match env::var("CLAWDESK_FAKE_BEHAVIOR").as_deref() {
        Ok("malformed") => Behavior::Malformed,
        Ok("not-json") => Behavior::NotJson,
        Ok("cli-error") => Behavior::CliError,
        // The real CLI exits 1 when no gateway is reachable (valid payload).
        Ok("stopped") => Behavior::Stopped,
        Ok("fail") => Behavior::Fail,
        Ok("sleep") => Behavior::Sleep,
        Ok("config-invalid") => Behavior::ConfigInvalid,
        _ => Behavior::Normal,
    }
}

fn update_mode() -> &'static str {
    match env::var("CLAWDESK_FAKE_UPDATE").as_deref() {
        Ok("available") => "available",
        _ => "updated",
    }
}

// --- Phase 1 handlers (unchanged behavior) -----------------------------------

fn handle_version() -> ExitCode {
    match behavior() {
        Behavior::Sleep => sleep_and_exit(),
        Behavior::Fail => fail_behavior(),
        Behavior::Malformed => print_payload("version-malformed.txt", 0),
        Behavior::NotJson => print_payload("not-json.txt", 0),
        Behavior::CliError => print_payload("gateway-error.json", 1),
        // `stopped` only affects gateway status; other commands keep working.
        Behavior::Stopped | Behavior::Normal | Behavior::ConfigInvalid => {
            print_payload("version.txt", 0)
        }
    }
}

fn handle_gateway() -> ExitCode {
    match behavior() {
        Behavior::Sleep => sleep_and_exit(),
        Behavior::Fail => fail_behavior(),
        Behavior::Malformed => print_payload("not-json.txt", 0),
        Behavior::NotJson => print_payload("not-json.txt", 0),
        Behavior::CliError => print_payload("gateway-error.json", 1),
        Behavior::Stopped => print_payload("gateway-stopped.json", 1),
        Behavior::Normal | Behavior::ConfigInvalid => print_payload("gateway.json", 0),
    }
}

fn handle_update() -> ExitCode {
    match behavior() {
        Behavior::Sleep => sleep_and_exit(),
        Behavior::Fail => fail_behavior(),
        Behavior::Malformed => print_payload("not-json.txt", 0),
        Behavior::NotJson => print_payload("not-json.txt", 0),
        Behavior::CliError => print_payload("gateway-error.json", 1),
        Behavior::Stopped | Behavior::Normal | Behavior::ConfigInvalid => {
            if update_mode() == "available" {
                print_payload("update-available.json", 0)
            } else {
                print_payload("update-updated.json", 0)
            }
        }
    }
}

// --- Phase 3: config state simulation -----------------------------------------

/// The sandbox directory holding the simulated `openclaw.json`.
fn state_dir() -> Option<PathBuf> {
    match env::var("CLAWDESK_FAKE_STATE") {
        Ok(dir) if !dir.is_empty() => Some(PathBuf::from(dir)),
        _ => None,
    }
}

fn state_path() -> Option<PathBuf> {
    state_dir().map(|dir| dir.join("openclaw.json"))
}

/// Loads the state (creating a minimal default when the file is absent).
fn load_state() -> Result<(Value, PathBuf), ExitCode> {
    let path = state_path().ok_or(ExitCode::from(64))?;
    let default = Map::from_iter([
        (
            "models".to_string(),
            Value::Object(Map::from_iter([(
                "providers".to_string(),
                Value::Object(Map::new()),
            )])),
        ),
        (
            "agents".to_string(),
            Value::Object(Map::from_iter([(
                "defaults".to_string(),
                Value::Object(Map::new()),
            )])),
        ),
        (
            "secrets".to_string(),
            Value::Object(Map::from_iter([(
                "providers".to_string(),
                Value::Object(Map::new()),
            )])),
        ),
    ])
    .into();
    let state = match fs::read_to_string(&path) {
        Ok(body) => serde_json::from_str::<Value>(&body).map_err(|err| {
            eprintln!("fake-openclaw: state file invalid: {err}");
            ExitCode::from(65)
        })?,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => default,
        Err(err) => {
            eprintln!("fake-openclaw: cannot read state: {err}");
            return Err(ExitCode::from(64));
        }
    };
    Ok((state, path))
}

fn save_state(state: &Value, path: &Path) -> Result<(), ExitCode> {
    fs::create_dir_all(path.parent().unwrap())
        .and_then(|_| {
            fs::write(
                path,
                serde_json::to_string_pretty(state).unwrap_or_default(),
            )
        })
        .map_err(|err| {
            eprintln!("fake-openclaw: cannot save state: {err}");
            ExitCode::from(64)
        })
}

/// Resolves a dot path immutably.
fn get_value<'v>(mut node: &'v Value, segments: &[&str]) -> Option<&'v Value> {
    for segment in segments {
        node = node.get(*segment)?;
    }
    Some(node)
}

/// Resolves a dot path mutably.
fn get_value_mut<'v>(mut node: &'v mut Value, segments: &[&str]) -> Option<&'v mut Value> {
    for segment in segments {
        node = node.get_mut(*segment)?;
    }
    Some(node)
}

/// Sets a dot path, creating intermediate objects. `root` is the container
/// holding the path; the last segment is inserted as a key in the final
/// container.
fn set_value(root: &mut Value, segments: &[String], new: Value) {
    if segments.len() == 1 {
        if !root.is_object() {
            *root = Value::Object(Map::new());
        }
        root.as_object_mut()
            .expect("object after normalize")
            .insert(segments[0].clone(), new);
        return;
    }
    if !root.is_object() {
        *root = Value::Object(Map::new());
    }
    let obj = root.as_object_mut().expect("object after normalize");
    let next = obj
        .entry(segments[0].clone())
        .or_insert_with(|| Value::Object(Map::new()));
    set_value(next, &segments[1..], new);
}

/// Removes a dot path; true when something was removed.
fn unset_value(root: &mut Value, segments: &[String]) -> bool {
    if segments.len() == 1 {
        return root
            .as_object_mut()
            .and_then(|obj| obj.remove(&segments[0]))
            .is_some();
    }
    root.as_object_mut()
        .and_then(|obj| obj.get_mut(&segments[0]))
        .map(|next| unset_value(next, &segments[1..]))
        .unwrap_or(false)
}

/// Deep-merge `new` into `existing` (objects merge recursively; arrays and
/// scalars are replaced) — the documented `--merge` semantics.
fn merge_values(existing: &mut Value, new: Value) {
    match (existing, new) {
        (Value::Object(obj_a), Value::Object(obj_b)) => {
            for (key, value) in obj_b {
                match obj_a.get_mut(&key) {
                    Some(target) => merge_values(target, value),
                    None => {
                        obj_a.insert(key, value);
                    }
                }
            }
        }
        (target, replacement) => {
            *target = replacement;
        }
    }
}

/// Recursively redacts secret-bearing string fields (simulates the real
/// CLI's redacted snapshot; SecretRef objects are not secrets and pass).
fn redact(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, value)| {
                    let key_norm: String = key
                        .to_ascii_lowercase()
                        .chars()
                        .filter(|c| c.is_ascii_alphanumeric())
                        .collect();
                    let is_secret_name = key_norm == "apikey"
                        || key_norm.ends_with("token")
                        || key_norm.ends_with("password")
                        || key_norm.ends_with("passwd")
                        || key_norm.ends_with("secret")
                        || key_norm.ends_with("credential");
                    let redacted = if is_secret_name && value.is_string() {
                        Value::String("***".to_string())
                    } else {
                        redact(value)
                    };
                    (key.clone(), redacted)
                })
                .collect(),
        ),
        other => other.clone(),
    }
}

fn handle_config_file() -> ExitCode {
    let Some(path) = state_path() else {
        eprintln!("fake-openclaw: CLAWDESK_FAKE_STATE is not set");
        return ExitCode::from(64);
    };
    let _ = fs::create_dir_all(path.parent().unwrap());
    if path.is_file() {
        let body = format!(
            r#"{{"path":"{}"}}"#,
            path.display().to_string().replace('\\', "\\\\")
        );
        print!("{body}");
        return ExitCode::SUCCESS;
    }
    // First run: create the default state.
    let (state, path) = match load_state() {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let _ = save_state(&state, &path);
    let body = format!(
        r#"{{"path":"{}"}}"#,
        path.display().to_string().replace('\\', "\\\\")
    );
    print!("{body}");
    ExitCode::SUCCESS
}

fn handle_config_get(path: &str) -> ExitCode {
    let (state, _path) = match load_state() {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let segments: Vec<&str> = path.split('.').collect();
    let value = get_value(&state, &segments).unwrap_or(&Value::Null);
    print!("{}", redact(value));
    ExitCode::SUCCESS
}

/// Protected paths (contract): replacement that would remove existing
/// entries requires an explicit `--replace`.
fn is_protected_path(segments: &[String]) -> bool {
    if segments.len() < 2 || segments[0] != "models" || segments[1] != "providers" {
        return false;
    }
    matches!(segments.len(), 2 | 3) || (segments.len() == 4 && segments[3] == "models")
}

/// Whether replacing `existing` with `new` would remove existing entries.
fn protected_path_removal(existing: &Value, new: &Value) -> bool {
    match (existing, new) {
        (Value::Object(map), Value::Object(new_map)) => {
            map.keys().any(|key| !new_map.contains_key(key))
        }
        (Value::Array(items), Value::Array(new_items)) => {
            items.iter().any(|item| !new_items.contains(item))
        }
        // Replacing a container with a scalar/null removes all entries.
        (Value::Object(_) | Value::Array(_), _) => true,
        _ => false,
    }
}

fn handle_config_set(args: &[String]) -> ExitCode {
    // Shape: set <path> <json> [--strict-json] [--merge|--replace] [--dry-run --json]
    let mut merge = false;
    let mut replace = false;
    let mut dry_run = false;
    let mut positional: Vec<usize> = Vec::new();
    for (index, arg) in args.iter().enumerate() {
        match arg.as_str() {
            // `--strict-json`, `--json` are accepted (the value is always
            // treated as strict JSON; replace = plain replace).
            "--strict-json" | "--json" => {}
            "--replace" => replace = true,
            "--merge" => merge = true,
            "--dry-run" => dry_run = true,
            _ => positional.push(index),
        }
    }
    if positional.len() < 2 {
        eprintln!("fake-openclaw: config set needs <path> <json>");
        return ExitCode::from(64);
    }
    let json_index = *positional.last().unwrap();
    let path = positional
        .iter()
        .take(positional.len() - 1)
        .map(|index| args[*index].as_str())
        .collect::<Vec<_>>()
        .join(".");

    // Strict JSON: the value must be a parseable JSON document.
    // A failed DRY RUN reports via the `ok:false` envelope with exit 0
    // (the envelope is the result channel); a failed real set exits 2.
    let parsed: Value = match serde_json::from_str(&args[json_index]) {
        Ok(value) => value,
        Err(err) => {
            if dry_run {
                println!(
                    r#"{{"ok":false,"operations":0,"errors":[{{"kind":"parse","message":"invalid JSON: {err}"}}]}}"#
                );
                return ExitCode::SUCCESS;
            }
            eprintln!("fake-openclaw: invalid JSON value: {err}");
            return ExitCode::from(2);
        }
    };

    // Simulated schema rejection (contract failure case).
    if matches!(behavior(), Behavior::ConfigInvalid) {
        if dry_run {
            println!(
                r#"{{"ok":false,"operations":0,"errors":[{{"kind":"schema","message":"simulated schema rejection"}}]}}"#
            );
            return ExitCode::SUCCESS;
        }
        eprintln!("fake-openclaw: simulated schema rejection");
        return ExitCode::from(2);
    }

    let (mut state, state_file) = match load_state() {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let segments: Vec<String> = path.split('.').map(str::to_string).collect();
    // Protected path rule: a plain (no `--merge`/`--replace`) replacement that
    // removes existing entries is rejected.
    if !merge && !replace && is_protected_path(&segments) {
        let segments_refs: Vec<&str> = segments.iter().map(|s| s.as_str()).collect();
        if let Some(existing) = get_value(&state, &segments_refs) {
            if protected_path_removal(existing, &parsed) {
                if dry_run {
                    println!(
                        r#"{{"ok":false,"operations":0,"errors":[{{"kind":"protected-path","message":"protected path: existing entries would be removed; --replace required"}}]}}"#
                    );
                    return ExitCode::SUCCESS;
                }
                eprintln!("fake-openclaw: protected path requires --replace");
                return ExitCode::from(2);
            }
        }
    }
    if dry_run {
        // Dry run: validate, report, do not persist.
        println!(
            r#"{{"ok":true,"operations":1,"configPath":"{}","errors":[]}}"#,
            state_file.display().to_string().replace('\\', "\\\\")
        );
        return ExitCode::SUCCESS;
    }
    let segments_refs: Vec<&str> = segments.iter().map(|s| s.as_str()).collect();
    if merge {
        match get_value_mut(&mut state, &segments_refs) {
            Some(existing) if existing.is_object() && parsed.is_object() => {
                merge_values(existing, parsed);
            }
            _ => set_value(&mut state, &segments, parsed),
        }
    } else {
        set_value(&mut state, &segments, parsed);
    }
    match save_state(&state, &state_file) {
        Ok(()) => ExitCode::SUCCESS,
        Err(code) => code,
    }
}

fn handle_config_unset(path: &str, dry_run: bool) -> ExitCode {
    let (mut state, state_file) = match load_state() {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let segments: Vec<String> = path.split('.').map(str::to_string).collect();
    let exists = get_value(
        &state,
        &segments.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
    )
    .is_some();
    if dry_run {
        // Dry run reports via the envelope (exit 0); it never mutates.
        if exists {
            println!(r#"{{"ok":true,"operations":1,"errors":[]}}"#);
        } else {
            println!(
                r#"{{"ok":false,"operations":0,"errors":[{{"kind":"not-found","message":"path does not exist"}}]}}"#
            );
        }
        return ExitCode::SUCCESS;
    }
    if !exists {
        // Real unset of a missing target: the real CLI exits 1, config intact.
        println!(
            r#"{{"ok":false,"operations":0,"errors":[{{"kind":"not-found","message":"path does not exist"}}]}}"#
        );
        return ExitCode::from(1);
    }
    if !unset_value(&mut state, &segments) {
        println!(
            r#"{{"ok":false,"operations":0,"errors":[{{"kind":"not-found","message":"path does not exist"}}]}}"#
        );
        return ExitCode::from(1);
    }
    if save_state(&state, &state_file).is_err() {
        return ExitCode::from(1);
    }
    println!(r#"{{"ok":true,"operations":1,"errors":[]}}"#);
    ExitCode::SUCCESS
}

fn handle_models_list() -> ExitCode {
    let (state, _path) = match load_state() {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let mut rows: Vec<Value> = Vec::new();
    if let Some(providers) = state
        .get("models")
        .and_then(|m| m.get("providers"))
        .and_then(|p| p.as_object())
    {
        let mut ids: Vec<&String> = providers.keys().collect();
        ids.sort();
        for provider_id in ids {
            if let Some(models) = providers
                .get(provider_id)
                .and_then(|p| p.get("models"))
                .and_then(|m| m.as_array())
            {
                for model in models {
                    if let Some(model_id) = model.get("id").and_then(|v| v.as_str()) {
                        let mut row = Map::new();
                        row.insert("provider".to_string(), Value::String(provider_id.clone()));
                        row.insert("model".to_string(), Value::String(model_id.to_string()));
                        row.insert(
                            "full".to_string(),
                            Value::String(format!("{provider_id}/{model_id}")),
                        );
                        if let Some(name) = model.get("name").and_then(|v| v.as_str()) {
                            row.insert("name".to_string(), Value::String(name.to_string()));
                        }
                        row.insert(
                            "reasoning".to_string(),
                            Value::Bool(
                                model
                                    .get("reasoning")
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or(false),
                            ),
                        );
                        if let Some(context) = model.get("contextWindow").and_then(|v| v.as_u64()) {
                            row.insert("contextTokens".to_string(), Value::from(context));
                        }
                        if let Some(efforts) = model
                            .get("compat")
                            .and_then(|c| c.get("supportedReasoningEfforts"))
                            .and_then(|e| e.as_array())
                            .map(|arr| Value::Array(arr.clone()))
                        {
                            row.insert("supportedReasoningEfforts".to_string(), efforts);
                        }
                        rows.push(Value::Object(row));
                    }
                }
            }
        }
    }
    println!(
        r#"{{"ok":true,"models":{}}}"#,
        serde_json::to_string(&rows).unwrap_or_default()
    );
    ExitCode::SUCCESS
}

fn handle_models_set(model_ref: &str) -> ExitCode {
    let (mut state, state_file) = match load_state() {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let (provider, model) = match model_ref.split_once('/') {
        Some(pair) => pair,
        None => {
            eprintln!("fake-openclaw: unknown model: {model_ref}");
            return ExitCode::from(2);
        }
    };
    let exists = state
        .get("models")
        .and_then(|m| m.get("providers"))
        .and_then(|p| p.get(provider))
        .and_then(|p| p.get("models"))
        .and_then(|m| m.as_array())
        .map(|models| {
            models
                .iter()
                .any(|entry| entry.get("id").and_then(|v| v.as_str()) == Some(model))
        })
        .unwrap_or(false);
    if !exists {
        eprintln!("fake-openclaw: unknown model: {model_ref}");
        return ExitCode::from(2);
    }
    set_value(
        &mut state,
        &[
            "agents".to_string(),
            "defaults".to_string(),
            "model".to_string(),
        ],
        serde_json::json!({ "primary": model_ref }),
    );
    match save_state(&state, &state_file) {
        Ok(()) => ExitCode::SUCCESS,
        Err(code) => code,
    }
}

// --- Phase 4: skills / plugins state simulation ----------------------------------
//
// State sections (sandbox `openclaw.json`):
//
// - `skills.catalog.<name>` — base skill definition (optional `description`,
//   `source`, `eligible` (default true))
// - `skills.entries.<name>.enabled` — the config override (default true)
// - `plugins.catalog.<id>` — base plugin definition (optional `name`,
//   `format`, `origin`/`source`, `version`, `dependencyStatus`, `runtime`,
//   `diagnostics`)
// - `plugins.entries.<id>.enabled` — the config state (default true)
//
// `OPENCLAW_NIX_MODE=1` makes `plugins enable/disable` reject with a
// non-zero exit (state unchanged), mirroring the real CLI.

/// Behavior overrides shared by the Phase 4 read handlers. Returns `Some`
/// when the handler must not proceed to the normal state-based output.
fn behavior_override() -> Option<ExitCode> {
    match behavior() {
        Behavior::Sleep => Some(sleep_and_exit()),
        Behavior::Fail => Some(fail_behavior()),
        Behavior::Malformed => {
            // Truncated JSON with exit 0: the parse failure must surface.
            print!(r#"{{"skills":[{{"name":"weather","enabled":tru"#);
            Some(ExitCode::SUCCESS)
        }
        Behavior::NotJson => {
            print!("skills and plugins are fine");
            Some(ExitCode::SUCCESS)
        }
        Behavior::CliError => {
            println!(r#"{{"ok":false,"error":"simulated cli error"}}"#);
            Some(ExitCode::from(1))
        }
        _ => None,
    }
}

fn handle_skills_list() -> ExitCode {
    if let Some(code) = behavior_override() {
        return code;
    }
    let (state, _path) = match load_state() {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let mut rows: Vec<Value> = Vec::new();
    if let Some(catalog) = state
        .get("skills")
        .and_then(|s| s.get("catalog"))
        .and_then(|c| c.as_object())
    {
        let mut names: Vec<&String> = catalog.keys().collect();
        names.sort();
        for name in names {
            let entry = catalog.get(name).expect("key present");
            rows.push(skill_row(&state, name, entry));
        }
    }
    println!(
        r#"{{"ok":true,"skills":{}}}"#,
        serde_json::to_string(&rows).unwrap_or_default()
    );
    ExitCode::SUCCESS
}

/// One skill row: the `skills.entries.<name>.enabled` override wins over
/// the default (true); `eligible` comes from the catalog (default true).
fn skill_row(state: &Value, name: &str, catalog_entry: &Value) -> Value {
    let mut row = Map::new();
    row.insert("name".to_string(), Value::String(name.to_string()));
    let enabled = state
        .get("skills")
        .and_then(|s| s.get("entries"))
        .and_then(|e| e.get(name))
        .and_then(|e| e.get("enabled"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    row.insert("enabled".to_string(), Value::Bool(enabled));
    let eligible = catalog_entry
        .get("eligible")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    row.insert("eligible".to_string(), Value::Bool(eligible));
    if let Some(description) = catalog_entry.get("description").and_then(|v| v.as_str()) {
        row.insert(
            "description".to_string(),
            Value::String(description.to_string()),
        );
    }
    if let Some(source) = catalog_entry
        .get("source")
        .or_else(|| catalog_entry.get("origin"))
        .and_then(|v| v.as_str())
    {
        row.insert("source".to_string(), Value::String(source.to_string()));
    }
    Value::Object(row)
}

fn handle_skills_info(name: &str) -> ExitCode {
    if let Some(code) = behavior_override() {
        return code;
    }
    let (state, _path) = match load_state() {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let catalog = state
        .get("skills")
        .and_then(|s| s.get("catalog"))
        .and_then(|c| c.as_object());
    match catalog.and_then(|c| c.get(name)) {
        Some(entry) => {
            let row = skill_row(&state, name, entry);
            println!(
                r#"{{"ok":true,"skill":{}}}"#,
                serde_json::to_string(&row).unwrap_or_default()
            );
            ExitCode::SUCCESS
        }
        None => {
            eprintln!("fake-openclaw: unknown skill: {name}");
            ExitCode::from(2)
        }
    }
}

fn handle_plugins_list() -> ExitCode {
    if let Some(code) = behavior_override() {
        return code;
    }
    let (state, _path) = match load_state() {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let mut rows: Vec<Value> = Vec::new();
    if let Some(catalog) = state
        .get("plugins")
        .and_then(|p| p.get("catalog"))
        .and_then(|c| c.as_object())
    {
        let mut ids: Vec<&String> = catalog.keys().collect();
        ids.sort();
        for id in ids {
            let entry = catalog.get(id).expect("key present");
            let mut row = Map::new();
            row.insert("id".to_string(), Value::String(id.clone()));
            let enabled = state
                .get("plugins")
                .and_then(|p| p.get("entries"))
                .and_then(|e| e.get(id))
                .and_then(|e| e.get("enabled"))
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            row.insert("enabled".to_string(), Value::Bool(enabled));
            for (key, wire) in [
                ("name", "name"),
                ("format", "format"),
                ("origin", "origin"),
                ("source", "origin"),
                ("version", "version"),
                ("dependencyStatus", "dependencyStatus"),
            ] {
                if let Some(value) = entry.get(key).and_then(|v| v.as_str()) {
                    row.insert(wire.to_string(), Value::String(value.to_string()));
                }
            }
            rows.push(Value::Object(row));
        }
    }
    println!(
        r#"{{"ok":true,"plugins":{}}}"#,
        serde_json::to_string(&rows).unwrap_or_default()
    );
    ExitCode::SUCCESS
}

fn handle_plugins_toggle(action: &str, id: &str) -> ExitCode {
    if env::var("OPENCLAW_NIX_MODE").is_ok_and(|value| value == "1") {
        eprintln!("fake-openclaw: nix mode: plugins {action} rejected");
        return ExitCode::from(2);
    }
    let (mut state, state_file) = match load_state() {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let known = state
        .get("plugins")
        .and_then(|p| p.get("catalog"))
        .and_then(|c| c.get(id))
        .is_some();
    if !known {
        eprintln!("fake-openclaw: unknown plugin: {id}");
        return ExitCode::from(2);
    }
    set_value(
        &mut state,
        &["plugins".to_string(), "entries".to_string(), id.to_string()],
        Value::Object(Map::from_iter([(
            "enabled".to_string(),
            Value::Bool(action == "enable"),
        )])),
    );
    if save_state(&state, &state_file).is_err() {
        return ExitCode::from(1);
    }
    println!(
        r#"{{"ok":true,"id":{},"enabled":{}}}"#,
        serde_json::to_string(id).unwrap_or_default(),
        action == "enable"
    );
    ExitCode::SUCCESS
}

fn handle_plugins_inspect(id: &str, runtime: bool) -> ExitCode {
    if let Some(code) = behavior_override() {
        return code;
    }
    let (state, _path) = match load_state() {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let catalog = state
        .get("plugins")
        .and_then(|p| p.get("catalog"))
        .and_then(|c| c.as_object());
    let entry = match catalog.and_then(|c| c.get(id)) {
        Some(entry) => entry,
        None => {
            eprintln!("fake-openclaw: unknown plugin: {id}");
            return ExitCode::from(2);
        }
    };
    let enabled = state
        .get("plugins")
        .and_then(|p| p.get("entries"))
        .and_then(|e| e.get(id))
        .and_then(|e| e.get("enabled"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let mut out = Map::new();
    out.insert("ok".to_string(), Value::Bool(true));
    out.insert("id".to_string(), Value::String(id.to_string()));
    if let Some(name) = entry.get("name").and_then(|v| v.as_str()) {
        out.insert("name".to_string(), Value::String(name.to_string()));
    }
    out.insert("enabled".to_string(), Value::Bool(enabled));
    if runtime {
        let mut surfaces = Map::new();
        for key in [
            "tools",
            "hooks",
            "services",
            "cliCommands",
            "gatewayMethods",
            "routes",
        ] {
            surfaces.insert(
                key.to_string(),
                entry
                    .get("runtime")
                    .and_then(|r| r.get(key))
                    .cloned()
                    .unwrap_or_else(|| Value::Array(Vec::new())),
            );
        }
        out.insert("runtime".to_string(), Value::Object(surfaces));
        if let Some(diagnostics) = entry.get("diagnostics") {
            out.insert("diagnostics".to_string(), diagnostics.clone());
        }
    }
    println!(
        "{}",
        serde_json::to_string(&Value::Object(out)).unwrap_or_default()
    );
    ExitCode::SUCCESS
}

// --- Phase 5: security audit state simulation ----------------------------------
//
// State sections (sandbox `openclaw.json`):
//
// - `tools.exec.mode` — drives the derived finding: `"full"` or unset →
//   `tools.exec.security_full_configured` (warn)
// - `securityAudit.findings` — passthrough finding rows (`checkId` required,
//   the rest optional)
// - `securityAudit.suppressedFindings` — passthrough (display count only)

fn handle_security_audit() -> ExitCode {
    if let Some(code) = behavior_override() {
        return code;
    }
    let (state, _path) = match load_state() {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let mut findings: Vec<Value> = Vec::new();
    let exec_mode = state
        .get("tools")
        .and_then(|t| t.get("exec"))
        .and_then(|e| e.get("mode"))
        .and_then(|m| m.as_str());
    if !matches!(exec_mode, Some("deny" | "allowlist" | "ask" | "auto")) {
        // `"full"` or unset: the no-approval gate warning (derived finding).
        let mut row = Map::new();
        row.insert(
            "checkId".to_string(),
            Value::String("tools.exec.security_full_configured".to_string()),
        );
        row.insert("severity".to_string(), Value::String("warn".to_string()));
        row.insert(
            "title".to_string(),
            Value::String("Exec can run without approval".to_string()),
        );
        row.insert(
            "detail".to_string(),
            Value::String("tools.exec.mode is \"full\" or unset".to_string()),
        );
        findings.push(Value::Object(row));
    }
    if let Some(pass_through) = state
        .get("securityAudit")
        .and_then(|s| s.get("findings"))
        .and_then(|f| f.as_array())
    {
        findings.extend(pass_through.iter().cloned());
    }
    let suppressed = state
        .get("securityAudit")
        .and_then(|s| s.get("suppressedFindings"))
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()));
    let summary = Map::from_iter([
        ("total".to_string(), Value::from(findings.len())),
        (
            "critical".to_string(),
            Value::from(count_severity(&findings, "critical")),
        ),
        (
            "warn".to_string(),
            Value::from(count_severity(&findings, "warn")),
        ),
        (
            "info".to_string(),
            Value::from(count_severity(&findings, "info")),
        ),
    ]);
    let mut out = Map::new();
    out.insert("ok".to_string(), Value::Bool(true));
    out.insert("findings".to_string(), Value::Array(findings));
    out.insert("summary".to_string(), Value::Object(summary));
    out.insert("suppressedFindings".to_string(), suppressed);
    println!(
        "{}",
        serde_json::to_string(&Value::Object(out)).unwrap_or_default()
    );
    ExitCode::SUCCESS
}

fn count_severity(findings: &[Value], severity: &str) -> u64 {
    findings
        .iter()
        .filter(|row| row.get("severity").and_then(|s| s.as_str()) == Some(severity))
        .count() as u64
}

// --- Phase 6: channels / pairing / plugin install state simulation -------------
//
// State sections (sandbox `openclaw.json`):
//
// - `channels` — either an explicit array of `{id,installed,configured,enabled}`
//   rows (contract fixtures) or an object of `channels.<id>` config sections
//   (created by `config set channels.<id>.<field>` writes)
// - `channelsStatus` — `{gatewayReachable, channels:[{id,state}]}` passthrough
// - `pairing.<channel>` — array of `{code,sender?}` pending requests
//
// The token value is never a fake-CLI argument: `set-channel-token` writes
// the ClawDesk exec SecretRef through `config set` (the value itself lives
// only in the ClawDesk secret store).

/// The npm id of the Discord channel plugin (the connect flow installs it).
const DISCORD_PLUGIN_ID: &str = "@openclaw/discord";

fn handle_plugins_install(npm_id: &str) -> ExitCode {
    if let Some(code) = behavior_override() {
        return code;
    }
    let (mut state, state_file) = match load_state() {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    // Dedup into the catalog so the `plugins list` post-check sees it.
    let plugins = state
        .as_object_mut()
        .expect("state is an object")
        .entry("plugins")
        .or_insert_with(|| Value::Object(Map::new()));
    if !plugins.is_object() {
        *plugins = Value::Object(Map::new());
    }
    let catalog = plugins
        .as_object_mut()
        .expect("plugins normalized")
        .entry("catalog")
        .or_insert_with(|| Value::Object(Map::new()));
    if !catalog.is_object() {
        *catalog = Value::Object(Map::new());
    }
    let catalog = catalog.as_object_mut().expect("catalog normalized");
    if !catalog.contains_key(npm_id) {
        catalog.insert(
            npm_id.to_string(),
            Value::Object(Map::from_iter([
                ("name".to_string(), Value::String(npm_id.to_string())),
                ("origin".to_string(), Value::String("npm".to_string())),
            ])),
        );
    }
    if save_state(&state, &state_file).is_err() {
        return ExitCode::from(1);
    }
    println!(
        r#"{{"ok":true,"id":{}}}"#,
        serde_json::to_string(npm_id).unwrap_or_default()
    );
    ExitCode::SUCCESS
}

fn handle_channels_list() -> ExitCode {
    if let Some(code) = behavior_override() {
        return code;
    }
    let (state, _path) = match load_state() {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let mut rows: Vec<Value> = Vec::new();
    match state.get("channels") {
        // Explicit scripted rows (contract fixtures) pass through unchanged.
        Some(Value::Array(items)) => rows = items.to_vec(),
        _ => {
            let section_map = state.get("channels").and_then(|c| c.as_object());
            for id in ["discord", "telegram"] {
                let installed = if id == "discord" {
                    state
                        .get("plugins")
                        .and_then(|p| p.get("catalog"))
                        .and_then(|c| c.get(DISCORD_PLUGIN_ID))
                        .is_some()
                } else {
                    true
                };
                let section = section_map.and_then(|m| m.get(id));
                let enabled = section
                    .and_then(|s| s.get("enabled"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let mut row = Map::new();
                row.insert("id".to_string(), Value::String(id.to_string()));
                row.insert("installed".to_string(), Value::Bool(installed));
                row.insert("configured".to_string(), Value::Bool(section.is_some()));
                row.insert("enabled".to_string(), Value::Bool(enabled));
                rows.push(Value::Object(row));
            }
        }
    }
    println!(
        "{}",
        serde_json::to_string(&Value::Object(Map::from_iter([
            ("ok".to_string(), Value::Bool(true)),
            ("channels".to_string(), Value::Array(rows)),
        ])))
        .unwrap_or_default()
    );
    ExitCode::SUCCESS
}

fn handle_channels_status() -> ExitCode {
    if let Some(code) = behavior_override() {
        return code;
    }
    let (state, _path) = match load_state() {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    // Absent section → `null`: the parser fails soft to config-only state.
    let section = state.get("channelsStatus").cloned().unwrap_or(Value::Null);
    println!("{}", serde_json::to_string(&section).unwrap_or_default());
    ExitCode::SUCCESS
}

fn handle_pairing_list(channel: &str) -> ExitCode {
    if let Some(code) = behavior_override() {
        return code;
    }
    let (state, _path) = match load_state() {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let requests = state
        .get("pairing")
        .and_then(|p| p.get(channel))
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default();
    println!(
        "{}",
        serde_json::to_string(&Value::Object(Map::from_iter([
            ("ok".to_string(), Value::Bool(true)),
            ("requests".to_string(), Value::Array(requests)),
        ])))
        .unwrap_or_default()
    );
    ExitCode::SUCCESS
}

fn handle_pairing_approve(channel: &str, code: &str) -> ExitCode {
    if let Some(code_) = behavior_override() {
        return code_;
    }
    let (mut state, state_file) = match load_state() {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let Some(array) = state
        .get_mut("pairing")
        .and_then(|p| p.get_mut(channel))
        .and_then(|r| r.as_array_mut())
    else {
        println!(r#"{{"ok":false,"error":"no pending pairing requests for {channel}"}}"#);
        return ExitCode::from(1);
    };
    let before = array.len();
    array.retain(|row| row.get("code").and_then(|v| v.as_str()) != Some(code));
    if array.len() == before {
        println!(r#"{{"ok":false,"error":"pairing code not found"}}"#);
        return ExitCode::from(1);
    }
    if save_state(&state, &state_file).is_err() {
        return ExitCode::from(1);
    }
    println!(
        r#"{{"ok":true,"code":{}}}"#,
        serde_json::to_string(code).unwrap_or_default()
    );
    ExitCode::SUCCESS
}

// --- Phase 7: automations state simulation ---------------------------------
//
// State sections (sandbox `openclaw.json`):
//
// - `automations.jobs` — array of job objects
//   `{id,name,enabled,status,nextRunAtMs,schedule:{kind,at|every|cron,tz?},
//   payload:{kind,text,wake?}}`
//
// `automations run`/`runs` have NO handler (non-goal — the unsupported
// exit 2 is the regression gate). Non-goal flags (`--command`, `--script`,
// `--webhook`, `--model`, `--channel`, `--to`, ...) are rejected with
// exit 2 exactly like an unsupported flag, so a regression that starts
// emitting them is caught at the contract layer.

/// Non-goal flags that must never be accepted by `automations add/edit`.
const AUTOMATION_NON_GOAL_FLAGS: &[&str] = &[
    "--command",
    "--command-argv",
    "--script",
    "--trigger-script",
    "--webhook",
    "--model",
    "--fallbacks",
    "--thinking",
    "--channel",
    "--to",
    "--thread-id",
    "--account",
    "--agent",
];

fn reject_non_goal_flags(args: &[String]) -> Option<&'static str> {
    args.iter().find_map(|arg| {
        AUTOMATION_NON_GOAL_FLAGS
            .iter()
            .find(|flag| **flag == arg.as_str())
            .copied()
    })
}

/// The `automations.jobs` array (read view).
fn automation_jobs(state: &Value) -> Option<&Vec<Value>> {
    state
        .get("automations")
        .and_then(|a| a.get("jobs"))
        .and_then(|j| j.as_array())
}

/// The `automations.jobs` array (write view; created when absent).
fn automation_jobs_mut(state: &mut Value) -> &mut Vec<Value> {
    let automations = state
        .as_object_mut()
        .expect("state is an object")
        .entry("automations")
        .or_insert_with(|| Value::Object(Map::new()));
    if !automations.is_object() {
        *automations = Value::Object(Map::new());
    }
    let jobs = automations
        .as_object_mut()
        .expect("automations normalized")
        .entry("jobs")
        .or_insert_with(|| Value::Array(Vec::new()));
    if !jobs.is_array() {
        *jobs = Value::Array(Vec::new());
    }
    jobs.as_array_mut().expect("jobs normalized")
}

fn find_automation_id(state: &Value, id: &str) -> Option<usize> {
    automation_jobs(state).and_then(|jobs| {
        jobs.iter()
            .position(|job| job.get("id").and_then(|v| v.as_str()) == Some(id))
    })
}

fn handle_automations_list() -> ExitCode {
    if let Some(code) = behavior_override() {
        return code;
    }
    let (state, _path) = match load_state() {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let jobs = automation_jobs(&state).cloned().unwrap_or_default();
    println!(
        "{}",
        serde_json::to_string(&Value::Object(Map::from_iter([
            ("ok".to_string(), Value::Bool(true)),
            ("jobs".to_string(), Value::Array(jobs)),
        ])))
        .unwrap_or_default()
    );
    ExitCode::SUCCESS
}

fn handle_automations_get(id: &str) -> ExitCode {
    if let Some(code) = behavior_override() {
        return code;
    }
    let (state, _path) = match load_state() {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let Some(job) = automation_jobs(&state).and_then(|jobs| {
        jobs.iter()
            .find(|job| job.get("id").and_then(|v| v.as_str()) == Some(id))
    }) else {
        println!(r#"{{"ok":false,"error":{{"type":"cli_error","message":"unknown job: {id}"}}}}"#);
        return ExitCode::from(1);
    };
    println!(
        "{}",
        serde_json::to_string(&Value::Object(Map::from_iter([
            ("ok".to_string(), Value::Bool(true)),
            ("job".to_string(), job.clone()),
        ])))
        .unwrap_or_default()
    );
    ExitCode::SUCCESS
}

/// The flag set shared by `automations add/edit`.
#[derive(Default)]
struct AutomationFlags {
    name: Option<String>,
    at: Option<String>,
    every: Option<String>,
    cron: Option<String>,
    tz: Option<String>,
    session: Option<String>,
    system_event: Option<String>,
    message: Option<String>,
    wake: Option<String>,
}

/// Parses `--flag value` pairs; unknown arguments are an error.
fn parse_automation_flags(args: &[String]) -> Result<AutomationFlags, String> {
    let mut flags = AutomationFlags::default();
    let mut index = 0usize;
    while index < args.len() {
        let key = args[index].as_str();
        let mut next = |key: &str| -> Result<String, String> {
            index += 1;
            if index >= args.len() {
                return Err(format!("missing value for {key}"));
            }
            Ok(args[index].clone())
        };
        match key {
            "--name" => flags.name = Some(next(key)?),
            "--at" => flags.at = Some(next(key)?),
            "--every" => flags.every = Some(next(key)?),
            "--cron" => flags.cron = Some(next(key)?),
            "--tz" => flags.tz = Some(next(key)?),
            "--session" => flags.session = Some(next(key)?),
            "--system-event" => flags.system_event = Some(next(key)?),
            "--message" => flags.message = Some(next(key)?),
            "--wake" => flags.wake = Some(next(key)?),
            "--json" => {}
            other => {
                if AUTOMATION_NON_GOAL_FLAGS.contains(&other) {
                    return Err(format!("non-goal flag rejected: {other}"));
                }
                return Err(format!("unknown argument: {other}"));
            }
        }
        index += 1;
    }
    Ok(flags)
}

/// Builds the `schedule` object from the flags (exactly one of at/every/cron;
/// tz is cron-only — the fixed pairing).
fn build_schedule(flags: &AutomationFlags) -> Result<Value, String> {
    let (kind, value_key, value) = match (
        flags.at.as_deref(),
        flags.every.as_deref(),
        flags.cron.as_deref(),
    ) {
        (Some(value), None, None) => ("at", "at", value),
        (None, Some(value), None) => ("every", "every", value),
        (None, None, Some(value)) => ("cron", "cron", value),
        _ => {
            return Err("exactly one of --at/--every/--cron is required".to_string());
        }
    };
    let tz_allowed = kind == "cron";
    if flags.tz.is_some() && !tz_allowed {
        return Err("--tz is cron-only".to_string());
    }
    let mut map = Map::new();
    map.insert("kind".to_string(), Value::String(kind.to_string()));
    map.insert(value_key.to_string(), Value::String(value.to_string()));
    if let Some(tz) = &flags.tz {
        map.insert("tz".to_string(), Value::String(tz.clone()));
    }
    Ok(Value::Object(map))
}

/// Builds the `payload` object from the flags (exactly one of
/// system-event/message; wake is reminder-only; the session pairing is
/// fixed: reminder → `main`, task → `isolated`).
fn build_payload(flags: &AutomationFlags) -> Result<Value, String> {
    let (kind, text) = match (flags.system_event.as_deref(), flags.message.as_deref()) {
        (Some(text), None) => ("reminder", text),
        (None, Some(text)) => ("task", text),
        _ => {
            return Err("exactly one of --system-event/--message is required".to_string());
        }
    };
    let expected_session = if kind == "reminder" {
        "main"
    } else {
        "isolated"
    };
    if let Some(session) = &flags.session {
        if session != expected_session {
            return Err(format!("session {session} is not allowed for a {kind} job"));
        }
    }
    if kind == "task" && flags.wake.is_some() {
        return Err("--wake is reminder-only".to_string());
    }
    if let Some(wake) = &flags.wake {
        if !matches!(wake.as_str(), "now" | "next-heartbeat") {
            return Err(format!("invalid wake: {wake}"));
        }
    }
    let mut payload = Map::new();
    payload.insert("kind".to_string(), Value::String(kind.to_string()));
    payload.insert("text".to_string(), Value::String(text.to_string()));
    if kind == "reminder" {
        payload.insert(
            "wake".to_string(),
            Value::String(flags.wake.clone().unwrap_or_else(|| "now".to_string())),
        );
    }
    Ok(Value::Object(payload))
}

fn handle_automations_add(args: &[String]) -> ExitCode {
    if let Some(code) = behavior_override() {
        return code;
    }
    if let Some(flag) = reject_non_goal_flags(args) {
        eprintln!("fake-openclaw: automations add rejects non-goal flag {flag}");
        return ExitCode::from(2);
    }
    let flags = match parse_automation_flags(args) {
        Ok(flags) => flags,
        Err(message) => {
            eprintln!("fake-openclaw: automations add: {message}");
            return ExitCode::from(2);
        }
    };
    let (mut state, state_file) = match load_state() {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let jobs = automation_jobs_mut(&mut state);
    let next = jobs
        .iter()
        .filter_map(|job| {
            job.get("id")
                .and_then(|v| v.as_str())
                .and_then(|id| id.strip_prefix("job-"))
                .and_then(|n| n.parse::<u64>().ok())
        })
        .max()
        .unwrap_or(0)
        + 1;
    let id = format!("job-{next}");
    let job = match (|| -> Result<Value, String> {
        let name = flags.name.as_deref().ok_or("missing --name")?;
        let schedule = build_schedule(&flags)?;
        let payload = build_payload(&flags)?;
        let mut map = Map::new();
        map.insert("id".to_string(), Value::String(id.clone()));
        map.insert("name".to_string(), Value::String(name.to_string()));
        map.insert("enabled".to_string(), Value::Bool(true));
        map.insert("status".to_string(), Value::String("ok".to_string()));
        map.insert("nextRunAtMs".to_string(), Value::Null);
        map.insert("schedule".to_string(), schedule);
        map.insert("payload".to_string(), payload);
        Ok(Value::Object(map))
    })() {
        Ok(job) => job,
        Err(message) => {
            println!(r#"{{"ok":false,"error":{{"type":"cli_error","message":"{message}"}}}}"#);
            return ExitCode::from(1);
        }
    };
    jobs.push(job);
    if save_state(&state, &state_file).is_err() {
        return ExitCode::from(1);
    }
    println!(
        r#"{{"ok":true,"id":{}}}"#,
        serde_json::to_string(&id).unwrap_or_default()
    );
    ExitCode::SUCCESS
}

fn handle_automations_edit(args: &[String]) -> ExitCode {
    if let Some(code) = behavior_override() {
        return code;
    }
    if args.is_empty() {
        eprintln!("fake-openclaw: automations edit needs <job-id>");
        return ExitCode::from(2);
    }
    let (id, flag_args) = (args[0].as_str(), &args[1..]);
    if let Some(flag) = reject_non_goal_flags(flag_args) {
        eprintln!("fake-openclaw: automations edit rejects non-goal flag {flag}");
        return ExitCode::from(2);
    }
    let flags = match parse_automation_flags(flag_args) {
        Ok(flags) => flags,
        Err(message) => {
            eprintln!("fake-openclaw: automations edit: {message}");
            return ExitCode::from(2);
        }
    };
    let (mut state, state_file) = match load_state() {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let Some(index) = find_automation_id(&state, id) else {
        println!(r#"{{"ok":false,"error":{{"type":"cli_error","message":"unknown job: {id}"}}}}"#);
        return ExitCode::from(1);
    };
    let result = (|| -> Result<(), String> {
        // The edit carries the full definition (name + schedule + payload).
        let name = flags.name.as_deref().ok_or("missing --name")?;
        let schedule = build_schedule(&flags)?;
        let payload = build_payload(&flags)?;
        let job = state
            .get_mut("automations")
            .and_then(|a| a.get_mut("jobs"))
            .and_then(|j| j.as_array_mut())
            .and_then(|jobs| jobs.get_mut(index))
            .and_then(|j| j.as_object_mut())
            .ok_or("job object missing")?;
        job.insert("name".to_string(), Value::String(name.to_string()));
        job.insert("schedule".to_string(), schedule);
        job.insert("payload".to_string(), payload);
        Ok(())
    })();
    if let Err(message) = result {
        println!(r#"{{"ok":false,"error":{{"type":"cli_error","message":"{message}"}}}}"#);
        return ExitCode::from(1);
    }
    if save_state(&state, &state_file).is_err() {
        return ExitCode::from(1);
    }
    println!(
        r#"{{"ok":true,"id":{}}}"#,
        serde_json::to_string(id).unwrap_or_default()
    );
    ExitCode::SUCCESS
}

fn handle_automations_toggle(action: &str, id: &str) -> ExitCode {
    if let Some(code) = behavior_override() {
        return code;
    }
    if id.is_empty() {
        eprintln!("fake-openclaw: automations {action} needs <job-id>");
        return ExitCode::from(2);
    }
    let (mut state, state_file) = match load_state() {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let Some(index) = find_automation_id(&state, id) else {
        println!(r#"{{"ok":false,"error":{{"type":"cli_error","message":"unknown job: {id}"}}}}"#);
        return ExitCode::from(1);
    };
    let enabled = action == "enable";
    let jobs = automation_jobs_mut(&mut state);
    if let Some(job) = jobs.get_mut(index) {
        job.as_object_mut()
            .expect("job is an object")
            .insert("enabled".to_string(), Value::Bool(enabled));
    }
    if save_state(&state, &state_file).is_err() {
        return ExitCode::from(1);
    }
    println!(
        r#"{{"ok":true,"id":{},"enabled":{}}}"#,
        serde_json::to_string(id).unwrap_or_default(),
        enabled
    );
    ExitCode::SUCCESS
}

fn handle_automations_remove(id: &str) -> ExitCode {
    if let Some(code) = behavior_override() {
        return code;
    }
    if id.is_empty() {
        eprintln!("fake-openclaw: automations remove needs <job-id>");
        return ExitCode::from(2);
    }
    let (mut state, state_file) = match load_state() {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let jobs = automation_jobs_mut(&mut state);
    let before = jobs.len();
    jobs.retain(|job| job.get("id").and_then(|v| v.as_str()) != Some(id));
    if jobs.len() == before {
        println!(r#"{{"ok":false,"error":{{"type":"cli_error","message":"unknown job: {id}"}}}}"#);
        return ExitCode::from(1);
    }
    if save_state(&state, &state_file).is_err() {
        return ExitCode::from(1);
    }
    println!(
        r#"{{"ok":true,"id":{}}}"#,
        serde_json::to_string(id).unwrap_or_default()
    );
    ExitCode::SUCCESS
}

// --- Phase 8: profile / update / diagnostics simulation ----------------------
//
// Read-only surface (PRODUCT_CONTRACT §4.7):
//
// - `agents list --json` — explicit `agents.list` rows pass through
//   (contract fixtures); otherwise the built-in fixture (the `main` default
//   agent + one other identity)
// - `logs --limit <n> --json` — one-shot tail: type-tagged line events
//   (`meta` + `log`/`raw` lines capped at `<n>` + `notice` truncation hint).
//   `--follow` (streaming) is a non-goal and rejected with exit 2, exactly
//   like the real CLI rejects unsupported flags. `logs.empty = true`
//   yields a zero-line tail (empty stdout, exit 0).

/// The synthetic log line pool (deterministic; newest last). One line
/// carries a fake `sk-` token to verify the S8 masking pipeline end to end
/// (fake token only — never a real secret).
const LOG_POOL: &[&str] = &[
    r#"{"type":"log","time":"2026-08-26T10:00:00.000Z","level":"info","subsystem":"gateway","message":"gateway started on 127.0.0.1:18789","hostname":"clawdesk-host"}"#,
    r#"{"type":"log","time":"2026-08-26T10:00:01.000Z","level":"info","subsystem":"agent","message":"agent session bootstrapped","agentId":"main","sessionId":"s-0001"}"#,
    r#"{"type":"log","time":"2026-08-26T10:00:02.000Z","level":"debug","subsystem":"config","message":"config loaded from user profile"}"#,
    r#"{"type":"raw","line":"unparsed legacy line 123"}"#,
    r#"{"type":"log","time":"2026-08-26T10:00:03.000Z","level":"info","subsystem":"channel","message":"discord channel connected"}"#,
    r#"{"type":"log","time":"2026-08-26T10:00:04.000Z","level":"warn","subsystem":"model","message":"model response slow, retrying"}"#,
    r#"{"type":"log","time":"2026-08-26T10:00:05.000Z","level":"info","subsystem":"skill","message":"skill weather loaded"}"#,
    r#"{"type":"log","time":"2026-08-26T10:00:06.000Z","level":"info","subsystem":"automation","message":"automation job-1 scheduled"}"#,
    r#"{"type":"log","time":"2026-08-26T10:00:07.000Z","level":"error","subsystem":"gateway","message":"upstream reset by peer"}"#,
    r#"{"type":"log","time":"2026-08-26T10:00:08.000Z","level":"info","subsystem":"auth","message":"provider token verified"}"#,
    r#"{"type":"log","time":"2026-08-26T10:00:09.000Z","level":"info","subsystem":"session","message":"session s-0001 completed"}"#,
    r#"{"type":"log","time":"2026-08-26T10:00:10.000Z","level":"info","subsystem":"auth","message":"configured provider token sk-fake123456789 loaded"}"#,
];

/// The built-in agent fixture rows (used when the state has no explicit
/// `agents.list`).
fn builtin_agent_rows() -> Vec<Value> {
    vec![
        Value::Object(Map::from_iter([
            ("id".to_string(), Value::String("main".to_string())),
            ("default".to_string(), Value::Bool(true)),
            ("name".to_string(), Value::String("Main Agent".to_string())),
            ("emoji".to_string(), Value::String("🦞".to_string())),
            (
                "workspace".to_string(),
                Value::String("~/openclaw-main".to_string()),
            ),
            ("bindings".to_string(), Value::from(2u64)),
        ])),
        Value::Object(Map::from_iter([
            ("id".to_string(), Value::String("ops".to_string())),
            ("default".to_string(), Value::Bool(false)),
            ("name".to_string(), Value::String("Ops Agent".to_string())),
            (
                "workspace".to_string(),
                Value::String("~/openclaw-ops".to_string()),
            ),
        ])),
    ]
}

fn handle_agents_list() -> ExitCode {
    if let Some(code) = behavior_override() {
        return code;
    }
    let (state, _path) = match load_state() {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    let rows = match state
        .get("agents")
        .and_then(|a| a.get("list"))
        .and_then(|l| l.as_array())
    {
        Some(rows) => rows.clone(),
        None => builtin_agent_rows(),
    };
    println!(
        "{}",
        serde_json::to_string(&Value::Object(Map::from_iter([
            ("ok".to_string(), Value::Bool(true)),
            ("agents".to_string(), Value::Array(rows)),
        ])))
        .unwrap_or_default()
    );
    ExitCode::SUCCESS
}

fn handle_logs(args: &[String]) -> ExitCode {
    // `--follow` is a non-goal (streaming): reject exactly like the real
    // CLI rejects unsupported flags.
    if args.iter().any(|arg| arg == "--follow") {
        eprintln!("fake-openclaw: logs rejects --follow (streaming is not supported)");
        return ExitCode::from(2);
    }
    // Shape: `logs --limit <n> --json` (one-shot tail only). Argument
    // validation runs first — the real CLI rejects unknown flags before
    // any behavior simulation (parallel tests may inherit an unrelated
    // `CLAWDESK_FAKE_BEHAVIOR` through the process environment).
    let mut limit: Option<u64> = None;
    let mut json = false;
    let mut index = 0usize;
    while index < args.len() {
        match args[index].as_str() {
            "--limit" => {
                index += 1;
                let raw = args.get(index).map(|a| a.as_str()).unwrap_or("");
                match raw.parse::<u64>() {
                    Ok(value) => limit = Some(value),
                    Err(_) => {
                        eprintln!("fake-openclaw: invalid --limit value: {raw}");
                        return ExitCode::from(2);
                    }
                }
            }
            "--json" => json = true,
            other => {
                eprintln!("fake-openclaw: logs: unknown argument: {other}");
                return ExitCode::from(2);
            }
        }
        index += 1;
    }
    if !json {
        eprintln!("fake-openclaw: logs: --json is required");
        return ExitCode::from(2);
    }
    if let Some(code) = behavior_override() {
        return code;
    }
    let (state, _path) = match load_state() {
        Ok(pair) => pair,
        Err(code) => return code,
    };
    // `logs.empty = true` → a zero-line tail (empty stdout, exit 0).
    if state
        .get("logs")
        .and_then(|l| l.get("empty"))
        .and_then(|v| v.as_bool())
        == Some(true)
    {
        return ExitCode::SUCCESS;
    }
    let count = limit.unwrap_or(200).min(LOG_POOL.len() as u64) as usize;
    let mut out = String::new();
    out.push_str(&format!(
        "{{\"type\":\"meta\",\"file\":\"openclaw-2026-08-26.log\",\"source\":\"file log\",\"sourceKind\":\"file\",\"service\":\"gateway\",\"cursor\":\"{}\",\"size\":{}}}\n",
        LOG_POOL.len() - count,
        4096
    ));
    for line in LOG_POOL.iter().rev().take(count) {
        out.push_str(line);
        out.push('\n');
    }
    out.push_str(
        "{\"type\":\"notice\",\"message\":\"showing most recent lines\",\"truncated\":true}\n",
    );
    print!("{out}");
    ExitCode::SUCCESS
}

// --- shared helpers ------------------------------------------------------------

fn print_payload(name: &str, exit: u8) -> ExitCode {
    match read_payload(name) {
        Ok(body) => {
            print!("{}", body);
            ExitCode::from(exit)
        }
        Err(code) => code,
    }
}

fn read_payload(name: &str) -> Result<String, ExitCode> {
    let dir = payloads_dir()?;
    let path = dir.join(name);
    match fs::read_to_string(&path) {
        Ok(body) => Ok(body),
        Err(e) => {
            eprintln!(
                "fake-openclaw: cannot read payload {} ({})",
                path.display(),
                e
            );
            Err(ExitCode::from(64))
        }
    }
}

fn payloads_dir() -> Result<PathBuf, ExitCode> {
    match env::var("CLAWDESK_FAKE_PAYLOADS") {
        Ok(dir) if !dir.is_empty() => Ok(PathBuf::from(dir)),
        _ => {
            eprintln!("fake-openclaw: CLAWDESK_FAKE_PAYLOADS is not set");
            Err(ExitCode::from(64))
        }
    }
}

fn fail_behavior() -> ExitCode {
    eprintln!("fake-openclaw: simulated failure referencing token sk-fake123456789");
    ExitCode::from(3)
}

fn sleep_and_exit() -> ExitCode {
    thread::sleep(Duration::from_secs(3));
    ExitCode::SUCCESS
}
