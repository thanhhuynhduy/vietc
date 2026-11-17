// SPDX-License-Identifier: MIT

use super::FakeDisplayServer;

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
