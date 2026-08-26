//! Phase 7 contract tests: real `ProcessRunner` + fake `openclaw` CLI with a
//! per-test state sandbox (S5: fake CLI only, no real OpenClaw, no system
//! mutation — sandboxes live under the cargo target dir).
//!
//! The contract asserts the exact `automations` argv byte-for-byte (S1/S2),
//! the fixed session pairing (reminder → `main` + `--system-event` +
//! `--wake`, task → `isolated` + `--message`, no `--wake`), zero non-goal
//! flags in every captured argv, fail-closed validation (0 CLI calls), and
//! the stable error codes. `automations run`/`runs` are never invoked
//! (non-goal regression gate).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use clawdesk_lib::application::AutomationService;
use clawdesk_lib::domain::models::openclaw::{GatewayStatus, OpenClawVersion, UpdateState};
use clawdesk_lib::domain::models::ExecutableDetection;
use clawdesk_lib::domain::ports::openclaw::OpenClawPort;
use clawdesk_lib::domain::ports::openclaw_automations::OpenClawAutomationsPort;
use clawdesk_lib::domain::ports::process::{
    ProcessError, ProcessOutput, ProcessPort, ProcessRequest,
};
use clawdesk_lib::error::AppError;
use clawdesk_lib::infrastructure::openclaw::OpenClawAutomationsAdapter;
use clawdesk_lib::infrastructure::process::ProcessRunner;

const FAKE_OPENCLAW: &str = env!("CARGO_BIN_EXE_clawdesk-fake-openclaw");
const TIMEOUT: Duration = Duration::from_secs(10);

/// The non-goal flags that must never appear in any captured argv
/// (contract §3 — delivery routing / payload / model override surface).
const NON_GOAL_FLAGS: [&str; 13] = [
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

fn target_dir() -> PathBuf {
    std::env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target"))
}

fn stamp() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

struct Sandbox {
    dir: PathBuf,
    capture: PathBuf,
}

impl Sandbox {
    fn new(tag: &str) -> Self {
        let dir = target_dir().join("clawdesk_test_state").join(format!(
            "{tag}-{}-{}",
            std::process::id(),
            stamp()
        ));
        fs::create_dir_all(&dir).expect("create sandbox");
        let capture = dir.join("capture.jsonl");
        Sandbox { dir, capture }
    }

    fn seed(&self, state: serde_json::Value) {
        fs::write(self.dir.join("openclaw.json"), state.to_string()).expect("seed state");
    }

    fn state(&self) -> serde_json::Value {
        fs::read_to_string(self.dir.join("openclaw.json"))
            .expect("state file exists")
            .parse()
            .expect("state is json")
    }

    fn state_body(&self) -> String {
        fs::read_to_string(self.dir.join("openclaw.json")).expect("state file exists")
    }

    fn captured(&self) -> Vec<Vec<String>> {
        let body = fs::read_to_string(&self.capture).expect("capture file exists");
        body.lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| {
                serde_json::from_str::<Vec<String>>(line)
                    .unwrap_or_else(|err| panic!("bad capture line {line:?}: {err}"))
            })
            .collect()
    }
}

/// No captured argv may carry a non-goal flag (contract §3).
fn assert_no_non_goal_flags(captured: &[Vec<String>]) {
    for line in captured {
        for arg in line {
            assert!(
                !NON_GOAL_FLAGS.contains(&arg.as_str()),
                "non-goal flag {arg} in argv: {line:?}"
            );
        }
    }
}

/// Temporarily sets inherited-process environment (removed on drop).
struct GlobalEnvGuard {
    keys: Vec<String>,
}

impl GlobalEnvGuard {
    fn set(pairs: &[(&str, &str)]) -> Self {
        let mut guard = GlobalEnvGuard { keys: Vec::new() };
        for (key, value) in pairs {
            std::env::set_var(key, value);
            guard.keys.push(key.to_string());
        }
        guard
    }
}

impl Drop for GlobalEnvGuard {
    fn drop(&mut self) {
        for key in &self.keys {
            std::env::remove_var(key);
        }
    }
}

