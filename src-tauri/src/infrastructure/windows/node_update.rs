//! `NodeUpdateAdapter` — Phase 8.1 one-shot Node.js update via winget.
//!
//! Every invocation is a structured `executable + argv` request through the
//! `ProcessPort` (S1/S2):
//!
//! 1. `winget --version` (10s) — availability probe (`winget-not-found` on
//!    absence, 0 install attempts)
//! 2. `winget install --id OpenJS.NodeJS.LTS --exact --silent
//!    --disable-interactivity --accept-source-agreements
//!    --accept-package-agreements` (900s — same budget as the Phase 2
//!    `INSTALL_TIMEOUT`)
//! 3. Post-update re-detection (`node --version`, 10s per candidate):
//!    ① the standard MSI location (`C:\Program Files\nodejs\node.exe`) —
//!    fresh after the install, because the running app's PATH block is
//!    stale — ② the PATH-resolved `node.exe`. Absence of every candidate
//!    is `Ok(NodeDetection::NotFound)`; the service decides what that
//!    means. The re-detection result is authoritative — a winget exit 0
//!    with an unsupported re-detection is a failure at the service layer.
//!
//! There is zero user input in any argv (S2): the package id, flags, and
//! probe flags are all fixed literals.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use crate::domain::models::windows::NodeDetection;
use crate::domain::ports::node_update::NodeUpdatePort;
use crate::domain::ports::process::{ProcessError, ProcessOutput, ProcessPort, ProcessRequest};
use crate::error::AppError;
use crate::infrastructure::windows::system::parse_node_version;

/// Probe timeout for `winget --version` and `node --version`.
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
/// Update budget — identical to the Phase 2 `INSTALL_TIMEOUT` (15 min).
const WINGET_INSTALL_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// The standard MSI install location of Node.js on Windows x64.
pub const DEFAULT_MSI_NODE_PATH: &str = r"C:\Program Files\nodejs\node.exe";

/// One-shot winget-based Node.js updater.
pub struct NodeUpdateAdapter {
    process: Arc<dyn ProcessPort>,
    winget_executable: PathBuf,
    /// ① re-detection candidate: the standard MSI location.
    msi_node_path: PathBuf,
    /// ② re-detection candidate: an explicit fallback path. `None` in
    /// production → resolve `node.exe` on PATH.
    path_fallback: Option<PathBuf>,
}

impl NodeUpdateAdapter {
    /// Production wiring: `winget` by name on PATH, standard MSI location,
    /// PATH fallback.
    pub fn new(process: Arc<dyn ProcessPort>) -> Self {
        Self {
            process,
            winget_executable: PathBuf::from("winget"),
            msi_node_path: PathBuf::from(DEFAULT_MSI_NODE_PATH),
            path_fallback: None,
        }
    }

    /// Test wiring: explicit winget binary, MSI candidate, and fallback.
    pub fn with_paths(
        process: Arc<dyn ProcessPort>,
        winget_executable: PathBuf,
        msi_node_path: PathBuf,
        path_fallback: Option<PathBuf>,
    ) -> Self {
        Self {
            process,
            winget_executable,
            msi_node_path,
            path_fallback,
        }
    }

    /// Runs a winget invocation, mapping process errors to stable codes.
    fn run_winget(&self, argv: Vec<String>, timeout: Duration) -> Result<ProcessOutput, AppError> {
        let request = ProcessRequest::new(self.winget_executable.clone(), argv, timeout);
        match self.process.run(&request) {
            Ok(output) => Ok(output),
            Err(ProcessError::NotFound { .. }) => Err(AppError::winget_not_found()),
            Err(ProcessError::Timeout { executable }) => {
                Err(AppError::process_timeout(&format!("winget {executable}")))
            }
            Err(ProcessError::SpawnFailed { message }) => {
                Err(AppError::process_failed("winget", message))
            }
        }
    }

