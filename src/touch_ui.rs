//! On-screen touch controls — the layouts the [`crate::touch`] pointer layer
//! hit-tests, and their drawing. Two surfaces:
//!
//! - the **sailing HUD** ([`sail_hud`] / [`draw_sail_hud`]): a steering wheel
//!   (lower-left), sail up/down (lower-right), dock (centre), and a stack of
//!   pause / log / look-astern on the right;
//! - the **menu nav cluster** ([`nav_cluster`] / [`draw_nav_cluster`]): a d-pad
//!   plus confirm / back, which emit the same arrow / Enter / Esc verbs the
//!   keyboard-driven pause menu and captain's log already consume. (The port
//!   board is tapped directly — see [`crate::port_view`] — and shows only the
//!   cast-off [`back_button`].)
//!
//! Each rect is derived from the screen size so the controls track rotation, and
//! the hit-test (in `touch.rs`) and the draw here read the *same* layout fns —
//! geometry lives in one place. Everything is translucent so it sits lightly over
//! the seascape / parchment. Screen-space maths use macroquad's glam `Vec2`/`Rect`
//! (pixels), not the game's `geometry::Vec2`.

use macroquad::prelude::*;

// --- Palette / sizing -------------------------------------------------------
fn fill() -> Color {
    Color::new(0.10, 0.07, 0.03, 0.30)
}
fn fill_hot() -> Color {
    Color::new(0.95, 0.84, 0.45, 0.34) // a control that's live / available
}
fn edge() -> Color {
    Color::new(0.96, 0.93, 0.84, 0.80)
}
fn glyph() -> Color {
    Color::new(0.98, 0.95, 0.86, 0.92)
}
fn glyph_dim() -> Color {
    Color::new(0.98, 0.95, 0.86, 0.30)
}
const EDGE_W: f32 = 2.0;

/// A control button big enough for a thumb, scaled to the screen's short edge.
fn btn(h: f32) -> f32 {
    (h * 0.11).clamp(54.0, 96.0)
}
/// The page margin the controls keep off the screen edge.
fn margin(h: f32) -> f32 {
    (h * 0.035).clamp(14.0, 32.0)
}

// --- A rounded, outlined button + helpers -----------------------------------
fn panel(r: Rect, hot: bool) {
    let bg = if hot { fill_hot() } else { fill() };
    draw_rectangle(r.x, r.y, r.w, r.h, bg);
    draw_rectangle_lines(r.x, r.y, r.w, r.h, EDGE_W, edge());
}

fn center(r: Rect) -> Vec2 {
    vec2(r.x + r.w * 0.5, r.y + r.h * 0.5)
}

/// Centre a short label in a button (sans face — the default each frame).
fn label(r: Rect, text: &str, col: Color) {
    let fs = (r.h * 0.32).clamp(12.0, 24.0) as u16;
    let d = measure_text(text, None, fs, 1.0);
    let c = center(r);
    draw_text(text, c.x - d.width * 0.5, c.y + d.height * 0.5, fs as f32, col);
}

fn tri_up(r: Rect, col: Color) {
    let c = center(r);
    let s = r.w * 0.22;
    draw_triangle(vec2(c.x, c.y - s), vec2(c.x - s, c.y + s), vec2(c.x + s, c.y + s), col);
}
fn tri_down(r: Rect, col: Color) {
    let c = center(r);
    let s = r.w * 0.22;
    draw_triangle(vec2(c.x, c.y + s), vec2(c.x - s, c.y - s), vec2(c.x + s, c.y - s), col);
}
fn tri_left(r: Rect, col: Color) {
    let c = center(r);
    let s = r.w * 0.22;
    draw_triangle(vec2(c.x - s, c.y), vec2(c.x + s, c.y - s), vec2(c.x + s, c.y + s), col);
}
fn tri_right(r: Rect, col: Color) {
    let c = center(r);
    let s = r.w * 0.22;
    draw_triangle(vec2(c.x + s, c.y), vec2(c.x - s, c.y - s), vec2(c.x - s, c.y + s), col);
}

// =====================================================================
// Sailing HUD
// =====================================================================

/// The sailing HUD's hit-rects. `dock` is only meaningful (and drawn) when a port
/// is in range; the rest are always live while sailing.
pub struct SailHud {
    pub wheel: Rect,
    pub sail_up: Rect,
    pub sail_down: Rect,
    pub dock: Rect,
    pub log: Rect,
    pub astern: Rect,
    pub pause: Rect,
}

/// Lay out the sailing HUD for a `w`×`h` screen (landscape). The wheel sits under
/// the left thumb, the sail buttons under the right; pause / log / astern stack on
/// the right edge below the minimap; dock is centred along the bottom.
pub fn sail_hud(w: f32, h: f32) -> SailHud {
    let mg = margin(h);
    let b = btn(h);
    let gap = b * 0.28;

    // Steering wheel, lower-left (square hit area; drawn as a ring within).
    let wheel_d = (h * 0.30).clamp(130.0, 240.0);
    let wheel = Rect::new(mg, h - wheel_d - mg, wheel_d, wheel_d);

    // Sail up/down, lower-right, stacked (up above down).
    let sx = w - b - mg;
    let sail_down = Rect::new(sx, h - b - mg, b, b);
    let sail_up = Rect::new(sx, sail_down.y - b - gap, b, b);

    // Dock, centred along the bottom (thumb-reachable when a port comes up).
    let dock = Rect::new(w * 0.5 - b * 0.5, h - b - mg, b, b);

    // Right-edge stack below the corner minimap (which is `~(h*0.24)` tall at the
    // top-right; keep clear of it).
    let map_h = (h * 0.24).clamp(140.0, 200.0);
    let rx = w - b - mg;
    let y0 = mg + map_h + gap;
    let pause = Rect::new(rx, y0, b, b);
    let log = Rect::new(rx, y0 + b + gap, b, b);
    let astern = Rect::new(rx, y0 + 2.0 * (b + gap), b, b);

    SailHud { wheel, sail_up, sail_down, dock, log, astern, pause }
}