/// Serializes scenario-based tests: the fake CLI receives its sandbox through
/// inherited (global) env, so two scenario tests running in parallel would
/// race on `CLAWDESK_FAKE_STATE`/`CLAWDESK_FAKE_CAPTURE`.
static SCENARIO_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Runs one serialized adapter/service-driven sub-scenario in its own
/// sandbox.
fn scenario(tag: &str, behavior: Option<&str>, run: impl FnOnce(&Sandbox)) {
    let _lock = SCENARIO_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let sandbox = Sandbox::new(tag);
    let mut pairs: Vec<(String, String)> = vec![
        (
            "CLAWDESK_FAKE_STATE".to_string(),
            sandbox.dir.to_string_lossy().into_owned(),
        ),
        (
            "CLAWDESK_FAKE_CAPTURE".to_string(),
            sandbox.capture.to_string_lossy().into_owned(),
        ),
    ];
    if let Some(behavior) = behavior {
        pairs.push(("CLAWDESK_FAKE_BEHAVIOR".to_string(), behavior.to_string()));
    }
    let refs: Vec<(&str, &str)> = pairs
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let _guard = GlobalEnvGuard::set(&refs);
    run(&sandbox);
}

/// OpenClaw port fake: detection resolves to the fake CLI binary.
struct DetectsFakeCli;

impl OpenClawPort for DetectsFakeCli {
    fn detect_executable(&self) -> ExecutableDetection {
        ExecutableDetection::Found {
            path: PathBuf::from(FAKE_OPENCLAW),
        }
    }
    fn version(&self, _exe: &Path) -> Result<OpenClawVersion, AppError> {
        unimplemented!()
    }
    fn version_from_entry(&self, _node: &Path, _entry: &Path) -> Result<OpenClawVersion, AppError> {
        unimplemented!()
    }
    fn gateway_status(&self, _exe: &Path) -> Result<GatewayStatus, AppError> {
        unimplemented!()
    }
    fn update_state(&self, _exe: &Path) -> Result<UpdateState, AppError> {
        unimplemented!()
    }
}

/// OpenClaw port fake: no executable (the stable not-found path).
struct NoOpenClaw;

impl OpenClawPort for NoOpenClaw {
    fn detect_executable(&self) -> ExecutableDetection {
        ExecutableDetection::NotFound
    }
    fn version(&self, _exe: &Path) -> Result<OpenClawVersion, AppError> {
        unimplemented!()
    }
    fn version_from_entry(&self, _node: &Path, _entry: &Path) -> Result<OpenClawVersion, AppError> {
        unimplemented!()
    }
    fn gateway_status(&self, _exe: &Path) -> Result<GatewayStatus, AppError> {
        unimplemented!()
    }
    fn update_state(&self, _exe: &Path) -> Result<UpdateState, AppError> {
        unimplemented!()
    }
}

fn automations_service() -> AutomationService {
    let process: Arc<dyn ProcessPort> = Arc::new(ProcessRunner);
    AutomationService::new(
        Arc::new(DetectsFakeCli),
        Arc::new(OpenClawAutomationsAdapter::new(process)),
    )
}

fn automations_service_no_openclaw() -> AutomationService {
    let process: Arc<dyn ProcessPort> = Arc::new(ProcessRunner);
    AutomationService::new(
        Arc::new(NoOpenClaw),
        Arc::new(OpenClawAutomationsAdapter::new(process)),
    )
}

// --- list / get ---------------------------------------------------------------------

#[test]
fn list_automations_over_fake_cli() {
    scenario("automations-list", None, |sandbox| {
        sandbox.seed(serde_json::json!({
            "automations": {
                "jobs": [
                    {
                        "id": "job-1",
                        "name": "알림",
                        "enabled": true,
                        "status": "ok",
                        "nextRunAtMs": 1798761600000u64,
                        "schedule": {"kind": "at", "at": "2099-01-01T00:00:00Z"},
                        "payload": {"kind": "reminder", "text": "약속", "wake": "now"}
                    },
                    {"id": "job-2", "status": "weird-future-status"},
                    {"name": "dropped — no id"}
                ]
            }
        }));
        let service = automations_service();
        let rows = service.list_automations().expect("list");
        assert_eq!(rows.len(), 2, "id-less rows dropped");
        let first = &rows[0];
        assert_eq!(first.id, "job-1");
        assert_eq!(first.name.as_deref(), Some("알림"));
        assert_eq!(first.enabled, Some(true));
        assert_eq!(first.status.as_deref(), Some("ok"));
        assert_eq!(first.next_run_at_ms, Some(1798761600000));
        assert_eq!(first.schedule.as_ref().expect("schedule").kind, "at");
        assert_eq!(first.payload.as_ref().expect("payload").kind, "reminder");
        assert_eq!(
            first.payload.as_ref().expect("payload").text.as_deref(),
            Some("약속")
        );
        let second = &rows[1];
        assert_eq!(second.id, "job-2");
        assert_eq!(second.status.as_deref(), Some("weird-future-status"));
        assert_eq!(second.name, None, "fail-soft: absent fields are null");
        assert_eq!(second.enabled, None);
        assert_eq!(second.schedule, None);
        assert_eq!(second.payload, None);
        let lines = sandbox.captured();
        assert_eq!(lines, vec![vec!["automations", "list", "--all", "--json"]]);
        assert_no_non_goal_flags(&lines);
    });
}

