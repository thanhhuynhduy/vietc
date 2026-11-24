// SPDX-License-Identifier: MIT
//! Real Wayland `zwlr_layer_shell_v1` floating candidate bar renderer (`wl_shm` ARGB8888).
//!
//! Guarantees:
//! - Always sets `KeyboardInteractivity::None` (NEVER steals keyboard focus).
//! - Anchors directly beneath the AT-SPI2 caret (`Anchor::Top | Anchor::Left`)
//!   with smart output-bounds clamping and above-caret flip near the bottom edge,
//!   falling back cleanly to top-center when caret coordinates are unavailable.
//! - Supports multi-monitor, rotated outputs (90°/270°), HiDPI (`1..=4` via
//!   `set_buffer_scale`), hover highlight, and left-click selection (`wl_pointer`).
//! - Respects `wl_buffer::Event::Release` so buffers are never destroyed while
//!   the compositor is still scanning them out (bounded to 4 in-flight buffers).

use std::io::{self, Write};
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::time::{Duration, Instant};

use wayland_client::protocol::{
    wl_buffer, wl_callback, wl_compositor, wl_output, wl_pointer, wl_registry, wl_seat, wl_shm,
    wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, WEnum};
use wayland_protocols::xdg::xdg_output::zv1::client::{zxdg_output_manager_v1, zxdg_output_v1};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

use crate::pill_render::{
    compute_pill_placement, compute_slot_layout, hit_test_slot, render_gboard_pill_argb8888,
    PillPlacement, BAR_HEIGHT, MIN_BAR_WIDTH,
};
use crate::CaretRect;

const BTN_LEFT: u32 = 0x110;
const MAX_IN_FLIGHT_BUFFERS: usize = 4;
const POPUP_ROUNDTRIP_TIMEOUT: Duration = Duration::from_millis(500);

#[derive(Debug, Clone)]
pub struct OutputInfo {
    pub global_name: u32,
    pub output: Option<wl_output::WlOutput>,
    pub xdg_output: Option<zxdg_output_v1::ZxdgOutputV1>,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub logical_x: Option<i32>,
    pub logical_y: Option<i32>,
    pub logical_width: Option<i32>,
    pub logical_height: Option<i32>,
    pub scale: i32,
    pub rotated_90: bool,
}

impl OutputInfo {
    /// Returns `(logical_x, logical_y, logical_width, logical_height, buffer_scale)`.
    /// Prefers authoritative `zxdg_output_v1` logical geometry (exact for fractional
    /// HiDPI such as 125%/150%/175% and rotated displays) and falls back to dividing
    /// physical `wl_output::Mode` by integer `wl_output::Scale`.
    fn logical_geometry(&self) -> (i32, i32, i32, i32, i32) {
        let scale = self.scale.clamp(1, 4);
        let lx = self.logical_x.unwrap_or(self.x);
        let ly = self.logical_y.unwrap_or(self.y);
        if let (Some(lw), Some(lh)) = (self.logical_width, self.logical_height) {
            if lw > 0 && lh > 0 {
                return (lx, ly, lw, lh, scale);
            }
        }
        let (raw_w, raw_h) = if self.rotated_90 {
            (self.height, self.width)
        } else {
            (self.width, self.height)
        };
        let lw = if raw_w > 0 { raw_w / scale } else { 1920 };
        let lh = if raw_h > 0 { raw_h / scale } else { 1080 };
        (lx, ly, lw, lh, scale)
    }
}

/// Select the output containing global `caret` coordinates and translate `caret`
/// into output-local logical coordinates `(x - out.logical_x, y - out.logical_y)`.
pub fn select_output_and_local_caret(
    outputs: &[OutputInfo],
    caret: Option<CaretRect>,
) -> (Option<u32>, i32, i32, i32, Option<CaretRect>) {
    let Some(first) = outputs.first() else {
        return (None, 1920, 1080, 1, caret);
    };
    let Some(c) = caret else {
        let (_, _, lw, lh, scale) = first.logical_geometry();
        return (None, lw, lh, scale, None);
    };
    let matched = outputs
        .iter()
        .find(|o| {
            let (ox, oy, lw, lh, _) = o.logical_geometry();
            c.x >= ox && c.x < ox + lw && c.y >= oy && c.y < oy + lh
        })
        .unwrap_or(first);
    let (ox, oy, lw, lh, scale) = matched.logical_geometry();
    let local_caret = CaretRect {
        x: c.x.saturating_sub(ox),
        y: c.y.saturating_sub(oy),
        h: c.h,
    };
    (Some(matched.global_name), lw, lh, scale, Some(local_caret))
}

