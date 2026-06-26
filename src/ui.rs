//! Shared primitives for the game's parchment UIs — the port board ([`crate::port_view`])
//! and the captain's log ([`crate::captains_log`]) — so the two draw on one type scale
//! and one ink/parchment palette instead of each carrying its own copy.
//!
//! Anything genuinely specific to one surface (the board's column splits, the log's
//! warning inks, each surface's button styling) stays in that surface's own module;
//! only what was byte-for-byte duplicated lives here.

use macroquad::prelude::*;

// --- UI scale -----------------------------------------------------------------
/// The parchment UIs' scale factor for the current screen, so they stay legible
/// from a phone up to a 4K display. Driven by the screen's short edge against a
/// 720 px design baseline: `high_dpi` reports *physical* pixels, so on a 4K screen
/// an unscaled board would be a tiny island of 15 px text. Clamped to a sane band.
/// Every type size and pixel step across the parchment UIs is multiplied by this.
pub fn scale() -> f32 {
    (screen_width().min(screen_height()) / 720.0).clamp(0.85, 3.0)
}

/// Scale a design-space pixel length (authored at scale 1.0) to the screen.
pub fn px(v: f32) -> f32 {
    v * scale()
}

/// Scale a design-space type size to the screen (never below 1 px).
fn fs(base: f32) -> u16 {
    (base * scale()).round().max(1.0) as u16
}

// --- Type scale — one tight ladder, used across the parchment UIs --------------
// Design sizes (px at scale 1.0); the live size is `fs(base)` for the screen.
pub fn fs_title() -> u16 {
    fs(24.0) // titles: the port name, a log spread's heading
}
pub fn fs_heading() -> u16 {
    fs(15.0) // section / board headers (display face) + the purse
}
pub fn fs_body() -> u16 {
    fs(14.0) // data lines, list rows, tab labels
}
pub fn fs_small() -> u16 {
    fs(12.0) // eyebrows, column labels, captions, hints
}
pub fn fs_chip() -> u16 {
    fs(13.0) // button / chip labels
}

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
/// Alarm ink — a full hold / battered hull (the laden-fill bar when it tops out).
/// Shared so the captain's log and the port header read the same red.
pub fn alarm_ink() -> Color {
    Color::new(150.0 / 255.0, 38.0 / 255.0, 24.0 / 255.0, 1.0)
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