#[test]
fn get_automation_over_fake_cli() {
    scenario("automations-get", None, |sandbox| {
        sandbox.seed(serde_json::json!({
            "automations": {
                "jobs": [{
                    "id": "job-1",
                    "name": "주말 보고",
                    "enabled": false,
                    "status": "ok",
                    "schedule": {"kind": "cron", "cron": "0 9 * * 1", "tz": "Asia/Seoul"},
                    "payload": {"kind": "task", "text": "보고서 작성"}
                }]
            }
        }));
        let service = automations_service();
        let job = service.get_automation("job-1").expect("get");
        assert_eq!(job.id, "job-1");
        assert_eq!(job.name.as_deref(), Some("주말 보고"));
        assert_eq!(job.enabled, Some(false));
        assert_eq!(job.status.as_deref(), Some("ok"));
        let schedule = job.schedule.expect("schedule");
        assert_eq!(schedule.kind, "cron");
        assert_eq!(schedule.value.as_deref(), Some("0 9 * * 1"));
        assert_eq!(schedule.tz.as_deref(), Some("Asia/Seoul"));
        let payload = job.payload.expect("payload");
        assert_eq!(payload.kind, "task");
        assert_eq!(payload.text.as_deref(), Some("보고서 작성"));
        let lines = sandbox.captured();
        assert_eq!(lines, vec![vec!["automations", "get", "job-1", "--json"]]);
        assert_no_non_goal_flags(&lines);
    });
}

// --- create (fixed pairing, single-argv user text) -----------------------------------

#[test]
fn create_reminder_over_fake_cli() {
    scenario("automations-create-reminder", None, |sandbox| {
        sandbox.seed(serde_json::json!({}));
        let service = automations_service();
        let job_id = service
            .create_automation(
                "한글 이름",
                "at",
                "2027-02-01T16:00:00Z",
                None,
                "reminder",
                "따옴표 \"와\" 공백, text",
                None,
            )
            .expect("create");
        assert_eq!(job_id, "job-1");
        let lines = sandbox.captured();
        assert_eq!(
            lines,
            vec![vec![
                "automations",
                "add",
                "--name",
                "한글 이름",
                "--at",
                "2027-02-01T16:00:00Z",
                "--session",
                "main",
                "--system-event",
                "따옴표 \"와\" 공백, text",
                "--wake",
                "now",
                "--json",
            ]]
        );
        // S2: the name/text are single argv elements — the fake stores them
        // byte-for-byte, so the parsed state carries the exact Unicode.
        let state_body = sandbox.state_body();
        assert!(state_body.contains("한글 이름"), "{state_body}");
        let state = sandbox.state();
        let job = &state["automations"]["jobs"][0];
        assert_eq!(job["id"], "job-1");
        assert_eq!(job["enabled"], true);
        assert_eq!(job["name"].as_str(), Some("한글 이름"));
        assert_eq!(job["schedule"]["kind"], "at");
        assert_eq!(job["schedule"]["at"], "2027-02-01T16:00:00Z");
        assert_eq!(job["payload"]["kind"], "reminder");
        assert_eq!(job["payload"]["wake"], "now");
        assert_eq!(
            job["payload"]["text"].as_str(),
            Some("따옴표 \"와\" 공백, text"),
            "text must round-trip byte-for-byte"
        );
        assert_no_non_goal_flags(&lines);
    });
}

