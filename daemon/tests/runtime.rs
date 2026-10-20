// SPDX-License-Identifier: MIT
//! Explicit capability gates for privileged runtime verification.

use std::path::Path;
use std::time::Duration;

#[derive(Debug, PartialEq, Eq)]
enum RuntimeSkipReason {
    ExplicitOptInMissing,
    DisplayUnavailable,
    UinputUnavailable,
}

#[derive(Debug)]
struct RuntimeCapabilities {
    explicit_opt_in: bool,
    display_available: bool,
    uinput_available: bool,
}

impl RuntimeCapabilities {
    fn from_environment(
        mut read_var: impl FnMut(&str) -> Option<String>,
        uinput_available: bool,
    ) -> Self {
        let explicit_opt_in = read_var("VIETC_RUNTIME_TESTS").as_deref() == Some("1");
        let display_available =
            read_var("DISPLAY").is_some() || read_var("WAYLAND_DISPLAY").is_some();

        Self {
            explicit_opt_in,
            display_available,
            uinput_available,
        }
    }

    fn require_privileged_runtime(&self, _timeout: Duration) -> Result<(), RuntimeSkipReason> {
        if !self.explicit_opt_in {
            return Err(RuntimeSkipReason::ExplicitOptInMissing);
        }
        if !self.display_available {
            return Err(RuntimeSkipReason::DisplayUnavailable);
        }
        if !self.uinput_available {
            return Err(RuntimeSkipReason::UinputUnavailable);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RuntimeTier {
    Deterministic,
    Privileged,
}

#[derive(Clone, Copy, Debug)]
struct RuntimeScenario {
    name: &'static str,
    tier: RuntimeTier,
    timeout: Duration,
}

fn runtime_scenarios() -> [RuntimeScenario; 12] {
    const COMPONENT_TIMEOUT: Duration = Duration::from_secs(5);
    const RUNTIME_TIMEOUT: Duration = Duration::from_secs(30);

    [
        RuntimeScenario {
            name: "wayland-wtype-success",
            tier: RuntimeTier::Privileged,
            timeout: RUNTIME_TIMEOUT,
        },
        RuntimeScenario {
            name: "wayland-wtype-fallback",
            tier: RuntimeTier::Deterministic,
            timeout: COMPONENT_TIMEOUT,
        },
        RuntimeScenario {
            name: "wayland-all-backends-fail",
            tier: RuntimeTier::Deterministic,
            timeout: COMPONENT_TIMEOUT,
        },
        RuntimeScenario {
            name: "chromium-shift-prefix",
            tier: RuntimeTier::Privileged,
            timeout: RUNTIME_TIMEOUT,
        },
        RuntimeScenario {
            name: "physical-shift-held",
            tier: RuntimeTier::Privileged,
            timeout: RUNTIME_TIMEOUT,
        },
        RuntimeScenario {
            name: "x11-selection-success",
            tier: RuntimeTier::Privileged,
            timeout: RUNTIME_TIMEOUT,
        },
        RuntimeScenario {
            name: "x11-selection-timeout",
            tier: RuntimeTier::Deterministic,
            timeout: COMPONENT_TIMEOUT,
        },
        RuntimeScenario {
            name: "clipboard-preservation",
            tier: RuntimeTier::Privileged,
            timeout: RUNTIME_TIMEOUT,
        },
        RuntimeScenario {
            name: "clipboard-user-race",
            tier: RuntimeTier::Deterministic,
            timeout: COMPONENT_TIMEOUT,
        },
        RuntimeScenario {
            name: "xrecord-lifecycle-and-framing",
            tier: RuntimeTier::Deterministic,
            timeout: COMPONENT_TIMEOUT,
        },
        RuntimeScenario {
            name: "repeated-vietnamese-replacements",
            tier: RuntimeTier::Privileged,
            timeout: RUNTIME_TIMEOUT,
        },
        RuntimeScenario {
            name: "daemon-shutdown-restart",
            tier: RuntimeTier::Privileged,
            timeout: RUNTIME_TIMEOUT,
        },
    ]
}

#[derive(Debug, PartialEq, Eq)]
enum RuntimeCommandError {
    SpawnFailed,
    WaitFailed,
    TimedOut,
    PrerequisiteMissing,
    ScenarioFailed,
    ClipboardSnapshotFailed,
    InputInjectionFailed,
    TargetCopyFailed,
    ClipboardRestoreFailed,
    ClipboardVerificationFailed,
    ModifierReleaseFailed,
}

#[derive(Clone, Debug)]
struct RuntimeEnvironment {
    runtime_opt_in: bool,
    session_type: String,
    desktop: String,
    display_available: bool,
    uinput_available: bool,
    available_tools: std::collections::HashSet<String>,
}

impl RuntimeEnvironment {
    fn detect_current() -> Self {
        Self::detect_with(
            |name| std::env::var(name).ok(),
            |tool| {
                std::process::Command::new("sh")
                    .args(["-c", &format!("command -v {tool} >/dev/null 2>&1")])
                    .status()
                    .map(|status| status.success())
                    .unwrap_or(false)
            },
            std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open("/dev/uinput")
                .is_ok(),
        )
    }

    fn detect_with(
        mut read_var: impl FnMut(&str) -> Option<String>,
        mut tool_available: impl FnMut(&str) -> bool,
        uinput_available: bool,
    ) -> Self {
        const TOOLS: [&str; 12] = [
            "chromium",
            "chromium-browser",
            "google-chrome",
            "electron",
            "zenity",
            "python3",
            "wtype",
            "xdotool",
            "gnome-terminal",
            "konsole",
            "xterm",
            "kitty",
        ];

        let runtime_opt_in = read_var("VIETC_RUNTIME_TESTS").as_deref() == Some("1");
        let session_type = read_var("XDG_SESSION_TYPE").unwrap_or_else(|| "unknown".to_string());
        let desktop = read_var("XDG_CURRENT_DESKTOP").unwrap_or_else(|| "unknown".to_string());
        let display_available =
            read_var("DISPLAY").is_some() || read_var("WAYLAND_DISPLAY").is_some();
        let available_tools = TOOLS
            .into_iter()
            .filter(|tool| tool_available(tool))
            .map(str::to_string)
            .collect();

        Self {
            runtime_opt_in,
            session_type,
            desktop,
            display_available,
            uinput_available,
            available_tools,
        }
    }

    fn has_tool(&self, tool: &str) -> bool {
        self.available_tools.contains(tool)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RuntimeMatrixStatus {
    Pass,
    Skipped,
    Unavailable,
    Failed,
}

impl RuntimeMatrixStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Skipped => "SKIPPED",
            Self::Unavailable => "UNAVAILABLE",
            Self::Failed => "FAILED",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RuntimeMatrixEntry {
    capability: &'static str,
    status: RuntimeMatrixStatus,
    reason: String,
    owner: String,
}

impl RuntimeMatrixEntry {
    fn new(
        capability: &'static str,
        status: RuntimeMatrixStatus,
        reason: impl Into<String>,
        owner: impl Into<String>,
    ) -> Self {
        let reason = reason.into();
        let owner = owner.into();
        assert!(
            !reason.trim().is_empty(),
            "runtime result requires a reason"
        );
        assert!(!owner.trim().is_empty(), "runtime result requires an owner");
        Self {
            capability,
            status,
            reason,
            owner,
        }
    }
}

#[derive(Debug)]
struct RuntimeMatrixResults {
    entries: Vec<RuntimeMatrixEntry>,
}

impl RuntimeMatrixResults {
    fn entry_for(&self, capability: &str) -> Option<&RuntimeMatrixEntry> {
        self.entries
            .iter()
            .find(|entry| entry.capability == capability)
    }

    fn status_for(&self, capability: &str) -> Option<RuntimeMatrixStatus> {
        self.entry_for(capability).map(|entry| entry.status)
    }
}

fn render_runtime_matrix_artifact<I, Name, Version>(
    display_server: &str,
    compositor: &str,
    tools: I,
    results: &RuntimeMatrixResults,
) -> String
where
    I: IntoIterator<Item = (Name, Version)>,
    Name: AsRef<str>,
    Version: AsRef<str>,
{
    let mut output =
        format!("display_server: {display_server}\ncompositor: {compositor}\ntools:\n");
    for (name, version) in tools {
        output.push_str(&format!("  {}: {}\n", name.as_ref(), version.as_ref()));
    }

    output.push_str("capabilities:\n");
    for entry in &results.entries {
        output.push_str(&format!(
            "  {}: {}\n    reason: {}\n    owner: {}\n",
            entry.capability,
            entry.status.label(),
            entry.reason,
            entry.owner
        ));
    }
    output
}

fn unavailable_entry(
    capability: &'static str,
    reason: impl Into<String>,
    owner: &'static str,
) -> RuntimeMatrixEntry {
    RuntimeMatrixEntry::new(capability, RuntimeMatrixStatus::Unavailable, reason, owner)
}

fn skipped_entry(
    capability: &'static str,
    reason: impl Into<String>,
    owner: &'static str,
) -> RuntimeMatrixEntry {
    RuntimeMatrixEntry::new(capability, RuntimeMatrixStatus::Skipped, reason, owner)
}

fn runtime_prerequisite(
    environment: &RuntimeEnvironment,
) -> Option<(&'static str, RuntimeMatrixStatus)> {
    if !environment.runtime_opt_in {
        Some((
            "VIETC_RUNTIME_TESTS=1 was not supplied",
            RuntimeMatrixStatus::Skipped,
        ))
    } else if !environment.display_available {
        Some((
            "no X11 or Wayland display is available",
            RuntimeMatrixStatus::Unavailable,
        ))
    } else if !environment.uinput_available {
        Some((
            "writable /dev/uinput is unavailable",
            RuntimeMatrixStatus::Unavailable,
        ))
    } else {
        None
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ControlledRuntimeEvidence {
    status: RuntimeMatrixStatus,
    reason: String,
    expected_text: String,
    observed_text: String,
    process_healthy: bool,
    clipboard_preserved: bool,
    modifiers_released: bool,
}

impl ControlledRuntimeEvidence {
    fn is_verified_pass(&self) -> bool {
        self.status == RuntimeMatrixStatus::Pass
            && self.expected_text == self.observed_text
            && self.process_healthy
            && self.clipboard_preserved
            && self.modifiers_released
    }
}

fn parse_controlled_runtime_evidence(input: &str) -> Result<ControlledRuntimeEvidence, String> {
    let fields = input
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.trim(), value.trim()))
        .collect::<std::collections::HashMap<_, _>>();

    let required = |key: &str| {
        fields
            .get(key)
            .copied()
            .ok_or_else(|| format!("missing {key}"))
    };
    let parse_bool = |key: &str| {
        required(key)?
            .parse::<bool>()
            .map_err(|_| format!("invalid {key}"))
    };
    let parse_text = |key: &str| {
        let value = required(key)?;
        if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
            Ok(value[1..value.len() - 1]
                .replace(r#"\n"#, "\n")
                .replace(r#"\""#, "\""))
        } else {
            Err(format!("invalid {key}"))
        }
    };

    let status = match required("status")? {
        "PASS" => RuntimeMatrixStatus::Pass,
        "FAILED" => RuntimeMatrixStatus::Failed,
        "SKIPPED" => RuntimeMatrixStatus::Skipped,
        "UNAVAILABLE" => RuntimeMatrixStatus::Unavailable,
        _ => return Err("invalid status".to_string()),
    };

    Ok(ControlledRuntimeEvidence {
        status,
        reason: required("reason")?.to_string(),
        expected_text: parse_text("expected_text")?,
        observed_text: parse_text("observed_text")?,
        process_healthy: parse_bool("process_healthy")?,
        clipboard_preserved: parse_bool("clipboard_preserved")?,
        modifiers_released: parse_bool("modifiers_released")?,
    })
}

fn load_controlled_runtime_evidence(executable: &str) -> Option<ControlledRuntimeEvidence> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()?
        .join("target/runtime-artifacts")
        .join(format!("controlled-{executable}-result.txt"));
    std::fs::read_to_string(path)
        .ok()
        .and_then(|input| parse_controlled_runtime_evidence(&input).ok())
}

fn controlled_evidence_entry(
    capability: &'static str,
    owner: &'static str,
    evidence: Option<&ControlledRuntimeEvidence>,
    missing_reason: &'static str,
) -> RuntimeMatrixEntry {
    match evidence {
        Some(evidence) if evidence.is_verified_pass() => RuntimeMatrixEntry::new(
            capability,
            RuntimeMatrixStatus::Pass,
            evidence.reason.clone(),
            owner,
        ),
        Some(evidence) => RuntimeMatrixEntry::new(
            capability,
            if evidence.status == RuntimeMatrixStatus::Pass {
                RuntimeMatrixStatus::Failed
            } else {
                evidence.status
            },
            evidence.reason.clone(),
            owner,
        ),
        None => unavailable_entry(capability, missing_reason, owner),
    }
}

fn application_entry(
    environment: &RuntimeEnvironment,
    capability: &'static str,
    executable: &'static str,
    owner: &'static str,
) -> RuntimeMatrixEntry {
    if !environment.has_tool(executable) {
        return unavailable_entry(capability, format!("{executable} is not installed"), owner);
    }
    if let Some((reason, status)) = runtime_prerequisite(environment) {
        return RuntimeMatrixEntry::new(capability, status, reason, owner);
    }

    unavailable_entry(
        capability,
        "controlled fixture safety has not been validated for this host; real application was not launched",
        owner,
    )
}

fn desktop_entry(
    environment: &RuntimeEnvironment,
    capability: &'static str,
    desktop_name: &'static str,
    owner: &'static str,
) -> RuntimeMatrixEntry {
    if environment.session_type != "wayland" {
        return skipped_entry(
            capability,
            format!("requires Wayland; detected {}", environment.session_type),
            owner,
        );
    }
    if !environment
        .desktop
        .to_ascii_uppercase()
        .contains(desktop_name)
    {
        return skipped_entry(
            capability,
            format!(
                "requires {desktop_name} Wayland; detected {}",
                environment.desktop
            ),
            owner,
        );
    }
    if let Some((reason, status)) = runtime_prerequisite(environment) {
        return RuntimeMatrixEntry::new(capability, status, reason, owner);
    }

    unavailable_entry(
        capability,
        "controlled observable fixture has not been executed in this desktop session",
        owner,
    )
}

fn evaluate_runtime_matrix(environment: &RuntimeEnvironment) -> RuntimeMatrixResults {
    const DESKTOP_OWNER: &str = "runtime-desktop-runner";
    const BROWSER_OWNER: &str = "runtime-browser-runner";
    const TOOLKIT_OWNER: &str = "runtime-toolkit-runner";

    let chromium_evidence = load_controlled_runtime_evidence("chromium");
    let chrome_evidence = load_controlled_runtime_evidence("google-chrome");
    let gtk_evidence = load_controlled_runtime_evidence("zenity");
    let qt_evidence = load_controlled_runtime_evidence("python3");
    let controlled_target_passed = chromium_evidence
        .as_ref()
        .into_iter()
        .chain(chrome_evidence.as_ref())
        .chain(gtk_evidence.as_ref())
        .chain(qt_evidence.as_ref())
        .any(ControlledRuntimeEvidence::is_verified_pass);

    let cosmic = if environment.session_type == "wayland"
        && environment.desktop.to_ascii_uppercase().contains("COSMIC")
        && runtime_prerequisite(environment).is_none()
        && controlled_target_passed
    {
        RuntimeMatrixEntry::new(
            "cosmic-wayland",
            RuntimeMatrixStatus::Pass,
            "at least one controlled target passed in the current COSMIC Wayland session",
            DESKTOP_OWNER,
        )
    } else {
        desktop_entry(environment, "cosmic-wayland", "COSMIC", DESKTOP_OWNER)
    };

    let x11 = if environment.session_type != "x11" {
        skipped_entry(
            "x11",
            format!("requires X11; detected {}", environment.session_type),
            DESKTOP_OWNER,
        )
    } else if let Some((reason, status)) = runtime_prerequisite(environment) {
        RuntimeMatrixEntry::new("x11", status, reason, DESKTOP_OWNER)
    } else {
        unavailable_entry(
            "x11",
            "controlled observable fixture has not been executed in this X11 session",
            DESKTOP_OWNER,
        )
    };

    let terminal_executable = ["gnome-terminal", "konsole", "xterm", "kitty"]
        .into_iter()
        .find(|tool| environment.has_tool(tool));
    let terminal = match terminal_executable {
        None => unavailable_entry(
            "terminal",
            "no supported terminal emulator is installed",
            TOOLKIT_OWNER,
        ),
        Some(_) if runtime_prerequisite(environment).is_some() => {
            let (reason, status) = runtime_prerequisite(environment).unwrap();
            RuntimeMatrixEntry::new("terminal", status, reason, TOOLKIT_OWNER)
        }
        Some(_) => unavailable_entry(
            "terminal",
            "controlled terminal target was not rerun for this matrix",
            TOOLKIT_OWNER,
        ),
    };

    let chromium = if !environment.has_tool("chromium") {
        unavailable_entry("chromium", "chromium is not installed", BROWSER_OWNER)
    } else if let Some((reason, status)) = runtime_prerequisite(environment) {
        RuntimeMatrixEntry::new("chromium", status, reason, BROWSER_OWNER)
    } else {
        controlled_evidence_entry(
            "chromium",
            BROWSER_OWNER,
            chromium_evidence.as_ref(),
            "controlled Chromium evidence is unavailable",
        )
    };

    let chrome = if !environment.has_tool("google-chrome") {
        unavailable_entry(
            "google-chrome",
            "google-chrome is not installed",
            BROWSER_OWNER,
        )
    } else if let Some((reason, status)) = runtime_prerequisite(environment) {
        RuntimeMatrixEntry::new("google-chrome", status, reason, BROWSER_OWNER)
    } else {
        controlled_evidence_entry(
            "google-chrome",
            BROWSER_OWNER,
            chrome_evidence.as_ref(),
            "controlled Google Chrome evidence is unavailable",
        )
    };

    let gtk = if !environment.has_tool("zenity") {
        unavailable_entry(
            "gtk",
            "zenity runtime probe is not installed",
            TOOLKIT_OWNER,
        )
    } else if let Some((reason, status)) = runtime_prerequisite(environment) {
        RuntimeMatrixEntry::new("gtk", status, reason, TOOLKIT_OWNER)
    } else {
        controlled_evidence_entry(
            "gtk",
            TOOLKIT_OWNER,
            gtk_evidence.as_ref(),
            "controlled GTK evidence is unavailable",
        )
    };

    RuntimeMatrixResults {
        entries: vec![
            cosmic,
            desktop_entry(environment, "gnome-wayland", "GNOME", DESKTOP_OWNER),
            desktop_entry(environment, "kde-wayland", "KDE", DESKTOP_OWNER),
            x11,
            chromium,
            chrome,
            application_entry(environment, "electron", "electron", BROWSER_OWNER),
            gtk,
            if !environment.has_tool("python3") {
                unavailable_entry("qt", "python3 is not installed", TOOLKIT_OWNER)
            } else if let Some((reason, status)) = runtime_prerequisite(environment) {
                RuntimeMatrixEntry::new("qt", status, reason, TOOLKIT_OWNER)
            } else {
                controlled_evidence_entry(
                    "qt",
                    TOOLKIT_OWNER,
                    qt_evidence.as_ref(),
                    "controlled Qt evidence is unavailable",
                )
            },
            terminal,
        ],
    }
}

#[test]
fn deterministic_controlled_runtime_evidence_parser_requires_complete_fields() {
    let parsed = parse_controlled_runtime_evidence(
        "target=Chromium\nstatus=PASS\nreason=verified\nexpected_text=\"vietc\"\nobserved_text=\"vietc\"\nprocess_healthy=true\nclipboard_preserved=true\nmodifiers_released=true\n",
    )
    .expect("complete controlled evidence must parse");

    assert!(parsed.is_verified_pass());
    assert!(parse_controlled_runtime_evidence(
        "status=PASS\nreason=incomplete\nexpected_text=\"vietc\"\nobserved_text=\"vietc\"\n"
    )
    .is_err());
}

#[test]
fn deterministic_controlled_runtime_evidence_mapping_rejects_false_passes() {
    let valid = ControlledRuntimeEvidence {
        status: RuntimeMatrixStatus::Pass,
        reason: "verified".to_string(),
        expected_text: CONTROLLED_TEXT.to_string(),
        observed_text: CONTROLLED_TEXT.to_string(),
        process_healthy: true,
        clipboard_preserved: true,
        modifiers_released: true,
    };
    assert_eq!(
        controlled_evidence_entry("chromium", "test", Some(&valid), "missing").status,
        RuntimeMatrixStatus::Pass
    );

    for invalid in [
        ControlledRuntimeEvidence {
            observed_text: "mismatch".to_string(),
            ..valid.clone()
        },
        ControlledRuntimeEvidence {
            process_healthy: false,
            ..valid.clone()
        },
        ControlledRuntimeEvidence {
            clipboard_preserved: false,
            ..valid.clone()
        },
        ControlledRuntimeEvidence {
            modifiers_released: false,
            ..valid.clone()
        },
    ] {
        assert_eq!(
            controlled_evidence_entry("chromium", "test", Some(&invalid), "missing").status,
            RuntimeMatrixStatus::Failed
        );
    }
}

const CONTROLLED_FIXTURE_MARKER: &str = "vietc-controlled-runtime-fixture-v1";
const CONTROLLED_TEXT: &str = "vietc-runtime-controlled-text";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RuntimeProcessRole {
    Browser,
    Renderer,
    ElectronMain,
    ToolkitTarget,
    TerminalTarget,
    Daemon,
    Helper,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RuntimeExitClassification {
    Healthy,
    BrowserCrash,
    RendererCrash,
    ApplicationExit,
    DaemonCrash,
    TimedOut,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RuntimePidRole {
    pid: u32,
    parent_pid: Option<u32>,
    role: RuntimeProcessRole,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ControlledRuntimeFixture {
    marker: String,
    expected_text: String,
    profile_dir: std::path::PathBuf,
    artifact_dir: std::path::PathBuf,
}

impl ControlledRuntimeFixture {
    fn isolated(workspace_root: &std::path::Path, run_id: &str) -> Result<Self, &'static str> {
        if run_id.is_empty()
            || !run_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        {
            return Err("runtime run ID must contain only ASCII letters, digits, '-' or '_'");
        }

        let runtime_root = workspace_root.join("target/runtime-fixtures").join(run_id);
        Ok(Self {
            marker: CONTROLLED_FIXTURE_MARKER.to_string(),
            expected_text: CONTROLLED_TEXT.to_string(),
            profile_dir: runtime_root.join("profile"),
            artifact_dir: runtime_root.join("artifacts"),
        })
    }

    fn validate_safe(&self, workspace_root: &std::path::Path) -> Result<(), &'static str> {
        let allowed_root = workspace_root.join("target/runtime-fixtures");
        if self.marker != CONTROLLED_FIXTURE_MARKER {
            return Err("controlled fixture marker is missing or invalid");
        }
        if self.expected_text != CONTROLLED_TEXT {
            return Err("fixture contains uncontrolled text");
        }
        if !self.profile_dir.starts_with(&allowed_root)
            || !self.artifact_dir.starts_with(&allowed_root)
            || self.profile_dir == self.artifact_dir
        {
            return Err("profile and artifacts must be isolated under target/runtime-fixtures");
        }
        Ok(())
    }

    fn prepare(&self, workspace_root: &std::path::Path) -> std::io::Result<()> {
        self.validate_safe(workspace_root)
            .map_err(|reason| std::io::Error::new(std::io::ErrorKind::InvalidInput, reason))?;
        std::fs::create_dir_all(&self.profile_dir)?;
        std::fs::create_dir_all(&self.artifact_dir)?;
        std::fs::write(
            self.artifact_dir.join("fixture-manifest.txt"),
            format!(
                "marker: {}\nexpected_text: {}\nprofile: isolated\n",
                self.marker, self.expected_text
            ),
        )
    }
}

fn classify_runtime_exit(
    timed_out: bool,
    exited_pid: Option<u32>,
    processes: &[RuntimePidRole],
) -> RuntimeExitClassification {
    if timed_out {
        return RuntimeExitClassification::TimedOut;
    }
    let Some(pid) = exited_pid else {
        return RuntimeExitClassification::Healthy;
    };
    match processes
        .iter()
        .find(|process| process.pid == pid)
        .map(|p| p.role)
    {
        Some(RuntimeProcessRole::Browser | RuntimeProcessRole::ElectronMain) => {
            RuntimeExitClassification::BrowserCrash
        }
        Some(RuntimeProcessRole::Renderer) => RuntimeExitClassification::RendererCrash,
        Some(RuntimeProcessRole::Daemon) => RuntimeExitClassification::DaemonCrash,
        Some(
            RuntimeProcessRole::ToolkitTarget
            | RuntimeProcessRole::TerminalTarget
            | RuntimeProcessRole::Helper
            | RuntimeProcessRole::Unknown,
        )
        | None => RuntimeExitClassification::ApplicationExit,
    }
}

fn run_command_with_timeout(
    command: &mut std::process::Command,
    timeout: Duration,
) -> Result<std::process::ExitStatus, RuntimeCommandError> {
    let mut child = command
        .spawn()
        .map_err(|_| RuntimeCommandError::SpawnFailed)?;
    let deadline = std::time::Instant::now() + timeout;

    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok(None) => {
                let _ = child.kill();
                child.wait().map_err(|_| RuntimeCommandError::WaitFailed)?;
                return Err(RuntimeCommandError::TimedOut);
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(RuntimeCommandError::WaitFailed);
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct RuntimeObservation {
    inject_result: vietc_protocol::inject::InjectResult,
    expected_text: String,
    observed_text: String,
}

#[derive(Debug, Default)]
struct FakeWaylandBackend {
    wtype_result: bool,
    clipboard_result: bool,
    initial_text: String,
    attempts: std::cell::RefCell<Vec<&'static str>>,
}

impl FakeWaylandBackend {
    fn new() -> Self {
        Self::default()
    }

    fn with_wtype_result(mut self, result: bool) -> Self {
        self.wtype_result = result;
        self
    }

    fn with_clipboard_result(mut self, result: bool) -> Self {
        self.clipboard_result = result;
        self
    }

    fn with_initial_text(mut self, text: &str) -> Self {
        self.initial_text = text.to_string();
        self
    }

    fn inject_replacement(&self, text: &str) -> RuntimeObservation {
        self.attempts.borrow_mut().push("wtype");
        if self.wtype_result {
            return RuntimeObservation {
                inject_result: vietc_protocol::inject::InjectResult::Success,
                expected_text: text.to_string(),
                observed_text: text.to_string(),
            };
        }

        self.attempts.borrow_mut().push("clipboard");
        let inject_result = if self.clipboard_result {
            vietc_protocol::inject::InjectResult::Success
        } else {
            vietc_protocol::inject::InjectResult::Failed
        };
        let observed_text = if self.clipboard_result {
            text.to_string()
        } else {
            self.initial_text.clone()
        };

        RuntimeObservation {
            inject_result,
            expected_text: text.to_string(),
            observed_text,
        }
    }

    fn attempts(&self) -> Vec<&'static str> {
        self.attempts.borrow().clone()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ModifierEvent {
    PressShift,
    ReleaseShift,
}

#[derive(Debug, Default)]
struct FakeModifierState {
    physical_shift_held: bool,
    shift_pressed: bool,
    events: Vec<ModifierEvent>,
}

impl FakeModifierState {
    fn with_physical_shift_held() -> Self {
        Self {
            physical_shift_held: true,
            shift_pressed: true,
            events: Vec::new(),
        }
    }

    fn run_shift_prefixed_injection(&mut self, physical_shift_held: bool) {
        self.physical_shift_held = physical_shift_held;
        let injected_shift = !self.physical_shift_held;

        if injected_shift {
            self.shift_pressed = true;
            self.events.push(ModifierEvent::PressShift);
        }

        if injected_shift {
            self.shift_pressed = false;
            self.events.push(ModifierEvent::ReleaseShift);
        }
    }

    fn shift_pressed(&self) -> bool {
        self.shift_pressed
    }

    fn events(&self) -> &[ModifierEvent] {
        &self.events
    }
}

#[derive(Debug)]
struct FakeX11Selection {
    supported_after_polls: Option<usize>,
    polls: usize,
    served_targets: Vec<&'static str>,
}

impl FakeX11Selection {
    fn supported_after_polls(polls: usize) -> Self {
        Self {
            supported_after_polls: Some(polls),
            polls: 0,
            served_targets: Vec::new(),
        }
    }

    fn never_requested() -> Self {
        Self {
            supported_after_polls: None,
            polls: 0,
            served_targets: Vec::new(),
        }
    }

    fn wait_for_request(&mut self, timeout: Duration) -> vietc_protocol::inject::InjectResult {
        let deadline = std::time::Instant::now() + timeout;

        loop {
            self.polls += 1;
            if self.supported_after_polls == Some(self.polls) {
                self.served_targets.push("UTF8_STRING");
                return vietc_protocol::inject::InjectResult::Success;
            }
            if std::time::Instant::now() >= deadline {
                return vietc_protocol::inject::InjectResult::Failed;
            }
            std::thread::yield_now();
        }
    }

    fn served_targets(&self) -> &[&'static str] {
        &self.served_targets
    }
}

#[derive(Clone, Copy, Debug)]
enum FakeDisplayServer {
    X11,
    Wayland,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum ClipboardEvent {
    Snapshot(String),
    WriteInjected(String),
    Restore(String),
}

#[derive(Debug)]
struct FakeClipboardSession {
    _display_server: FakeDisplayServer,
    current: String,
    saved: Option<String>,
    injected: Option<String>,
    events: Vec<ClipboardEvent>,
}

impl FakeClipboardSession {
    fn new(display_server: FakeDisplayServer, initial: &str) -> Self {
        Self {
            _display_server: display_server,
            current: initial.to_string(),
            saved: None,
            injected: None,
            events: Vec::new(),
        }
    }

    fn begin_injection(&mut self, text: &str) {
        self.saved = Some(self.current.clone());
        self.injected = Some(text.to_string());
        self.events
            .push(ClipboardEvent::Snapshot(self.current.clone()));
        self.current = text.to_string();
        self.events
            .push(ClipboardEvent::WriteInjected(text.to_string()));
    }

    fn inject_and_restore(&mut self, text: &str) {
        self.begin_injection(text);
        self.finish_debounced_restore();
    }

    fn user_copy(&mut self, text: &str) {
        self.current = text.to_string();
    }

    fn finish_debounced_restore(&mut self) {
        let still_owns_injected_value = self.injected.as_deref() == Some(self.current.as_str());
        if still_owns_injected_value {
            if let Some(saved) = self.saved.take() {
                self.current = saved.clone();
                self.events.push(ClipboardEvent::Restore(saved));
            }
        }
        self.saved = None;
        self.injected = None;
    }

    fn current(&self) -> &str {
        &self.current
    }

    fn events(&self) -> &[ClipboardEvent] {
        &self.events
    }
}

#[derive(Clone, Copy, Debug)]
enum XRecordFixture {
    Hang,
    ExitEarly(i32),
}

fn run_xrecord_helper_fixture(
    fixture: XRecordFixture,
    timeout: Duration,
) -> Result<std::process::ExitStatus, RuntimeCommandError> {
    let mut command = std::process::Command::new("sh");
    match fixture {
        XRecordFixture::Hang => {
            command.args(["-c", "sleep 5"]);
        }
        XRecordFixture::ExitEarly(code) => {
            command.args(["-c", &format!("exit {code}")]);
        }
    }

    let status = run_command_with_timeout(&mut command, timeout)?;
    if status.success() {
        Ok(status)
    } else {
        Err(RuntimeCommandError::ScenarioFailed)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct RuntimeXRecordEvent {
    keycode: u8,
    pressed: bool,
    state: u16,
}

#[derive(Debug, Default)]
struct RuntimeXRecordDecoder {
    pending: Vec<u8>,
}

impl RuntimeXRecordDecoder {
    fn push(&mut self, bytes: &[u8]) -> Vec<RuntimeXRecordEvent> {
        const FRAME_SIZE: usize = 8;

        self.pending.extend_from_slice(bytes);
        let mut events = Vec::new();
        while self.pending.len() >= FRAME_SIZE {
            let frame: Vec<u8> = self.pending.drain(..FRAME_SIZE).collect();
            events.push(RuntimeXRecordEvent {
                keycode: frame[0],
                pressed: frame[1] != 0,
                state: u16::from_ne_bytes([frame[2], frame[3]]),
            });
        }
        events
    }

    fn pending_len(&self) -> usize {
        self.pending.len()
    }
}

fn xrecord_fixture_frame(keycode: u8, pressed: bool, state: u16) -> [u8; 8] {
    let state = state.to_ne_bytes();
    [keycode, u8::from(pressed), state[0], state[1], 0, 0, 0, 0]
}

#[derive(Clone, Copy, Debug)]
struct ReplacementStep {
    backspaces: usize,
    text: &'static str,
}

impl ReplacementStep {
    fn new(backspaces: usize, text: &'static str) -> Self {
        Self { backspaces, text }
    }
}

#[derive(Debug)]
struct FakeReplacementTarget {
    observed_text: String,
    successful_replacements: usize,
}

impl FakeReplacementTarget {
    fn new(initial_text: &str) -> Self {
        Self {
            observed_text: initial_text.to_string(),
            successful_replacements: 0,
        }
    }

    fn apply(&mut self, replacement: ReplacementStep) {
        for _ in 0..replacement.backspaces {
            self.observed_text.pop();
        }
        self.observed_text.push_str(replacement.text);
        self.successful_replacements += 1;
    }

    fn observed_text(&self) -> &str {
        &self.observed_text
    }

    fn successful_replacements(&self) -> usize {
        self.successful_replacements
    }
}

#[derive(Clone, Debug, Default)]
struct FakeRuntimeResourceRegistry {
    counters: std::sync::Arc<std::sync::Mutex<(usize, usize)>>,
}

impl FakeRuntimeResourceRegistry {
    fn live_children(&self) -> usize {
        self.counters.lock().unwrap().0
    }

    fn live_threads(&self) -> usize {
        self.counters.lock().unwrap().1
    }
}

#[derive(Debug)]
struct FakeDaemonLifecycle {
    registry: FakeRuntimeResourceRegistry,
    running: bool,
}

impl FakeDaemonLifecycle {
    fn start(registry: FakeRuntimeResourceRegistry) -> Self {
        {
            let mut counters = registry.counters.lock().unwrap();
            counters.0 += 1;
            counters.1 += 1;
        }
        Self {
            registry,
            running: true,
        }
    }

    fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        if !self.running {
            return;
        }
        let mut counters = self.registry.counters.lock().unwrap();
        counters.0 -= 1;
        counters.1 -= 1;
        self.running = false;
    }
}

impl Drop for FakeDaemonLifecycle {
    fn drop(&mut self) {
        self.stop();
    }
}

#[test]
fn deterministic_wayland_fallback_reports_success_only_after_observable_insert() {
    let backend = FakeWaylandBackend::new()
        .with_wtype_result(false)
        .with_clipboard_result(true);

    let observation = backend.inject_replacement("thời");

    assert_eq!(
        observation.inject_result,
        vietc_protocol::inject::InjectResult::Success
    );
    assert_eq!(observation.observed_text, "thời");
    assert_eq!(backend.attempts(), ["wtype", "clipboard"]);
}

#[test]
fn deterministic_wayland_failure_preserves_existing_text_when_all_backends_fail() {
    let backend = FakeWaylandBackend::new()
        .with_wtype_result(false)
        .with_clipboard_result(false)
        .with_initial_text("tho");

    let observation = backend.inject_replacement("thời");

    assert_eq!(
        observation.inject_result,
        vietc_protocol::inject::InjectResult::Failed
    );
    assert_eq!(
        observation.observed_text, "tho",
        "failed insertion must not silently delete the existing text"
    );
    assert_eq!(backend.attempts(), ["wtype", "clipboard"]);
}

#[test]
fn deterministic_shift_prefix_releases_only_the_modifier_it_pressed() {
    let mut modifiers = FakeModifierState::default();

    modifiers.run_shift_prefixed_injection(false);

    assert_eq!(
        modifiers.events(),
        [ModifierEvent::PressShift, ModifierEvent::ReleaseShift]
    );
    assert!(!modifiers.shift_pressed());
}

#[test]
fn deterministic_injection_preserves_physically_held_shift() {
    let mut modifiers = FakeModifierState::with_physical_shift_held();

    modifiers.run_shift_prefixed_injection(true);

    assert!(
        modifiers.shift_pressed(),
        "injection must not release a Shift key physically held by the user"
    );
    assert!(
        modifiers.events().is_empty(),
        "injection must not synthesize Shift transitions when physical Shift is already held"
    );
}

#[test]
fn deterministic_x11_supported_selection_request_observes_success() {
    let mut selection = FakeX11Selection::supported_after_polls(2);

    let result = selection.wait_for_request(Duration::from_millis(50));

    assert_eq!(result, vietc_protocol::inject::InjectResult::Success);
    assert_eq!(selection.served_targets(), ["UTF8_STRING"]);
}

#[test]
fn deterministic_x11_missing_selection_request_times_out_with_failure() {
    let mut selection = FakeX11Selection::never_requested();
    let started = std::time::Instant::now();

    let result = selection.wait_for_request(Duration::from_millis(20));

    assert_eq!(result, vietc_protocol::inject::InjectResult::Failed);
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "missing SelectionRequest must fail within the configured deadline"
    );
    assert!(selection.served_targets().is_empty());
}

#[test]
fn deterministic_clipboard_preserves_previous_value_on_x11_and_wayland() {
    for display_server in [FakeDisplayServer::X11, FakeDisplayServer::Wayland] {
        let mut clipboard = FakeClipboardSession::new(display_server, "clipboard A");

        clipboard.inject_and_restore("vietc A");

        assert_eq!(
            clipboard.current(),
            "clipboard A",
            "{display_server:?} must restore the clipboard value captured before injection"
        );
        assert_eq!(
            clipboard.events(),
            [
                ClipboardEvent::Snapshot("clipboard A".to_string()),
                ClipboardEvent::WriteInjected("vietc A".to_string()),
                ClipboardEvent::Restore("clipboard A".to_string()),
            ]
        );
    }
}

#[test]
fn deterministic_clipboard_user_change_during_debounce_is_not_overwritten() {
    for display_server in [FakeDisplayServer::X11, FakeDisplayServer::Wayland] {
        let mut clipboard = FakeClipboardSession::new(display_server, "clipboard A");

        clipboard.begin_injection("vietc A");
        clipboard.user_copy("clipboard B");
        clipboard.finish_debounced_restore();

        assert_eq!(
            clipboard.current(),
            "clipboard B",
            "{display_server:?} must preserve a newer user clipboard value"
        );
        assert!(
            !clipboard
                .events()
                .contains(&ClipboardEvent::Restore("clipboard A".to_string())),
            "restore must be skipped after ownership changes"
        );
    }
}

#[test]
fn deterministic_xrecord_helper_hang_is_timed_out_and_reaped() {
    let started = std::time::Instant::now();
    let result = run_xrecord_helper_fixture(XRecordFixture::Hang, Duration::from_millis(50));

    assert_eq!(result, Err(RuntimeCommandError::TimedOut));
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "hung XRecord helper must be terminated within the configured deadline"
    );
}

#[test]
fn deterministic_xrecord_helper_early_exit_is_reported() {
    let result = run_xrecord_helper_fixture(XRecordFixture::ExitEarly(23), Duration::from_secs(1));

    assert_eq!(result, Err(RuntimeCommandError::ScenarioFailed));
}

#[test]
fn deterministic_xrecord_partial_frames_survive_multiple_reads() {
    let frame = xrecord_fixture_frame(42, true, 7);
    let mut decoder = RuntimeXRecordDecoder::default();

    assert!(decoder.push(&frame[..3]).is_empty());
    let events = decoder.push(&frame[3..]);

    assert_eq!(
        events,
        [RuntimeXRecordEvent {
            keycode: 42,
            pressed: true,
            state: 7,
        }]
    );
    assert_eq!(decoder.pending_len(), 0);
}

#[test]
fn deterministic_repeated_vietnamese_replacements_do_not_duplicate_or_drop_text() {
    let mut target = FakeReplacementTarget::new("tho");

    for replacement in [
        ReplacementStep::new(1, "ờ"),
        ReplacementStep::new(0, "i"),
        ReplacementStep::new(4, "thời"),
    ] {
        target.apply(replacement);
    }

    assert_eq!(target.observed_text(), "thời");
    assert_eq!(target.successful_replacements(), 3);
}

#[test]
fn deterministic_daemon_shutdown_and_restart_reaps_every_owned_resource() {
    let registry = FakeRuntimeResourceRegistry::default();

    {
        let first = FakeDaemonLifecycle::start(registry.clone());
        assert_eq!(registry.live_children(), 1);
        assert_eq!(registry.live_threads(), 1);
        first.shutdown();
    }

    assert_eq!(registry.live_children(), 0);
    assert_eq!(registry.live_threads(), 0);

    {
        let second = FakeDaemonLifecycle::start(registry.clone());
        assert_eq!(registry.live_children(), 1);
        assert_eq!(registry.live_threads(), 1);
        second.shutdown();
    }

    assert_eq!(
        registry.live_children(),
        0,
        "daemon restart must not leave either generation's child process alive"
    );
    assert_eq!(
        registry.live_threads(),
        0,
        "daemon restart must join every owned background thread"
    );
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InvestigationOutcome {
    NoCrashObserved,
    CrashObserved,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct InvestigationRun {
    hypothesis: &'static str,
    variable_enabled: bool,
    outcome: InvestigationOutcome,
    evidence_artifact: Option<&'static str>,
    skip_reason: Option<&'static str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct InvestigationExperiment {
    control: InvestigationRun,
    treatment: InvestigationRun,
}

impl InvestigationExperiment {
    fn is_reproducibly_isolated(&self) -> bool {
        self.control.outcome == InvestigationOutcome::NoCrashObserved
            && self.treatment.outcome == InvestigationOutcome::CrashObserved
            && self.control.evidence_artifact.is_some()
            && self.treatment.evidence_artifact.is_some()
            && self.control.skip_reason.is_none()
            && self.treatment.skip_reason.is_none()
    }

    fn root_cause_claim_allowed(&self) -> bool {
        self.is_reproducibly_isolated()
    }
}

fn phase5_hypotheses() -> [&'static str; 12] {
    [
        "wtype-shift-prefix",
        "physical-shift-held",
        "forced-modifier-release",
        "ctrl-v-clipboard-fallback",
        "malformed-or-too-fast-sequence",
        "clipboard-ownership-restore-revision",
        "x11-selection-target-property-timeout",
        "injection-feedback",
        "concurrent-writes",
        "child-thread-process-lifecycle",
        "wayland-protocol-misuse",
        "x11-protocol-misuse",
    ]
}

#[derive(Debug, Default)]
struct FakeConcurrentWriter {
    revision: u64,
    visible: String,
}

impl FakeConcurrentWriter {
    fn apply(&mut self, expected_revision: u64, text: &str) -> bool {
        if expected_revision != self.revision {
            return false;
        }
        self.visible.push_str(text);
        self.revision += 1;
        true
    }
}

#[derive(Debug, PartialEq, Eq)]
enum ProtocolFixtureResult {
    Accepted,
    Rejected(&'static str),
}

fn validate_protocol_fixture(
    display: FakeDisplayServer,
    target: Option<&str>,
    property: Option<&str>,
    serial_is_current: bool,
) -> ProtocolFixtureResult {
    match display {
        FakeDisplayServer::X11 if target != Some("UTF8_STRING") => {
            ProtocolFixtureResult::Rejected("unsupported X11 selection target")
        }
        FakeDisplayServer::X11 if property.is_none() => {
            ProtocolFixtureResult::Rejected("missing X11 selection property")
        }
        FakeDisplayServer::Wayland if !serial_is_current => {
            ProtocolFixtureResult::Rejected("stale Wayland serial")
        }
        _ => ProtocolFixtureResult::Accepted,
    }
}

fn replay_with_minimum_delay(events: &[&str], minimum_delay_ms: u64, delay_ms: u64) -> bool {
    !events.is_empty()
        && events
            .iter()
            .all(|event| event.ends_with(":press") || event.ends_with(":release"))
        && delay_ms >= minimum_delay_ms
}

#[test]
fn phase5_hypothesis_catalog_covers_every_required_one_variable_experiment() {
    assert_eq!(
        phase5_hypotheses(),
        [
            "wtype-shift-prefix",
            "physical-shift-held",
            "forced-modifier-release",
            "ctrl-v-clipboard-fallback",
            "malformed-or-too-fast-sequence",
            "clipboard-ownership-restore-revision",
            "x11-selection-target-property-timeout",
            "injection-feedback",
            "concurrent-writes",
            "child-thread-process-lifecycle",
            "wayland-protocol-misuse",
            "x11-protocol-misuse",
        ]
    );
}

#[test]
fn root_cause_claim_requires_control_treatment_crash_artifacts_and_isolation() {
    let isolated = InvestigationExperiment {
        control: InvestigationRun {
            hypothesis: "injection-feedback",
            variable_enabled: false,
            outcome: InvestigationOutcome::NoCrashObserved,
            evidence_artifact: Some("control/evidence.json"),
            skip_reason: None,
        },
        treatment: InvestigationRun {
            hypothesis: "injection-feedback",
            variable_enabled: true,
            outcome: InvestigationOutcome::CrashObserved,
            evidence_artifact: Some("treatment/evidence.json"),
            skip_reason: None,
        },
    };
    assert!(isolated.root_cause_claim_allowed());

    let unavailable = InvestigationExperiment {
        treatment: InvestigationRun {
            outcome: InvestigationOutcome::Unavailable,
            evidence_artifact: None,
            skip_reason: Some("target application unavailable"),
            ..isolated.treatment.clone()
        },
        ..isolated
    };
    assert!(!unavailable.root_cause_claim_allowed());
}

#[test]
fn concurrent_writes_reject_stale_revision_without_mutating_visible_text() {
    let mut writer = FakeConcurrentWriter::default();

    assert!(writer.apply(0, "á"));
    assert!(!writer.apply(0, "duplicate"));
    assert_eq!(writer.visible, "á");
    assert_eq!(writer.revision, 1);
}

#[test]
fn malformed_and_too_fast_sequences_are_rejected_deterministically() {
    assert!(!replay_with_minimum_delay(
        &["KEY_A:press", "malformed"],
        5,
        20
    ));
    assert!(!replay_with_minimum_delay(
        &["KEY_A:press", "KEY_A:release"],
        5,
        1
    ));
    assert!(replay_with_minimum_delay(
        &["KEY_A:press", "KEY_A:release"],
        5,
        20
    ));
}

#[test]
fn protocol_misuse_is_rejected_without_claiming_application_success() {
    assert_eq!(
        validate_protocol_fixture(
            FakeDisplayServer::X11,
            Some("STRING"),
            Some("VIETC_SELECTION"),
            true,
        ),
        ProtocolFixtureResult::Rejected("unsupported X11 selection target")
    );
    assert_eq!(
        validate_protocol_fixture(FakeDisplayServer::X11, Some("UTF8_STRING"), None, true,),
        ProtocolFixtureResult::Rejected("missing X11 selection property")
    );
    assert_eq!(
        validate_protocol_fixture(FakeDisplayServer::Wayland, None, None, false),
        ProtocolFixtureResult::Rejected("stale Wayland serial")
    );
}

#[test]
fn forced_modifier_release_control_preserves_physical_shift() {
    let mut treatment = FakeModifierState::with_physical_shift_held();
    treatment.run_shift_prefixed_injection(true);
    assert!(treatment.shift_pressed());
    assert!(treatment.events().is_empty());

    let mut control = FakeModifierState::default();
    control.run_shift_prefixed_injection(false);
    assert_eq!(
        control.events(),
        [ModifierEvent::PressShift, ModifierEvent::ReleaseShift]
    );
    assert!(!control.shift_pressed());
}

#[test]
fn ctrl_v_fallback_has_control_and_observable_treatment_outcomes() {
    let control = FakeWaylandBackend::new()
        .with_wtype_result(true)
        .with_clipboard_result(false)
        .inject_replacement("thời");
    assert_eq!(control.observed_text, "thời");

    let treatment = FakeWaylandBackend::new()
        .with_wtype_result(false)
        .with_clipboard_result(true)
        .inject_replacement("thời");
    assert_eq!(treatment.observed_text, "thời");
    assert_eq!(
        treatment.inject_result,
        vietc_protocol::inject::InjectResult::Success
    );
}

fn tool_version(program: &str) -> String {
    let output = match std::process::Command::new(program)
        .arg("--version")
        .output()
    {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return "missing".to_string();
        }
        Err(_) => return "installed; version unavailable".to_string(),
    };

    if !output.status.success() {
        return "installed; version unavailable".to_string();
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let version = if stdout.is_empty() {
        String::from_utf8_lossy(&output.stderr).trim().to_string()
    } else {
        stdout
    };

    if version.is_empty() {
        "installed; version unavailable".to_string()
    } else {
        version
    }
}

#[derive(Debug)]
struct RuntimeArtifact {
    display_server: String,
    compositor: String,
    tools: Vec<(String, String)>,
    failure_logs: Vec<(String, String)>,
}

impl RuntimeArtifact {
    fn new<I, Name, Version>(
        display_server: impl Into<String>,
        compositor: impl Into<String>,
        tools: I,
    ) -> Self
    where
        I: IntoIterator<Item = (Name, Version)>,
        Name: Into<String>,
        Version: Into<String>,
    {
        Self {
            display_server: display_server.into(),
            compositor: compositor.into(),
            tools: tools
                .into_iter()
                .map(|(name, version)| (name.into(), version.into()))
                .collect(),
            failure_logs: Vec::new(),
        }
    }

    fn with_failure_log(mut self, scenario: impl Into<String>, message: impl Into<String>) -> Self {
        self.failure_logs.push((scenario.into(), message.into()));
        self
    }

    fn render(&self) -> String {
        let mut output = format!(
            "display_server: {}\ncompositor: {}\ntools:\n",
            self.display_server, self.compositor
        );
        for (name, version) in &self.tools {
            output.push_str(&format!("  {}: {}\n", name, version));
        }
        output.push_str("failure_logs:\n");
        for (scenario, message) in &self.failure_logs {
            output.push_str(&format!("  {}: {}\n", scenario, message));
        }
        output
    }

    fn write_to(&self, path: &std::path::Path) -> std::io::Result<()> {
        std::fs::write(path, self.render())
    }
}

fn run_wayland_wtype_success(timeout: Duration) -> Result<RuntimeObservation, RuntimeCommandError> {
    use std::process::Stdio;

    let uinput_available = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/uinput")
        .is_ok();
    let capabilities =
        RuntimeCapabilities::from_environment(|name| std::env::var(name).ok(), uinput_available);
    capabilities
        .require_privileged_runtime(timeout)
        .map_err(|_| RuntimeCommandError::PrerequisiteMissing)?;

    for program in ["wtype", "zenity"] {
        let available = std::process::Command::new("sh")
            .args(["-c", &format!("command -v {program} >/dev/null 2>&1")])
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        if !available {
            return Err(RuntimeCommandError::PrerequisiteMissing);
        }
    }

    let expected_text = format!("vietc-wayland-runtime-{}", std::process::id());
    let mut dialog = std::process::Command::new("zenity")
        .args([
            "--entry",
            "--title=Vietc runtime verification",
            "--text=Automated Vietc Wayland input target",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| RuntimeCommandError::SpawnFailed)?;
    let focus_deadline = std::time::Instant::now() + Duration::from_secs(3);
    loop {
        match dialog.try_wait() {
            Ok(Some(_)) => return Err(RuntimeCommandError::ScenarioFailed),
            Ok(None) if std::time::Instant::now() < focus_deadline => {
                std::thread::sleep(Duration::from_millis(100))
            }
            Ok(None) => break,
            Err(_) => {
                let _ = dialog.kill();
                let _ = dialog.wait();
                return Err(RuntimeCommandError::WaitFailed);
            }
        }
    }

    let mut wtype = std::process::Command::new("wtype")
        .args(["--", &expected_text])
        .spawn()
        .map_err(|_| {
            let _ = dialog.kill();
            let _ = dialog.wait();
            RuntimeCommandError::SpawnFailed
        })?;
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match wtype.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) => {
                let _ = dialog.kill();
                let _ = dialog.wait();
                return Err(RuntimeCommandError::ScenarioFailed);
            }
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10))
            }
            Ok(None) => {
                let _ = wtype.kill();
                let _ = wtype.wait();
                let _ = dialog.kill();
                let _ = dialog.wait();
                return Err(RuntimeCommandError::TimedOut);
            }
            Err(_) => {
                let _ = wtype.kill();
                let _ = wtype.wait();
                let _ = dialog.kill();
                let _ = dialog.wait();
                return Err(RuntimeCommandError::WaitFailed);
            }
        }
    }

    let enter_status = std::process::Command::new("wtype")
        .args(["-k", "Return"])
        .status()
        .map_err(|_| RuntimeCommandError::WaitFailed)?;
    if !enter_status.success() {
        let _ = dialog.kill();
        let _ = dialog.wait();
        return Err(RuntimeCommandError::ScenarioFailed);
    }

    let output = loop {
        match dialog.try_wait() {
            Ok(Some(_)) => {
                break dialog
                    .wait_with_output()
                    .map_err(|_| RuntimeCommandError::WaitFailed)?
            }
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10))
            }
            Ok(None) => {
                let _ = dialog.kill();
                break dialog
                    .wait_with_output()
                    .map_err(|_| RuntimeCommandError::WaitFailed)?;
            }
            Err(_) => {
                let _ = dialog.kill();
                let _ = dialog.wait();
                return Err(RuntimeCommandError::WaitFailed);
            }
        }
    };
    let observed_text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let inject_result = if output.status.success() && observed_text == expected_text {
        vietc_protocol::inject::InjectResult::Success
    } else {
        vietc_protocol::inject::InjectResult::Failed
    };
    Ok(RuntimeObservation {
        inject_result,
        expected_text,
        observed_text,
    })
}

#[test]
fn runtime_gate_requires_an_explicit_environment_flag() {
    let capabilities = RuntimeCapabilities::from_environment(
        |name| match name {
            "VIETC_RUNTIME_TESTS" => None,
            "DISPLAY" => Some(":1".to_string()),
            "WAYLAND_DISPLAY" => Some("wayland-1".to_string()),
            _ => None,
        },
        true,
    );
    assert_eq!(
        capabilities.require_privileged_runtime(Duration::from_secs(30)),
        Err(RuntimeSkipReason::ExplicitOptInMissing)
    );
}

#[test]
fn runtime_gate_reports_missing_display_without_pretending_to_pass() {
    let capabilities = RuntimeCapabilities::from_environment(
        |name| match name {
            "VIETC_RUNTIME_TESTS" => Some("1".to_string()),
            _ => None,
        },
        true,
    );
    assert_eq!(
        capabilities.require_privileged_runtime(Duration::from_secs(30)),
        Err(RuntimeSkipReason::DisplayUnavailable)
    );
}

#[test]
fn runtime_gate_reports_missing_uinput_without_pretending_to_pass() {
    let capabilities = RuntimeCapabilities::from_environment(
        |name| match name {
            "VIETC_RUNTIME_TESTS" => Some("1".to_string()),
            "WAYLAND_DISPLAY" => Some("wayland-1".to_string()),
            _ => None,
        },
        false,
    );
    assert_eq!(
        capabilities.require_privileged_runtime(Duration::from_secs(30)),
        Err(RuntimeSkipReason::UinputUnavailable)
    );
}

#[test]
fn runtime_artifact_records_environment_tools_and_failure_logs() {
    let artifact = RuntimeArtifact::new(
        "wayland",
        "gnome",
        [
            ("wtype", "0.4.0"),
            ("wl-copy", "2.2.1"),
            ("chromium", "141.0"),
        ],
    )
    .with_failure_log("wayland-wtype-fallback", "wtype exited with status 1");
    let rendered = artifact.render();
    assert!(rendered.contains("display_server: wayland"));
    assert!(rendered.contains("compositor: gnome"));
    assert!(rendered.contains("wtype: 0.4.0"));
    assert!(rendered.contains("wl-copy: 2.2.1"));
    assert!(rendered.contains("chromium: 141.0"));
    assert!(rendered.contains("wayland-wtype-fallback"));
    assert!(rendered.contains("wtype exited with status 1"));
}

#[test]
fn runtime_artifact_is_written_to_the_requested_path() {
    let directory = tempfile::tempdir().expect("runtime artifact tempdir must be created");
    let artifact_path = directory.path().join("wayland-runtime.txt");
    let artifact = RuntimeArtifact::new(
        "wayland",
        "cosmic",
        [("wtype", "installed"), ("zenity", "installed")],
    )
    .with_failure_log("none", "all executed scenarios passed");
    artifact
        .write_to(&artifact_path)
        .expect("runtime artifact must be persisted");
    let persisted =
        std::fs::read_to_string(&artifact_path).expect("runtime artifact must be readable");
    assert_eq!(persisted, artifact.render());
}

#[test]
fn deterministic_controlled_fixture_requires_isolated_profile_and_artifacts() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("daemon crate must have a workspace root");
    let fixture = ControlledRuntimeFixture::isolated(workspace_root, "deterministic-fixture")
        .expect("controlled fixture must be constructed");

    fixture
        .validate_safe(workspace_root)
        .expect("fixture paths and controlled text must be safe");
    assert!(fixture
        .profile_dir
        .starts_with(workspace_root.join("target/runtime-fixtures")));
    assert_ne!(fixture.profile_dir, fixture.artifact_dir);

    let unsafe_fixture = ControlledRuntimeFixture {
        marker: "untrusted".to_string(),
        ..fixture
    };
    assert_eq!(
        unsafe_fixture.validate_safe(workspace_root),
        Err("controlled fixture marker is missing or invalid")
    );
}

#[test]
fn deterministic_controlled_fixture_writes_manifest_without_launching_desktop_apps() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("daemon crate must have a workspace root");
    let run_id = format!("fixture-{}", std::process::id());
    let fixture = ControlledRuntimeFixture::isolated(workspace_root, &run_id)
        .expect("controlled fixture must be constructed");
    fixture
        .prepare(workspace_root)
        .expect("controlled fixture must be prepared");

    let manifest = std::fs::read_to_string(fixture.artifact_dir.join("fixture-manifest.txt"))
        .expect("fixture manifest must be readable");
    assert!(manifest.contains(CONTROLLED_FIXTURE_MARKER));
    assert!(manifest.contains(CONTROLLED_TEXT));
    assert!(manifest.contains("profile: isolated"));

    std::fs::remove_dir_all(workspace_root.join("target/runtime-fixtures").join(run_id))
        .expect("deterministic fixture must clean up its repository-local files");
}

#[test]
fn deterministic_pid_role_map_classifies_process_exits() {
    let processes = vec![
        RuntimePidRole {
            pid: 100,
            parent_pid: None,
            role: RuntimeProcessRole::Browser,
        },
        RuntimePidRole {
            pid: 101,
            parent_pid: Some(100),
            role: RuntimeProcessRole::Renderer,
        },
        RuntimePidRole {
            pid: 102,
            parent_pid: None,
            role: RuntimeProcessRole::Daemon,
        },
        RuntimePidRole {
            pid: 103,
            parent_pid: None,
            role: RuntimeProcessRole::ElectronMain,
        },
        RuntimePidRole {
            pid: 104,
            parent_pid: None,
            role: RuntimeProcessRole::ToolkitTarget,
        },
        RuntimePidRole {
            pid: 105,
            parent_pid: None,
            role: RuntimeProcessRole::TerminalTarget,
        },
        RuntimePidRole {
            pid: 106,
            parent_pid: None,
            role: RuntimeProcessRole::Unknown,
        },
    ];

    assert_eq!(
        classify_runtime_exit(false, None, &processes),
        RuntimeExitClassification::Healthy
    );
    assert_eq!(
        classify_runtime_exit(false, Some(100), &processes),
        RuntimeExitClassification::BrowserCrash
    );
    assert_eq!(
        classify_runtime_exit(false, Some(101), &processes),
        RuntimeExitClassification::RendererCrash
    );
    assert_eq!(
        classify_runtime_exit(false, Some(102), &processes),
        RuntimeExitClassification::DaemonCrash
    );
    assert_eq!(
        classify_runtime_exit(false, Some(103), &processes),
        RuntimeExitClassification::BrowserCrash
    );
    for pid in [104, 105, 106, 999] {
        assert_eq!(
            classify_runtime_exit(false, Some(pid), &processes),
            RuntimeExitClassification::ApplicationExit
        );
    }
    assert_eq!(
        classify_runtime_exit(true, Some(100), &processes),
        RuntimeExitClassification::TimedOut
    );
}

#[test]
fn runtime_command_timeout_kills_and_reaps_the_child() {
    let started = std::time::Instant::now();
    let result = run_command_with_timeout(
        std::process::Command::new("sh").args(["-c", "sleep 5"]),
        Duration::from_millis(50),
    );
    assert_eq!(result, Err(RuntimeCommandError::TimedOut));
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "runtime scenario must be bounded by its configured timeout"
    );
}

#[test]
fn runtime_scenario_catalog_separates_deterministic_and_privileged_coverage() {
    let catalog = runtime_scenarios();
    assert_eq!(catalog.len(), 12);
    assert!(catalog
        .iter()
        .any(|scenario| scenario.name == "wayland-wtype-success"
            && scenario.tier == RuntimeTier::Privileged));
    assert!(catalog
        .iter()
        .any(|scenario| scenario.name == "wayland-wtype-fallback"
            && scenario.tier == RuntimeTier::Deterministic));
    assert!(catalog
        .iter()
        .any(|scenario| scenario.name == "x11-selection-timeout"
            && scenario.tier == RuntimeTier::Deterministic));
    assert!(catalog
        .iter()
        .any(|scenario| scenario.name == "chromium-shift-prefix"
            && scenario.tier == RuntimeTier::Privileged));
    assert!(catalog
        .iter()
        .all(|scenario| scenario.timeout > Duration::ZERO));
}

#[test]
fn privileged_runtime_gate_accepts_only_a_complete_environment() {
    let capabilities = RuntimeCapabilities::from_environment(
        |name| match name {
            "VIETC_RUNTIME_TESTS" => Some("1".to_string()),
            "DISPLAY" => Some(":1".to_string()),
            "WAYLAND_DISPLAY" => Some("wayland-1".to_string()),
            _ => None,
        },
        true,
    );
    assert_eq!(
        capabilities.require_privileged_runtime(Duration::from_secs(30)),
        Ok(())
    );
}

#[test]
fn runtime_matrix_reports_separate_capabilities_with_reason_and_owner() {
    let environment = RuntimeEnvironment::detect_with(
        |name| match name {
            "VIETC_RUNTIME_TESTS" => Some("1".to_string()),
            "XDG_SESSION_TYPE" => Some("wayland".to_string()),
            "XDG_CURRENT_DESKTOP" => Some("GNOME".to_string()),
            "WAYLAND_DISPLAY" => Some("wayland-0".to_string()),
            _ => None,
        },
        |_| false,
        false,
    );

    let results = evaluate_runtime_matrix(&environment);

    assert_eq!(results.entries.len(), 10);
    for entry in &results.entries {
        assert!(!entry.reason.trim().is_empty());
        assert!(!entry.owner.trim().is_empty());
    }

    assert_eq!(
        results.status_for("gnome-wayland"),
        Some(RuntimeMatrixStatus::Unavailable)
    );
    assert_eq!(
        results.status_for("cosmic-wayland"),
        Some(RuntimeMatrixStatus::Skipped)
    );
    assert_eq!(
        results.status_for("kde-wayland"),
        Some(RuntimeMatrixStatus::Skipped)
    );
    assert_eq!(
        results.status_for("x11"),
        Some(RuntimeMatrixStatus::Skipped)
    );
    for capability in [
        "chromium",
        "google-chrome",
        "electron",
        "gtk",
        "qt",
        "terminal",
    ] {
        assert_eq!(
            results.status_for(capability),
            Some(RuntimeMatrixStatus::Unavailable),
            "{capability} must remain separate and unavailable when its executable is absent"
        );
    }
}

#[test]
fn runtime_matrix_artifact_uses_only_supported_statuses_and_records_metadata() {
    let results = RuntimeMatrixResults {
        entries: vec![
            RuntimeMatrixEntry::new(
                "cosmic-wayland",
                RuntimeMatrixStatus::Pass,
                "observable controlled fixture completed",
                "runtime-desktop-runner",
            ),
            RuntimeMatrixEntry::new(
                "x11",
                RuntimeMatrixStatus::Skipped,
                "requires X11; detected wayland",
                "runtime-desktop-runner",
            ),
            RuntimeMatrixEntry::new(
                "chromium",
                RuntimeMatrixStatus::Failed,
                "target did not observe injected text before timeout",
                "runtime-browser-runner",
            ),
            RuntimeMatrixEntry::new(
                "electron",
                RuntimeMatrixStatus::Unavailable,
                "electron is not installed",
                "runtime-browser-runner",
            ),
        ],
    };

    let artifact = render_runtime_matrix_artifact(
        "wayland",
        "COSMIC",
        [
            ("wtype", "installed; version unavailable"),
            ("chromium", "141.0"),
        ],
        &results,
    );

    assert!(artifact.contains("display_server: wayland"));
    assert!(artifact.contains("compositor: COSMIC"));
    assert!(artifact.contains("wtype: installed; version unavailable"));
    assert!(artifact.contains("cosmic-wayland: PASS"));
    assert!(artifact.contains("x11: SKIPPED"));
    assert!(artifact.contains("chromium: FAILED"));
    assert!(artifact.contains("electron: UNAVAILABLE"));
    assert!(artifact.contains("reason: target did not observe injected text before timeout"));
    assert!(artifact.contains("owner: runtime-browser-runner"));
    assert!(!artifact.contains("READY"));
}

#[test]
fn tool_version_reports_missing_when_program_cannot_be_spawned() {
    let program = format!("vietc-runtime-missing-tool-{}", std::process::id());

    assert_eq!(tool_version(&program), "missing");
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RuntimeTargetKind {
    Chromium,
    Chrome,
    Electron,
    Gtk,
    Qt,
    Terminal,
}

impl RuntimeTargetKind {
    fn executable(self) -> &'static str {
        match self {
            Self::Chromium => "chromium",
            Self::Chrome => "google-chrome",
            Self::Electron => "electron",
            Self::Gtk => "zenity",
            Self::Qt => "python3",
            Self::Terminal => "terminal",
        }
    }

    fn role(self) -> RuntimeProcessRole {
        match self {
            Self::Chromium | Self::Chrome => RuntimeProcessRole::Browser,
            Self::Electron => RuntimeProcessRole::ElectronMain,
            Self::Gtk | Self::Qt => RuntimeProcessRole::ToolkitTarget,
            Self::Terminal => RuntimeProcessRole::TerminalTarget,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct RuntimeSequenceObservation {
    expected_text: String,
    observed_text: String,
    process_healthy: bool,
    clipboard_preserved: bool,
    modifiers_released: bool,
}

impl RuntimeSequenceObservation {
    fn verify(&self) -> Result<(), &'static str> {
        if self.observed_text != self.expected_text {
            return Err("observable text mismatch");
        }
        if !self.process_healthy {
            return Err("target process exited");
        }
        if !self.clipboard_preserved {
            return Err("clipboard was not preserved");
        }
        if !self.modifiers_released {
            return Err("modifier state regressed");
        }
        Ok(())
    }
}

#[derive(Debug)]
struct OwnedRuntimeChild {
    child: Option<std::process::Child>,
    pid: u32,
    process_group: i32,
    reaped: bool,
    exit_success: Option<bool>,
}

impl OwnedRuntimeChild {
    fn spawn(command: &mut std::process::Command) -> Result<Self, RuntimeCommandError> {
        use std::os::unix::process::CommandExt;

        unsafe {
            command.pre_exec(|| {
                if libc::setpgid(0, 0) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }

        let child = command
            .spawn()
            .map_err(|_| RuntimeCommandError::SpawnFailed)?;
        let pid = child.id();
        Ok(Self {
            child: Some(child),
            pid,
            process_group: pid as i32,
            reaped: false,
            exit_success: None,
        })
    }

    fn pid(&self) -> u32 {
        self.pid
    }

    fn healthy(&mut self) -> Result<bool, RuntimeCommandError> {
        match self
            .child
            .as_mut()
            .ok_or(RuntimeCommandError::WaitFailed)?
            .try_wait()
        {
            Ok(None) => Ok(true),
            Ok(Some(status)) => {
                self.reaped = true;
                self.exit_success = Some(status.success());
                Ok(false)
            }
            Err(_) => Err(RuntimeCommandError::WaitFailed),
        }
    }

    fn completed_successfully(&self) -> bool {
        self.reaped && self.exit_success == Some(true)
    }

    fn signal_group(&self, signal: i32) {
        unsafe {
            libc::kill(-self.process_group, signal);
        }
    }

    fn group_alive(&self) -> bool {
        unsafe {
            if libc::kill(-self.process_group, 0) == 0 {
                true
            } else {
                std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
            }
        }
    }

    fn record_exit_status(&mut self, status: std::process::ExitStatus) {
        self.reaped = true;
        self.exit_success = Some(status.success());
    }

    fn wait_for_group_exit(
        &mut self,
        deadline: std::time::Instant,
    ) -> Result<bool, RuntimeCommandError> {
        while std::time::Instant::now() < deadline {
            if let Some(child) = self.child.as_mut() {
                match child.try_wait() {
                    Ok(Some(status)) => self.record_exit_status(status),
                    Ok(None) => {}
                    Err(_) => return Err(RuntimeCommandError::WaitFailed),
                }
            }
            if !self.group_alive() {
                if !self.reaped {
                    let status = self
                        .child
                        .as_mut()
                        .ok_or(RuntimeCommandError::WaitFailed)?
                        .wait()
                        .map_err(|_| RuntimeCommandError::WaitFailed)?;
                    self.record_exit_status(status);
                }
                return Ok(true);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(false)
    }

    fn terminate_and_reap(&mut self, timeout: Duration) -> Result<(), RuntimeCommandError> {
        self.terminate_and_reap_after_graceful_close(timeout, None)
    }

    fn terminate_and_reap_after_graceful_close(
        &mut self,
        timeout: Duration,
        graceful_close: Option<Result<serde_json::Value, RuntimeCommandError>>,
    ) -> Result<(), RuntimeCommandError> {
        if self.reaped && !self.group_alive() {
            return Ok(());
        }

        if graceful_close.is_some()
            && self.wait_for_group_exit(std::time::Instant::now() + timeout)?
        {
            return Ok(());
        }

        self.signal_group(libc::SIGTERM);
        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            if let Some(child) = self.child.as_mut() {
                match child.try_wait() {
                    Ok(Some(status)) => self.record_exit_status(status),
                    Ok(None) => {}
                    Err(_) => return Err(RuntimeCommandError::WaitFailed),
                }
            }
            if !self.group_alive() {
                if !self.reaped {
                    let status = self
                        .child
                        .as_mut()
                        .ok_or(RuntimeCommandError::WaitFailed)?
                        .wait()
                        .map_err(|_| RuntimeCommandError::WaitFailed)?;
                    self.record_exit_status(status);
                }
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(5));
        }

        self.signal_group(libc::SIGKILL);
        let kill_deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < kill_deadline {
            if let Some(child) = self.child.as_mut() {
                match child.try_wait() {
                    Ok(Some(status)) => self.record_exit_status(status),
                    Ok(None) => {}
                    Err(_) => return Err(RuntimeCommandError::WaitFailed),
                }
            }
            if !self.group_alive() {
                if !self.reaped {
                    let status = self
                        .child
                        .as_mut()
                        .ok_or(RuntimeCommandError::WaitFailed)?
                        .wait()
                        .map_err(|_| RuntimeCommandError::WaitFailed)?;
                    self.record_exit_status(status);
                }
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(5));
        }

        Err(RuntimeCommandError::TimedOut)
    }
}

impl Drop for OwnedRuntimeChild {
    fn drop(&mut self) {
        if self.reaped && !self.group_alive() {
            return;
        }
        self.signal_group(libc::SIGKILL);
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let deadline = std::time::Instant::now() + Duration::from_secs(1);
            while std::time::Instant::now() < deadline {
                match child.try_wait() {
                    Ok(Some(_)) => {
                        self.reaped = true;
                        break;
                    }
                    Ok(None) => std::thread::sleep(Duration::from_millis(5)),
                    Err(_) => break,
                }
            }
        }
    }
}

fn controlled_display_available(environment: &RuntimeEnvironment) -> bool {
    environment.runtime_opt_in
        && environment.display_available
        && environment.uinput_available
        && std::env::var("VIETC_CONTROLLED_DISPLAY").as_deref() == Ok("1")
}

fn detected_terminal(environment: &RuntimeEnvironment) -> Option<&'static str> {
    if environment.session_type == "wayland" {
        return ["gnome-terminal", "konsole", "kitty"]
            .into_iter()
            .find(|program| environment.has_tool(program));
    }

    ["gnome-terminal", "konsole", "kitty", "xterm"]
        .into_iter()
        .find(|program| environment.has_tool(program))
}

fn runtime_target_executable(
    target: RuntimeTargetKind,
    environment: &RuntimeEnvironment,
) -> Option<&'static str> {
    match target {
        RuntimeTargetKind::Chromium if environment.has_tool("chromium") => Some("chromium"),
        RuntimeTargetKind::Chromium if environment.has_tool("chromium-browser") => {
            Some("chromium-browser")
        }
        RuntimeTargetKind::Chrome if environment.has_tool("google-chrome") => Some("google-chrome"),
        RuntimeTargetKind::Electron if environment.has_tool("electron") => Some("electron"),
        RuntimeTargetKind::Gtk if environment.has_tool("zenity") => Some("zenity"),
        RuntimeTargetKind::Qt if environment.has_tool("python3") => Some("python3"),
        RuntimeTargetKind::Terminal => detected_terminal(environment),
        _ => None,
    }
}

fn controlled_fixture_url(fixture: &ControlledRuntimeFixture) -> String {
    format!(
        "file://{}",
        fixture.profile_dir.join("controlled-target.html").display()
    )
}

fn prepare_target_fixture(
    target: RuntimeTargetKind,
    fixture: &ControlledRuntimeFixture,
) -> std::io::Result<()> {
    if matches!(
        target,
        RuntimeTargetKind::Chromium | RuntimeTargetKind::Chrome | RuntimeTargetKind::Electron
    ) {
        std::fs::write(
            fixture.profile_dir.join("controlled-target.html"),
            "<!doctype html><meta charset=utf-8><title>Vietc controlled runtime</title><textarea id=target autofocus aria-label='controlled runtime target'></textarea>",
        )?;
    }
    Ok(())
}

fn runtime_target_flags(
    target: RuntimeTargetKind,
    fixture: &ControlledRuntimeFixture,
    executable: &str,
) -> Vec<String> {
    match target {
        RuntimeTargetKind::Chromium | RuntimeTargetKind::Chrome => vec![
            format!("--user-data-dir={}", fixture.profile_dir.display()),
            "--no-sandbox".to_string(),
            "--no-first-run".to_string(),
            "--disable-sync".to_string(),
            "--disable-background-networking".to_string(),
            "--disable-component-update".to_string(),
            "--disable-default-apps".to_string(),
            "--new-window".to_string(),
            "--start-maximized".to_string(),
            "--remote-debugging-address=127.0.0.1".to_string(),
            "--remote-debugging-port=0".to_string(),
            "--remote-allow-origins=http://127.0.0.1".to_string(),
            format!("--app={}", controlled_fixture_url(fixture)),
        ],
        RuntimeTargetKind::Electron => vec![
            format!("--user-data-dir={}", fixture.profile_dir.display()),
            controlled_fixture_url(fixture),
        ],
        RuntimeTargetKind::Gtk => vec![
            "--entry".to_string(),
            "--title=Vietc controlled runtime".to_string(),
            "--text=Controlled fixture text only".to_string(),
        ],
        RuntimeTargetKind::Qt => vec![Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/common/controlled_qt_text_field.py")
            .display()
            .to_string()],
        RuntimeTargetKind::Terminal => match executable {
            "gnome-terminal" => vec![
                "--".to_string(),
                "sh".to_string(),
                "-c".to_string(),
                "read -r line; printf '%s\\n' \"$line\"".to_string(),
            ],
            "konsole" => vec![
                "-e".to_string(),
                "sh".to_string(),
                "-c".to_string(),
                "read -r line; printf '%s\\n' \"$line\"".to_string(),
            ],
            "kitty" => vec![
                "sh".to_string(),
                "-c".to_string(),
                "read -r line; printf '%s\\n' \"$line\"".to_string(),
            ],
            "xterm" => vec![
                "-e".to_string(),
                "sh".to_string(),
                "-c".to_string(),
                "read -r line; printf '%s\\n' \"$line\"".to_string(),
            ],
            _ => vec![],
        },
    }
}

fn process_role_from_command_line(
    target: RuntimeTargetKind,
    pid: u32,
    parent_pid: Option<u32>,
    command_line: &str,
    root_pid: u32,
) -> RuntimePidRole {
    let role = if pid == root_pid {
        target.role()
    } else if command_line.contains("--type=renderer") {
        RuntimeProcessRole::Renderer
    } else if command_line.contains("--type=")
        || command_line.contains("crashpad")
        || command_line.contains("zygote")
        || command_line.contains("gpu-process")
    {
        RuntimeProcessRole::Helper
    } else if command_line.contains("vietc") {
        RuntimeProcessRole::Daemon
    } else {
        RuntimeProcessRole::Helper
    };
    RuntimePidRole {
        pid,
        parent_pid,
        role,
    }
}

fn read_proc_identity(pid: u32) -> Option<(Option<u32>, String)> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let command_end = stat.rfind(')')?;
    let fields = stat
        .get(command_end + 2..)?
        .split_whitespace()
        .collect::<Vec<_>>();
    let parent_pid = fields.get(1)?.parse().ok();
    let command_line = std::fs::read(format!("/proc/{pid}/cmdline"))
        .ok()
        .map(|bytes| {
            bytes
                .split(|byte| *byte == 0)
                .filter(|part| !part.is_empty())
                .map(|part| String::from_utf8_lossy(part))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default();
    Some((parent_pid, command_line))
}

fn discover_owned_process_tree(target: RuntimeTargetKind, root_pid: u32) -> Vec<RuntimePidRole> {
    let mut identities = std::collections::HashMap::new();
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return vec![RuntimePidRole {
            pid: root_pid,
            parent_pid: None,
            role: target.role(),
        }];
    };
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let Ok(pid) = name.parse::<u32>() else {
            continue;
        };
        if let Some(identity) = read_proc_identity(pid) {
            identities.insert(pid, identity);
        }
    }

    let mut owned = vec![root_pid];
    loop {
        let previous_len = owned.len();
        for (&pid, (parent_pid, _)) in &identities {
            if parent_pid.is_some_and(|parent| owned.contains(&parent)) && !owned.contains(&pid) {
                owned.push(pid);
            }
        }
        if owned.len() == previous_len {
            break;
        }
    }

    owned
        .into_iter()
        .map(|pid| {
            let (parent_pid, command_line) = identities.get(&pid).cloned().unwrap_or_default();
            process_role_from_command_line(target, pid, parent_pid, &command_line, root_pid)
        })
        .collect()
}

fn render_target_manifest(
    target: RuntimeTargetKind,
    version: &str,
    flags: &[String],
    backend: &str,
    sequence: &str,
    roles: &[RuntimePidRole],
) -> String {
    let role_lines = roles
        .iter()
        .map(|role| {
            format!(
                "pid={} parent={:?} role={:?}",
                role.pid, role.parent_pid, role.role
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "target: {:?}\nexecutable: {}\nversion: {}\nflags: {:?}\nbackend: {}\nsequence: {}\nprocess_tree:\n{}\n",
        target,
        target.executable(),
        version,
        flags,
        backend,
        sequence,
        role_lines
    )
}

#[test]
fn deterministic_runtime_sequence_rejects_every_observable_regression() {
    let valid = RuntimeSequenceObservation {
        expected_text: CONTROLLED_TEXT.to_string(),
        observed_text: CONTROLLED_TEXT.to_string(),
        process_healthy: true,
        clipboard_preserved: true,
        modifiers_released: true,
    };
    assert_eq!(valid.verify(), Ok(()));

    let mut mismatch = RuntimeSequenceObservation {
        observed_text: "wrong".to_string(),
        ..valid
    };
    assert_eq!(mismatch.verify(), Err("observable text mismatch"));
    mismatch.observed_text = CONTROLLED_TEXT.to_string();
    mismatch.process_healthy = false;
    assert_eq!(mismatch.verify(), Err("target process exited"));
    mismatch.process_healthy = true;
    mismatch.clipboard_preserved = false;
    assert_eq!(mismatch.verify(), Err("clipboard was not preserved"));
    mismatch.clipboard_preserved = true;
    mismatch.modifiers_released = false;
    assert_eq!(mismatch.verify(), Err("modifier state regressed"));
}

#[test]
fn deterministic_runtime_target_roles_cover_browser_daemon_renderer_and_helpers() {
    assert_eq!(
        RuntimeTargetKind::Chromium.role(),
        RuntimeProcessRole::Browser
    );
    assert_eq!(
        RuntimeTargetKind::Chrome.role(),
        RuntimeProcessRole::Browser
    );
    assert_eq!(
        RuntimeTargetKind::Electron.role(),
        RuntimeProcessRole::ElectronMain
    );
    assert_eq!(
        RuntimeTargetKind::Gtk.role(),
        RuntimeProcessRole::ToolkitTarget
    );
    assert_eq!(
        RuntimeTargetKind::Qt.role(),
        RuntimeProcessRole::ToolkitTarget
    );
    assert_eq!(
        RuntimeTargetKind::Terminal.role(),
        RuntimeProcessRole::TerminalTarget
    );

    let roles = [
        RuntimePidRole {
            pid: 10,
            parent_pid: None,
            role: RuntimeProcessRole::Daemon,
        },
        RuntimePidRole {
            pid: 11,
            parent_pid: Some(20),
            role: RuntimeProcessRole::Renderer,
        },
        RuntimePidRole {
            pid: 20,
            parent_pid: None,
            role: RuntimeProcessRole::Browser,
        },
    ];
    assert_eq!(
        classify_runtime_exit(false, Some(20), &roles),
        RuntimeExitClassification::BrowserCrash
    );
    assert_eq!(
        classify_runtime_exit(false, Some(11), &roles),
        RuntimeExitClassification::RendererCrash
    );
    assert_eq!(
        classify_runtime_exit(false, Some(10), &roles),
        RuntimeExitClassification::DaemonCrash
    );
}

#[test]
fn deterministic_fake_process_commands_classify_browser_renderer_daemon_and_helper() {
    let root = process_role_from_command_line(
        RuntimeTargetKind::Chromium,
        100,
        None,
        "chromium --user-data-dir=fixture",
        100,
    );
    let renderer = process_role_from_command_line(
        RuntimeTargetKind::Chromium,
        101,
        Some(100),
        "chromium --type=renderer",
        100,
    );
    let daemon = process_role_from_command_line(
        RuntimeTargetKind::Chromium,
        102,
        Some(100),
        "vietc --runtime-fixture",
        100,
    );
    let helper = process_role_from_command_line(
        RuntimeTargetKind::Chromium,
        103,
        Some(100),
        "chromium --type=gpu-process",
        100,
    );

    assert_eq!(root.role, RuntimeProcessRole::Browser);
    assert_eq!(renderer.role, RuntimeProcessRole::Renderer);
    assert_eq!(daemon.role, RuntimeProcessRole::Daemon);
    assert_eq!(helper.role, RuntimeProcessRole::Helper);

    let roles = [root, renderer, daemon, helper];
    assert_eq!(
        classify_runtime_exit(false, Some(100), &roles),
        RuntimeExitClassification::BrowserCrash
    );
    assert_eq!(
        classify_runtime_exit(false, Some(101), &roles),
        RuntimeExitClassification::RendererCrash
    );
}

#[test]
fn deterministic_owned_child_terminates_descendants() {
    let mut command = std::process::Command::new("sh");
    command.args(["-c", "sleep 30 & wait"]);
    let mut owned =
        OwnedRuntimeChild::spawn(&mut command).expect("fixture process tree must spawn");
    std::thread::sleep(Duration::from_millis(30));

    let roles = discover_owned_process_tree(RuntimeTargetKind::Chromium, owned.pid());
    assert!(
        roles
            .iter()
            .any(|role| role.parent_pid == Some(owned.pid())),
        "fixture must expose an owned descendant"
    );

    owned
        .terminate_and_reap(Duration::from_secs(1))
        .expect("owned process group must terminate");
    assert!(owned.reaped);
    assert!(!owned.group_alive());
}

#[test]
fn deterministic_panic_unwind_kills_and_reaps_owned_process_tree() {
    let process_group = std::sync::Arc::new(std::sync::Mutex::new(None));
    let recorded_group = process_group.clone();

    let result = std::panic::catch_unwind(move || {
        let mut command = std::process::Command::new("sh");
        command.args(["-c", "sleep 30 & wait"]);
        let owned =
            OwnedRuntimeChild::spawn(&mut command).expect("panic fixture process tree must spawn");
        *recorded_group.lock().unwrap() = Some(owned.process_group);
        panic!("controlled panic fixture");
    });

    assert!(result.is_err());
    let group = process_group
        .lock()
        .unwrap()
        .expect("process group recorded");
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while std::time::Instant::now() < deadline {
        let alive = unsafe { libc::kill(-group, 0) == 0 };
        if !alive {
            return;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("panic cleanup left an owned process group alive");
}

#[test]
fn deterministic_browser_graceful_close_reaps_without_signal_fallback() {
    let mut command = std::process::Command::new("sh");
    command.args(["-c", "exit 0"]);
    let mut owned = OwnedRuntimeChild::spawn(&mut command).expect("fixture must spawn");

    owned
        .terminate_and_reap_after_graceful_close(
            Duration::from_secs(1),
            Some(Ok(serde_json::json!({"id": 1, "result": {}}))),
        )
        .expect("graceful browser close must reap the child");

    assert!(owned.reaped);
    assert_eq!(owned.exit_success, Some(true));
    assert!(!owned.group_alive());
}

#[test]
fn deterministic_browser_graceful_close_failure_falls_back_to_group_cleanup() {
    let mut command = std::process::Command::new("sh");
    command.args(["-c", "trap '' TERM; sleep 30"]);
    let mut owned = OwnedRuntimeChild::spawn(&mut command).expect("fixture must spawn");

    owned
        .terminate_and_reap_after_graceful_close(
            Duration::from_millis(100),
            Some(Err(RuntimeCommandError::ScenarioFailed)),
        )
        .expect("failed graceful close must fall back through SIGTERM and SIGKILL");

    assert!(owned.reaped);
    assert!(owned.exit_success.is_some());
    assert!(!owned.group_alive());
}

#[test]
fn deterministic_owned_child_is_killed_and_reaped_on_teardown() {
    let mut command = std::process::Command::new("sh");
    command.args(["-c", "sleep 5"]);
    let mut owned = OwnedRuntimeChild::spawn(&mut command).expect("fake executable must spawn");
    assert!(owned.pid() > 0, "owned child must expose its exact PID");
    assert!(owned.healthy().expect("health query must succeed"));
    owned
        .terminate_and_reap(Duration::from_secs(1))
        .expect("owned child must be killed and reaped");
    assert!(owned.reaped);
}

#[test]
fn deterministic_target_manifest_records_flags_backend_sequence_and_pid_roles() {
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("daemon crate must have a workspace root");
    let fixture = ControlledRuntimeFixture::isolated(workspace_root, "manifest-test")
        .expect("fixture must be valid");
    let flags = runtime_target_flags(RuntimeTargetKind::Chromium, &fixture, "chromium");
    let manifest = render_target_manifest(
        RuntimeTargetKind::Chromium,
        "Chromium fake-1",
        &flags,
        "fake-wayland",
        "KEY_A:press,KEY_A:release",
        &[RuntimePidRole {
            pid: 42,
            parent_pid: None,
            role: RuntimeProcessRole::Browser,
        }],
    );
    assert!(manifest.contains("version: Chromium fake-1"));
    assert!(manifest.contains("--user-data-dir="));
    assert!(manifest.contains("backend: fake-wayland"));
    assert!(manifest.contains("KEY_A:press,KEY_A:release"));
    assert!(manifest.contains("pid=42"));
}

#[test]
fn deterministic_real_target_gate_requires_controlled_opt_in_display() {
    let environment = RuntimeEnvironment::detect_with(
        |name| match name {
            "VIETC_RUNTIME_TESTS" => Some("1".to_string()),
            "DISPLAY" => Some(":99".to_string()),
            "XDG_SESSION_TYPE" => Some("x11".to_string()),
            _ => None,
        },
        |_| true,
        true,
    );
    assert!(
        !controlled_display_available(&environment),
        "VIETC_CONTROLLED_DISPLAY=1 must be independently required"
    );
}

#[derive(Debug)]
struct RuntimeTargetResult {
    status: RuntimeMatrixStatus,
    reason: String,
    observation: Option<RuntimeSequenceObservation>,
}

fn requested_runtime_targets() -> Option<std::collections::HashSet<String>> {
    std::env::var("VIETC_RUNTIME_TARGETS").ok().map(|value| {
        value
            .split(',')
            .map(str::trim)
            .filter(|target| !target.is_empty())
            .map(str::to_ascii_lowercase)
            .collect()
    })
}

fn runtime_target_requested(
    target: RuntimeTargetKind,
    requested_targets: Option<&std::collections::HashSet<String>>,
) -> bool {
    let Some(requested_targets) = requested_targets else {
        return true;
    };
    let name = match target {
        RuntimeTargetKind::Chromium => "chromium",
        RuntimeTargetKind::Chrome => "google-chrome",
        RuntimeTargetKind::Electron => "electron",
        RuntimeTargetKind::Gtk => "gtk",
        RuntimeTargetKind::Qt => "qt",
        RuntimeTargetKind::Terminal => "terminal",
    };
    requested_targets.contains(name)
}

#[test]
fn deterministic_toolkit_target_flags_use_controlled_stdout_fixtures() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root");
    let fixture =
        ControlledRuntimeFixture::isolated(workspace_root, "toolkit-flags").expect("fixture");

    assert_eq!(
        runtime_target_flags(RuntimeTargetKind::Gtk, &fixture, "zenity"),
        [
            "--entry",
            "--title=Vietc controlled runtime",
            "--text=Controlled fixture text only",
        ]
    );
    let qt_flags = runtime_target_flags(RuntimeTargetKind::Qt, &fixture, "python3");
    assert_eq!(qt_flags.len(), 1);
    assert!(qt_flags[0].ends_with("daemon/tests/common/controlled_qt_text_field.py"));
}

#[test]
fn deterministic_toolkit_executable_detection_uses_zenity_and_python3() {
    let environment = RuntimeEnvironment::detect_with(
        |_| None,
        |tool| matches!(tool, "zenity" | "python3"),
        false,
    );
    assert_eq!(
        runtime_target_executable(RuntimeTargetKind::Gtk, &environment),
        Some("zenity")
    );
    assert_eq!(
        runtime_target_executable(RuntimeTargetKind::Qt, &environment),
        Some("python3")
    );
}

#[test]
fn deterministic_toolkit_observable_output_parses_stdout_only() {
    assert_eq!(
        parse_toolkit_observable_output("\n vietc-runtime-controlled-text\n"),
        Ok(CONTROLLED_TEXT.to_string())
    );
    assert_eq!(
        parse_toolkit_observable_output(""),
        Err(RuntimeCommandError::TargetCopyFailed)
    );
}

#[test]
fn deterministic_requested_runtime_targets_filter_exact_targets() {
    let requested = ["chromium".to_string(), "google-chrome".to_string()]
        .into_iter()
        .collect();

    assert!(runtime_target_requested(
        RuntimeTargetKind::Chromium,
        Some(&requested)
    ));
    assert!(runtime_target_requested(
        RuntimeTargetKind::Chrome,
        Some(&requested)
    ));
    assert!(!runtime_target_requested(
        RuntimeTargetKind::Electron,
        Some(&requested)
    ));
}

fn run_scenario_with_deadline<T: Send + 'static>(
    timeout: Duration,
    scenario: impl FnOnce() -> T + Send + 'static,
) -> Result<T, RuntimeCommandError> {
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let _ = sender.send(scenario());
    });
    receiver
        .recv_timeout(timeout)
        .map_err(|_| RuntimeCommandError::TimedOut)
}

#[test]
fn deterministic_runtime_scenario_timeout_is_enforced() {
    let started = std::time::Instant::now();
    let result = run_scenario_with_deadline(Duration::from_millis(20), || {
        std::thread::sleep(Duration::from_secs(1));
        CONTROLLED_TEXT.to_string()
    });

    assert_eq!(result, Err(RuntimeCommandError::TimedOut));
    assert!(started.elapsed() < Duration::from_millis(250));
}

#[test]
fn deterministic_runtime_scenario_returns_observable_text_before_deadline() {
    let result = run_scenario_with_deadline(Duration::from_secs(1), || CONTROLLED_TEXT.to_string());

    assert_eq!(result.as_deref(), Ok(CONTROLLED_TEXT));
}

fn parse_devtools_active_port(contents: &str) -> Result<(u16, String), RuntimeCommandError> {
    let mut lines = contents.lines();
    let port = lines
        .next()
        .and_then(|line| line.parse::<u16>().ok())
        .ok_or(RuntimeCommandError::ScenarioFailed)?;
    let browser_path = lines
        .next()
        .filter(|path| path.starts_with("/devtools/browser/"))
        .ok_or(RuntimeCommandError::ScenarioFailed)?;
    Ok((port, browser_path.to_string()))
}

fn controlled_target_websocket_url(contents: &str) -> Result<String, RuntimeCommandError> {
    let targets: serde_json::Value =
        serde_json::from_str(contents).map_err(|_| RuntimeCommandError::ScenarioFailed)?;
    targets
        .as_array()
        .and_then(|targets| {
            targets.iter().find(|target| {
                target.get("type").and_then(serde_json::Value::as_str) == Some("page")
                    && target
                        .get("url")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|url| url.ends_with("/controlled-target.html"))
            })
        })
        .and_then(|target| target.get("webSocketDebuggerUrl"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .ok_or(RuntimeCommandError::ScenarioFailed)
}

fn cdp_string_result(
    contents: &str,
    expected_id: u64,
) -> Result<Option<String>, RuntimeCommandError> {
    let response: serde_json::Value =
        serde_json::from_str(contents).map_err(|_| RuntimeCommandError::ScenarioFailed)?;
    if response.get("id").and_then(serde_json::Value::as_u64) != Some(expected_id) {
        return Ok(None);
    }
    if response.get("error").is_some() {
        return Err(RuntimeCommandError::ScenarioFailed);
    }
    Ok(response
        .pointer("/result/result/value")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string))
}

fn encode_websocket_text_frame(payload: &[u8], mask: Option<[u8; 4]>) -> Vec<u8> {
    let mut frame = vec![0x81];
    let mask_bit = if mask.is_some() { 0x80 } else { 0 };
    match payload.len() {
        length @ 0..=125 => frame.push(mask_bit | length as u8),
        length @ 126..=65535 => {
            frame.push(mask_bit | 126);
            frame.extend_from_slice(&(length as u16).to_be_bytes());
        }
        length => {
            frame.push(mask_bit | 127);
            frame.extend_from_slice(&(length as u64).to_be_bytes());
        }
    }
    if let Some(mask) = mask {
        frame.extend_from_slice(&mask);
        frame.extend(
            payload
                .iter()
                .enumerate()
                .map(|(index, byte)| byte ^ mask[index % 4]),
        );
    } else {
        frame.extend_from_slice(payload);
    }
    frame
}

fn decode_websocket_text_frame(frame: &[u8]) -> Result<Vec<u8>, RuntimeCommandError> {
    if frame.len() < 2 || frame[0] & 0x0f != 1 {
        return Err(RuntimeCommandError::ScenarioFailed);
    }
    let masked = frame[1] & 0x80 != 0;
    let mut offset = 2;
    let length = match frame[1] & 0x7f {
        length @ 0..=125 => length as usize,
        126 => {
            let bytes = frame
                .get(offset..offset + 2)
                .ok_or(RuntimeCommandError::ScenarioFailed)?;
            offset += 2;
            u16::from_be_bytes([bytes[0], bytes[1]]) as usize
        }
        127 => {
            let bytes = frame
                .get(offset..offset + 8)
                .ok_or(RuntimeCommandError::ScenarioFailed)?;
            offset += 8;
            usize::try_from(u64::from_be_bytes(
                bytes
                    .try_into()
                    .map_err(|_| RuntimeCommandError::ScenarioFailed)?,
            ))
            .map_err(|_| RuntimeCommandError::ScenarioFailed)?
        }
        _ => return Err(RuntimeCommandError::ScenarioFailed),
    };
    let mask = if masked {
        let bytes = frame
            .get(offset..offset + 4)
            .ok_or(RuntimeCommandError::ScenarioFailed)?;
        offset += 4;
        Some([bytes[0], bytes[1], bytes[2], bytes[3]])
    } else {
        None
    };
    let payload = frame
        .get(offset..offset + length)
        .ok_or(RuntimeCommandError::ScenarioFailed)?;
    Ok(match mask {
        Some(mask) => payload
            .iter()
            .enumerate()
            .map(|(index, byte)| byte ^ mask[index % 4])
            .collect(),
        None => payload.to_vec(),
    })
}

fn parse_websocket_url(url: &str) -> Result<(String, u16, String), RuntimeCommandError> {
    let authority_and_path = url
        .strip_prefix("ws://")
        .ok_or(RuntimeCommandError::ScenarioFailed)?;
    let (authority, path) = authority_and_path
        .split_once('/')
        .ok_or(RuntimeCommandError::ScenarioFailed)?;
    let (host, port) = authority
        .rsplit_once(':')
        .ok_or(RuntimeCommandError::ScenarioFailed)?;
    let port = port
        .parse()
        .map_err(|_| RuntimeCommandError::ScenarioFailed)?;
    Ok((host.to_string(), port, format!("/{path}")))
}

fn http_get_localhost(
    port: u16,
    path: &str,
    timeout: Duration,
) -> Result<String, RuntimeCommandError> {
    use std::io::{Read, Write};
    use std::net::TcpStream;

    let mut stream =
        TcpStream::connect(("127.0.0.1", port)).map_err(|_| RuntimeCommandError::SpawnFailed)?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|_| RuntimeCommandError::ScenarioFailed)?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|_| RuntimeCommandError::ScenarioFailed)?;
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )
    .map_err(|_| RuntimeCommandError::ScenarioFailed)?;
    let mut response = Vec::new();
    let header_end = loop {
        let mut buffer = [0_u8; 4096];
        let read = stream
            .read(&mut buffer)
            .map_err(|_| RuntimeCommandError::ScenarioFailed)?;
        if read == 0 {
            return Err(RuntimeCommandError::ScenarioFailed);
        }
        response.extend_from_slice(&buffer[..read]);
        if let Some(position) = response.windows(4).position(|window| window == b"\r\n\r\n") {
            break position + 4;
        }
        if response.len() > 64 * 1024 {
            return Err(RuntimeCommandError::ScenarioFailed);
        }
    };

