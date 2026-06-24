//! The captain's log: a parchment panel the captain flips open to read the ship's
//! state at leisure, ported in spirit from `client.SailingView`'s logbook spread.
//!
//! The original is a leather book that opens to two-page spreads. We keep its
//! *content* but not its DOM/CSS theatrics (the 3D `perspective()` page flips):
//! a flat parchment book, flipped open with **L** and paged with the **arrow
//! keys** (no mouse to click the original's nav arrows). Three spreads, in
//! reading order:
//!
//! 0. **Course & Conditions** | **The Chart** — the live readouts beside the
//!    parchment [[crate::minimap]] (the opening spread, the look kept from before).
//! 1. **The Vessel** | **The Hold** — purse/hull/rig figures, and the manifest.
//! 2. **Bearings** | **Performance** — contract/race/shipyard headings, and FPS.

use macroquad::prelude::*;

use crate::game_state::{hull, upgrades, GameState, Good};
use crate::minimap::{self, MinimapPalette};
use crate::palette::Daytime;
use crate::sailing::{Kinematics, Wind, KNOT};
use crate::world::World;

/// How many two-page spreads the book holds; `main` clamps the page cursor to this.
pub const NUM_SPREADS: usize = 3;

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
/// Warning inks for a battered hull / overladen hold (amber → red), matching the
/// original's `log-damaged` / `log-crippled` value classes.
fn warn_ink() -> Color {
    Color::new(150.0 / 255.0, 78.0 / 255.0, 20.0 / 255.0, 1.0)
}
fn alarm_ink() -> Color {
    Color::new(150.0 / 255.0, 38.0 / 255.0, 24.0 / 255.0, 1.0)
}

/// A short distance readout for a heading: km past 1 km, metres below it. Mirrors
/// `main::format_dist` / `SailingView.formatDist`.
fn format_dist(m: f32) -> String {
    if m >= 1000.0 {
        format!("{:.1} km", m / 1000.0)
    } else {
        format!("{} m", m.round() as i32)
    }
}

/// Draw a label (left) and value (right-aligned) within a column of width `col_w`.
fn row(label: &str, value: &str, x: f32, y: f32, col_w: f32, fs: u16) {
    row_colored(label, value, ink(), x, y, col_w, fs);
}

/// As [`row`], but the value is inked in `value_col` (used to flag a wounded hull,
/// a bare larder, or an overladen hold).
fn row_colored(label: &str, value: &str, value_col: Color, x: f32, y: f32, col_w: f32, fs: u16) {
    draw_text(label, x, y, fs as f32, dim_ink());
    let dims = measure_text(value, None, fs, 1.0);
    draw_text(value, x + col_w - dims.width, y, fs as f32, value_col);
}

/// The geometry of one page: its left edge `x`, its content width `col_w`, the
/// baseline of its heading, and the baseline of its first body row.
struct Page {
    x: f32,
    col_w: f32,
    title_y: f32,
    body_y: f32,
}

/// Draw a page's heading: a proper title at the top of the page, underlined to set
/// it off from the body (replacing the old centred "Captain's Log" that straddled
/// the spine).
fn heading(p: &Page, text: &str) {
    draw_text(text, p.x, p.title_y, 28.0, ink());
    draw_line(p.x, p.title_y + 10.0, p.x + p.col_w, p.title_y + 10.0, 1.5, dim_ink());
}

