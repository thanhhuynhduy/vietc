// SPDX-License-Identifier: MIT

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::process::ExitStatusExt;

pub const CRASH_EVIDENCE_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessClassification {
    DaemonCrash,
    RendererCrash,
    BrowserProcessCrash,
    ProtocolTriggeredExit,
    NoCrashObserved,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReproductionIdentity {
    pub reproduction_id: String,
    pub transaction_id: String,
    pub visible_revision: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MinimizedSequence {
    pub format_version: u32,
    pub physical_keys: Vec<String>,
    pub inter_key_delay_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessEvidence {
    pub pid: u32,
    pub parent_pid: Option<u32>,
    pub process_type: String,
    pub command_line: Vec<String>,
    pub exit_status: Option<i32>,
    pub signal: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoundedCommandOutput {
    pub pid: u32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_status: Option<i32>,
    pub signal: Option<i32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RedactedMetadata {
    pub serialized_metadata: String,
    pub redacted_fields: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct EvidenceCollectionRequest {
    pub bundle_dir: PathBuf,
    pub identity: ReproductionIdentity,
    pub application_name: String,
    pub application_command: Vec<String>,
    pub daemon_command: Vec<String>,
    pub backend: String,
    pub config_summary: Vec<String>,
    pub minimized_sequence: MinimizedSequence,
    pub uncontrolled_input: Option<String>,
    pub command_timeout: Duration,
}

#[derive(Clone, Debug)]
pub struct SystemEvidenceTools {
    pub journalctl: PathBuf,
    pub coredumpctl: PathBuf,
}

impl Default for SystemEvidenceTools {
    fn default() -> Self {
        Self {
            journalctl: PathBuf::from("journalctl"),
            coredumpctl: PathBuf::from("coredumpctl"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvidenceCollectionResult {
    pub bundle_dir: PathBuf,
    pub classification: ProcessClassification,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrashEvidence {
    pub schema_version: u32,
    pub identity: ReproductionIdentity,
    pub classification: ProcessClassification,
    pub utc_started_at: String,
    pub utc_finished_at: String,
    pub daemon_stderr_path: Option<String>,
    pub application_stderr_path: Option<String>,
    pub journal_path: Option<String>,
    pub coredump_list_path: Option<String>,
    pub coredump_info_path: Option<String>,
    pub core_debug_workflow_path: Option<String>,
    pub process_tree_path: Option<String>,
    pub browser_crash_reporting: String,
    pub application_name: String,
    pub application_version: String,
    pub application_command: Vec<String>,
    pub distro: String,
    pub kernel: String,
    pub session_type: String,
    pub compositor: String,
    pub backend: String,
    pub config_summary: Vec<String>,
    pub processes: Vec<ProcessEvidence>,
    pub minimized_sequence: MinimizedSequence,
    pub redacted_fields: Vec<String>,
}

impl CrashEvidence {
    pub fn write_json(&self, path: &Path) -> std::io::Result<()> {
        let payload = serde_json::to_vec_pretty(self)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
        std::fs::write(path, payload)
    }

    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema_version != CRASH_EVIDENCE_SCHEMA_VERSION {
            return Err("unsupported schema version");
        }
        if self.identity.reproduction_id.is_empty() {
            return Err("reproduction_id is required");
        }
        if self.identity.transaction_id.is_empty() {
            return Err("transaction_id is required");
        }
        if self.utc_started_at.is_empty() || self.utc_finished_at.is_empty() {
            return Err("reproduction timestamps are required");
        }
        if self.minimized_sequence.format_version != 1 {
            return Err("unsupported minimized sequence format");
        }
        if self.minimized_sequence.physical_keys.is_empty() {
            return Err("at least one controlled physical key is required");
        }
        Ok(())
    }
}

pub fn run_bounded_command(
    program: &str,
    args: &[&str],
    timeout: Duration,
) -> std::io::Result<BoundedCommandOutput> {
    let mut child = Command::new(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let pid = child.id();

    let deadline = Instant::now() + timeout;
    loop {
        if child.try_wait()?.is_some() {
            let output = child.wait_with_output()?;
            return Ok(BoundedCommandOutput {
                pid,
                stdout: output.stdout,
                stderr: output.stderr,
                exit_status: output.status.code(),
                #[cfg(unix)]
                signal: output.status.signal(),
                #[cfg(not(unix))]
                signal: None,
            });
        }

        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!(
                    "collector command timed out after {} ms",
                    timeout.as_millis()
                ),
            ));
        }

        std::thread::sleep(Duration::from_millis(10));
    }
}

pub fn redact_uncontrolled_input(
    _uncontrolled_input: &str,
    controlled_sequence: &MinimizedSequence,
    application_command: &[String],
) -> RedactedMetadata {
    let redacted_command = application_command
        .iter()
        .map(|argument| {
            if argument.starts_with("--user-data-dir=") {
                "--user-data-dir=<redacted>".to_string()
            } else {
                argument.clone()
            }
        })
        .collect::<Vec<_>>();

    let serialized_metadata = serde_json::json!({
        "application_command": redacted_command,
        "minimized_sequence": controlled_sequence,
    })
    .to_string();

    RedactedMetadata {
        serialized_metadata,
        redacted_fields: vec![
            "application_command.user_data_dir".to_string(),
            "uncontrolled_user_input".to_string(),
        ],
    }
}

pub fn collect_evidence_bundle(
    request: &EvidenceCollectionRequest,
) -> std::io::Result<EvidenceCollectionResult> {
    collect_evidence_bundle_with_tools(request, &SystemEvidenceTools::default())
}

pub fn collect_evidence_bundle_with_tools(
    request: &EvidenceCollectionRequest,
    tools: &SystemEvidenceTools,
) -> std::io::Result<EvidenceCollectionResult> {
    request
        .minimized_sequence
        .physical_keys
        .first()
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "controlled physical-key sequence is required",
            )
        })?;
    std::fs::create_dir_all(&request.bundle_dir)?;

    let started_at = utc_timestamp(request.command_timeout);
    let daemon_output = run_requested_command(&request.daemon_command, request.command_timeout)?;
    write_command_artifacts(&request.bundle_dir, "daemon", &daemon_output)?;

    let application_output =
        run_requested_command(&request.application_command, request.command_timeout)?;
    write_command_artifacts(&request.bundle_dir, "application", &application_output)?;

    let journal_path = collect_optional_command(
        &request.bundle_dir,
        &tools.journalctl,
        &["--user", "--since", &started_at],
        "journal",
        request.command_timeout,
    )?;
    let coredump_list_path = collect_optional_command(
        &request.bundle_dir,
        &tools.coredumpctl,
        &["list", "--no-pager"],
        "coredump-list",
        request.command_timeout,
    )?;
    let coredump_info_path = collect_optional_command(
        &request.bundle_dir,
        &tools.coredumpctl,
        &["info", "--no-pager"],
        "coredump-info",
        request.command_timeout,
    )?;

    let process_tree_path = collect_optional_command(
        &request.bundle_dir,
        Path::new("ps"),
        &["-eo", "pid=,ppid=,comm=,args="],
        "process-tree",
        request.command_timeout,
    )?;
    let core_debug_workflow_path = write_core_debug_workflow(&request.bundle_dir)?;
    let application_version = collect_version(
        request.application_command.first().map(String::as_str),
        request.command_timeout,
    );
    let redacted = redact_uncontrolled_input(
        request.uncontrolled_input.as_deref().unwrap_or_default(),
        &request.minimized_sequence,
        &request.application_command,
    );
    let redacted_command = serde_json::from_str::<serde_json::Value>(&redacted.serialized_metadata)
        .ok()
        .and_then(|metadata| metadata.get("application_command").cloned())
        .and_then(|command| serde_json::from_value(command).ok())
        .unwrap_or_default();

    let evidence = CrashEvidence {
        schema_version: CRASH_EVIDENCE_SCHEMA_VERSION,
        identity: request.identity.clone(),
        classification: ProcessClassification::Unknown,
        utc_started_at: started_at,
        utc_finished_at: utc_timestamp(request.command_timeout),
        daemon_stderr_path: Some("daemon.stderr.log".to_string()),
        application_stderr_path: Some("application.stderr.log".to_string()),
        journal_path: Some(journal_path),
        coredump_list_path: Some(coredump_list_path),
        coredump_info_path: Some(coredump_info_path),
        core_debug_workflow_path: Some(core_debug_workflow_path),
        process_tree_path: Some(process_tree_path),
        browser_crash_reporting: detect_browser_crash_reporting(&request.application_command),
        application_name: request.application_name.clone(),
        application_version,
        application_command: redacted_command,
        distro: read_distro(),
        kernel: collect_single_line("uname", &["-sr"], request.command_timeout),
        session_type: environment_or_unknown("XDG_SESSION_TYPE"),
        compositor: environment_or_unknown("XDG_CURRENT_DESKTOP"),
        backend: request.backend.clone(),
        config_summary: request.config_summary.clone(),
        processes: vec![
            ProcessEvidence {
                pid: std::process::id(),
                parent_pid: parent_pid(std::process::id()),
                process_type: "collector".to_string(),
                command_line: std::env::args().collect(),
                exit_status: Some(0),
                signal: None,
            },
            ProcessEvidence {
                pid: daemon_output.pid,
                parent_pid: Some(std::process::id()),
                process_type: "daemon_command".to_string(),
                command_line: redact_command(&request.daemon_command),
                exit_status: daemon_output.exit_status,
                signal: daemon_output.signal,
            },
            ProcessEvidence {
                pid: application_output.pid,
                parent_pid: Some(std::process::id()),
                process_type: application_process_type(&request.application_command).to_string(),
                command_line: redact_command(&request.application_command),
                exit_status: application_output.exit_status,
                signal: application_output.signal,
            },
        ],
        minimized_sequence: request.minimized_sequence.clone(),
        redacted_fields: redacted.redacted_fields,
    };
    evidence
        .validate()
        .map_err(|message| std::io::Error::new(std::io::ErrorKind::InvalidData, message))?;
    evidence.write_json(&request.bundle_dir.join("evidence.json"))?;
    write_json(
        &request.bundle_dir.join("replay.json"),
        &request.minimized_sequence,
    )?;

    Ok(EvidenceCollectionResult {
        bundle_dir: request.bundle_dir.clone(),
        classification: ProcessClassification::Unknown,
    })
}

fn redact_command(command: &[String]) -> Vec<String> {
    command
        .iter()
        .map(|argument| {
            if argument.starts_with("--user-data-dir=") {
                "--user-data-dir=<redacted>".to_string()
            } else {
                argument.clone()
            }
        })
        .collect()
}

fn parent_pid(pid: u32) -> Option<u32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (_, fields) = stat.rsplit_once(')')?;
    fields.split_whitespace().nth(1)?.parse().ok()
}

fn application_process_type(command: &[String]) -> &'static str {
    if command
        .iter()
        .any(|argument| argument == "--type=renderer" || argument.starts_with("--type=renderer"))
    {
        "renderer"
    } else {
        "browser_process"
    }
}

fn detect_browser_crash_reporting(command: &[String]) -> String {
    if command
        .iter()
        .any(|argument| argument == "--disable-breakpad" || argument == "--disable-crash-reporter")
    {
        "DISABLED_BY_COMMAND_LINE".to_string()
    } else if command.first().is_some_and(|program| {
        let name = Path::new(program)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        name.contains("chromium") || name.contains("chrome") || name.contains("electron")
    }) {
        "AVAILABLE_APPLICATION_DEPENDENT".to_string()
    } else {
        "UNAVAILABLE_NOT_A_SUPPORTED_BROWSER".to_string()
    }
}

fn write_core_debug_workflow(bundle_dir: &Path) -> std::io::Result<String> {
    let file_name = "core-debug-workflow.txt";
    std::fs::write(
        bundle_dir.join(file_name),
        "Core debug workflow (run only when coredump-list/info identifies a related PID):\n\
         1. Match PID, executable, timestamp, and reproduction window.\n\
         2. Run: coredumpctl info PID --no-pager\n\
         3. Run interactively: coredumpctl debug PID\n\
         4. In gdb run: set pagination off; thread apply all bt full\n\
         5. Save symbols, executable version, stack trace, and matching evidence.json.\n\
         Never claim a root cause from an unrelated or missing core.\n",
    )?;
    Ok(file_name.to_string())
}

pub fn classify_process_outcome(
    daemon: &ProcessEvidence,
    application: &ProcessEvidence,
    related_core_process_types: &[String],
    protocol_error_observed: bool,
) -> ProcessClassification {
    let crashed = |process: &ProcessEvidence| {
        process.signal.is_some() || process.exit_status.is_some_and(|status| status != 0)
    };

    if crashed(daemon) {
        return ProcessClassification::DaemonCrash;
    }
    if crashed(application) {
        if related_core_process_types
            .iter()
            .any(|kind| kind == "renderer")
            || application.process_type == "renderer"
        {
            return ProcessClassification::RendererCrash;
        }
        if related_core_process_types
            .iter()
            .any(|kind| kind == "browser_process")
            || application.process_type == "browser_process"
        {
            return ProcessClassification::BrowserProcessCrash;
        }
        if protocol_error_observed {
            return ProcessClassification::ProtocolTriggeredExit;
        }
    }
    if !crashed(daemon) && !crashed(application) {
        ProcessClassification::NoCrashObserved
    } else {
        ProcessClassification::Unknown
    }
}

fn run_requested_command(
    command: &[String],
    timeout: Duration,
) -> std::io::Result<BoundedCommandOutput> {
    let (program, args) = command.split_first().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "command is required")
    })?;
    let args = args.iter().map(String::as_str).collect::<Vec<_>>();
    run_bounded_command(program, &args, timeout)
}

