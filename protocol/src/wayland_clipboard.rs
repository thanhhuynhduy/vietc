// SPDX-License-Identifier: MIT
//! Scoped, immutable native Wayland clipboard source. Success means **PayloadServed**:
//! an armed text request was fully written and its FD closed, NOT target insertion
//! or proof the reader consumed bytes (they may still be buffered in the pipe).
//! A clipboard manager requesting after arming is indistinguishable from a target.
//! Arming uses a display-sync boundary; initiation means source-event dispatch,
//! not an unknowable timestamp in another process. Pre-boundary sends never count.
//! Wait timeout/focus loss does not restore, republish, or withdraw the selection;
//! retain the offer to keep serving it. Drop aborts outstanding transfers and
//! disconnects without restoring old data. After successful cleanup, retire instead:
//! it rejects new requests and drains accepted transfers to their existing deadlines.
//! request_retire() starts this without joining; retain the handle until is_finished().
//! Transport errors report Unavailable promptly, then drain accepted pipes even
//! without retirement. Only an explicit source Cancelled event reports Replaced.
//! Only one seat is supported. Inherited WAYLAND_SOCKET connections are rejected
//! rather than consumed or modified. Transfer FDs must be pipes or sockets
//! (regular files cannot guarantee nonblocking writes). Transfers expire after 5s;
//! new requests remain serviceable. No clipboard reads or keyboard commands occur.

use std::{
    io,
    os::{
        fd::{AsFd, AsRawFd, FromRawFd, OwnedFd},
        unix::{ffi::OsStrExt, net::UnixStream},
    },
    sync::Arc,
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use transfer::*;
use wayland_client::{
    backend::WaylandError,
    protocol::{wl_callback, wl_registry, wl_seat},
    Connection, Dispatch, QueueHandle,
};
use wayland_protocols::ext::data_control::v1::client::*;
use wayland_protocols_wlr::data_control::v1::client::*;

const TICK: Duration = Duration::from_millis(20);
const CONTROL_TIMEOUT: Duration = Duration::from_secs(5);
const MIMES: [&str; 3] = ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING"];

/// Only an explicit source cancellation proves replacement. Other loss of
/// observability is Unavailable: callers must not infer that restoring is safe.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Ownership {
    Owned,
    Replaced,
    #[default]
    Unavailable,
}

pub(super) struct ClipboardOffer {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}

impl ClipboardOffer {
    pub(super) fn publish(text: &str, timeout: Duration) -> io::Result<Self> {
        // Check before allocating state, spawning, connecting, or changing selection.
        if unsafe { libc::getuid() } == 0 {
            return Err(err(
                io::ErrorKind::PermissionDenied,
                "clipboard source refuses root",
            ));
        }
        let until = deadline(timeout)?;
        // Return ownership before waiting so a publication timeout cannot
        // discard the caller's only clipboard snapshot or a possibly live source.
        Self::start(text, until, connect)
    }

    // The connector is injected only internally; tests use a socket pair, never
    // the environment. Root rejection remains mandatory at the public entry.
    fn start(
        text: &str,
        until: Instant,
        connector: impl FnOnce() -> io::Result<Connection> + Send + 'static,
    ) -> io::Result<Self> {
        let payload = freeze_payload(text)?;
        let shared = Arc::new(Shared::default());
        let state = shared.clone();
        let worker = thread::Builder::new()
            .name("wayland-clipboard".into())
            .spawn(move || {
                let result = if state
                    .state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .stop
                {
                    Ok(())
                } else {
                    connector()
                        .and_then(|connection| run(connection, state.clone(), payload, until))
                };
                if let Err(e) = result {
                    state.fail(&e);
                } else {
                    let mut s = state
                        .state
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner());
                    if s.ownership == Ownership::Owned {
                        s.ownership = Ownership::Unavailable;
                    }
                    state.changed.notify_all();
                }
            })?;
        Ok(Self {
            shared,
            worker: Some(worker),
        })
    }

    pub(super) fn wait_until_published(&self, timeout: Duration) -> io::Result<()> {
        self.await_control(deadline(timeout)?, |s| s.ready)
    }

    /// Idempotent, one-time arming; waits for a server ordering barrier.
    pub(super) fn arm(&self) -> io::Result<()> {
        let until = deadline(CONTROL_TIMEOUT)?;
        {
            let mut s = self
                .shared
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            s.ensure_owned()?;
            if !s.ready {
                return Err(err(io::ErrorKind::WouldBlock, "publication not ready"));
            }
            s.arm_requested = true;
        }
        self.await_control(until, |s| s.armed)
    }

    /// Reports only PayloadServed. Failure leaves this offer and its payload alive.
    /// The focus callback must be nonblocking. Repeated waits share one armed result.
    pub(super) fn wait_for_transfer(
        &self,
        timeout: Duration,
        still_focused: impl Fn() -> bool,
    ) -> io::Result<()> {
        let until = deadline(timeout)?;
        loop {
            // Do not invoke caller code while holding the state lock.
            let focused = still_focused();
            let s = self
                .shared
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let now = Instant::now();
            if let Some(result) = s.check(now, until, focused) {
                return result;
            }
            drop(
                self.shared
                    .changed
                    .wait_timeout(s, TICK.min(until.saturating_duration_since(now)))
                    .unwrap(),
            );
        }
    }

    /// Last observed ownership. Unavailable must not be treated as Replaced.
    pub(super) fn ownership(&self) -> Ownership {
        let s = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if s.ownership == Ownership::Owned && (!s.ready || s.stop) {
            Ownership::Unavailable
        } else {
            s.ownership
        }
    }

    pub(super) fn owns_selection(&self) -> bool {
        self.ownership() == Ownership::Owned
    }

    /// Initiate bounded draining without joining. Keep this handle alive until
    /// is_finished(); dropping it earlier still explicitly aborts accepted pipes.
    pub(super) fn request_retire(&self) {
        self.shared
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .retire = true;
        self.shared.changed.notify_all();
    }

    pub(super) fn is_finished(&self) -> bool {
        self.worker.as_ref().is_none_or(JoinHandle::is_finished)
    }

    /// Stop accepting sends, drain accepted transfers to their existing deadlines
    /// (at most five seconds remaining), and join the existing worker. No restore
    /// is performed. Unlike Drop, this preserves accepted readers after cleanup.
    #[cfg(test)]
    pub(super) fn retire(mut self) {
        self.request_retire();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }

    fn await_control(&self, until: Instant, done: impl Fn(&State) -> bool) -> io::Result<()> {
        let mut s = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        loop {
            if let Some((kind, message)) = &s.error {
                return Err(err(*kind, message));
            }
            if s.ready {
                s.ensure_owned()?;
            }
            if done(&s) {
                return Ok(());
            }
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(err(io::ErrorKind::TimedOut, "Wayland control deadline"));
            }
            s = self
                .shared
                .changed
                .wait_timeout(s, left.min(TICK))
                .unwrap()
                .0;
        }
    }
}