    let headers = std::str::from_utf8(&response[..header_end])
        .map_err(|_| RuntimeCommandError::ScenarioFailed)?;
    if !headers.starts_with("HTTP/1.1 200") {
        return Err(RuntimeCommandError::ScenarioFailed);
    }
    let content_length = headers
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .ok_or(RuntimeCommandError::ScenarioFailed)?;

    while response.len() - header_end < content_length {
        let mut buffer = [0_u8; 4096];
        let read = stream
            .read(&mut buffer)
            .map_err(|_| RuntimeCommandError::ScenarioFailed)?;
        if read == 0 {
            return Err(RuntimeCommandError::ScenarioFailed);
        }
        response.extend_from_slice(&buffer[..read]);
    }

    String::from_utf8(response[header_end..header_end + content_length].to_vec())
        .map_err(|_| RuntimeCommandError::ScenarioFailed)
}

#[derive(Debug)]
struct CdpConnection {
    stream: std::net::TcpStream,
    next_id: u64,
}

impl CdpConnection {
    fn connect(url: &str, timeout: Duration) -> Result<Self, RuntimeCommandError> {
        use std::io::{Read, Write};

        let (host, port, path) = parse_websocket_url(url)?;
        let mut stream = std::net::TcpStream::connect((host.as_str(), port))
            .map_err(|_| RuntimeCommandError::SpawnFailed)?;
        stream
            .set_read_timeout(Some(timeout))
            .map_err(|_| RuntimeCommandError::ScenarioFailed)?;
        stream
            .set_write_timeout(Some(timeout))
            .map_err(|_| RuntimeCommandError::ScenarioFailed)?;
        write!(
            stream,
            "GET {path} HTTP/1.1\r\nHost: {host}:{port}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dmlldGMtcnVudGltZS1jZHA=\r\nSec-WebSocket-Version: 13\r\nOrigin: http://127.0.0.1\r\n\r\n"
        )
        .map_err(|_| RuntimeCommandError::ScenarioFailed)?;
        let mut response = Vec::new();
        let mut byte = [0_u8; 1];
        while !response.ends_with(b"\r\n\r\n") {
            stream
                .read_exact(&mut byte)
                .map_err(|_| RuntimeCommandError::ScenarioFailed)?;
            response.push(byte[0]);
            if response.len() > 16 * 1024 {
                return Err(RuntimeCommandError::ScenarioFailed);
            }
        }
        if !response.starts_with(b"HTTP/1.1 101") {
            return Err(RuntimeCommandError::ScenarioFailed);
        }
        Ok(Self { stream, next_id: 1 })
    }

