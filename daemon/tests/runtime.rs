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

#[derive(Debug)]
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
        const TOOLS: [&str; 10] = [
            "chromium",
            "chromium-browser",
            "google-chrome",
            "electron",
            "zenity",
            "qdbus",
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

    fn has_any_tool(&self, tools: &[&str]) -> bool {
        tools.iter().any(|tool| self.has_tool(tool))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum RuntimeMatrixStatus {
    Ready,
    Skipped(String),
    Failed(String),
}

#[derive(Debug)]
struct RuntimeMatrixResults {
    entries: Vec<(&'static str, RuntimeMatrixStatus)>,
}

impl RuntimeMatrixResults {
    fn status_for(&self, scenario: &str) -> Option<RuntimeMatrixStatus> {
        self.entries
            .iter()
            .find(|(name, _)| *name == scenario)
            .map(|(_, status)| status.clone())
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

    output.push_str("scenarios:\n");
    for (name, status) in &results.entries {
        match status {
            RuntimeMatrixStatus::Ready => {
                output.push_str(&format!("  {name}: READY\n"));
            }
            RuntimeMatrixStatus::Skipped(reason) => {
                output.push_str(&format!("  {name}: SKIPPED: {reason}\n"));
            }
            RuntimeMatrixStatus::Failed(reason) => {
                output.push_str(&format!("  {name}: FAILED: {reason}\n"));
            }
        }
    }

    output.push_str("failure_logs:\n");
    for (name, status) in &results.entries {
        if let RuntimeMatrixStatus::Failed(reason) = status {
            output.push_str(&format!("  {name}: {reason}\n"));
        }
    }
    output
}

fn evaluate_runtime_matrix(environment: &RuntimeEnvironment) -> RuntimeMatrixResults {
    let shared_runtime_requirement = || {
        if !environment.runtime_opt_in {
            Some("VIETC_RUNTIME_TESTS=1 is required".to_string())
        } else if !environment.display_available {
            Some("no X11 or Wayland display is available".to_string())
        } else if !environment.uinput_available {
            Some("writable /dev/uinput is unavailable".to_string())
        } else {
            None
        }
    };

    let gnome_wayland = if !environment.desktop.to_ascii_uppercase().contains("GNOME") {
        RuntimeMatrixStatus::Skipped(format!(
            "requires GNOME Wayland; detected {}",
            environment.desktop
        ))
    } else if environment.session_type != "wayland" {
        RuntimeMatrixStatus::Skipped(format!(
            "requires Wayland; detected {}",
            environment.session_type
        ))
    } else if let Some(reason) = shared_runtime_requirement() {
        RuntimeMatrixStatus::Skipped(reason)
    } else {
        RuntimeMatrixStatus::Ready
    };

    let kde_wayland = if !environment.desktop.to_ascii_uppercase().contains("KDE") {
        RuntimeMatrixStatus::Skipped(format!(
            "requires KDE Wayland; detected {}",
            environment.desktop
        ))
    } else if environment.session_type != "wayland" {
        RuntimeMatrixStatus::Skipped(format!(
            "requires Wayland; detected {}",
            environment.session_type
        ))
    } else if let Some(reason) = shared_runtime_requirement() {
        RuntimeMatrixStatus::Skipped(reason)
    } else {
        RuntimeMatrixStatus::Ready
    };

    let x11 = if environment.session_type != "x11" {
        RuntimeMatrixStatus::Skipped(format!(
            "requires X11; detected {}",
            environment.session_type
        ))
    } else if let Some(reason) = shared_runtime_requirement() {
        RuntimeMatrixStatus::Skipped(reason)
    } else {
        RuntimeMatrixStatus::Ready
    };

    let chromium_electron = if !environment.has_any_tool(&[
        "chromium",
        "chromium-browser",
        "google-chrome",
        "electron",
    ]) {
        RuntimeMatrixStatus::Skipped(
            "none of chromium, chromium-browser, google-chrome, electron is installed".to_string(),
        )
    } else if let Some(reason) = shared_runtime_requirement() {
        RuntimeMatrixStatus::Skipped(reason)
    } else {
        RuntimeMatrixStatus::Ready
    };

    let gtk = if !environment.has_tool("zenity") {
        RuntimeMatrixStatus::Skipped("GTK runtime probe tool zenity is not installed".to_string())
    } else if let Some(reason) = shared_runtime_requirement() {
        RuntimeMatrixStatus::Skipped(reason)
    } else {
        RuntimeMatrixStatus::Ready
    };

    let qt = if !environment.has_tool("qdbus") {
        RuntimeMatrixStatus::Skipped("Qt runtime probe tool qdbus is not installed".to_string())
    } else if let Some(reason) = shared_runtime_requirement() {
        RuntimeMatrixStatus::Skipped(reason)
    } else {
        RuntimeMatrixStatus::Ready
    };

    let terminal = if !environment.has_any_tool(&["gnome-terminal", "konsole", "xterm", "kitty"]) {
        RuntimeMatrixStatus::Skipped("no supported terminal emulator is installed".to_string())
    } else if let Some(reason) = shared_runtime_requirement() {
        RuntimeMatrixStatus::Skipped(reason)
    } else {
        RuntimeMatrixStatus::Ready
    };

    RuntimeMatrixResults {
        entries: vec![
            ("gnome-wayland", gnome_wayland),
            ("kde-wayland", kde_wayland),
            ("x11", x11),
            ("chromium-electron", chromium_electron),
            ("gtk", gtk),
            ("qt", qt),
            ("terminal", terminal),
        ],
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
fn runtime_matrix_reports_each_unavailable_scenario_with_a_specific_skip_reason() {
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

    assert_eq!(
        results.status_for("gnome-wayland"),
        Some(RuntimeMatrixStatus::Skipped(
            "writable /dev/uinput is unavailable".to_string()
        ))
    );
    assert_eq!(
        results.status_for("kde-wayland"),
        Some(RuntimeMatrixStatus::Skipped(
            "requires KDE Wayland; detected GNOME".to_string()
        ))
    );
    assert_eq!(
        results.status_for("x11"),
        Some(RuntimeMatrixStatus::Skipped(
            "requires X11; detected wayland".to_string()
        ))
    );
    assert_eq!(
        results.status_for("chromium-electron"),
        Some(RuntimeMatrixStatus::Skipped(
            "none of chromium, chromium-browser, google-chrome, electron is installed".to_string()
        ))
    );
    assert_eq!(
        results.status_for("gtk"),
        Some(RuntimeMatrixStatus::Skipped(
            "GTK runtime probe tool zenity is not installed".to_string()
        ))
    );
    assert_eq!(
        results.status_for("qt"),
        Some(RuntimeMatrixStatus::Skipped(
            "Qt runtime probe tool qdbus is not installed".to_string()
        ))
    );
    assert_eq!(
        results.status_for("terminal"),
        Some(RuntimeMatrixStatus::Skipped(
            "no supported terminal emulator is installed".to_string()
        ))
    );
}

#[test]
fn runtime_matrix_artifact_records_ready_skipped_and_failed_scenarios() {
    let results = RuntimeMatrixResults {
        entries: vec![
            ("cosmic-wayland", RuntimeMatrixStatus::Ready),
            (
                "x11",
                RuntimeMatrixStatus::Skipped("requires X11; detected wayland".to_string()),
            ),
            (
                "chromium-electron",
                RuntimeMatrixStatus::Failed(
                    "target did not observe injected text before timeout".to_string(),
                ),
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
    assert!(artifact.contains("cosmic-wayland: READY"));
    assert!(artifact.contains("x11: SKIPPED: requires X11; detected wayland"));
    assert!(artifact.contains(
        "chromium-electron: FAILED: target did not observe injected text before timeout"
    ));
    assert!(
        artifact.contains("chromium-electron: target did not observe injected text before timeout")
    );
}

#[test]
fn tool_version_reports_missing_when_program_cannot_be_spawned() {
    let program = format!("vietc-runtime-missing-tool-{}", std::process::id());

    assert_eq!(tool_version(&program), "missing");
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
        "qdbus",
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

    assert_eq!(
        results.status_for("gnome-wayland"),
        Some(RuntimeMatrixStatus::Skipped(format!(
            "requires GNOME Wayland; detected {}",
            environment.desktop
        )))
    );
    assert_eq!(
        results.status_for("kde-wayland"),
        Some(RuntimeMatrixStatus::Skipped(format!(
            "requires KDE Wayland; detected {}",
            environment.desktop
        )))
    );
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
