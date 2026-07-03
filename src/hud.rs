//! The sailing HUD's keyboard-mode overlays: the bottom-left keybind reminders
//! and the top-left status readout (purse, speed, wind, hull) with its debuff
//! badges. Split out of `main` so the loop's HUD section reads as a couple of
//! calls rather than inline drawing.

use macroquad::prelude::*;

use crate::game_state::{hull, upgrades, GameState};
use crate::geometry::{compass, wrap_angle};
use crate::sailing::{self, Kinematics, Wind};
use crate::ui::{self, px};

/// Faint keybind reminders in the bottom-left while sailing. Only shown in
/// keyboard mode (the touch HUD carries its own glyphs), so a captain at the
/// helm always sees how to reach the log, furl sail, steer and dock. The
/// dock hint only appears when there's a harbour within reach.
pub fn keybind_hints(dockable: bool, extra: &[(&str, &str)], h: f32) {
    // (key, action) top to bottom; the log sits first as the headline hint.
    let mut hints: Vec<(&str, &str)> = vec![
        ("L", "Captain's Log"),
        ("G", "Guide"),
        ("Esc", "Pause"),
        ("\u{2191}\u{2193}", "Sail"),
        ("\u{2190}\u{2192}", "Steer"),
        ("C", "Look astern"),
        ("H", "Hide HUD"),
    ];
    // Any tavern wares the captain has bought (the world map's M, the active wares'
    // number keys), so their shortcuts are reminded only once they're earned.
    hints.extend_from_slice(extra);
    if dockable {
        hints.push(("Space", "Dock"));
    }

    let fs = ui::fs_small();
    let step = ui::line_h(fs);
    let margin = px(14.0);
    let key_w = px(38.0); // gutter the action text clears, so keys/actions align
    let key_col = Color::new(0.98, 0.95, 0.86, 0.92);
    let act_col = Color::new(0.96, 0.92, 0.80, 0.62);
    let shadow = Color::new(0.0, 0.0, 0.0, 0.5);

    // Stack upward from the bottom edge so the list grows from a fixed baseline.
    let mut y = h - margin;
    for (key, action) in hints.iter().rev() {
        let x = margin;
        // A faint drop shadow keeps the text legible over bright water or foam.
        draw_text(key, x + 1.0, y + 1.0, fs as f32, shadow);
        draw_text(key, x, y, fs as f32, key_col);
        draw_text(action, x + key_w + 1.0, y + 1.0, fs as f32, shadow);
        draw_text(action, x + key_w, y, fs as f32, act_col);
        y -= step;
    }
}

/// The top-left status readout: a coin and the purse, then speed, the wind's
/// quarter, the point of sail and the hull's condition, on one dot-separated
/// row; below it, warning badges for any handling debuff in force (a battered
/// hull, an overladen hold). Wind is shown by the quarter it blows *from* (the
/// seaman's convention). `burst_kn` is the Dolphin's Draught's extra way over
/// the ground, riding on top of the hull's own speed in the readout.
pub fn status_readout(gs: &GameState, kin: &Kinematics, wind: Wind, burst_kn: f32) {
    let knots = kin.speed() / sailing::KNOT + burst_kn;
    let wind_from = compass(wrap_angle(wind.toward_rad + std::f32::consts::PI));
    let point = wind.point_of_sail(kin.heading_rad).label();
    let hull_pct = (hull::fraction(gs) * 100.0).round() as i32;
    // Everything in one row, at one font size, dot-separated: a coin icon and
    // the purse, then speed · wind quarter · point of sail.
    let fs = px(16.0);
    let baseline = px(26.0);
    // Coin icon, vertically centred on the text's cap height.
    let r = px(7.0);
    let cx = px(16.0) + r;
    let cy = baseline - fs * 0.34;
    let rim = Color::new(0.78, 0.58, 0.12, 1.0); // darker milled edge
    let face = Color::new(1.0, 0.84, 0.32, 1.0); // bright gold face
    let shine = Color::new(1.0, 0.97, 0.78, 1.0); // glint
    draw_circle(cx, cy, r, rim);
    draw_circle(cx, cy, r * 0.82, face);
    draw_circle_lines(cx, cy, r * 0.82, px(1.0), rim);
    draw_circle(cx - r * 0.3, cy - r * 0.3, r * 0.2, shine);
    // The rest of the row, starting just right of the coin.
    let line = format!(
        "{}  ·  {:.1} kn  ·  Wind {}  ({})  ·  Hull {}%",
        gs.gold, knots, wind_from, point, hull_pct
    );
    draw_text(&line, px(16.0) + 2.0 * r + px(8.0), baseline, fs, WHITE);

    // Active-debuff badges: a warning triangle (and a word) for a battered
    // hull and/or an overladen hold — the handling penalties in force.
    let mut badges: Vec<String> = Vec::new();
    if hull::fraction(gs) <= 0.90 {
        badges.push("Hull".to_string());
    }
    // Overladen: show the load against the rig's haul tolerance (e.g. 17/16)
    // and the speed penalty it costs, so the cause and the cost are both legible.
    let load = gs.hold_used();
    let haul = upgrades::max_haul(gs.sail_level);
    let pen = upgrades::overload_penalty(gs.sail_level, load);
    if pen > 0.0 {
        badges.push(format!(
            "Overladen {}/{}  (-{}% speed)",
            load,
            haul,
            (pen * 100.0).round() as i32
        ));
    }
    let warn = Color::new(1.0, 0.78, 0.2, 1.0);
    let mut x = px(16.0);
    let y = px(56.0);
    let s = px(13.0); // triangle size
    for label in &badges {
        draw_triangle(vec2(x + s * 0.5, y - s), vec2(x, y), vec2(x + s, y), warn);
        draw_text(
            "!",
            x + s * 0.5 - px(2.0),
            y - px(2.0),
            px(14.0),
            Color::new(0.1, 0.05, 0.0, 1.0),
        );
        let lx = x + s + px(6.0);
        draw_text(label, lx, y, px(15.0), warn);
        x = lx + measure_text(label, None, px(15.0) as u16, 1.0).width + px(18.0);
    }
}