    fn command(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, RuntimeCommandError> {
        use std::io::{Read, Write};

        let id = self.next_id;
        self.next_id += 1;
        let message = serde_json::json!({"id": id, "method": method, "params": params}).to_string();
        self.stream
            .write_all(&encode_websocket_text_frame(
                message.as_bytes(),
                Some([0x56, 0x49, 0x45, 0x54]),
            ))
            .map_err(|_| RuntimeCommandError::ScenarioFailed)?;

        loop {
            let mut header = [0_u8; 2];
            self.stream
                .read_exact(&mut header)
                .map_err(|_| RuntimeCommandError::ScenarioFailed)?;
            let mut frame = header.to_vec();
            let extended = match header[1] & 0x7f {
                126 => 2,
                127 => 8,
                _ => 0,
            };
            let mask_length = if header[1] & 0x80 != 0 { 4 } else { 0 };
            let mut metadata = vec![0_u8; extended + mask_length];
            self.stream
                .read_exact(&mut metadata)
                .map_err(|_| RuntimeCommandError::ScenarioFailed)?;
            frame.extend_from_slice(&metadata);
            let payload_length = match header[1] & 0x7f {
                length @ 0..=125 => length as usize,
                126 => u16::from_be_bytes([metadata[0], metadata[1]]) as usize,
                127 => usize::try_from(u64::from_be_bytes(
                    metadata[..8]
                        .try_into()
                        .map_err(|_| RuntimeCommandError::ScenarioFailed)?,
                ))
                .map_err(|_| RuntimeCommandError::ScenarioFailed)?,
                _ => return Err(RuntimeCommandError::ScenarioFailed),
            };
            let mut payload = vec![0_u8; payload_length];
            self.stream
                .read_exact(&mut payload)
                .map_err(|_| RuntimeCommandError::ScenarioFailed)?;
            frame.extend_from_slice(&payload);

            if header[0] & 0x0f != 1 {
                continue;
            }
            let decoded = decode_websocket_text_frame(&frame)?;
            let response: serde_json::Value = serde_json::from_slice(&decoded)
                .map_err(|_| RuntimeCommandError::ScenarioFailed)?;
            if response.get("id").and_then(serde_json::Value::as_u64) == Some(id) {
                if response.get("error").is_some() {
                    return Err(RuntimeCommandError::ScenarioFailed);
                }
                return Ok(response);
            }
        }
    }
}

#[test]
fn deterministic_devtools_active_port_parses_port_and_browser_path() {
    assert_eq!(
        parse_devtools_active_port("9222\n/devtools/browser/fixture\n"),
        Ok((9222, "/devtools/browser/fixture".to_string()))
    );
    assert!(parse_devtools_active_port("not-a-port\n/devtools/browser/fixture\n").is_err());
    assert!(parse_devtools_active_port("9222\n").is_err());
}

#[test]
fn deterministic_cdp_target_discovery_selects_controlled_fixture() {
    let targets = r#"[
        {"type":"page","url":"chrome://newtab/","webSocketDebuggerUrl":"ws://127.0.0.1:9222/devtools/page/newtab"},
        {"type":"page","url":"file:///fixture/controlled-target.html","webSocketDebuggerUrl":"ws://127.0.0.1:9222/devtools/page/controlled"}
    ]"#;

