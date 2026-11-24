// SPDX-License-Identifier: MIT
//! Native Wayland multi-MIME clipboard restoration via `ext_data_control_manager_v1`
//! or `zwlr_data_control_manager_v1`.
//!
//! Unlike `wl-copy` (which only accepts a single `--type` per invocation and drops
//! secondary formats such as `text/plain` alongside `text/html`), this module offers
//! all backed-up `(mime, bytes)` pairs on a single Wayland data source and serves
//! each requested MIME until the user copies something new (`Cancelled`).

use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

use wayland_client::backend::WaylandError;
use wayland_client::protocol::{wl_callback, wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};
use wayland_protocols::ext::data_control::v1::client::{
    ext_data_control_device_v1, ext_data_control_manager_v1, ext_data_control_offer_v1,
    ext_data_control_source_v1,
};
use wayland_protocols_wlr::data_control::v1::client::{
    zwlr_data_control_device_v1, zwlr_data_control_manager_v1, zwlr_data_control_offer_v1,
    zwlr_data_control_source_v1,
};

const PIPE_TRANSFER_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_CONCURRENT_SEND_WORKERS: usize = 8;
static ACTIVE_RESTORE_GEN: AtomicU64 = AtomicU64::new(0);

struct SendWorkerGuard(Arc<AtomicUsize>);

impl Drop for SendWorkerGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

struct RestoreState {
    globals: Vec<(u32, String)>,
    entries: Arc<Vec<(String, Vec<u8>)>>,
    active_sends: Arc<AtomicUsize>,
    cancelled: bool,
    sync_done: bool,
}

impl Dispatch<wl_callback::WlCallback, ()> for RestoreState {
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

fn roundtrip_with_deadline(
    connection: &Connection,
    queue: &mut EventQueue<RestoreState>,
    state: &mut RestoreState,
    deadline: Instant,
    aborted: &AtomicBool,
) -> io::Result<()> {
    state.sync_done = false;
    let qh = queue.handle();
    let _cb = connection.display().sync(&qh, ());
    while !state.sync_done {
        if aborted.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "clipboard restore aborted",
            ));
        }
        queue.flush().map_err(io::Error::other)?;
        queue.dispatch_pending(state).map_err(io::Error::other)?;
        if state.sync_done {
            return Ok(());
        }
        let now = Instant::now();
        if now >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "clipboard restore roundtrip timed out",
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
                    "clipboard restore roundtrip timed out",
                ));
            }
            if (pfd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL)) != 0 {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "Wayland compositor disconnected during restore",
                ));
            }
            if let Err(WaylandError::Io(e)) = guard.read() {
                if !matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                ) {
                    return Err(e);
                }
            }
        }
    }
    Ok(())
}

impl RestoreState {
    fn payload_index_for(&self, mime: &str) -> Option<usize> {
        self.entries.iter().position(|(m, _)| m == mime)
    }

    fn spawn_send_worker(&self, mime: &str, fd: OwnedFd) {
        let Some(idx) = self.payload_index_for(mime) else {
            return;
        };
        // Bound concurrent Send workers so a stalled reader (e.g. clipboard manager
        // reading `image/png`) never blocks concurrent MIME transfers or `Cancelled`,
        // while also capping thread count under adversarial `Send` floods.
        let prev = self.active_sends.fetch_add(1, Ordering::AcqRel);
        if prev >= MAX_CONCURRENT_SEND_WORKERS {
            self.active_sends.fetch_sub(1, Ordering::AcqRel);
            return;
        }
        let entries = Arc::clone(&self.entries);
        let guard = SendWorkerGuard(Arc::clone(&self.active_sends));
        let spawned = thread::Builder::new()
            .name("dboard-clip-send".into())
            .spawn(move || {
                let _guard = guard;
                block_sigpipe_on_thread();
                write_pipe_nonblocking(fd, &entries[idx].1);
            });
        if spawned.is_err() {
            // `guard` dropped here if spawn failed (already moved into closure only on Ok,
            // Wait: closure moved `guard`, so `Err` drops closure + `guard`, decrementing count).
        }
    }
}

fn block_sigpipe_on_thread() {
    let mut set = std::mem::MaybeUninit::<libc::sigset_t>::uninit();
    unsafe {
        libc::sigemptyset(set.as_mut_ptr());
        libc::sigaddset(set.as_mut_ptr(), libc::SIGPIPE);
        let _ = libc::pthread_sigmask(libc::SIG_BLOCK, set.as_ptr(), std::ptr::null_mut());
    }
}

