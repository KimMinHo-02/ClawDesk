//! Domain types for Windows environment detection.

/// Detected CPU architecture. Non-x64 is reported as a structured error,
/// so only the supported variant exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Architecture {
    X64,
}

/// Detected Windows version information.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowsVersion {
    /// Major product version: 10 (Windows 10) or 11 (Windows 11).
    pub major_version: u32,
    /// OS build number, e.g. 26100.
    pub build: u32,
    /// Update Build Revision (UBR). 0 when not set.
    pub ubr: u32,
    /// Registry product name, e.g. "Windows 11 Pro". `None` when unreadable.
    pub product_name: Option<String>,
}

/// Node.js detection result.
///
/// `NotFound` is a structured "not found" value, not an error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeDetection {
    NotFound,
    Found { version: String },
}
