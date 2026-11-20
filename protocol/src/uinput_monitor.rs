// SPDX-License-Identifier: MIT
mod clipboard;
mod clipboard_backup;
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

#[cfg(feature = "wayland-protocol")]
struct WaylandVkSlot {
    vk: Option<crate::wayland_vk::WaylandVirtualKeyboard>,
    disabled: bool,
    last_attempt: Option<std::time::Instant>,
}

/// Readiness/transfer wait budget, not proof that an application inserted text.
/// Native Wayland prefers persistent `zwp_virtual_keyboard_v1`, falling back to
/// data-control clipboard offers; X11 uses best-effort selection stability.
pub struct UinputInjector {
    file: File,
    #[cfg(feature = "wayland-protocol")]
    wayland_vk: Mutex<WaylandVkSlot>,
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
    bypass_mode: std::sync::atomic::AtomicBool,
    uinput_mod_mask: std::sync::atomic::AtomicU8,
    uinput_shift_mask: std::sync::atomic::AtomicU8,
    omnibox_inline_selection: std::sync::atomic::AtomicBool,
}

fn system_modifier_bit(keycode: u16) -> Option<u8> {
    match keycode {
        29 => Some(1 << 0),  // KEY_LEFTCTRL
        97 => Some(1 << 1),  // KEY_RIGHTCTRL
        56 => Some(1 << 2),  // KEY_LEFTALT
        100 => Some(1 << 3), // KEY_RIGHTALT
        125 => Some(1 << 4), // KEY_LEFTMETA
        126 => Some(1 << 5), // KEY_RIGHTMETA
        _ => None,
    }
}

fn shift_bit(keycode: u16) -> Option<u8> {
    match keycode {
        42 => Some(1 << 0), // KEY_LEFTSHIFT
        54 => Some(1 << 1), // KEY_RIGHTSHIFT
        _ => None,
    }
}