fn write_pipe_nonblocking(fd: OwnedFd, bytes: &[u8]) {
    write_pipe_nonblocking_with_timeout(fd, bytes, PIPE_TRANSFER_TIMEOUT);
}

fn write_pipe_nonblocking_with_timeout(fd: OwnedFd, bytes: &[u8], timeout: Duration) -> bool {
    let raw_fd = fd.as_raw_fd();
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    if unsafe { libc::fstat(raw_fd, stat.as_mut_ptr()) } < 0 {
        return false;
    }
    let kind = unsafe { stat.assume_init() }.st_mode & libc::S_IFMT;
    if kind != libc::S_IFIFO && kind != libc::S_IFSOCK {
        return false;
    }
    let flags = unsafe { libc::fcntl(raw_fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(raw_fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return false;
    }

    let deadline = Instant::now() + timeout;
    let mut offset = 0usize;
    while offset < bytes.len() {
        let chunk = &bytes[offset..bytes.len().min(offset + 65536)];
        let n = unsafe { libc::write(raw_fd, chunk.as_ptr().cast(), chunk.len()) };
        if n > 0 {
            offset += n as usize;
            continue;
        }
        if n == 0 {
            return false;
        }
        let err = io::Error::last_os_error();
        if err.kind() == io::ErrorKind::Interrupted {
            continue;
        }
        if err.kind() != io::ErrorKind::WouldBlock {
            return false;
        }
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return false;
        };
        let wait_ms = remaining.as_millis().clamp(1, 200) as libc::c_int;
        let mut pfd = libc::pollfd {
            fd: raw_fd,
            events: libc::POLLOUT,
            revents: 0,
        };
        let rc = unsafe { libc::poll(&mut pfd, 1, wait_ms) };
        if rc < 0 {
            if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return false;
        }
        if (pfd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL)) != 0 {
            return false;
        }
    }
    true
}

impl Dispatch<wl_registry::WlRegistry, ()> for RestoreState {
    fn event(
        state: &mut Self,
        _: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name, interface, ..
        } = event
        {
            if matches!(
                interface.as_str(),
                "wl_seat" | "ext_data_control_manager_v1" | "zwlr_data_control_manager_v1"
            ) {
                state.globals.push((name, interface));
            }
        }
    }
}

wayland_client::delegate_noop!(RestoreState: ignore wl_seat::WlSeat);
wayland_client::delegate_noop!(RestoreState: ignore ext_data_control_manager_v1::ExtDataControlManagerV1);
wayland_client::delegate_noop!(RestoreState: ignore zwlr_data_control_manager_v1::ZwlrDataControlManagerV1);

macro_rules! impl_restore_dispatch {
    ($dm:ident, $dt:ident, $sm:ident, $st:ident, $om:ident, $ot:ident) => {
        impl Dispatch<$dm::$dt, ()> for RestoreState {
            fn event(
                _: &mut Self,
                _: &$dm::$dt,
                event: $dm::Event,
                _: &(),
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
                if let $dm::Event::DataOffer { id } = event {
                    id.destroy();
                }
            }
            wayland_client::event_created_child!(RestoreState, $dm::$dt, [0 => ($om::$ot, ())]);
        }
        impl Dispatch<$sm::$st, ()> for RestoreState {
            fn event(
                state: &mut Self,
                source: &$sm::$st,
                event: $sm::Event,
                _: &(),
                _: &Connection,
                _: &QueueHandle<Self>,
            ) {
                match event {
                    $sm::Event::Send { mime_type, fd } => {
                        state.spawn_send_worker(&mime_type, fd);
                    }
                    $sm::Event::Cancelled => {
                        state.cancelled = true;
                        source.destroy();
                    }
                    _ => {}
                }
            }
        }
        wayland_client::delegate_noop!(RestoreState: ignore $om::$ot);
    };
}

impl_restore_dispatch!(
    ext_data_control_device_v1,
    ExtDataControlDeviceV1,
    ext_data_control_source_v1,
    ExtDataControlSourceV1,
    ext_data_control_offer_v1,
    ExtDataControlOfferV1
);
impl_restore_dispatch!(
    zwlr_data_control_device_v1,
    ZwlrDataControlDeviceV1,
    zwlr_data_control_source_v1,
    ZwlrDataControlSourceV1,
    zwlr_data_control_offer_v1,
    ZwlrDataControlOfferV1
);

fn connect_wayland_display() -> io::Result<Connection> {
    let stream = crate::wayland_vk_connect::connect_wayland_socket()?;
    Connection::from_socket(stream).map_err(io::Error::other)
}

