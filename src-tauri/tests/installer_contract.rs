//! Phase 2 contract tests: real `ProcessRunner` + fake `node`/`npm` runtime.
//!
//! Per S5/S6, only the fake fixture is used — no real npm, no system
//! mutation. All per-test configuration travels through the fake's entry
//! file markers, so tests are parallel-safe.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use clawdesk_lib::application::{InstallResult, InstallService};
use clawdesk_lib::domain::models::openclaw::{
    ExecutableDetection, GatewayStatus, OpenClawVersion, UpdateState,
};
use clawdesk_lib::domain::models::windows::{Architecture, NodeDetection, WindowsVersion};
use clawdesk_lib::domain::ports::process::{ProcessError, ProcessPort, ProcessRequest};
use clawdesk_lib::domain::ports::{OpenClawInstallerPort, OpenClawPort, WindowsSystemPort};
use clawdesk_lib::error::AppError;
use clawdesk_lib::infrastructure::openclaw::{OpenClawAdapter, OpenClawInstaller};
use clawdesk_lib::infrastructure::process::ProcessRunner;

const FAKE_NPM: &str = env!("CARGO_BIN_EXE_clawdesk-fake-npm");

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("tests")
        .join("fixtures")
        .join("npm")
}

fn target_dir() -> PathBuf {
    std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("target")
                .to_path_buf()
        })
}