impl Drop for ClipboardOffer {
    fn drop(&mut self) {
        self.shared
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .stop = true;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn deadline(timeout: Duration) -> io::Result<Instant> {
    Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| err(io::ErrorKind::InvalidInput, "deadline overflow"))
}
fn other(e: impl std::fmt::Display) -> io::Error {
    io::Error::other(e.to_string())
}

// Nonblocking AF_UNIX connect fails fast if the compositor backlog is full.
// Unlike connect_to_env(), this cannot wait indefinitely or mutate environment/FDs.
fn connect() -> io::Result<Connection> {
    if std::env::var_os("WAYLAND_SOCKET").is_some() {
        return Err(err(
            io::ErrorKind::Unsupported,
            "inherited WAYLAND_SOCKET is unsupported",
        ));
    }
    let display = std::path::PathBuf::from(
        std::env::var_os("WAYLAND_DISPLAY").unwrap_or_else(|| "wayland-0".into()),
    );
    let path = if display.is_absolute() {
        display
    } else {
        let runtime = std::path::PathBuf::from(
            std::env::var_os("XDG_RUNTIME_DIR")
                .ok_or_else(|| err(io::ErrorKind::NotFound, "XDG_RUNTIME_DIR missing"))?,
        );
        if !runtime.is_absolute() {
            return Err(err(
                io::ErrorKind::InvalidInput,
                "runtime directory must be absolute",
            ));
        }
        runtime.join(display)
    };
    let bytes = path.as_os_str().as_bytes();
    // Zeroed sockaddr_un has a valid family/path representation and NUL terminator.
    let mut addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if bytes.len() >= addr.sun_path.len() || bytes.contains(&0) {
        return Err(err(
            io::ErrorKind::InvalidInput,
            "invalid Wayland socket path",
        ));
    }
    addr.sun_family = libc::AF_UNIX as _;
    for (dst, src) in addr.sun_path.iter_mut().zip(bytes) {
        *dst = *src as _;
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
    Connection::from_socket(UnixStream::from(fd)).map_err(other)
}

#[derive(Clone, Copy)]
enum Barrier {
    Registry,
    Published,
    Armed,
}
struct Worker {
    shared: Arc<Shared>,
    globals: Vec<(u32, String, u32)>,
    discovered: bool,
    seat: Option<u32>,
    transfers: Vec<Transfer>,
}

fn run(
    connection: Connection,
    shared: Arc<Shared>,
    payload: Arc<[u8]>,
    until: Instant,
) -> io::Result<()> {
    block_sigpipe()?; // Thread-local: never change the process-wide SIGPIPE disposition.
    if shared
        .state
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .stop
    {
        return Ok(());
    }
    let mut queue = connection.new_event_queue::<Worker>();
    let qh = queue.handle();
    let registry = connection.display().get_registry(&qh, ());
    connection.display().sync(&qh, Barrier::Registry);
    let mut w = Worker {
        shared,
        globals: Vec::new(),
        discovered: false,
        seat: None,
        transfers: Vec::new(),
    };
    let mut published = false;
    let mut arm_pending = false;
    let result = (|| loop {
        let retiring = {
            let state = w
                .shared
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if state.stop {
                return Ok(());
            }
            state.retire
        };
        if retiring {
            return Ok(());
        }
        queue.dispatch_pending(&mut w).map_err(other)?;
        if !published && Instant::now() >= until {
            return Err(err(io::ErrorKind::TimedOut, "discovery deadline"));
        }
        if w.discovered && !published {
            w.publish(&registry, &qh)?;
            connection.display().sync(&qh, Barrier::Published);
            published = true;
        }
        {
            let s = w
                .shared
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if !s.ready && Instant::now() >= until {
                return Err(err(io::ErrorKind::TimedOut, "publication deadline"));
            }
            if s.arm_requested && !s.armed && !arm_pending && s.ownership == Ownership::Owned {
                connection.display().sync(&qh, Barrier::Armed);
                arm_pending = true;
            }
        }
        w.serve_pending(&payload);
        let writable = match connection.flush() {
            Ok(()) => false,
            Err(WaylandError::Io(e)) if e.kind() == io::ErrorKind::WouldBlock => true,
            Err(e) => return Err(other(e)),
        };
        let Some(guard) = queue.prepare_read() else {
            continue;
        };
        let mut fds = vec![libc::pollfd {
            fd: queue.as_fd().as_raw_fd(),
            events: libc::POLLIN | if writable { libc::POLLOUT } else { 0 },
            revents: 0,
        }];
        fds.extend(w.transfers.iter().map(|t| libc::pollfd {
            fd: t.fd.as_raw_fd(),
            events: libc::POLLOUT,
            revents: 0,
        }));
        let result = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as _, TICK.as_millis() as _) };
        if result < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e);
        }
        if fds[0].revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
            return Err(err(io::ErrorKind::BrokenPipe, "Wayland disconnected"));
        }
        if fds[0].revents & libc::POLLIN != 0 {
            match guard.read() {
                Ok(_) => {}
                Err(WaylandError::Io(e))
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) => {}
                Err(e) => return Err(other(e)),
            }
        }
    })();
    // Report uncertainty BEFORE draining: a stalled reader must not delay the
    // caller learning that the Wayland connection has failed. Accepted pipes
    // survive any terminal error, even before a retirement request arrives.
    if let Err(ref e) = result {
        w.shared.fail(e);
    }
    let drained = w.drain(&payload); // Only explicit stop/Drop aborts this drain.
    result.and(drained)
}

impl Worker {
    fn serve_pending(&mut self, payload: &[u8]) {
        let mut i = 0;
        while i < self.transfers.len() {
            let transfer = &mut self.transfers[i];
            if transfer.expired(Instant::now()) {
                self.transfers.swap_remove(i);
                continue;
            }
            match transfer.step(payload) {
                Ok(false) => i += 1,
                Ok(true) => self.shared.complete(self.transfers.swap_remove(i)),
                Err(_) => {
                    self.transfers.swap_remove(i);
                } // Never acknowledge a failed reader.
            }
        }
    }