/// Render the open log over the scene. Dims the world behind it first. `spread` is
/// the page cursor (0..[`NUM_SPREADS`]); `frame_dt` is the last frame time, read on
/// the Performance page.
#[allow(clippy::too_many_arguments)]
pub fn render(
    world: &World,
    gs: &GameState,
    kin: &Kinematics,
    wind: Wind,
    sail_name: &str,
    day: Daytime,
    weather_label: &str,
    chart_marks: &[i32],
    spread: usize,
    frame_dt: f32,
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

    // Each page carries its own heading near the top of the page; the old centred
    // "Captain's Log" title (which sat awkwardly across the spine) is gone. Headings
    // sit on `head_y`, the first body row on `body_y`.
    let head_y = y0 + 52.0;
    let body_y = y0 + 98.0;

    // The two page slots either side of the spine.
    let left = Page {
        x: x0 + pad,
        col_w: pw / 2.0 - pad - 22.0,
        title_y: head_y,
        body_y,
    };
    let right = Page {
        x: spine + pad,
        col_w: x0 + pw - pad - (spine + pad),
        title_y: head_y,
        body_y,
    };

    // The pages of each spread, in reading order.
    match spread {
        0 => {
            page_course(&left, kin, wind, sail_name, day, weather_label);
            page_chart(&right, world, kin, wind, chart_marks, y0, ph, pad);
        }
        1 => {
            page_vessel(&left, gs);
            page_hold(&right, gs);
        }
        _ => {
            page_bearings(&left, world, kin, gs);
            page_performance(&right, frame_dt);
        }
    }

    // --- Footer: page dots + the navigation hint ---------------------------
    let foot_y = y0 + ph - 16.0;
    // Three dots centred on the spine, the current spread filled.
    let gap = 18.0;
    let dots_w = gap * (NUM_SPREADS as f32 - 1.0);
    let mut dx = spine - dots_w / 2.0;
    for i in 0..NUM_SPREADS {
        if i == spread {
            draw_circle(dx, foot_y - 5.0, 4.0, ink());
        } else {
            draw_circle_lines(dx, foot_y - 5.0, 4.0, 1.0, dim_ink());
        }
        dx += gap;
    }
    draw_text("\u{25C4} \u{25BA} turn the page", x0 + pad, foot_y, 18.0, dim_ink());
    let close = "L  close";
    let cd = measure_text(close, None, 18, 1.0);
    draw_text(close, x0 + pw - pad - cd.width, foot_y, 18.0, dim_ink());
}

/// **Course & Conditions** — the live readouts (the opening left page, unchanged).
fn page_course(p: &Page, kin: &Kinematics, wind: Wind, sail_name: &str, day: Daytime, weather_label: &str) {
    heading(p, "Course & Conditions");

    let knots = kin.speed() / KNOT;
    let deg = kin.heading_rad.to_degrees().rem_euclid(360.0).round() as i32;
    let head = format!("{} {}°", crate::compass(kin.heading_rad), deg);
    let wind_from =
        crate::compass(crate::geometry::wrap_angle(wind.toward_rad + std::f32::consts::PI));
    let point = wind.point_of_sail(kin.heading_rad).label();

    let fs = 22;
    let lh = 30.0;
    let mut y = p.body_y;
    row("Speed", &format!("{knots:.1} kn"), p.x, y, p.col_w, fs);
    y += lh;
    row("Heading", &head, p.x, y, p.col_w, fs);
    y += lh;
    row("Sail", sail_name, p.x, y, p.col_w, fs);
    y += lh * 0.6;
    draw_line(p.x, y, p.x + p.col_w, y, 1.0, dim_ink());
    y += lh * 0.8;
    row("Wind", &format!("from {wind_from}"), p.x, y, p.col_w, fs);
    y += lh;
    row("Point of sail", point, p.x, y, p.col_w, fs);
    y += lh;
    row("Weather", weather_label, p.x, y, p.col_w, fs);
    y += lh;
    row("Time", day.label(), p.x, y, p.col_w, fs);
}