#[test]
fn create_task_over_fake_cli() {
    scenario("automations-create-task", None, |sandbox| {
        sandbox.seed(serde_json::json!({}));
        let service = automations_service();
        let job_id = service
            .create_automation(
                "task job",
                "cron",
                "0 9 * * 1",
                Some("Asia/Seoul"),
                "task",
                "보고서 작성",
                None,
            )
            .expect("create");
        assert_eq!(job_id, "job-1");
        let lines = sandbox.captured();
        assert_eq!(
            lines,
            vec![vec![
                "automations",
                "add",
                "--name",
                "task job",
                "--cron",
                "0 9 * * 1",
                "--tz",
                "Asia/Seoul",
                "--session",
                "isolated",
                "--message",
                "보고서 작성",
                "--json",
            ]]
        );
        // The fixed pairing: a task never emits `--wake`/`--system-event`.
        assert!(!lines[0].iter().any(|arg| arg == "--wake"));
        assert!(!lines[0].iter().any(|arg| arg == "--system-event"));
        let state = sandbox.state();
        let job = &state["automations"]["jobs"][0];
        assert_eq!(job["schedule"]["tz"], "Asia/Seoul");
        assert_eq!(job["payload"]["kind"], "task");
        assert!(job["payload"].get("wake").is_none(), "no wake for tasks");
        assert_no_non_goal_flags(&lines);
    });
}

// --- update / enable-disable / delete --------------------------------------------------

#[test]
fn update_automation_over_fake_cli() {
    scenario("automations-update", None, |sandbox| {
        sandbox.seed(serde_json::json!({
            "automations": {
                "jobs": [{
                    "id": "job-1",
                    "name": "old",
                    "enabled": true,
                    "schedule": {"kind": "at", "at": "2027-01-01T00:00:00Z"},
                    "payload": {"kind": "reminder", "text": "old text", "wake": "now"}
                }]
            }
        }));
        let service = automations_service();
        service
            .update_automation(
                "job-1",
                "새 이름",
                "every",
                "30m",
                None,
                "reminder",
                "새 내용",
                Some("next-heartbeat"),
            )
            .expect("update");
        let lines = sandbox.captured();
        assert_eq!(
            lines,
            vec![vec![
                "automations",
                "edit",
                "job-1",
                "--name",
                "새 이름",
                "--every",
                "30m",
                "--system-event",
                "새 내용",
                "--wake",
                "next-heartbeat",
                "--json",
            ]]
        );
        // Edit carries no `--session` (contract §4).
        assert!(!lines[0].iter().any(|arg| arg == "--session"));
        let state = sandbox.state();
        let job = &state["automations"]["jobs"][0];
        assert_eq!(job["name"], "새 이름");
        assert_eq!(job["schedule"]["kind"], "every");
        assert_eq!(job["schedule"]["every"], "30m");
        assert_eq!(job["payload"]["text"], "새 내용");
        assert_eq!(job["payload"]["wake"], "next-heartbeat");
        assert_no_non_goal_flags(&lines);
    });
}

#[test]
fn enable_disable_over_fake_cli() {
    scenario("automations-toggle", None, |sandbox| {
        sandbox.seed(serde_json::json!({
            "automations": {
                "jobs": [{"id": "job-1", "name": "n", "enabled": false}]
            }
        }));
        let service = automations_service();
        service
            .set_automation_enabled("job-1", true)
            .expect("enable");
        assert_eq!(sandbox.state()["automations"]["jobs"][0]["enabled"], true);
        service
            .set_automation_enabled("job-1", false)
            .expect("disable");
        assert_eq!(sandbox.state()["automations"]["jobs"][0]["enabled"], false);
        let lines = sandbox.captured();
        assert_eq!(
            lines,
            vec![
                vec!["automations", "enable", "job-1", "--json"],
                vec!["automations", "disable", "job-1", "--json"],
            ]
        );
        assert_no_non_goal_flags(&lines);
    });
}

