use std::env;
use std::fs;
use std::path::Path;
use std::process::ExitCode;
use std::thread;
use std::time::Duration;

/// A deterministic, std-only fake "Node runtime" for Phase 2 install tests.
///
/// The adapter always launches npm as `node.exe + npm-cli.js` (never a shim),
/// so this binary stands in for `node.exe`. It dispatches on the first
/// argument (the entry script, like real Node does):
///
/// - entry named `npm-cli.js` -> npm behavior (`--version`, `install -g`)
/// - any other entry (e.g. the installed `openclaw.mjs`) -> OpenClaw CLI
///   behavior (`--version`)
///
/// Per-test configuration is read from marker comments inside the entry file
/// itself (`// CLAWDESK_FAKE_<KEY>: <value>`), so tests stay parallel-safe
/// without shared child-process environment variables:
///
/// npm-cli.js markers:
///   NPM_VERSION        npm --version output (default `10.9.0`)
///   NPM_BEHAVIOR       `normal` (default) | `fail` | `sleep`
///   NPM_INSTALL_ROOT   global root where `install -g` creates the package
///   NPM_CAPTURE        file to write the received executable + argv to
///   NPM_TEMPLATES      dir with the fake openclaw package templates
///
/// openclaw.mjs markers:
///   OPENCLAW_VERSION   openclaw --version output (default `2026.7.1-2`)
fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let (entry, rest) = match args.split_first() {
        Some((entry, rest)) => (entry, rest.to_vec()),
        None => {
            eprintln!("fake-npm: missing entry script argument");
            return ExitCode::from(64);
        }
    };
    let entry_path = Path::new(entry);
    let entry_body = fs::read_to_string(entry_path).unwrap_or_default();
    let is_npm = entry_path.file_name().and_then(|name| name.to_str()) == Some("npm-cli.js");
    if is_npm {
        handle_npm(&entry_body, &rest)
    } else {
        handle_openclaw(&entry_body, &rest)
    }
}

/// Reads a `// CLAWDESK_FAKE_<key>: <value>` marker from an entry file.
fn marker(body: &str, key: &str) -> Option<String> {
    body.lines().find_map(|line| {
        let rest = line.trim().strip_prefix("// CLAWDESK_FAKE_")?;
        let (marker_key, value) = rest.split_once(':')?;
        (marker_key.trim() == key).then(|| value.trim().to_string())
    })
}

// --- npm -------------------------------------------------------------------------

fn handle_npm(entry_body: &str, args: &[String]) -> ExitCode {
    if args.len() == 1 && args[0] == "--version" {
        let version = marker(entry_body, "NPM_VERSION").unwrap_or_else(|| "10.9.0".to_string());
        println!("{version}");
        return ExitCode::SUCCESS;
    }
    if matches_install(args) {
        capture(entry_body, args);
        return match marker(entry_body, "NPM_BEHAVIOR").as_deref() {
            Some("fail") => {
                eprintln!("fake-npm: simulated install failure referencing token sk-fake123456789");
                ExitCode::from(3)
            }
            Some("sleep") => {
                thread::sleep(Duration::from_secs(3));
                ExitCode::SUCCESS
            }
            _ => run_install(entry_body),
        };
    }
    eprintln!("fake-npm: unsupported npm command: {}", args.join(" "));
    ExitCode::from(2)
}

/// Exact npm invocation the adapter must use:
/// `install -g openclaw@latest [--allow-scripts=openclaw]`
fn matches_install(args: &[String]) -> bool {
    const PREFIX: [&str; 3] = ["install", "-g", "openclaw@latest"];
    if !matches!(args.len(), 3 | 4) {
        return false;
    }
    for (index, expected) in PREFIX.iter().enumerate() {
        if args.get(index).map(|arg| arg.as_str()) != Some(expected) {
            return false;
        }
    }
    match args.len() {
        4 => args[3] == "--allow-scripts=openclaw",
        _ => true,
    }
}

/// Writes the received executable path and argv (one per line) so contract
/// tests can verify the exact structured spawn.
fn capture(entry_body: &str, args: &[String]) {
    let Some(path) = marker(entry_body, "NPM_CAPTURE") else {
        return;
    };
    let mut lines = vec![env::args().next().unwrap_or_default()];
    lines.push(entry_path_from_args());
    lines.extend_from_slice(args);
    let _ = fs::write(path, lines.join("\n") + "\n");
}

/// argv[0] as seen by the "node" process is the npm-cli.js path.
fn entry_path_from_args() -> String {
    env::args().nth(1).unwrap_or_default()
}

/// Simulates a successful `npm install -g openclaw@latest`: materializes the
/// fake package under the configured global root from template files.
fn run_install(entry_body: &str) -> ExitCode {
    let (Some(root), Some(templates)) = (
        marker(entry_body, "NPM_INSTALL_ROOT"),
        marker(entry_body, "NPM_TEMPLATES"),
    ) else {
        eprintln!("fake-npm: NPM_INSTALL_ROOT / NPM_TEMPLATES markers are required");
        return ExitCode::from(64);
    };
    let package_dir = Path::new(&root).join("node_modules").join("openclaw");
    if let Err(err) = fs::create_dir_all(&package_dir) {
        eprintln!("fake-npm: cannot create package dir: {err}");
        return ExitCode::from(65);
    }
    let templates = Path::new(&templates);
    let files = [
        (
            templates.join("openclaw-package.json"),
            package_dir.join("package.json"),
        ),
        (
            templates.join("openclaw.mjs"),
            package_dir.join("openclaw.mjs"),
        ),
    ];
    for (from, to) in &files {
        if let Err(err) = fs::copy(from, to) {
            eprintln!("fake-npm: cannot copy template {}: {}", from.display(), err);
            return ExitCode::from(65);
        }
    }
    println!("added 1 package in 2s");
    ExitCode::SUCCESS
}

// --- openclaw (package entry) ------------------------------------------------------

fn handle_openclaw(entry_body: &str, args: &[String]) -> ExitCode {
    if args.len() == 1 && args[0] == "--version" {
        let version =
            marker(entry_body, "OPENCLAW_VERSION").unwrap_or_else(|| "2026.7.1-2".to_string());
        println!("OpenClaw {version}");
        return ExitCode::SUCCESS;
    }
    eprintln!("fake-npm: unsupported openclaw command: {}", args.join(" "));
    ExitCode::from(2)
}
