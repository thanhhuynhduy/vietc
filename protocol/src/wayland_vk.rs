// SPDX-License-Identifier: MIT
//! Persistent in-process Wayland `zwp_virtual_keyboard_v1` backend.
//!
//! Injects BackSpace, Delete, ASCII, and all 134 Vietnamese Unicode characters
//! directly over a single Wayland virtual keyboard channel with zero preedit
//! underline, zero IBus/Fcitx5 dependency, and zero clipboard interaction.

use std::io;
use std::os::fd::{AsFd, AsRawFd, OwnedFd};
#[cfg(test)]
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use wayland_client::protocol::{wl_callback, wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};

use crate::layout::KeyboardLayout;
#[cfg(test)]
use crate::wayland_vk_keymap::char_to_vk_stroke;
use crate::wayland_vk_keymap::{
    build_xkb_keymap_for_layout, char_to_vk_stroke_for_layout, is_layout_alpha_evdev,
    is_vk_typing_or_nav_keycode, XKB_MOD_LOCK, XKB_MOD_MOD2, XKB_MOD_SHIFT,
};

#[allow(
    dead_code,
    non_camel_case_types,
    non_upper_case_globals,
    unused_imports,
    clippy::single_component_path_imports
)]
mod proto {
    use wayland_client;
    use wayland_client::protocol::__interfaces::*;
    use wayland_client::protocol::*;

    pub mod __interfaces {
        use wayland_client::backend as wayland_backend;
        use wayland_client::protocol::__interfaces::*;
        wayland_scanner::generate_interfaces!("protocols/virtual-keyboard-unstable-v1.xml");
    }
    use self::__interfaces::*;
    wayland_scanner::generate_client_code!("protocols/virtual-keyboard-unstable-v1.xml");
}

use proto::zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1;
use proto::zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1;

const WL_KEYBOARD_KEYMAP_FORMAT_XKB_V1: u32 = 1;
const KEY_BACKSPACE: u32 = 14;
const KEY_DELETE: u32 = 111;
const KEY_LEFTSHIFT: u16 = 42;
const KEY_RIGHTSHIFT: u16 = 54;

/// Bounded Wayland roundtrip timeout so a stalled compositor or dead socket
/// can NEVER permanently freeze `evdev_loop` (Invariant 4).
const VK_ROUNDTRIP_TIMEOUT: Duration = Duration::from_millis(200);

#[derive(Default)]
struct VkDiscovery {
    seat: Option<wl_seat::WlSeat>,
    seat_global_name: Option<u32>,
    manager: Option<ZwpVirtualKeyboardManagerV1>,
    manager_global_name: Option<u32>,
    global_removed: bool,
    sync_done: bool,
}

impl Dispatch<wl_callback::WlCallback, ()> for VkDiscovery {
    fn event(
        state: &mut Self,
        _: &wl_callback::WlCallback,
        event: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if matches!(event, wl_callback::Event::Done { .. }) {
            state.sync_done = true;
        }
    }
}

fn roundtrip_with_timeout(
    connection: &Connection,
    queue: &mut EventQueue<VkDiscovery>,
    discovery: &mut VkDiscovery,
    timeout: Duration,
    label: &str,
) -> io::Result<()> {
    discovery.sync_done = false;
    let qh = queue.handle();
    let _cb = connection.display().sync(&qh, ());
    let deadline = Instant::now() + timeout;
    while !discovery.sync_done {
        queue
            .flush()
            .map_err(|e| io::Error::other(format!("{label}: {e}")))?;
        queue
            .dispatch_pending(discovery)
            .map_err(|e| io::Error::other(format!("{label}: {e}")))?;
        if discovery.sync_done {
            return Ok(());
        }
        let now = Instant::now();
        if now >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("{label} timed out"),
            ));
        }
        if let Some(guard) = queue.prepare_read() {
            let remaining_ms = deadline
                .saturating_duration_since(now)
                .as_millis()
                .clamp(1, i32::MAX as u128) as i32;
            let mut pfd = libc::pollfd {
                fd: guard.connection_fd().as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            let rc = unsafe { libc::poll(&mut pfd, 1, remaining_ms) };
            if rc < 0 {
                let err = io::Error::last_os_error();
                if err.raw_os_error() == Some(libc::EINTR) {
                    continue;
                }
                return Err(err);
            }
            if rc == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("{label} timed out"),
                ));
            }
            if (pfd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL)) != 0 {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    format!("{label} disconnected"),
                ));
            }
            match guard.read() {
                Ok(_) => {}
                Err(wayland_client::backend::WaylandError::Io(e))
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) => {}
                Err(wayland_client::backend::WaylandError::Io(e)) => return Err(e),
                Err(wayland_client::backend::WaylandError::Protocol(e)) => {
                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        format!("{label} protocol error: {e}"),
                    ));
                }
            }
        }
    }
    Ok(())
}

