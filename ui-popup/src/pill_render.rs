// SPDX-License-Identifier: MIT
//! Pure software layout, clamping, hit-testing, and ARGB8888 HiDPI pill rendering
//! for `dboard-popup`.

use crate::CaretRect;

pub const MIN_BAR_WIDTH: u32 = 220;
pub const MAX_BAR_WIDTH: u32 = 560;
pub const BAR_HEIGHT: u32 = 44;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PillPlacement {
    pub anchored_to_caret: bool,
    pub margin_top: i32,
    pub margin_bottom: i32,
    pub margin_left: i32,
}

/// Compute screen-clamped placement for a pill of size `(bar_w, bar_h)` inside
/// an output of `(out_w, out_h)`. Flips above the caret if below-caret would
/// overflow the bottom edge; falls back to top-center when `caret` is `None`
/// (avoiding bottom chat/agent input boxes in modern IDEs and apps).
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
            margin_top: 36,
            margin_bottom: 0,
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
/// All slot bounds are guaranteed to stay strictly within `14..=(total_w - 14)`.
pub fn compute_slot_layout(candidates: &[String]) -> (u32, Vec<(i32, i32)>) {
    let count = candidates.len().min(3);
    if count == 0 {
        return (MIN_BAR_WIDTH, Vec::new());
    }
    let gap = 10i32;
    let margin = 14i32;
    let mut raw_widths: Vec<i32> = candidates
        .iter()
        .take(count)
        .map(|cand| {
            let chars = cand.chars().count().min(18) as i32;
            (4 + chars) * 8 + 20
        })
        .collect();
    let total_gaps = gap * ((count - 1) as i32);
    let max_slots_w = (MAX_BAR_WIDTH as i32) - margin * 2 - total_gaps;
    let sum_w: i32 = raw_widths.iter().sum();
    if sum_w > max_slots_w && sum_w > 0 {
        for w in &mut raw_widths {
            *w = ((*w * max_slots_w) / sum_w).max(48);
        }
    }
    let mut x = margin;
    let max_x = (MAX_BAR_WIDTH as i32) - margin;
    let mut bounds = Vec::with_capacity(count);
    for (idx, w) in raw_widths.into_iter().enumerate() {
        let x_end = (x + w).min(max_x);
        bounds.push((x, x_end));
        x = x_end + if idx + 1 < count { gap } else { 0 };
    }
    let total_w = ((x + margin) as u32).clamp(MIN_BAR_WIDTH, MAX_BAR_WIDTH);
    (total_w, bounds)
}

pub fn hit_test_slot(slot_bounds: &[(i32, i32)], pointer_x: f64) -> Option<usize> {
    let px = pointer_x.round() as i32;
    slot_bounds
        .iter()
        .position(|&(start, end)| px >= start && px <= end)
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

#[inline]
fn rounded_rect_sdf(px: f32, py: f32, cx: f32, cy: f32, hx: f32, hy: f32, r: f32) -> f32 {
    let qx = (px - cx).abs() - (hx - r);
    let qy = (py - cy).abs() - (hy - r);
    let ox = qx.max(0.0);
    let oy = qy.max(0.0);
    (ox * ox + oy * oy).sqrt() + qx.max(qy).min(0.0) - r
}

/// Renders a polished Gboard-style floating pill (`ARGB8888`, premultiplied)
/// with anti-aliased rounded corners, HiDPI `scale` (`1..=4`), active/hover
/// slot highlight pills, and crisp 134-char Vietnamese Unicode glyphs.
pub fn render_gboard_pill_argb8888(
    candidates: &[String],
    slot_bounds: &[(i32, i32)],
    hovered_slot: Option<usize>,
    logical_w: u32,
    logical_h: u32,
    scale: u32,
) -> Vec<u8> {
    let scale = scale.clamp(1, 4);
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
            let border_alpha = (1.0 - (d + 0.75).abs()).clamp(0.0, 1.0) * cov;
            if border_alpha > 0.01 {
                blend_premul(&mut buf[off..off + 4], border, border_alpha);
            }
        }
    }

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

    for (idx, (cand, &(sx, ex))) in candidates
        .iter()
        .take(3)
        .zip(slot_bounds.iter())
        .enumerate()
    {
        let label = format!("[{}] {}", idx + 1, cand);
        crate::vector_font::draw_candidate_label(
            &mut buf,
            width,
            height,
            logical_w,
            logical_h,
            scale,
            sx,
            ex,
            &label,
            badge_fg,
            fg,
            blend_premul,
        );
    }

    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_caret_placement_with_clamping_and_bottom_flip() {
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

        let p_none = compute_pill_placement(None, 320, BAR_HEIGHT, 1920, 1080);
        assert!(!p_none.anchored_to_caret);
        assert_eq!(p_none.margin_top, 36);
        assert_eq!(p_none.margin_bottom, 0);
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

        let long_cands = vec![
            "abcdefghijklmnopqr".to_string(),
            "stuvwxyz0123456789".to_string(),
            "tiengvietdboard123".to_string(),
        ];
        let (lw, lbounds) = compute_slot_layout(&long_cands);
        assert!(lw <= MAX_BAR_WIDTH);
        assert!(lbounds
            .iter()
            .all(|&(s, e)| s >= 14 && e <= (lw as i32) - 14));

        for scale in [1u32, 2, 3, 4] {
            let pixels =
                render_gboard_pill_argb8888(&cands, &bounds, Some(1), w, BAR_HEIGHT, scale);
            assert_eq!(
                pixels.len(),
                ((w * scale) * (BAR_HEIGHT * scale) * 4) as usize,
                "SHM buffer byte length must match Wayland create_buffer dimensions at scale={scale}"
            );
            // Count crisp foreground glyph pixels (opaque bitmap `#CDD6F4` and
            // high-coverage anti-aliased TrueType strokes over the 0xF0-alpha
            // pill background), while strictly excluding bg/pill/border/badge
            // (all of which have red channel `px[2] <= 0x89`).
            let fg_pixels = pixels
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|px| px[0] >= 0xB0 && px[1] >= 0xA0 && px[2] >= 0x98 && px[3] >= 0xF0)
                .count();
            assert!(
                fg_pixels > 50,
                "expected crisp HiDPI glyphs at scale={scale}, got {fg_pixels}"
            );
        }
    }
}
