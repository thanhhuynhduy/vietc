// SPDX-License-Identifier: MIT
mod clipboard;
mod key_injection;
mod replacement_transaction;
mod session_environment;
use std::fs::{File, OpenOptions};
use std::os::unix::io::AsRawFd;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use super::inject::{InjectResult, KeyInjector};
use clipboard::{trace_clipboard, ClipInner, ClipState, ClipboardRestorer, RetainedWaylandPaste};
use key_injection::{
    char_to_linux_keycode, ioctl, serialize_transaction, strip_vn_diacritic, uinput_setup, EV_KEY,
    KEYBOARD_KEY_MAX, UINPUT_MAX_NAME_SIZE, UI_DEV_CREATE, UI_DEV_SETUP, UI_SET_EVBIT,
    UI_SET_KEYBIT,
};
use replacement_transaction::ClipboardBurstState;

/// Readiness/transfer wait budget, not proof that an application inserted text.
/// Native Wayland waits for a source transfer; the legacy X11 clipboard path
/// still uses a best-effort stability delay.
pub struct UinputInjector {
    file: File,
    #[cfg(feature = "wayland-protocol")]
    wayland_paste: Mutex<Option<RetainedWaylandPaste>>,
    #[cfg(feature = "wayland-protocol")]
    retired_wayland_offers: Mutex<Vec<crate::wayland_clipboard::ClipboardOffer>>,
    #[cfg(feature = "wayland-protocol")]
    uncertain_focus: Mutex<Option<u64>>,
    transaction_lock: Mutex<()>,
    clip: Arc<ClipState>,
    restorer: ClipboardRestorer,
    caps_lock: std::sync::atomic::AtomicBool,
    terminal_app: std::sync::atomic::AtomicBool,
}

impl UinputInjector {
    pub fn new(name: &str) -> Result<Self, Box<dyn std::error::Error>> {
        // Ensure uinput kernel module is loaded before trying to open the device
        let _ = std::process::Command::new("modprobe")
            .args(["uinput"])
            .output();
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/uinput")?;

        let fd = file.as_raw_fd();

        // Enable EV_KEY
        ioctl(fd, UI_SET_EVBIT, EV_KEY as u64)
            .map_err(|e| format!("UI_SET_EVBIT failed: {}", e))?;

        // Enable all keyboard keycodes we'll need (0-255).
        // KEY_MAX (0x1ff) includes mouse button codes (BTN_LEFT=272 etc.)
        // which would make X11 treat this device as a pointer and conflict
        // with the real mouse.
        for code in 0..=KEYBOARD_KEY_MAX {
            ioctl(fd, UI_SET_KEYBIT, code as u64)
                .map_err(|e| format!("UI_SET_KEYBIT {} failed: {}", code, e))?;
        }

        // Create uinput device
        let mut usetup: uinput_setup = unsafe { std::mem::zeroed() };
        let name_bytes = name.as_bytes();
        let copy_len = name_bytes.len().min(UINPUT_MAX_NAME_SIZE - 1);
        for (i, &byte) in name_bytes.iter().enumerate().take(copy_len) {
            usetup.name[i] = byte as i8;
        }
        usetup.name[copy_len] = 0;
        usetup.id.bustype = 0x03; // BUS_USB
        usetup.id.vendor = 0x1234;
        usetup.id.product = 0x5678;
        usetup.id.version = 1;

        ioctl(fd, UI_DEV_SETUP, &usetup as *const uinput_setup as u64)
            .map_err(|e| format!("UI_DEV_SETUP failed: {}", e))?;

        ioctl(fd, UI_DEV_CREATE, 0).map_err(|e| format!("UI_DEV_CREATE failed: {}", e))?;

        // Small delay for device to be ready
        std::thread::sleep(std::time::Duration::from_millis(10));

        let clip = Arc::new(ClipState {
            inner: Mutex::new(ClipInner {
                burst: ClipboardBurstState::default(),
                restore_due: None,
                shutdown: false,
                active_window: String::new(),
            }),
            io_lock: Mutex::new(()),
            cv: Condvar::new(),
            focus_generation: Mutex::new(None),
        });
        let restorer = ClipboardRestorer::spawn(Arc::clone(&clip), clipboard::run_restorer);

        Ok(Self {
            file,
            #[cfg(feature = "wayland-protocol")]
            wayland_paste: Mutex::new(None),
            #[cfg(feature = "wayland-protocol")]
            retired_wayland_offers: Mutex::new(Vec::new()),
            #[cfg(feature = "wayland-protocol")]
            uncertain_focus: Mutex::new(None),
            transaction_lock: Mutex::new(()),
            clip,
            restorer,
            caps_lock: std::sync::atomic::AtomicBool::new(false),
            terminal_app: std::sync::atomic::AtomicBool::new(false),
        })
    }
}

