// SPDX-License-Identifier: MIT
//! Persistent in-process Wayland `zwp_virtual_keyboard_v1` backend.
//!
//! Injects BackSpace, Delete, ASCII, and all 134 Vietnamese Unicode characters
//! directly over a single Wayland virtual keyboard channel with zero preedit
//! underline, zero IBus/Fcitx5 dependency, and zero clipboard interaction.

use std::io;
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};

use crate::wayland_vk_keymap::{
    build_xkb_keymap, char_to_vk_stroke, is_vk_typing_or_nav_keycode, XKB_MOD_LOCK, XKB_MOD_MOD2,
    XKB_MOD_SHIFT,
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

#[derive(Default)]
struct VkDiscovery {
    seat: Option<wl_seat::WlSeat>,
    manager: Option<ZwpVirtualKeyboardManagerV1>,
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
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            if interface == "wl_seat" && state.seat.is_none() {
                state.seat =
                    Some(registry.bind::<wl_seat::WlSeat, _, _>(name, version.min(7), qh, ()));
            } else if interface == "zwp_virtual_keyboard_manager_v1" && state.manager.is_none() {
                state.manager =
                    Some(registry.bind::<ZwpVirtualKeyboardManagerV1, _, _>(name, 1, qh, ()));
            }
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
}

pub(crate) struct WaylandVirtualKeyboard {
    session: Mutex<VkSession>,
    healthy: AtomicBool,
}

impl WaylandVirtualKeyboard {
    pub(crate) fn connect(initial_caps_lock: bool, initial_num_lock: bool) -> io::Result<Self> {
        if unsafe { libc::geteuid() } == 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "virtual keyboard refuses root connection",
            ));
        }
        let socket_path = resolve_wayland_socket_path()?;
        let stream = connect_unix_nonblocking(&socket_path)?;
        let connection = Connection::from_socket(stream).map_err(io::Error::other)?;
        Self::from_connection(connection, initial_caps_lock, initial_num_lock)
    }

    fn from_connection(
        connection: Connection,
        initial_caps_lock: bool,
        initial_num_lock: bool,
    ) -> io::Result<Self> {
        let mut queue = connection.new_event_queue::<VkDiscovery>();
        let qh = queue.handle();
        let _registry = connection.display().get_registry(&qh, ());
        let mut discovery = VkDiscovery::default();

        queue
            .roundtrip(&mut discovery)
            .map_err(|e| io::Error::other(format!("waylandvk registry roundtrip: {e}")))?;

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
        let keymap = build_xkb_keymap();
        let (keymap_fd, keymap_size) = create_keymap_memfd(keymap.as_bytes())?;
        vk.keymap(
            WL_KEYBOARD_KEYMAP_FORMAT_XKB_V1,
            keymap_fd.as_fd(),
            keymap_size,
        );
        let initial_locked = (if initial_caps_lock { XKB_MOD_LOCK } else { 0 })
            | (if initial_num_lock { XKB_MOD_MOD2 } else { 0 });
        vk.modifiers(0, 0, initial_locked, 0);

        queue
            .roundtrip(&mut discovery)
            .map_err(|e| io::Error::other(format!("waylandvk keymap roundtrip: {e}")))?;

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
        text.chars().all(|ch| char_to_vk_stroke(ch).is_some())
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
                let ts = sess.next_timestamp_ms();
                sess.set_bit(keycode, true);
                sess.vk.key(ts, keycode as u32, 1);
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
        let ts = sess.next_timestamp_ms();
        if left {
            sess.left_shift = false;
            sess.set_bit(KEY_LEFTSHIFT, false);
            sess.vk.key(ts, KEY_LEFTSHIFT as u32, 0);
        }
        if right {
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
    pub(crate) fn adopt_shift_from_uinput(&self, left: bool, right: bool) {
        if !self.is_healthy() || (!left && !right) {
            return;
        }
        let mut sess = self
            .session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let ts = sess.next_timestamp_ms();
        if left && !sess.left_shift {
            sess.left_shift = true;
            sess.set_bit(KEY_LEFTSHIFT, true);
            sess.vk.key(ts, KEY_LEFTSHIFT as u32, 1);
        }
        if right && !sess.right_shift {
            sess.right_shift = true;
            sess.set_bit(KEY_RIGHTSHIFT, true);
            sess.vk.key(ts, KEY_RIGHTSHIFT as u32, 1);
        }
        let dep = sess.phys_depressed();
        let lck = sess.phys_locked();
        sess.vk.modifiers(dep, 0, lck, 0);
        if sess.flush_and_drain().is_err() {
            self.healthy.store(false, Ordering::Relaxed);
        }
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
        let mut strokes = Vec::with_capacity(text.chars().count());
        for ch in text.chars() {
            let Some(stroke) = char_to_vk_stroke(ch) else {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "character outside virtual-keyboard keymap",
                ));
            };
            strokes.push(stroke);
        }

        let mut sess = self
            .session
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

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

        // Neutralize Shift and CapsLock during atomic edit so BackSpace/Delete
        // and ASCII case are 100% deterministic regardless of physical keys,
        // while preserving NumLock (Mod2) so the compositor never disables it.
        let edit_locked = sess.num_lock_mask();
        let mut active_depressed = 0u32;
        sess.vk.modifiers(0, 0, edit_locked, 0);

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
        let restored_dep = sess.phys_depressed();
        let restored_lck = sess.phys_locked();
        if active_depressed != restored_dep || restored_lck != edit_locked {
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
        Ok(())
    }
}

