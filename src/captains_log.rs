//! The captain's log: a parchment panel the captain flips open to read the ship's
//! state at leisure, ported in spirit from `client.SailingView`'s logbook spread.
//!
//! The original is a leather book that opens to two-page spreads (the vessel, the
//! course & conditions, the hold, the chart…). Most of those pages need game
//! systems not yet ported (gold, hold, hull, contracts). This is the slice we can
//! fill today: a "Course & Conditions" page of live readouts beside the chart
//! spread (a [[crate::minimap]] inked on parchment). Toggled with the **L** key.

use macroquad::prelude::*;

use crate::minimap::{self, MinimapPalette};
use crate::palette::Daytime;
use crate::sailing::{Kinematics, Wind, KNOT};
use crate::world::World;

/// Parchment + ink colours for the open book.
fn ink() -> Color {
    Color::new(79.0 / 255.0, 47.0 / 255.0, 23.0 / 255.0, 1.0)
}
fn dim_ink() -> Color {
    Color::new(79.0 / 255.0, 47.0 / 255.0, 23.0 / 255.0, 0.62)
}
fn parchment() -> Color {
    Color::new(230.0 / 255.0, 216.0 / 255.0, 176.0 / 255.0, 1.0)
}
fn parchment_edge() -> Color {
    Color::new(120.0 / 255.0, 90.0 / 255.0, 55.0 / 255.0, 0.9)
}

/// Draw a label (left) and value (right-aligned) within a column of width `col_w`.
fn row(label: &str, value: &str, x: f32, y: f32, col_w: f32, fs: u16) {
    draw_text(label, x, y, fs as f32, dim_ink());
    let dims = measure_text(value, None, fs, 1.0);
    draw_text(value, x + col_w - dims.width, y, fs as f32, ink());
}

/// Render the open log over the scene. Dims the world behind it first.
#[allow(clippy::too_many_arguments)]
pub fn render(
    world: &World,
    kin: &Kinematics,
    wind: Wind,
    sail_name: &str,
    day: Daytime,
    weather_label: &str,
    mission_targets: &[i32],
    w: f32,
    h: f32,
) {
    // Dim the world so the book reads as the captain's focus.
    draw_rectangle(0.0, 0.0, w, h, Color::new(0.0, 0.0, 0.0, 0.45));

    // The open book, centred.
    let pw = (w * 0.86).min(760.0);
    let ph = (h * 0.86).min(400.0);
    let x0 = (w - pw) / 2.0;
    let y0 = (h - ph) / 2.0;
    draw_rectangle(x0, y0, pw, ph, parchment());
    draw_rectangle_lines(x0, y0, pw, ph, 3.0, parchment_edge());
    // The spine down the middle.
    let spine = x0 + pw / 2.0;
    draw_line(spine, y0 + 10.0, spine, y0 + ph - 10.0, 1.5, dim_ink());

    let pad = 28.0;

    // --- Header (across the spread) ----------------------------------------
    let title = "Captain's Log";
    let td = measure_text(title, None, 30, 1.0);
    draw_text(title, x0 + (pw - td.width) / 2.0, y0 + 44.0, 30.0, ink());
    draw_line(x0 + pad, y0 + 58.0, x0 + pw - pad, y0 + 58.0, 1.0, dim_ink());

    // --- Left page: Course & Conditions ------------------------------------
    let lx = x0 + pad;
    let col_w = pw / 2.0 - pad - 22.0;
    draw_text("Course & Conditions", lx, y0 + 92.0, 22.0, dim_ink());

    let knots = kin.speed() / KNOT;
    let deg = kin.heading_rad.to_degrees().rem_euclid(360.0).round() as i32;
    let heading = format!("{} {}°", crate::compass(kin.heading_rad), deg);
    let wind_from =
        crate::compass(crate::geometry::wrap_angle(wind.toward_rad + std::f32::consts::PI));
    let point = wind.point_of_sail(kin.heading_rad).label();

    let fs = 22;
    let mut y = y0 + 126.0;
    let lh = 30.0;
    row("Speed", &format!("{knots:.1} kn"), lx, y, col_w, fs);
    y += lh;
    row("Heading", &heading, lx, y, col_w, fs);
    y += lh;
    row("Sail", sail_name, lx, y, col_w, fs);
    y += lh * 0.6;
    draw_line(lx, y, lx + col_w, y, 1.0, dim_ink());
    y += lh * 0.8;
    row("Wind", &format!("from {wind_from}"), lx, y, col_w, fs);
    y += lh;
    row("Point of sail", point, lx, y, col_w, fs);
    y += lh;
    row("Weather", weather_label, lx, y, col_w, fs);
    y += lh;
    row("Time", day.label(), lx, y, col_w, fs);

    // --- Right page: The Chart ---------------------------------------------
    let rx = spine + pad;
    let rw = x0 + pw - pad - rx;
    draw_text("The Chart", rx, y0 + 92.0, 22.0, dim_ink());

    // The chart square fills the right page below the subtitle, leaving room for a
    // caption naming the local waters underneath.
    let chart_top = y0 + 108.0;
    let caption_h = 30.0;
    let chart_size = rw.min(ph - (chart_top - y0) - pad - caption_h);
    let chart_x = rx + (rw - chart_size) / 2.0;
    let chart = Rect::new(chart_x, chart_top, chart_size, chart_size);
    let pal = MinimapPalette::parchment();
    minimap::render(world, kin, wind, chart, &pal, mission_targets, None, &[]);

    // Name the local waters under the chart.
    let cluster = world.cluster_at(kin.pos);
    let cd = measure_text(&cluster.name, None, 20, 1.0);
    draw_text(
        &cluster.name,
        chart_x + (chart_size - cd.width) / 2.0,
        chart_top + chart_size + 22.0,
        20.0,
        ink(),
    );
}
