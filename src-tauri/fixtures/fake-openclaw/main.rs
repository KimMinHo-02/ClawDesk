use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;
use std::thread;
use std::time::Duration;

/// A deterministic, std-only stand-in for the real `openclaw` CLI.
///
/// It supports only the read-only commands used by Phase 1 and emits fixture
/// payloads chosen by child-process environment variables, so contract and
/// integration tests can exercise the adapter without a real installation.
fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if matches_command(&args, &["--version"]) {
        return handle_version();
    }
    if matches_command(&args, &["gateway", "status", "--json"]) {
        return handle_gateway();
    }
    if matches_command(&args, &["update", "status", "--json"]) {
        return handle_update();
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

enum Behavior {
    Normal,
    Malformed,
    NotJson,
    CliError,
    Stopped,
    Fail,
    Sleep,
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
        _ => Behavior::Normal,
    }
}

fn update_mode() -> &'static str {
    match env::var("CLAWDESK_FAKE_UPDATE").as_deref() {
        Ok("available") => "available",
        _ => "updated",
    }
}

fn handle_version() -> ExitCode {
    match behavior() {
        Behavior::Sleep => sleep_and_exit(),
        Behavior::Fail => fail_behavior(),
        Behavior::Malformed => print_payload("version-malformed.txt", 0),
        Behavior::NotJson => print_payload("not-json.txt", 0),
        Behavior::CliError => print_payload("gateway-error.json", 1),
        // `stopped` only affects gateway status; other commands keep working.
        Behavior::Stopped | Behavior::Normal => print_payload("version.txt", 0),
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
        Behavior::Normal => print_payload("gateway.json", 0),
    }
}

fn handle_update() -> ExitCode {
    match behavior() {
        Behavior::Sleep => sleep_and_exit(),
        Behavior::Fail => fail_behavior(),
        Behavior::Malformed => print_payload("not-json.txt", 0),
        Behavior::NotJson => print_payload("not-json.txt", 0),
        Behavior::CliError => print_payload("gateway-error.json", 1),
        Behavior::Stopped | Behavior::Normal => {
            if update_mode() == "available" {
                print_payload("update-available.json", 0)
            } else {
                print_payload("update-updated.json", 0)
            }
        }
    }
}

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