impl Drop for VkSession {
    fn drop(&mut self) {
        self.vk.destroy();
        let _ = self.connection.flush();
    }
}

fn create_keymap_memfd(keymap_bytes: &[u8]) -> io::Result<(OwnedFd, u32)> {
    let raw = unsafe { libc::memfd_create(c"dboard-vk-keymap".as_ptr(), libc::MFD_CLOEXEC) };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    let mut buf = Vec::with_capacity(keymap_bytes.len() + 1);
    buf.extend_from_slice(keymap_bytes);
    buf.push(0); // XKB v1 Wayland format requires NUL-terminated buffer included in size.

    let mut offset = 0;
    while offset < buf.len() {
        let written = unsafe {
            libc::write(
                fd.as_raw_fd(),
                buf[offset..].as_ptr().cast(),
                buf.len() - offset,
            )
        };
        if written < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
        }
        offset += written as usize;
    }
    let _ = unsafe { libc::lseek(fd.as_raw_fd(), 0, libc::SEEK_SET) };
    Ok((fd, buf.len() as u32))
}

fn resolve_wayland_socket_path() -> io::Result<PathBuf> {
    if std::env::var_os("WAYLAND_SOCKET").is_some() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "inherited WAYLAND_SOCKET is unsupported",
        ));
    }
    let runtime = PathBuf::from(
        std::env::var_os("XDG_RUNTIME_DIR")
            .filter(|v| !v.is_empty())
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "XDG_RUNTIME_DIR missing"))?,
    );
    if !runtime.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "XDG_RUNTIME_DIR must be absolute",
        ));
    }

    if let Some(display) = std::env::var_os("WAYLAND_DISPLAY").filter(|v| !v.is_empty()) {
        let candidate = PathBuf::from(display);
        let path = if candidate.is_absolute() {
            candidate
        } else {
            runtime.join(candidate)
        };
        if path.exists() {
            return Ok(path);
        }
    }

    for index in 0..10 {
        let candidate = runtime.join(format!("wayland-{index}"));
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "no Wayland socket found in XDG_RUNTIME_DIR",
    ))
}

