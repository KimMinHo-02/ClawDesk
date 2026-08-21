//! `OpenClawInstaller` — npm-based OpenClaw installation (Phase 2).
//!
//! All process execution goes through the `ProcessPort` (S1/S2/S10). npm is
//! launched exclusively as structured `node.exe + npm-cli.js` argv — the
//! official PowerShell installer and npm shims (`npm.cmd` / `npm.ps1`) are
//! never used. The install target is always `openclaw@latest`; there is no
//! user-controlled version or channel input.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::domain::models::install::{NpmEntry, ResolvedOpenClawEntry};
use crate::domain::ports::installer::OpenClawInstallerPort;
use crate::domain::ports::process::{ProcessError, ProcessOutput, ProcessPort, ProcessRequest};
use crate::error::AppError;
use crate::infrastructure::masking::mask_secrets;

/// Timeout for the `npm --version` probe.
const NPM_VERSION_TIMEOUT: Duration = Duration::from_secs(15);
/// Timeout for `npm install -g openclaw@latest` (Phase 2 contract: 15 minutes).
const INSTALL_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// npm-based OpenClaw installer adapter.
pub struct OpenClawInstaller {
    process: Arc<dyn ProcessPort>,
    /// npm global prefix (e.g. `%APPDATA%\npm` on Windows).
    global_root: PathBuf,
}

impl OpenClawInstaller {
    pub fn new(process: Arc<dyn ProcessPort>, global_root: PathBuf) -> Self {
        Self {
            process,
            global_root,
        }
    }

    /// Production wiring: npm global prefix from `%APPDATA%\npm` (the default
    /// per-user npm prefix on Windows).
    pub fn production(process: Arc<dyn ProcessPort>) -> Self {
        let global_root = std::env::var_os("APPDATA")
            .map(|appdata| PathBuf::from(appdata).join("npm"))
            .unwrap_or_default();
        Self::new(process, global_root)
    }

    /// Runs an npm command through the process port as `node npm-cli.js ...`.
    fn run_npm(
        &self,
        entry: &NpmEntry,
        args: &[&str],
        timeout: Duration,
        label: &str,
    ) -> Result<ProcessOutput, AppError> {
        let mut argv: Vec<String> = vec![entry.npm_cli.to_string_lossy().into_owned()];
        argv.extend(args.iter().map(|arg| arg.to_string()));
        let request = ProcessRequest::new(entry.node.clone(), argv, timeout);
        match self.process.run(&request) {
            Ok(output) => Ok(output),
            Err(ProcessError::NotFound { .. }) => Err(AppError::npm_not_found()),
            Err(ProcessError::Timeout { .. }) => Err(AppError::process_timeout(label)),
            Err(ProcessError::SpawnFailed { message }) => {
                Err(AppError::process_failed(label, message))
            }
        }
    }
}

impl OpenClawInstallerPort for OpenClawInstaller {
    fn resolve_npm_entry(&self, node_executable: &Path) -> Result<NpmEntry, AppError> {
        let node_dir = node_executable
            .parent()
            .filter(|dir| !dir.as_os_str().is_empty())
            .ok_or_else(AppError::npm_not_found)?;
        let candidates = [
            node_dir.join("npm-cli.js"),
            node_dir
                .join("node_modules")
                .join("npm")
                .join("bin")
                .join("npm-cli.js"),
        ];
        for candidate in &candidates {
            if candidate.is_file() {
                return Ok(NpmEntry {
                    node: node_executable.to_path_buf(),
                    npm_cli: candidate.clone(),
                });
            }
        }
        Err(AppError::npm_not_found())
    }

    fn npm_version(&self, entry: &NpmEntry) -> Result<String, AppError> {
        const LABEL: &str = "npm --version";
        let output = self.run_npm(entry, &["--version"], NPM_VERSION_TIMEOUT, LABEL)?;
        if output.exit_code != 0 {
            return Err(AppError::process_failed(
                LABEL,
                format!("exit code {}: {}", output.exit_code, output.stderr.trim()),
            ));
        }
        parse_npm_version(&output.stdout).ok_or_else(|| {
            AppError::process_failed(
                LABEL,
                format!(
                    "cannot parse npm version from output: {}",
                    mask_secrets(&output.stdout)
                ),
            )
        })
    }