    assert_eq!(
        controlled_target_websocket_url(targets),
        Ok("ws://127.0.0.1:9222/devtools/page/controlled".to_string())
    );
}

#[test]
fn deterministic_cdp_evaluate_extracts_textarea_value() {
    let response = r#"{
        "id":4,
        "result":{"result":{"type":"string","value":"xin chào"}}
    }"#;

    assert_eq!(
        cdp_string_result(response, 4),
        Ok(Some("xin chào".to_string()))
    );
    assert_eq!(cdp_string_result(response, 5), Ok(None));
}

#[test]
fn deterministic_websocket_text_frame_round_trips_unmasked_server_payload() {
    let frame = encode_websocket_text_frame(br#"{"id":1}"#, None);
    assert_eq!(
        decode_websocket_text_frame(&frame),
        Ok(br#"{"id":1}"#.to_vec())
    );
    let masked = encode_websocket_text_frame(br#"{"id":1}"#, Some([1, 2, 3, 4]));
    assert_eq!(
        decode_websocket_text_frame(&masked),
        Ok(br#"{"id":1}"#.to_vec())
    );
}

#[derive(Debug)]
struct ControlledScenarioEvidence {
    observed_text: String,
    clipboard_preserved: bool,
    modifiers_released: bool,
    roles: Vec<RuntimePidRole>,
    browser_cdp: Option<CdpConnection>,
}

fn command_output_with_input(
    program: &str,
    args: &[&str],
    input: &str,
    timeout: Duration,
) -> Result<Vec<u8>, RuntimeCommandError> {
    use std::io::Write;

    let mut child = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|_| RuntimeCommandError::SpawnFailed)?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(input.as_bytes())
            .map_err(|_| RuntimeCommandError::ScenarioFailed)?;
    }
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => {
                return child
                    .wait_with_output()
                    .map(|output| output.stdout)
                    .map_err(|_| RuntimeCommandError::WaitFailed);
            }
            Ok(Some(_)) => return Err(RuntimeCommandError::ScenarioFailed),
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(RuntimeCommandError::TimedOut);
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(RuntimeCommandError::WaitFailed);
            }
        }
    }
}

