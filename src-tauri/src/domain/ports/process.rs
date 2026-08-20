//! Process execution port.
//!
//! Security invariants S1/S2: every process run is a structured
//! `executable + argv` request. No shell strings, ever.

use std::path::PathBuf;
use std::time::Duration;

/// A structured process run request.
#[derive(Debug, Clone)]
pub struct ProcessRequest {
    pub executable: PathBuf,
    pub argv: Vec<String>,
    pub timeout: Duration,
    /// Extra environment variables for the child (parent env is inherited).
    pub env: Vec<(String, String)>,
}

impl ProcessRequest {
    pub fn new(executable: impl Into<PathBuf>, argv: Vec<String>, timeout: Duration) -> Self {
        Self {
            executable: executable.into(),
            argv,
            timeout,
            env: Vec::new(),
        }
    }
}

/// Captured process result. stdout/stderr are already masked (S8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

/// Structured process failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessError {
    /// The executable does not exist (structured "not found").
    NotFound { executable: String },
    /// The process could not be started, for a reason other than a
    /// missing executable.
    SpawnFailed { message: String },
    /// The process exceeded its timeout and was terminated.
    Timeout { executable: String },
}

impl std::fmt::Display for ProcessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound { executable } => {
                write!(f, "executable not found: {executable}")
            }
            Self::SpawnFailed { message } => write!(f, "spawn failed: {message}"),
            Self::Timeout { executable } => write!(f, "process timed out: {executable}"),
        }
    }
}

impl std::error::Error for ProcessError {}

/// The single port through which all processes are started.
///
/// `ProcessRunner` is the only spawn point in ClawDesk.
/// `Send + Sync` so the port can be shared across Tauri's async commands.
pub trait ProcessPort: Send + Sync {
    fn run(&self, request: &ProcessRequest) -> Result<ProcessOutput, ProcessError>;
}
