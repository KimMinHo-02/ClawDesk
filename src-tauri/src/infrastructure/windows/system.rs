//! `WindowsSystemAdapter` — Windows version, architecture, and Node detection.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::domain::models::windows::{Architecture, NodeDetection, WindowsVersion};
use crate::domain::ports::process::{ProcessError, ProcessPort, ProcessRequest};
use crate::domain::ports::windows_system::WindowsSystemPort;
use crate::error::AppError;
use crate::infrastructure::masking::mask_secrets;

/// Timeout for the `node --version` probe.
const NODE_TIMEOUT: Duration = Duration::from_secs(10);
/// First Windows 10 build.
const FIRST_WIN10_BUILD: u32 = 10240;
/// First Windows 11 build.
const FIRST_WIN11_BUILD: u32 = 22000;

const CURRENT_VERSION_KEY: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";

/// Native system architecture codes (as returned by kernel32).
const ARCH_INTEL: u32 = 0;
const ARCH_AMD64: u32 = 9;
const ARCH_ARM64: u32 = 12;
/// Sentinel for "could not determine".
const ARCH_UNKNOWN: u32 = u32::MAX;

/// Windows system detection adapter.
pub struct WindowsSystemAdapter {
    process: Arc<dyn ProcessPort>,
    node_executable: PathBuf,
}

impl WindowsSystemAdapter {
    pub fn new(process: Arc<dyn ProcessPort>) -> Self {
        Self {
            process,
            node_executable: PathBuf::from("node"),
        }
    }

    /// Test/support constructor with an explicit Node lookup name.
    pub fn with_node_executable(process: Arc<dyn ProcessPort>, node_executable: PathBuf) -> Self {
        Self {
            process,
            node_executable,
        }
    }
}

impl WindowsSystemPort for WindowsSystemAdapter {
    fn os_version(&self) -> Result<WindowsVersion, AppError> {
        windows_version_from_registry()
    }

    fn architecture(&self) -> Result<Architecture, AppError> {
        map_native_architecture(native_architecture_raw())
    }

    fn detect_node(&self) -> Result<NodeDetection, AppError> {
        let request = ProcessRequest::new(
            self.node_executable.clone(),
            vec!["--version".to_string()],
            NODE_TIMEOUT,
        );
        let output = match self.process.run(&request) {
            Ok(output) => output,
            Err(ProcessError::NotFound { .. }) => return Ok(NodeDetection::NotFound),
            Err(ProcessError::Timeout { executable }) => {
                return Err(AppError::process_timeout(&format!("node {executable}")));
            }
            Err(ProcessError::SpawnFailed { message }) => {
                return Err(AppError::process_failed("node --version", message));
            }
        };
        if output.exit_code != 0 {
            return Err(AppError::process_failed(
                "node --version",
                format!("exit code {}: {}", output.exit_code, output.stderr.trim()),
            ));
        }
        let version = parse_node_version(&output.stdout)?;
        Ok(NodeDetection::Found { version })
    }

    fn node_executable(&self) -> Result<PathBuf, AppError> {
        // A configured absolute path is authoritative (test/support wiring).
        if self.node_executable.is_absolute() {
            return if self.node_executable.is_file() {
                Ok(self.node_executable.clone())
            } else {
                Err(AppError::node_not_found())
            };
        }
        // Otherwise resolve `node.exe` on PATH — the same discovery the
        // `node --version` probe relies on (spawn of the bare `node` name).
        if let Ok(path_var) = std::env::var("PATH") {
            for dir in std::env::split_paths(&path_var) {
                for name in ["node.exe", "node"] {
                    let candidate = dir.join(name);
                    if candidate.is_file() {
                        return Ok(candidate);
                    }
                }
            }
        }
        Err(AppError::node_not_found())
    }
}

/// Maps a kernel32 architecture code to the supported architecture.
pub fn map_native_architecture(raw: u32) -> Result<Architecture, AppError> {
    match raw {
        ARCH_AMD64 => Ok(Architecture::X64),
        other => Err(AppError::unsupported_architecture(describe_architecture(
            other,
        ))),
    }
}

fn describe_architecture(raw: u32) -> String {
    match raw {
        ARCH_INTEL => "x86".to_string(),
        ARCH_ARM64 => "arm64".to_string(),
        ARCH_UNKNOWN => "unknown".to_string(),
        other => format!("unknown({other})"),
    }
}

/// Maps a build number to the Windows major product version.
///
/// Unsupported (pre-Windows 10) builds are a structured error.
pub fn build_to_major_version(build: u32) -> Result<u32, AppError> {
    if build >= FIRST_WIN11_BUILD {
        Ok(11)
    } else if build >= FIRST_WIN10_BUILD {
        Ok(10)
    } else {
        Err(AppError::unsupported_os_version(build))
    }
}