#[test]
fn delete_automation_over_fake_cli() {
    scenario("automations-delete", None, |sandbox| {
        sandbox.seed(serde_json::json!({
            "automations": {
                "jobs": [
                    {"id": "job-1", "name": "n1"},
                    {"id": "job-2", "name": "n2"}
                ]
            }
        }));
        let service = automations_service();
        service.remove_automation("job-1").expect("delete");
        let lines = sandbox.captured();
        assert_eq!(
            lines,
            vec![vec!["automations", "remove", "job-1", "--json"]]
        );
        let state = sandbox.state();
        let jobs = state["automations"]["jobs"].as_array().expect("jobs array");
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0]["id"], "job-2");
        assert_no_non_goal_flags(&lines);
    });
}

// --- fail-closed validation (0 CLI calls) ------------------------------------------------

#[test]
fn validation_fail_closed_zero_cli() {
    scenario("automations-validation", None, |sandbox| {
        sandbox.seed(serde_json::json!({}));
        let service = automations_service();

        // Id shape (get / update / toggle / delete share the same gate).
        for bad in ["", "bad id", "id/", &"x".repeat(65)] {
            assert_eq!(
                service.get_automation(bad).unwrap_err().code,
                "automation-id-invalid",
                "get {bad:?}"
            );
            assert_eq!(
                service.set_automation_enabled(bad, true).unwrap_err().code,
                "automation-id-invalid",
                "toggle {bad:?}"
            );
            assert_eq!(
                service.remove_automation(bad).unwrap_err().code,
                "automation-id-invalid",
                "delete {bad:?}"
            );
        }

        // Name / schedule / payload (create + update share the field gate).
        #[allow(clippy::type_complexity)] // positional case table; fields destructured below
        let create_cases: Vec<(
            &str,
            &str,
            &str,
            Option<&str>,
            &str,
            &str,
            Option<&str>,
            &str,
        )> = vec![
            (
                "",
                "at",
                "2027-02-01T16:00:00Z",
                None,
                "reminder",
                "t",
                None,
                "name empty",
            ),
            (
                "   ",
                "at",
                "2027-02-01T16:00:00Z",
                None,
                "reminder",
                "t",
                None,
                "name blank",
            ),
            (
                "a\u{01}b",
                "at",
                "2027-02-01T16:00:00Z",
                None,
                "reminder",
                "t",
                None,
                "name control",
            ),
            (
                "n",
                "at",
                "2027-02-01T16:00:00",
                None,
                "reminder",
                "t",
                None,
                "at offset-less",
            ),
            (
                "n",
                "at",
                "2027-02-01T16:00:00Z",
                Some("Asia/Seoul"),
                "reminder",
                "t",
                None,
                "at + tz",
            ),
            (
                "n",
                "every",
                "0m",
                None,
                "reminder",
                "t",
                None,
                "every zero",
            ),
            (
                "n",
                "every",
                "1.5h",
                None,
                "reminder",
                "t",
                None,
                "every fractional",
            ),
            (
                "n",
                "cron",
                "0 9 * *",
                None,
                "reminder",
                "t",
                None,
                "cron 4 fields",
            ),
            (
                "n",
                "cron",
                "a 9 * * *",
                None,
                "reminder",
                "t",
                None,
                "cron bad char",
            ),
            (
                "n",
                "stream",
                "x",
                None,
                "reminder",
                "t",
                None,
                "non-goal kind",
            ),
            (
                "n",
                "at",
                "2027-02-01T16:00:00Z",
                None,
                "reminder",
                "   ",
                None,
                "text blank",
            ),
            (
                "n",
                "at",
                "2027-02-01T16:00:00Z",
                None,
                "command",
                "ls",
                None,
                "command payload",
            ),
            (
                "n",
                "at",
                "2027-02-01T16:00:00Z",
                None,
                "task",
                "t",
                Some("now"),
                "task + wake",
            ),
            (
                "n",
                "at",
                "2027-02-01T16:00:00Z",
                None,
                "reminder",
                "t",
                Some("later"),
                "invalid wake",
            ),
        ];
        for (name, kind, value, tz, pkind, text, wake, label) in &create_cases {
            let expected = if label.starts_with("name") {
                "automation-name-invalid"
            } else if label.starts_with("at")
                || label.starts_with("every")
                || label.starts_with("cron")
                || label.starts_with("non-goal")
            {
                "automation-schedule-invalid"
            } else {
                "automation-payload-invalid"
            };
            let err = service
                .create_automation(name, kind, value, *tz, pkind, text, *wake)
                .unwrap_err();
            assert_eq!(err.code, expected, "{label}");
            let err = service
                .update_automation("job-1", name, kind, value, *tz, pkind, text, *wake)
                .unwrap_err();
            assert_eq!(err.code, expected, "update {label}");
        }

        assert!(!sandbox.capture.exists(), "no CLI call may be captured");
        let state = sandbox.state();
        assert!(
            state.get("automations").is_none(),
            "state must be unchanged"
        );
    });
}

