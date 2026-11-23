// SPDX-License-Identifier: MIT
//! Real Wayland `zwlr_layer_shell_v1` floating candidate bar renderer (`wl_shm` ARGB8888).
//!
//! Guarantees:
//! - Always sets `KeyboardInteractivity::None` (NEVER steals keyboard focus).
//! - Anchors directly beneath the AT-SPI2 caret (`Anchor::Top | Anchor::Left`)
//!   with smart output-bounds clamping and above-caret flip near the bottom edge,
//!   falling back cleanly to bottom-center when caret coordinates are unavailable.
//! - Supports HiDPI (`wl_output` scale via `set_buffer_scale`), dynamic pill
//!   width, anti-aliased rounded corners, hover highlight, and left-click
//!   candidate selection (`wl_pointer`).
//! - Attaches `None` buffer on `hide()` so the surface unmaps immediately on
//!   password fields, bypass apps, focus loss, or word commit.

use std::io::{self, Write};
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd, RawFd};

use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_output, wl_pointer, wl_registry, wl_seat, wl_shm, wl_shm_pool,
    wl_surface,
};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, WEnum};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

use crate::CaretRect;

pub const MIN_BAR_WIDTH: u32 = 220;
pub const MAX_BAR_WIDTH: u32 = 560;
pub const BAR_HEIGHT: u32 = 44;
const BTN_LEFT: u32 = 0x110;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PillPlacement {
    pub anchored_to_caret: bool,
    pub margin_top: i32,
    pub margin_bottom: i32,
    pub margin_left: i32,
}

/// Compute screen-clamped placement for a pill of size `(bar_w, bar_h)` inside
/// an output of `(out_w, out_h)`. Flips above the caret if below-caret would
/// overflow the bottom edge; falls back to bottom-center when `caret` is `None`.
pub fn compute_pill_placement(
    caret: Option<CaretRect>,
    bar_w: u32,
    bar_h: u32,
    out_w: i32,
    out_h: i32,
) -> PillPlacement {
    let Some(c) = caret else {
        return PillPlacement {
            anchored_to_caret: false,
            margin_top: 0,
            margin_bottom: 28,
            margin_left: 0,
        };
    };
    let bw = bar_w as i32;
    let bh = bar_h as i32;
    let max_left = (out_w - bw - 8).max(8);
    let left = c.x.clamp(8, max_left);
    let below = c.y.saturating_add(c.h).saturating_add(8);
    let top = if below + bh <= out_h - 8 {
        below
    } else {
        c.y.saturating_sub(bh + 8).max(8)
    };
    PillPlacement {
        anchored_to_caret: true,
        margin_top: top,
        margin_bottom: 0,
        margin_left: left,
    }
}

/// Compute dynamic pill width and per-slot logical `(x_start, x_end)` hit-boxes.
pub fn compute_slot_layout(candidates: &[String]) -> (u32, Vec<(i32, i32)>) {
    let mut x = 14i32;
    let mut bounds = Vec::with_capacity(candidates.len().min(3));
    for (idx, cand) in candidates.iter().take(3).enumerate() {
        let chars = cand.chars().count().min(18) as i32;
        // `[1] ` (4 chars * 8px) + candidate (chars * 8px) + horizontal pill padding
        let slot_w = (4 + chars) * 8 + 20;
        let x_end = x + slot_w;
        bounds.push((x, x_end));
        x = x_end
            + if idx + 1 < candidates.len().min(3) {
                10
            } else {
                0
            };
    }
    let total_w = ((x + 14) as u32).clamp(MIN_BAR_WIDTH, MAX_BAR_WIDTH);
    (total_w, bounds)
}

pub fn hit_test_slot(slot_bounds: &[(i32, i32)], pointer_x: f64) -> Option<usize> {
    let px = pointer_x.round() as i32;
    slot_bounds
        .iter()
        .position(|&(start, end)| px >= start && px <= end)
}