    fn install_openclaw_latest(
        &self,
        entry: &NpmEntry,
        allow_scripts: bool,
    ) -> Result<(), AppError> {
        const LABEL: &str = "npm install -g openclaw@latest";
        let mut args: Vec<&str> = vec!["install", "-g", "openclaw@latest"];
        if allow_scripts {
            args.push("--allow-scripts=openclaw");
        }
        let output = self.run_npm(entry, &args, INSTALL_TIMEOUT, LABEL)?;
        if output.exit_code == 0 {
            return Ok(());
        }
        Err(AppError::openclaw_install_failed(format!(
            "exit code {}: {}",
            output.exit_code,
            output.stderr.trim()
        )))
    }

    fn resolve_openclaw_entry(&self) -> Result<ResolvedOpenClawEntry, AppError> {
        if self.global_root.as_os_str().is_empty() {
            return Err(AppError::openclaw_install_verify_failed(
                "npm global prefix is unavailable",
            ));
        }
        let package_root = self.global_root.join("node_modules").join("openclaw");
        if !package_root.is_dir() {
            return Err(AppError::openclaw_install_verify_failed(
                "OpenClaw package root was not found in the npm global prefix",
            ));
        }
        let manifest_path = package_root.join("package.json");
        let manifest_raw = std::fs::read_to_string(&manifest_path).map_err(|err| {
            AppError::openclaw_install_verify_failed(format!(
                "cannot read OpenClaw package.json: {}",
                mask_secrets(&err.to_string())
            ))
        })?;
        let manifest: serde_json::Value = serde_json::from_str(&manifest_raw).map_err(|err| {
            AppError::openclaw_install_verify_failed(format!(
                "OpenClaw package.json is not valid JSON: {}",
                mask_secrets(&err.to_string())
            ))
        })?;
        let bin = bin_openclaw_entry(&manifest).ok_or_else(|| {
            AppError::openclaw_install_verify_failed(
                "package.json has no usable bin.openclaw entry",
            )
        })?;
        // Absolute/canonical path based package boundary validation.
        let canonical_root = package_root.canonicalize().map_err(|err| {
            AppError::openclaw_install_verify_failed(format!(
                "cannot resolve OpenClaw package root: {}",
                mask_secrets(&err.to_string())
            ))
        })?;
        let canonical_entry = package_root.join(bin).canonicalize().map_err(|err| {
            AppError::openclaw_install_verify_failed(format!(
                "cannot resolve OpenClaw JS entry: {}",
                mask_secrets(&err.to_string())
            ))
        })?;
        if !canonical_entry.starts_with(&canonical_root) {
            return Err(AppError::openclaw_install_verify_failed(
                "OpenClaw JS entry escapes the package root",
            ));
        }
        if !canonical_entry.is_file() {
            return Err(AppError::openclaw_install_verify_failed(
                "OpenClaw JS entry is not a file",
            ));
        }
        Ok(ResolvedOpenClawEntry {
            package_root: canonical_root,
            entry: canonical_entry,
        })
    }
}

/// Extracts `bin.openclaw` from a package manifest: the shorthand string form
/// (single-bin package) or the object form with an `openclaw` key.
fn bin_openclaw_entry(manifest: &serde_json::Value) -> Option<String> {
    let bin = manifest.get("bin")?;
    if let Some(path) = bin.as_str() {
        return Some(path.to_string());
    }
    bin.get("openclaw")?.as_str().map(str::to_string)
}

/// Parses the `X.Y.Z` core of a version string; the pre-release/build tail
/// is ignored.
fn parse_semver_core(version: &str) -> Option<(u64, u64, u64)> {
    let core = version.trim().split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next()?.parse::<u64>().ok()?;
    let patch = parts.next().unwrap_or("0").parse::<u64>().ok()?;
    Some((major, minor, patch))
}

/// Whether `version` satisfies the OpenClaw install Node support range.
///
/// Supported (2026-08-20 baseline): 22.22.3+, 24.15+, 25.9+, 26+.
/// Node 23 is officially unsupported; every other major is rejected.
pub fn node_version_supported(version: &str) -> bool {
    let Some((major, minor, patch)) = parse_semver_core(version) else {
        return false;
    };
    match major {
        22 => (minor, patch) >= (22, 3),
        24 => minor >= 15,
        25 => minor >= 9,
        26.. => true,
        _ => false,
    }
}

/// Parses `npm --version` stdout into the printed version string.
pub fn parse_npm_version(stdout: &str) -> Option<String> {
    let line = stdout
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())?;
    let core = line.split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let major = parts.next()?;
    let minor = parts.next()?;
    let digits = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    if digits(major) && digits(minor) {
        Some(line.to_string())
    } else {
        None
    }
}