fn serve_multi_mime_connection(
    connection: Connection,
    entries: Vec<(String, Vec<u8>)>,
    ready_tx: mpsc::SyncSender<io::Result<()>>,
    deadline: Instant,
    aborted: Arc<AtomicBool>,
    generation: u64,
) {
    block_sigpipe_on_thread();
    let mut queue = connection.new_event_queue::<RestoreState>();
    let qh = queue.handle();
    let display = connection.display();
    let registry = display.get_registry(&qh, ());
    let mut state = RestoreState {
        globals: Vec::new(),
        entries: Arc::new(entries),
        active_sends: Arc::new(AtomicUsize::new(0)),
        cancelled: false,
        sync_done: false,
    };

    if let Err(err) =
        roundtrip_with_deadline(&connection, &mut queue, &mut state, deadline, &aborted)
    {
        let _ = ready_tx.send(Err(err));
        return;
    }

    // Never publish a stale clipboard selection if the caller already timed out
    // (or fell back to `wl-copy`) or if a newer restore generation superseded us.
    if aborted.load(Ordering::Acquire)
        || Instant::now() >= deadline
        || ACTIVE_RESTORE_GEN.load(Ordering::Acquire) != generation
    {
        return;
    }

    let seats: Vec<u32> = state
        .globals
        .iter()
        .filter_map(|(name, iface)| (iface == "wl_seat").then_some(*name))
        .collect();
    if seats.len() != 1 {
        let _ = ready_tx.send(Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "exactly one Wayland seat required",
        )));
        return;
    }
    let seat: wl_seat::WlSeat = registry.bind(seats[0], 1, &qh, ());

    macro_rules! publish_all {
        ($module:ident, $manager:ident, $name:expr) => {{
            let manager: $module::$manager = registry.bind($name, 1, &qh, ());
            let source = manager.create_data_source(&qh, ());
            for (mime, _) in state.entries.iter() {
                source.offer(mime.clone());
            }
            let device = manager.get_data_device(&seat, &qh, ());
            device.set_selection(Some(&source));
            manager.destroy();
        }};
    }

    if let Some((name, _)) = state
        .globals
        .iter()
        .find(|(_, i)| i == "ext_data_control_manager_v1")
    {
        publish_all!(ext_data_control_manager_v1, ExtDataControlManagerV1, *name);
    } else if let Some((name, _)) = state
        .globals
        .iter()
        .find(|(_, i)| i == "zwlr_data_control_manager_v1")
    {
        publish_all!(
            zwlr_data_control_manager_v1,
            ZwlrDataControlManagerV1,
            *name
        );
    } else {
        let _ = ready_tx.send(Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "no Wayland data-control protocol",
        )));
        return;
    }

    if let Err(err) =
        roundtrip_with_deadline(&connection, &mut queue, &mut state, deadline, &aborted)
    {
        let _ = ready_tx.send(Err(err));
        return;
    }
    if aborted.load(Ordering::Acquire) {
        return;
    }
    let _ = ready_tx.send(Ok(()));

    let max_lifetime = Instant::now() + Duration::from_secs(300);
    while !state.cancelled
        && !aborted.load(Ordering::Acquire)
        && ACTIVE_RESTORE_GEN.load(Ordering::Acquire) == generation
        && Instant::now() < max_lifetime
    {
        let _ = queue.flush();
        let Some(guard) = queue.prepare_read() else {
            if queue.dispatch_pending(&mut state).is_err() {
                break;
            }
            continue;
        };
        let mut pfd = libc::pollfd {
            fd: guard.connection_fd().as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let rc = unsafe { libc::poll(&mut pfd, 1, 200) };
        if rc > 0 {
            if (pfd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL)) != 0 {
                break;
            }
            match guard.read() {
                Ok(_) => {}
                Err(WaylandError::Io(ref e))
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) =>
                {
                    continue;
                }
                Err(_) => break,
            }
            if queue.dispatch_pending(&mut state).is_err() {
                break;
            }
        } else if rc < 0 {
            let err = io::Error::last_os_error();
            if err.kind() != io::ErrorKind::Interrupted {
                break;
            }
        }
    }
}