fn native_architecture_raw() -> u32 {
    #[cfg(windows)]
    {
        use std::mem;
        use windows::Win32::System::SystemInformation::{GetNativeSystemInfo, SYSTEM_INFO};
        // GetNativeSystemInfo reports the machine's native architecture
        // (unaffected by WOW64/emulation of this process). It cannot fail on
        // Windows 10/11; if it did, the zeroed value maps to x86 and the
        // structured unsupported-architecture error is reported.
        let mut info: SYSTEM_INFO = unsafe { mem::zeroed() };
        unsafe { GetNativeSystemInfo(&mut info) };
        // SAFETY: GetNativeSystemInfo just filled the struct, so the union
        // member backing wProcessorArchitecture is initialized.
        let arch = unsafe { info.Anonymous.Anonymous.wProcessorArchitecture.0 };
        arch as u32
    }
    #[cfg(not(windows))]
    {
        ARCH_UNKNOWN
    }
}

#[cfg(windows)]
fn windows_version_from_registry() -> Result<WindowsVersion, AppError> {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;

    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let key = hklm
        .open_subkey(CURRENT_VERSION_KEY)
        .map_err(|err| AppError::os_info_unavailable(mask_secrets(&err.to_string())))?;

    let build = read_build(&key)?;
    let ubr: u32 = key.get_value("UBR").unwrap_or(0);
    let product_name: Option<String> = key.get_value("ProductName").ok();
    let major_version = build_to_major_version(build)?;

    Ok(WindowsVersion {
        major_version,
        build,
        ubr,
        product_name,
    })
}

#[cfg(not(windows))]
fn windows_version_from_registry() -> Result<WindowsVersion, AppError> {
    Err(AppError::os_info_unavailable("not a Windows host"))
}

#[cfg(windows)]
fn read_build(key: &winreg::RegKey) -> Result<u32, AppError> {
    if let Ok(value) = key.get_value::<String, _>("CurrentBuild") {
        if let Ok(build) = value.trim().parse::<u32>() {
            return Ok(build);
        }
    }
    if let Ok(value) = key.get_value::<u32, _>("CurrentBuildNumber") {
        return Ok(value);
    }
    Err(AppError::os_info_unavailable(
        "registry CurrentVersion key has neither CurrentBuild nor CurrentBuildNumber",
    ))
}

/// Parses `node --version` output: first non-empty line, leading `v` stripped.
pub fn parse_node_version(stdout: &str) -> Result<String, AppError> {
    let line = stdout
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("");
    let version = line.strip_prefix('v').unwrap_or(line);
    let valid = version.chars().next().is_some_and(|c| c.is_ascii_digit());
    if !valid {
        return Err(AppError::node_version_unavailable(format!(
            "cannot parse Node.js version from output: {line}"
        )));
    }
    Ok(version.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_mapping_windows_11() {
        assert_eq!(build_to_major_version(26100).unwrap(), 11);
        assert_eq!(build_to_major_version(22000).unwrap(), 11);
    }

    #[test]
    fn build_mapping_windows_10() {
        assert_eq!(build_to_major_version(19045).unwrap(), 10);
        assert_eq!(build_to_major_version(10240).unwrap(), 10);
    }

    #[test]
    fn build_mapping_unsupported() {
        let err = build_to_major_version(9600).unwrap_err();
        assert_eq!(err.code, "unsupported-os-version");
    }

    #[test]
    fn architecture_amd64_is_supported() {
        assert_eq!(map_native_architecture(9).unwrap(), Architecture::X64);
    }

    #[test]
    fn architecture_arm64_is_unsupported_error() {
        let err = map_native_architecture(12).unwrap_err();
        assert_eq!(err.code, "unsupported-architecture");
        assert!(err.message.contains("arm64"));
    }

    #[test]
    fn architecture_x86_is_unsupported_error() {
        let err = map_native_architecture(0).unwrap_err();
        assert_eq!(err.code, "unsupported-architecture");
        assert!(err.message.contains("x86"));
    }

    #[test]
    fn node_version_parses_with_v_prefix() {
        assert_eq!(parse_node_version("v22.14.0\n").unwrap(), "22.14.0");
    }

    #[test]
    fn node_version_parses_without_prefix() {
        assert_eq!(parse_node_version("20.11.1").unwrap(), "20.11.1");
    }

    #[test]
    fn node_version_rejects_garbage() {
        let err = parse_node_version("not a version\n").unwrap_err();
        assert_eq!(err.code, "node-version-unavailable");
    }

    #[test]
    fn node_version_rejects_empty() {
        assert!(parse_node_version("").is_err());
    }
}