// --- unknown job / adapter failures / masking --------------------------------------------

#[test]
fn unknown_job_is_failed() {
    scenario("automations-unknown-job", None, |sandbox| {
        sandbox.seed(serde_json::json!({}));
        let service = automations_service();
        let err = service.get_automation("job-404").expect_err("unknown get");
        assert_eq!(err.code, "openclaw-automations-failed");
        assert!(
            err.message.contains("unknown job: job-404"),
            "{}",
            err.message
        );
        let err = service
            .update_automation("job-404", "n", "every", "10m", None, "reminder", "t", None)
            .expect_err("unknown edit");
        assert_eq!(err.code, "openclaw-automations-failed");
        let err = service
            .set_automation_enabled("job-404", true)
            .expect_err("unknown enable");
        assert_eq!(err.code, "openclaw-automations-failed");
        let err = service
            .remove_automation("job-404")
            .expect_err("unknown remove");
        assert_eq!(err.code, "openclaw-automations-failed");
        let lines = sandbox.captured();
        assert_eq!(
            lines,
            vec![
                vec!["automations", "get", "job-404", "--json"],
                vec![
                    "automations",
                    "edit",
                    "job-404",
                    "--name",
                    "n",
                    "--every",
                    "10m",
                    "--system-event",
                    "t",
                    "--wake",
                    "now",
                    "--json"
                ],
                vec!["automations", "enable", "job-404", "--json"],
                vec!["automations", "remove", "job-404", "--json"],
            ]
        );
        let state = sandbox.state();
        assert!(
            state.get("automations").is_none(),
            "a failed mutation must not mutate state"
        );
    });
}

#[test]
fn adapter_failures_over_fake_cli() {
    // a) cli-error envelope on every call.
    scenario(
        "automations-adapter-cli-error",
        Some("cli-error"),
        |sandbox| {
            sandbox.seed(serde_json::json!({}));
            let exe = Path::new(FAKE_OPENCLAW);
            let adapter = OpenClawAutomationsAdapter::new(Arc::new(ProcessRunner));
            assert_eq!(
                adapter.list_automations(exe).unwrap_err().code,
                "openclaw-automations-failed"
            );
            assert_eq!(
                adapter.get_automation(exe, "job-1").unwrap_err().code,
                "openclaw-automations-failed"
            );
            let err = adapter
                .add_automation(exe, "n", "every", "10m", None, "reminder", "t", None)
                .unwrap_err();
            assert_eq!(err.code, "openclaw-automations-failed");
            assert!(
                sandbox.state().get("automations").is_none(),
                "a failed add must not mutate state"
            );
        },
    );

    // b) Malformed / not-json output → the stable parse-failure code.
    scenario(
        "automations-adapter-malformed",
        Some("malformed"),
        |sandbox| {
            sandbox.seed(serde_json::json!({}));
            let adapter = OpenClawAutomationsAdapter::new(Arc::new(ProcessRunner));
            assert_eq!(
                adapter
                    .list_automations(Path::new(FAKE_OPENCLAW))
                    .unwrap_err()
                    .code,
                "openclaw-automations-failed"
            );
        },
    );
    scenario(
        "automations-adapter-not-json",
        Some("not-json"),
        |sandbox| {
            sandbox.seed(serde_json::json!({}));
            let adapter = OpenClawAutomationsAdapter::new(Arc::new(ProcessRunner));
            assert_eq!(
                adapter
                    .get_automation(Path::new(FAKE_OPENCLAW), "job-1")
                    .unwrap_err()
                    .code,
                "openclaw-automations-failed"
            );
        },
    );

    // c) Failing CLI: the error message stays masked (S3/S8).
    scenario("automations-adapter-fail-masked", Some("fail"), |sandbox| {
        sandbox.seed(serde_json::json!({}));
        let adapter = OpenClawAutomationsAdapter::new(Arc::new(ProcessRunner));
        let err = adapter
            .list_automations(Path::new(FAKE_OPENCLAW))
            .unwrap_err();
        assert_eq!(err.code, "openclaw-automations-failed");
        assert!(
            !err.message.contains("sk-fake123456789"),
            "secret must not leak into the error: {}",
            err.message
        );
        assert!(err.message.contains("sk-****"), "stderr should be masked");
    });
}