/// npm version policy (Phase 2 contract).
///
/// `Ok(true)`: `--allow-scripts=openclaw` must be included (npm >= 11.16,
/// npm 12+). `Ok(false)`: install without the flag (npm <= 11.12).
/// `Err(unsupported-npm-version)`: npm 11.13-11.15 blocks the install.
pub fn npm_install_policy(npm_version: &str) -> Result<bool, AppError> {
    let Some((major, minor, _)) = parse_semver_core(npm_version) else {
        return Err(AppError::process_failed(
            "npm --version",
            "cannot parse npm version",
        ));
    };
    match major {
        11 => match minor {
            0..=12 => Ok(false),
            13..=15 => Err(AppError::unsupported_npm_version(npm_version)),
            _ => Ok(true),
        },
        12.. => Ok(true),
        _ => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(exit_code: i32, stdout: &str, stderr: &str) -> ProcessOutput {
        ProcessOutput {
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
            exit_code,
        }
    }

    /// Deterministic in-memory ProcessPort capturing the last request.
    struct Scripted {
        response: Result<ProcessOutput, ProcessError>,
        last_request: std::sync::Arc<std::sync::Mutex<Option<ProcessRequest>>>,
    }

    impl Scripted {
        fn new(response: Result<ProcessOutput, ProcessError>) -> Self {
            Self {
                response,
                last_request: Arc::new(std::sync::Mutex::new(None)),
            }
        }
    }

    impl ProcessPort for Scripted {
        fn run(&self, request: &ProcessRequest) -> Result<ProcessOutput, ProcessError> {
            *self.last_request.lock().unwrap() = Some(request.clone());
            self.response.clone()
        }
    }

    fn scripted(
        response: Result<ProcessOutput, ProcessError>,
    ) -> (Arc<Scripted>, Arc<std::sync::Mutex<Option<ProcessRequest>>>) {
        let last: Arc<std::sync::Mutex<Option<ProcessRequest>>> =
            Arc::new(std::sync::Mutex::new(None));
        let fake = Arc::new(Scripted {
            response,
            last_request: Arc::clone(&last),
        });
        (fake, last)
    }

    fn scratch(name: &str) -> PathBuf {
        let target = std::env::var("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target"));
        let dir = target.join("clawdesk-installer-test").join(name);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    // --- node/npm entry resolution ------------------------------------------

    #[test]
    fn npm_entry_found_next_to_node() {
        let dir = scratch("npm-entry-side");
        let node = dir.join("node.exe");
        std::fs::write(&node, b"").expect("write fake node");
        let npm_cli = dir.join("npm-cli.js");
        std::fs::write(&npm_cli, b"").expect("write npm-cli.js");
        let installer =
            OpenClawInstaller::new(Arc::new(Scripted::new(Ok(output(0, "", "")))), dir.clone());
        let entry = installer
            .resolve_npm_entry(&node)
            .expect("entry should resolve");
        assert_eq!(entry.node, node);
        assert_eq!(entry.npm_cli, npm_cli);
    }

    #[test]
    fn npm_entry_found_in_node_modules() {
        let dir = scratch("npm-entry-nodemodules");
        let node = dir.join("node.exe");
        std::fs::write(&node, b"").expect("write fake node");
        let npm_bin = dir.join("node_modules").join("npm").join("bin");
        std::fs::create_dir_all(&npm_bin).expect("create npm bin dir");
        let npm_cli = npm_bin.join("npm-cli.js");
        std::fs::write(&npm_cli, b"").expect("write npm-cli.js");
        let installer =
            OpenClawInstaller::new(Arc::new(Scripted::new(Ok(output(0, "", "")))), dir.clone());
        let entry = installer
            .resolve_npm_entry(&node)
            .expect("entry should resolve");
        assert_eq!(entry.npm_cli, npm_cli);
    }

    #[test]
    fn npm_entry_missing_is_npm_not_found() {
        let dir = scratch("npm-entry-missing");
        let node = dir.join("node.exe");
        std::fs::write(&node, b"").expect("write fake node");
        let installer =
            OpenClawInstaller::new(Arc::new(Scripted::new(Ok(output(0, "", "")))), dir.clone());
        let err = installer
            .resolve_npm_entry(&node)
            .expect_err("missing npm-cli.js must fail");
        assert_eq!(err.code, "npm-not-found");
    }

    #[test]
    fn npm_entry_without_node_parent_is_npm_not_found() {
        let installer = OpenClawInstaller::new(
            Arc::new(Scripted::new(Ok(output(0, "", "")))),
            PathBuf::from("/unused"),
        );
        let err = installer
            .resolve_npm_entry(Path::new("node"))
            .expect_err("bare name has no resolvable parent");
        assert_eq!(err.code, "npm-not-found");
    }

    // --- npm version ---------------------------------------------------------

    fn entry_in(dir: &Path) -> NpmEntry {
        NpmEntry {
            node: dir.join("node.exe"),
            npm_cli: dir.join("npm-cli.js"),
        }
    }

    #[test]
    fn npm_version_parses() {
        let (fake, _) = scripted(Ok(output(0, "11.16.0\n", "")));
        let installer = OpenClawInstaller::new(fake, PathBuf::from("/unused"));
        let dir = scratch("npm-version");
        let version = installer
            .npm_version(&entry_in(&dir))
            .expect("version should parse");
        assert_eq!(version, "11.16.0");
    }

    #[test]
    fn npm_version_non_zero_exit_is_process_failed() {
        let (fake, _) = scripted(Ok(output(1, "", "boom")));
        let installer = OpenClawInstaller::new(fake, PathBuf::from("/unused"));
        let err = installer
            .npm_version(&entry_in(Path::new("/unused")))
            .expect_err("non-zero exit must fail");
        assert_eq!(err.code, "process-failed");
    }

    #[test]
    fn npm_version_unparseable_is_process_failed() {
        let (fake, _) = scripted(Ok(output(0, "not a version\n", "")));
        let installer = OpenClawInstaller::new(fake, PathBuf::from("/unused"));
        let err = installer
            .npm_version(&entry_in(Path::new("/unused")))
            .expect_err("unparseable version must fail");
        assert_eq!(err.code, "process-failed");
    }

    #[test]
    fn npm_version_not_found_is_npm_not_found() {
        let (fake, _) = scripted(Err(ProcessError::NotFound {
            executable: "node.exe".to_string(),
        }));
        let installer = OpenClawInstaller::new(fake, PathBuf::from("/unused"));
        let err = installer
            .npm_version(&entry_in(Path::new("/unused")))
            .expect_err("not found must fail");
        assert_eq!(err.code, "npm-not-found");
    }

    #[test]
    fn npm_version_timeout_is_process_timeout() {
        let (fake, _) = scripted(Err(ProcessError::Timeout {
            executable: "node.exe".to_string(),
        }));
        let installer = OpenClawInstaller::new(fake, PathBuf::from("/unused"));
        let err = installer
            .npm_version(&entry_in(Path::new("/unused")))
            .expect_err("timeout must fail");
        assert_eq!(err.code, "process-timeout");
    }

    // --- npm install ---------------------------------------------------------

    #[test]
    fn install_argv_without_allow_scripts_for_old_npm() {
        let (fake, last_request) = scripted(Ok(output(0, "added 1 package in 2s", "")));
        let installer = OpenClawInstaller::new(fake, PathBuf::from("/unused"));
        let dir = scratch("install-argv-old");
        installer
            .install_openclaw_latest(&entry_in(&dir), false)
            .expect("install should succeed");
        let request = last_request
            .lock()
            .unwrap()
            .take()
            .expect("request should be recorded");
        assert_eq!(request.executable, dir.join("node.exe"));
        assert_eq!(
            request.argv,
            vec![
                dir.join("npm-cli.js").to_string_lossy().into_owned(),
                "install".to_string(),
                "-g".to_string(),
                "openclaw@latest".to_string(),
            ]
        );
        assert!(request.env.is_empty(), "no shell, no injected commands");
    }

    #[test]
    fn install_argv_with_allow_scripts_for_new_npm() {
        let (fake, last_request) = scripted(Ok(output(0, "added 1 package in 2s", "")));
        let installer = OpenClawInstaller::new(fake, PathBuf::from("/unused"));
        let dir = scratch("install-argv-new");
        installer
            .install_openclaw_latest(&entry_in(&dir), true)
            .expect("install should succeed");
        let request = last_request
            .lock()
            .unwrap()
            .take()
            .expect("request should be recorded");
        assert!(request
            .argv
            .contains(&"--allow-scripts=openclaw".to_string()));
        assert_eq!(request.timeout, INSTALL_TIMEOUT);
    }

    #[test]
    fn install_non_zero_exit_is_install_failed_with_masked_stderr() {
        // ProcessRunner already masks; the error must keep the masked form.
        let (fake, _) = scripted(Ok(output(3, "", "boom sk-****")));
        let installer = OpenClawInstaller::new(fake, PathBuf::from("/unused"));
        let err = installer
            .install_openclaw_latest(&entry_in(Path::new("/unused")), false)
            .expect_err("non-zero exit must fail");
        assert_eq!(err.code, "openclaw-install-failed");
        assert!(err.message.contains("sk-****"));
    }

    #[test]
    fn install_timeout_is_process_timeout() {
        let (fake, _) = scripted(Err(ProcessError::Timeout {
            executable: "node.exe".to_string(),
        }));
        let installer = OpenClawInstaller::new(fake, PathBuf::from("/unused"));
        let err = installer
            .install_openclaw_latest(&entry_in(Path::new("/unused")), false)
            .expect_err("timeout must fail");
        assert_eq!(err.code, "process-timeout");
        assert!(err.message.contains("npm install -g openclaw@latest"));
    }

    // --- node / npm policy ---------------------------------------------------

    #[test]
    fn node_support_range() {
        let supported = [
            "22.22.3", "22.23.0", "22.99.0", "24.15.0", "24.16.1", "25.9.0", "25.10.0", "26.0.0",
            "26.1.2", "27.0.0",
        ];
        for version in supported {
            assert!(
                node_version_supported(version),
                "{version} must be supported"
            );
        }
        let rejected = [
            "22.22.2", "22.0.0", "23.0.0", "23.11.0", "24.14.9", "25.8.9", "21.7.3", "20.11.1",
            "garbage", "",
        ];
        for version in rejected {
            assert!(
                !node_version_supported(version),
                "{version} must be rejected"
            );
        }
    }

    #[test]
    fn npm_version_parsing() {
        assert_eq!(parse_npm_version("11.15.0\n").as_deref(), Some("11.15.0"));
        assert_eq!(parse_npm_version("  10.9.2 \n").as_deref(), Some("10.9.2"));
        // The first non-empty line is the version; anything else is invalid.
        assert_eq!(parse_npm_version("noise\n12.0.0\n"), None);
        assert_eq!(parse_npm_version("v11.0.0"), None);
        assert_eq!(parse_npm_version("garbage"), None);
        assert_eq!(parse_npm_version("11"), None);
        assert_eq!(parse_npm_version(""), None);
    }

    #[test]
    fn npm_policy_allows_without_flag_below_11_13() {
        for version in ["9.0.0", "10.9.0", "11.0.0", "11.12.0", "11.12.9"] {
            assert_eq!(npm_install_policy(version), Ok(false), "{version}");
        }
    }

    #[test]
    fn npm_policy_blocks_11_13_to_11_15() {
        for version in ["11.13.0", "11.14.2", "11.15.9"] {
            let err = npm_install_policy(version).expect_err("{version} must be blocked");
            assert_eq!(err.code, "unsupported-npm-version");
        }
    }

    #[test]
    fn npm_policy_allows_with_flag_from_11_16() {
        for version in ["11.16.0", "11.16.1", "12.0.0", "13.1.0"] {
            assert_eq!(npm_install_policy(version), Ok(true), "{version}");
        }
    }

    #[test]
    fn npm_policy_rejects_unparseable() {
        let err = npm_install_policy("garbage").expect_err("unparseable must fail");
        assert_eq!(err.code, "process-failed");
    }

    // --- openclaw package entry resolution ------------------------------------

    fn build_package(root: &Path, manifest: Option<&str>, entry_files: &[&str]) {
        let package = root.join("node_modules").join("openclaw");
        std::fs::create_dir_all(&package).expect("create package dir");
        if let Some(manifest) = manifest {
            std::fs::write(package.join("package.json"), manifest).expect("write manifest");
        }
        for relative in entry_files {
            let path = package.join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("create entry parent");
            }
            std::fs::write(&path, b"// fake entry\n").expect("write entry");
        }
    }

    #[test]
    fn entry_resolves_object_bin_form() {
        let root = scratch("entry-object-bin");
        build_package(
            &root,
            Some(r#"{"name":"openclaw","version":"2026.7.1-2","bin":{"openclaw":"openclaw.mjs"}}"#),
            &["openclaw.mjs"],
        );
        let installer =
            OpenClawInstaller::new(Arc::new(Scripted::new(Ok(output(0, "", "")))), root.clone());
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
    fn entry_resolves_string_bin_form() {
        let root = scratch("entry-string-bin");
        build_package(
            &root,
            Some(r#"{"name":"openclaw","version":"2026.7.1-2","bin":"./openclaw.mjs"}"#),
            &["openclaw.mjs"],
        );
        let installer =
            OpenClawInstaller::new(Arc::new(Scripted::new(Ok(output(0, "", "")))), root.clone());
        assert!(installer.resolve_openclaw_entry().is_ok());
    }

    #[test]
    fn entry_resolves_nested_entry_inside_root() {
        let root = scratch("entry-nested");
        build_package(
            &root,
            Some(r#"{"bin":{"openclaw":"bin/openclaw.mjs"}}"#),
            &["bin/openclaw.mjs"],
        );
        let installer =
            OpenClawInstaller::new(Arc::new(Scripted::new(Ok(output(0, "", "")))), root.clone());
        let resolved = installer
            .resolve_openclaw_entry()
            .expect("nested entry should resolve");
        assert!(resolved.entry.starts_with(&resolved.package_root));
    }

    #[test]
    fn entry_rejects_escape_outside_package_root() {
        let root = scratch("entry-escape");
        build_package(&root, Some(r#"{"bin":{"openclaw":"../escape.mjs"}}"#), &[]);
        // The escape target exists but lives outside the package root.
        let escape = root.join("node_modules").join("escape.mjs");
        std::fs::write(&escape, b"// evil\n").expect("write escape file");
        let installer =
            OpenClawInstaller::new(Arc::new(Scripted::new(Ok(output(0, "", "")))), root.clone());
        let err = installer
            .resolve_openclaw_entry()
            .expect_err("escape must be rejected");
        assert_eq!(err.code, "openclaw-install-verify-failed");
        assert!(err.message.contains("escapes the package root"));
    }

    #[test]
    fn entry_missing_package_root_is_verify_failed() {
        let root = scratch("entry-no-package");
        std::fs::create_dir_all(&root).expect("create root");
        let installer =
            OpenClawInstaller::new(Arc::new(Scripted::new(Ok(output(0, "", "")))), root.clone());
        let err = installer
            .resolve_openclaw_entry()
            .expect_err("missing package must fail");
        assert_eq!(err.code, "openclaw-install-verify-failed");
    }

    #[test]
    fn entry_missing_manifest_is_verify_failed() {
        let root = scratch("entry-no-manifest");
        build_package(&root, None, &["openclaw.mjs"]);
        let installer =
            OpenClawInstaller::new(Arc::new(Scripted::new(Ok(output(0, "", "")))), root.clone());
        assert_eq!(
            installer.resolve_openclaw_entry().unwrap_err().code,
            "openclaw-install-verify-failed"
        );
    }

    #[test]
    fn entry_invalid_json_is_verify_failed() {
        let root = scratch("entry-bad-json");
        build_package(&root, Some("not json"), &["openclaw.mjs"]);
        let installer =
            OpenClawInstaller::new(Arc::new(Scripted::new(Ok(output(0, "", "")))), root.clone());
        assert_eq!(
            installer.resolve_openclaw_entry().unwrap_err().code,
            "openclaw-install-verify-failed"
        );
    }

    #[test]
    fn entry_without_bin_is_verify_failed() {
        let root = scratch("entry-no-bin");
        build_package(&root, Some(r#"{"name":"openclaw"}"#), &["openclaw.mjs"]);
        let installer =
            OpenClawInstaller::new(Arc::new(Scripted::new(Ok(output(0, "", "")))), root.clone());
        assert_eq!(
            installer.resolve_openclaw_entry().unwrap_err().code,
            "openclaw-install-verify-failed"
        );
    }

    #[test]
    fn entry_bin_missing_openclaw_key_is_verify_failed() {
        let root = scratch("entry-bin-other-name");
        build_package(
            &root,
            Some(r#"{"bin":{"other":"openclaw.mjs"}}"#),
            &["openclaw.mjs"],
        );
        let installer =
            OpenClawInstaller::new(Arc::new(Scripted::new(Ok(output(0, "", "")))), root.clone());
        assert_eq!(
            installer.resolve_openclaw_entry().unwrap_err().code,
            "openclaw-install-verify-failed"
        );
    }

    #[test]
    fn entry_empty_global_root_is_verify_failed() {
        let installer = OpenClawInstaller::new(
            Arc::new(Scripted::new(Ok(output(0, "", "")))),
            PathBuf::new(),
        );
        let err = installer
            .resolve_openclaw_entry()
            .expect_err("empty prefix must fail closed");
        assert_eq!(err.code, "openclaw-install-verify-failed");
    }
}
