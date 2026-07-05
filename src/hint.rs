//! Badge-styled key/button labels for on-screen hints. A keyboard key renders
//! as a keycap square; a gamepad button renders as a coloured circle — so a
//! hint reads at a glance which device it means without the player having to
//! parse the word. The shape follows [`crate::device::gamepad`] rather than the
//! label text itself, so a hint built once from [`crate::device::hint`]'s
//! wording badges correctly whichever device last gave input.
//!
//! Related buttons that act as one control (the left/right or up/down pair, an
//! `LB/RB` shoulder pair) share a single badge rather than one each — that
//! mirrors how the hint text already names them as one unit.

use crate::ui::px;
use macroquad::prelude::*;

/// One piece of a hint line, built with [`text`] or [`key`].
pub enum Part<'a> {
    Text(&'a str),
    Key(&'a str),
}

pub fn text(s: &str) -> Part<'_> {
    Part::Text(s)
}
pub fn key(s: &str) -> Part<'_> {
    Part::Key(s)
}

/// Xbox-style face-button tint (A/B/X/Y = South/East/West/North = green/red/
/// blue/yellow, per `pad.rs`); anything else (LB/RB/RT, D-pad, arrows) gets a
/// neutral steel circle since it isn't a face button with a standard colour.
fn pad_color(label: &str) -> Color {
    match label {
        "A" => Color::new(0.22, 0.62, 0.27, 1.0),
        "B" => Color::new(0.78, 0.18, 0.18, 1.0),
        "X" => Color::new(0.16, 0.42, 0.82, 1.0),
        "Y" => Color::new(0.86, 0.70, 0.12, 1.0),
        _ => Color::new(0.40, 0.40, 0.44, 1.0),
    }
}

fn badge_w(label: &str, fs: u16) -> f32 {
    let d = measure_text(label, None, fs, 1.0);
    (d.width + px(12.0)).max(px(22.0))
}

fn part_w(part: &Part, fs: u16) -> f32 {
    match part {
        Part::Text(s) => measure_text(s, None, fs, 1.0).width,
        Part::Key(label) => badge_w(label, fs),
    }
}

/// Total width of a hint line at font size `fs`, badges included — for
/// centring or right-aligning a line before calling [`draw`].
pub fn measure(parts: &[Part], fs: u16) -> f32 {
    parts.iter().map(|p| part_w(p, fs)).sum()
}

/// Draw a hint line left to right from `(x, y)` (`y` the text baseline),
/// badging each [`Part::Key`] as a keycap square (keyboard) or a coloured
/// circle (gamepad). `color` tints the plain-text parts only — a badge keeps
/// its own fixed palette so it reads the same regardless of context. Returns
/// the x just past the last part.
pub fn draw(parts: &[Part], x: f32, y: f32, fs: u16, color: Color) -> f32 {
    let gamepad = crate::device::gamepad();
    let mut cx = x;
    for part in parts {
        match part {
            Part::Text(s) => {
                draw_text(s, cx, y, fs as f32, color);
                cx += measure_text(s, None, fs, 1.0).width;
            }
            Part::Key(label) => {
                let w = badge_w(label, fs);
                let d = measure_text(label, None, fs, 1.0);
                let mid_y = y - fs as f32 * 0.36; // roughly the cap-height centre
                let tx = cx + (w - d.width) / 2.0;
                if gamepad {
                    let r = w * 0.5;
                    draw_circle(cx + r, mid_y, r, pad_color(label));
                    draw_circle_lines(cx + r, mid_y, r, px(1.0), Color::new(0.0, 0.0, 0.0, 0.4));
                    draw_text(label, tx, y, fs as f32, WHITE);
                } else {
                    let h = fs as f32 * 1.25;
                    let ry = mid_y - h / 2.0;
                    draw_rectangle(cx, ry, w, h, Color::new(0.97, 0.94, 0.86, 0.95));
                    draw_rectangle_lines(cx, ry, w, h, px(1.2), Color::new(0.35, 0.27, 0.15, 0.9));
                    draw_text(label, tx, y, fs as f32, Color::new(0.16, 0.10, 0.05, 1.0));
                }
                cx += w;
            }
        }
    }
    cx
}