impl Dispatch<wl_registry::WlRegistry, ()> for VkDiscovery {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => {
                if interface == "wl_seat" && state.seat.is_none() {
                    state.seat =
                        Some(registry.bind::<wl_seat::WlSeat, _, _>(name, version.min(7), qh, ()));
                    state.seat_global_name = Some(name);
                } else if interface == "zwp_virtual_keyboard_manager_v1" && state.manager.is_none()
                {
                    state.manager =
                        Some(registry.bind::<ZwpVirtualKeyboardManagerV1, _, _>(name, 1, qh, ()));
                    state.manager_global_name = Some(name);
                }
            }
            wl_registry::Event::GlobalRemove { name }
                if state.seat_global_name == Some(name)
                    || state.manager_global_name == Some(name) =>
            {
                state.global_removed = true;
            }
            _ => {}
        }
    }
}

wayland_client::delegate_noop!(VkDiscovery: ignore wl_seat::WlSeat);
wayland_client::delegate_noop!(VkDiscovery: ignore ZwpVirtualKeyboardManagerV1);
wayland_client::delegate_noop!(VkDiscovery: ignore ZwpVirtualKeyboardV1);

struct VkSession {
    connection: Connection,
    queue: EventQueue<VkDiscovery>,
    discovery: VkDiscovery,
    vk: ZwpVirtualKeyboardV1,
    _keymap_fd: OwnedFd,
    started: Instant,
    last_timestamp_ms: u32,
    pressed_bitmap: [u64; 4],
    early_released_bitmap: [u64; 4],
    left_shift: bool,
    right_shift: bool,
    caps_lock: bool,
    num_lock: bool,
    keymap_synced: bool,
    layout: KeyboardLayout,
}

pub(crate) struct WaylandVirtualKeyboard {
    session: Mutex<VkSession>,
    healthy: AtomicBool,
}