    fn drain(&mut self, payload: &[u8]) -> io::Result<()> {
        // Accepted FDs are independent of the selection and Wayland connection.
        // In particular, cancellation/disconnection must not truncate these pipes.
        while !self
            .shared
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .stop
        {
            self.serve_pending(payload);
            if self.transfers.is_empty() {
                return Ok(());
            }
            let mut fds: Vec<_> = self
                .transfers
                .iter()
                .map(|transfer| libc::pollfd {
                    fd: transfer.fd.as_raw_fd(),
                    events: libc::POLLOUT,
                    revents: 0,
                })
                .collect();
            let left = self
                .transfers
                .iter()
                .map(|transfer| transfer.deadline.saturating_duration_since(Instant::now()))
                .min()
                .unwrap();
            if unsafe {
                libc::poll(
                    fds.as_mut_ptr(),
                    fds.len() as _,
                    left.min(TICK).as_millis() as _,
                )
            } < 0
            {
                let e = io::Error::last_os_error();
                if e.kind() != io::ErrorKind::Interrupted {
                    return Err(e);
                }
            }
        }
        Ok(())
    }

    fn publish(
        &mut self,
        registry: &wl_registry::WlRegistry,
        qh: &QueueHandle<Self>,
    ) -> io::Result<()> {
        let seats: Vec<_> = self
            .globals
            .iter()
            .filter(|(_, interface, _)| interface == "wl_seat")
            .collect();
        if seats.len() != 1 {
            return Err(err(
                io::ErrorKind::Unsupported,
                "exactly one Wayland seat required",
            ));
        }
        let name = seats[0].0;
        self.seat = Some(name);
        let seat: wl_seat::WlSeat = registry.bind(name, 1, qh, ());
        macro_rules! publish {
            ($module:ident, $manager:ident, $global:expr) => {{
                let manager: $module::$manager = registry.bind($global.0, 1, qh, ());
                let source = manager.create_data_source(qh, ());
                for mime in MIMES {
                    source.offer(mime.into());
                }
                let device = manager.get_data_device(&seat, qh, ());
                device.set_selection(Some(&source));
                manager.destroy();
            }};
        }
        if let Some(global) = self
            .globals
            .iter()
            .find(|(_, i, _)| i == "ext_data_control_manager_v1")
        {
            publish!(ext_data_control_manager_v1, ExtDataControlManagerV1, global);
        } else if let Some(global) = self
            .globals
            .iter()
            .find(|(_, i, _)| i == "zwlr_data_control_manager_v1")
        {
            publish!(
                zwlr_data_control_manager_v1,
                ZwlrDataControlManagerV1,
                global
            );
        } else {
            return Err(err(
                io::ErrorKind::Unsupported,
                "no Wayland data-control protocol",
            ));
        }
        self.shared
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .ownership = Ownership::Owned;
        Ok(())
    }
}

impl Dispatch<wl_callback::WlCallback, Barrier> for Worker {
    fn event(
        w: &mut Self,
        _: &wl_callback::WlCallback,
        _: wl_callback::Event,
        barrier: &Barrier,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let mut s = w
            .shared
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match barrier {
            Barrier::Registry => w.discovered = true,
            Barrier::Published => s.ready = true,
            Barrier::Armed => {
                if s.ready && s.ownership == Ownership::Owned {
                    s.armed = true;
                }
            }
        }
        w.shared.changed.notify_all();
    }
}
impl Dispatch<wl_registry::WlRegistry, ()> for Worker {
    fn event(
        w: &mut Self,
        _: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => {
                if interface == "wl_seat" && w.seat.is_some() {
                    w.shared.lose(Ownership::Unavailable);
                }
                if interface == "wl_seat"
                    || interface == "ext_data_control_manager_v1"
                    || interface == "zwlr_data_control_manager_v1"
                {
                    w.globals.push((name, interface, version));
                }
            }
            wl_registry::Event::GlobalRemove { name } => {
                w.globals.retain(|(n, _, _)| *n != name);
                if w.seat == Some(name) {
                    w.shared.lose(Ownership::Unavailable);
                }
            }
            _ => {}
        }
    }
}
wayland_client::delegate_noop!(Worker: ignore wl_seat::WlSeat);
wayland_client::delegate_noop!(Worker: ignore ext_data_control_manager_v1::ExtDataControlManagerV1);
wayland_client::delegate_noop!(Worker: ignore zwlr_data_control_manager_v1::ZwlrDataControlManagerV1);

macro_rules! protocol_dispatch {
    ($dm:ident, $dt:ident, $sm:ident, $st:ident, $om:ident, $ot:ident) => {
        impl Dispatch<$dm::$dt, ()> for Worker {
            fn event(w: &mut Self, device: &$dm::$dt, event: $dm::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
                match event {
                    $dm::Event::Selection { id } | $dm::Event::PrimarySelection { id } => { if let Some(offer) = id { offer.destroy(); } },
                    $dm::Event::Finished => { w.shared.lose(Ownership::Unavailable); device.destroy(); },
                    _ => {},
                }
            }
            wayland_client::event_created_child!(Worker, $dm::$dt, [0 => ($om::$ot, ())]);
        }
        impl Dispatch<$sm::$st, ()> for Worker {
            fn event(w: &mut Self, source: &$sm::$st, event: $sm::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
                match event {
                    $sm::Event::Send { mime_type, fd } => w.shared.enqueue(fd, &mime_type, &mut w.transfers),
                    $sm::Event::Cancelled => { w.shared.lose(Ownership::Replaced); source.destroy(); },
                    _ => {},
                }
            }
        }
        wayland_client::delegate_noop!(Worker: ignore $om::$ot);
    };
}
protocol_dispatch!(
    ext_data_control_device_v1,
    ExtDataControlDeviceV1,
    ext_data_control_source_v1,
    ExtDataControlSourceV1,
    ext_data_control_offer_v1,
    ExtDataControlOfferV1
);
protocol_dispatch!(
    zwlr_data_control_device_v1,
    ZwlrDataControlDeviceV1,
    zwlr_data_control_source_v1,
    ZwlrDataControlSourceV1,
    zwlr_data_control_offer_v1,
    ZwlrDataControlOfferV1
);

// Kept independent of Wayland so tests exercise real pipes without a compositor.
mod transfer {
    use super::Ownership;
    use std::{
        io,
        os::fd::{AsRawFd, OwnedFd},
        sync::{Arc, Condvar, Mutex},
        time::{Duration, Instant},
    };
    pub(super) const MAX_TRANSFERS: usize = 16;