// --- missing executable ---------------------------------------------------------------------

#[test]
fn automations_missing_executable() {
    // Service level: detection failure → the reused Phase 1 code, 0 CLI calls.
    let service = automations_service_no_openclaw();
    let failures = [
        service.list_automations().unwrap_err(),
        service.get_automation("job-1").unwrap_err(),
        service
            .create_automation("n", "every", "10m", None, "reminder", "t", None)
            .unwrap_err(),
        service
            .update_automation("job-1", "n", "every", "10m", None, "reminder", "t", None)
            .unwrap_err(),
        service.set_automation_enabled("job-1", true).unwrap_err(),
        service.remove_automation("job-1").unwrap_err(),
    ];
    for err in failures {
        assert_eq!(err.code, "openclaw-not-found");
    }
}

// --- timeout -----------------------------------------------------------------------------------

#[test]
fn automations_list_slow_process_times_out() {
    let sandbox = Sandbox::new("fake-automations-timeout");
    sandbox.seed(serde_json::json!({}));
    let argv = vec![
        "automations".to_string(),
        "list".to_string(),
        "--all".to_string(),
        "--json".to_string(),
    ];
    let mut request = ProcessRequest::new(FAKE_OPENCLAW, argv, Duration::from_millis(400));
    request.env.push((
        "CLAWDESK_FAKE_STATE".to_string(),
        sandbox.dir.to_string_lossy().into_owned(),
    ));
    request
        .env
        .push(("CLAWDESK_FAKE_BEHAVIOR".to_string(), "sleep".to_string()));
    let err = ProcessRunner
        .run(&request)
        .expect_err("sleeping fake CLI must time out");
    match err {
        ProcessError::Timeout { executable } => {
            assert!(executable.contains("clawdesk-fake-openclaw"));
        }
        other => panic!("expected Timeout, got {other:?}"),
    }
}

// --- fake-level behavior (per-request env; parallel-safe) -------------------------------------

#[test]
fn fake_rejects_non_goal_flags() {
    let sandbox = Sandbox::new("fake-automations-non-goal");
    sandbox.seed(serde_json::json!({}));
    for (argv, label) in [
        (
            vec![
                "automations",
                "add",
                "--name",
                "n",
                "--at",
                "2027-02-01T16:00:00Z",
                "--session",
                "main",
                "--system-event",
                "t",
                "--wake",
                "now",
                "--command",
                "echo hi",
            ],
            "add --command",
        ),
        (
            vec![
                "automations",
                "add",
                "--name",
                "n",
                "--every",
                "10m",
                "--session",
                "isolated",
                "--message",
                "t",
                "--webhook",
                "https://x",
            ],
            "add --webhook",
        ),
        (
            vec![
                "automations",
                "edit",
                "job-1",
                "--name",
                "n",
                "--every",
                "10m",
                "--system-event",
                "t",
                "--model",
                "gpt-x",
            ],
            "edit --model",
        ),
        (
            vec![
                "automations",
                "add",
                "--name",
                "n",
                "--at",
                "2027-02-01T16:00:00Z",
                "--session",
                "main",
                "--system-event",
                "t",
                "--to",
                "+123",
            ],
            "add --to",
        ),
    ] {
        let output = run_fake(&argv, &sandbox, &[]).expect("invocation should run");
        assert_eq!(output.exit_code, 2, "{label}");
        assert!(
            output.stderr.contains("non-goal flag"),
            "{label}: {:?}",
            output.stderr
        );
    }
    let state = sandbox.state();
    assert!(
        state.get("automations").is_none(),
        "rejected adds must not mutate state"
    );
}

