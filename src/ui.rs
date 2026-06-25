//! Shared primitives for the game's parchment UIs — the port board ([`crate::port_view`])
//! and the captain's log ([`crate::captains_log`]) — so the two draw on one type scale
//! and one ink/parchment palette instead of each carrying its own copy.
//!
//! Anything genuinely specific to one surface (the board's column splits, the log's
//! warning inks, each surface's button styling) stays in that surface's own module;
//! only what was byte-for-byte duplicated lives here.

use macroquad::prelude::*;

// --- Type scale (px) — one tight ladder, used across the parchment UIs ---------
pub const FS_TITLE: u16 = 26; // titles: the port name, a log spread's heading
pub const FS_HEADING: u16 = 16; // section / board headers (display face) + the purse
pub const FS_BODY: u16 = 15; // data lines, list rows, tab labels
pub const FS_SMALL: u16 = 13; // eyebrows, column labels, captions, hints
pub const FS_CHIP: u16 = 14; // button / chip labels

/// A text line's height is its font size times this — the list/row step.
pub const LINE_RATIO: f32 = 1.55;
pub fn line_h(fs: u16) -> f32 {
    (fs as f32 * LINE_RATIO).round()
}

// --- The ink / parchment palette ----------------------------------------------
pub fn ink() -> Color {
    Color::new(79.0 / 255.0, 47.0 / 255.0, 23.0 / 255.0, 1.0)
}
pub fn dim_ink() -> Color {
    Color::new(79.0 / 255.0, 47.0 / 255.0, 23.0 / 255.0, 0.62)
}
pub fn parchment() -> Color {
    Color::new(230.0 / 255.0, 216.0 / 255.0, 176.0 / 255.0, 1.0)
}
pub fn parchment_edge() -> Color {
    Color::new(120.0 / 255.0, 90.0 / 255.0, 55.0 / 255.0, 0.9)
}

/// A short distance readout: kilometres past 1 km, metres below it.
/// (`SailingView.formatDist` in the original.)
pub fn format_dist(m: f32) -> String {
    if m >= 1000.0 {
        format!("{:.1} km", m / 1000.0)
    } else {
        format!("{} m", m.round() as i32)
    }
}