impl UinputInjector {
    pub fn new(name: &str) -> Result<Self, Box<dyn std::error::Error>> {
        let _ = std::process::Command::new("modprobe")
            .args(["uinput"])
            .output();
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/uinput")?;

        let fd = file.as_raw_fd();

        ioctl(fd, UI_SET_EVBIT, EV_KEY as u64)
            .map_err(|e| format!("UI_SET_EVBIT failed: {}", e))?;

        for code in 0..=KEYBOARD_KEY_MAX {
            ioctl(fd, UI_SET_KEYBIT, code as u64)
                .map_err(|e| format!("UI_SET_KEYBIT {} failed: {}", code, e))?;
        }

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

        std::thread::sleep(std::time::Duration::from_millis(10));

        #[cfg(feature = "wayland-protocol")]
        let vk_slot = {
            let disabled = std::env::var_os("VIETC_DISABLE_WAYLAND_VK").is_some();
            let vk = if disabled {
                None
            } else {
                match crate::wayland_vk::WaylandVirtualKeyboard::connect() {
                    Ok(vk) => {
                        eprintln!(
                            "[vietc] Wayland zwp_virtual_keyboard_v1 active (zero-clipboard direct Unicode mode)"
                        );
                        Some(vk)
                    }
                    Err(_) => None,
                }
            };
            WaylandVkSlot {
                vk,
                disabled,
                last_attempt: Some(std::time::Instant::now()),
            }
        };

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
            wayland_vk: Mutex::new(vk_slot),
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
            bypass_mode: std::sync::atomic::AtomicBool::new(false),
            uinput_mod_mask: std::sync::atomic::AtomicU8::new(0),
            uinput_shift_mask: std::sync::atomic::AtomicU8::new(0),
            omnibox_inline_selection: std::sync::atomic::AtomicBool::new(false),
        })
    }

    #[cfg(feature = "wayland-protocol")]
    fn with_wayland_vk<R>(
        &self,
        op: impl FnOnce(&crate::wayland_vk::WaylandVirtualKeyboard) -> R,
    ) -> Option<R> {
        let mut slot = self
            .wayland_vk
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if slot.disabled {
            return None;
        }
        if slot.vk.as_ref().is_some_and(|vk| !vk.is_healthy()) {
            slot.vk = None;
        }
        if slot.vk.is_none() {
            let now = std::time::Instant::now();
            let can_retry = slot
                .last_attempt
                .is_none_or(|prev| now.duration_since(prev) >= Duration::from_secs(3));
            if can_retry {
                slot.last_attempt = Some(now);
                if let Ok(vk) = crate::wayland_vk::WaylandVirtualKeyboard::connect() {
                    vk.set_caps_lock(self.caps_lock.load(Ordering::Relaxed));
                    eprintln!("[vietc] Wayland zwp_virtual_keyboard_v1 connected");
                    slot.vk = Some(vk);
                }
            }
        }
        slot.vk.as_ref().map(op)
    }

    #[cfg(feature = "wayland-protocol")]
    fn transfer_vk_shift_to_uinput(&self) {
        let (left_shift, right_shift) = self
            .with_wayland_vk(|vk| vk.handoff_shift_to_uinput())
            .unwrap_or((false, false));
        if left_shift || right_shift {
            let mut bits = 0u8;
            if left_shift {
                bits |= 1 << 0;
            }
            if right_shift {
                bits |= 1 << 1;
            }
            self.uinput_shift_mask.fetch_or(bits, Ordering::AcqRel);
            let _ = serialize_transaction(&self.transaction_lock, || {
                if left_shift {
                    self.send_uinput_event_unlocked(EV_KEY, 42, 1)?;
                }
                if right_shift {
                    self.send_uinput_event_unlocked(EV_KEY, 54, 1)?;
                }
                self.send_uinput_event_unlocked(0, 0, 0)
            });
        }
    }

    #[cfg(feature = "wayland-protocol")]
    fn transfer_uinput_shift_to_vk(&self) {
        let bits = self.uinput_shift_mask.swap(0, Ordering::AcqRel);
        if bits == 0 {
            return;
        }
        let left = (bits & (1 << 0)) != 0;
        let right = (bits & (1 << 1)) != 0;
        let _ = serialize_transaction(&self.transaction_lock, || {
            if left {
                self.send_uinput_event_unlocked(EV_KEY, 42, 0)?;
            }
            if right {
                self.send_uinput_event_unlocked(EV_KEY, 54, 0)?;
            }
            self.send_uinput_event_unlocked(0, 0, 0)
        });
        let _ = self.with_wayland_vk(|vk| vk.adopt_shift_from_uinput(left, right));
    }
}

impl KeyInjector for UinputInjector {
    fn send_backspace(&self) -> InjectResult {
        #[cfg(feature = "wayland-protocol")]
        if !self.bypass_mode.load(Ordering::Relaxed)
            && self.uinput_mod_mask.load(Ordering::Relaxed) == 0
            && self
                .with_wayland_vk(|vk| vk.replace_text(1, 0, "").is_ok())
                .unwrap_or(false)
        {
            return InjectResult::Success;
        }
        if self.send_backspaces(1).is_ok() {
            InjectResult::Success
        } else {
            InjectResult::Failed
        }
    }