    pub(super) fn freeze_payload(text: &str) -> io::Result<Arc<[u8]>> {
        if text.len() > 1024 * 1024 {
            return Err(err(
                io::ErrorKind::InvalidInput,
                "clipboard payload exceeds 1 MiB",
            ));
        }
        Ok(Arc::from(text.as_bytes()))
    }
    pub(super) fn err(kind: io::ErrorKind, message: &str) -> io::Error {
        io::Error::new(kind, message)
    }
    #[derive(Default)]
    pub(super) struct State {
        pub ready: bool,
        pub ownership: Ownership,
        pub armed: bool,
        pub arm_requested: bool,
        pub served: bool,
        pub retire: bool,
        pub stop: bool,
        pub error: Option<(io::ErrorKind, String)>,
    }
    impl State {
        pub fn ensure_owned(&self) -> io::Result<()> {
            if let Some((kind, message)) = &self.error {
                return Err(err(*kind, message));
            }
            if self.ownership != Ownership::Owned || self.stop {
                return Err(err(io::ErrorKind::BrokenPipe, "clipboard ownership lost"));
            }
            Ok(())
        }
        pub fn check(&self, now: Instant, until: Instant, focused: bool) -> Option<io::Result<()>> {
            if let Err(e) = self.ensure_owned() {
                return Some(Err(e));
            }
            if !self.ready || !self.armed {
                return Some(Err(err(
                    io::ErrorKind::InvalidInput,
                    "clipboard is not armed",
                )));
            }
            if !focused {
                return Some(Err(err(io::ErrorKind::Interrupted, "target focus lost")));
            }
            if self.served {
                return Some(Ok(()));
            }
            if now >= until {
                return Some(Err(err(
                    io::ErrorKind::TimedOut,
                    "no armed text transfer completed",
                )));
            }
            None
        }
    }
    #[derive(Default)]
    pub(super) struct Shared {
        pub state: Mutex<State>,
        pub changed: Condvar,
    }
    impl Shared {
        pub fn lose(&self, ownership: Ownership) {
            self.state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .ownership = ownership;
            self.changed.notify_all();
        }
        pub fn fail(&self, error: &io::Error) {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.ownership = Ownership::Unavailable;
            state.error = Some((error.kind(), error.to_string()));
            self.changed.notify_all();
        }
        pub fn enqueue(&self, fd: OwnedFd, mime: &str, pending: &mut Vec<Transfer>) {
            let s = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if s.ownership != Ownership::Owned
                || s.stop
                || s.retire
                || pending.len() >= MAX_TRANSFERS
                || !matches!(
                    mime,
                    "text/plain;charset=utf-8" | "text/plain" | "UTF8_STRING"
                )
            {
                return;
            }
            if let Ok(t) = Transfer::new(fd, s.armed, Instant::now()) {
                pending.push(t);
            }
        }
        pub fn complete(&self, transfer: Transfer) {
            let eligible = transfer.eligible;
            drop(transfer); // Close BEFORE publishing PayloadServed under the mutex.
            let mut s = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if eligible && s.ownership == Ownership::Owned && !s.stop {
                s.served = true;
                self.changed.notify_all();
            }
        }
    }
    pub(super) struct Transfer {
        pub fd: OwnedFd,
        pub offset: usize,
        pub eligible: bool,
        pub deadline: Instant,
    }
    impl Transfer {
        pub fn new(fd: OwnedFd, eligible: bool, now: Instant) -> io::Result<Self> {
            let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
            if unsafe { libc::fstat(fd.as_raw_fd(), stat.as_mut_ptr()) } < 0 {
                return Err(io::Error::last_os_error());
            }
            // O_NONBLOCK is ineffective on regular files/devices; never write those.
            let kind = unsafe { stat.assume_init() }.st_mode & libc::S_IFMT;
            if kind != libc::S_IFIFO && kind != libc::S_IFSOCK {
                return Err(err(
                    io::ErrorKind::Unsupported,
                    "transfer requires a pipe or socket",
                ));
            }
            let flags = unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_GETFL) };
            if flags < 0
                || unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) }
                    < 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(Self {
                fd,
                offset: 0,
                eligible,
                deadline: now + Duration::from_secs(5),
            })
        }
        pub fn expired(&self, now: Instant) -> bool {
            now >= self.deadline
        }
        pub fn step(&mut self, payload: &[u8]) -> io::Result<bool> {
            if self.offset == payload.len() {
                return Ok(true);
            }
            let bytes = &payload[self.offset..payload.len().min(self.offset + 65536)];
            // fd is owned and O_NONBLOCK, bytes remains valid for this syscall.
            let n = unsafe { libc::write(self.fd.as_raw_fd(), bytes.as_ptr().cast(), bytes.len()) };
            if n < 0 {
                let e = io::Error::last_os_error();
                return if matches!(
                    e.kind(),
                    io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
                ) {
                    Ok(false)
                } else {
                    Err(e)
                };
            }
            if n == 0 {
                return Err(err(
                    io::ErrorKind::WriteZero,
                    "clipboard pipe made no progress",
                ));
            }
            self.offset += n as usize;
            Ok(self.offset == payload.len())
        }
    }
    pub(super) fn block_sigpipe() -> io::Result<()> {
        // A closed reader must yield EPIPE, even if the application changed SIGPIPE.
        // Mask only this dedicated thread; its pending signals die with the thread.
        let mut set = std::mem::MaybeUninit::<libc::sigset_t>::uninit();
        let result = unsafe {
            libc::sigemptyset(set.as_mut_ptr());
            libc::sigaddset(set.as_mut_ptr(), libc::SIGPIPE);
            libc::pthread_sigmask(libc::SIG_BLOCK, set.as_ptr(), std::ptr::null_mut())
        };
        if result == 0 {
            Ok(())
        } else {
            Err(io::Error::from_raw_os_error(result))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{transfer::*, Ownership};
    use std::{
        io::{self, Read},
        os::fd::{FromRawFd, OwnedFd},
        time::{Duration, Instant},
    };

    fn pipe() -> (std::fs::File, OwnedFd) {
        let mut fds = [-1; 2];
        assert_eq!(
            unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_NONBLOCK | libc::O_CLOEXEC) },
            0
        );
        // Force a small capacity so a 64 KiB write really is a short syscall.
        assert!(unsafe { libc::fcntl(fds[1], libc::F_SETPIPE_SZ, 4096) } >= 4096);
        unsafe {
            (
                std::fs::File::from_raw_fd(fds[0]),
                OwnedFd::from_raw_fd(fds[1]),
            )
        }
    }
    fn ready() -> Shared {
        let shared = Shared::default();
        {
            let mut s = shared
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            s.ready = true;
            s.ownership = Ownership::Owned;
        }
        shared
    }

    #[test]
    fn short_stalled_and_repeated_pipe_writes_keep_payload_immutable() {
        let mut original = String::from("immutable text");
        let payload = freeze_payload(&original).unwrap();
        original.clear();
        let large = payload.repeat(32768);
        for _ in 0..2 {
            let (mut reader, fd) = pipe();
            let mut t = Transfer::new(fd, true, Instant::now()).unwrap();
            assert!(!t.step(&large).unwrap());
            assert!(t.offset > 0 && t.offset < large.len());
            let offset = t.offset;
            assert!(!t.step(&large).unwrap());
            assert_eq!(t.offset, offset); // Full pipe, no reader progress.
            let mut received = Vec::new();
            for _ in 0..large.len() {
                let mut buf = [0; 8192];
                match reader.read(&mut buf) {
                    Ok(n) => received.extend_from_slice(&buf[..n]),
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                    result => panic!("unexpected read: {result:?}"),
                }
                if t.step(&large).unwrap() {
                    break;
                }
            }
            drop(t); // Completion is not observable until the writer is closed.
            reader.read_to_end(&mut received).unwrap();
            assert_eq!(received, large);
        }
        assert_eq!(&*payload, b"immutable text");
    }

    #[test]
    fn payload_limit_is_in_bytes_and_inclusive() {
        let exact = "é".repeat(512 * 1024);
        assert_eq!(freeze_payload(&exact).unwrap().len(), 1024 * 1024);
        assert_eq!(
            freeze_payload(&(exact + "x")).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }

    #[test]
    fn broken_reader_is_not_a_success() {
        block_sigpipe().unwrap();
        let (reader, fd) = pipe();
        drop(reader);
        let mut t = Transfer::new(fd, true, Instant::now()).unwrap();
        assert_eq!(
            t.step(b"text").unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
    }

    #[test]
    fn pre_arm_request_finishing_after_arm_does_not_satisfy_wait() {
        let shared = ready();
        let (mut reader, fd) = pipe();
        let mut pending = Vec::new();
        shared.enqueue(fd, "text/plain", &mut pending);
        let mut t = pending.pop().unwrap();
        assert!(!t.eligible);
        shared
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .armed = true;
        assert!(t.step(b"pre-arm").unwrap());
        shared.complete(t);
        let mut received = Vec::new();
        reader.read_to_end(&mut received).unwrap();
        assert_eq!(received, b"pre-arm");
        assert!(
            !shared
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .served
        );
    }

    #[test]
    fn only_armed_text_full_write_and_close_counts() {
        let shared = ready();
        shared
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .armed = true;
        let (mut reader, fd) = pipe();
        let mut pending = Vec::new();
        shared.enqueue(fd, "image/png", &mut pending);
        assert!(pending.is_empty());
        assert_eq!(reader.read(&mut [0; 1]).unwrap(), 0);
        for mime in ["UTF8_STRING", "text/plain;charset=utf-8", "text/plain"] {
            shared
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .served = false;
            let (mut reader, fd) = pipe();
            shared.enqueue(fd, mime, &mut pending);
            let mut t = pending.pop().unwrap();
            assert!(t.eligible && t.step(b"text").unwrap());
            let mut bytes = [0; 4];
            reader.read_exact(&mut bytes).unwrap();
            assert_eq!(&bytes, b"text");
            assert_eq!(
                reader.read(&mut [0; 1]).unwrap_err().kind(),
                io::ErrorKind::WouldBlock
            );
            assert!(
                !shared
                    .state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .served
            );
            shared.complete(t);
            assert_eq!(reader.read(&mut [0; 1]).unwrap(), 0);
            assert!(
                shared
                    .state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .served
            );
        }
    }

    #[test]
    fn deadline_focus_and_cancellation_are_deterministic_and_non_destructive() {
        let shared = ready();
        let now = Instant::now();
        let mut s = shared
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!(
            s.check(now, now, true).unwrap().unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        s.armed = true;
        assert!(s.check(now, now + Duration::from_secs(1), true).is_none());
        assert_eq!(
            s.check(now, now, true).unwrap().unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
        assert!(s.ownership == Ownership::Owned && s.armed && !s.stop);
        assert_eq!(
            s.check(now, now, false).unwrap().unwrap_err().kind(),
            io::ErrorKind::Interrupted
        );
        s.served = true;
        drop(s);
        shared.lose(Ownership::Replaced);
        let s = shared
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!(
            s.check(now, now, true).unwrap().unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
    }

    #[test]
    fn timeout_keeps_service_alive_but_cancellation_prevents_late_ack() {
        let shared = ready();
        shared
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .armed = true;
        let now = Instant::now();
        assert_eq!(
            shared
                .state
                .lock()
                .unwrap()
                .check(now, now, true)
                .unwrap()
                .unwrap_err()
                .kind(),
            io::ErrorKind::TimedOut
        );
        let mut pending = Vec::new();
        let (mut reader, fd) = pipe();
        shared.enqueue(fd, "text/plain", &mut pending);
        let mut t = pending.pop().unwrap();
        assert!(t.step(b"still available").unwrap());
        shared.complete(t);
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"still available");
        assert!(shared
            .state
            .lock()
            .unwrap()
            .check(now, now, true)
            .unwrap()
            .is_ok());
        shared
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .served = false;
        let (_reader, fd) = pipe();
        shared.enqueue(fd, "text/plain", &mut pending);
        let mut t = pending.pop().unwrap();
        shared.lose(Ownership::Replaced);
        assert!(t.step(b"late").unwrap());
        shared.complete(t);
        assert!(
            !shared
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .served
        );
    }

    #[test]
    fn empty_payload_completes_only_when_fd_is_closed() {
        let shared = ready();
        shared
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .armed = true;
        let (mut reader, fd) = pipe();
        let mut pending = Vec::new();
        shared.enqueue(fd, "text/plain", &mut pending);
        let mut t = pending.pop().unwrap();
        assert!(t.step(b"").unwrap());
        assert_eq!(
            reader.read(&mut [0; 1]).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        shared.complete(t);
        assert_eq!(reader.read(&mut [0; 1]).unwrap(), 0);
        assert!(
            shared
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .served
        );
    }

    #[test]
    fn retirement_rejects_new_sends_without_changing_accepted_deadlines() {
        let shared = ready();
        let (_reader, fd) = pipe();
        let mut pending = Vec::new();
        shared.enqueue(fd, "text/plain", &mut pending);
        let deadline = pending[0].deadline;
        shared
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .retire = true;
        let (mut reader, fd) = pipe();
        shared.enqueue(fd, "text/plain", &mut pending);
        assert_eq!(reader.read(&mut [0; 1]).unwrap(), 0);
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].deadline, deadline);
        assert!(
            !shared
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .stop
        );
    }

    #[test]
    fn outstanding_limit_and_expiry_do_not_acknowledge() {
        let shared = ready();
        shared
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .armed = true;
        let mut pending = Vec::new();
        let mut readers = Vec::new();
        for _ in 0..MAX_TRANSFERS + 1 {
            let (r, fd) = pipe();
            readers.push(r);
            shared.enqueue(fd, "text/plain", &mut pending);
        }
        assert_eq!(pending.len(), MAX_TRANSFERS);
        assert!(pending[0].expired(pending[0].deadline));
        drop(pending);
        assert!(
            !shared
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .served
        );
        assert_eq!(
            shared
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .ownership,
            Ownership::Owned
        );
    }
}

#[cfg(test)]
mod wire_tests {
    use super::*;
    use std::{
        fs::File,
        io::{Read, Write},
    };

    const BOUND: Duration = Duration::from_secs(3);

    // Deliberately only the wire subset used by this source: no socket pathname,
    // environment, registry library, desktop connection, or compositor thread.
    struct Peer {
        socket: UnixStream,
        registry: u32,
        source: u32,
        device: u32,
        publication: u32,
    }
    fn uint(bytes: &[u8]) -> u32 {
        u32::from_ne_bytes(bytes[..4].try_into().unwrap())
    }
    fn string(value: &str) -> Vec<u8> {
        let mut bytes = ((value.len() + 1) as u32).to_ne_bytes().to_vec();
        bytes.extend_from_slice(value.as_bytes());
        bytes.push(0);
        bytes.resize((bytes.len() + 3) & !3, 0);
        bytes
    }
    impl Peer {
        fn request(&mut self, object: u32, opcode: u16) -> Vec<u8> {
            let mut header = [0; 8];
            self.socket.read_exact(&mut header).unwrap();
            assert_eq!(uint(&header), object);
            let word = uint(&header[4..]);
            assert_eq!(word as u16, opcode);
            let size = (word >> 16) as usize;
            assert!((8..=4096).contains(&size) && size.is_multiple_of(4));
            let mut args = vec![0; size - 8];
            self.socket.read_exact(&mut args).unwrap();
            args
        }
        fn event(&mut self, object: u32, opcode: u16, args: &[u8], fd: Option<OwnedFd>) {
            let mut bytes = object.to_ne_bytes().to_vec();
            bytes.extend_from_slice(
                &((((args.len() + 8) as u32) << 16) | u32::from(opcode)).to_ne_bytes(),
            );
            bytes.extend_from_slice(args);
            let mut iov = libc::iovec {
                iov_base: bytes.as_mut_ptr().cast(),
                iov_len: bytes.len(),
            };
            // usize storage supplies cmsghdr alignment; CMSG_SPACE includes padding.
            let mut control = [0usize; 8];
            let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
            msg.msg_iov = &mut iov;
            msg.msg_iovlen = 1;
            if let Some(fd) = &fd {
                msg.msg_control = control.as_mut_ptr().cast();
                msg.msg_controllen =
                    unsafe { libc::CMSG_SPACE(std::mem::size_of::<libc::c_int>() as _) } as usize;
                unsafe {
                    let header = libc::CMSG_FIRSTHDR(&msg);
                    (*header).cmsg_level = libc::SOL_SOCKET;
                    (*header).cmsg_type = libc::SCM_RIGHTS;
                    (*header).cmsg_len =
                        libc::CMSG_LEN(std::mem::size_of::<libc::c_int>() as _) as usize;
                    libc::CMSG_DATA(header)
                        .cast::<libc::c_int>()
                        .write(fd.as_raw_fd());
                }
            }
            let n = loop {
                let n = unsafe { libc::sendmsg(self.socket.as_raw_fd(), &msg, libc::MSG_NOSIGNAL) };
                if n >= 0 {
                    break n as usize;
                }
                let e = io::Error::last_os_error();
                assert_eq!(e.kind(), io::ErrorKind::Interrupted, "sendmsg: {e}");
            };
            assert!(n > 0);
            self.socket.write_all(&bytes[n..]).unwrap();
            // Close our copy of the writer: only the client can now produce EOF.
        }
        fn done(&mut self, callback: u32) {
            self.event(callback, 0, &1u32.to_ne_bytes(), None);
            self.event(1, 1, &callback.to_ne_bytes(), None); // wl_display.delete_id
        }
        fn global(&mut self, registry: u32, name: u32, interface: &str) {
            let mut args = name.to_ne_bytes().to_vec();
            args.extend(string(interface));
            args.extend(1u32.to_ne_bytes());
            self.event(registry, 0, &args, None);
        }
        fn discovery(&mut self, seats: u32, ext: bool) -> u32 {
            let registry = uint(&self.request(1, 1)); // get_registry
            self.registry = registry;
            let sync = uint(&self.request(1, 0));
            for seat in 0..seats {
                self.global(registry, 10 + seat, "wl_seat");
            }
            self.global(registry, 20, "zwlr_data_control_manager_v1");
            if ext {
                self.global(registry, 21, "ext_data_control_manager_v1");
            }
            self.done(sync);
            registry
        }
        fn publication(&mut self, registry: u32, ext: bool) {
            fn binding(args: &[u8], name: u32, interface: &str) -> u32 {
                let mut expected = name.to_ne_bytes().to_vec();
                expected.extend(string(interface));
                expected.extend(1u32.to_ne_bytes());
                assert_eq!(&args[..args.len() - 4], &expected);
                uint(&args[args.len() - 4..])
            }
            let seat = binding(&self.request(registry, 0), 10, "wl_seat");
            let (name, interface) = if ext {
                (21, "ext_data_control_manager_v1")
            } else {
                (20, "zwlr_data_control_manager_v1")
            };
            let manager = binding(&self.request(registry, 0), name, interface);
            self.source = uint(&self.request(manager, 0));
            for mime in MIMES {
                assert_eq!(self.request(self.source, 0), string(mime));
            }
            let args = self.request(manager, 1);
            let device = uint(&args);
            self.device = device;
            assert_eq!(uint(&args[4..]), seat);
            self.event(device, 1, &0u32.to_ne_bytes(), None); // initial null selection
            assert_eq!(self.request(device, 0), self.source.to_ne_bytes());
            assert!(self.request(manager, 2).is_empty());
            self.publication = uint(&self.request(1, 0));
        }
        fn send(&mut self, stalled: bool) -> File {
            let mut fds = [-1; 2];
            assert_eq!(
                unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_NONBLOCK | libc::O_CLOEXEC) },
                0
            );
            let (reader, writer) =
                unsafe { (File::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
            assert_eq!(
                unsafe { libc::fcntl(writer.as_raw_fd(), libc::F_SETPIPE_SZ, 4096) },
                4096
            );
            if stalled {
                let filler = [b'x'; 4096];
                assert_eq!(
                    unsafe {
                        libc::write(writer.as_raw_fd(), filler.as_ptr().cast(), filler.len())
                    },
                    4096
                );
            }
            self.event(self.source, 0, &string(MIMES[0]), Some(writer));
            reader
        }
    }
    fn start(text: &str) -> (ClipboardOffer, Peer) {
        let (client, server) = UnixStream::pair().unwrap();
        client.set_nonblocking(true).unwrap();
        server.set_read_timeout(Some(BOUND)).unwrap();
        server.set_write_timeout(Some(BOUND)).unwrap();
        let offer = ClipboardOffer::start(text, deadline(BOUND).unwrap(), move || {
            Connection::from_socket(client).map_err(other)
        })
        .unwrap();
        (
            offer,
            Peer {
                socket: server,
                registry: 0,
                source: 0,
                device: 0,
                publication: 0,
            },
        )
    }
    fn published(text: &str, ext: bool) -> (ClipboardOffer, Peer) {
        let (offer, mut peer) = start(text);
        let registry = peer.discovery(1, ext);
        peer.publication(registry, ext);
        // Seeing set_selection is not sufficient: publication requires sync.done.
        assert!(
            !offer
                .shared
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .ready
        );
        assert!(!offer.owns_selection());
        peer.done(peer.publication);
        offer
            .await_control(deadline(BOUND).unwrap(), |s| s.ready)
            .unwrap();
        assert!(offer.owns_selection());
        assert_eq!(offer.ownership(), Ownership::Owned);
        (offer, peer)
    }
    fn arm(offer: &ClipboardOffer, peer: &mut Peer, before_done: impl FnOnce(&mut Peer)) {
        thread::scope(|scope| {
            let result = scope.spawn(|| offer.arm());
            let callback = uint(&peer.request(1, 0));
            assert!(
                !offer
                    .shared
                    .state
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .armed
            );
            before_done(peer);
            peer.done(callback);
            result.join().unwrap().unwrap();
        });
    }
    fn read_to_eof(mut reader: File) -> Vec<u8> {
        let until = deadline(BOUND).unwrap();
        let mut bytes = Vec::new();
        loop {
            let mut buf = [0; 8192];
            match reader.read(&mut buf) {
                Ok(0) => return bytes,
                Ok(n) => bytes.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    let left = until.saturating_duration_since(Instant::now());
                    assert!(!left.is_zero(), "worker did not close transfer FD");
                    let mut poll = libc::pollfd {
                        fd: reader.as_raw_fd(),
                        events: libc::POLLIN,
                        revents: 0,
                    };
                    assert!(unsafe { libc::poll(&mut poll, 1, left.as_millis().max(1) as _) } > 0);
                }
                Err(e) => panic!("pipe read: {e}"),
            }
        }
    }
    fn no_transfer(offer: &ClipboardOffer) {
        assert_eq!(
            offer
                .wait_for_transfer(Duration::from_millis(25), || true)
                .unwrap_err()
                .kind(),
            io::ErrorKind::TimedOut
        );
        assert!(offer.owns_selection());
    }

    #[test]
    fn wire_ext_preferred_and_wlr_fallback_serve_delayed_immutable_payload_after_timeout() {
        for ext in [true, false] {
            let mut text = String::from("tự");
            let (offer, mut peer) = published(&text, ext);
            text.clear();
            arm(&offer, &mut peer, |_| {});
            no_transfer(&offer); // No self-read or automatic withdrawal.
            let mut reader = peer.send(true);
            no_transfer(&offer); // Even a Send is not enough while writes stall.
            let mut filler = [0; 4096];
            reader.read_exact(&mut filler).unwrap();
            assert_eq!(filler, [b'x'; 4096]);
            offer.wait_for_transfer(BOUND, || true).unwrap();
            // This pipe is nonblocking: WouldBlock here would prove that wait
            // acknowledged before the payload was available and the FD closed.
            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes).unwrap();
            assert_eq!(bytes, "tự".as_bytes());
            assert!(offer.owns_selection());
            drop(offer); // Exercises bounded stop/join with the fake still connected.
            let mut byte = [0];
            assert_eq!(peer.socket.read(&mut byte).unwrap(), 0);
        }
    }

    #[test]
    fn wire_prearm_send_finishing_after_arm_never_counts() {
        let (offer, mut peer) = published("tự", true);
        let mut reader = None;
        arm(&offer, &mut peer, |peer| reader = Some(peer.send(true)));
        let mut reader = reader.unwrap();
        let mut filler = [0; 4096];
        reader.read_exact(&mut filler).unwrap();
        assert_eq!(read_to_eof(reader), "tự".as_bytes());
        no_transfer(&offer);
        let reader = peer.send(false);
        offer.wait_for_transfer(BOUND, || true).unwrap();
        assert_eq!(read_to_eof(reader), "tự".as_bytes());
    }

    #[test]
    fn wire_retire_after_success_and_restore_drains_another_accepted_stalled_request() {
        let text = "tự".repeat(16384); // Far larger than either transfer pipe.
        let (offer, mut peer) = published(&text, true);
        arm(&offer, &mut peer, |_| {});
        let mut late = peer.send(true);
        let eager = peer.send(false);
        thread::scope(|scope| {
            let eager = scope.spawn(|| read_to_eof(eager));
            offer.wait_for_transfer(BOUND, || true).unwrap();
            assert_eq!(eager.join().unwrap(), text.as_bytes());
            // A restore by the caller replaces this source: model the server's
            // resulting cancellation, without touching any actual clipboard.
            peer.event(peer.source, 1, &[], None);
            assert!(peer.request(peer.source, 1).is_empty());
            assert!(!offer.owns_selection());
            let shared = offer.shared.clone();
            shared
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .served = false;
            let retiring = scope.spawn(move || offer.retire());
            let until = deadline(BOUND).unwrap();
            let mut state = shared
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            while !state.retire {
                let left = until.saturating_duration_since(Instant::now());
                assert!(!left.is_zero(), "retirement did not start");
                state = shared.changed.wait_timeout(state, left).unwrap().0;
            }
            assert!(!state.stop);
            drop(state);
            assert!(!retiring.is_finished(), "stalled transfer was aborted");
            let mut filler = [0; 4096];
            late.read_exact(&mut filler).unwrap();
            assert_eq!(filler, [b'x'; 4096]);
            assert_eq!(read_to_eof(late), text.as_bytes());
            retiring.join().unwrap();
            let state = shared
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            assert!(!state.served, "cancelled transfer must not acknowledge");
            assert_eq!(state.ownership, Ownership::Replaced);
            assert!(state.error.is_none());
        });
    }

    #[test]
    fn wire_drop_still_aborts_an_accepted_stalled_request() {
        let (offer, mut peer) = published(&"tự".repeat(16384), true);
        let mut reader = None;
        // The arm callback follows Send, proving the stalled FD was accepted.
        arm(&offer, &mut peer, |peer| reader = Some(peer.send(true)));
        drop(offer);
        assert_eq!(read_to_eof(reader.unwrap()), vec![b'x'; 4096]);
    }

    #[test]
    fn retiring_worker_expires_accepted_fd_at_its_original_deadline() {
        let shared = Arc::new(Shared::default());
        shared
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .retire = true;
        let (mut reader, writer) = UnixStream::pair().unwrap();
        reader.set_read_timeout(Some(BOUND)).unwrap();
        let mut transfer = Transfer::new(writer.into(), true, Instant::now()).unwrap();
        transfer.deadline = Instant::now(); // Expired already: no clock-based sleep.
        let mut worker = Worker {
            shared: shared.clone(),
            globals: Vec::new(),
            discovered: true,
            seat: None,
            transfers: vec![transfer],
        };
        worker.drain(b"must not extend the deadline").unwrap();
        assert_eq!(reader.read(&mut [0; 1]).unwrap(), 0);
        assert!(worker.transfers.is_empty());
        assert!(
            !shared
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .served
        );
    }

    fn wait_until_finished(offer: &ClipboardOffer) {
        let until = deadline(BOUND).unwrap();
        while !offer.is_finished() {
            assert!(Instant::now() < until, "worker did not finish draining");
            thread::yield_now();
        }
    }

    #[test]
    fn wire_disconnect_before_retirement_reports_unavailable_but_drains_accepted_payload() {
        let text = "tự".repeat(16384);
        let (offer, mut peer) = published(&text, true);
        arm(&offer, &mut peer, |_| {});
        let mut reader = peer.send(false);
        let mut poll = libc::pollfd {
            fd: reader.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        assert!(unsafe { libc::poll(&mut poll, 1, BOUND.as_millis() as _) } > 0);
        let mut bytes = vec![0; 4096];
        reader.read_exact(&mut bytes).unwrap(); // Proves a post-arm request was accepted.
        assert_eq!(bytes, &text.as_bytes()[..4096]);
        drop(peer); // No request_retire yet; the remaining large transfer is stalled.
        assert_eq!(
            offer.wait_for_transfer(BOUND, || true).unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
        assert_eq!(offer.ownership(), Ownership::Unavailable);
        assert!(!offer.owns_selection());
        assert!(
            !offer.is_finished(),
            "transport failure aborted the accepted pipe"
        );
        {
            let state = offer
                .shared
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            assert!(state.error.is_some());
            assert!(!state.retire && !state.stop && !state.served);
        }
        offer.request_retire();
        bytes.extend(read_to_eof(reader));
        assert_eq!(bytes, text.as_bytes());
        wait_until_finished(&offer);
        assert_eq!(offer.ownership(), Ownership::Unavailable);
        assert!(
            !offer
                .shared
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .served
        );
        assert!(offer.wait_for_transfer(Duration::ZERO, || true).is_err());
        offer.retire(); // Join only after is_finished: no outstanding read is aborted.
    }

    #[test]
    fn wire_request_retire_returns_while_accepted_transfer_is_stalled() {
        let text = "tự".repeat(16384);
        let (offer, mut peer) = published(&text, true);
        let mut reader = None;
        arm(&offer, &mut peer, |peer| reader = Some(peer.send(true)));
        thread::scope(|scope| {
            let (sent, returned) = std::sync::mpsc::channel();
            let offer = &offer;
            let request = scope.spawn(move || {
                offer.request_retire();
                sent.send(()).unwrap();
            });
            returned
                .recv_timeout(BOUND)
                .expect("request_retire joined the stalled worker");
            request.join().unwrap();
        });
        assert!(!offer.is_finished());
        assert!(
            !offer
                .shared
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .stop
        );
        let mut reader = reader.unwrap();
        let mut filler = [0; 4096];
        reader.read_exact(&mut filler).unwrap();
        assert_eq!(filler, [b'x'; 4096]);
        assert_eq!(read_to_eof(reader), text.as_bytes());
        wait_until_finished(&offer);
        offer.retire();
    }

    #[test]
    fn wire_seat_removal_and_device_finished_are_unavailable_not_replaced() {
        for ext in [true, false] {
            for finished in [false, true] {
                let (offer, mut peer) = published("tự", ext);
                arm(&offer, &mut peer, |_| {});
                if finished {
                    peer.event(peer.device, 2, &[], None);
                } else {
                    peer.event(peer.registry, 1, &10u32.to_ne_bytes(), None);
                }
                assert_eq!(
                    offer.wait_for_transfer(BOUND, || true).unwrap_err().kind(),
                    io::ErrorKind::BrokenPipe
                );
                assert_eq!(offer.ownership(), Ownership::Unavailable);
                offer.request_retire();
                wait_until_finished(&offer);
                assert_eq!(offer.ownership(), Ownership::Unavailable);
                offer.retire();
            }
        }
    }

    #[test]
    fn wire_source_cancelled_revokes_ownership_and_wait() {
        let (offer, mut peer) = published("tự", true);
        arm(&offer, &mut peer, |_| {});
        peer.event(peer.source, 1, &[], None);
        assert_eq!(
            offer.wait_for_transfer(BOUND, || true).unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
        assert!(!offer.owns_selection());
        assert_eq!(offer.ownership(), Ownership::Replaced);
        assert_eq!(offer.arm().unwrap_err().kind(), io::ErrorKind::BrokenPipe);
        assert!(peer.request(peer.source, 1).is_empty()); // source.destroy, not disconnect
        offer.request_retire();
        wait_until_finished(&offer);
        assert_eq!(offer.ownership(), Ownership::Replaced);
        offer.retire();
    }

    #[test]
    fn wire_registry_rejects_zero_or_multiple_seats_without_publishing() {
        for seats in [0, 2] {
            let (offer, mut peer) = start("tự");
            peer.discovery(seats, true);
            assert_eq!(
                offer
                    .await_control(deadline(BOUND).unwrap(), |s| s.ready)
                    .unwrap_err()
                    .kind(),
                io::ErrorKind::Unsupported
            );
            assert!(!offer.owns_selection());
            let mut byte = [0];
            assert_eq!(peer.socket.read(&mut byte).unwrap(), 0);
        }
    }
}