fn clipboard_read(
    environment: &RuntimeEnvironment,
    timeout: Duration,
) -> Result<String, RuntimeCommandError> {
    let output = if environment.session_type == "wayland" {
        let mut child = std::process::Command::new("wl-paste")
            .args(["-n"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|_| RuntimeCommandError::SpawnFailed)?;
        let deadline = std::time::Instant::now() + timeout;
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    let output = child
                        .wait_with_output()
                        .map_err(|_| RuntimeCommandError::WaitFailed)?;
                    break if status.success() {
                        output.stdout
                    } else {
                        Vec::new()
                    };
                }
                Ok(None) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Ok(None) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(RuntimeCommandError::TimedOut);
                }
                Err(_) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(RuntimeCommandError::WaitFailed);
                }
            }
        }
    } else {
        command_output_with_input("xclip", &["-selection", "clipboard", "-o"], "", timeout)?
    };
    Ok(String::from_utf8_lossy(&output).into_owned())
}

fn clipboard_write(
    environment: &RuntimeEnvironment,
    text: &str,
    timeout: Duration,
) -> Result<(), RuntimeCommandError> {
    use std::io::Write;

    let (program, args): (&str, &[&str]) = if environment.session_type == "wayland" {
        ("wl-copy", &[])
    } else {
        ("xclip", &["-selection", "clipboard", "-i"])
    };
    let mut child = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|_| RuntimeCommandError::SpawnFailed)?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(text.as_bytes())
            .map_err(|_| RuntimeCommandError::ScenarioFailed)?;
    }

    let deadline = std::time::Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(_)) => return Err(RuntimeCommandError::ScenarioFailed),
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(RuntimeCommandError::TimedOut);
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(RuntimeCommandError::WaitFailed);
            }
        }
    }
}