fn write_command_artifacts(
    bundle_dir: &Path,
    prefix: &str,
    output: &BoundedCommandOutput,
) -> std::io::Result<()> {
    std::fs::write(
        bundle_dir.join(format!("{prefix}.stdout.log")),
        &output.stdout,
    )?;
    std::fs::write(
        bundle_dir.join(format!("{prefix}.stderr.log")),
        &output.stderr,
    )
}

fn collect_optional_command(
    bundle_dir: &Path,
    program: &Path,
    args: &[&str],
    artifact_name: &str,
    timeout: Duration,
) -> std::io::Result<String> {
    let program_name = program.to_string_lossy();
    match run_bounded_command(&program_name, args, timeout) {
        Ok(output) => {
            let file_name = format!("{artifact_name}.txt");
            let mut contents = output.stdout;
            if !output.stderr.is_empty() {
                contents.extend_from_slice(b"\n--- stderr ---\n");
                contents.extend_from_slice(&output.stderr);
            }
            std::fs::write(bundle_dir.join(&file_name), contents)?;
            Ok(file_name)
        }
        Err(error) => {
            let file_name = format!("{artifact_name}.unavailable.txt");
            std::fs::write(
                bundle_dir.join(&file_name),
                format!("unavailable: {error}\n"),
            )?;
            Ok(file_name)
        }
    }
}