/// Per-test scratch runtime: the fake "node.exe" plus a global install root.
fn runtime(name: &str) -> (PathBuf, PathBuf, PathBuf) {
    let dir = target_dir().join("clawdesk-fake-npm").join(name);
    fs::create_dir_all(&dir).expect("create scratch dir");
    let node_exe = dir.join("node.exe");
    // Other tests run the fake binary concurrently; Windows can briefly lock
    // an image file, so retry the copy.
    let mut attempts = 0u32;
    loop {
        match fs::copy(FAKE_NPM, &node_exe) {
            Ok(_) => break,
            Err(err) => {
                attempts += 1;
                if attempts >= 100 {
                    panic!("copy fake npm: {err}");
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
    // Per-execution stamped root: repeated `cargo test` runs must never
    // observe a previous run's fake install (idempotent without cleanup).
    let install_root = target_dir().join("clawdesk-fake-npm-install").join(format!(
        "{name}-{}-{}",
        run_stamp(),
        std::process::id()
    ));
    (dir, node_exe, install_root)
}

/// High-resolution timestamp marking the current test execution.
fn run_stamp() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0)
}

/// Writes the fake `npm-cli.js` entry with the given marker lines.
fn write_npm_cli(dir: &Path, markers: &[&str]) -> PathBuf {
    let path = dir.join("npm-cli.js");
    let mut body = String::from("// fake npm-cli.js\n");
    for marker in markers {
        body.push_str(marker);
        body.push('\n');
    }
    fs::write(&path, body).expect("write npm-cli.js");
    path
}

fn installer_for(install_root: &Path) -> OpenClawInstaller {
    OpenClawInstaller::new(Arc::new(ProcessRunner), install_root.to_path_buf())
}

// --- npm --version ------------------------------------------------------------------

#[test]
fn fake_npm_version_reports_configured_version() {
    let (dir, node_exe, _) = runtime("version");
    write_npm_cli(&dir, &["// CLAWDESK_FAKE_NPM_VERSION: 11.16.0"]);
    let installer = installer_for(Path::new("/unused"));
    let entry = installer
        .resolve_npm_entry(&node_exe)
        .expect("npm entry should resolve");
    assert_eq!(entry.node, node_exe);
    let version = installer.npm_version(&entry).expect("version should parse");
    assert_eq!(version, "11.16.0");
}

#[test]
fn fake_npm_version_unparseable_is_process_failed() {
    let (dir, node_exe, _) = runtime("version-garbage");
    write_npm_cli(&dir, &["// CLAWDESK_FAKE_NPM_VERSION: not-a-version"]);
    let installer = installer_for(Path::new("/unused"));
    let entry = installer
        .resolve_npm_entry(&node_exe)
        .expect("entry should resolve");
    let err = installer.npm_version(&entry).expect_err("must fail");
    assert_eq!(err.code, "process-failed");
}

// --- npm install -g ------------------------------------------------------------------

#[test]
fn fake_npm_install_success_creates_package_and_captures_argv() {
    let (dir, node_exe, install_root) = runtime("install-plain");
    let capture = dir.join("capture.txt");
    write_npm_cli(
        &dir,
        &[
            "// CLAWDESK_FAKE_NPM_VERSION: 11.12.0",
            &format!(
                "// CLAWDESK_FAKE_NPM_INSTALL_ROOT: {}",
                install_root.display()
            ),
            &format!(
                "// CLAWDESK_FAKE_NPM_TEMPLATES: {}",
                fixtures_dir().display()
            ),
            &format!("// CLAWDESK_FAKE_NPM_CAPTURE: {}", capture.display()),
        ],
    );
    let installer = installer_for(&install_root);
    let entry = installer
        .resolve_npm_entry(&node_exe)
        .expect("npm entry should resolve");
    installer
        .install_openclaw_latest(&entry, false)
        .expect("fake install should succeed");

    // The fake package materialized under the global root.
    let package = install_root.join("node_modules").join("openclaw");
    assert!(package.join("package.json").is_file());
    assert!(package.join("openclaw.mjs").is_file());

    // Exact structured spawn: node.exe + npm-cli.js install -g openclaw@latest
    // (npm <= 11.12 → no --allow-scripts).
    let capture_body = fs::read_to_string(&capture).expect("capture should exist");
    let lines: Vec<&str> = capture_body.lines().collect();
    assert_eq!(lines[0], node_exe.to_string_lossy());
    assert_eq!(lines[1], dir.join("npm-cli.js").to_string_lossy());
    assert_eq!(&lines[2..], &["install", "-g", "openclaw@latest"]);
}

#[test]
fn fake_npm_install_failure_exit_code_and_masked_stderr() {
    let (dir, node_exe, install_root) = runtime("install-fail");
    write_npm_cli(&dir, &["// CLAWDESK_FAKE_NPM_BEHAVIOR: fail"]);
    let installer = installer_for(&install_root);
    let entry = installer
        .resolve_npm_entry(&node_exe)
        .expect("npm entry should resolve");
    let err = installer
        .install_openclaw_latest(&entry, false)
        .expect_err("failing install must error");
    assert_eq!(err.code, "openclaw-install-failed");
    assert!(
        !err.message.contains("sk-fake123456789"),
        "secret must not appear in the error"
    );
    assert!(
        err.message.contains("sk-****"),
        "stderr should be masked, got: {}",
        err.message
    );
}

#[test]
fn fake_npm_install_sleep_times_out_with_short_timeout() {
    let (dir, node_exe, _) = runtime("install-sleep");
    write_npm_cli(&dir, &["// CLAWDESK_FAKE_NPM_BEHAVIOR: sleep"]);
    let request = ProcessRequest::new(
        node_exe,
        vec![
            dir.join("npm-cli.js").to_string_lossy().into_owned(),
            "install".to_string(),
            "-g".to_string(),
            "openclaw@latest".to_string(),
        ],
        Duration::from_millis(400),
    );
    let err = ProcessRunner
        .run(&request)
        .expect_err("sleeping fake npm must time out");
    match err {
        ProcessError::Timeout { executable } => {
            assert!(executable.contains("node.exe"));
        }
        other => panic!("expected Timeout, got {other:?}"),
    }
}

// --- post-install package entry ---------------------------------------------------------

fn fake_install(dir: &Path, node_exe: &Path, install_root: &Path, npm_version: &str) {
    write_npm_cli(
        dir,
        &[
            &format!("// CLAWDESK_FAKE_NPM_VERSION: {npm_version}"),
            &format!(
                "// CLAWDESK_FAKE_NPM_INSTALL_ROOT: {}",
                install_root.display()
            ),
            &format!(
                "// CLAWDESK_FAKE_NPM_TEMPLATES: {}",
                fixtures_dir().display()
            ),
        ],
    );
    let installer = installer_for(install_root);
    let entry = installer
        .resolve_npm_entry(node_exe)
        .expect("npm entry should resolve");
    installer
        .install_openclaw_latest(
            &entry,
            npm_version.starts_with("11.16") || npm_version.starts_with("12"),
        )
        .expect("fake install should succeed");
}

#[test]
fn installed_package_entry_resolves_inside_package_root() {
    let (dir, node_exe, install_root) = runtime("entry-resolve");
    fake_install(&dir, &node_exe, &install_root, "11.16.0");
    let installer = installer_for(&install_root);
    let resolved = installer
        .resolve_openclaw_entry()
        .expect("entry should resolve");
    assert!(resolved.entry.starts_with(&resolved.package_root));
    assert_eq!(
        resolved.entry.file_name().and_then(|name| name.to_str()),
        Some("openclaw.mjs")
    );
}

#[test]
fn installed_entry_version_via_package_entry() {
    let (dir, node_exe, install_root) = runtime("entry-version");
    fake_install(&dir, &node_exe, &install_root, "11.16.0");
    let installer = installer_for(&install_root);
    let resolved = installer
        .resolve_openclaw_entry()
        .expect("entry should resolve");
    let adapter = OpenClawAdapter::new(Arc::new(ProcessRunner), Vec::new());
    let version = adapter
        .version_from_entry(&node_exe, &resolved.entry)
        .expect("version via entry should parse");
    assert_eq!(version.raw, "2026.7.1-2");
}

// --- full InstallService flow over the fake npm ------------------------------------------

/// OpenClaw port whose detect reflects the fake install's filesystem state
/// and whose version_from_entry delegates to the real adapter.
struct DynamicOpenClaw {
    adapter: OpenClawAdapter,
    package_marker: PathBuf,
}

impl OpenClawPort for DynamicOpenClaw {
    fn detect_executable(&self) -> ExecutableDetection {
        if self.package_marker.is_file() {
            ExecutableDetection::Found {
                path: self.package_marker.clone(),
            }
        } else {
            ExecutableDetection::NotFound
        }
    }
    fn version(&self, exe: &Path) -> Result<OpenClawVersion, AppError> {
        self.adapter.version(exe)
    }
    fn version_from_entry(&self, node: &Path, entry: &Path) -> Result<OpenClawVersion, AppError> {
        self.adapter.version_from_entry(node, entry)
    }
    fn gateway_status(&self, exe: &Path) -> Result<GatewayStatus, AppError> {
        self.adapter.gateway_status(exe)
    }
    fn update_state(&self, exe: &Path) -> Result<UpdateState, AppError> {
        self.adapter.update_state(exe)
    }
}

/// Fixed supported Node ("22.22.3") with an explicit executable path.
struct FixedWindows {
    node_exe: PathBuf,
}

impl WindowsSystemPort for FixedWindows {
    fn os_version(&self) -> Result<WindowsVersion, AppError> {
        unimplemented!("not used by InstallService")
    }
    fn architecture(&self) -> Result<Architecture, AppError> {
        unimplemented!("not used by InstallService")
    }
    fn detect_node(&self) -> Result<NodeDetection, AppError> {
        Ok(NodeDetection::Found {
            version: "22.22.3".to_string(),
        })
    }
    fn node_executable(&self) -> Result<PathBuf, AppError> {
        Ok(self.node_exe.clone())
    }
}

fn service_for(node_exe: &Path, install_root: &Path) -> (InstallService, PathBuf, PathBuf) {
    let process: Arc<dyn ProcessPort> = Arc::new(ProcessRunner);
    let adapter = OpenClawAdapter::new(Arc::clone(&process), Vec::new());
    let package_marker = install_root
        .join("node_modules")
        .join("openclaw")
        .join("package.json");
    let openclaw = DynamicOpenClaw {
        adapter,
        package_marker: package_marker.clone(),
    };
    let installer = OpenClawInstaller::new(Arc::clone(&process), install_root.to_path_buf());
    let service = InstallService::new(
        Arc::new(FixedWindows {
            node_exe: node_exe.to_path_buf(),
        }),
        Arc::new(openclaw),
        Arc::new(installer),
    );
    (service, node_exe.to_path_buf(), package_marker)
}

#[test]
fn install_service_fresh_install_with_allow_scripts() {
    let (dir, node_exe, install_root) = runtime("service-new-npm");
    let capture = dir.join("capture.txt");
    write_npm_cli(
        &dir,
        &[
            "// CLAWDESK_FAKE_NPM_VERSION: 11.16.0",
            &format!(
                "// CLAWDESK_FAKE_NPM_INSTALL_ROOT: {}",
                install_root.display()
            ),
            &format!(
                "// CLAWDESK_FAKE_NPM_TEMPLATES: {}",
                fixtures_dir().display()
            ),
            &format!("// CLAWDESK_FAKE_NPM_CAPTURE: {}", capture.display()),
        ],
    );
    let (service, node_exe, package_marker) = service_for(&node_exe, &install_root);
    assert!(!package_marker.is_file(), "precondition: not installed");

    let result = service
        .install_openclaw()
        .expect("fresh install should succeed");
    assert_eq!(
        result,
        InstallResult::Installed {
            version: "2026.7.1-2".to_string()
        }
    );

    // Exact structured spawn with --allow-scripts=openclaw (npm >= 11.16).
    let capture_body = fs::read_to_string(&capture).expect("capture should exist");
    let lines: Vec<&str> = capture_body.lines().collect();
    assert_eq!(lines[0], node_exe.to_string_lossy());
    assert_eq!(lines[1], dir.join("npm-cli.js").to_string_lossy());
    assert_eq!(
        &lines[2..],
        &[
            "install",
            "-g",
            "openclaw@latest",
            "--allow-scripts=openclaw"
        ]
    );
}

#[test]
fn install_service_fresh_install_without_allow_scripts() {
    let (dir, node_exe, install_root) = runtime("service-old-npm");
    let capture = dir.join("capture.txt");
    write_npm_cli(
        &dir,
        &[
            "// CLAWDESK_FAKE_NPM_VERSION: 11.12.0",
            &format!(
                "// CLAWDESK_FAKE_NPM_INSTALL_ROOT: {}",
                install_root.display()
            ),
            &format!(
                "// CLAWDESK_FAKE_NPM_TEMPLATES: {}",
                fixtures_dir().display()
            ),
            &format!("// CLAWDESK_FAKE_NPM_CAPTURE: {}", capture.display()),
        ],
    );
    let (service, _, _) = service_for(&node_exe, &install_root);
    let result = service
        .install_openclaw()
        .expect("fresh install should succeed");
    assert_eq!(
        result,
        InstallResult::Installed {
            version: "2026.7.1-2".to_string()
        }
    );
    // npm <= 11.12 → no --allow-scripts flag.
    let capture_body = fs::read_to_string(&capture).expect("capture should exist");
    let lines: Vec<&str> = capture_body.lines().collect();
    assert_eq!(&lines[2..], &["install", "-g", "openclaw@latest"]);
}

#[test]
fn install_service_is_idempotent_after_install() {
    let (dir, node_exe, install_root) = runtime("service-idempotent");
    let capture = dir.join("capture.txt");
    write_npm_cli(
        &dir,
        &[
            "// CLAWDESK_FAKE_NPM_VERSION: 11.16.0",
            &format!(
                "// CLAWDESK_FAKE_NPM_INSTALL_ROOT: {}",
                install_root.display()
            ),
            &format!(
                "// CLAWDESK_FAKE_NPM_TEMPLATES: {}",
                fixtures_dir().display()
            ),
            &format!("// CLAWDESK_FAKE_NPM_CAPTURE: {}", capture.display()),
        ],
    );
    let (service, _, _) = service_for(&node_exe, &install_root);

    let first = service
        .install_openclaw()
        .expect("fresh install should succeed");
    assert_eq!(
        first,
        InstallResult::Installed {
            version: "2026.7.1-2".to_string()
        }
    );
    let after_first = fs::read_to_string(&capture).expect("capture should exist");

    // Second call: existing install → same version, no npm install spawn.
    let second = service
        .install_openclaw()
        .expect("idempotent call should succeed");
    assert_eq!(
        second,
        InstallResult::AlreadyInstalled {
            version: "2026.7.1-2".to_string()
        }
    );
    let after_second = fs::read_to_string(&capture).expect("capture should exist");
    assert_eq!(
        after_first, after_second,
        "second call must not spawn npm install"
    );
}