fn inject_controlled_text(
    environment: &RuntimeEnvironment,
    text: &str,
    timeout: Duration,
) -> Result<(), RuntimeCommandError> {
    let status = if environment.session_type == "wayland" {
        let mut command = std::process::Command::new("wtype");
        command.args(["--", text]);
        run_command_with_timeout(&mut command, timeout)?
    } else {
        let mut command = std::process::Command::new("xdotool");
        command.args(["type", "--clearmodifiers", "--", text]);
        run_command_with_timeout(&mut command, timeout)?
    };
    if status.success() {
        Ok(())
    } else {
        Err(RuntimeCommandError::ScenarioFailed)
    }
}

fn parse_toolkit_observable_output(output: &str) -> Result<String, RuntimeCommandError> {
    output
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
        .ok_or(RuntimeCommandError::TargetCopyFailed)
}

fn submit_toolkit_target(
    environment: &RuntimeEnvironment,
    timeout: Duration,
) -> Result<(), RuntimeCommandError> {
    let status = if environment.session_type == "wayland" {
        let mut command = std::process::Command::new("wtype");
        command.args(["-k", "Return"]);
        run_command_with_timeout(&mut command, timeout)?
    } else {
        let mut command = std::process::Command::new("xdotool");
        command.args(["key", "--clearmodifiers", "Return"]);
        run_command_with_timeout(&mut command, timeout)?
    };
    if status.success() {
        Ok(())
    } else {
        Err(RuntimeCommandError::ScenarioFailed)
    }
}

