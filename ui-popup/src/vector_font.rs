// SPDX-License-Identifier: MIT
//! Pure-Rust anti-aliased TrueType renderer (`swash`) for `dboard-popup` with
//! automatic fallback to the embedded 8×16 Vietnamese bitmap font (`font.rs`).

use std::fs;
use std::sync::OnceLock;

use swash::scale::{Render, ScaleContext, Source};
use swash::zeno::Format;
use swash::FontRef;

const FONT_CANDIDATES: &[&str] = &[
    "/usr/share/fonts/truetype/noto/NotoSans-Regular.ttf",
    "/usr/share/fonts/noto/NotoSans-Regular.ttf",
    "/usr/share/fonts/truetype/roboto/unhinted/RobotoTTF/Roboto-Regular.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    "/usr/share/fonts/TTF/DejaVuSans.ttf",
    "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
    "/usr/share/fonts/truetype/freefont/FreeSans.ttf",
];

fn system_font_bytes() -> Option<&'static [u8]> {
    static FONT_DATA: OnceLock<Option<Vec<u8>>> = OnceLock::new();
    FONT_DATA
        .get_or_init(|| {
            for path in FONT_CANDIDATES {
                if let Ok(bytes) = fs::read(path) {
                    if let Some(font) = FontRef::from_index(&bytes, 0) {
                        let cmap = font.charmap();
                        // Ensure the font covers Vietnamese diacritics (`ế`, `ư`, `đ`).
                        if cmap.map('ế') != 0 && cmap.map('ư') != 0 && cmap.map('đ') != 0 {
                            return Some(bytes);
                        }
                    }
                }
            }
            None
        })
        .as_deref()
}

#[allow(clippy::too_many_arguments)]
pub fn draw_candidate_label(
    buf: &mut [u8],
    width: u32,
    height: u32,
    logical_w: u32,
    logical_h: u32,
    scale: u32,
    slot_x: i32,
    slot_end_x: i32,
    label: &str,
    badge_fg: [u8; 4],
    fg: [u8; 4],
    blend: impl Fn(&mut [u8], [u8; 4], f32),
) {
    if let Some(font) = system_font_bytes().and_then(|b| FontRef::from_index(b, 0)) {
        draw_vector_label(
            font, buf, width, height, logical_w, logical_h, scale, slot_x, slot_end_x, label,
            badge_fg, fg, blend,
        );
    } else {
        draw_bitmap_label(
            buf, width, height, logical_w, logical_h, scale, slot_x, slot_end_x, label, badge_fg,
            fg,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_vector_label(
    font: FontRef<'_>,
    buf: &mut [u8],
    width: u32,
    height: u32,
    logical_w: u32,
    logical_h: u32,
    scale: u32,
    slot_x: i32,
    slot_end_x: i32,
    label: &str,
    badge_fg: [u8; 4],
    fg: [u8; 4],
    blend: impl Fn(&mut [u8], [u8; 4], f32),
) {
    let px_size = 15.0 * (scale as f32);
    let metrics = font.metrics(&[]).scale(px_size);
    let text_h = metrics.ascent + metrics.descent;
    let baseline_y = ((logical_h * scale) as f32 - text_h) * 0.5 + metrics.ascent;
    let slot_clip = (slot_end_x.max(slot_x + 8) as u32)
        .saturating_sub(4)
        .min(logical_w.saturating_sub(8));
    let max_x = (slot_clip * scale) as f32;
    let mut pen_x = ((slot_x + 8).max(8) as u32 * scale) as f32;

    let cmap = font.charmap();
    let glyph_metrics = font.glyph_metrics(&[]).scale(px_size);
    thread_local! {
        static SCALE_CTX: std::cell::RefCell<ScaleContext> =
            std::cell::RefCell::new(ScaleContext::new());
    }
    SCALE_CTX.with(|cell| {
        let mut ctx = cell.borrow_mut();
        let mut scaler = ctx.builder(font).size(px_size).hint(true).build();

        for (ci, ch) in label.chars().enumerate() {
            let gid = cmap.map(ch);
            let advance = glyph_metrics.advance_width(gid);
            if pen_x + advance > max_x {
                break;
            }
            let color = if ci < 3 { badge_fg } else { fg };
            if let Some(img) = Render::new(&[Source::Outline])
                .format(Format::Alpha)
                .render(&mut scaler, gid)
            {
                let gx0 = pen_x.round() as i32 + img.placement.left;
                let gy0 = baseline_y.round() as i32 - img.placement.top;
                let gw = img.placement.width as usize;
                let gh = img.placement.height as usize;

                for row in 0..gh {
                    let py = gy0 + row as i32;
                    if py < 0 || py >= height as i32 {
                        continue;
                    }
                    for col in 0..gw {
                        let cov = img.data[row * gw + col];
                        if cov == 0 {
                            continue;
                        }
                        let px = gx0 + col as i32;
                        if px < 0 || px >= width as i32 {
                            continue;
                        }
                        let off = ((py as u32 * width + px as u32) * 4) as usize;
                        let alpha = (cov as f32) / 255.0;
                        blend(&mut buf[off..off + 4], color, alpha);
                    }
                }
            }
            pen_x += advance;
        }
    });
}

#[allow(clippy::too_many_arguments)]
fn draw_bitmap_label(
    buf: &mut [u8],
    width: u32,
    height: u32,
    logical_w: u32,
    logical_h: u32,
    scale: u32,
    slot_x: i32,
    slot_end_x: i32,
    label: &str,
    badge_fg: [u8; 4],
    fg: [u8; 4],
) {
    let base_y = (logical_h.saturating_sub(crate::font::CELL_H)) / 2;
    let slot_clip = (slot_end_x.max(slot_x + 8) as u32)
        .saturating_sub(4)
        .min(logical_w.saturating_sub(8));
    let mut pen_x = (slot_x + 8).max(8) as u32;
    for (ci, ch) in label.chars().enumerate() {
        if pen_x + crate::font::CELL_W > slot_clip {
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