    fn send_key_event(&self, keycode: u16, value: i32) -> InjectResult {
        let bypassed = self.bypass_mode.load(Ordering::Acquire);

        if let Some(bit) = system_modifier_bit(keycode) {
            if value == 0 {
                let prev = self.uinput_mod_mask.fetch_and(!bit, Ordering::AcqRel);
                let now_zero = (prev & !bit) == 0;
                #[cfg(feature = "wayland-protocol")]
                if prev != 0 && now_zero && !bypassed {
                    let result = serialize_transaction(&self.transaction_lock, || {
                        self.send_uinput_event_unlocked(EV_KEY, keycode, 0)?;
                        self.send_uinput_event_unlocked(0, 0, 0)
                    });
                    self.transfer_uinput_shift_to_vk();
                    return if matches!(result, Ok(Ok(()))) {
                        InjectResult::Success
                    } else {
                        InjectResult::Failed
                    };
                }
            } else {
                let prev = self.uinput_mod_mask.fetch_or(bit, Ordering::AcqRel);
                #[cfg(feature = "wayland-protocol")]
                if prev == 0 && value == 1 {
                    self.transfer_vk_shift_to_uinput();
                }
            }
        } else if !bypassed || value == 0 {
            #[cfg(feature = "wayland-protocol")]
            {
                let mods_active = self.uinput_mod_mask.load(Ordering::Acquire) != 0;
                if self
                    .with_wayland_vk(|vk| vk.send_key_event(keycode, value, mods_active))
                    .unwrap_or(false)
                {
                    return InjectResult::Success;
                }
            }
        }

        if let Some(sbit) = shift_bit(keycode) {
            if value == 0 {
                self.uinput_shift_mask.fetch_and(!sbit, Ordering::AcqRel);
            } else if value == 1 {
                self.uinput_shift_mask.fetch_or(sbit, Ordering::AcqRel);
            }
        }

        let result = serialize_transaction(&self.transaction_lock, || {
            self.send_uinput_event_unlocked(EV_KEY, keycode, value)?;
            self.send_uinput_event_unlocked(0, 0, 0)
        });
        if matches!(result, Ok(Ok(()))) {
            InjectResult::Success
        } else {
            InjectResult::Failed
        }
    }

    fn send_char(&self, ch: char) -> InjectResult {
        #[cfg(feature = "wayland-protocol")]
        {
            let mut buf = [0u8; 4];
            let s = ch.encode_utf8(&mut buf);
            if crate::wayland_vk::WaylandVirtualKeyboard::can_encode_str(s)
                && self
                    .with_wayland_vk(|vk| vk.replace_text(0, 0, s).is_ok())
                    .unwrap_or(false)
            {
                return InjectResult::Success;
            }
        }

        let caps_active = self.caps_lock.load(Ordering::Relaxed);
        if let Some(keycode) = char_to_linux_keycode(ch) {
            let needs_shift = if ch.is_ascii_alphabetic() {
                ch.is_uppercase() ^ caps_active
            } else {
                "~!@#$%^&*()_+{}|:\"<>?".contains(ch)
            };
            return if self.send_key_stroke(keycode, needs_shift).is_ok() {
                InjectResult::Success
            } else {
                InjectResult::Failed
            };
        }
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
        #[cfg(feature = "wayland-protocol")]
        if crate::wayland_vk::WaylandVirtualKeyboard::can_encode_str(s)
            && self
                .with_wayland_vk(|vk| vk.replace_text(0, 0, s).is_ok())
                .unwrap_or(false)
        {
            return InjectResult::Success;
        }

        let is_ascii = s.chars().all(|c| char_to_linux_keycode(c).is_some());

        if is_ascii {
            for ch in s.chars() {
                let result = self.send_char(ch);
                if result != InjectResult::Success {
                    return result;
                }
            }
            return InjectResult::Success;
        }

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
        *self
            .clip
            .focus_generation
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = Some(generation);
        InjectResult::Success
    }

    fn has_uncertain_edit(&self) -> bool {
        #[cfg(feature = "wayland-protocol")]
        return self.wayland_paste_is_uncertain();
        #[cfg(not(feature = "wayland-protocol"))]
        false
    }

