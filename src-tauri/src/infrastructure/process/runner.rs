//! `ProcessRunner` — the single process spawn point in ClawDesk (S1).

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::domain::ports::process::{ProcessError, ProcessOutput, ProcessPort, ProcessRequest};
use crate::infrastructure::masking::mask_secrets;

/// Interval between `try_wait` polls while waiting for a child process.
const WAIT_POLL_INTERVAL: Duration = Duration::from_millis(25);

/// Spawns structured `executable + argv` processes with a timeout and
/// captures masked stdout/stderr.
#[derive(Debug)]
pub struct ProcessRunner;

impl ProcessPort for ProcessRunner {
    fn run(&self, request: &ProcessRequest) -> Result<ProcessOutput, ProcessError> {
        let mut command = Command::new(&request.executable);
        command.args(&request.argv);
        for (key, value) in &request.env {
            command.env(key, value);
        }

        let mut child = match command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Err(ProcessError::NotFound {
                    executable: request.executable.display().to_string(),
                });
            }
            Err(err) => {
                return Err(ProcessError::SpawnFailed {
                    message: mask_secrets(&err.to_string()),
                });
            }
        };

        // Drain stdout/stderr on reader threads so piped output cannot dead
        // lock the process while we wait for exit with a timeout.
        let stdout_pipe = child.stdout.take().expect("stdout was piped");
        let stderr_pipe = child.stderr.take().expect("stderr was piped");
        let (stdout_buf, stdout_thread) = read_pipe(stdout_pipe);
        let (stderr_buf, stderr_thread) = read_pipe(stderr_pipe);

        let exit_code = match wait_within(
            &mut child,
            request.timeout,
            request.executable.display().to_string(),
        ) {
            Ok(code) => code,
            Err(err) => {
                // On timeout the child was killed, so pipes close and the
                // reader threads finish shortly; drain before returning.
                let _ = stdout_thread.join();
                let _ = stderr_thread.join();
                return Err(err);
            }
        };

        // The child has exited, pipes are closed, readers will finish.
        let _ = stdout_thread.join();
        let _ = stderr_thread.join();

        Ok(ProcessOutput {
            stdout: masked_output(&stdout_buf),
            stderr: masked_output(&stderr_buf),
            exit_code,
        })
    }
}

type SharedBuffer = Arc<Mutex<Vec<u8>>>;

/// Reads a child pipe fully on a background thread.
fn read_pipe<R: Read + Send + 'static>(pipe: R) -> (SharedBuffer, thread::JoinHandle<()>) {
    let buf: SharedBuffer = Arc::new(Mutex::new(Vec::new()));
    let shared = Arc::clone(&buf);
    let handle = thread::spawn(move || {
        let mut reader = pipe;
        let mut chunk = [0u8; 8192];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => shared.lock().unwrap().extend_from_slice(&chunk[..n]),
                Err(_) => break,
            }
        }
    });
    (buf, handle)
}

/// Waits for the child until it exits or `timeout` elapses.
///
/// Returns the exit code, or `ProcessError::Timeout` after killing the child.
fn wait_within(
    child: &mut Child,
    timeout: Duration,
    executable: String,
) -> Result<i32, ProcessError> {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status.code().unwrap_or(-1)),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(ProcessError::Timeout { executable });
                }
                thread::sleep(WAIT_POLL_INTERVAL);
            }
            Err(err) => {
                return Err(ProcessError::SpawnFailed {
                    message: mask_secrets(&err.to_string()),
                });
            }
        }
    }
}

/// Converts a captured pipe buffer to a masked, lossy UTF-8 string.
fn masked_output(buf: &SharedBuffer) -> String {
    let raw = buf.lock().unwrap();
    mask_secrets(&String::from_utf8_lossy(&raw))
}