#[test]
fn fake_run_and_runs_are_unsupported() {
    // Non-goal regression gate: manual execution has no fake handler and
    // falls through to the unsupported exit 2.
    let sandbox = Sandbox::new("fake-automations-run");
    sandbox.seed(serde_json::json!({
        "automations": {"jobs": [{"id": "job-1", "name": "n"}]}
    }));
    for argv in [
        vec!["automations", "run", "job-1"],
        vec!["automations", "runs"],
        vec!["automations", "runs", "job-1"],
    ] {
        let output = run_fake(&argv, &sandbox, &[]).expect("invocation should run");
        assert_eq!(output.exit_code, 2, "{argv:?}");
        assert!(
            output.stderr.contains("unsupported command"),
            "{argv:?}: {:?}",
            output.stderr
        );
    }
}

#[test]
fn fake_add_enforces_the_fixed_pairing() {
    let sandbox = Sandbox::new("fake-automations-pairing");
    sandbox.seed(serde_json::json!({}));
    let cases: Vec<(Vec<&str>, &str, &str)> = vec![
        (
            vec![
                "automations",
                "add",
                "--name",
                "n",
                "--at",
                "2027-02-01T16:00:00Z",
                "--every",
                "10m",
                "--session",
                "main",
                "--system-event",
                "t",
                "--wake",
                "now",
            ],
            "exactly one of --at/--every/--cron",
            "two schedules",
        ),
        (
            vec![
                "automations",
                "add",
                "--name",
                "n",
                "--at",
                "2027-02-01T16:00:00Z",
                "--tz",
                "Asia/Seoul",
                "--session",
                "main",
                "--system-event",
                "t",
                "--wake",
                "now",
            ],
            "--tz is cron-only",
            "at + tz",
        ),
        (
            vec![
                "automations",
                "add",
                "--name",
                "n",
                "--at",
                "2027-02-01T16:00:00Z",
                "--session",
                "isolated",
                "--system-event",
                "t",
                "--wake",
                "now",
            ],
            "not allowed for a reminder job",
            "reminder in isolated",
        ),
        (
            vec![
                "automations",
                "add",
                "--name",
                "n",
                "--at",
                "2027-02-01T16:00:00Z",
                "--session",
                "main",
                "--message",
                "t",
            ],
            "not allowed for a task job",
            "task in main",
        ),
        (
            vec![
                "automations",
                "add",
                "--name",
                "n",
                "--at",
                "2027-02-01T16:00:00Z",
                "--session",
                "isolated",
                "--message",
                "t",
                "--wake",
                "now",
            ],
            "--wake is reminder-only",
            "task + wake",
        ),
    ];
    for (argv, expected, label) in &cases {
        let output = run_fake(argv, &sandbox, &[]).expect("invocation should run");
        assert_eq!(output.exit_code, 1, "{label}");
        assert!(
            output.stdout.contains(expected),
            "{label}: stdout={:?}",
            output.stdout
        );
    }
    let state = sandbox.state();
    assert!(
        state.get("automations").is_none(),
        "state must be unchanged"
    );
}

// --- fake-level runner ---------------------------------------------------------------------

fn run_fake(
    argv: &[&str],
    sandbox: &Sandbox,
    extra_envs: &[(&str, &str)],
) -> Result<ProcessOutput, ProcessError> {
    let argv: Vec<String> = argv.iter().map(|arg| arg.to_string()).collect();
    let mut request = ProcessRequest::new(FAKE_OPENCLAW, argv, TIMEOUT);
    request.env.push((
        "CLAWDESK_FAKE_STATE".to_string(),
        sandbox.dir.to_string_lossy().into_owned(),
    ));
    request.env.push((
        "CLAWDESK_FAKE_CAPTURE".to_string(),
        sandbox.capture.to_string_lossy().into_owned(),
    ));
    // Fake-level tests run in parallel with the serialized scenario tests,
    // which set `CLAWDESK_FAKE_BEHAVIOR` as inherited global env. Pin the
    // behavior per request (default: normal) so a scenario's override can
    // never leak into a fake-level invocation.
    let behavior_pinned = extra_envs
        .iter()
        .any(|(key, _)| *key == "CLAWDESK_FAKE_BEHAVIOR");
    if !behavior_pinned {
        request
            .env
            .push(("CLAWDESK_FAKE_BEHAVIOR".to_string(), "normal".to_string()));
    }
    for (key, value) in extra_envs {
        request.env.push((key.to_string(), value.to_string()));
    }
    ProcessRunner.run(&request)
}