    fn cancel_pending_replacement(&self, focus_generation: u64, reason: &str) -> InjectResult {
        self.omnibox_inline_selection
            .store(false, Ordering::Release);
        #[cfg(feature = "wayland-protocol")]
        {
            let mut uncertain = self
                .uncertain_focus
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            if uncertain.is_some_and(|previous| previous != focus_generation) {
                *uncertain = None;
            }
        }
        let mut state = self.clip.inner.lock().unwrap_or_else(|p| p.into_inner());
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

    fn update_pasted_text(&self, _text: &str) -> InjectResult {
        InjectResult::Success
    }

    fn set_active_window(&self, window_class: &str) -> InjectResult {
        let mut st = self.clip.inner.lock().unwrap_or_else(|p| p.into_inner());
        st.active_window = window_class.to_lowercase();
        InjectResult::Success
    }

    fn set_caps_lock(&self, enabled: bool) -> InjectResult {
        self.caps_lock.store(enabled, Ordering::Relaxed);
        #[cfg(feature = "wayland-protocol")]
        {
            let _ = self.with_wayland_vk(|vk| vk.set_caps_lock(enabled));
        }
        InjectResult::Success
    }

    fn set_terminal_app(&self, is_terminal: bool) -> InjectResult {
        self.terminal_app.store(is_terminal, Ordering::Relaxed);
        InjectResult::Success
    }

    fn set_bypass_mode(&self, bypass: bool) -> InjectResult {
        let was_bypass = self.bypass_mode.swap(bypass, Ordering::AcqRel);
        #[cfg(feature = "wayland-protocol")]
        if bypass && !was_bypass {
            self.transfer_vk_shift_to_uinput();
        } else if !bypass && was_bypass && self.uinput_mod_mask.load(Ordering::Acquire) == 0 {
            self.transfer_uinput_shift_to_vk();
        }
        InjectResult::Success
    }

    fn uses_virtual_keyboard(&self) -> bool {
        #[cfg(feature = "wayland-protocol")]
        return self.with_wayland_vk(|vk| vk.is_healthy()).unwrap_or(false);
        #[cfg(not(feature = "wayland-protocol"))]
        false
    }

    fn set_omnibox_inline_selection(&self, active: bool) -> InjectResult {
        self.omnibox_inline_selection
            .store(active, Ordering::Release);
        InjectResult::Success
    }
}

impl UinputInjector {
    fn current_focus_generation(&self) -> u64 {
        self.clip
            .focus_generation
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .map_or(0, |generation| generation.load(Ordering::Acquire))
    }

    fn inject_replacement_atomic(
        &self,
        backspaces: usize,
        text: &str,
        focus_generation: u64,
    ) -> InjectResult {
        let extra_deletes =
            usize::from(self.omnibox_inline_selection.swap(false, Ordering::AcqRel));

        #[cfg(feature = "wayland-protocol")]
        if crate::wayland_vk::WaylandVirtualKeyboard::can_encode_str(text)
            && self
                .with_wayland_vk(|vk| vk.replace_text(backspaces, extra_deletes, text).is_ok())
                .unwrap_or(false)
        {
            return InjectResult::Success;
        }

        #[cfg(feature = "wayland-protocol")]
        if self.wayland_paste_is_uncertain() {
            return InjectResult::Failed;
        }

        if extra_deletes > 0 {
            for _ in 0..extra_deletes {
                if self.send_key_stroke(111, false).is_err() {
                    return InjectResult::Failed;
                }
            }
        }

        if text.is_empty() {
            if backspaces == 0 || self.send_backspaces(backspaces).is_ok() {
                return InjectResult::Success;
            }
            return InjectResult::Failed;
        }

        if text
            .chars()
            .all(|c| char_to_linux_keycode(c).is_some() || c == '\n')
        {
            if backspaces > 0 && self.send_backspaces(backspaces).is_err() {
                return InjectResult::Failed;
            }
            for ch in text.chars() {
                if ch == '\n' {
                    if self.send_enter().is_err() {
                        return InjectResult::Failed;
                    }
                } else if self.send_char(ch) != InjectResult::Success {
                    return InjectResult::Failed;
                }
            }
            return InjectResult::Success;
        }

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