struct InFlightBuffer {
    id: u64,
    buffer: wl_buffer::WlBuffer,
    pool: wl_shm_pool::WlShmPool,
    _fd: OwnedFd,
}

impl InFlightBuffer {
    fn destroy(self) {
        self.buffer.destroy();
        self.pool.destroy();
    }
}

struct LayerState {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    layer_shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    xdg_output_manager: Option<zxdg_output_manager_v1::ZxdgOutputManagerV1>,
    seat: Option<wl_seat::WlSeat>,
    pointer: Option<wl_pointer::WlPointer>,
    outputs: Vec<OutputInfo>,
    in_flight_buffers: Vec<InFlightBuffer>,
    pointer_x: f64,
    hovered_slot: Option<usize>,
    clicked_slot: Option<usize>,
    hover_dirty: bool,
    slot_bounds: Vec<(i32, i32)>,
    configured: bool,
    closed: bool,
    sync_done: bool,
}

impl Default for LayerState {
    fn default() -> Self {
        Self {
            compositor: None,
            shm: None,
            layer_shell: None,
            xdg_output_manager: None,
            seat: None,
            pointer: None,
            outputs: Vec::new(),
            in_flight_buffers: Vec::new(),
            pointer_x: -1.0,
            hovered_slot: None,
            clicked_slot: None,
            hover_dirty: false,
            slot_bounds: Vec::new(),
            configured: false,
            closed: false,
            sync_done: false,
        }
    }
}

impl LayerState {
    fn release_buffer(&mut self, id: u64) {
        if let Some(pos) = self.in_flight_buffers.iter().position(|b| b.id == id) {
            self.in_flight_buffers.remove(pos).destroy();
        }
    }

    fn destroy_all_buffers(&mut self) {
        for buf in self.in_flight_buffers.drain(..) {
            buf.destroy();
        }
    }
}