/// Restores multiple `(mime, bytes)` formats atomically over Wayland data-control.
/// Returns `Ok(())` once the compositor has acknowledged the selection; the
/// background thread keeps serving `Send` requests until `Cancelled`.
pub(crate) fn restore_multi_mime_native(
    entries: Vec<(String, Vec<u8>)>,
    timeout: Duration,
) -> io::Result<()> {
    if entries.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "no MIME entries to restore",
        ));
    }
    if unsafe { libc::geteuid() } == 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "clipboard restore refuses root",
        ));
    }
    let connection = connect_wayland_display()?;
    let generation = ACTIVE_RESTORE_GEN.fetch_add(1, Ordering::AcqRel) + 1;
    let aborted = Arc::new(AtomicBool::new(false));
    let aborted_bg = Arc::clone(&aborted);
    let deadline = Instant::now() + timeout;
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    thread::Builder::new()
        .name("dboard-clip-restore".into())
        .spawn(move || {
            serve_multi_mime_connection(
                connection, entries, ready_tx, deadline, aborted_bg, generation,
            )
        })?;
    match ready_rx.recv_timeout(timeout) {
        Ok(result) => result,
        Err(_) => {
            aborted.store(true, Ordering::Release);
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "multi-MIME restore timed out",
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::os::fd::FromRawFd;

    fn make_pipe() -> (OwnedFd, OwnedFd) {
        let mut fds = [0i32; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) }
    }

    #[test]
    fn write_pipe_nonblocking_transfers_payload_larger_than_pipe_buffer() {
        let (r_fd, w_fd) = make_pipe();
        let payload: Vec<u8> = (0..(192 * 1024)).map(|i| (i % 251) as u8).collect();
        let expected = payload.clone();
        let reader = thread::spawn(move || {
            let mut f = std::fs::File::from(r_fd);
            let mut buf = Vec::new();
            f.read_to_end(&mut buf).unwrap();
            buf
        });
        assert!(write_pipe_nonblocking_with_timeout(
            w_fd,
            &payload,
            Duration::from_secs(2)
        ));
        assert_eq!(reader.join().unwrap(), expected);
    }

    #[test]
    fn write_pipe_nonblocking_times_out_on_stalled_reader() {
        let (_r_fd, w_fd) = make_pipe();
        let payload = vec![0xABu8; 256 * 1024];
        let start = Instant::now();
        let ok = write_pipe_nonblocking_with_timeout(w_fd, &payload, Duration::from_millis(60));
        assert!(!ok, "stalled pipe must abort after timeout");
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "must not block indefinitely on stalled reader"
        );
    }

    #[test]
    fn stalled_compositor_restore_aborts_cleanly_without_hanging() {
        let (client, _stalled_server) = std::os::unix::net::UnixStream::pair().unwrap();
        client.set_nonblocking(true).unwrap();
        let conn = Connection::from_socket(client).unwrap();
        let (tx, rx) = mpsc::sync_channel(1);
        let aborted = Arc::new(AtomicBool::new(false));
        let deadline = Instant::now() + Duration::from_millis(80);
        let gen = ACTIVE_RESTORE_GEN.fetch_add(1, Ordering::AcqRel) + 1;
        let start = Instant::now();
        let handle = thread::spawn(move || {
            serve_multi_mime_connection(
                conn,
                vec![("text/html".into(), b"<b>hi</b>".to_vec())],
                tx,
                deadline,
                aborted,
                gen,
            );
        });
        let err = rx
            .recv_timeout(Duration::from_secs(1))
            .expect("worker must reply on timeout")
            .expect_err("stalled compositor must return TimedOut");
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        handle.join().expect("worker thread must exit cleanly");
        assert!(start.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn stalled_mime_send_does_not_block_concurrent_mime_transfer() {
        let (_stalled_r, stalled_w) = make_pipe();
        let (fast_r, fast_w) = make_pipe();
        let state = RestoreState {
            globals: Vec::new(),
            entries: Arc::new(vec![
                ("image/png".into(), vec![0x89u8; 256 * 1024]),
                ("text/plain".into(), b"hello world".to_vec()),
            ]),
            active_sends: Arc::new(AtomicUsize::new(0)),
            cancelled: false,
            sync_done: false,
        };

        let start = Instant::now();
        // First Send stalls on an unread 256KB pipe; must return immediately without blocking.
        state.spawn_send_worker("image/png", stalled_w);
        // Second Send for `text/plain` must complete immediately in parallel.
        state.spawn_send_worker("text/plain", fast_w);

        let mut f = std::fs::File::from(fast_r);
        let mut buf = Vec::new();
        f.read_to_end(&mut buf).unwrap();
        assert_eq!(buf, b"hello world");
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "concurrent MIME transfer must not wait for stalled reader"
        );
    }
}