impl WaylandVirtualKeyboard {
    pub(crate) fn connect_with_layout(
        layout: KeyboardLayout,
        initial_caps_lock: bool,
        initial_num_lock: bool,
    ) -> io::Result<Self> {
        if unsafe { libc::geteuid() } == 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "virtual keyboard refuses root connection",
            ));
        }
        let stream = crate::wayland_vk_connect::connect_wayland_socket()?;
        let connection = Connection::from_socket(stream).map_err(io::Error::other)?;
        Self::from_connection_with_layout(connection, layout, initial_caps_lock, initial_num_lock)
    }

    #[cfg(test)]
    fn from_connection(
        connection: Connection,
        initial_caps_lock: bool,
        initial_num_lock: bool,
    ) -> io::Result<Self> {
        Self::from_connection_with_layout(
            connection,
            KeyboardLayout::UsQwerty,
            initial_caps_lock,
            initial_num_lock,
        )
    }

    fn from_connection_with_layout(
        connection: Connection,
        layout: KeyboardLayout,
        initial_caps_lock: bool,
        initial_num_lock: bool,
    ) -> io::Result<Self> {
        let mut queue = connection.new_event_queue::<VkDiscovery>();
        let qh = queue.handle();
        let _registry = connection.display().get_registry(&qh, ());
        let mut discovery = VkDiscovery::default();

        roundtrip_with_timeout(
            &connection,
            &mut queue,
            &mut discovery,
            VK_ROUNDTRIP_TIMEOUT,
            "waylandvk registry roundtrip",
        )?;

        let seat = discovery.seat.as_ref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                "Wayland compositor has no wl_seat",
            )
        })?;
        let manager = discovery.manager.as_ref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                "Wayland compositor does not advertise zwp_virtual_keyboard_manager_v1",
            )
        })?;

        let vk = manager.create_virtual_keyboard(seat, &qh, ());
        let keymap = build_xkb_keymap_for_layout(layout);
        let (keymap_fd, keymap_size) =
            crate::wayland_vk_connect::create_keymap_memfd(keymap.as_bytes())?;
        vk.keymap(
            WL_KEYBOARD_KEYMAP_FORMAT_XKB_V1,
            keymap_fd.as_fd(),
            keymap_size,
        );
        let initial_locked = (if initial_caps_lock { XKB_MOD_LOCK } else { 0 })
            | (if initial_num_lock { XKB_MOD_MOD2 } else { 0 });
        vk.modifiers(0, 0, initial_locked, 0);

        roundtrip_with_timeout(
            &connection,
            &mut queue,
            &mut discovery,
            VK_ROUNDTRIP_TIMEOUT,
            "waylandvk keymap roundtrip",
        )?;

        Ok(Self {
            session: Mutex::new(VkSession {
                connection,
                queue,
                discovery,
                vk,
                _keymap_fd: keymap_fd,
                started: Instant::now(),
                last_timestamp_ms: 0,
                pressed_bitmap: [0; 4],
                early_released_bitmap: [0; 4],
                left_shift: false,
                right_shift: false,
                caps_lock: initial_caps_lock,
                num_lock: initial_num_lock,
                keymap_synced: false,
                layout,
            }),
            healthy: AtomicBool::new(true),
        })
    }

    #[inline]
    pub(crate) fn is_healthy(&self) -> bool {
        self.healthy.load(Ordering::Relaxed)
    }

    #[inline]
    pub(crate) fn can_encode_str(text: &str) -> bool {
        text.chars()
            .all(|ch| char_to_vk_stroke_for_layout(KeyboardLayout::UsQwerty, ch).is_some())
    }

    /// Record the desktop's tracked CapsLock state in memory without emitting a
    /// standalone `vk.modifiers` message: physical `KEY_CAPSLOCK` is forwarded
    /// via `/dev/uinput` so the compositor toggles it natively once, and future
    /// virtual-keyboard modifier updates preserve `XKB_MOD_LOCK` without racing.
    pub(crate) fn set_caps_lock(&self, enabled: bool) {
        if !self.is_healthy() {
            return;
        }
        let mut sess = self
            .session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        sess.caps_lock = enabled;
    }

    /// Record the desktop's tracked NumLock state in memory without emitting a
    /// standalone `vk.modifiers` message: physical `KEY_NUMLOCK` is forwarded
    /// via `/dev/uinput` so the compositor toggles it natively once, and future
    /// virtual-keyboard modifier updates preserve `XKB_MOD_MOD2` without racing
    /// or double-toggling NumLock back off.
    pub(crate) fn set_num_lock(&self, enabled: bool) {
        if !self.is_healthy() {
            return;
        }
        let mut sess = self
            .session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        sess.num_lock = enabled;
    }

    /// Mark that a key event was routed via `/dev/uinput`, which causes
    /// Smithay/COSMIC compositors to swap back to the system XKB keymap and
    /// recreate `xkb::State` with `mods_locked = 0`.
    pub(crate) fn mark_uinput_activity(&self) {
        if !self.is_healthy() {
            return;
        }
        let mut sess = self
            .session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        sess.keymap_synced = false;
    }

    /// Re-apply tracked locked modifiers (`NumLock` / `CapsLock`) via
    /// `zwp_virtual_keyboard_v1.modifiers` when no `/dev/uinput` system
    /// modifiers are held. On Smithay/COSMIC, `VirtualKeyboardSpecialEvent::Modifiers`
    /// updates the seat's `mods_locked` without switching keymaps, restoring
    /// the hardware NumLock/CapsLock LEDs immediately after a `uinput` keymap swap.
    pub(crate) fn sync_locked_modifiers(&self, force: bool) {
        if !self.is_healthy() {
            return;
        }
        let mut sess = self
            .session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let locked = sess.phys_locked();
        if !force && locked == 0 {
            return;
        }
        let depressed = sess.phys_depressed();
        sess.vk.modifiers(depressed, 0, locked, 0);
        if sess.flush_and_drain().is_err() {
            self.healthy.store(false, Ordering::Relaxed);
        }
    }

    /// Returns `true` if the key event was handled by `WaylandVirtualKeyboard`,
    /// or `false` if the caller should dispatch/release it via `uinput`.
    pub(crate) fn send_key_event(
        &self,
        keycode: u16,
        value: i32,
        uinput_mods_active: bool,
    ) -> bool {
        if !self.is_healthy() || keycode >= 256 {
            return false;
        }
        let mut sess = self
            .session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let was_pressed_on_vk = sess.is_bit_set(keycode);
        let was_early_released = sess.is_early_released(keycode);

        if value == 1 && was_early_released {
            sess.set_early_released(keycode, false);
        } else if value == 0 && !was_pressed_on_vk && was_early_released {
            sess.set_early_released(keycode, false);
            return true;
        } else if value == 2 && !was_pressed_on_vk && was_early_released {
            return true;
        }

        // If Ctrl/Alt/Super is held on uinput, new presses go to uinput,
        // EXCEPT releases of keys that were originally pressed on vk.
        if uinput_mods_active && value != 0 {
            return false;
        }
        if value == 0 && !was_pressed_on_vk {
            return false;
        }

        if keycode == KEY_LEFTSHIFT || keycode == KEY_RIGHTSHIFT {
            match value {
                1 => {
                    sess.set_bit(keycode, true);
                    if keycode == KEY_LEFTSHIFT {
                        sess.left_shift = true;
                    } else {
                        sess.right_shift = true;
                    }
                    let ts = sess.next_timestamp_ms();
                    sess.vk.key(ts, keycode as u32, 1);
                    let dep = sess.phys_depressed();
                    let lck = sess.phys_locked();
                    sess.vk.modifiers(dep, 0, lck, 0);
                    sess.keymap_synced = true;
                }
                0 => {
                    sess.set_bit(keycode, false);
                    if keycode == KEY_LEFTSHIFT {
                        sess.left_shift = false;
                    } else {
                        sess.right_shift = false;
                    }
                    let ts = sess.next_timestamp_ms();
                    sess.vk.key(ts, keycode as u32, 0);
                    let dep = sess.phys_depressed();
                    let lck = sess.phys_locked();
                    sess.vk.modifiers(dep, 0, lck, 0);
                    sess.keymap_synced = true;
                }
                _ => return true,
            }
            if sess.flush_and_drain().is_err() {
                self.healthy.store(false, Ordering::Relaxed);
                return false;
            }
            return true;
        }

        if !is_vk_typing_or_nav_keycode(keycode) {
            return false;
        }

        match value {
            1 => {
                let needs_sync = !sess.keymap_synced;
                let lck = sess.phys_locked();
                let dep = sess.phys_depressed();
                if needs_sync && (lck & XKB_MOD_LOCK) != 0 {
                    // Apply CapsLock before the key press so compositors that
                    // honor `modifiers` immediately decode the first letter uppercase.
                    sess.vk.modifiers(dep, 0, lck, 0);
                }
                let ts = sess.next_timestamp_ms();
                sess.set_bit(keycode, true);
                sess.vk.key(ts, keycode as u32, 1);
                if needs_sync {
                    sess.keymap_synced = true;
                    if lck != 0 {
                        // Also re-apply after the first key press for Smithay/COSMIC,
                        // which resets `mods_locked` when swapping back to the VK keymap.
                        sess.vk.modifiers(dep, 0, lck, 0);
                    }
                }
            }
            2 => {
                if !was_pressed_on_vk {
                    return false;
                }
                // Wayland compositors synthesize key-repeat natively on `wl_keyboard`
                // while a key remains pressed (`state = 1`). Emitting synthetic
                // release+press pairs here would double-repeat held keys and violate
                // strict per-event monotonic timestamps.
                return true;
            }
            0 => {
                let ts = sess.next_timestamp_ms();
                sess.set_bit(keycode, false);
                sess.vk.key(ts, keycode as u32, 0);
                if !sess.keymap_synced {
                    sess.keymap_synced = true;
                    let lck = sess.phys_locked();
                    if lck != 0 {
                        let dep = sess.phys_depressed();
                        sess.vk.modifiers(dep, 0, lck, 0);
                    }
                }
            }
            _ => return false,
        }

        if sess.flush_and_drain().is_err() {
            self.healthy.store(false, Ordering::Relaxed);
            return false;
        }
        true
    }

    /// Releases any Shift currently held on `WaylandVirtualKeyboard` and returns
    /// `(left_shift_was_held, right_shift_was_held)` so `UinputInjector` can
    /// seamlessly transfer Shift to `uinput` when Ctrl/Alt/Super is pressed.
    pub(crate) fn handoff_shift_to_uinput(&self) -> (bool, bool) {
        if !self.is_healthy() {
            return (false, false);
        }
        let mut sess = self
            .session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let left = sess.left_shift;
        let right = sess.right_shift;
        if !left && !right {
            return (false, false);
        }
        if left {
            let ts = sess.next_timestamp_ms();
            sess.left_shift = false;
            sess.set_bit(KEY_LEFTSHIFT, false);
            sess.vk.key(ts, KEY_LEFTSHIFT as u32, 0);
        }
        if right {
            let ts = sess.next_timestamp_ms();
            sess.right_shift = false;
            sess.set_bit(KEY_RIGHTSHIFT, false);
            sess.vk.key(ts, KEY_RIGHTSHIFT as u32, 0);
        }
        let lck = sess.phys_locked();
        sess.vk.modifiers(0, 0, lck, 0);
        if sess.flush_and_drain().is_err() {
            self.healthy.store(false, Ordering::Relaxed);
        }
        (left, right)
    }

    /// Reverse handoff: when the last Ctrl/Alt/Super modifier is released on `uinput`
    /// while `Shift` is still physically held, adopt `Shift` back onto `WaylandVirtualKeyboard`
    /// so subsequent typing and `Shift+Arrow` selection stay 100% synchronized.
    /// Returns `true` only if the handoff flushed cleanly to the compositor.
    pub(crate) fn adopt_shift_from_uinput(&self, left: bool, right: bool) -> bool {
        if !self.is_healthy() || (!left && !right) {
            return false;
        }
        let mut sess = self
            .session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if left && !sess.left_shift {
            let ts = sess.next_timestamp_ms();
            sess.left_shift = true;
            sess.set_bit(KEY_LEFTSHIFT, true);
            sess.vk.key(ts, KEY_LEFTSHIFT as u32, 1);
        }
        if right && !sess.right_shift {
            let ts = sess.next_timestamp_ms();
            sess.right_shift = true;
            sess.set_bit(KEY_RIGHTSHIFT, true);
            sess.vk.key(ts, KEY_RIGHTSHIFT as u32, 1);
        }
        let dep = sess.phys_depressed();
        let lck = sess.phys_locked();
        sess.vk.modifiers(dep, 0, lck, 0);
        if sess.flush_and_drain().is_err() {
            self.healthy.store(false, Ordering::Relaxed);
            return false;
        }
        true
    }

    /// Atomically deletes `extra_deletes_after_caret` forward selections (for
    /// browser omnibox inline autocomplete), deletes `backspaces` characters
    /// before the caret, and types `text` over the single `zwp_virtual_keyboard_v1`
    /// stream with explicit modifier isolation.
    pub(crate) fn replace_text(
        &self,
        backspaces: usize,
        extra_deletes_after_caret: usize,
        text: &str,
    ) -> io::Result<()> {
        if !self.is_healthy() {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "virtual keyboard unhealthy",
            ));
        }
        let mut sess = self
            .session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut strokes = Vec::with_capacity(text.chars().count());
        for ch in text.chars() {
            let Some(stroke) = char_to_vk_stroke_for_layout(sess.layout, ch) else {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "character outside virtual-keyboard keymap",
                ));
            };
            strokes.push(stroke);
        }

        // Release any held typing keys before atomic backspaces and strokes.
        // During fast typing with key overlap, the preceding character (e.g. 'h' in "thành"
        // or 'c' in "các") may still be physically down on evdev and marked pressed on vk.
        // If left pressed, emitting that same scancode in the replacement is discarded
        // by compositors/clients as a duplicate KeyDown, dropping the letter!
        for idx in 0..4 {
            let word = sess.pressed_bitmap[idx];
            if word != 0 {
                for bit in 0..64 {
                    if (word & (1u64 << bit)) != 0 {
                        let code = (idx * 64 + bit) as u16;
                        if code != KEY_LEFTSHIFT && code != KEY_RIGHTSHIFT {
                            let ts = sess.next_timestamp_ms();
                            sess.vk.key(ts, code as u32, 0);
                            sess.early_released_bitmap[idx] |= 1u64 << bit;
                        }
                    }
                }
                let shift_mask = if idx == 0 {
                    (1u64 << KEY_LEFTSHIFT) | (1u64 << KEY_RIGHTSHIFT)
                } else {
                    0
                };
                sess.pressed_bitmap[idx] &= shift_mask;
            }
        }

        // Neutralize Shift (and CapsLock only when active AND inserting lock-affected
        // alphabetic/Vietnamese keys) so BackSpace/Delete and ASCII case stay 100%
        // deterministic without flickering the CapsLock LED or emitting redundant
        // modifier packets on the common path.
        let needs_caps_unlock = sess.caps_lock
            && strokes.iter().any(|s| {
                is_layout_alpha_evdev(sess.layout, s.evdev_code)
                    || ((s.modifiers & !XKB_MOD_SHIFT) != 0
                        && crate::wayland_vk_keymap::is_eight_level_evdev(s.evdev_code))
            });
        let edit_locked = if needs_caps_unlock {
            sess.num_lock_mask()
        } else {
            sess.phys_locked()
        };
        let mut active_depressed = 0u32;
        if sess.phys_depressed() != 0 || edit_locked != sess.phys_locked() {
            sess.vk.modifiers(0, 0, edit_locked, 0);
        }

        for _ in 0..extra_deletes_after_caret {
            let ts = sess.next_timestamp_ms();
            sess.vk.key(ts, KEY_DELETE, 1);
            let ts = sess.next_timestamp_ms();
            sess.vk.key(ts, KEY_DELETE, 0);
        }
        for _ in 0..backspaces {
            let ts = sess.next_timestamp_ms();
            sess.vk.key(ts, KEY_BACKSPACE, 1);
            let ts = sess.next_timestamp_ms();
            sess.vk.key(ts, KEY_BACKSPACE, 0);
        }

        for stroke in strokes {
            let needed_depressed = stroke.modifiers;
            if needed_depressed != active_depressed {
                sess.vk.modifiers(needed_depressed, 0, edit_locked, 0);
                active_depressed = needed_depressed;
            }
            let ts = sess.next_timestamp_ms();
            sess.vk.key(ts, stroke.evdev_code, 1);
            let ts = sess.next_timestamp_ms();
            sess.vk.key(ts, stroke.evdev_code, 0);
        }

        // Restore physical Shift + CapsLock + NumLock state for subsequent pass-through keys.
        let was_unsynced = !sess.keymap_synced;
        sess.keymap_synced = true;
        let restored_dep = sess.phys_depressed();
        let restored_lck = sess.phys_locked();
        if active_depressed != restored_dep
            || restored_lck != edit_locked
            || (was_unsynced && restored_lck != 0)
        {
            sess.vk.modifiers(restored_dep, 0, restored_lck, 0);
        }

        if let Err(err) = sess.flush_and_drain() {
            self.healthy.store(false, Ordering::Relaxed);
            return Err(err);
        }
        Ok(())
    }
}