fn connect_unix_nonblocking(path: &Path) -> io::Result<UnixStream> {
    let bytes = path.as_os_str().as_bytes();
    let mut addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if bytes.len() >= addr.sun_path.len() || bytes.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid Wayland socket path",
        ));
    }
    addr.sun_family = libc::AF_UNIX as _;
    for (dst, &src) in addr.sun_path.iter_mut().zip(bytes) {
        *dst = src as _;
    }
    let raw = unsafe {
        libc::socket(
            libc::AF_UNIX,
            libc::SOCK_STREAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
            0,
        )
    };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    if unsafe {
        libc::connect(
            fd.as_raw_fd(),
            (&addr as *const libc::sockaddr_un).cast(),
            std::mem::size_of_val(&addr) as _,
        )
    } < 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(UnixStream::from(fd))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::thread;
    use std::time::Duration;

    fn u32_ne(bytes: &[u8]) -> u32 {
        u32::from_ne_bytes(bytes[..4].try_into().unwrap())
    }

    fn wl_string(value: &str) -> Vec<u8> {
        let mut bytes = ((value.len() + 1) as u32).to_ne_bytes().to_vec();
        bytes.extend_from_slice(value.as_bytes());
        bytes.push(0);
        bytes.resize((bytes.len() + 3) & !3, 0);
        bytes
    }

    fn read_msg(sock: &mut UnixStream) -> (u32, u16, Vec<u8>) {
        let mut header = [0u8; 8];
        sock.read_exact(&mut header).unwrap();
        let obj = u32_ne(&header);
        let word = u32_ne(&header[4..]);
        let op = word as u16;
        let size = (word >> 16) as usize;
        let mut payload = vec![0u8; size - 8];
        sock.read_exact(&mut payload).unwrap();
        (obj, op, payload)
    }

    fn send_event(sock: &mut UnixStream, obj: u32, op: u16, args: &[u8]) {
        let mut pkt = obj.to_ne_bytes().to_vec();
        let header = (((args.len() + 8) as u32) << 16) | u32::from(op);
        pkt.extend_from_slice(&header.to_ne_bytes());
        pkt.extend_from_slice(args);
        sock.write_all(&pkt).unwrap();
    }

    #[test]
    fn wire_virtual_keyboard_uploads_keymap_and_emits_atomic_vietnamese_replacement() {
        let (client, mut server) = UnixStream::pair().unwrap();
        client.set_nonblocking(true).unwrap();
        server
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        server
            .set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();

        let server_thread = thread::spawn(move || {
            // 1. Registry discovery + roundtrip
            let (disp1, op1, reg_args) = read_msg(&mut server);
            assert_eq!((disp1, op1), (1, 1));
            let registry_id = u32_ne(&reg_args);

            let (disp2, op2, sync1_args) = read_msg(&mut server);
            assert_eq!((disp2, op2), (1, 0));
            let sync1_id = u32_ne(&sync1_args);

            let mut seat_global = 10u32.to_ne_bytes().to_vec();
            seat_global.extend(wl_string("wl_seat"));
            seat_global.extend(7u32.to_ne_bytes());
            send_event(&mut server, registry_id, 0, &seat_global);

            let mut vk_global = 20u32.to_ne_bytes().to_vec();
            vk_global.extend(wl_string("zwp_virtual_keyboard_manager_v1"));
            vk_global.extend(1u32.to_ne_bytes());
            send_event(&mut server, registry_id, 0, &vk_global);

            send_event(&mut server, sync1_id, 0, &1u32.to_ne_bytes());
            send_event(&mut server, 1, 1, &sync1_id.to_ne_bytes());

            // 2. Bind seat, bind vk manager, create_virtual_keyboard, keymap, modifiers, roundtrip
            let (r1, rop1, _) = read_msg(&mut server);
            assert_eq!((r1, rop1), (registry_id, 0));
            let (r2, rop2, bind_mgr_args) = read_msg(&mut server);
            assert_eq!((r2, rop2), (registry_id, 0));
            let mgr_id = u32_ne(&bind_mgr_args[bind_mgr_args.len() - 4..]);

            let (m_obj, m_op, create_vk_args) = read_msg(&mut server);
            assert_eq!((m_obj, m_op), (mgr_id, 0));
            let vk_id = u32_ne(&create_vk_args[4..]);

            let (k_obj, k_op, keymap_args) = read_msg(&mut server);
            assert_eq!((k_obj, k_op), (vk_id, 0));
            assert_eq!(u32_ne(&keymap_args), WL_KEYBOARD_KEYMAP_FORMAT_XKB_V1);
            assert!(u32_ne(&keymap_args[4..]) > 1000);

            let (mod_obj, mod_op, _) = read_msg(&mut server);
            assert_eq!((mod_obj, mod_op), (vk_id, 2));

            let (disp3, op3, sync2_args) = read_msg(&mut server);
            assert_eq!((disp3, op3), (1, 0));
            let sync2_id = u32_ne(&sync2_args);
            send_event(&mut server, sync2_id, 0, &2u32.to_ne_bytes());
            send_event(&mut server, 1, 1, &sync2_id.to_ne_bytes());

            // 3. Read atomic replacement: extra_deletes=1, backspaces=2, text="Việt"
            let mut key_ops = Vec::new();
            let mut mod_ops = Vec::new();
            loop {
                let (obj, op, args) = read_msg(&mut server);
                if obj == vk_id && op == 3 {
                    // destroy on drop
                    break;
                }
                if obj == vk_id && op == 1 {
                    let key = u32_ne(&args[4..]);
                    let state = u32_ne(&args[8..]);
                    key_ops.push((key, state));
                } else if obj == vk_id && op == 2 {
                    let depressed = u32_ne(&args[0..4]);
                    let locked = u32_ne(&args[8..12]);
                    mod_ops.push((depressed, locked));
                }
            }
            (key_ops, mod_ops)
        });

        let conn = Connection::from_socket(client).unwrap();
        let vk = WaylandVirtualKeyboard::from_connection(conn, false, true).unwrap();
        assert!(vk.is_healthy());
        assert!(WaylandVirtualKeyboard::can_encode_str(
            "Xin chào Việt Nam! Đỗ Ước 2026"
        ));
        assert!(!WaylandVirtualKeyboard::can_encode_str("🚀"));

        vk.set_num_lock(true);
        vk.set_caps_lock(true);
        // Held key press + hardware repeat (value=2) + release must not emit
        // synthetic release/press pairs (compositor handles native key repeat).
        assert!(vk.send_key_event(30, 1, false));
        assert!(vk.send_key_event(30, 2, false));
        assert!(vk.send_key_event(30, 0, false));
        vk.replace_text(2, 1, "Việt").unwrap();
        drop(vk);

        let (key_ops, mod_ops) = server_thread.join().unwrap();
        assert!(
            !mod_ops.is_empty() && mod_ops.iter().all(|&(_, locked)| (locked & XKB_MOD_MOD2) != 0),
            "NumLock (XKB_MOD_MOD2) must never be cleared during virtual-keyboard modifier updates: {mod_ops:?}"
        );
        assert_eq!(
            mod_ops.last().copied(),
            Some((0, XKB_MOD_LOCK | XKB_MOD_MOD2)),
            "CapsLock and NumLock must both be restored after atomic replacement"
        );
        let v_stroke = char_to_vk_stroke('V').unwrap();
        let i_stroke = char_to_vk_stroke('i').unwrap();
        let e_stroke = char_to_vk_stroke('ệ').unwrap();
        let t_stroke = char_to_vk_stroke('t').unwrap();

        assert_eq!(
            key_ops,
            vec![
                (30, 1),
                (30, 0),
                (KEY_DELETE, 1),
                (KEY_DELETE, 0),
                (KEY_BACKSPACE, 1),
                (KEY_BACKSPACE, 0),
                (KEY_BACKSPACE, 1),
                (KEY_BACKSPACE, 0),
                (v_stroke.evdev_code, 1),
                (v_stroke.evdev_code, 0),
                (i_stroke.evdev_code, 1),
                (i_stroke.evdev_code, 0),
                (e_stroke.evdev_code, 1),
                (e_stroke.evdev_code, 0),
                (t_stroke.evdev_code, 1),
                (t_stroke.evdev_code, 0),
            ]
        );
    }
}