impl Dispatch<wl_registry::WlRegistry, ()> for LayerState {
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
            } => match interface.as_str() {
                "wl_compositor" if state.compositor.is_none() => {
                    state.compositor = Some(registry.bind(name, version.min(4), qh, ()));
                }
                "wl_shm" if state.shm.is_none() => {
                    state.shm = Some(registry.bind(name, version.min(1), qh, ()));
                }
                "zwlr_layer_shell_v1" if state.layer_shell.is_none() => {
                    state.layer_shell = Some(registry.bind(name, version.min(4), qh, ()));
                }
                "zxdg_output_manager_v1" if state.xdg_output_manager.is_none() => {
                    let mgr: zxdg_output_manager_v1::ZxdgOutputManagerV1 =
                        registry.bind(name, version.min(3), qh, ());
                    for out in &mut state.outputs {
                        if out.xdg_output.is_none() {
                            if let Some(ref wl_out) = out.output {
                                out.xdg_output =
                                    Some(mgr.get_xdg_output(wl_out, qh, out.global_name));
                            }
                        }
                    }
                    state.xdg_output_manager = Some(mgr);
                }
                "wl_output" => {
                    let out: wl_output::WlOutput = registry.bind(name, version.min(3), qh, name);
                    let xdg_output = state
                        .xdg_output_manager
                        .as_ref()
                        .map(|mgr| mgr.get_xdg_output(&out, qh, name));
                    state.outputs.push(OutputInfo {
                        global_name: name,
                        output: Some(out),
                        xdg_output,
                        x: 0,
                        y: 0,
                        width: 0,
                        height: 0,
                        logical_x: None,
                        logical_y: None,
                        logical_width: None,
                        logical_height: None,
                        scale: 1,
                        rotated_90: false,
                    });
                }
                "wl_seat" if state.seat.is_none() => {
                    state.seat = Some(registry.bind(name, version.min(5), qh, ()));
                }
                _ => {}
            },
            wl_registry::Event::GlobalRemove { name } => {
                state.outputs.retain(|o| {
                    if o.global_name == name {
                        if let Some(ref xdg) = o.xdg_output {
                            xdg.destroy();
                        }
                        if let Some(ref out) = o.output {
                            out.release();
                        }
                        false
                    } else {
                        true
                    }
                });
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_output::WlOutput, u32> for LayerState {
    fn event(
        state: &mut Self,
        _: &wl_output::WlOutput,
        event: wl_output::Event,
        global_name: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(out) = state
            .outputs
            .iter_mut()
            .find(|o| o.global_name == *global_name)
        else {
            return;
        };
        match event {
            wl_output::Event::Geometry {
                x, y, transform, ..
            } => {
                out.x = x;
                out.y = y;
                out.rotated_90 = matches!(
                    transform,
                    WEnum::Value(
                        wl_output::Transform::_90
                            | wl_output::Transform::_270
                            | wl_output::Transform::Flipped90
                            | wl_output::Transform::Flipped270
                    )
                );
            }
            wl_output::Event::Mode {
                flags: WEnum::Value(f),
                width,
                height,
                ..
            } if f.contains(wl_output::Mode::Current) => {
                if width > 0 && height > 0 {
                    out.width = width;
                    out.height = height;
                }
            }
            wl_output::Event::Scale { factor } if factor > 0 => {
                out.scale = factor.clamp(1, 4);
            }
            _ => {}
        }
    }
}

wayland_client::delegate_noop!(LayerState: ignore zxdg_output_manager_v1::ZxdgOutputManagerV1);

impl Dispatch<zxdg_output_v1::ZxdgOutputV1, u32> for LayerState {
    fn event(
        state: &mut Self,
        _: &zxdg_output_v1::ZxdgOutputV1,
        event: zxdg_output_v1::Event,
        global_name: &u32,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(out) = state
            .outputs
            .iter_mut()
            .find(|o| o.global_name == *global_name)
        else {
            return;
        };
        match event {
            zxdg_output_v1::Event::LogicalPosition { x, y } => {
                out.logical_x = Some(x);
                out.logical_y = Some(y);
            }
            zxdg_output_v1::Event::LogicalSize { width, height } if width > 0 && height > 0 => {
                out.logical_width = Some(width);
                out.logical_height = Some(height);
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for LayerState {
    fn event(
        state: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(caps),
        } = event
        {
            let has_pointer = caps.contains(wl_seat::Capability::Pointer);
            if has_pointer && state.pointer.is_none() {
                state.pointer = Some(seat.get_pointer(qh, ()));
            } else if !has_pointer {
                if let Some(ptr) = state.pointer.take() {
                    ptr.release();
                }
                state.hovered_slot = None;
                state.clicked_slot = None;
            }
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for LayerState {
    fn event(
        state: &mut Self,
        _: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Enter { surface_x, .. }
            | wl_pointer::Event::Motion { surface_x, .. } => {
                state.pointer_x = surface_x;
                let next = hit_test_slot(&state.slot_bounds, surface_x);
                if next != state.hovered_slot {
                    state.hovered_slot = next;
                    state.hover_dirty = true;
                }
            }
            wl_pointer::Event::Leave { .. } => {
                state.pointer_x = -1.0;
                if state.hovered_slot.take().is_some() {
                    state.hover_dirty = true;
                }
            }
            wl_pointer::Event::Button {
                button,
                state: WEnum::Value(wl_pointer::ButtonState::Pressed),
                ..
            } if button == BTN_LEFT => {
                if let Some(slot) = state.hovered_slot.or_else(|| {
                    (state.pointer_x >= 0.0)
                        .then(|| hit_test_slot(&state.slot_bounds, state.pointer_x))
                        .flatten()
                }) {
                    state.clicked_slot = Some(slot);
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_buffer::WlBuffer, u64> for LayerState {
    fn event(
        state: &mut Self,
        _: &wl_buffer::WlBuffer,
        event: wl_buffer::Event,
        id: &u64,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if matches!(event, wl_buffer::Event::Release) {
            state.release_buffer(*id);
        }
    }
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for LayerState {
    fn event(
        state: &mut Self,
        surface: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_layer_surface_v1::Event::Configure { serial, .. } => {
                surface.ack_configure(serial);
                state.configured = true;
            }
            zwlr_layer_surface_v1::Event::Closed => {
                state.closed = true;
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_callback::WlCallback, ()> for LayerState {
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
    queue: &mut EventQueue<LayerState>,
    state: &mut LayerState,
    timeout: Duration,
    label: &str,
) -> io::Result<()> {
    state.sync_done = false;
    let qh = queue.handle();
    let _cb = connection.display().sync(&qh, ());
    let deadline = Instant::now() + timeout;
    while !state.sync_done {
        queue
            .flush()
            .map_err(|e| io::Error::other(format!("{label}: {e}")))?;
        queue
            .dispatch_pending(state)
            .map_err(|e| io::Error::other(format!("{label}: {e}")))?;
        if state.sync_done {
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
            if let Err(wayland_client::backend::WaylandError::Io(e)) = guard.read() {
                if e.kind() != io::ErrorKind::WouldBlock {
                    return Err(e);
                }
            }
        }
    }
    Ok(())
}

wayland_client::delegate_noop!(LayerState: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(LayerState: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(LayerState: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(LayerState: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(LayerState: ignore zwlr_layer_shell_v1::ZwlrLayerShellV1);

pub struct WaylandLayerBar {
    connection: Connection,
    queue: EventQueue<LayerState>,
    state: LayerState,
    surface: wl_surface::WlSurface,
    layer_surface: zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
    shm: wl_shm::WlShm,
    next_buffer_id: u64,
    last_candidates: Vec<String>,
    last_caret: Option<CaretRect>,
    last_placement: Option<PillPlacement>,
    last_width: u32,
    bound_output_name: Option<u32>,
}

impl WaylandLayerBar {
    pub fn connect() -> io::Result<Self> {
        let connection = Connection::connect_to_env().map_err(io::Error::other)?;
        let mut queue = connection.new_event_queue::<LayerState>();
        let qh = queue.handle();
        let _registry = connection.display().get_registry(&qh, ());
        let mut state = LayerState::default();
        roundtrip_with_timeout(
            &connection,
            &mut queue,
            &mut state,
            POPUP_ROUNDTRIP_TIMEOUT,
            "popup registry roundtrip",
        )?;

        let compositor = state.compositor.as_ref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                "compositor missing wl_compositor",
            )
        })?;
        let shm = state.shm.clone().ok_or_else(|| {
            io::Error::new(io::ErrorKind::Unsupported, "compositor missing wl_shm")
        })?;
        let layer_shell = state.layer_shell.as_ref().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                "compositor missing zwlr_layer_shell_v1",
            )
        })?;

        let surface = compositor.create_surface(&qh, ());
        let layer_surface = layer_shell.get_layer_surface(
            &surface,
            None,
            zwlr_layer_shell_v1::Layer::Overlay,
            "dboard-candidates".into(),
            &qh,
            (),
        );
        layer_surface.set_size(MIN_BAR_WIDTH, BAR_HEIGHT);
        layer_surface.set_anchor(zwlr_layer_surface_v1::Anchor::Top);
        layer_surface.set_margin(36, 0, 0, 0);
        layer_surface.set_exclusive_zone(0);
        // Critical invariant: NEVER steal keyboard focus from the active editor!
        layer_surface
            .set_keyboard_interactivity(zwlr_layer_surface_v1::KeyboardInteractivity::None);
        surface.commit();
        roundtrip_with_timeout(
            &connection,
            &mut queue,
            &mut state,
            POPUP_ROUNDTRIP_TIMEOUT,
            "popup initial surface roundtrip",
        )?;

        Ok(Self {
            connection,
            queue,
            state,
            surface,
            layer_surface,
            shm,
            next_buffer_id: 1,
            last_candidates: Vec::new(),
            last_caret: None,
            last_placement: None,
            last_width: MIN_BAR_WIDTH,
            bound_output_name: None,
        })
    }

    pub fn connection_fd(&self) -> RawFd {
        self.connection.backend().poll_fd().as_raw_fd()
    }

    pub fn is_closed(&self) -> bool {
        self.state.closed
    }

    pub fn take_clicked_slot(&mut self) -> Option<usize> {
        self.state.clicked_slot.take()
    }

    pub fn dispatch_pending(&mut self) -> io::Result<()> {
        let was_configured = self.state.configured;
        self.queue.flush().map_err(io::Error::other)?;
        if let Some(guard) = self.queue.prepare_read() {
            match guard.read() {
                Ok(_) => {}
                Err(wayland_client::backend::WaylandError::Io(e))
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) => {}
                Err(err) => return Err(io::Error::other(err)),
            }
        }
        self.queue
            .dispatch_pending(&mut self.state)
            .map_err(io::Error::other)?;
        let just_configured = !was_configured && self.state.configured;
        if (self.state.hover_dirty || just_configured) && !self.last_candidates.is_empty() {
            self.state.hover_dirty = false;
            let cands = self.last_candidates.clone();
            let caret = self.last_caret;
            self.show(&cands, caret)?;
        }
        Ok(())
    }

    fn rebind_layer_surface_for_output(
        &mut self,
        target_output: Option<u32>,
        bar_w: u32,
        placement: PillPlacement,
    ) -> io::Result<()> {
        let bound_still_exists = self
            .bound_output_name
            .is_none_or(|n| self.state.outputs.iter().any(|o| o.global_name == n));
        if self.bound_output_name == target_output && bound_still_exists {
            return Ok(());
        }
        let (Some(compositor), Some(layer_shell)) = (
            self.state.compositor.clone(),
            self.state.layer_shell.clone(),
        ) else {
            return Ok(());
        };
        let wl_out = target_output.and_then(|name| {
            self.state
                .outputs
                .iter()
                .find(|o| o.global_name == name)
                .and_then(|o| o.output.clone())
        });
        let qh = self.queue.handle();
        // Per Wayland core protocol (`wl_surface` role lifecycle), destroy the old
        // role + surface + buffers and allocate a fresh `wl_surface` before rebinding.
        self.layer_surface.destroy();
        self.surface.destroy();
        self.state.destroy_all_buffers();
        self.state.hovered_slot = None;
        self.state.clicked_slot = None;

        let new_surface = compositor.create_surface(&qh, ());
        let next = layer_shell.get_layer_surface(
            &new_surface,
            wl_out.as_ref(),
            zwlr_layer_shell_v1::Layer::Overlay,
            "dboard-candidates".into(),
            &qh,
            (),
        );
        next.set_size(bar_w, BAR_HEIGHT);
        if placement.anchored_to_caret {
            next.set_anchor(
                zwlr_layer_surface_v1::Anchor::Top | zwlr_layer_surface_v1::Anchor::Left,
            );
        } else {
            next.set_anchor(zwlr_layer_surface_v1::Anchor::Top);
        }
        next.set_margin(
            placement.margin_top,
            0,
            placement.margin_bottom,
            placement.margin_left,
        );
        next.set_exclusive_zone(0);
        next.set_keyboard_interactivity(zwlr_layer_surface_v1::KeyboardInteractivity::None);
        self.surface = new_surface;
        self.layer_surface = next;
        self.bound_output_name = if wl_out.is_some() {
            target_output
        } else {
            None
        };
        self.last_width = bar_w;
        self.last_placement = Some(placement);
        self.state.configured = false;
        self.surface.commit();
        roundtrip_with_timeout(
            &self.connection,
            &mut self.queue,
            &mut self.state,
            POPUP_ROUNDTRIP_TIMEOUT,
            "popup output rebind roundtrip",
        )?;
        // Any `Closed` event dispatched during this roundtrip belonged to the
        // just-destroyed previous layer_surface; keep the new surface alive.
        self.state.closed = false;
        Ok(())
    }

    pub fn show(&mut self, candidates: &[String], caret: Option<CaretRect>) -> io::Result<()> {
        let (bar_w, slot_bounds) = compute_slot_layout(candidates);
        self.state.slot_bounds = slot_bounds.clone();
        self.last_candidates = candidates.to_vec();
        self.last_caret = caret;

        let (target_output, out_w, out_h, scale, local_caret) =
            select_output_and_local_caret(&self.state.outputs, caret);
        let placement = compute_pill_placement(local_caret, bar_w, BAR_HEIGHT, out_w, out_h);
        let bound_removed = self
            .bound_output_name
            .is_some_and(|n| !self.state.outputs.iter().any(|o| o.global_name == n));
        if self.state.outputs.len() > 1 || bound_removed {
            let effective_target = if self.state.outputs.len() > 1 {
                target_output
            } else {
                None
            };
            self.rebind_layer_surface_for_output(effective_target, bar_w, placement)?;
        }

        if self.last_width != bar_w || self.last_placement != Some(placement) {
            self.layer_surface.set_size(bar_w, BAR_HEIGHT);
            if placement.anchored_to_caret {
                self.layer_surface.set_anchor(
                    zwlr_layer_surface_v1::Anchor::Top | zwlr_layer_surface_v1::Anchor::Left,
                );
            } else {
                self.layer_surface
                    .set_anchor(zwlr_layer_surface_v1::Anchor::Top);
            }
            self.layer_surface.set_margin(
                placement.margin_top,
                0,
                placement.margin_bottom,
                placement.margin_left,
            );
            self.last_width = bar_w;
            self.last_placement = Some(placement);
        }

        if !self.state.configured {
            self.surface.commit();
            self.connection.flush().map_err(io::Error::other)?;
            return Ok(());
        }

        let phys_w = bar_w * (scale as u32);
        let phys_h = BAR_HEIGHT * (scale as u32);
        let pixels = render_gboard_pill_argb8888(
            candidates,
            &slot_bounds,
            self.state.hovered_slot,
            bar_w,
            BAR_HEIGHT,
            scale as u32,
        );
        let fd = create_shm_memfd(&pixels)?;
        let qh = self.queue.handle();
        let buf_id = self.next_buffer_id;
        self.next_buffer_id = self.next_buffer_id.wrapping_add(1);
        let pool = self
            .shm
            .create_pool(fd.as_fd(), pixels.len() as i32, &qh, ());
        let stride = (phys_w * 4) as i32;
        let buffer = pool.create_buffer(
            0,
            phys_w as i32,
            phys_h as i32,
            stride,
            wl_shm::Format::Argb8888,
            &qh,
            buf_id,
        );
        self.surface.set_buffer_scale(scale);
        self.surface.attach(Some(&buffer), 0, 0);
        self.surface
            .damage_buffer(0, 0, phys_w as i32, phys_h as i32);
        self.surface.commit();

        // Keep buffer alive until compositor sends `wl_buffer::Event::Release`,
        // capping in-flight buffers to prevent unbounded FD growth on buggy compositors.
        while self.state.in_flight_buffers.len() >= MAX_IN_FLIGHT_BUFFERS {
            self.state.in_flight_buffers.remove(0).destroy();
        }
        self.state.in_flight_buffers.push(InFlightBuffer {
            id: buf_id,
            buffer,
            pool,
            _fd: fd,
        });
        self.connection.flush().map_err(io::Error::other)?;
        Ok(())
    }

    pub fn hide(&mut self) -> io::Result<()> {
        self.last_candidates.clear();
        self.state.slot_bounds.clear();
        self.state.hovered_slot = None;
        self.state.clicked_slot = None;
        self.surface.attach(None, 0, 0);
        self.surface.commit();
        self.connection.flush().map_err(io::Error::other)?;
        Ok(())
    }
}

impl Drop for WaylandLayerBar {
    fn drop(&mut self) {
        self.state.destroy_all_buffers();
        if let Some(pointer) = self.state.pointer.take() {
            pointer.release();
        }
        self.layer_surface.destroy();
        self.surface.destroy();
        let _ = self.connection.flush();
    }
}

fn create_shm_memfd(bytes: &[u8]) -> io::Result<OwnedFd> {
    let raw = unsafe { libc::memfd_create(c"dboard-popup-shm".as_ptr(), libc::MFD_CLOEXEC) };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    let mut file = std::fs::File::from(fd);
    file.write_all(bytes)?;
    Ok(OwnedFd::from(file))
}

#[cfg(test)]
#[path = "wayland_layer_tests.rs"]
mod tests;