impl VkSession {
    #[inline]
    fn next_timestamp_ms(&mut self) -> u32 {
        let wall = self.started.elapsed().as_millis() as u32;
        // Wrap-safe monotonic advance across the 49.7-day u32::MAX boundary (Invariant 2):
        // treat `wall` as ahead of `last_timestamp_ms` only when their modular delta < 2^31.
        let ahead = wall.wrapping_sub(self.last_timestamp_ms);
        let base = if ahead > 0 && ahead < 0x8000_0000 {
            wall
        } else {
            self.last_timestamp_ms
        };
        self.last_timestamp_ms = base.wrapping_add(1);
        self.last_timestamp_ms
    }

    #[inline]
    fn phys_depressed(&self) -> u32 {
        if self.left_shift || self.right_shift {
            XKB_MOD_SHIFT
        } else {
            0
        }
    }

    #[inline]
    fn num_lock_mask(&self) -> u32 {
        if self.num_lock {
            XKB_MOD_MOD2
        } else {
            0
        }
    }

    #[inline]
    fn phys_locked(&self) -> u32 {
        let mut locked = self.num_lock_mask();
        if self.caps_lock {
            locked |= XKB_MOD_LOCK;
        }
        locked
    }

    #[inline]
    fn is_bit_set(&self, keycode: u16) -> bool {
        let idx = (keycode / 64) as usize;
        let bit = keycode % 64;
        (self.pressed_bitmap[idx] & (1u64 << bit)) != 0
    }

