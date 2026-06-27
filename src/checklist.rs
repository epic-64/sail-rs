//! A short beginner's checklist, drawn on the sailing HUD under the corner chart.
//!
//! It points a fresh captain at the basics worth doing on a first voyage (tie up at
//! a port, run a contract, sail a race, mend the hull, and so on) without holding
//! their hand: each step is a one-line nudge that ticks off the moment the captain
//! does the deed. Progress is read straight from the lifetime [`Stats`] tally, so it
//! survives a save and never double-counts. Once every step is struck the list
//! vanishes for good, leaving the chart corner clear for the seasoned hand. The
//! steps themselves are the single source of truth (see [`steps`]); add or drop one
//! there and the panel resizes to suit.

use macroquad::prelude::*;

use crate::game_state::Stats;
use crate::ui::px;

/// One row of the list: a label and whether the captain has done it yet.
struct Step {
    label: &'static str,
    done: bool,
}

/// The steps, in the order a first voyage tends to meet them, each resolved against
/// the lifetime tally. Editing this array is all it takes to change the list: the
/// panel sizes and lays itself out off its length and contents.
fn steps(stats: &Stats) -> [Step; 6] {
    [
        Step { label: "Dock at a port", done: stats.times_docked > 0 },
        Step { label: "Complete a contract", done: stats.contracts_fulfilled > 0 },
        // A race counts whether it was won or lost: the point is to have sailed one.
        Step {
            label: "Sail a race (win or lose)",
            done: stats.races_won + stats.races_lost > 0,
        },
        Step { label: "Repair the hull at a port", done: stats.hull_repairs > 0 },
        Step { label: "Buy a ship upgrade", done: stats.upgrades_bought > 0 },
        Step { label: "Inspect the captain's log", done: stats.log_opened > 0 },
    ]
}

/// Draw the checklist as a dark-glass panel whose top edge sits a small gap below
/// `chart`, the corner minimap's rect (so the two share a right margin and read as
/// one stack). A no-op once every step is done. Drawn straight to the screen, so
/// call it after `set_default_camera`, alongside the other HUD overlays.
pub fn render(stats: &Stats, chart: Rect) {
    let steps = steps(stats);
    if steps.iter().all(|s| s.done) {
        return;
    }

    // Glass + ink to match the corner chart's HUD palette (see `MinimapPalette::hud`).
    let panel = Color::new(8.0 / 255.0, 16.0 / 255.0, 28.0 / 255.0, 0.55);
    let border = Color::new(150.0 / 255.0, 200.0 / 255.0, 255.0 / 255.0, 0.28);
    let title_ink = Color::new(255.0 / 255.0, 224.0 / 255.0, 138.0 / 255.0, 0.95);
    let done_ink = Color::new(150.0 / 255.0, 200.0 / 255.0, 255.0 / 255.0, 0.55);
    let todo_ink = Color::new(1.0, 1.0, 1.0, 0.95);
    let check_ink = Color::new(120.0 / 255.0, 210.0 / 255.0, 140.0 / 255.0, 0.95);

    let pad = px(8.0);
    let title_fs = px(14.0) as u16;
    let row_fs = px(13.0) as u16;
    let title_h = px(20.0);
    let row_h = px(18.0);
    let mark_w = measure_text("[x] ", None, row_fs, 1.0).width; // gutter for the checkbox

    // Width fits the widest line (title or any row) so a long label never clips; the
    // panel is then right-aligned to the chart's edge so it reads as one stack with
    // the chart, but is at least as wide as the chart.
    let content_w = steps
        .iter()
        .map(|s| mark_w + measure_text(s.label, None, row_fs, 1.0).width)
        .fold(measure_text("First Voyage", None, title_fs, 1.0).width, f32::max);
    let w = (content_w + pad * 2.0).max(chart.w);
    let h = title_h + row_h * steps.len() as f32 + pad * 2.0;
    let x = chart.x + chart.w - w; // share the chart's right edge
    let y = chart.y + chart.h + px(8.0); // a small gap below the chart

    draw_rectangle(x, y, w, h, panel);
    draw_rectangle_lines(x, y, w, h, 2.0, border);

    let mut ty = y + pad + title_fs as f32;
    draw_text("First Voyage", x + pad, ty, title_fs as f32, title_ink);
    ty += title_h - title_fs as f32 + row_fs as f32;

    for step in &steps {
        let (mark, mark_ink, label_ink) = if step.done {
            ("[x]", check_ink, done_ink)
        } else {
            ("[ ]", todo_ink, todo_ink)
        };
        draw_text(mark, x + pad, ty, row_fs as f32, mark_ink);
        draw_text(step.label, x + pad + mark_w, ty, row_fs as f32, label_ink);
        ty += row_h;
    }
}