#[derive(Default)]
struct LayerState {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    layer_shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
    seat: Option<wl_seat::WlSeat>,
    pointer: Option<wl_pointer::WlPointer>,
    output_width: i32,
    output_height: i32,
    output_scale: i32,
    pointer_x: f64,
    hovered_slot: Option<usize>,
    clicked_slot: Option<usize>,
    hover_dirty: bool,
    slot_bounds: Vec<(i32, i32)>,
    configured: bool,
    closed: bool,
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
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            match interface.as_str() {
                "wl_compositor" if state.compositor.is_none() => {
                    state.compositor = Some(registry.bind(name, version.min(4), qh, ()));
                }
                "wl_shm" if state.shm.is_none() => {
                    state.shm = Some(registry.bind(name, version.min(1), qh, ()));
                }
                "zwlr_layer_shell_v1" if state.layer_shell.is_none() => {
                    state.layer_shell = Some(registry.bind(name, version.min(4), qh, ()));
                }
                "wl_output" if state.output_width == 0 => {
                    let _: wl_output::WlOutput = registry.bind(name, version.min(3), qh, ());
                }
                "wl_seat" if state.seat.is_none() => {
                    state.seat = Some(registry.bind(name, version.min(5), qh, ()));
                }
                _ => {}
            }
        }
    }
}