impl KeyInjector for UinputInjector {
    fn send_backspace(&self) -> InjectResult {
        if self.send_backspaces(1).is_ok() {
            InjectResult::Success
        } else {
            InjectResult::Failed
        }
    }

    fn send_key_event(&self, keycode: u16, value: i32) -> InjectResult {
        let result = serialize_transaction(&self.transaction_lock, || {
            self.send_uinput_event_unlocked(EV_KEY, keycode, value)?;
            self.send_uinput_event_unlocked(0, 0, 0)
        });
        if value == 0 {
            std::thread::sleep(Duration::from_millis(2));
        }
        if matches!(result, Ok(Ok(()))) {
            InjectResult::Success
        } else {
            InjectResult::Failed
        }
    }

    fn send_char(&self, ch: char) -> InjectResult {
        let caps_active = self.caps_lock.load(Ordering::Relaxed);
        if let Some(keycode) = char_to_linux_keycode(ch) {
            let needs_shift = if ch.is_ascii_alphabetic() {
                ch.is_uppercase() ^ caps_active
            } else {
                "!@#$%^&*()_+{}|:\"<>?".contains(ch)
            };
            return if self.send_key_stroke(keycode, needs_shift).is_ok() {
                InjectResult::Success
            } else {
                InjectResult::Failed
            };
        }
        // Vietnamese Unicode char: map to base ASCII and send via uinput
        let ascii = strip_vn_diacritic(ch);
        if let Some(keycode) = char_to_linux_keycode(ascii) {
            let needs_shift = if ascii.is_ascii_alphabetic() {
                ascii.is_uppercase() ^ caps_active
            } else {
                ascii.is_uppercase()
            };
            return if self.send_key_stroke(keycode, needs_shift).is_ok() {
                InjectResult::Success
            } else {
                InjectResult::Failed
            };
        }
        InjectResult::NotSupported
    }