fn collect_version(program: Option<&str>, timeout: Duration) -> String {
    program
        .map(|program| collect_single_line(program, &["--version"], timeout))
        .unwrap_or_else(|| "unknown".to_string())
}

fn collect_single_line(program: &str, args: &[&str], timeout: Duration) -> String {
    run_bounded_command(program, args, timeout)
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|output| output.lines().next().map(str::trim).map(str::to_string))
        .filter(|output| !output.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

fn read_distro() -> String {
    std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|contents| {
            contents
                .lines()
                .find_map(|line| line.strip_prefix("PRETTY_NAME="))
                .map(|value| value.trim_matches('"').to_string())
        })
        .unwrap_or_else(|| "unknown".to_string())
}

fn environment_or_unknown(name: &str) -> String {
    std::env::var(name)
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

fn utc_timestamp(timeout: Duration) -> String {
    collect_single_line(
        "date",
        &["-u", "+%Y-%m-%dT%H:%M:%SZ"],
        timeout.min(Duration::from_secs(1)),
    )
}

fn write_json(path: &Path, value: &impl Serialize) -> std::io::Result<()> {
    let payload = serde_json::to_vec_pretty(value)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    std::fs::write(path, payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn fixture() -> CrashEvidence {
        CrashEvidence {
            schema_version: CRASH_EVIDENCE_SCHEMA_VERSION,
            identity: ReproductionIdentity {
                reproduction_id: "repro-20261019-001".to_string(),
                transaction_id: "tx-0001".to_string(),
                visible_revision: 3,
            },
            classification: ProcessClassification::RendererCrash,
            utc_started_at: "2026-10-19T09:00:00Z".to_string(),
            utc_finished_at: "2026-10-19T09:00:03Z".to_string(),
            daemon_stderr_path: Some("daemon.stderr.log".to_string()),
            application_stderr_path: Some("chromium.stderr.log".to_string()),
            journal_path: Some("journal.log".to_string()),
            coredump_list_path: Some("coredump-list.txt".to_string()),
            coredump_info_path: Some("coredump-info.txt".to_string()),
            core_debug_workflow_path: Some("core-debug-workflow.txt".to_string()),
            process_tree_path: Some("process-tree.txt".to_string()),
            browser_crash_reporting: "AVAILABLE_APPLICATION_DEPENDENT".to_string(),
            application_name: "chromium".to_string(),
            application_version: "fixture-version".to_string(),
            application_command: vec![
                "chromium".to_string(),
                "--user-data-dir=<redacted>".to_string(),
            ],
            distro: "fixture-distro".to_string(),
            kernel: "fixture-kernel".to_string(),
            session_type: "wayland".to_string(),
            compositor: "fixture-compositor".to_string(),
            backend: "non-grabbed-evdev".to_string(),
            config_summary: vec![
                "auto_restore=true".to_string(),
                "input_method=telex".to_string(),
            ],
            processes: vec![ProcessEvidence {
                pid: 1234,
                parent_pid: Some(1000),
                process_type: "renderer".to_string(),
                command_line: vec!["chromium".to_string(), "--type=renderer".to_string()],
                exit_status: None,
                signal: Some(11),
            }],
            minimized_sequence: MinimizedSequence {
                format_version: 1,
                physical_keys: vec!["KEY_A:press".to_string(), "KEY_A:release".to_string()],
                inter_key_delay_ms: 20,
            },
            redacted_fields: vec![
                "application_command.user_data_dir".to_string(),
                "uncontrolled_user_input".to_string(),
            ],
        }
    }

    #[test]
    fn evidence_fixture_has_required_identity_classification_and_redaction() {
        let evidence = fixture();

        assert_eq!(evidence.validate(), Ok(()));
        assert_eq!(
            evidence.classification,
            ProcessClassification::RendererCrash
        );
        assert!(evidence
            .redacted_fields
            .contains(&"uncontrolled_user_input".to_string()));
    }

    #[test]
    fn evidence_json_round_trips_without_uncontrolled_input() {
        let evidence = fixture();
        let serialized = serde_json::to_string_pretty(&evidence).unwrap();
        let restored: CrashEvidence = serde_json::from_str(&serialized).unwrap();

        assert_eq!(restored, evidence);
        assert!(!serialized.contains("typed password"));
    }

    #[test]
    fn evidence_is_written_to_the_requested_bundle_path() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("evidence.json");
        let evidence = fixture();

        evidence.write_json(&path).unwrap();

        let restored: CrashEvidence =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(restored, evidence);
    }

    #[test]
    fn bounded_command_times_out_and_reaps_child() {
        let started = Instant::now();
        let result = run_bounded_command("sh", &["-c", "sleep 5"], Duration::from_millis(100));

        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::TimedOut);
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "collector command exceeded its bounded timeout"
        );
    }

    #[test]
    fn collector_captures_stdout_stderr_and_exit_metadata() {
        let output = run_bounded_command(
            "sh",
            &["-c", "printf fixture-out; printf fixture-err >&2; exit 7"],
            Duration::from_secs(1),
        )
        .unwrap();

        assert_eq!(output.stdout, b"fixture-out");
        assert_eq!(output.stderr, b"fixture-err");
        assert_eq!(output.exit_status, Some(7));
        assert_eq!(output.signal, None);
    }

    #[test]
    fn redaction_keeps_controlled_fixture_and_removes_uncontrolled_input() {
        let controlled = MinimizedSequence {
            format_version: 1,
            physical_keys: vec!["KEY_A:press".to_string(), "KEY_A:release".to_string()],
            inter_key_delay_ms: 20,
        };

        let redacted = redact_uncontrolled_input(
            "typed password",
            &controlled,
            &["--user-data-dir=/home/user/private-profile".to_string()],
        );

        assert!(!redacted.serialized_metadata.contains("typed password"));
        assert!(!redacted
            .serialized_metadata
            .contains("/home/user/private-profile"));
        assert!(redacted.serialized_metadata.contains("KEY_A:press"));
        assert!(redacted
            .redacted_fields
            .contains(&"uncontrolled_user_input".to_string()));
        assert!(redacted
            .redacted_fields
            .contains(&"application_command.user_data_dir".to_string()));
    }

    #[test]
    fn collector_writes_a_complete_replayable_artifact_bundle() {
        let directory = tempfile::tempdir().unwrap();
        let request = EvidenceCollectionRequest {
            bundle_dir: directory.path().join("repro-20261019-002"),
            identity: ReproductionIdentity {
                reproduction_id: "repro-20261019-002".to_string(),
                transaction_id: "tx-0002".to_string(),
                visible_revision: 4,
            },
            application_name: "fixture-app".to_string(),
            application_command: vec![
                "sh".to_string(),
                "-c".to_string(),
                "printf app-out; printf app-err >&2".to_string(),
                "--user-data-dir=/home/user/private-profile".to_string(),
            ],
            daemon_command: vec![
                "sh".to_string(),
                "-c".to_string(),
                "printf daemon-out; printf daemon-err >&2".to_string(),
            ],
            backend: "non-grabbed-evdev".to_string(),
            config_summary: vec![
                "auto_restore=true".to_string(),
                "input_method=telex".to_string(),
            ],
            minimized_sequence: MinimizedSequence {
                format_version: 1,
                physical_keys: vec![
                    "KEY_A:press".to_string(),
                    "KEY_A:release".to_string(),
                    "KEY_T:press".to_string(),
                    "KEY_T:release".to_string(),
                    "KEY_X:press".to_string(),
                    "KEY_X:release".to_string(),
                    "KEY_X:press".to_string(),
                    "KEY_X:release".to_string(),
                ],
                inter_key_delay_ms: 20,
            },
            uncontrolled_input: Some("typed password".to_string()),
            command_timeout: Duration::from_secs(1),
        };

        let result = collect_evidence_bundle(&request).unwrap();

        assert_eq!(result.classification, ProcessClassification::Unknown);
        assert!(result.bundle_dir.join("evidence.json").is_file());
        assert!(result.bundle_dir.join("replay.json").is_file());
        assert_eq!(
            std::fs::read_to_string(result.bundle_dir.join("daemon.stdout.log")).unwrap(),
            "daemon-out"
        );
        assert_eq!(
            std::fs::read_to_string(result.bundle_dir.join("daemon.stderr.log")).unwrap(),
            "daemon-err"
        );
        assert_eq!(
            std::fs::read_to_string(result.bundle_dir.join("application.stdout.log")).unwrap(),
            "app-out"
        );
        assert_eq!(
            std::fs::read_to_string(result.bundle_dir.join("application.stderr.log")).unwrap(),
            "app-err"
        );

        let serialized = std::fs::read_to_string(result.bundle_dir.join("evidence.json")).unwrap();
        assert!(!serialized.contains("typed password"));
        assert!(!serialized.contains("/home/user/private-profile"));
        assert!(serialized.contains("non-grabbed-evdev"));
        assert!(serialized.contains("auto_restore=true"));
        assert!(serialized.contains("input_method=telex"));

        let evidence: CrashEvidence = serde_json::from_str(&serialized).unwrap();
        let daemon_process = evidence
            .processes
            .iter()
            .find(|process| process.process_type == "daemon_command")
            .unwrap();
        let application_process = evidence
            .processes
            .iter()
            .find(|process| process.process_type == "browser_process")
            .unwrap();
        assert_ne!(daemon_process.pid, std::process::id());
        assert_ne!(application_process.pid, std::process::id());
        assert_ne!(daemon_process.pid, application_process.pid);
        assert_eq!(daemon_process.exit_status, Some(0));
        assert_eq!(application_process.exit_status, Some(0));

        let replay: MinimizedSequence =
            serde_json::from_slice(&std::fs::read(result.bundle_dir.join("replay.json")).unwrap())
                .unwrap();
        assert_eq!(replay, request.minimized_sequence);
    }

    fn process(
        process_type: &str,
        exit_status: Option<i32>,
        signal: Option<i32>,
    ) -> ProcessEvidence {
        ProcessEvidence {
            pid: 1234,
            parent_pid: Some(1000),
            process_type: process_type.to_string(),
            command_line: vec!["fixture".to_string()],
            exit_status,
            signal,
        }
    }

    #[test]
    fn classification_distinguishes_daemon_renderer_browser_and_protocol_exits() {
        let healthy_daemon = process("daemon_command", Some(0), None);
        let healthy_browser = process("browser_process", Some(0), None);

        assert_eq!(
            classify_process_outcome(
                &process("daemon_command", None, Some(11)),
                &healthy_browser,
                &[],
                false,
            ),
            ProcessClassification::DaemonCrash
        );
        assert_eq!(
            classify_process_outcome(
                &healthy_daemon,
                &process("renderer", None, Some(11)),
                &["renderer".to_string()],
                false,
            ),
            ProcessClassification::RendererCrash
        );
        assert_eq!(
            classify_process_outcome(
                &healthy_daemon,
                &process("browser_process", Some(1), None),
                &["browser_process".to_string()],
                false,
            ),
            ProcessClassification::BrowserProcessCrash
        );
        assert_eq!(
            classify_process_outcome(
                &healthy_daemon,
                &process("application_command", Some(1), None),
                &[],
                true,
            ),
            ProcessClassification::ProtocolTriggeredExit
        );
        assert_eq!(
            classify_process_outcome(&healthy_daemon, &healthy_browser, &[], false),
            ProcessClassification::NoCrashObserved
        );
    }

    #[test]
    fn process_type_and_browser_crash_reporting_are_capability_aware() {
        assert_eq!(
            application_process_type(&["chromium".to_string(), "--type=renderer".to_string(),]),
            "renderer"
        );
        assert_eq!(
            application_process_type(&["google-chrome".to_string()]),
            "browser_process"
        );
        assert_eq!(
            detect_browser_crash_reporting(&[
                "electron".to_string(),
                "--disable-crash-reporter".to_string(),
            ]),
            "DISABLED_BY_COMMAND_LINE"
        );
        assert_eq!(
            detect_browser_crash_reporting(&["fixture-app".to_string()]),
            "UNAVAILABLE_NOT_A_SUPPORTED_BROWSER"
        );
    }

    #[test]
    fn collector_records_missing_optional_system_tools_without_claiming_no_crash() {
        let directory = tempfile::tempdir().unwrap();
        let request = EvidenceCollectionRequest {
            bundle_dir: directory.path().join("missing-tools"),
            identity: ReproductionIdentity {
                reproduction_id: "repro-missing-tools".to_string(),
                transaction_id: "tx-missing-tools".to_string(),
                visible_revision: 0,
            },
            application_name: "fixture-app".to_string(),
            application_command: vec!["true".to_string()],
            daemon_command: vec!["true".to_string()],
            backend: "fixture-backend".to_string(),
            config_summary: vec![],
            minimized_sequence: MinimizedSequence {
                format_version: 1,
                physical_keys: vec!["KEY_A:press".to_string()],
                inter_key_delay_ms: 0,
            },
            uncontrolled_input: None,
            command_timeout: Duration::from_secs(1),
        };

        let tools = SystemEvidenceTools {
            journalctl: "/definitely/missing/journalctl".into(),
            coredumpctl: "/definitely/missing/coredumpctl".into(),
        };
        let result = collect_evidence_bundle_with_tools(&request, &tools).unwrap();

        assert_eq!(result.classification, ProcessClassification::Unknown);
        assert!(result.bundle_dir.join("journal.unavailable.txt").is_file());
        assert!(result
            .bundle_dir
            .join("coredump-list.unavailable.txt")
            .is_file());
        assert!(result
            .bundle_dir
            .join("coredump-info.unavailable.txt")
            .is_file());
    }
}