    /// The one-shot upsert. Fixed argv — zero user input (S2).
    fn install_node(&self) -> Result<(), AppError> {
        const LABEL: &str = "winget install OpenJS.NodeJS.LTS";
        let output = self.run_winget(
            vec![
                "install".to_string(),
                "--id".to_string(),
                "OpenJS.NodeJS.LTS".to_string(),
                "--exact".to_string(),
                "--silent".to_string(),
                "--disable-interactivity".to_string(),
                "--accept-source-agreements".to_string(),
                "--accept-package-agreements".to_string(),
            ],
            WINGET_INSTALL_TIMEOUT,
        )?;
        if output.exit_code != 0 {
            return Err(AppError::node_update_failed(format!(
                "{LABEL}: exit code {}: {}",
                output.exit_code,
                output.stderr.trim()
            )));
        }
        Ok(())
    }

    /// Probes `node --version` at one candidate. `Ok(None)` = candidate
    /// absent or unusable (a value, not an error).
    fn probe_node(&self, executable: &Path) -> Result<Option<NodeDetection>, AppError> {
        let request = ProcessRequest::new(
            executable.to_path_buf(),
            vec!["--version".to_string()],
            PROBE_TIMEOUT,
        );
        match self.process.run(&request) {
            Ok(output) if output.exit_code == 0 => match parse_node_version(&output.stdout) {
                Ok(version) => Ok(Some(NodeDetection::Found { version })),
                // Unparseable version output: treat as unusable, fall on.
                Err(_) => Ok(None),
            },
            Ok(_) => Ok(None),
            Err(ProcessError::NotFound { .. }) => Ok(None),
            Err(ProcessError::Timeout { .. }) => Err(AppError::process_timeout("node --version")),
            Err(ProcessError::SpawnFailed { message }) => {
                Err(AppError::process_failed("node --version", message))
            }
        }
    }

    /// Post-update re-detection: MSI location first (fresh), then the
    /// fallback (PATH-resolved in production).
    fn redetect_node(&self) -> Result<NodeDetection, AppError> {
        if self.msi_node_path.is_file() {
            if let Some(detected) = self.probe_node(&self.msi_node_path)? {
                return Ok(detected);
            }
        }
        let fallback = match &self.path_fallback {
            Some(path) => Some(path.clone()),
            None => resolve_node_on_path(),
        };
        if let Some(path) = fallback {
            if let Some(detected) = self.probe_node(&path)? {
                return Ok(detected);
            }
        }
        Ok(NodeDetection::NotFound)
    }
}