    fn send_string(&self, s: &str) -> InjectResult {
        // ASCII characters: inject directly via uinput keycodes
        let is_ascii = s.chars().all(|c| char_to_linux_keycode(c).is_some());

        if is_ascii {
            for ch in s.chars() {
                let result = self.send_char(ch);
                if result != InjectResult::Success {
                    return result;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            return InjectResult::Success;
        }

        // Fallback: clipboard copy + paste
        if self.paste_via_clipboard(s, 0, self.current_focus_generation()) {
            InjectResult::Success
        } else {
            eprintln!("[vietc] Unicode insertion failed: clipboard unavailable");
            InjectResult::Failed
        }
    }

    fn inject_replacement(&self, backspaces: usize, text: &str) -> InjectResult {
        self.inject_replacement_atomic(backspaces, text, self.current_focus_generation())
    }

    fn bind_focus_generation(&self, generation: Arc<AtomicU64>) -> InjectResult {
        *self.clip.focus_generation.lock().unwrap() = Some(generation);
        InjectResult::Success
    }

    fn has_uncertain_edit(&self) -> bool {
        #[cfg(feature = "wayland-protocol")]
        return self.wayland_paste_is_uncertain();
        #[cfg(not(feature = "wayland-protocol"))]
        false
    }

    fn cancel_pending_replacement(&self, focus_generation: u64, reason: &str) -> InjectResult {
        #[cfg(feature = "wayland-protocol")]
        {
            let mut uncertain = self.uncertain_focus.lock().unwrap();
            if uncertain.is_some_and(|previous| previous != focus_generation) {
                // A new target starts with an empty composition. Do not infer
                // recovery merely from a clipboard worker or connection dying.
                *uncertain = None;
            }
        }
        let mut state = self.clip.inner.lock().unwrap();
        if state.burst.cancel_for_focus_change(focus_generation) {
            state.restore_due = None;
            trace_clipboard(
                0,
                "cancelled",
                &format!("focus_generation={focus_generation} reason={reason}"),
            );
        }
        self.clip.cv.notify_all();
        InjectResult::Success
    }

    fn inject_replacement_for_generation(
        &self,
        backspaces: usize,
        text: &str,
        focus_generation: u64,
    ) -> InjectResult {
        if self.current_focus_generation() != focus_generation {
            return InjectResult::Failed;
        }
        self.inject_replacement_atomic(backspaces, text, focus_generation)
    }

    fn flush(&self) -> InjectResult {
        InjectResult::Success
    }

    /// Record that Unicode text was pasted via clipboard (for future delete/backspace support)
    fn update_pasted_text(&self, text: &str) -> InjectResult {
        // Text tracking happens through OutputCommand pipeline in daemon
        // This is called after clipboard paste to inform engine of pasted content
        eprintln!(
            "[vietc] update_pasted_text: recorded '{}' (len={})",
            text.escape_default(),
            text.len()
        );
        InjectResult::Success
    }

    fn set_active_window(&self, window_class: &str) -> InjectResult {
        let mut st = self.clip.inner.lock().unwrap();
        st.active_window = window_class.to_lowercase();
        InjectResult::Success
    }

    fn set_caps_lock(&self, enabled: bool) -> InjectResult {
        self.caps_lock.store(enabled, Ordering::Relaxed);
        InjectResult::Success
    }

    fn set_terminal_app(&self, is_terminal: bool) -> InjectResult {
        self.terminal_app.store(is_terminal, Ordering::Relaxed);
        InjectResult::Success
    }
}

impl UinputInjector {
    fn current_focus_generation(&self) -> u64 {
        self.clip
            .focus_generation
            .lock()
            .unwrap()
            .as_ref()
            .map_or(0, |generation| generation.load(Ordering::Acquire))
    }

    /// Send backspaces and replacement text through one uinput device.
    /// ASCII is injected directly. Unicode is copied to the clipboard and
    /// pasted with Ctrl+V through the same device, avoiding mixed-channel
    /// editing transactions.
    fn inject_replacement_atomic(
        &self,
        backspaces: usize,
        text: &str,
        focus_generation: u64,
    ) -> InjectResult {
        #[cfg(feature = "wayland-protocol")]
        if self.wayland_paste_is_uncertain() {
            // The editor may still be processing the previous paste. Do not
            // follow it with another delete or publish a different payload.
            return InjectResult::Failed;
        }
        if text.is_empty() {
            if backspaces == 0 || self.send_backspaces(backspaces).is_ok() {
                return InjectResult::Success;
            }
            return InjectResult::Failed;
        }

        // Fast path: pure ASCII (including newline) via uinput batch — single channel
        if text
            .chars()
            .all(|c| char_to_linux_keycode(c).is_some() || c == '\n')
        {
            if backspaces > 0 {
                if self.send_backspaces(backspaces).is_err() {
                    return InjectResult::Failed;
                }
                std::thread::sleep(Duration::from_millis(8));
            }
            for ch in text.chars() {
                if ch == '\n' {
                    if self.send_enter().is_err() {
                        return InjectResult::Failed;
                    }
                } else if self.send_char(ch) != InjectResult::Success {
                    return InjectResult::Failed;
                }
                std::thread::sleep(Duration::from_millis(3));
            }
            return InjectResult::Success;
        }

        // Keep delete and insert on the same uinput channel. Mixing uinput
        // Backspace with Wayland virtual-keyboard text can crash Microsoft Edge
        // with SIGILL even for a minimal replacement such as `Da` -> `Dạ`.
        // Prepare and verify the clipboard BEFORE sending Backspaces so a
        // clipboard failure never deletes the user's already-typed characters.
        if self.paste_via_clipboard(text, backspaces, focus_generation) {
            InjectResult::Success
        } else {
            eprintln!("[vietc] Unicode insertion failed: clipboard unavailable");
            InjectResult::Failed
        }
    }
}

impl Drop for UinputInjector {
    fn drop(&mut self) {
        let _ = self.restorer.shutdown_and_join();
        let _ = key_injection::destroy_device(self.file.as_raw_fd());
    }
}