/// **The Chart** — the parchment minimap of the local waters (the opening right
/// page, unchanged).
#[allow(clippy::too_many_arguments)]
fn page_chart(p: &Page, world: &World, kin: &Kinematics, wind: Wind, chart_marks: &[i32], y0: f32, ph: f32, pad: f32) {
    heading(p, "The Chart");

    // The chart square fills the page below the heading, leaving room for a
    // caption naming the local waters underneath.
    let chart_top = p.title_y + 24.0;
    let caption_h = 30.0;
    let chart_size = p.col_w.min((y0 + ph) - chart_top - pad - caption_h);
    let chart_x = p.x + (p.col_w - chart_size) / 2.0;
    let chart = Rect::new(chart_x, chart_top, chart_size, chart_size);
    let pal = MinimapPalette::parchment();
    minimap::render(world, kin, wind, chart, &pal, chart_marks, None, &[]);

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

/// **The Vessel** — purse, hull, larder, and the rig's figures.
fn page_vessel(p: &Page, gs: &GameState) {
    heading(p, "The Vessel");

    let fs = 22;
    let lh = 30.0;
    let mut y = p.body_y;

    row("Gold", &format!("{} g", gs.gold), p.x, y, p.col_w, fs);
    y += lh;

    // Hull, inked by condition: sound (ink), damaged ≤50% (amber), crippled ≤25%.
    let frac = hull::fraction(gs);
    let hull_pct = (frac * 100.0).round() as i32;
    let hull_col = if frac <= 0.25 {
        alarm_ink()
    } else if frac <= 0.50 {
        warn_ink()
    } else {
        ink()
    };
    row_colored("Hull", &format!("{hull_pct}%"), hull_col, p.x, y, p.col_w, fs);
    y += lh;

    let food = gs.food();
    let food_col = if food == 0 { alarm_ink() } else { ink() };
    row_colored("Food", &format!("{food}"), food_col, p.x, y, p.col_w, fs);
    y += lh * 0.6;
    draw_line(p.x, y, p.x + p.col_w, y, 1.0, dim_ink());
    y += lh * 0.8;

    let top = upgrades::top_knots(gs.sail_level, 0);
    row("Max speed", &format!("{:.1} kn", top), p.x, y, p.col_w, fs);
    y += lh;
    row(
        "Sail cargo",
        &format!("{} units", upgrades::max_haul(gs.sail_level)),
        p.x,
        y,
        p.col_w,
        fs,
    );
    y += lh;
    let pen = upgrades::overload_penalty(gs.sail_level, gs.hold_used());
    let pen_pct = (pen * 100.0).round() as i32;
    let (pen_txt, pen_col) = if pen_pct == 0 {
        ("none".to_string(), ink())
    } else {
        (format!("-{pen_pct}%"), alarm_ink())
    };
    row_colored("Speed penalty", &pen_txt, pen_col, p.x, y, p.col_w, fs);
}

/// **The Hold** — the laden fraction with a fill bar, then the manifest.
fn page_hold(p: &Page, gs: &GameState) {
    heading(p, "The Hold");

    let fs = 22;
    let lh = 30.0;
    let mut y = p.body_y;

    let used = gs.hold_used();
    let cap = gs.hold_capacity;
    let full = gs.hold_free() <= 0;
    let cargo_col = if full { alarm_ink() } else { ink() };
    row_colored("Cargo", &format!("{used} / {cap}"), cargo_col, p.x, y, p.col_w, fs);
    y += lh;
    row(
        "Sail tolerance",
        &format!("{} units", upgrades::max_haul(gs.sail_level)),
        p.x,
        y,
        p.col_w,
        fs,
    );
    y += lh * 0.7;

    // A fill bar reading the laden fraction; red when full. A notch marks where the
    // rig can no longer haul the full hold — load past it and she takes a penalty.
    let bar_h = 12.0;
    let bar_w = p.col_w;
    draw_rectangle(p.x, y, bar_w, bar_h, Color::new(0.0, 0.0, 0.0, 0.10));
    let frac = if cap > 0 {
        (used as f32 / cap as f32).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let fill_col = if full { alarm_ink() } else { parchment_edge() };
    draw_rectangle(p.x, y, bar_w * frac, bar_h, fill_col);
    draw_rectangle_lines(p.x, y, bar_w, bar_h, 1.0, dim_ink());
    let haul = upgrades::max_haul(gs.sail_level);
    if cap > 0 && haul < cap {
        let nx = p.x + bar_w * (haul as f32 / cap as f32).clamp(0.0, 1.0);
        draw_line(nx, y - 2.0, nx, y + bar_h + 2.0, 1.5, alarm_ink());
    }
    y += bar_h + lh * 0.8;
    draw_line(p.x, y, p.x + p.col_w, y, 1.0, dim_ink());
    y += lh * 0.8;

    // The manifest: every good aboard, plus a line for any contract-bound cargo.
    let mut any = false;
    for good in Good::ALL {
        let qty = gs.quantity_of(good);
        if qty > 0 {
            row(good.label(), &format!("{qty}"), p.x, y, p.col_w, fs);
            y += lh;
            any = true;
        }
    }
    let reserved = gs.mission_hold();
    if reserved > 0 {
        row_colored("Contract cargo", &format!("{reserved}"), warn_ink(), p.x, y, p.col_w, fs);
        y += lh;
        any = true;
    }
    if !any {
        draw_text("The hold lies empty.", p.x, y, fs as f32, dim_ink());
    }
}

/// **Bearings** — the headings the captain steers by: contract destinations, the
/// race mark, and the nearest shipyard, each as name + distance.
fn page_bearings(p: &Page, world: &World, kin: &Kinematics, gs: &GameState) {
    heading(p, "Bearings");

    let fs = 20;
    let lh = 27.0;
    let mut y = p.body_y;

    // Contracts: each accepted haul's destination and how far it lies.
    draw_text("Contracts", p.x, y, 18.0, dim_ink());
    y += lh * 0.9;
    if gs.active_missions.is_empty() {
        draw_text("No active contracts.", p.x, y, fs as f32, dim_ink());
        y += lh;
    } else {
        for m in &gs.active_missions {
            let isle = &world.islands[m.target_id as usize];
            row(&isle.name, &format_dist(kin.pos.distance_to(isle.pos)), p.x, y, p.col_w, fs);
            y += lh;
        }
    }
    y += lh * 0.3;
    draw_line(p.x, y, p.x + p.col_w, y, 1.0, dim_ink());
    y += lh * 0.9;

    // Race: the mark, its distance, and the speed made good toward it (VMG).
    draw_text("Race", p.x, y, 18.0, dim_ink());
    y += lh * 0.9;
    match gs.race {
        Some(r) => {
            let isle = &world.islands[r.target_id as usize];
            let to_mark = isle.pos - kin.pos;
            let dist = to_mark.length();
            let vmg = if dist > 1e-6 {
                kin.vel.dot(to_mark * (1.0 / dist)) / KNOT
            } else {
                0.0
            };
            row(&isle.name, &format_dist(dist), p.x, y, p.col_w, fs);
            y += lh;
            row("VMG", &format!("{vmg:.1} kn"), p.x, y, p.col_w, fs);
            y += lh;
        }
        None => {
            draw_text("No wager afoot.", p.x, y, fs as f32, dim_ink());
            y += lh;
        }
    }
    y += lh * 0.3;
    draw_line(p.x, y, p.x + p.col_w, y, 1.0, dim_ink());
    y += lh * 0.9;

    // Nearest shipyard: where to mend the hull and buy fittings.
    draw_text("Nearest Shipyard", p.x, y, 18.0, dim_ink());
    y += lh * 0.9;
    let nearest = world
        .islands
        .iter()
        .filter(|i| i.is_shipyard)
        .map(|i| (i, kin.pos.distance_to(i.pos)))
        .min_by(|a, b| a.1.total_cmp(&b.1));
    match nearest {
        Some((isle, d)) => row(&isle.name, &format_dist(d), p.x, y, p.col_w, fs),
        None => {
            draw_text("None charted.", p.x, y, fs as f32, dim_ink());
        }
    }
}

/// **Performance** — the native renderer's FPS and frame time (the original's
/// dev-only readout; genuinely useful here, where the dense mesh is the cost).
fn page_performance(p: &Page, frame_dt: f32) {
    heading(p, "Performance");

    let fs = 22;
    let lh = 30.0;
    let mut y = p.body_y;
    row("FPS", &format!("{}", get_fps()), p.x, y, p.col_w, fs);
    y += lh;
    row("Frame time", &format!("{:.1} ms", frame_dt * 1000.0), p.x, y, p.col_w, fs);
}
