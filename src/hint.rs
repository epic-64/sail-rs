//! Badge-styled key/button labels for on-screen hints. A keyboard key renders
//! as a keycap square; a gamepad button renders as a coloured badge, so a
//! hint reads at a glance which device it means without the player having to
//! parse the word. The shape follows [`crate::device::gamepad`] rather than the
//! label text itself, so a hint built once from [`crate::device::hint`]'s
//! wording badges correctly whichever device last gave input.
//!
//! All badges on a line share one fixed height (the keycap height). A gamepad
//! face button (single-character label) is a circle of exactly that height; a
//! wider label (`LB/RB`, `D-pad`, an arrow pair) becomes a pill of the same
//! height rather than a bigger circle, so mixed hints sit on a steady baseline
//! and no badge balloons past its neighbours.
//!
//! Related buttons that act as one control (the left/right or up/down pair, an
//! `LB/RB` shoulder pair) share a single badge rather than one each — that
//! mirrors how the hint text already names them as one unit.

use crate::font::{draw_text, measure_text};
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
/// neutral steel badge since it isn't a face button with a standard colour.
fn pad_color(label: &str) -> Color {
    match label {
        "A" => Color::new(0.22, 0.62, 0.27, 1.0),
        "B" => Color::new(0.78, 0.18, 0.18, 1.0),
        "X" => Color::new(0.16, 0.42, 0.82, 1.0),
        "Y" => Color::new(0.86, 0.70, 0.12, 1.0),
        _ => Color::new(0.40, 0.40, 0.44, 1.0),
    }
}

/// Shared badge height at font size `fs`: the keycap height, which the gamepad
/// circle/pill reuses so both device styles occupy the same vertical space.
fn badge_h(fs: u16) -> f32 {
    fs as f32 * 1.25
}

fn badge_w(label: &str, fs: u16) -> f32 {
    if crate::device::gamepad() {
        let h = badge_h(fs);
        if label.chars().count() == 1 {
            return h; // face button: a circle, diameter = badge height
        }
        let d = measure_text(label, None, fs, 1.0);
        (d.width + px(12.0)).max(h)
    } else {
        let d = measure_text(label, None, fs, 1.0);
        (d.width + px(12.0)).max(px(22.0))
    }
}

/// Filled pill (stadium) spanning `x..x+w` at height `h`; degenerates to a
/// circle when `w == h`, so face buttons and wide labels share one code path.
fn fill_pill(x: f32, y: f32, w: f32, h: f32, color: Color) {
    let r = h / 2.0;
    draw_circle(x + r, y + r, r, color);
    draw_circle(x + w - r, y + r, r, color);
    if w > h {
        draw_rectangle(x + r, y, w - h, h, color);
    }
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
/// circle/pill (gamepad). `color` tints the plain-text parts only; a badge
/// keeps its own fixed palette so it reads the same regardless of context.
/// Returns the x just past the last part.
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
                let h = badge_h(fs);
                let ry = mid_y - h / 2.0;
                if gamepad {
                    let o = px(1.0);
                    fill_pill(cx - o, ry - o, w + 2.0 * o, h + 2.0 * o, Color::new(0.0, 0.0, 0.0, 0.4));
                    fill_pill(cx, ry, w, h, pad_color(label));
                    draw_text(label, tx, y, fs as f32, WHITE);
                } else {
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
