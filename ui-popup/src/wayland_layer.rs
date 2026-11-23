// SPDX-License-Identifier: MIT
//! Real Wayland `zwlr_layer_shell_v1` floating candidate bar renderer (`wl_shm` ARGB8888).
//!
//! Guarantees:
//! - Always sets `KeyboardInteractivity::None` (NEVER steals keyboard focus).
//! - Renders a crisp dark-mode floating pill at the bottom-center of the screen
//!   with zero GTK/Qt/Cairo dependencies.
//! - Attaches `None` buffer on `hide()` so the surface unmaps immediately on
//!   password fields, bypass apps, focus loss, or word commit.

use std::io::{self, Write};
use std::os::fd::{AsFd, AsRawFd, FromRawFd, OwnedFd, RawFd};

use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_registry, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

pub const BAR_WIDTH: u32 = 520;
pub const BAR_HEIGHT: u32 = 44;

#[derive(Default)]
struct LayerState {
    compositor: Option<wl_compositor::WlCompositor>,
    shm: Option<wl_shm::WlShm>,
    layer_shell: Option<zwlr_layer_shell_v1::ZwlrLayerShellV1>,
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
                _ => {}
            }
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
}

impl WaylandLayerBar {
    pub fn connect() -> io::Result<Self> {
        let connection = Connection::connect_to_env().map_err(io::Error::other)?;
        let mut queue = connection.new_event_queue::<LayerState>();
        let qh = queue.handle();
        let _registry = connection.display().get_registry(&qh, ());
        let mut state = LayerState::default();
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
        layer_surface.set_size(BAR_WIDTH, BAR_HEIGHT);
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
        })
    }

    pub fn connection_fd(&self) -> RawFd {
        self.connection.backend().poll_fd().as_raw_fd()
    }

    pub fn is_closed(&self) -> bool {
        self.state.closed
    }

    pub fn dispatch_pending(&mut self) -> io::Result<()> {
        let _ = self.queue.flush();
        if let Some(guard) = self.queue.prepare_read() {
            let _ = guard.read();
        }
        self.queue
            .dispatch_pending(&mut self.state)
            .map_err(io::Error::other)?;
        Ok(())
    }

    pub fn show(&mut self, strip_text: &str) -> io::Result<()> {
        let pixels = render_pill_argb8888(strip_text, BAR_WIDTH, BAR_HEIGHT);
        let fd = create_shm_memfd(&pixels)?;
        let qh = self.queue.handle();
        let pool = self
            .shm
            .create_pool(fd.as_fd(), pixels.len() as i32, &qh, ());
        let stride = (BAR_WIDTH * 4) as i32;
        let buffer = pool.create_buffer(
            0,
            BAR_WIDTH as i32,
            BAR_HEIGHT as i32,
            stride,
            wl_shm::Format::Argb8888,
            &qh,
            (),
        );
        self.surface.attach(Some(&buffer), 0, 0);
        self.surface
            .damage_buffer(0, 0, BAR_WIDTH as i32, BAR_HEIGHT as i32);
        self.surface.commit();
        self.connection.flush().map_err(io::Error::other)?;

        if let Some((old_buf, old_pool, _old_fd)) = self.active_buffer.replace((buffer, pool, fd)) {
            old_buf.destroy();
            old_pool.destroy();
        }
        Ok(())
    }

    pub fn hide(&mut self) -> io::Result<()> {
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

/// Renders a dark-mode rounded pill bar (`ARGB8888`, premultiplied) with full
/// Unicode Vietnamese 8x16 glyphs (including all 134 accented characters,
/// lowercase/uppercase, and accent-colored `[1]`/`[2]`/`[3]` badges).
pub fn render_pill_argb8888(text: &str, width: u32, height: u32) -> Vec<u8> {
    let mut buf = vec![0u8; (width * height * 4) as usize];
    let bg: [u8; 4] = [0x2E, 0x1E, 0x1E, 0xEB]; // ARGB8888 little-endian BGRA: #1E1E2E @ 92%
    let border: [u8; 4] = [0xFA, 0xB4, 0x89, 0xFF]; // #89B4FA accent
    let fg: [u8; 4] = [0xF4, 0xD6, 0xCD, 0xFF]; // #CDD6F4 light text
    let badge_fg: [u8; 4] = [0xFA, 0xB4, 0x89, 0xFF]; // #89B4FA index badge
    let sep_fg: [u8; 4] = [0x86, 0x70, 0x6C, 0xFF]; // #6C7086 subtle separator
    let radius: i32 = 12;

    let set_px = |buf: &mut [u8], x: u32, y: u32, px: [u8; 4]| {
        if x < width && y < height {
            let off = ((y * width + x) * 4) as usize;
            buf[off..off + 4].copy_from_slice(&px);
        }
    };

    for y in 0..height {
        for x in 0..width {
            let dx = if (x as i32) < radius {
                radius - x as i32
            } else if (x as i32) >= width as i32 - radius {
                x as i32 - (width as i32 - radius - 1)
            } else {
                0
            };
            let dy = if (y as i32) < radius {
                radius - y as i32
            } else if (y as i32) >= height as i32 - radius {
                y as i32 - (height as i32 - radius - 1)
            } else {
                0
            };
            let dist_sq = dx * dx + dy * dy;
            if dist_sq <= radius * radius {
                let is_edge = x == 0
                    || y == 0
                    || x == width - 1
                    || y == height - 1
                    || dist_sq >= (radius - 1) * (radius - 1);
                set_px(&mut buf, x, y, if is_edge { border } else { bg });
            }
        }
    }

    // Draw scaled 2x 8x16 Unicode Vietnamese glyphs centered vertically
    let scale = 2u32;
    let char_advance = (crate::font::CELL_W + 1) * scale;
    let start_y = height.saturating_sub(crate::font::CELL_H * scale) / 2;
    let mut cursor_x = 14u32;
    let mut in_badge = false;

    for ch in text.chars() {
        if cursor_x + crate::font::CELL_W * scale > width.saturating_sub(10) {
            break;
        }
        if ch == '[' {
            in_badge = true;
        }
        let color = if ch == '|' {
            sep_fg
        } else if in_badge {
            badge_fg
        } else {
            fg
        };
        let glyph = crate::font::glyph_8x16(ch);
        for (row, &bits) in glyph.iter().enumerate() {
            if bits == 0 {
                continue;
            }
            for col in 0..crate::font::CELL_W {
                if (bits >> (7 - col)) & 1 != 0 {
                    for sy in 0..scale {
                        for sx in 0..scale {
                            set_px(
                                &mut buf,
                                cursor_x + col * scale + sx,
                                start_y + (row as u32) * scale + sy,
                                color,
                            );
                        }
                    }
                }
            }
        }
        if ch == ']' {
            in_badge = false;
        }
        cursor_x += char_advance;
    }

    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_non_empty_argb8888_pill_surface_with_vietnamese_diacritics() {
        let vn_pixels = render_pill_argb8888("[1] tiếng | [2] tieengs", BAR_WIDTH, BAR_HEIGHT);
        let ascii_pixels = render_pill_argb8888("[1] tieng | [2] tieengs", BAR_WIDTH, BAR_HEIGHT);
        assert_eq!(vn_pixels.len(), (BAR_WIDTH * BAR_HEIGHT * 4) as usize);
        let non_zero = vn_pixels.chunks_exact(4).filter(|p| p[3] != 0).count();
        assert!(non_zero > 1000, "rendered pill should have opaque pixels");
        assert_ne!(
            vn_pixels, ascii_pixels,
            "Vietnamese diacritics must render distinctly from plain ASCII"
        );
    }
}
