// SPDX-License-Identifier: MIT
//! Explicit capability gates for privileged runtime verification.

use crate::common;

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RuntimeFailureCategory {
    EngineMismatch,
    TransactionFailure,
    FocusFailure,
    ObservationFailure,
    StartupFailure,
    LifecycleFailure,
    Unavailable,
}

impl RuntimeFailureCategory {
    fn label(self) -> &'static str {
        match self {
            Self::EngineMismatch => "ENGINE_MISMATCH",
            Self::TransactionFailure => "TRANSACTION_FAILURE",
            Self::FocusFailure => "FOCUS_FAILURE",
            Self::ObservationFailure => "OBSERVATION_FAILURE",
            Self::StartupFailure => "STARTUP_FAILURE",
            Self::LifecycleFailure => "LIFECYCLE_FAILURE",
            Self::Unavailable => "UNAVAILABLE",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RuntimeFailure {
    category: RuntimeFailureCategory,
    checkpoint: String,
    detail: String,
}

impl RuntimeFailure {
    fn new(
        category: RuntimeFailureCategory,
        checkpoint: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            category,
            checkpoint: checkpoint.into(),
            detail: detail.into(),
        }
    }

    fn render(&self) -> String {
        format!(
            "category={}; checkpoint={}; detail={}",
            self.category.label(),
            self.checkpoint,
            self.detail
        )
    }
}

fn classify_runtime_command_error(error: &RuntimeCommandError, checkpoint: &str) -> RuntimeFailure {
    let category = match error {
        RuntimeCommandError::PrerequisiteMissing => RuntimeFailureCategory::Unavailable,
        RuntimeCommandError::SpawnFailed => RuntimeFailureCategory::StartupFailure,
        RuntimeCommandError::ClipboardSnapshotFailed
        | RuntimeCommandError::ClipboardRestoreFailed
        | RuntimeCommandError::ClipboardVerificationFailed
        | RuntimeCommandError::InputInjectionFailed
        | RuntimeCommandError::ModifierReleaseFailed => RuntimeFailureCategory::TransactionFailure,
        RuntimeCommandError::TargetCopyFailed => RuntimeFailureCategory::ObservationFailure,
        RuntimeCommandError::WaitFailed => RuntimeFailureCategory::LifecycleFailure,
        RuntimeCommandError::TimedOut | RuntimeCommandError::ScenarioFailed => {
            RuntimeFailureCategory::ObservationFailure
        }
    };
    RuntimeFailure::new(category, checkpoint, format!("{error:?}"))
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
        const TOOLS: [&str; 19] = [
            "chromium",
            "chromium-browser",
            "google-chrome",
            "microsoft-edge",
            "microsoft-edge-stable",
            "microsoft-edge-beta",
            "microsoft-edge-dev",
            "electron",
            "firefox",
            "cosmic-edit",
            "zenity",
            "python3",
            "wtype",
            "ydotool",
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
    let mut child = match std::process::Command::new(program)
        .arg("--version")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return "missing".to_string();
        }
        Err(_) => return "installed; version unavailable".to_string(),
    };

    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let output = match child.wait_with_output() {
                    Ok(output) => output,
                    Err(_) => return "installed; version unavailable".to_string(),
                };
                if !status.success() {
                    return "installed; version unavailable".to_string();
                }

                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let version = if stdout.is_empty() {
                    String::from_utf8_lossy(&output.stderr).trim().to_string()
                } else {
                    stdout
                };
                return if version.is_empty() {
                    "installed; version unavailable".to_string()
                } else {
                    version
                };
            }
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return "installed; version unavailable".to_string();
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return "installed; version unavailable".to_string();
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct UnicodeDivergence {
    scalar_index: usize,
    expected_scalar: Option<char>,
    observed_scalar: Option<char>,
    first_incorrect_word: Option<String>,
}

fn first_unicode_divergence(expected: &str, observed: &str) -> Option<UnicodeDivergence> {
    let expected_scalars: Vec<char> = expected.chars().collect();
    let observed_scalars: Vec<char> = observed.chars().collect();
    let scalar_index = expected_scalars
        .iter()
        .zip(&observed_scalars)
        .position(|(expected, observed)| expected != observed)
        .or_else(|| {
            (expected_scalars.len() != observed_scalars.len())
                .then_some(expected_scalars.len().min(observed_scalars.len()))
        })?;

    let word_start = expected_scalars[..scalar_index.min(expected_scalars.len())]
        .iter()
        .rposition(|scalar| scalar.is_whitespace())
        .map_or(0, |index| index + 1);
    let first_incorrect_word = expected_scalars[word_start..]
        .iter()
        .take_while(|scalar| !scalar.is_whitespace())
        .collect::<String>();

    Some(UnicodeDivergence {
        scalar_index,
        expected_scalar: expected_scalars.get(scalar_index).copied(),
        observed_scalar: observed_scalars.get(scalar_index).copied(),
        first_incorrect_word: (!first_incorrect_word.is_empty()).then_some(first_incorrect_word),
    })
}

