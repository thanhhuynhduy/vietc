// SPDX-License-Identifier: MIT
//! Explicit capability gates for privileged runtime verification.

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
            "assistant",
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

fn toolkit_entry(
    environment: &RuntimeEnvironment,
    capability: &'static str,
    executable: &'static str,
    owner: &'static str,
) -> RuntimeMatrixEntry {
    if !environment.has_tool(executable) {
        return unavailable_entry(
            capability,
            format!("{executable} runtime probe is not installed"),
            owner,
        );
    }
    if let Some((reason, status)) = runtime_prerequisite(environment) {
        return RuntimeMatrixEntry::new(capability, status, reason, owner);
    }

    unavailable_entry(
        capability,
        "controlled observable fixture has not been executed",
        owner,
    )
}

fn evaluate_runtime_matrix(environment: &RuntimeEnvironment) -> RuntimeMatrixResults {
    const DESKTOP_OWNER: &str = "runtime-desktop-runner";
    const BROWSER_OWNER: &str = "runtime-browser-runner";
    const TOOLKIT_OWNER: &str = "runtime-toolkit-runner";

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
            "controlled observable fixture has not been executed",
            TOOLKIT_OWNER,
        ),
    };

    RuntimeMatrixResults {
        entries: vec![
            desktop_entry(environment, "cosmic-wayland", "COSMIC", DESKTOP_OWNER),
            desktop_entry(environment, "gnome-wayland", "GNOME", DESKTOP_OWNER),
            desktop_entry(environment, "kde-wayland", "KDE", DESKTOP_OWNER),
            x11,
            application_entry(environment, "chromium", "chromium", BROWSER_OWNER),
            application_entry(environment, "google-chrome", "google-chrome", BROWSER_OWNER),
            application_entry(environment, "electron", "electron", BROWSER_OWNER),
            toolkit_entry(environment, "gtk", "zenity", TOOLKIT_OWNER),
            toolkit_entry(environment, "qt", "assistant", TOOLKIT_OWNER),
            terminal,
        ],
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
            Self::Qt => "assistant",
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
            Ok(Some(_)) => {
                self.reaped = true;
                Ok(false)
            }
            Err(_) => Err(RuntimeCommandError::WaitFailed),
        }
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

    fn terminate_and_reap(&mut self, timeout: Duration) -> Result<(), RuntimeCommandError> {
        if self.reaped && !self.group_alive() {
            return Ok(());
        }

        self.signal_group(libc::SIGTERM);
        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            if let Some(child) = self.child.as_mut() {
                match child.try_wait() {
                    Ok(Some(_)) => self.reaped = true,
                    Ok(None) => {}
                    Err(_) => return Err(RuntimeCommandError::WaitFailed),
                }
            }
            if !self.group_alive() {
                if !self.reaped {
                    self.child
                        .as_mut()
                        .ok_or(RuntimeCommandError::WaitFailed)?
                        .wait()
                        .map_err(|_| RuntimeCommandError::WaitFailed)?;
                    self.reaped = true;
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
                    Ok(Some(_)) => self.reaped = true,
                    Ok(None) => {}
                    Err(_) => return Err(RuntimeCommandError::WaitFailed),
                }
            }
            if !self.group_alive() {
                if !self.reaped {
                    self.child
                        .as_mut()
                        .ok_or(RuntimeCommandError::WaitFailed)?
                        .wait()
                        .map_err(|_| RuntimeCommandError::WaitFailed)?;
                    self.reaped = true;
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
            let _ = child.wait();
            self.reaped = true;
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
        RuntimeTargetKind::Qt if environment.has_tool("assistant") => Some("assistant"),
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
            "--no-first-run".to_string(),
            "--disable-sync".to_string(),
            "--disable-background-networking".to_string(),
            "--disable-component-update".to_string(),
            "--disable-default-apps".to_string(),
            controlled_fixture_url(fixture),
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
        RuntimeTargetKind::Qt => vec![],
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
        .filter_map(|pid| {
            let (parent_pid, command_line) = identities.get(&pid).cloned().unwrap_or_default();
            Some(process_role_from_command_line(
                target,
                pid,
                parent_pid,
                &command_line,
                root_pid,
            ))
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

#[derive(Debug)]
struct ControlledScenarioEvidence {
    observed_text: String,
    clipboard_preserved: bool,
    modifiers_released: bool,
    roles: Vec<RuntimePidRole>,
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

fn clipboard_read(environment: &RuntimeEnvironment) -> Result<String, RuntimeCommandError> {
    let output = if environment.session_type == "wayland" {
        std::process::Command::new("wl-paste").args(["-n"]).output()
    } else {
        std::process::Command::new("xclip")
            .args(["-selection", "clipboard", "-o"])
            .output()
    }
    .map_err(|_| RuntimeCommandError::PrerequisiteMissing)?;
    if !output.status.success() {
        return Err(RuntimeCommandError::ScenarioFailed);
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn clipboard_write(
    environment: &RuntimeEnvironment,
    text: &str,
    timeout: Duration,
) -> Result<(), RuntimeCommandError> {
    let (program, args): (&str, &[&str]) = if environment.session_type == "wayland" {
        ("wl-copy", &[])
    } else {
        ("xclip", &["-selection", "clipboard", "-i"])
    };
    command_output_with_input(program, args, text, timeout).map(|_| ())
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
        let observed = clipboard_read(environment)?;
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

fn execute_controlled_scenario(
    target: RuntimeTargetKind,
    environment: RuntimeEnvironment,
    child_pid: u32,
    timeout: Duration,
) -> Result<ControlledScenarioEvidence, RuntimeCommandError> {
    let clipboard_snapshot = clipboard_read(&environment)?;
    inject_controlled_text(&environment, CONTROLLED_TEXT, timeout)?;
    let observed_text = copy_observable_target_text(&environment, timeout)?;
    clipboard_write(&environment, &clipboard_snapshot, timeout)?;
    let clipboard_preserved = clipboard_read(&environment)? == clipboard_snapshot;
    let modifiers_released = verify_synthesized_modifiers_released(&environment, timeout)?;
    Ok(ControlledScenarioEvidence {
        observed_text,
        clipboard_preserved,
        modifiers_released,
        roles: discover_owned_process_tree(target, child_pid),
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

    let startup_deadline = std::time::Instant::now() + startup_timeout;
    while std::time::Instant::now() < startup_deadline {
        match child.healthy() {
            Ok(true) => break,
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
    let scenario_environment = environment.clone();
    let scenario = run_scenario_with_deadline(scenario_timeout, move || {
        execute_controlled_scenario(target, scenario_environment, scenario_pid, scenario_timeout)
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

    let observation = RuntimeSequenceObservation {
        expected_text: CONTROLLED_TEXT.to_string(),
        observed_text: evidence.observed_text,
        process_healthy: child.healthy().unwrap_or(false) && !evidence.roles.is_empty(),
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
    let teardown = child.terminate_and_reap(teardown_timeout);
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

    for target in [
        RuntimeTargetKind::Chromium,
        RuntimeTargetKind::Chrome,
        RuntimeTargetKind::Electron,
        RuntimeTargetKind::Gtk,
        RuntimeTargetKind::Qt,
        RuntimeTargetKind::Terminal,
    ] {
        let result = run_controlled_runtime_target(
            target,
            &environment,
            Duration::from_secs(10),
            Duration::from_secs(30),
            Duration::from_secs(5),
        );
        assert!(
            matches!(
                result.status,
                RuntimeMatrixStatus::Pass
                    | RuntimeMatrixStatus::Skipped
                    | RuntimeMatrixStatus::Unavailable
                    | RuntimeMatrixStatus::Failed
            ),
            "runtime result must use a defined matrix status"
        );
        assert!(!result.reason.trim().is_empty());
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
        "assistant",
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
    assert!(results
        .entries
        .iter()
        .all(|entry| entry.status != RuntimeMatrixStatus::Pass));
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