/// Resolves `node.exe` on PATH — the same discovery the Phase 1
/// `node --version` probe relies on (bare-name spawn).
fn resolve_node_on_path() -> Option<PathBuf> {
    let Ok(path_var) = std::env::var("PATH") else {
        return None;
    };
    for dir in std::env::split_paths(&path_var) {
        for name in ["node.exe", "node"] {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

impl NodeUpdatePort for NodeUpdateAdapter {
    fn update_node(&self) -> Result<NodeDetection, AppError> {
        // 1. Availability probe — a missing winget stops before any
        //    install attempt (0 mutation).
        let probe = self.run_winget(vec!["--version".to_string()], PROBE_TIMEOUT)?;
        if probe.exit_code != 0 {
            return Err(AppError::node_update_failed(format!(
                "winget availability probe: exit code {}: {}",
                probe.exit_code,
                probe.stderr.trim()
            )));
        }
        // 2. One-shot upsert (install works whether or not the package is
        //    winget-registered).
        self.install_node()?;
        // 3. Verify by re-detection — never trust exit code 0 alone.
        self.redetect_node()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::Mutex;

    struct ScriptedProcess {
        responses: Arc<Mutex<Vec<Result<ProcessOutput, ProcessError>>>>,
        requests: Arc<Mutex<Vec<ProcessRequest>>>,
    }

    impl ProcessPort for ScriptedProcess {
        fn run(&self, request: &ProcessRequest) -> Result<ProcessOutput, ProcessError> {
            self.requests.lock().unwrap().push(request.clone());
            let mut queue = self.responses.lock().unwrap();
            match queue.first().cloned() {
                Some(response) => {
                    let _ = queue.remove(0);
                    response
                }
                // An exhausted queue means "nothing to spawn" — model it as
                // a missing executable, like a real spawn failure.
                None => Err(ProcessError::NotFound {
                    executable: request.executable.display().to_string(),
                }),
            }
        }
    }

    fn scripted(
        responses: Vec<Result<ProcessOutput, ProcessError>>,
    ) -> (Arc<Mutex<Vec<ProcessRequest>>>, Arc<dyn ProcessPort>) {
        let requests: Arc<Mutex<Vec<ProcessRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let fake: Arc<dyn ProcessPort> = Arc::new(ScriptedProcess {
            responses: Arc::new(Mutex::new(responses)),
            requests: Arc::clone(&requests),
        });
        (requests, fake)
    }

    fn output(exit_code: i32, stdout: &str, stderr: &str) -> ProcessOutput {
        ProcessOutput {
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
            exit_code,
        }
    }

    const WINGET: &str = "C:\\fake\\winget.exe";
    const MSI_NODE: &str = "C:\\fake\\msi\\node.exe";
    const PATH_NODE: &str = "C:\\fake\\path\\node.exe";

    /// A scratch file that makes the MSI candidate "exist".
    fn msi_file(tag: &str) -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("clawdesk_node_update_unit")
            .join(tag);
        fs::create_dir_all(&dir).expect("create unit scratch dir");
        let file = dir.join("node.exe");
        if !file.exists() {
            fs::write(&file, b"unit-test-msi-node").expect("write unit scratch file");
        }
        file
    }

    const INSTALL_ARGV: [&str; 8] = [
        "install",
        "--id",
        "OpenJS.NodeJS.LTS",
        "--exact",
        "--silent",
        "--disable-interactivity",
        "--accept-source-agreements",
        "--accept-package-agreements",
    ];

    fn install_argv_strings() -> Vec<String> {
        INSTALL_ARGV.iter().map(|arg| arg.to_string()).collect()
    }

    #[test]
    fn update_node_probe_install_redetect_order() {
        let msi = msi_file("order");
        let responses = vec![
            Ok(output(0, "v1.8.2000\n", "")), // winget --version
            Ok(output(0, "installed\n", "")), // winget install
            Ok(output(0, "v24.15.0\n", "")),  // MSI node --version
        ];
        let (requests, process) = scripted(responses);
        let adapter = NodeUpdateAdapter::with_paths(
            process,
            WINGET.into(),
            msi.clone(),
            Some(PATH_NODE.into()),
        );
        let detected = adapter.update_node().expect("update + redetect");
        assert_eq!(
            detected,
            NodeDetection::Found {
                version: "24.15.0".into()
            }
        );

        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 3, "probe → install → MSI probe only");
        assert_eq!(requests[0].executable, PathBuf::from(WINGET));
        assert_eq!(requests[0].argv, vec!["--version".to_string()]);
        assert_eq!(requests[0].timeout, PROBE_TIMEOUT);
        assert_eq!(requests[1].executable, PathBuf::from(WINGET));
        assert_eq!(requests[1].argv, install_argv_strings());
        assert_eq!(requests[1].timeout, WINGET_INSTALL_TIMEOUT);
        // The fresh MSI location is probed before any PATH fallback.
        assert_eq!(requests[2].executable, msi);
        assert_eq!(requests[2].argv, vec!["--version".to_string()]);
        assert_eq!(requests[2].timeout, PROBE_TIMEOUT);
    }

    #[test]
    fn winget_probe_not_found_is_winget_not_found() {
        let (requests, process) = scripted(vec![Err(ProcessError::NotFound {
            executable: WINGET.into(),
        })]);
        let adapter = NodeUpdateAdapter::with_paths(process, WINGET.into(), MSI_NODE.into(), None);
        let err = adapter.update_node().expect_err("missing winget must fail");
        assert_eq!(err.code, "winget-not-found");
        assert_eq!(requests.lock().unwrap().len(), 1, "0 install attempts");
    }

    #[test]
    fn winget_probe_nonzero_is_update_failed() {
        let (requests, process) = scripted(vec![Ok(output(1, "", "probe failed"))]);
        let adapter = NodeUpdateAdapter::with_paths(process, WINGET.into(), MSI_NODE.into(), None);
        let err = adapter.update_node().expect_err("bad probe must fail");
        assert_eq!(err.code, "node-update-failed");
        assert!(err.message.contains("probe failed"));
        assert_eq!(requests.lock().unwrap().len(), 1, "0 install attempts");
    }

    #[test]
    fn install_nonzero_is_update_failed() {
        let (requests, process) = scripted(vec![
            Ok(output(0, "v1.8.2000\n", "")),
            Ok(output(1, "", "source agreement not accepted")),
        ]);
        let adapter = NodeUpdateAdapter::with_paths(process, WINGET.into(), MSI_NODE.into(), None);
        let err = adapter
            .update_node()
            .expect_err("non-zero install must fail");
        assert_eq!(err.code, "node-update-failed");
        assert!(err.message.contains("source agreement not accepted"));
        assert_eq!(
            requests.lock().unwrap().len(),
            2,
            "no re-detection after failure"
        );
    }

    #[test]
    fn install_timeout_is_process_timeout() {
        let (requests, process) = scripted(vec![
            Ok(output(0, "v1.8.2000\n", "")),
            Err(ProcessError::Timeout {
                executable: WINGET.into(),
            }),
        ]);
        let adapter = NodeUpdateAdapter::with_paths(process, WINGET.into(), MSI_NODE.into(), None);
        let err = adapter.update_node().expect_err("timeout must fail");
        assert_eq!(err.code, "process-timeout");
        assert_eq!(
            requests.lock().unwrap().len(),
            2,
            "no re-detection after timeout"
        );
    }

    #[test]
    fn msi_absent_falls_back_to_injected_candidate() {
        let missing_msi = PathBuf::from("C:\\fake\\msi-absent\\node.exe");
        let responses = vec![
            Ok(output(0, "v1.8.2000\n", "")),
            Ok(output(0, "installed\n", "")),
            Ok(output(0, "v26.1.0\n", "")), // fallback node --version
        ];
        let (requests, process) = scripted(responses);
        let adapter = NodeUpdateAdapter::with_paths(
            process,
            WINGET.into(),
            missing_msi.clone(),
            Some(PATH_NODE.into()),
        );
        let detected = adapter.update_node().expect("fallback probe");
        assert_eq!(
            detected,
            NodeDetection::Found {
                version: "26.1.0".into()
            }
        );
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[2].executable, PathBuf::from(PATH_NODE));
    }

    #[test]
    fn all_candidates_absent_is_not_found() {
        let responses = vec![
            Ok(output(0, "v1.8.2000\n", "")),
            Ok(output(0, "installed\n", "")),
        ];
        let (requests, process) = scripted(responses);
        let adapter = NodeUpdateAdapter::with_paths(
            process,
            WINGET.into(),
            PathBuf::from("C:\\fake\\msi-absent\\node.exe"),
            Some(PathBuf::from("C:\\fake\\path-absent\\node.exe")),
        );
        let detected = adapter.update_node().expect("absence is a value");
        assert_eq!(detected, NodeDetection::NotFound);
        // probe + install + the fallback probe (absent → NotFound → None).
        assert_eq!(requests.lock().unwrap().len(), 3);
    }

    #[test]
    fn msi_probe_unparseable_falls_back() {
        let msi = msi_file("unparseable");
        let responses = vec![
            Ok(output(0, "v1.8.2000\n", "")),
            Ok(output(0, "installed\n", "")),
            Ok(output(0, "garbage\n", "")),  // MSI probe unparseable
            Ok(output(0, "v24.15.0\n", "")), // fallback probe
        ];
        let (requests, process) = scripted(responses);
        let adapter = NodeUpdateAdapter::with_paths(
            process,
            WINGET.into(),
            msi.clone(),
            Some(PATH_NODE.into()),
        );
        let detected = adapter
            .update_node()
            .expect("fallback after bad MSI output");
        assert_eq!(
            detected,
            NodeDetection::Found {
                version: "24.15.0".into()
            }
        );
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 4);
        assert_eq!(requests[3].executable, PathBuf::from(PATH_NODE));
    }
}