#[derive(Debug, Default)]
struct RuntimeArtifact {
    distro: Option<String>,
    desktop: Option<String>,
    session_type: Option<String>,
    display_server: Option<String>,
    compositor: Option<String>,
    application: Option<String>,
    application_version: Option<String>,
    application_pid_tree: Option<String>,
    window_title: Option<String>,
    app_id: Option<String>,
    tools: Vec<(String, String)>,
    raw_key_timestamps: Option<String>,
    engine_raw_buffers: Option<String>,
    parsed_syllables: Option<String>,
    rendered_text: Option<String>,
    replacement_delete_count: Option<usize>,
    replacement_inserted_unicode: Option<String>,
    transaction_id: Option<u64>,
    transaction_state_transitions: Option<String>,
    focus_history: Option<String>,
    focus_generation: Option<u64>,
    expected_text: Option<String>,
    observed_text: Option<String>,
    divergence: Option<UnicodeDivergence>,
    clipboard_polling_timeline: Option<String>,
    exit_reason: Option<String>,
    teardown_result: Option<String>,
    failures: Vec<(String, RuntimeFailure)>,
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
            display_server: Some(display_server.into()),
            compositor: Some(compositor.into()),
            tools: tools
                .into_iter()
                .map(|(name, version)| (name.into(), version.into()))
                .collect(),
            ..Self::default()
        }
    }

    fn with_failure(mut self, scenario: impl Into<String>, failure: RuntimeFailure) -> Self {
        self.failures.push((scenario.into(), failure));
        self
    }

    fn with_text_observation(
        mut self,
        expected: impl Into<String>,
        observed: impl Into<String>,
    ) -> Self {
        let expected = expected.into();
        let observed = observed.into();
        self.divergence = first_unicode_divergence(&expected, &observed);
        self.expected_text = Some(expected);
        self.observed_text = Some(observed);
        self
    }

    fn render_optional<T: std::fmt::Display>(output: &mut String, name: &str, value: Option<&T>) {
        match value {
            Some(value) => output.push_str(&format!("{name}: {value}\n")),
            None => output.push_str(&format!("{name}: <absent>\n")),
        }
    }

    fn render(&self) -> String {
        let mut output = String::new();
        Self::render_optional(&mut output, "distro", self.distro.as_ref());
        Self::render_optional(&mut output, "desktop", self.desktop.as_ref());
        Self::render_optional(&mut output, "session_type", self.session_type.as_ref());
        Self::render_optional(&mut output, "display_server", self.display_server.as_ref());
        Self::render_optional(&mut output, "compositor", self.compositor.as_ref());
        Self::render_optional(&mut output, "application", self.application.as_ref());
        Self::render_optional(
            &mut output,
            "application_version",
            self.application_version.as_ref(),
        );
        Self::render_optional(
            &mut output,
            "application_pid_tree",
            self.application_pid_tree.as_ref(),
        );
        Self::render_optional(&mut output, "window_title", self.window_title.as_ref());
        Self::render_optional(&mut output, "app_id", self.app_id.as_ref());
        output.push_str("tools:\n");
        if self.tools.is_empty() {
            output.push_str("  <absent>\n");
        } else {
            for (name, version) in &self.tools {
                output.push_str(&format!("  {name}: {version}\n"));
            }
        }
        Self::render_optional(
            &mut output,
            "raw_key_timestamps",
            self.raw_key_timestamps.as_ref(),
        );
        Self::render_optional(
            &mut output,
            "engine_raw_buffers",
            self.engine_raw_buffers.as_ref(),
        );
        Self::render_optional(
            &mut output,
            "parsed_syllables",
            self.parsed_syllables.as_ref(),
        );
        Self::render_optional(&mut output, "rendered_text", self.rendered_text.as_ref());
        Self::render_optional(
            &mut output,
            "replacement_delete_count",
            self.replacement_delete_count.as_ref(),
        );
        Self::render_optional(
            &mut output,
            "replacement_inserted_unicode",
            self.replacement_inserted_unicode.as_ref(),
        );
        Self::render_optional(&mut output, "transaction_id", self.transaction_id.as_ref());
        Self::render_optional(
            &mut output,
            "transaction_state_transitions",
            self.transaction_state_transitions.as_ref(),
        );
        Self::render_optional(&mut output, "focus_history", self.focus_history.as_ref());
        Self::render_optional(
            &mut output,
            "focus_generation",
            self.focus_generation.as_ref(),
        );
        Self::render_optional(&mut output, "expected_text", self.expected_text.as_ref());
        Self::render_optional(&mut output, "observed_text", self.observed_text.as_ref());
        match &self.divergence {
            Some(divergence) => {
                output.push_str(&format!(
                    "first_difference_scalar_index: {}\nfirst_difference_expected_scalar: {:?}\nfirst_difference_observed_scalar: {:?}\n",
                    divergence.scalar_index,
                    divergence.expected_scalar,
                    divergence.observed_scalar,
                ));
                Self::render_optional(
                    &mut output,
                    "first_incorrect_word",
                    divergence.first_incorrect_word.as_ref(),
                );
            }
            None => {
                output.push_str("first_difference_scalar_index: <absent>\n");
                output.push_str("first_difference_expected_scalar: <absent>\n");
                output.push_str("first_difference_observed_scalar: <absent>\n");
                output.push_str("first_incorrect_word: <absent>\n");
            }
        }
        Self::render_optional(
            &mut output,
            "clipboard_polling_timeline",
            self.clipboard_polling_timeline.as_ref(),
        );
        Self::render_optional(&mut output, "exit_reason", self.exit_reason.as_ref());
        Self::render_optional(
            &mut output,
            "teardown_result",
            self.teardown_result.as_ref(),
        );
        output.push_str("failures:\n");
        if self.failures.is_empty() {
            output.push_str("  <absent>\n");
        } else {
            for (scenario, failure) in &self.failures {
                output.push_str(&format!("  {scenario}: {}\n", failure.render()));
            }
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
fn runtime_failure_taxonomy_has_stable_labels_and_checkpointed_rendering() {
    let categories = [
        (RuntimeFailureCategory::EngineMismatch, "ENGINE_MISMATCH"),
        (
            RuntimeFailureCategory::TransactionFailure,
            "TRANSACTION_FAILURE",
        ),
        (RuntimeFailureCategory::FocusFailure, "FOCUS_FAILURE"),
        (
            RuntimeFailureCategory::ObservationFailure,
            "OBSERVATION_FAILURE",
        ),
        (RuntimeFailureCategory::StartupFailure, "STARTUP_FAILURE"),
        (
            RuntimeFailureCategory::LifecycleFailure,
            "LIFECYCLE_FAILURE",
        ),
        (RuntimeFailureCategory::Unavailable, "UNAVAILABLE"),
    ];
    for (category, label) in categories {
        assert_eq!(category.label(), label);
        let rendered = RuntimeFailure::new(category, "focus_probe", "failed").render();
        assert!(rendered.contains(label));
        assert!(rendered.contains("checkpoint=focus_probe"));
    }
}

#[test]
fn runtime_artifact_reports_unicode_divergence_by_scalar_and_word() {
    let artifact =
        RuntimeArtifact::default().with_text_observation("xin chào thế giới", "xin chao thế giới");
    let divergence = artifact.divergence.as_ref().expect("texts must diverge");
    assert_eq!(divergence.scalar_index, 6);
    assert_eq!(divergence.expected_scalar, Some('à'));
    assert_eq!(divergence.observed_scalar, Some('a'));
    assert_eq!(divergence.first_incorrect_word.as_deref(), Some("chào"));
}

#[test]
fn runtime_artifact_renders_missing_evidence_explicitly() {
    let rendered = RuntimeArtifact::default().render();
    assert!(rendered.contains("distro: <absent>"));
    assert!(rendered.contains("application_pid_tree: <absent>"));
    assert!(rendered.contains("raw_key_timestamps: <absent>"));
    assert!(rendered.contains("transaction_id: <absent>"));
    assert!(rendered.contains("first_difference_scalar_index: <absent>"));
    assert!(rendered.contains("teardown_result: <absent>"));
    assert!(rendered.contains("failures:\n  <absent>"));
}

#[test]
fn runtime_command_errors_are_surfaced_with_category_and_checkpoint() {
    let failure =
        classify_runtime_command_error(&RuntimeCommandError::TargetCopyFailed, "clipboard_read");
    assert_eq!(failure.category, RuntimeFailureCategory::ObservationFailure);
    assert_eq!(failure.checkpoint, "clipboard_read");

    let failure =
        classify_runtime_command_error(&RuntimeCommandError::TimedOut, "target_observation");
    assert_eq!(failure.category, RuntimeFailureCategory::ObservationFailure);
    assert_eq!(failure.checkpoint, "target_observation");
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
    .with_failure(
        "wayland-wtype-fallback",
        RuntimeFailure::new(
            RuntimeFailureCategory::TransactionFailure,
            "input_dispatch",
            "wtype exited with status 1",
        ),
    );
    let rendered = artifact.render();
    assert!(rendered.contains("display_server: wayland"));
    assert!(rendered.contains("compositor: gnome"));
    assert!(rendered.contains("wtype: 0.4.0"));
    assert!(rendered.contains("wl-copy: 2.2.1"));
    assert!(rendered.contains("chromium: 141.0"));
    assert!(rendered.contains("wayland-wtype-fallback"));
    assert!(rendered.contains("category=TRANSACTION_FAILURE"));
    assert!(rendered.contains("checkpoint=input_dispatch"));
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
    .with_failure(
        "none",
        RuntimeFailure::new(
            RuntimeFailureCategory::ObservationFailure,
            "artifact_test",
            "synthetic persisted failure",
        ),
    );
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
    Edge,
    Firefox,
    Electron,
    CosmicEdit,
    Gtk,
    Qt,
    Terminal,
}

impl RuntimeTargetKind {
    fn executable(self) -> &'static str {
        match self {
            Self::Chromium => "chromium",
            Self::Chrome => "google-chrome",
            Self::Edge => "microsoft-edge",
            Self::Firefox => "firefox",
            Self::Electron => "electron",
            Self::CosmicEdit => "cosmic-edit",
            Self::Gtk => "zenity",
            Self::Qt => "python3",
            Self::Terminal => "terminal",
        }
    }

    fn role(self) -> RuntimeProcessRole {
        match self {
            Self::Chromium | Self::Chrome | Self::Edge | Self::Firefox => {
                RuntimeProcessRole::Browser
            }
            Self::Electron => RuntimeProcessRole::ElectronMain,
            Self::CosmicEdit | Self::Gtk | Self::Qt => RuntimeProcessRole::ToolkitTarget,
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
        return ["gnome-terminal", "konsole", "kitty", "xterm"]
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
        RuntimeTargetKind::Edge => [
            "microsoft-edge",
            "microsoft-edge-stable",
            "microsoft-edge-beta",
            "microsoft-edge-dev",
        ]
        .into_iter()
        .find(|executable| environment.has_tool(executable)),
        RuntimeTargetKind::Firefox if environment.has_tool("firefox") => Some("/usr/bin/firefox"),
        RuntimeTargetKind::Electron if environment.has_tool("electron") => Some("electron"),
        RuntimeTargetKind::CosmicEdit if environment.has_tool("cosmic-edit") => {
            Some("/usr/bin/cosmic-edit")
        }
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
        RuntimeTargetKind::Chromium
            | RuntimeTargetKind::Chrome
            | RuntimeTargetKind::Edge
            | RuntimeTargetKind::Firefox
            | RuntimeTargetKind::Electron
    ) {
        let run_id = fixture
            .profile_dir
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .unwrap_or("runtime");
        let title = format!("Vietc controlled runtime {run_id}");
        std::fs::write(
            fixture.profile_dir.join("controlled-target.html"),
            format!(
                "<!doctype html><meta charset=utf-8><title>{title}</title><style>html,body,#target{{box-sizing:border-box;width:100%;height:100%;margin:0}}</style><textarea id=target autofocus aria-label='controlled runtime target'></textarea><script>const target=document.querySelector('#target'); target.focus(); target.select();</script>"
            ),
        )?;
    }
    if target == RuntimeTargetKind::CosmicEdit {
        std::fs::write(fixture.profile_dir.join("controlled-target.txt"), "")?;
    }
    Ok(())
}

fn runtime_target_flags(
    target: RuntimeTargetKind,
    fixture: &ControlledRuntimeFixture,
    executable: &str,
) -> Vec<String> {
    match target {
        RuntimeTargetKind::Chromium | RuntimeTargetKind::Chrome | RuntimeTargetKind::Edge => vec![
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
        RuntimeTargetKind::Firefox => {
            vec!["--new-window".to_string(), controlled_fixture_url(fixture)]
        }
        RuntimeTargetKind::Electron => vec![
            format!("--user-data-dir={}", fixture.profile_dir.display()),
            "--remote-debugging-address=127.0.0.1".to_string(),
            "--remote-debugging-port=0".to_string(),
            "--remote-allow-origins=http://127.0.0.1".to_string(),
            controlled_fixture_url(fixture),
        ],
        RuntimeTargetKind::CosmicEdit => vec![fixture
            .profile_dir
            .join("controlled-target.txt")
            .display()
            .to_string()],
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
                "-T".to_string(),
                "Vietc controlled runtime".to_string(),
                "-e".to_string(),
                "sh".to_string(),
                "-c".to_string(),
                format!(
                    "read -r line; printf '%s\\n' \"$line\" > {}",
                    fixture
                        .artifact_dir
                        .join("application.stdout.log")
                        .display()
                ),
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
        RuntimeTargetKind::Edge => "microsoft-edge",
        RuntimeTargetKind::Firefox => "firefox",
        RuntimeTargetKind::Electron => "electron",
        RuntimeTargetKind::CosmicEdit => "cosmic-edit",
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

fn focus_controlled_terminal(
    environment: &RuntimeEnvironment,
    timeout: Duration,
) -> Result<(), RuntimeCommandError> {
    if environment.session_type != "wayland" || !environment.has_tool("xdotool") {
        return Ok(());
    }

    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        let output = std::process::Command::new("xdotool")
            .args(["search", "--name", "^Vietc controlled runtime$"])
            .output()
            .map_err(|_| RuntimeCommandError::ScenarioFailed)?;
        if output.status.success() {
            if let Some(window_id) = String::from_utf8_lossy(&output.stdout)
                .lines()
                .map(str::trim)
                .find(|line| {
                    !line.is_empty() && line.chars().all(|character| character.is_ascii_digit())
                })
            {
                let mut command = std::process::Command::new("xdotool");
                command.args(["windowactivate", "--sync", window_id]);
                let status = run_command_with_timeout(&mut command, timeout)?;
                if status.success() {
                    return Ok(());
                }
            }
        }
        std::thread::sleep(Duration::from_millis(25));
    }

    Err(RuntimeCommandError::TimedOut)
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
    let sentinel = "vietc-clipboard-sentinel";
    let mut writer = if environment.session_type == "wayland" {
        std::process::Command::new("wl-copy")
    } else {
        let mut command = std::process::Command::new("xclip");
        command.args(["-selection", "clipboard", "-i"]);
        command
    };
    let mut writer_child = writer
        .stdin(std::process::Stdio::piped())
        .spawn()
        .map_err(|_| RuntimeCommandError::SpawnFailed)?;
    if let Some(mut stdin) = writer_child.stdin.take() {
        use std::io::Write;
        stdin
            .write_all(sentinel.as_bytes())
            .map_err(|_| RuntimeCommandError::ScenarioFailed)?;
    }
    writer_child
        .wait()
        .map_err(|_| RuntimeCommandError::WaitFailed)?;

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
        if !observed.is_empty() && observed != sentinel {
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

#[derive(Clone, Debug, PartialEq, Eq)]
struct X11WindowCandidate {
    window_id: String,
    pid: u32,
}

fn process_tree_contains_pid(root_pid: u32, candidate_pid: u32) -> bool {
    if root_pid == candidate_pid {
        return true;
    }
    discover_owned_process_tree(RuntimeTargetKind::Firefox, root_pid)
        .iter()
        .any(|process| process.pid == candidate_pid)
}

fn select_controlled_firefox_window(
    candidates: &[X11WindowCandidate],
    firefox_root_pid: u32,
) -> Option<X11WindowCandidate> {
    candidates
        .iter()
        .find(|candidate| process_tree_contains_pid(firefox_root_pid, candidate.pid))
        .cloned()
}

fn controlled_firefox_window(
    firefox_root_pid: u32,
    timeout: Duration,
) -> Result<X11WindowCandidate, RuntimeCommandError> {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        let output = std::process::Command::new("xdotool")
            .args(["search", "--name", "Vietc controlled runtime"])
            .output()
            .map_err(|_| RuntimeCommandError::PrerequisiteMissing)?;
        let candidates = String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::trim)
            .filter(|window_id| {
                !window_id.is_empty()
                    && window_id
                        .chars()
                        .all(|character| character.is_ascii_digit())
            })
            .filter_map(|window_id| {
                let output = std::process::Command::new("xdotool")
                    .args(["getwindowpid", window_id])
                    .output()
                    .ok()?;
                let pid = String::from_utf8_lossy(&output.stdout)
                    .trim()
                    .parse()
                    .ok()?;
                Some(X11WindowCandidate {
                    window_id: window_id.to_string(),
                    pid,
                })
            })
            .collect::<Vec<_>>();

        if let Some(window) = select_controlled_firefox_window(&candidates, firefox_root_pid) {
            return Ok(window);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err(RuntimeCommandError::TimedOut)
}

#[test]
fn deterministic_firefox_window_selection_accepts_descendant_pid_only() {
    let mut command = std::process::Command::new("sh");
    command.args(["-c", "sleep 30 & wait"]);
    let mut owned =
        OwnedRuntimeChild::spawn(&mut command).expect("fixture process tree must spawn");
    std::thread::sleep(Duration::from_millis(30));

    let descendant = discover_owned_process_tree(RuntimeTargetKind::Firefox, owned.pid())
        .into_iter()
        .find(|process| process.pid != owned.pid())
        .expect("fixture must expose a descendant");
    let candidates = [
        X11WindowCandidate {
            window_id: "100".to_string(),
            pid: std::process::id(),
        },
        X11WindowCandidate {
            window_id: "200".to_string(),
            pid: descendant.pid,
        },
    ];

    assert_eq!(
        select_controlled_firefox_window(&candidates, owned.pid()),
        Some(candidates[1].clone())
    );
    owned
        .terminate_and_reap(Duration::from_secs(1))
        .expect("fixture process tree must be reaped");
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

    if target == RuntimeTargetKind::Terminal {
        trace("terminal_focus:start");
        focus_controlled_terminal(&environment, operation_timeout)?;
        trace("terminal_focus:done");
    }

    let mut browser_cdp = None;
    let scenario_result = (|| {
        let mut firefox_window = None;
        let mut cdp = if matches!(
            target,
            RuntimeTargetKind::Chromium
                | RuntimeTargetKind::Chrome
                | RuntimeTargetKind::Edge
                | RuntimeTargetKind::Electron
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

        if target == RuntimeTargetKind::Firefox {
            trace("firefox_wayland_focus:start");
            let window = controlled_firefox_window(child_pid, operation_timeout)?;
            let window_id = window.window_id.as_str();

            let mut activate = std::process::Command::new("xdotool");
            activate.args(["windowactivate", "--sync", window_id]);
            if !run_command_with_timeout(&mut activate, operation_timeout)?.success() {
                return Err(RuntimeCommandError::ScenarioFailed);
            }

            let mut location = std::process::Command::new("xdotool");
            location.args(["key", "--window", window_id, "--clearmodifiers", "ctrl+l"]);
            if !run_command_with_timeout(&mut location, operation_timeout)?.success() {
                return Err(RuntimeCommandError::ScenarioFailed);
            }

            let fixture_url = controlled_fixture_url(&fixture);
            let mut navigate = std::process::Command::new("xdotool");
            navigate.args([
                "type",
                "--window",
                window_id,
                "--clearmodifiers",
                "--delay",
                "1",
                &fixture_url,
            ]);
            if !run_command_with_timeout(&mut navigate, operation_timeout)?.success() {
                return Err(RuntimeCommandError::ScenarioFailed);
            }

            let mut submit = std::process::Command::new("xdotool");
            submit.args(["key", "--window", window_id, "--clearmodifiers", "Return"]);
            if !run_command_with_timeout(&mut submit, operation_timeout)?.success() {
                return Err(RuntimeCommandError::ScenarioFailed);
            }

            std::thread::sleep(Duration::from_secs(1));
            let mut focus_content = std::process::Command::new("xdotool");
            focus_content.args([
                "key",
                "--window",
                window_id,
                "--clearmodifiers",
                "F6",
                "Tab",
            ]);
            if !run_command_with_timeout(&mut focus_content, operation_timeout)?.success() {
                return Err(RuntimeCommandError::ScenarioFailed);
            }

            const FOCUS_PROBE: &str = "vietc-focus-probe";
            let mut probe = std::process::Command::new("xdotool");
            probe.args([
                "type",
                "--window",
                window_id,
                "--clearmodifiers",
                "--delay",
                "1",
                FOCUS_PROBE,
            ]);
            if !run_command_with_timeout(&mut probe, operation_timeout)?.success() {
                return Err(RuntimeCommandError::InputInjectionFailed);
            }

            let mut select_copy = std::process::Command::new("xdotool");
            select_copy.args([
                "key",
                "--window",
                window_id,
                "--clearmodifiers",
                "ctrl+a",
                "ctrl+c",
            ]);
            if !run_command_with_timeout(&mut select_copy, operation_timeout)?.success() {
                return Err(RuntimeCommandError::TargetCopyFailed);
            }
            std::thread::sleep(Duration::from_millis(200));
            if clipboard_read(&environment, operation_timeout)
                .map_err(|_| RuntimeCommandError::TargetCopyFailed)?
                != FOCUS_PROBE
            {
                return Err(RuntimeCommandError::TargetCopyFailed);
            }

            let mut clear = std::process::Command::new("xdotool");
            clear.args([
                "key",
                "--window",
                window_id,
                "--clearmodifiers",
                "BackSpace",
            ]);
            if !run_command_with_timeout(&mut clear, operation_timeout)?.success() {
                return Err(RuntimeCommandError::ScenarioFailed);
            }
            firefox_window = Some(window);
            trace("firefox_wayland_focus:done");
        } else if target == RuntimeTargetKind::CosmicEdit {
            trace("cosmic_native_activate:start");
            let (editor_x, editor_y) = common::cosmic_toplevel::activate_cosmic_edit(
                "controlled-target.txt",
                operation_timeout,
            )
            .map_err(|_| RuntimeCommandError::TimedOut)?;

            let mut move_pointer = std::process::Command::new("ydotool");
            move_pointer.args([
                "mousemove",
                "--delay",
                "50",
                &editor_x.to_string(),
                &editor_y.to_string(),
            ]);
            if !run_command_with_timeout(&mut move_pointer, operation_timeout)?.success() {
                return Err(RuntimeCommandError::ScenarioFailed);
            }

            let mut click_editor = std::process::Command::new("ydotool");
            click_editor.args(["click", "--delay", "50", "1"]);
            if !run_command_with_timeout(&mut click_editor, operation_timeout)?.success() {
                return Err(RuntimeCommandError::ScenarioFailed);
            }

            std::thread::sleep(Duration::from_millis(250));
            trace("cosmic_native_activate:done");
        }

        trace("input_injection:start");
        if target == RuntimeTargetKind::Firefox {
            let window_id = firefox_window
                .as_ref()
                .ok_or(RuntimeCommandError::InputInjectionFailed)?
                .window_id
                .as_str();
            let mut command = std::process::Command::new("xdotool");
            command.args([
                "type",
                "--window",
                window_id,
                "--clearmodifiers",
                "--delay",
                "1",
                CONTROLLED_TEXT,
            ]);
            let status = run_command_with_timeout(&mut command, operation_timeout)?;
            if !status.success() {
                return Err(RuntimeCommandError::InputInjectionFailed);
            }
        } else if target == RuntimeTargetKind::CosmicEdit {
            let mut command = std::process::Command::new("ydotool");
            command.args(["type", "--key-delay", "12", CONTROLLED_TEXT]);
            if !run_command_with_timeout(&mut command, operation_timeout)?.success() {
                return Err(RuntimeCommandError::InputInjectionFailed);
            }
        } else {
            inject_controlled_text(&environment, CONTROLLED_TEXT, operation_timeout)
                .map_err(|_| RuntimeCommandError::InputInjectionFailed)?;
        }
        trace("input_injection:done");

        let observed_text = if target == RuntimeTargetKind::Firefox {
            let window_id = firefox_window
                .as_ref()
                .ok_or(RuntimeCommandError::TargetCopyFailed)?
                .window_id
                .as_str();
            let mut select_cmd = std::process::Command::new("xdotool");
            select_cmd.args(["key", "--window", window_id, "--clearmodifiers", "ctrl+a"]);
            if !run_command_with_timeout(&mut select_cmd, operation_timeout)?.success() {
                return Err(RuntimeCommandError::TargetCopyFailed);
            }
            std::thread::sleep(Duration::from_millis(200));
            let mut copy_cmd = std::process::Command::new("xdotool");
            copy_cmd.args(["key", "--window", window_id, "--clearmodifiers", "ctrl+c"]);
            if !run_command_with_timeout(&mut copy_cmd, operation_timeout)?.success() {
                return Err(RuntimeCommandError::TargetCopyFailed);
            }
            std::thread::sleep(Duration::from_millis(500));
            clipboard_read(&environment, operation_timeout)
                .map_err(|_| RuntimeCommandError::TargetCopyFailed)?
        } else {
            match cdp.as_mut() {
                Some(connection) => read_controlled_browser_target(connection)
                    .map_err(|_| RuntimeCommandError::TargetCopyFailed)?,
                None if matches!(
                    target,
                    RuntimeTargetKind::Gtk | RuntimeTargetKind::Qt | RuntimeTargetKind::Terminal
                ) =>
                {
                    submit_toolkit_target(&environment, operation_timeout)?;
                    read_toolkit_stdout(&fixture, operation_timeout)?
                }
                None if target == RuntimeTargetKind::CosmicEdit => {
                    let mut save = std::process::Command::new("ydotool");
                    save.args(["key", "29:1", "31:1", "31:0", "29:0"]);
                    if !run_command_with_timeout(&mut save, operation_timeout)?.success() {
                        return Err(RuntimeCommandError::TargetCopyFailed);
                    }

                    let target_path = fixture.profile_dir.join("controlled-target.txt");
                    let deadline = std::time::Instant::now() + operation_timeout;
                    loop {
                        if let Ok(value) = std::fs::read_to_string(&target_path) {
                            if !value.is_empty() {
                                break value;
                            }
                        }
                        if std::time::Instant::now() >= deadline {
                            return Err(RuntimeCommandError::TargetCopyFailed);
                        }
                        std::thread::sleep(Duration::from_millis(25));
                    }
                }
                None => copy_observable_target_text(&environment, operation_timeout)
                    .map_err(|_| RuntimeCommandError::TargetCopyFailed)?,
            }
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

fn cosmic_edit_reparented_pid(fixture_path: &std::path::Path) -> Option<u32> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return None;
    };
    let fixture_str = fixture_path.display().to_string();
    for entry in entries.flatten() {
        let Some(name) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let Ok(pid) = name.parse::<u32>() else {
            continue;
        };
        let Ok(cmdline_bytes) = std::fs::read(format!("/proc/{pid}/cmdline")) else {
            continue;
        };
        let cmdline = cmdline_bytes
            .split(|&b| b == 0)
            .filter(|s| !s.is_empty())
            .map(|s| String::from_utf8_lossy(s))
            .collect::<Vec<_>>()
            .join(" ");
        if cmdline.contains("cosmic-edit") && cmdline.contains(&fixture_str) {
            return Some(pid);
        }
    }
    None
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
    command.args(&flags);
    if target == RuntimeTargetKind::Firefox {
        command.env("MOZ_ENABLE_WAYLAND", "0");
    }
    if target == RuntimeTargetKind::CosmicEdit {
        command
            .env("XDG_CONFIG_HOME", fixture.profile_dir.join("config"))
            .env("XDG_STATE_HOME", fixture.profile_dir.join("state"))
            .env("XDG_DATA_HOME", fixture.profile_dir.join("data"));
        for directory in ["config", "state", "data"] {
            if let Err(error) = std::fs::create_dir_all(fixture.profile_dir.join(directory)) {
                return RuntimeTargetResult {
                    status: RuntimeMatrixStatus::Failed,
                    reason: format!("COSMIC Edit profile isolation failed: {error}"),
                    observation: None,
                };
            }
        }
    }
    if target == RuntimeTargetKind::CosmicEdit
        && cosmic_edit_reparented_pid(&fixture.profile_dir.join("controlled-target.txt")).is_some()
    {
        return RuntimeTargetResult {
            status: RuntimeMatrixStatus::Failed,
            reason: "a COSMIC Edit process already owns the controlled fixture".to_string(),
            observation: None,
        };
    }
    command
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
    let effective_pid = child.pid();
    match child.healthy() {
        Ok(true) => {
            if target == RuntimeTargetKind::CosmicEdit {
                if let Some(editor_pid) =
                    cosmic_edit_reparented_pid(&fixture.profile_dir.join("controlled-target.txt"))
                {
                    let owned_pids = discover_owned_process_tree(target, child.pid())
                        .into_iter()
                        .map(|role| role.pid)
                        .collect::<std::collections::HashSet<_>>();
                    if !owned_pids.contains(&editor_pid) {
                        let teardown = child.terminate_and_reap(teardown_timeout);
                        return RuntimeTargetResult {
                            status: RuntimeMatrixStatus::Failed,
                            reason: match teardown {
                                Ok(()) => format!(
                                    "COSMIC Edit escaped the owned process group: pid {editor_pid}"
                                ),
                                Err(error) => format!(
                                    "COSMIC Edit escaped the owned process group: pid {editor_pid}; teardown failed: {error:?}"
                                ),
                            },
                            observation: None,
                        };
                    }
                }
            }
        }
        Ok(false) if target == RuntimeTargetKind::CosmicEdit => {
            let escaped =
                cosmic_edit_reparented_pid(&fixture.profile_dir.join("controlled-target.txt"));
            let teardown = child.terminate_and_reap(teardown_timeout);
            return RuntimeTargetResult {
                status: RuntimeMatrixStatus::Failed,
                reason: match (escaped, teardown) {
                    (Some(pid), Ok(())) => format!(
                        "COSMIC Edit daemonized outside the owned process group: pid {pid}"
                    ),
                    (Some(pid), Err(error)) => format!(
                        "COSMIC Edit daemonized outside the owned process group: pid {pid}; owned-group teardown failed: {error:?}"
                    ),
                    (None, Ok(())) => "COSMIC Edit exited during startup".to_string(),
                    (None, Err(error)) => format!(
                        "COSMIC Edit exited during startup; teardown failed: {error:?}"
                    ),
                },
                observation: None,
            };
        }
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

    let roles = discover_owned_process_tree(target, effective_pid);
    let target_version = if target == RuntimeTargetKind::CosmicEdit {
        "installed; version unavailable".to_string()
    } else {
        tool_version(executable)
    };
    let manifest = render_target_manifest(
        target,
        &target_version,
        &flags,
        &environment.session_type,
        "controlled-text-sequence-v1",
        &roles,
    );
    let _ = std::fs::write(fixture.artifact_dir.join("target-manifest.txt"), manifest);

    let scenario_pid = effective_pid;
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
            let still_running = child.healthy().unwrap_or(false)
                || (target == RuntimeTargetKind::CosmicEdit
                    && std::path::Path::new(&format!("/proc/{effective_pid}")).exists());
            let expected_submit_exit = matches!(
                target,
                RuntimeTargetKind::Gtk | RuntimeTargetKind::Qt | RuntimeTargetKind::Terminal
            ) && child.completed_successfully();
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
        RuntimeTargetKind::Edge,
        RuntimeTargetKind::Firefox,
        RuntimeTargetKind::Electron,
        RuntimeTargetKind::CosmicEdit,
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
        "firefox",
        "electron",
        "cosmic-edit",
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
    .with_text_observation(&result.expected_text, &result.observed_text);
    artifact
        .write_to(&artifact_directory.join("wayland-wtype-success.txt"))
        .expect("Wayland runtime artifact must be persisted");
}

#[derive(Debug)]
struct DirectUinputProtocolEvidence {
    ping_response: String,
    after_type: String,
    after_paste: String,
    after_backspace: String,
    after_replace: String,
    clipboard_preserved: bool,
    modifiers_released: bool,
}

fn send_direct_uinput_command(
    socket_path: &std::path::Path,
    command: &str,
    timeout: Duration,
) -> Result<String, RuntimeCommandError> {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;

    let mut stream =
        UnixStream::connect(socket_path).map_err(|_| RuntimeCommandError::PrerequisiteMissing)?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|_| RuntimeCommandError::ScenarioFailed)?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|_| RuntimeCommandError::ScenarioFailed)?;
    writeln!(stream, "{command}").map_err(|_| RuntimeCommandError::ScenarioFailed)?;

    let mut response = String::new();
    BufReader::new(stream)
        .read_line(&mut response)
        .map_err(|_| RuntimeCommandError::ScenarioFailed)?;
    Ok(response.trim().to_string())
}

fn wait_for_controlled_browser_value(
    cdp: &mut CdpConnection,
    expected: &str,
    timeout: Duration,
    evidence_path: &std::path::Path,
) -> Result<String, RuntimeCommandError> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        let observed = read_controlled_browser_target(cdp)?;
        if observed == expected {
            return Ok(observed);
        }
        if std::time::Instant::now() >= deadline {
            let _ = std::fs::write(
                evidence_path,
                format!("expected={expected:?}\nobserved={observed:?}\n"),
            );
            return Err(RuntimeCommandError::TimedOut);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn run_direct_uinput_protocol_scenario(
    timeout: Duration,
) -> Result<DirectUinputProtocolEvidence, RuntimeCommandError> {
    let environment = RuntimeEnvironment::detect_current();
    if !controlled_display_available(&environment) {
        return Err(RuntimeCommandError::PrerequisiteMissing);
    }

    let runtime_dir =
        std::env::var_os("XDG_RUNTIME_DIR").ok_or(RuntimeCommandError::PrerequisiteMissing)?;
    let socket_path = std::path::PathBuf::from(runtime_dir)
        .join("vietc")
        .join("uinput.sock");
    if !socket_path.exists() {
        return Err(RuntimeCommandError::PrerequisiteMissing);
    }

    let target = RuntimeTargetKind::Chromium;
    let executable = runtime_target_executable(target, &environment)
        .ok_or(RuntimeCommandError::PrerequisiteMissing)?;
    let workspace_root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .ok_or(RuntimeCommandError::ScenarioFailed)?;
    let run_id = format!("direct-uinput-{}", std::process::id());
    let fixture = ControlledRuntimeFixture::isolated(workspace_root, &run_id)
        .map_err(|_| RuntimeCommandError::ScenarioFailed)?;
    fixture
        .prepare(workspace_root)
        .map_err(|_| RuntimeCommandError::ScenarioFailed)?;
    prepare_target_fixture(target, &fixture).map_err(|_| RuntimeCommandError::ScenarioFailed)?;
    let flags = runtime_target_flags(target, &fixture, executable);

    let stdout = std::fs::File::create(fixture.artifact_dir.join("application.stdout.log"))
        .map_err(|_| RuntimeCommandError::ScenarioFailed)?;
    let stderr = std::fs::File::create(fixture.artifact_dir.join("application.stderr.log"))
        .map_err(|_| RuntimeCommandError::ScenarioFailed)?;
    let mut command = std::process::Command::new(executable);
    command
        .args(&flags)
        .stdin(std::process::Stdio::null())
        .stdout(stdout)
        .stderr(stderr);
    let mut child = OwnedRuntimeChild::spawn(&mut command)?;

    let trace_path = fixture.artifact_dir.join("direct-uinput-trace.txt");
    let trace = |step: &str| {
        use std::io::Write;
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&trace_path)
        {
            let _ = writeln!(file, "{step}");
        }
    };

    let scenario_result = (|| {
        trace("cdp-connect:start");
        let mut cdp = connect_controlled_browser_cdp(&fixture, timeout)?;
        trace("cdp-connect:done");
        prepare_controlled_browser_target(&mut cdp)?;
        trace("cdp-prepare:done");

        let clipboard_snapshot = "vietc-direct-uinput-clipboard".to_string();
        clipboard_write(&environment, &clipboard_snapshot, timeout)
            .map_err(|_| RuntimeCommandError::ClipboardSnapshotFailed)?;
        if clipboard_read(&environment, timeout)
            .map_err(|_| RuntimeCommandError::ClipboardSnapshotFailed)?
            != clipboard_snapshot
        {
            return Err(RuntimeCommandError::ClipboardSnapshotFailed);
        }

        trace("clipboard-seed:done");
        prepare_controlled_browser_target(&mut cdp)?;
        std::thread::sleep(Duration::from_millis(500));
        trace("target-refocus:done");
        let ping_response = send_direct_uinput_command(&socket_path, "PING", timeout)?;
        trace("ping:done");
        if ping_response != "PONG" {
            return Err(RuntimeCommandError::ScenarioFailed);
        }

        if send_direct_uinput_command(&socket_path, "TYPE:hello", timeout)? != "OK" {
            return Err(RuntimeCommandError::ScenarioFailed);
        }
        trace("type-command:done");
        let after_type = wait_for_controlled_browser_value(
            &mut cdp,
            "hello",
            timeout,
            &fixture.artifact_dir.join("type-timeout.txt"),
        )?;
        trace("type-observation:done");

        if send_direct_uinput_command(&socket_path, "PASTE: tiếng Việt", timeout)? != "OK" {
            return Err(RuntimeCommandError::ScenarioFailed);
        }
        trace("paste-command:done");
        let after_paste = wait_for_controlled_browser_value(
            &mut cdp,
            "hello tiếng Việt",
            timeout,
            &fixture.artifact_dir.join("paste-timeout.txt"),
        )?;
        trace("paste-observation:done");

        if send_direct_uinput_command(&socket_path, "BACKSPACE:1", timeout)? != "OK" {
            return Err(RuntimeCommandError::ScenarioFailed);
        }
        trace("backspace-command:done");
        let after_backspace = wait_for_controlled_browser_value(
            &mut cdp,
            "hello tiếng Việ",
            timeout,
            &fixture.artifact_dir.join("backspace-timeout.txt"),
        )?;
        trace("backspace-observation:done");

        if send_direct_uinput_command(&socket_path, "REPLACE:9:thời", timeout)? != "OK" {
            return Err(RuntimeCommandError::ScenarioFailed);
        }
        trace("replace-command:done");
        let after_replace = wait_for_controlled_browser_value(
            &mut cdp,
            "hello thời",
            timeout,
            &fixture.artifact_dir.join("replace-timeout.txt"),
        )?;
        trace("replace-observation:done");

        clipboard_write(&environment, &clipboard_snapshot, timeout)
            .map_err(|_| RuntimeCommandError::ClipboardRestoreFailed)?;
        let clipboard_preserved = clipboard_read(&environment, timeout)
            .map_err(|_| RuntimeCommandError::ClipboardVerificationFailed)?
            == clipboard_snapshot;
        let modifiers_released = verify_synthesized_modifiers_released(&environment, timeout)
            .map_err(|_| RuntimeCommandError::ModifierReleaseFailed)?;

        let evidence = DirectUinputProtocolEvidence {
            ping_response,
            after_type,
            after_paste,
            after_backspace,
            after_replace,
            clipboard_preserved,
            modifiers_released,
        };
        std::fs::write(
            fixture.artifact_dir.join("direct-uinput-protocol.txt"),
            format!(
                "ping={:?}\nafter_type={:?}\nafter_paste={:?}\nafter_backspace={:?}\nafter_replace={:?}\nclipboard_preserved={}\nmodifiers_released={}\n",
                evidence.ping_response,
                evidence.after_type,
                evidence.after_paste,
                evidence.after_backspace,
                evidence.after_replace,
                evidence.clipboard_preserved,
                evidence.modifiers_released,
            ),
        )
        .map_err(|_| RuntimeCommandError::ScenarioFailed)?;

        let _ = cdp.command("Browser.close", serde_json::json!({}));
        Ok(evidence)
    })();

    let teardown = child.terminate_and_reap(Duration::from_secs(5));
    match (scenario_result, teardown) {
        (Ok(evidence), Ok(())) => Ok(evidence),
        (Err(error), _) => Err(error),
        (_, Err(error)) => Err(error),
    }
}

#[test]
#[ignore = "requires VIETC_RUNTIME_TESTS=1, VIETC_CONTROLLED_DISPLAY=1, a running vietc-uinputd, Chromium, and a controlled Wayland session"]
fn privileged_uinput_socket_commands_are_observable_end_to_end() {
    let evidence = run_direct_uinput_protocol_scenario(Duration::from_secs(30))
        .expect("direct uinput protocol scenario must complete");

    assert_eq!(evidence.ping_response, "PONG");
    assert_eq!(evidence.after_type, "hello");
    assert_eq!(evidence.after_paste, "hello tiếng Việt");
    assert_eq!(evidence.after_backspace, "hello tiếng Việ");
    assert_eq!(evidence.after_replace, "hello thời");
    assert!(evidence.clipboard_preserved);
    assert!(evidence.modifiers_released);
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FocusSwitchObservation {
    target: String,
    expected: String,
    observed: String,
}

fn verify_focus_switch_observations(
    observations: &[FocusSwitchObservation],
    initial_target: &str,
    final_target: &str,
) -> Result<(), &'static str> {
    if observations.len() < 3 {
        return Err("focus-switch requires at least three observations");
    }
    if initial_target != final_target {
        return Err("focus-switch did not return to the initial target");
    }
    if observations
        .iter()
        .any(|observation| observation.expected != observation.observed)
    {
        return Err("focus-switch observable text mismatch");
    }
    for (index, observation) in observations.iter().enumerate() {
        if observations.iter().enumerate().any(|(other_index, other)| {
            index != other_index
                && observation.target != other.target
                && observation.observed.contains(&other.expected)
        }) {
            return Err("focus-switch leaked text between targets");
        }
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FastInputCase {
    name: &'static str,
    raw: &'static str,
    expected: &'static str,
}

fn controlled_fast_input_cases() -> [FastInputCase; 7] {
    [
        FastInputCase {
            name: "free-order-tone-mark",
            raw: "dduojwc",
            expected: "được",
        },
        FastInputCase {
            name: "mark-then-tone",
            raw: "neues",
            expected: "nếu",
        },
        FastInputCase {
            name: "compound-tone",
            raw: "tieengs",
            expected: "tiếng",
        },
        FastInputCase {
            name: "boundary",
            raw: "dduojwc ",
            expected: "được ",
        },
        FastInputCase {
            name: "backspace",
            raw: "dduojwcx\u{8}s",
            expected: "đướcs",
        },
        FastInputCase {
            name: "disabled-literal",
            raw: "dduojwc",
            expected: "dduojwc",
        },
        FastInputCase {
            name: "macro-isolation",
            raw: "ko",
            expected: "ko",
        },
    ]
}

fn verify_fast_input_observations(
    observations: &[(&str, &str, &str)],
    key_delay_ms: u64,
) -> Result<(), &'static str> {
    if !(1..=20).contains(&key_delay_ms) {
        return Err("fast-input key delay is outside the controlled bound");
    }
    if observations.is_empty() {
        return Err("fast-input observations are empty");
    }
    if observations
        .iter()
        .any(|(_, expected, observed)| expected != observed)
    {
        return Err("fast-input observable text mismatch");
    }
    Ok(())
}

fn activate_stress_browser(
    fixture: &ControlledRuntimeFixture,
    timeout: Duration,
) -> Result<(), RuntimeCommandError> {
    let run_id = fixture
        .profile_dir
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .ok_or(RuntimeCommandError::ScenarioFailed)?;
    let title = format!("Vietc controlled runtime {run_id}");
    common::cosmic_toplevel::activate_chromium(&title, timeout)
        .map(|_| ())
        .map_err(|_| RuntimeCommandError::TimedOut)
}

fn switch_stress_browser(timeout: Duration) -> Result<(), RuntimeCommandError> {
    let mut keyboard =
        common::virtual_keyboard::VirtualKeyboard::create("vietc-phase6-focus-switch-vk")
            .map_err(|_| RuntimeCommandError::InputInjectionFailed)?;
    keyboard
        .wait_for_devnode(timeout)
        .ok_or(RuntimeCommandError::InputInjectionFailed)?;
    std::thread::sleep(Duration::from_secs(1));
    keyboard
        .alt_tab()
        .map_err(|_| RuntimeCommandError::InputInjectionFailed)?;
    std::thread::sleep(Duration::from_millis(750));
    Ok(())
}

fn inject_controlled_keys(
    environment: &RuntimeEnvironment,
    text: &str,
    key_delay_ms: u64,
    timeout: Duration,
) -> Result<(), RuntimeCommandError> {
    if environment.session_type == "wayland" {
        let mut keyboard =
            common::virtual_keyboard::VirtualKeyboard::create("vietc-phase6-runtime-vk")
                .map_err(|_| RuntimeCommandError::InputInjectionFailed)?;
        keyboard
            .wait_for_devnode(timeout)
            .ok_or(RuntimeCommandError::InputInjectionFailed)?;

        // Keep the virtual device alive long enough for the production daemon's
        // 100 ms hotplug scan to discover and grab it before emitting keys.
        std::thread::sleep(Duration::from_secs(1));
        for ch in text.chars() {
            keyboard
                .type_char(ch)
                .map_err(|_| RuntimeCommandError::InputInjectionFailed)?;
            std::thread::sleep(Duration::from_millis(key_delay_ms));
        }
        // Do not destroy the uinput device until the daemon has drained the
        // emitted events; dropping it immediately produces ENODEV and loses input.
        std::thread::sleep(Duration::from_millis(500));
        Ok(())
    } else {
        let mut command = std::process::Command::new("xdotool");
        command.args([
            "type",
            "--clearmodifiers",
            "--delay",
            &key_delay_ms.to_string(),
            "--",
            text,
        ]);
        if run_command_with_timeout(&mut command, timeout)?.success() {
            Ok(())
        } else {
            Err(RuntimeCommandError::InputInjectionFailed)
        }
    }
}

fn launch_stress_browser_target(
    workspace_root: &Path,
    run_id: &str,
    target: RuntimeTargetKind,
    environment: &RuntimeEnvironment,
    timeout: Duration,
) -> Result<(ControlledRuntimeFixture, OwnedRuntimeChild, CdpConnection), RuntimeCommandError> {
    if !matches!(
        target,
        RuntimeTargetKind::Chromium | RuntimeTargetKind::Chrome | RuntimeTargetKind::Edge
    ) {
        return Err(RuntimeCommandError::PrerequisiteMissing);
    }
    let executable = runtime_target_executable(target, environment)
        .ok_or(RuntimeCommandError::PrerequisiteMissing)?;
    let fixture = ControlledRuntimeFixture::isolated(workspace_root, run_id)
        .map_err(|_| RuntimeCommandError::ScenarioFailed)?;
    fixture
        .prepare(workspace_root)
        .map_err(|_| RuntimeCommandError::ScenarioFailed)?;
    prepare_target_fixture(target, &fixture).map_err(|_| RuntimeCommandError::ScenarioFailed)?;

    let stdout = std::fs::File::create(fixture.artifact_dir.join("application.stdout.log"))
        .map_err(|_| RuntimeCommandError::ScenarioFailed)?;
    let stderr = std::fs::File::create(fixture.artifact_dir.join("application.stderr.log"))
        .map_err(|_| RuntimeCommandError::ScenarioFailed)?;
    let mut command = std::process::Command::new(executable);
    command
        .args(runtime_target_flags(target, &fixture, executable))
        .stdin(std::process::Stdio::null())
        .stdout(stdout)
        .stderr(stderr);
    let mut child = OwnedRuntimeChild::spawn(&mut command)?;
    if !child.healthy()? {
        return Err(RuntimeCommandError::ScenarioFailed);
    }

    let mut cdp = connect_controlled_browser_cdp(&fixture, timeout)?;
    prepare_controlled_browser_target(&mut cdp)?;
    Ok((fixture, child, cdp))
}

fn launch_stress_browser(
    workspace_root: &Path,
    run_id: &str,
    environment: &RuntimeEnvironment,
    timeout: Duration,
) -> Result<(ControlledRuntimeFixture, OwnedRuntimeChild, CdpConnection), RuntimeCommandError> {
    launch_stress_browser_target(
        workspace_root,
        run_id,
        RuntimeTargetKind::Chromium,
        environment,
        timeout,
    )
}

#[test]
#[ignore = "requires VIETC_RUNTIME_TESTS=1, VIETC_CONTROLLED_DISPLAY=1, Chromium, a running vietc daemon, and writable /dev/uinput"]
fn privileged_focus_switch_stress_is_observable_end_to_end() {
    let environment = RuntimeEnvironment::detect_current();
    assert!(controlled_display_available(&environment));
    assert!(environment.has_tool("chromium") || environment.has_tool("chromium-browser"));

    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root");
    let timeout = Duration::from_secs(30);
    let (fixture_a, mut child_a, mut cdp_a) =
        launch_stress_browser(workspace_root, "focus-switch-a", &environment, timeout)
            .expect("first controlled target must launch");
    let (fixture_b, mut child_b, mut cdp_b) =
        launch_stress_browser(workspace_root, "focus-switch-b", &environment, timeout)
            .expect("second controlled target must launch");

    let scenario = (|| {
        prepare_controlled_browser_target(&mut cdp_a)?;
        activate_stress_browser(&fixture_a, timeout)?;
        std::thread::sleep(Duration::from_millis(750));
        inject_controlled_keys(&environment, "tieengs", 12, timeout)?;
        let first_a = match wait_for_controlled_browser_value(
            &mut cdp_a,
            "tiếng",
            timeout,
            &fixture_a.artifact_dir.join("first-a-timeout.txt"),
        ) {
            Ok(observed) => observed,
            Err(error) => {
                let observed_a = read_controlled_browser_target(&mut cdp_a)
                    .unwrap_or_else(|_| "<unavailable>".to_string());
                let observed_b = read_controlled_browser_target(&mut cdp_b)
                    .unwrap_or_else(|_| "<unavailable>".to_string());
                let _ = std::fs::write(
                    fixture_a
                        .artifact_dir
                        .join("first-a-cross-target-timeout.txt"),
                    format!(
                        "expected_a={:?}\nobserved_a={:?}\nobserved_b={:?}\n",
                        "tiếng", observed_a, observed_b
                    ),
                );
                return Err(error);
            }
        };

        prepare_controlled_browser_target(&mut cdp_b)?;
        activate_stress_browser(&fixture_b, timeout)?;
        std::thread::sleep(Duration::from_millis(750));
        inject_controlled_keys(&environment, "chaof", 12, timeout)?;
        let observed_b = match wait_for_controlled_browser_value(
            &mut cdp_b,
            "chào",
            timeout,
            &fixture_b.artifact_dir.join("target-b-timeout.txt"),
        ) {
            Ok(observed) => observed,
            Err(error) => {
                let observed_a = read_controlled_browser_target(&mut cdp_a)
                    .unwrap_or_else(|_| "<unavailable>".to_string());
                let observed_b = read_controlled_browser_target(&mut cdp_b)
                    .unwrap_or_else(|_| "<unavailable>".to_string());
                let _ = std::fs::write(
                    fixture_b
                        .artifact_dir
                        .join("target-b-cross-target-timeout.txt"),
                    format!(
                        "expected_b={:?}\nobserved_a={:?}\nobserved_b={:?}\n",
                        "chào", observed_a, observed_b
                    ),
                );
                return Err(error);
            }
        };

        activate_stress_browser(&fixture_a, timeout)?;
        std::thread::sleep(Duration::from_millis(750));
        let final_a = wait_for_controlled_browser_value(
            &mut cdp_a,
            "tiếng",
            timeout,
            &fixture_a.artifact_dir.join("final-a-timeout.txt"),
        )?;

        let observations = [
            FocusSwitchObservation {
                target: "chromium-a".to_string(),
                expected: "tiếng".to_string(),
                observed: first_a,
            },
            FocusSwitchObservation {
                target: "chromium-b".to_string(),
                expected: "chào".to_string(),
                observed: observed_b,
            },
            FocusSwitchObservation {
                target: "chromium-a".to_string(),
                expected: "tiếng".to_string(),
                observed: final_a,
            },
        ];
        verify_focus_switch_observations(&observations, "chromium-a", "chromium-a")
            .map_err(|_| RuntimeCommandError::ScenarioFailed)?;

        std::fs::write(
            workspace_root.join("target/runtime-artifacts/phase6-focus-switch-observed.txt"),
            format!(
                "key_delay_ms=12\ninitial_target=chromium-a\ninitial_input=tieengs\ninitial_expected=tiếng\nsecond_target=chromium-b\nsecond_input=chaof\nsecond_expected=chào\nfinal_target=chromium-a\nfinal_expected=tiếng\nobservations={observations:#?}\nstatus=PASS\n"
            ),
        )
        .map_err(|_| RuntimeCommandError::ScenarioFailed)?;
        Ok::<(), RuntimeCommandError>(())
    })();

    let _ = cdp_a.command("Browser.close", serde_json::json!({}));
    let _ = cdp_b.command("Browser.close", serde_json::json!({}));
    let teardown_a = child_a.terminate_and_reap(Duration::from_secs(5));
    let teardown_b = child_b.terminate_and_reap(Duration::from_secs(5));
    scenario.expect("focus-switch stress must pass exact observation");
    teardown_a.expect("first controlled target must be reaped");
    teardown_b.expect("second controlled target must be reaped");
}

#[test]
#[ignore = "requires VIETC_RUNTIME_TESTS=1, VIETC_CONTROLLED_DISPLAY=1, Chromium, a running vietc daemon, and writable /dev/uinput"]
fn privileged_fast_input_stress_is_observable_end_to_end() {
    let environment = RuntimeEnvironment::detect_current();
    assert!(controlled_display_available(&environment));
    assert!(environment.has_tool("chromium") || environment.has_tool("chromium-browser"));

    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root");
    let timeout = Duration::from_secs(30);
    let key_delay_ms = 8;
    let (fixture, mut child, mut cdp) =
        launch_stress_browser(workspace_root, "fast-input", &environment, timeout)
            .expect("controlled fast-input target must launch");

    let scenario = (|| {
        let mut observations = Vec::new();
        for case in controlled_fast_input_cases()
            .into_iter()
            .filter(|case| !matches!(case.name, "backspace" | "disabled-literal"))
        {
            prepare_controlled_browser_target(&mut cdp)?;
            activate_stress_browser(&fixture, timeout)?;
            inject_controlled_keys(&environment, case.raw, key_delay_ms, timeout)?;
            let observed = wait_for_controlled_browser_value(
                &mut cdp,
                case.expected,
                timeout,
                &fixture
                    .artifact_dir
                    .join(format!("{}-timeout.txt", case.name)),
            )?;
            observations.push((case.name, case.expected.to_string(), observed));
        }

        prepare_controlled_browser_target(&mut cdp)?;
        activate_stress_browser(&fixture, timeout)?;
        inject_controlled_keys(&environment, "dduojwc", key_delay_ms, timeout)?;
        wait_for_controlled_browser_value(
            &mut cdp,
            "được",
            timeout,
            &fixture.artifact_dir.join("backspace-before-timeout.txt"),
        )?;

        let mut backspace =
            common::virtual_keyboard::VirtualKeyboard::create("vietc-phase6-backspace-vk")
                .map_err(|_| RuntimeCommandError::InputInjectionFailed)?;
        backspace
            .wait_for_devnode(timeout)
            .ok_or(RuntimeCommandError::InputInjectionFailed)?;
        std::thread::sleep(Duration::from_secs(1));
        backspace
            .backspace()
            .map_err(|_| RuntimeCommandError::InputInjectionFailed)?;
        std::thread::sleep(Duration::from_millis(500));

        let backspace_observed = wait_for_controlled_browser_value(
            &mut cdp,
            "đượ",
            timeout,
            &fixture.artifact_dir.join("backspace-after-timeout.txt"),
        )?;
        observations.push(("backspace", "đượ".to_string(), backspace_observed));

        let status_path = dirs::config_dir()
            .ok_or(RuntimeCommandError::PrerequisiteMissing)?
            .join("vietc/status");
        std::fs::write(&status_path, "en").map_err(|_| RuntimeCommandError::ScenarioFailed)?;
        std::thread::sleep(Duration::from_secs(1));
        prepare_controlled_browser_target(&mut cdp)?;
        activate_stress_browser(&fixture, timeout)?;
        inject_controlled_keys(&environment, "dduojwc", key_delay_ms, timeout)?;
        let disabled = wait_for_controlled_browser_value(
            &mut cdp,
            "dduojwc",
            timeout,
            &fixture.artifact_dir.join("disabled-timeout.txt"),
        )?;
        observations.push(("disabled-literal", "dduojwc".to_string(), disabled));
        std::fs::write(&status_path, "vn").map_err(|_| RuntimeCommandError::ScenarioFailed)?;
        std::thread::sleep(Duration::from_secs(1));

        let borrowed = observations
            .iter()
            .map(|(name, expected, observed)| (*name, expected.as_str(), observed.as_str()))
            .collect::<Vec<_>>();
        verify_fast_input_observations(&borrowed, key_delay_ms)
            .map_err(|_| RuntimeCommandError::ScenarioFailed)?;

        std::fs::write(
            workspace_root.join("target/runtime-artifacts/phase6-fast-input-observed.txt"),
            format!("key_delay_ms={key_delay_ms}\nobservations={observations:#?}\nstatus=PASS\n"),
        )
        .map_err(|_| RuntimeCommandError::ScenarioFailed)?;
        Ok::<(), RuntimeCommandError>(())
    })();

    if let Some(config_dir) = dirs::config_dir() {
        let _ = std::fs::write(config_dir.join("vietc/status"), "vn");
    }
    let _ = cdp.command("Browser.close", serde_json::json!({}));
    let teardown = child.terminate_and_reap(Duration::from_secs(5));
    scenario.expect("fast-input stress must pass exact observation");
    teardown.expect("controlled fast-input target must be reaped");
}

const LONG_TEXT_MIN_WORDS: usize = 700;
const LONG_TEXT_MAX_WORDS: usize = 1000;
const DEFAULT_LONG_TEXT_KEY_DELAY_MS: u64 = 85;
const MIN_LONG_TEXT_KEY_DELAY_MS: u64 = 40;
const MAX_LONG_TEXT_KEY_DELAY_MS: u64 = 250;

fn long_text_key_delay_ms() -> u64 {
    std::env::var("VIETC_LONG_TEXT_KEY_DELAY_MS")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|delay| (MIN_LONG_TEXT_KEY_DELAY_MS..=MAX_LONG_TEXT_KEY_DELAY_MS).contains(delay))
        .unwrap_or(DEFAULT_LONG_TEXT_KEY_DELAY_MS)
}

fn controlled_long_text_input() -> String {
    const PARAGRAPH: &str = "Mooix sangs, laapj trinhf vieen moowr mays, docj email, kieemr tra lichj hopj vaf xem dashboard cuar duwj ans. Sau ddoos, hoj pull source code, chayj test, review pull request vaf ghi chus casc looix caanf suwar. Mootj soos teen tieengs Anh nhu test, suite, status, issue, user, feature, release, database vaf service dduocwj giuwx nguyeen ddeer phast hieenj truowngf hopwj Telex dieenx giair nhaamf. Trong lucs lamf vieecj, Minh vieets hamf mowis, Huowng capj nhaatj API, conf An kieemr tra log treen mooi truowngf staging. Car nhoms trao ddooir bawngf tieengs Vieetj cos chuwx hoa vaf chuwx thuowngf, vis duj Hoom Nay Chungs Tooi Suwar Looix, rooif xasc nhaanj keets quar bawngf unit test, integration test vaf cargo check. Buooir chieeuf, moi nguowif xuwr lys phanr hooif, toois uuw hieeuj nawng, capj nhaatj taif lieeuj, chuaanr bij banr phast hanhf vaf toongr keets nhuwngx vieecj ddax hoanf thanhf.";
    std::iter::repeat_n(PARAGRAPH, 6)
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn controlled_long_text() -> String {
    const PARAGRAPH: &str = "Mỗi sáng, lập trình viên mở máy, đọc email, kiểm tra lịch họp và xem dashboard của dự án. Sau đó, họ pull source code, chạy test, review pull request và ghi chú các lỗi cần sửa. Một số tên tiếng Anh như test, suite, status, issue, user, feature, release, database và service được giữ nguyên để phát hiện trường hợp Telex diễn giải nhầm. Trong lúc làm việc, Minh viết hàm mới, Hương cập nhật API, còn An kiểm tra log trên môi trường staging. Cả nhóm trao đổi bằng tiếng Việt có chữ hoa và chữ thường, ví dụ Hôm Nay Chúng Tôi Sửa Lỗi, rồi xác nhận kết quả bằng unit test, integration test và cargo check. Buổi chiều, mọi người xử lý phản hồi, tối ưu hiệu năng, cập nhật tài liệu, chuẩn bị bản phát hành và tổng kết những việc đã hoàn thành.";
    std::iter::repeat_n(PARAGRAPH, 6)
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn summarize_text_mismatch(expected: &str, observed: &str) -> String {
    if expected == observed {
        return "none".to_string();
    }

    let first_difference = expected
        .chars()
        .zip(observed.chars())
        .position(|(expected, observed)| expected != observed)
        .unwrap_or_else(|| expected.chars().count().min(observed.chars().count()));
    format!(
        "first_difference_char={first_difference}; expected_chars={}; observed_chars={}",
        expected.chars().count(),
        observed.chars().count()
    )
}

fn render_long_text_artifact(
    target: RuntimeTargetKind,
    key_delay_ms: u64,
    expected: &str,
    observed: Option<&str>,
    status: RuntimeMatrixStatus,
    error: Option<&str>,
) -> String {
    let observed = observed.unwrap_or("");
    format!(
        "target={target:?}\nkey_delay_ms={key_delay_ms}\nstatus={}\nerror={}\nword_count={}\nmismatch_summary={}\nexpected_text={expected:?}\nobserved_text={observed:?}\n",
        status.label(),
        error.unwrap_or("none"),
        expected.split_whitespace().count(),
        summarize_text_mismatch(expected, observed),
    )
}

fn selected_long_text_targets() -> Result<Vec<RuntimeTargetKind>, String> {
    let configured = std::env::var("VIETC_LONG_TEXT_TARGETS")
        .unwrap_or_else(|_| "cosmic-edit,microsoft-edge,firefox".to_string());
    configured
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| match name {
            "cosmic-edit" => Ok(RuntimeTargetKind::CosmicEdit),
            "microsoft-edge" => Ok(RuntimeTargetKind::Edge),
            "firefox" => Ok(RuntimeTargetKind::Firefox),
            _ => Err(format!("unsupported VIETC_LONG_TEXT_TARGETS value: {name}")),
        })
        .collect()
}

#[test]
fn deterministic_long_text_fixture_has_required_length_language_and_telex_risks() {
    let input = controlled_long_text_input();
    let text = controlled_long_text();
    let word_count = text.split_whitespace().count();

    assert_eq!(input.split_whitespace().count(), word_count);
    assert!(
        input.is_ascii(),
        "physical Telex input must contain only ASCII"
    );
    assert!((LONG_TEXT_MIN_WORDS..=LONG_TEXT_MAX_WORDS).contains(&word_count));
    for required in [
        "Mỗi sáng",
        "Hôm Nay Chúng Tôi Sửa Lỗi",
        "test",
        "suite",
        "status",
        "user",
        "feature",
        "database",
        "service",
    ] {
        assert!(
            text.contains(required),
            "missing controlled corpus token {required:?}"
        );
    }
}

#[test]
fn deterministic_long_text_delay_and_artifact_are_bounded_and_exact() {
    assert!((MIN_LONG_TEXT_KEY_DELAY_MS..=MAX_LONG_TEXT_KEY_DELAY_MS)
        .contains(&long_text_key_delay_ms()));

    let expected = controlled_long_text();
    let artifact = render_long_text_artifact(
        RuntimeTargetKind::Edge,
        DEFAULT_LONG_TEXT_KEY_DELAY_MS,
        &expected,
        Some(&expected),
        RuntimeMatrixStatus::Pass,
        None,
    );
    assert!(artifact.contains("target=Edge"));
    assert!(artifact.contains("key_delay_ms=85"));
    assert!(artifact.contains("status=PASS"));
    assert!(artifact.contains("mismatch_summary=none"));
    assert!(artifact.contains(&format!("expected_text={expected:?}")));
    assert!(artifact.contains(&format!("observed_text={expected:?}")));

    assert!(summarize_text_mismatch("test", "tést").contains("first_difference_char=1"));
}

fn write_long_text_result(
    artifact_directory: &Path,
    target: RuntimeTargetKind,
    key_delay_ms: u64,
    expected: &str,
    observed: Option<&str>,
    status: RuntimeMatrixStatus,
    error: Option<&str>,
) -> Result<(), RuntimeCommandError> {
    std::fs::create_dir_all(artifact_directory).map_err(|_| RuntimeCommandError::ScenarioFailed)?;
    std::fs::write(
        artifact_directory.join(format!("long-text-{}.txt", target.executable())),
        render_long_text_artifact(target, key_delay_ms, expected, observed, status, error),
    )
    .map_err(|_| RuntimeCommandError::ScenarioFailed)
}

fn activate_long_text_target(
    target: RuntimeTargetKind,
    fixture: &ControlledRuntimeFixture,
    timeout: Duration,
) -> Result<(), RuntimeCommandError> {
    let run_id = fixture
        .profile_dir
        .parent()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .ok_or(RuntimeCommandError::ScenarioFailed)?;
    let title = format!("Vietc controlled runtime {run_id}");

    match target {
        RuntimeTargetKind::Edge => common::cosmic_toplevel::activate_chromium(&title, timeout),
        RuntimeTargetKind::Firefox => common::cosmic_toplevel::activate_firefox(&title, timeout),
        RuntimeTargetKind::CosmicEdit => {
            common::cosmic_toplevel::activate_cosmic_edit("controlled-target.txt", timeout)
        }
        _ => return Err(RuntimeCommandError::PrerequisiteMissing),
    }
    .map(|_| ())
    .map_err(|_| RuntimeCommandError::TimedOut)
}

fn launch_long_text_target(
    workspace_root: &Path,
    target: RuntimeTargetKind,
    environment: &RuntimeEnvironment,
    timeout: Duration,
) -> Result<
    (
        ControlledRuntimeFixture,
        OwnedRuntimeChild,
        Option<CdpConnection>,
    ),
    RuntimeCommandError,
> {
    let run_id = format!("long-text-{}", target.executable());
    let fixture = ControlledRuntimeFixture::isolated(workspace_root, &run_id)
        .map_err(|_| RuntimeCommandError::ScenarioFailed)?;
    fixture
        .prepare(workspace_root)
        .map_err(|_| RuntimeCommandError::ScenarioFailed)?;
    prepare_target_fixture(target, &fixture).map_err(|_| RuntimeCommandError::ScenarioFailed)?;

    let executable = runtime_target_executable(target, environment)
        .ok_or(RuntimeCommandError::PrerequisiteMissing)?;
    let stdout = std::fs::File::create(fixture.artifact_dir.join("application.stdout.log"))
        .map_err(|_| RuntimeCommandError::ScenarioFailed)?;
    let stderr = std::fs::File::create(fixture.artifact_dir.join("application.stderr.log"))
        .map_err(|_| RuntimeCommandError::ScenarioFailed)?;
    let mut command = std::process::Command::new(executable);
    command
        .args(runtime_target_flags(target, &fixture, executable))
        .stdin(std::process::Stdio::null())
        .stdout(stdout)
        .stderr(stderr);
    let mut child = OwnedRuntimeChild::spawn(&mut command)?;
    if !child.healthy()? {
        return Err(RuntimeCommandError::ScenarioFailed);
    }

    let cdp = if target == RuntimeTargetKind::Edge {
        let mut cdp = connect_controlled_browser_cdp(&fixture, timeout)?;
        prepare_controlled_browser_target(&mut cdp)?;
        Some(cdp)
    } else {
        None
    };
    Ok((fixture, child, cdp))
}

fn observe_long_text_target(
    target: RuntimeTargetKind,
    fixture: &ControlledRuntimeFixture,
    cdp: Option<&mut CdpConnection>,
    environment: &RuntimeEnvironment,
    timeout: Duration,
) -> Result<String, RuntimeCommandError> {
    match target {
        RuntimeTargetKind::Edge => {
            read_controlled_browser_target(cdp.ok_or(RuntimeCommandError::TargetCopyFailed)?)
        }
        RuntimeTargetKind::CosmicEdit => {
            let mut save =
                common::virtual_keyboard::VirtualKeyboard::create("vietc-long-text-save")
                    .map_err(|_| RuntimeCommandError::TargetCopyFailed)?;
            save.wait_for_devnode(timeout)
                .ok_or(RuntimeCommandError::TargetCopyFailed)?;
            std::thread::sleep(Duration::from_secs(1));
            save.modifier_chord(29, 31)
                .map_err(|_| RuntimeCommandError::TargetCopyFailed)?;

            let target_path = fixture.profile_dir.join("controlled-target.txt");
            let deadline = std::time::Instant::now() + timeout;
            loop {
                if let Ok(text) = std::fs::read_to_string(&target_path) {
                    if !text.is_empty() {
                        return Ok(text);
                    }
                }
                if std::time::Instant::now() >= deadline {
                    return Err(RuntimeCommandError::TimedOut);
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        RuntimeTargetKind::Firefox => copy_observable_target_text(environment, timeout),
        _ => Err(RuntimeCommandError::PrerequisiteMissing),
    }
}

#[test]
#[ignore = "requires VIETC_RUNTIME_TESTS=1, VIETC_CONTROLLED_DISPLAY=1, COSMIC Wayland, COSMIC Edit, Microsoft Edge, Firefox, a running vietc daemon, and writable /dev/uinput"]
fn privileged_long_text_is_exact_in_cosmic_edit_edge_and_firefox() {
    let environment = RuntimeEnvironment::detect_current();
    assert!(controlled_display_available(&environment));
    assert_eq!(environment.session_type, "wayland");
    assert!(environment.desktop.to_ascii_uppercase().contains("COSMIC"));

    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root");
    let artifact_directory = workspace_root.join("target/runtime-artifacts");
    let key_delay_ms = long_text_key_delay_ms();
    let input = controlled_long_text_input();
    let typing_timeout = Duration::from_millis(
        (input.chars().count() as u64 * key_delay_ms)
            .saturating_add(Duration::from_secs(30).as_millis() as u64),
    );
    let launch_timeout = Duration::from_secs(30);
    let observe_timeout = Duration::from_secs(30);
    let teardown_timeout = Duration::from_secs(5);
    let expected = controlled_long_text();
    let targets = selected_long_text_targets().expect("long-text target selection must be valid");

    for target in targets {
        let mut observed = None;
        let mut child = None;
        let mut cdp = None;
        let scenario = (|| {
            let (fixture, launched_child, launched_cdp) =
                launch_long_text_target(workspace_root, target, &environment, launch_timeout)?;
            child = Some(launched_child);
            cdp = launched_cdp;

            activate_long_text_target(target, &fixture, launch_timeout)?;
            std::thread::sleep(Duration::from_secs(1));
            inject_controlled_keys(&environment, &input, key_delay_ms, typing_timeout)?;
            observed = Some(observe_long_text_target(
                target,
                &fixture,
                cdp.as_mut(),
                &environment,
                observe_timeout,
            )?);
            if observed.as_deref() == Some(expected.as_str()) {
                Ok(())
            } else {
                Err(RuntimeCommandError::ScenarioFailed)
            }
        })();

        if let Some(connection) = cdp.as_mut() {
            let _ = connection.command("Browser.close", serde_json::json!({}));
        }
        let teardown = child
            .as_mut()
            .map(|child| child.terminate_and_reap(teardown_timeout))
            .transpose();

        let final_error = match (&scenario, &teardown) {
            (Err(error), _) => Some(format!("scenario: {error:?}")),
            (Ok(()), Err(error)) => Some(format!("teardown: {error:?}")),
            (Ok(()), Ok(_)) => None,
        };
        let status = if final_error.is_none() {
            RuntimeMatrixStatus::Pass
        } else {
            RuntimeMatrixStatus::Failed
        };
        write_long_text_result(
            &artifact_directory,
            target,
            key_delay_ms,
            &expected,
            observed.as_deref(),
            status,
            final_error.as_deref(),
        )
        .expect("long-text artifact must always be written");

        scenario.expect("long-text target must match expected text exactly");
        teardown.expect("long-text target teardown must complete");
    }
}

#[test]
fn deterministic_focus_switch_rejects_stale_composition_and_cross_target_text() {
    let valid = [
        FocusSwitchObservation {
            target: "chromium-a".to_string(),
            expected: "alpha-dduoj".to_string(),
            observed: "alpha-dduoj".to_string(),
        },
        FocusSwitchObservation {
            target: "chromium-b".to_string(),
            expected: "beta-neues".to_string(),
            observed: "beta-neues".to_string(),
        },
        FocusSwitchObservation {
            target: "chromium-a".to_string(),
            expected: "alpha-return".to_string(),
            observed: "alpha-return".to_string(),
        },
    ];
    assert_eq!(
        verify_focus_switch_observations(&valid, "chromium-a", "chromium-a"),
        Ok(())
    );

    let mut stale = valid.clone();
    stale[2].observed = "beta-neuesalpha-return".to_string();
    assert_eq!(
        verify_focus_switch_observations(&stale, "chromium-a", "chromium-a"),
        Err("focus-switch observable text mismatch")
    );
    assert_eq!(
        verify_focus_switch_observations(&valid, "chromium-a", "chromium-b"),
        Err("focus-switch did not return to the initial target")
    );
}

#[test]
fn deterministic_fast_input_requires_bounded_rate_and_exact_observable_output() {
    let cases = controlled_fast_input_cases();
    assert_eq!(cases.len(), 7);
    assert!(cases.iter().any(|case| case.name == "boundary"));
    assert!(cases.iter().any(|case| case.name == "backspace"));
    assert!(cases.iter().any(|case| case.name == "disabled-literal"));
    assert!(cases.iter().any(|case| case.name == "macro-isolation"));

    let observations = cases
        .iter()
        .map(|case| (case.name, case.expected, case.expected))
        .collect::<Vec<_>>();
    assert_eq!(verify_fast_input_observations(&observations, 8), Ok(()));
    assert_eq!(
        verify_fast_input_observations(&observations, 0),
        Err("fast-input key delay is outside the controlled bound")
    );

    let mismatched = [("free-order-tone-mark", "được", "dduojwc")];
    assert_eq!(
        verify_fast_input_observations(&mismatched, 8),
        Err("fast-input observable text mismatch")
    );
}