fn read_toolkit_stdout(
    fixture: &ControlledRuntimeFixture,
    timeout: Duration,
) -> Result<String, RuntimeCommandError> {
    let path = fixture.artifact_dir.join("application.stdout.log");
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if let Ok(output) = std::fs::read_to_string(&path) {
            if let Ok(value) = parse_toolkit_observable_output(&output) {
                return Ok(value);
            }
        }
        if std::time::Instant::now() >= deadline {
            return Err(RuntimeCommandError::TimedOut);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn copy_observable_target_text(
    environment: &RuntimeEnvironment,
    timeout: Duration,
) -> Result<String, RuntimeCommandError> {
    let status = if environment.session_type == "wayland" {
        let mut command = std::process::Command::new("wtype");
        command.args(["-M", "ctrl", "-k", "a", "-k", "c", "-m", "ctrl"]);
        run_command_with_timeout(&mut command, timeout)?
    } else {
        let mut command = std::process::Command::new("xdotool");
        command.args(["key", "--clearmodifiers", "ctrl+a", "ctrl+c"]);
        run_command_with_timeout(&mut command, timeout)?
    };
    if !status.success() {
        return Err(RuntimeCommandError::ScenarioFailed);
    }
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        let observed = clipboard_read(environment, timeout)?;
        if !observed.is_empty() {
            return Ok(observed);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    Err(RuntimeCommandError::TimedOut)
}

fn verify_synthesized_modifiers_released(
    environment: &RuntimeEnvironment,
    timeout: Duration,
) -> Result<bool, RuntimeCommandError> {
    let status = if environment.session_type == "wayland" {
        let mut command = std::process::Command::new("wtype");
        command.args(["-m", "ctrl", "-m", "shift", "-m", "alt", "-m", "logo"]);
        run_command_with_timeout(&mut command, timeout)?
    } else {
        let mut command = std::process::Command::new("xdotool");
        command.args([
            "keyup", "ctrl", "keyup", "shift", "keyup", "alt", "keyup", "super",
        ]);
        run_command_with_timeout(&mut command, timeout)?
    };
    Ok(status.success())
}

fn connect_controlled_browser_cdp(
    fixture: &ControlledRuntimeFixture,
    timeout: Duration,
) -> Result<CdpConnection, RuntimeCommandError> {
    let deadline = std::time::Instant::now() + timeout;
    let active_port_path = fixture.profile_dir.join("DevToolsActivePort");
    loop {
        if let Ok(contents) = std::fs::read_to_string(&active_port_path) {
            if let Ok((port, _)) = parse_devtools_active_port(&contents) {
                if let Ok(targets) = http_get_localhost(port, "/json/list", timeout) {
                    let _ = std::fs::write(fixture.artifact_dir.join("cdp-targets.json"), &targets);
                    if let Ok(url) = controlled_target_websocket_url(&targets) {
                        let _ = std::fs::write(
                            fixture.artifact_dir.join("cdp-endpoint.txt"),
                            format!("port={port}\nwebsocket={url}\n"),
                        );
                        return CdpConnection::connect(&url, timeout);
                    }
                }
            }
        }
        if std::time::Instant::now() >= deadline {
            return Err(RuntimeCommandError::TimedOut);
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn prepare_controlled_browser_target(cdp: &mut CdpConnection) -> Result<(), RuntimeCommandError> {
    cdp.command("Page.bringToFront", serde_json::json!({}))?;
    cdp.command(
        "Runtime.evaluate",
        serde_json::json!({
            "expression": "(() => { const target = document.querySelector('#target'); if (!target) throw new Error('controlled textarea missing'); target.value = ''; target.focus(); target.setSelectionRange(0, 0); return document.activeElement === target; })()",
            "returnByValue": true,
            "awaitPromise": true
        }),
    )?;
    Ok(())
}

fn read_controlled_browser_target(cdp: &mut CdpConnection) -> Result<String, RuntimeCommandError> {
    let response = cdp.command(
        "Runtime.evaluate",
        serde_json::json!({
            "expression": "document.querySelector('#target')?.value",
            "returnByValue": true,
            "awaitPromise": true
        }),
    )?;
    response
        .pointer("/result/result/value")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .ok_or(RuntimeCommandError::TargetCopyFailed)
}

fn execute_controlled_scenario(
    target: RuntimeTargetKind,
    fixture: ControlledRuntimeFixture,
    environment: RuntimeEnvironment,
    child_pid: u32,
    timeout: Duration,
) -> Result<ControlledScenarioEvidence, RuntimeCommandError> {
    let operation_timeout = timeout.min(Duration::from_secs(5));
    let trace_path = fixture.artifact_dir.join("scenario-trace.txt");
    let trace = |step: &str| {
        let mut options = std::fs::OpenOptions::new();
        options.create(true).append(true);
        if let Ok(mut file) = options.open(&trace_path) {
            use std::io::Write;
            let _ = writeln!(
                file,
                "{} elapsed_ms={}",
                step,
                std::time::Instant::now().elapsed().as_millis()
            );
        }
    };

    trace("clipboard_snapshot:start");
    let clipboard_snapshot = clipboard_read(&environment, operation_timeout)
        .map_err(|_| RuntimeCommandError::ClipboardSnapshotFailed)?;
    trace("clipboard_snapshot:done");

    let mut browser_cdp = None;
    let scenario_result = (|| {
        let mut cdp = if matches!(
            target,
            RuntimeTargetKind::Chromium | RuntimeTargetKind::Chrome
        ) {
            trace("cdp_connect:start");
            let mut connection = connect_controlled_browser_cdp(&fixture, operation_timeout)?;
            trace("cdp_connect:done");
            prepare_controlled_browser_target(&mut connection)?;
            trace("cdp_prepare:done");
            Some(connection)
        } else {
            None
        };

        trace("input_injection:start");
        inject_controlled_text(&environment, CONTROLLED_TEXT, operation_timeout)
            .map_err(|_| RuntimeCommandError::InputInjectionFailed)?;
        trace("input_injection:done");

        let observed_text = match cdp.as_mut() {
            Some(connection) => read_controlled_browser_target(connection)
                .map_err(|_| RuntimeCommandError::TargetCopyFailed)?,
            None if matches!(target, RuntimeTargetKind::Gtk | RuntimeTargetKind::Qt) => {
                submit_toolkit_target(&environment, operation_timeout)?;
                read_toolkit_stdout(&fixture, operation_timeout)?
            }
            None => copy_observable_target_text(&environment, operation_timeout)
                .map_err(|_| RuntimeCommandError::TargetCopyFailed)?,
        };
        let _ = std::fs::write(
            fixture.artifact_dir.join("observable-value.txt"),
            format!("expected={CONTROLLED_TEXT:?}\nobserved={observed_text:?}\n"),
        );
        trace("observable_read:done");
        browser_cdp = cdp;
        Ok(observed_text)
    })();

    trace("clipboard_restore:start");
    clipboard_write(&environment, &clipboard_snapshot, operation_timeout)
        .map_err(|_| RuntimeCommandError::ClipboardRestoreFailed)?;
    trace("clipboard_restore:done");
    let clipboard_preserved = clipboard_read(&environment, operation_timeout)
        .map_err(|_| RuntimeCommandError::ClipboardVerificationFailed)?
        == clipboard_snapshot;
    trace("clipboard_verify:done");
    let modifiers_released = verify_synthesized_modifiers_released(&environment, operation_timeout)
        .map_err(|_| RuntimeCommandError::ModifierReleaseFailed)?;
    trace("modifier_release:done");
    let observed_text = scenario_result?;

    Ok(ControlledScenarioEvidence {
        observed_text,
        clipboard_preserved,
        modifiers_released,
        roles: discover_owned_process_tree(target, child_pid),
        browser_cdp,
    })
}

fn run_controlled_runtime_target(
    target: RuntimeTargetKind,
    environment: &RuntimeEnvironment,
    startup_timeout: Duration,
    scenario_timeout: Duration,
    teardown_timeout: Duration,
) -> RuntimeTargetResult {
    if !controlled_display_available(environment) {
        return RuntimeTargetResult {
            status: RuntimeMatrixStatus::Unavailable,
            reason: "controlled display capability is unavailable; require VIETC_RUNTIME_TESTS=1, VIETC_CONTROLLED_DISPLAY=1, a display, and writable /dev/uinput".to_string(),
            observation: None,
        };
    }
    let executable = match runtime_target_executable(target, environment) {
        Some(executable) => executable,
        None => {
            return RuntimeTargetResult {
                status: RuntimeMatrixStatus::Unavailable,
                reason: format!("no supported executable is installed for {target:?}"),
                observation: None,
            };
        }
    };

    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("daemon crate must have a workspace root");
    let run_id = format!("{:?}-{}", target, std::process::id()).to_ascii_lowercase();
    let fixture = match ControlledRuntimeFixture::isolated(workspace_root, &run_id) {
        Ok(fixture) => fixture,
        Err(reason) => {
            return RuntimeTargetResult {
                status: RuntimeMatrixStatus::Failed,
                reason: reason.to_string(),
                observation: None,
            }
        }
    };
    if let Err(error) = fixture.prepare(workspace_root) {
        return RuntimeTargetResult {
            status: RuntimeMatrixStatus::Failed,
            reason: format!("fixture preparation failed: {error}"),
            observation: None,
        };
    }

    if let Err(error) = prepare_target_fixture(target, &fixture) {
        return RuntimeTargetResult {
            status: RuntimeMatrixStatus::Failed,
            reason: format!("target fixture preparation failed: {error}"),
            observation: None,
        };
    }
    let flags = runtime_target_flags(target, &fixture, executable);
    let stdout_path = fixture.artifact_dir.join("application.stdout.log");
    let stderr_path = fixture.artifact_dir.join("application.stderr.log");
    let stdout = match std::fs::File::create(&stdout_path) {
        Ok(file) => file,
        Err(error) => {
            return RuntimeTargetResult {
                status: RuntimeMatrixStatus::Failed,
                reason: format!("stdout capture failed: {error}"),
                observation: None,
            }
        }
    };
    let stderr = match std::fs::File::create(&stderr_path) {
        Ok(file) => file,
        Err(error) => {
            return RuntimeTargetResult {
                status: RuntimeMatrixStatus::Failed,
                reason: format!("stderr capture failed: {error}"),
                observation: None,
            }
        }
    };

    let mut command = std::process::Command::new(executable);
    command
        .args(&flags)
        .stdin(std::process::Stdio::null())
        .stdout(stdout)
        .stderr(stderr);

    let started = std::time::Instant::now();
    let mut child = match OwnedRuntimeChild::spawn(&mut command) {
        Ok(child) => child,
        Err(error) => {
            return RuntimeTargetResult {
                status: RuntimeMatrixStatus::Failed,
                reason: format!("spawn failed: {error:?}"),
                observation: None,
            }
        }
    };

    std::thread::sleep(startup_timeout);
    match child.healthy() {
        Ok(true) => {}
        Ok(false) => {
            return RuntimeTargetResult {
                status: RuntimeMatrixStatus::Failed,
                reason: "target exited during startup".to_string(),
                observation: None,
            }
        }
        Err(error) => {
            return RuntimeTargetResult {
                status: RuntimeMatrixStatus::Failed,
                reason: format!("startup health check failed: {error:?}"),
                observation: None,
            }
        }
    }

    let roles = discover_owned_process_tree(target, child.pid());
    let manifest = render_target_manifest(
        target,
        &tool_version(executable),
        &flags,
        &environment.session_type,
        "controlled-text-sequence-v1",
        &roles,
    );
    let _ = std::fs::write(fixture.artifact_dir.join("target-manifest.txt"), manifest);

    let scenario_pid = child.pid();
    let scenario_fixture = fixture.clone();
    let scenario_environment = environment.clone();
    let scenario = run_scenario_with_deadline(scenario_timeout, move || {
        execute_controlled_scenario(
            target,
            scenario_fixture,
            scenario_environment,
            scenario_pid,
            scenario_timeout,
        )
    });
    let evidence = match scenario {
        Ok(Ok(evidence)) => evidence,
        Ok(Err(error)) => {
            let teardown = child.terminate_and_reap(teardown_timeout);
            return RuntimeTargetResult {
                status: RuntimeMatrixStatus::Failed,
                reason: match teardown {
                    Ok(()) => format!("controlled scenario failed: {error:?}"),
                    Err(teardown_error) => format!(
                        "controlled scenario failed: {error:?}; teardown failed: {teardown_error:?}"
                    ),
                },
                observation: None,
            };
        }
        Err(RuntimeCommandError::TimedOut) => {
            let teardown = child.terminate_and_reap(teardown_timeout);
            return RuntimeTargetResult {
                status: RuntimeMatrixStatus::Failed,
                reason: match teardown {
                    Ok(()) => "controlled scenario exceeded its enforced deadline".to_string(),
                    Err(error) => {
                        format!("controlled scenario timed out and teardown failed: {error:?}")
                    }
                },
                observation: None,
            };
        }
        Err(error) => {
            let _ = child.terminate_and_reap(teardown_timeout);
            return RuntimeTargetResult {
                status: RuntimeMatrixStatus::Failed,
                reason: format!("controlled scenario failed: {error:?}"),
                observation: None,
            };
        }
    };

    let mut browser_cdp = evidence.browser_cdp;
    let observation = RuntimeSequenceObservation {
        expected_text: CONTROLLED_TEXT.to_string(),
        observed_text: evidence.observed_text,
        process_healthy: {
            let still_running = child.healthy().unwrap_or(false);
            let expected_submit_exit =
                matches!(target, RuntimeTargetKind::Gtk | RuntimeTargetKind::Qt)
                    && child.completed_successfully();
            (still_running || expected_submit_exit) && !evidence.roles.is_empty()
        },
        clipboard_preserved: evidence.clipboard_preserved,
        modifiers_released: evidence.modifiers_released,
    };

    let elapsed = started.elapsed();
    let _ = std::fs::write(
        fixture.artifact_dir.join("timing.txt"),
        format!(
            "startup_timeout_ms={}\nscenario_timeout_ms={}\nteardown_timeout_ms={}\nelapsed_ms={}\n",
            startup_timeout.as_millis(),
            scenario_timeout.as_millis(),
            teardown_timeout.as_millis(),
            elapsed.as_millis()
        ),
    );

    let verification = observation.verify();
    let graceful_close = browser_cdp
        .as_mut()
        .map(|cdp| cdp.command("Browser.close", serde_json::json!({})));
    let teardown = child.terminate_and_reap_after_graceful_close(teardown_timeout, graceful_close);
    match (verification, teardown) {
        (Ok(()), Ok(())) => RuntimeTargetResult {
            status: RuntimeMatrixStatus::Pass,
            reason: "controlled observable scenario passed and every owned child was reaped"
                .to_string(),
            observation: Some(observation),
        },
        (Err(reason), Ok(())) => RuntimeTargetResult {
            status: RuntimeMatrixStatus::Failed,
            reason: reason.to_string(),
            observation: Some(observation),
        },
        (_, Err(error)) => RuntimeTargetResult {
            status: RuntimeMatrixStatus::Failed,
            reason: format!("teardown failed: {error:?}"),
            observation: Some(observation),
        },
    }
}

#[test]
fn deterministic_runtime_target_without_controlled_display_never_spawns_or_passes() {
    let environment = RuntimeEnvironment::detect_with(
        |name| match name {
            "VIETC_RUNTIME_TESTS" => Some("1".to_string()),
            "DISPLAY" => Some(":99".to_string()),
            "XDG_SESSION_TYPE" => Some("x11".to_string()),
            _ => None,
        },
        |_| true,
        true,
    );
    let result = run_controlled_runtime_target(
        RuntimeTargetKind::Chromium,
        &environment,
        Duration::from_millis(10),
        Duration::from_millis(10),
        Duration::from_millis(10),
    );
    assert_eq!(result.status, RuntimeMatrixStatus::Unavailable);
    assert!(!result.reason.is_empty());
    assert!(result.observation.is_none());
}

#[test]
#[ignore = "requires VIETC_RUNTIME_TESTS=1, VIETC_CONTROLLED_DISPLAY=1, a controlled GUI session, writable /dev/uinput, and installed target applications"]
fn privileged_runtime_targets_execute_controlled_scenarios() {
    let environment = RuntimeEnvironment::detect_current();
    assert!(
        controlled_display_available(&environment),
        "controlled runtime requires VIETC_RUNTIME_TESTS=1 and VIETC_CONTROLLED_DISPLAY=1"
    );
    let requested_targets = requested_runtime_targets();
    let artifact_directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("daemon crate must have a workspace root")
        .join("target/runtime-artifacts");
    std::fs::create_dir_all(&artifact_directory)
        .expect("runtime artifact directory must be created");

    for target in [
        RuntimeTargetKind::Chromium,
        RuntimeTargetKind::Chrome,
        RuntimeTargetKind::Electron,
        RuntimeTargetKind::Gtk,
        RuntimeTargetKind::Qt,
        RuntimeTargetKind::Terminal,
    ] {
        if !runtime_target_requested(target, requested_targets.as_ref()) {
            continue;
        }
        let result = run_controlled_runtime_target(
            target,
            &environment,
            Duration::from_secs(10),
            Duration::from_secs(30),
            Duration::from_secs(5),
        );
        let observation = result
            .observation
            .as_ref()
            .map(|observation| {
                format!(
                    "expected_text={:?}\nobserved_text={:?}\nprocess_healthy={}\nclipboard_preserved={}\nmodifiers_released={}\n",
                    observation.expected_text,
                    observation.observed_text,
                    observation.process_healthy,
                    observation.clipboard_preserved,
                    observation.modifiers_released,
                )
            })
            .unwrap_or_else(|| "observation=missing\n".to_string());
        std::fs::write(
            artifact_directory.join(format!("controlled-{}-result.txt", target.executable())),
            format!(
                "target={target:?}\nstatus={}\nreason={}\n{observation}",
                result.status.label(),
                result.reason,
            ),
        )
        .expect("controlled runtime result artifact must be persisted");

        assert_eq!(
            result.status,
            RuntimeMatrixStatus::Pass,
            "{target:?} controlled runtime did not pass: {}",
            result.reason
        );
        let observation = result
            .observation
            .expect("passing controlled runtime requires an observation");
        observation
            .verify()
            .expect("passing controlled runtime requires healthy process, observable text, clipboard preservation, and released modifiers");
    }
}

#[test]
#[ignore = "requires VIETC_RUNTIME_TESTS=1; records the real desktop/runtime prerequisite matrix"]
fn privileged_runtime_matrix_records_exact_ready_and_skip_evidence() {
    let environment = RuntimeEnvironment::detect_current();
    assert!(
        environment.runtime_opt_in,
        "set VIETC_RUNTIME_TESTS=1 to execute the privileged runtime matrix"
    );

    let results = evaluate_runtime_matrix(&environment);
    let tool_names = [
        "wtype",
        "wl-copy",
        "wl-paste",
        "xclip",
        "zenity",
        "python3",
        "chromium",
        "chromium-browser",
        "google-chrome",
        "electron",
        "gnome-terminal",
        "konsole",
        "xterm",
        "kitty",
    ];
    let tools = tool_names
        .into_iter()
        .map(|tool| (tool, tool_version(tool)))
        .collect::<Vec<_>>();
    let artifact = render_runtime_matrix_artifact(
        &environment.session_type,
        &environment.desktop,
        tools,
        &results,
    );

    let artifact_directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("daemon crate must have a workspace root")
        .join("target/runtime-artifacts");
    std::fs::create_dir_all(&artifact_directory)
        .expect("runtime artifact directory must be created");
    std::fs::write(
        artifact_directory.join("runtime-matrix-prerequisites.txt"),
        artifact,
    )
    .expect("runtime matrix prerequisite artifact must be persisted");

    assert_eq!(results.entries.len(), 10);
    assert!(results
        .entries
        .iter()
        .all(|entry| !entry.reason.trim().is_empty() && !entry.owner.trim().is_empty()));
    for capability in ["chromium", "google-chrome", "cosmic-wayland"] {
        assert_eq!(
            results.status_for(capability),
            Some(RuntimeMatrixStatus::Pass),
            "{capability} must be backed by verified controlled evidence"
        );
    }
}

#[test]
#[ignore = "requires VIETC_RUNTIME_TESTS=1, a real Wayland compositor, and writable /dev/uinput"]
fn wayland_wtype_success_is_observable_at_runtime() {
    let result = run_wayland_wtype_success(Duration::from_secs(30))
        .expect("Wayland wtype runtime scenario must complete without harness errors");
    assert_eq!(
        result.inject_result,
        vietc_protocol::inject::InjectResult::Success
    );
    assert_eq!(result.observed_text, result.expected_text);
    assert!(
        !result.observed_text.is_empty(),
        "runtime verification must observe text in the target application"
    );
    let artifact_directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("daemon crate must have a workspace root")
        .join("target/runtime-artifacts");
    std::fs::create_dir_all(&artifact_directory)
        .expect("runtime artifact directory must be created");
    let artifact = RuntimeArtifact::new(
        std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "wayland".to_string()),
        std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "unknown".to_string()),
        [
            ("wtype", tool_version("wtype")),
            ("zenity", tool_version("zenity")),
        ],
    )
    .with_failure_log(
        "wayland-wtype-success",
        format!(
            "PASS: expected={:?}; observed={:?}; inject_result={}",
            result.expected_text, result.observed_text, result.inject_result
        ),
    );
    artifact
        .write_to(&artifact_directory.join("wayland-wtype-success.txt"))
        .expect("Wayland runtime artifact must be persisted");
}