    #[inline]
    fn set_bit(&mut self, keycode: u16, pressed: bool) {
        let idx = (keycode / 64) as usize;
        let bit = keycode % 64;
        if pressed {
            self.pressed_bitmap[idx] |= 1u64 << bit;
        } else {
            self.pressed_bitmap[idx] &= !(1u64 << bit);
        }
    }

    #[inline]
    fn is_early_released(&self, keycode: u16) -> bool {
        let idx = (keycode / 64) as usize;
        let bit = keycode % 64;
        (self.early_released_bitmap[idx] & (1u64 << bit)) != 0
    }

    #[inline]
    fn set_early_released(&mut self, keycode: u16, released: bool) {
        let idx = (keycode / 64) as usize;
        let bit = keycode % 64;
        if released {
            self.early_released_bitmap[idx] |= 1u64 << bit;
        } else {
            self.early_released_bitmap[idx] &= !(1u64 << bit);
        }
    }

    fn flush_and_drain(&mut self) -> io::Result<()> {
        for attempt in 0..5 {
            match self.connection.flush() {
                Ok(()) => break,
                Err(wayland_client::backend::WaylandError::Io(ref e))
                    if e.kind() == io::ErrorKind::WouldBlock && attempt < 4 =>
                {
                    let mut pfd = libc::pollfd {
                        fd: self.connection.backend().poll_fd().as_raw_fd(),
                        events: libc::POLLOUT,
                        revents: 0,
                    };
                    let _ = unsafe { libc::poll(&mut pfd, 1, 5) };
                }
                Err(e) => return Err(io::Error::other(e)),
            }
        }
        if let Some(guard) = self.queue.prepare_read() {
            let mut pfd = libc::pollfd {
                fd: guard.connection_fd().as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            };
            let ready = unsafe { libc::poll(&mut pfd, 1, 0) };
            if (pfd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL)) != 0 {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "Wayland compositor socket closed",
                ));
            }
            if ready > 0 {
                match guard.read() {
                    Ok(_) => {}
                    Err(wayland_client::backend::WaylandError::Io(e))
                        if e.kind() == io::ErrorKind::WouldBlock => {}
                    Err(wayland_client::backend::WaylandError::Io(e)) => return Err(e),
                    Err(wayland_client::backend::WaylandError::Protocol(e)) => {
                        return Err(io::Error::other(e));
                    }
                }
            }
        }
        self.queue
            .dispatch_pending(&mut self.discovery)
            .map_err(io::Error::other)?;
        if self.discovery.global_removed {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "Wayland seat or virtual keyboard manager global removed",
            ));
        }
        Ok(())
    }
}

impl Drop for VkSession {
    fn drop(&mut self) {
        self.vk.destroy();
        let _ = self.connection.flush();
    }
}

#[cfg(test)]
#[path = "wayland_vk_tests.rs"]
mod tests;