impl Dispatch<wl_output::WlOutput, ()> for LayerState {
    fn event(
        state: &mut Self,
        _: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_output::Event::Mode {
                flags: WEnum::Value(f),
                width,
                height,
                ..
            } if f.contains(wl_output::Mode::Current) && width > 0 && height > 0 => {
                state.output_width = width;
                state.output_height = height;
            }
            wl_output::Event::Scale { factor } => {
                if factor >= 1 {
                    state.output_scale = factor.min(2);
                }
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
            if caps.contains(wl_seat::Capability::Pointer) && state.pointer.is_none() {
                state.pointer = Some(seat.get_pointer(qh, ()));
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
                let next_hover = hit_test_slot(&state.slot_bounds, surface_x);
                if next_hover != state.hovered_slot {
                    state.hovered_slot = next_hover;
                    state.hover_dirty = true;
                }
            }
            wl_pointer::Event::Leave { .. } => {
                if state.hovered_slot.is_some() {
                    state.hovered_slot = None;
                    state.hover_dirty = true;
                }
            }
            wl_pointer::Event::Button {
                button,
                state: WEnum::Value(wl_pointer::ButtonState::Pressed),
                ..
            } if button == BTN_LEFT => {
                if let Some(slot) = hit_test_slot(&state.slot_bounds, state.pointer_x) {
                    state.clicked_slot = Some(slot);
                }
            }
            _ => {}
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

wayland_client::delegate_noop!(LayerState: ignore wl_compositor::WlCompositor);
wayland_client::delegate_noop!(LayerState: ignore wl_surface::WlSurface);
wayland_client::delegate_noop!(LayerState: ignore wl_shm::WlShm);
wayland_client::delegate_noop!(LayerState: ignore wl_shm_pool::WlShmPool);
wayland_client::delegate_noop!(LayerState: ignore wl_buffer::WlBuffer);
wayland_client::delegate_noop!(LayerState: ignore zwlr_layer_shell_v1::ZwlrLayerShellV1);

pub struct WaylandLayerBar {
    connection: Connection,
    queue: EventQueue<LayerState>,
    state: LayerState,
    surface: wl_surface::WlSurface,
    layer_surface: zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
    shm: wl_shm::WlShm,
    active_buffer: Option<(wl_buffer::WlBuffer, wl_shm_pool::WlShmPool, OwnedFd)>,
    last_candidates: Vec<String>,
    last_caret: Option<CaretRect>,
    last_placement: Option<PillPlacement>,
    last_width: u32,
}

impl WaylandLayerBar {
    pub fn connect() -> io::Result<Self> {
        let connection = Connection::connect_to_env().map_err(io::Error::other)?;
        let mut queue = connection.new_event_queue::<LayerState>();
        let qh = queue.handle();
        let _registry = connection.display().get_registry(&qh, ());
        let mut state = LayerState {
            output_scale: 1,
            ..LayerState::default()
        };
        queue.roundtrip(&mut state).map_err(io::Error::other)?;

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
        layer_surface.set_anchor(zwlr_layer_surface_v1::Anchor::Bottom);
        layer_surface.set_margin(0, 0, 28, 0);
        layer_surface.set_exclusive_zone(0);
        // Critical invariant: NEVER steal keyboard focus from the active editor!
        layer_surface
            .set_keyboard_interactivity(zwlr_layer_surface_v1::KeyboardInteractivity::None);
        surface.commit();
        queue.roundtrip(&mut state).map_err(io::Error::other)?;

        Ok(Self {
            connection,
            queue,
            state,
            surface,
            layer_surface,
            shm,
            active_buffer: None,
            last_candidates: Vec::new(),
            last_caret: None,
            last_placement: None,
            last_width: MIN_BAR_WIDTH,
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
        let _ = self.queue.flush();
        if let Some(guard) = self.queue.prepare_read() {
            let _ = guard.read();
        }
        self.queue
            .dispatch_pending(&mut self.state)
            .map_err(io::Error::other)?;
        if self.state.hover_dirty && !self.last_candidates.is_empty() {
            self.state.hover_dirty = false;
            let cands = self.last_candidates.clone();
            let caret = self.last_caret;
            let _ = self.show(&cands, caret);
        }
        Ok(())
    }

    pub fn show(&mut self, candidates: &[String], caret: Option<CaretRect>) -> io::Result<()> {
        let (bar_w, slot_bounds) = compute_slot_layout(candidates);
        self.state.slot_bounds = slot_bounds.clone();
        self.last_candidates = candidates.to_vec();
        self.last_caret = caret;

        let scale = self.state.output_scale.clamp(1, 2);
        let out_w = if self.state.output_width > 0 {
            self.state.output_width / scale
        } else {
            1920
        };
        let out_h = if self.state.output_height > 0 {
            self.state.output_height / scale
        } else {
            1080
        };
        let placement = compute_pill_placement(caret, bar_w, BAR_HEIGHT, out_w, out_h);

        if self.last_width != bar_w || self.last_placement != Some(placement) {
            self.layer_surface.set_size(bar_w, BAR_HEIGHT);
            if placement.anchored_to_caret {
                self.layer_surface.set_anchor(
                    zwlr_layer_surface_v1::Anchor::Top | zwlr_layer_surface_v1::Anchor::Left,
                );
            } else {
                self.layer_surface
                    .set_anchor(zwlr_layer_surface_v1::Anchor::Bottom);
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
            (),
        );
        self.surface.set_buffer_scale(scale);
        self.surface.attach(Some(&buffer), 0, 0);
        self.surface
            .damage_buffer(0, 0, phys_w as i32, phys_h as i32);
        self.surface.commit();
        self.connection.flush().map_err(io::Error::other)?;

        if let Some((old_buf, old_pool, _old_fd)) = self.active_buffer.replace((buffer, pool, fd)) {
            old_buf.destroy();
            old_pool.destroy();
        }
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
        if let Some((old_buf, old_pool, _old_fd)) = self.active_buffer.take() {
            old_buf.destroy();
            old_pool.destroy();
        }
        Ok(())
    }
}

impl Drop for WaylandLayerBar {
    fn drop(&mut self) {
        if let Some((buf, pool, _)) = self.active_buffer.take() {
            buf.destroy();
            pool.destroy();
        }
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

#[inline]
fn blend_premul(dst: &mut [u8], bgra: [u8; 4], alpha_f: f32) {
    let a = (f32::from(bgra[3]) * alpha_f.clamp(0.0, 1.0)).round() as u8;
    if a == 0 {
        return;
    }
    let af = f32::from(a) / 255.0;
    let inv = 1.0 - af;
    let sr = (f32::from(bgra[2]) * af).round();
    let sg = (f32::from(bgra[1]) * af).round();
    let sb = (f32::from(bgra[0]) * af).round();
    dst[0] = (sb + f32::from(dst[0]) * inv).min(255.0) as u8;
    dst[1] = (sg + f32::from(dst[1]) * inv).min(255.0) as u8;
    dst[2] = (sr + f32::from(dst[2]) * inv).min(255.0) as u8;
    dst[3] = (f32::from(a) + f32::from(dst[3]) * inv).min(255.0) as u8;
}

/// Signed distance to a rounded rectangle centered at `(cx, cy)` with half-size `(hx, hy)` and radius `r`.
#[inline]
fn rounded_rect_sdf(px: f32, py: f32, cx: f32, cy: f32, hx: f32, hy: f32, r: f32) -> f32 {
    let qx = (px - cx).abs() - (hx - r);
    let qy = (py - cy).abs() - (hy - r);
    let outside = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    let inside = qx.max(qy).min(0.0);
    outside + inside - r
}

/// Renders a polished Gboard-style floating pill (`ARGB8888`, premultiplied)
/// with anti-aliased rounded corners, HiDPI `scale` (`1` or `2`), active/hover
/// slot highlight pills, and crisp 134-char Vietnamese Unicode glyphs.
pub fn render_gboard_pill_argb8888(
    candidates: &[String],
    slot_bounds: &[(i32, i32)],
    hovered_slot: Option<usize>,
    logical_w: u32,
    logical_h: u32,
    scale: u32,
) -> Vec<u8> {
    let scale = scale.clamp(1, 2);
    let width = logical_w * scale;
    let height = logical_h * scale;
    let mut buf = vec![0u8; (width * height * 4) as usize];

    let bg: [u8; 4] = [0x2A, 0x1E, 0x1E, 0xF0]; // Dark slate #1E1E2A @ 94%
    let border: [u8; 4] = [0xFA, 0xB4, 0x89, 0xCC]; // Soft accent border #89B4FA
    let hover_bg: [u8; 4] = [0x58, 0x44, 0x3B, 0xE8]; // Highlighted slot pill
    let primary_bg: [u8; 4] = [0x45, 0x33, 0x2E, 0xB0]; // Subtle slot #1 pill
    let fg: [u8; 4] = [0xF4, 0xD6, 0xCD, 0xFF]; // #CDD6F4 crisp text
    let badge_fg: [u8; 4] = [0xFA, 0xB4, 0x89, 0xFF]; // #89B4FA badge

    let cx = (width as f32) * 0.5;
    let cy = (height as f32) * 0.5;
    let hx = cx - 1.5;
    let hy = cy - 1.5;
    let radius = (14.0 * scale as f32).min(hy);

    for py in 0..height {
        for px in 0..width {
            let d = rounded_rect_sdf(px as f32 + 0.5, py as f32 + 0.5, cx, cy, hx, hy, radius);
            if d > 1.0 {
                continue;
            }
            let off = ((py * width + px) * 4) as usize;
            let cov = (0.5 - d).clamp(0.0, 1.0);
            blend_premul(&mut buf[off..off + 4], bg, cov);
            // Crisp 1.25px anti-aliased border ring
            let border_alpha = (1.0 - (d + 0.75).abs()).clamp(0.0, 1.0) * cov;
            if border_alpha > 0.01 {
                blend_premul(&mut buf[off..off + 4], border, border_alpha);
            }
        }
    }

    // Draw slot highlight pills (hovered slot or primary slot #1)
    for (idx, &(sx, ex)) in slot_bounds.iter().enumerate() {
        let pill_color = if hovered_slot == Some(idx) {
            Some(hover_bg)
        } else if idx == 0 {
            Some(primary_bg)
        } else {
            None
        };
        if let Some(color) = pill_color {
            let s_cx = ((sx + ex) as f32) * 0.5 * (scale as f32);
            let s_hx = ((ex - sx) as f32) * 0.5 * (scale as f32);
            let s_hy = (logical_h as f32 * 0.5 - 6.0) * (scale as f32);
            let s_r = 8.0 * (scale as f32);
            let x0 = ((sx.max(0) as u32) * scale).min(width);
            let x1 = ((ex.max(0) as u32) * scale).min(width);
            for py in (4 * scale)..(height.saturating_sub(4 * scale)) {
                for px in x0..x1 {
                    let d = rounded_rect_sdf(
                        px as f32 + 0.5,
                        py as f32 + 0.5,
                        s_cx,
                        cy,
                        s_hx,
                        s_hy,
                        s_r,
                    );
                    if d < 1.0 {
                        let off = ((py * width + px) * 4) as usize;
                        blend_premul(&mut buf[off..off + 4], color, (0.5 - d).clamp(0.0, 1.0));
                    }
                }
            }
        }
    }

    // Draw candidate badges & Vietnamese glyphs scaled crisply by `scale`
    let base_y = (logical_h.saturating_sub(crate::font::CELL_H)) / 2;
    for (idx, (cand, &(sx, _))) in candidates
        .iter()
        .take(3)
        .zip(slot_bounds.iter())
        .enumerate()
    {
        let label = format!("[{}] {}", idx + 1, cand);
        let mut pen_x = (sx + 8).max(8) as u32;
        for (ci, ch) in label.chars().enumerate() {
            if pen_x + crate::font::CELL_W >= logical_w.saturating_sub(8) {
                break;
            }
            let color = if ci < 3 { badge_fg } else { fg };
            let glyph = crate::font::glyph_8x16(ch);
            for (row, &bits) in glyph.iter().enumerate() {
                for col in 0..crate::font::CELL_W {
                    if (bits & (1 << (7 - col))) != 0 {
                        for sy in 0..scale {
                            for sx_p in 0..scale {
                                let px = (pen_x + col) * scale + sx_p;
                                let py = (base_y + row as u32) * scale + sy;
                                if px < width && py < height {
                                    let off = ((py * width + px) * 4) as usize;
                                    buf[off..off + 4].copy_from_slice(&color);
                                }
                            }
                        }
                    }
                }
            }
            pen_x += crate::font::CELL_W;
        }
    }

    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_caret_placement_with_clamping_and_bottom_flip() {
        // Normal caret mid-screen -> placed right below caret
        let p = compute_pill_placement(
            Some(CaretRect {
                x: 400,
                y: 300,
                h: 20,
            }),
            320,
            BAR_HEIGHT,
            1920,
            1080,
        );
        assert!(p.anchored_to_caret);
        assert_eq!(p.margin_left, 400);
        assert_eq!(p.margin_top, 328);

        // Near bottom edge -> flips ABOVE caret
        let p_bot = compute_pill_placement(
            Some(CaretRect {
                x: 1850,
                y: 1040,
                h: 20,
            }),
            320,
            BAR_HEIGHT,
            1920,
            1080,
        );
        assert!(p_bot.anchored_to_caret);
        assert_eq!(p_bot.margin_left, 1920 - 320 - 8);
        assert_eq!(p_bot.margin_top, 1040 - (BAR_HEIGHT as i32) - 8);

        // No caret -> falls back cleanly to bottom-center
        let p_none = compute_pill_placement(None, 320, BAR_HEIGHT, 1920, 1080);
        assert!(!p_none.anchored_to_caret);
        assert_eq!(p_none.margin_bottom, 28);
    }

    #[test]
    fn renders_non_empty_hidpi_gboard_pill_and_hit_tests_slots() {
        let cands = vec![
            "tiếng".to_string(),
            "tieengs".to_string(),
            "tiến".to_string(),
        ];
        let (w, bounds) = compute_slot_layout(&cands);
        assert!((MIN_BAR_WIDTH..=MAX_BAR_WIDTH).contains(&w));
        assert_eq!(bounds.len(), 3);
        assert_eq!(hit_test_slot(&bounds, f64::from(bounds[0].0 + 4)), Some(0));
        assert_eq!(hit_test_slot(&bounds, f64::from(bounds[1].0 + 4)), Some(1));
        assert_eq!(hit_test_slot(&bounds, f64::from(bounds[2].0 + 4)), Some(2));
        assert_eq!(hit_test_slot(&bounds, 2.0), None);

        let pixels = render_gboard_pill_argb8888(&cands, &bounds, Some(1), w, BAR_HEIGHT, 2);
        assert_eq!(pixels.len(), ((w * 2) * (BAR_HEIGHT * 2) * 4) as usize);
        let fg_pixels = pixels
            .chunks_exact(4)
            .filter(|px| px[0] == 0xF4 && px[1] == 0xD6 && px[2] == 0xCD && px[3] == 0xFF)
            .count();
        assert!(
            fg_pixels > 200,
            "expected crisp HiDPI glyphs, got {fg_pixels}"
        );
    }
}