/// Draw the sailing HUD. `turn` (−1..1) tilts the wheel's spoke for feedback,
/// `sail_mode`/`sail_max` dim the sail arrows at the end stops, `dockable` shows
/// the dock button, and `astern_held` lights the look-astern button while held.
#[allow(clippy::too_many_arguments)]
pub fn draw_sail_hud(
    hud: &SailHud,
    turn: f32,
    sail_mode: usize,
    sail_max: usize,
    dockable: bool,
    astern_held: bool,
) {
    // --- Steering wheel: a ring with a spoke that swings with the rudder ---
    let c = center(hud.wheel);
    let rad = hud.wheel.w * 0.5;
    draw_circle(c.x, c.y, rad, fill());
    draw_circle_lines(c.x, c.y, rad, EDGE_W, edge());
    draw_circle_lines(c.x, c.y, rad * 0.30, EDGE_W, edge()); // hub
    // The spoke leans by the rudder demand (full lock ≈ 60° off vertical).
    let ang = turn * 1.05; // rad
    let tip = vec2(c.x + ang.sin() * rad * 0.92, c.y - ang.cos() * rad * 0.92);
    draw_line(c.x, c.y, tip.x, tip.y, EDGE_W + 1.0, glyph());
    draw_circle(tip.x, tip.y, rad * 0.10, glyph());

    // --- Sails: up / down arrows, dimmed at the stops ---
    panel(hud.sail_up, false);
    tri_up(hud.sail_up, if sail_mode < sail_max { glyph() } else { glyph_dim() });
    panel(hud.sail_down, false);
    tri_down(hud.sail_down, if sail_mode > 0 { glyph() } else { glyph_dim() });

    // --- Pause / log / astern stack ---
    panel(hud.pause, false);
    {
        // two bars
        let c = center(hud.pause);
        let bw = hud.pause.w * 0.10;
        let bh = hud.pause.h * 0.34;
        draw_rectangle(c.x - bw * 1.8, c.y - bh * 0.5, bw, bh, glyph());
        draw_rectangle(c.x + bw * 0.8, c.y - bh * 0.5, bw, bh, glyph());
    }
    panel(hud.log, false);
    label(hud.log, "LOG", glyph());
    panel(hud.astern, astern_held);
    label(hud.astern, "AFT", glyph());

    // --- Dock, only when a port is in range ---
    if dockable {
        panel(hud.dock, true);
        label(hud.dock, "DOCK", glyph());
    }
}

// =====================================================================
// Menu nav cluster
// =====================================================================

/// The menu nav cluster's hit-rects: a d-pad (lower-left) plus confirm / back
/// (lower-right). Used by the pause menu and the captain's log (the port board is
/// tapped directly instead).
pub struct NavRects {
    pub up: Rect,
    pub down: Rect,
    pub left: Rect,
    pub right: Rect,
    pub confirm: Rect,
    pub back: Rect,
}

/// Lay out the nav cluster for a `w`×`h` screen.
pub fn nav_cluster(w: f32, h: f32) -> NavRects {
    let mg = margin(h);
    let b = btn(h);

    // D-pad plus, lower-left.
    let cx = mg + 1.5 * b;
    let cy = h - mg - 1.5 * b;
    let up = Rect::new(cx - 0.5 * b, cy - 1.5 * b, b, b);
    let down = Rect::new(cx - 0.5 * b, cy + 0.5 * b, b, b);
    let left = Rect::new(cx - 1.5 * b, cy - 0.5 * b, b, b);
    let right = Rect::new(cx + 0.5 * b, cy - 0.5 * b, b, b);

    // Confirm / back, lower-right.
    let gap = b * 0.3;
    let confirm = Rect::new(w - mg - b, h - mg - b, b, b);
    let back = Rect::new(confirm.x - b - gap, h - mg - b, b, b);

    NavRects { up, down, left, right, confirm, back }
}

fn check(r: Rect) {
    let c = center(r);
    let s = r.w * 0.22;
    draw_line(c.x - s, c.y, c.x - s * 0.2, c.y + s, EDGE_W + 1.0, glyph());
    draw_line(c.x - s * 0.2, c.y + s, c.x + s, c.y - s, EDGE_W + 1.0, glyph());
}
fn cross(r: Rect) {
    let c = center(r);
    let s = r.w * 0.2;
    draw_line(c.x - s, c.y - s, c.x + s, c.y + s, EDGE_W + 1.0, glyph());
    draw_line(c.x - s, c.y + s, c.x + s, c.y - s, EDGE_W + 1.0, glyph());
}

/// Draw the nav cluster over an open board / menu.
pub fn draw_nav_cluster(n: &NavRects) {
    panel(n.up, false);
    tri_up(n.up, glyph());
    panel(n.down, false);
    tri_down(n.down, glyph());
    panel(n.left, false);
    tri_left(n.left, glyph());
    panel(n.right, false);
    tri_right(n.right, glyph());

    panel(n.confirm, true);
    check(n.confirm);
    panel(n.back, false);
    cross(n.back);
}

/// The lone "cast off" (✕) button the port board shows — its rows and chips are
/// tapped directly, so it needs no d-pad. Shares the nav cluster's back corner.
pub fn back_button(w: f32, h: f32) -> Rect {
    nav_cluster(w, h).back
}

/// Draw the board's cast-off button.
pub fn draw_back_button(r: Rect) {
    panel(r, false);
    cross(r);
}
