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
//! 3. **The Ledger** | **The Wager Book** — the captain's lifetime tally: contracts
//!    honoured and sea-miles logged, then the race record (see `game_state::Stats`).
//! 4. **The World** — a full-spread (no spine), fully zoomed-out, hand-drawn chart of
//!    every archipelago at once, named, with no ship marked: the captain's keepsake
//!    map (see [[crate::minimap]] `render_world`).

use macroquad::prelude::*;

use crate::game_state::{hull, upgrades, GameState, Good};
use crate::minimap::{self, MinimapPalette};
use crate::palette::Daytime;
use crate::sailing::{Kinematics, Wind, KNOT};
// The ink/parchment palette, the type scale and `format_dist` are shared with the
// port board; see `crate::ui`. Only the log's own warning inks live below.
use crate::ui::{
    alarm_ink, dim_ink, format_dist, fs_body, fs_chip, fs_heading, fs_small, fs_title, ink, line_h,
    parchment, parchment_edge, px,
};
use crate::world::World;

/// How many two-page spreads the book holds; `main` clamps the page cursor to this.
pub const NUM_SPREADS: usize = 5;

/// The closing spread: the full-spread world map (no spine, no second page). Singled
/// out so `render` can skip the centre spine and hand the whole leaf to the chart.
const WORLD_SPREAD: usize = NUM_SPREADS - 1;

/// How many pressable buttons the given spread carries — `main` uses it to clamp
/// the Up/Down selection cursor. Only the Vessel spread (1) has one so far: the
/// **Caulk hull** field repair.
pub fn button_count(spread: usize) -> usize {
    match spread {
        1 => 1,
        _ => 0,
    }
}

/// Warning ink for a battered hull (amber), matching the original's `log-damaged`
/// value class — the log's own, atop the shared parchment palette. The deeper
/// `log-crippled` red is [`crate::ui::alarm_ink`], shared with the port header.
fn warn_ink() -> Color {
    Color::new(150.0 / 255.0, 78.0 / 255.0, 20.0 / 255.0, 1.0)
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

/// A parchment action button, styled like the port board's chips: filled when
/// focused (the cursor on it), outlined otherwise; the label greys when the action
/// is unavailable (`enabled` false), so a button can show *why* it can't be pressed.
fn button(x: f32, y: f32, w: f32, h: f32, label: &str, focused: bool, enabled: bool) {
    if focused {
        draw_rectangle(x, y, w, h, ink());
    } else {
        draw_rectangle_lines(x, y, w, h, px(1.5), parchment_edge());
    }
    let fs = fs_chip();
    let dims = measure_text(label, None, fs, 1.0);
    let c = if focused {
        parchment()
    } else if enabled {
        ink()
    } else {
        dim_ink()
    };
    draw_text(label, x + (w - dims.width) / 2.0, y + h / 2.0 + fs as f32 * 0.35, fs as f32, c);
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
    crate::font::heading(|| draw_text(text, p.x, p.title_y, fs_title() as f32, ink()));
    let under = p.title_y + px(10.0);
    draw_line(p.x, under, p.x + p.col_w, under, px(1.5), dim_ink());
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
    race_marks: &[i32],
    spread: usize,
    sel: usize,
    frame_dt: f32,
    // Whether the dev controls are unlocked (the "banana" cheat, see `main`): the
    // opening page flags it so the captain can see the cheat is live.
    dev_mode: bool,
    w: f32,
    h: f32,
) {
    // Dim the world so the book reads as the captain's focus.
    draw_rectangle(0.0, 0.0, w, h, Color::new(0.0, 0.0, 0.0, 0.45));

    // The open book, centred.
    let pw = (w * 0.86).min(px(760.0));
    let ph = (h * 0.90).min(px(470.0));
    let x0 = (w - pw) / 2.0;
    let y0 = (h - ph) / 2.0;
    draw_rectangle(x0, y0, pw, ph, parchment());
    draw_rectangle_lines(x0, y0, pw, ph, px(3.0), parchment_edge());
    // The spine down the middle. The world map spans the whole spread, so it gets no
    // spine; every other spread is a two-page split divided by one.
    let spine = x0 + pw / 2.0;
    if spread != WORLD_SPREAD {
        draw_line(spine, y0 + px(10.0), spine, y0 + ph - px(10.0), px(1.5), dim_ink());
    }

    let pad = px(28.0);

    // Each page carries its own heading near the top of the page; the old centred
    // "Captain's Log" title (which sat awkwardly across the spine) is gone. Headings
    // sit on `head_y`, the first body row on `body_y`.
    let head_y = y0 + px(52.0);
    let body_y = y0 + px(98.0);

    // The two page slots either side of the spine.
    let left = Page {
        x: x0 + pad,
        col_w: pw / 2.0 - pad - px(22.0),
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
            page_course(&left, kin, wind, sail_name, day, weather_label, dev_mode);
            page_chart(&right, world, kin, wind, chart_marks, race_marks, y0, ph, pad);
        }
        1 => {
            page_vessel(&left, gs, sel);
            page_hold(&right, gs);
        }
        2 => {
            page_bearings(&left, world, kin, gs);
            page_performance(&right, frame_dt);
        }
        3 => {
            page_ledger(&left, gs);
            page_wagers(&right, gs);
        }
        _ => {
            page_world(world, x0, y0, pw, ph, pad, head_y);
        }
    }

    // --- Footer: page dots + the navigation hint ---------------------------
    let foot_y = y0 + ph - px(16.0);
    // Three dots centred on the spine, the current spread filled.
    let gap = px(18.0);
    let dots_w = gap * (NUM_SPREADS as f32 - 1.0);
    let mut dx = spine - dots_w / 2.0;
    for i in 0..NUM_SPREADS {
        if i == spread {
            draw_circle(dx, foot_y - px(5.0), px(4.0), ink());
        } else {
            draw_circle_lines(dx, foot_y - px(5.0), px(4.0), px(1.0), dim_ink());
        }
        dx += gap;
    }
    // Left footer: paging, plus the button cursor's keys on spreads that have one.
    let nav = if button_count(spread) > 0 {
        "\u{25C4} \u{25BA} pages   \u{25B2} \u{25BC} Enter  use"
    } else {
        "\u{25C4} \u{25BA} turn the page"
    };
    draw_text(nav, x0 + pad, foot_y, fs_small() as f32, dim_ink());
    let close = "L  close";
    let cd = measure_text(close, None, fs_small(), 1.0);
    draw_text(close, x0 + pw - pad - cd.width, foot_y, fs_small() as f32, dim_ink());
}

/// **Course & Conditions** — the live readouts (the opening left page). Carries a
/// "dev mode active" flag at its foot when the dev controls are unlocked (see the
/// "banana" cheat in `main`); the line is absent while dev mode is off.
fn page_course(p: &Page, kin: &Kinematics, wind: Wind, sail_name: &str, day: Daytime, weather_label: &str, dev_mode: bool) {
    heading(p, "Course & Conditions");

    let knots = kin.speed() / KNOT;
    let deg = kin.heading_rad.to_degrees().rem_euclid(360.0).round() as i32;
    let head = format!("{} {}°", crate::compass(kin.heading_rad), deg);
    let wind_from =
        crate::compass(crate::geometry::wrap_angle(wind.toward_rad + std::f32::consts::PI));
    let point = wind.point_of_sail(kin.heading_rad).label();

    let fs = fs_body();
    let lh = line_h(fs);
    let mut y = p.body_y;
    row("Speed", &format!("{knots:.1} kn"), p.x, y, p.col_w, fs);
    y += lh;
    row("Heading", &head, p.x, y, p.col_w, fs);
    y += lh;
    row("Sail", sail_name, p.x, y, p.col_w, fs);
    y += lh * 0.6;
    draw_line(p.x, y, p.x + p.col_w, y, px(1.0), dim_ink());
    y += lh * 0.8;
    row("Wind", &format!("from {wind_from}"), p.x, y, p.col_w, fs);
    y += lh;
    row("Point of sail", point, p.x, y, p.col_w, fs);
    y += lh;
    row("Weather", weather_label, p.x, y, p.col_w, fs);
    y += lh;
    row("Time", day.label(), p.x, y, p.col_w, fs);

    // The cheat tell: only inked while the dev controls are unlocked. Set off below a
    // rule and in the amber warning ink so it reads as out-of-the-ordinary.
    if dev_mode {
        y += lh * 0.6;
        draw_line(p.x, y, p.x + p.col_w, y, px(1.0), dim_ink());
        y += lh * 0.8;
        draw_text("Dev mode active", p.x, y, fs as f32, warn_ink());
    }
}

/// **The Chart** — the parchment minimap of the local waters (the opening right
/// page, unchanged).
#[allow(clippy::too_many_arguments)]
fn page_chart(p: &Page, world: &World, kin: &Kinematics, wind: Wind, chart_marks: &[i32], race_marks: &[i32], y0: f32, ph: f32, pad: f32) {
    heading(p, "The Chart");

    // The chart square fills the page below the heading, leaving room for a
    // caption naming the local waters underneath.
    let chart_top = p.title_y + px(24.0);
    let caption_h = px(30.0);
    let chart_size = p.col_w.min((y0 + ph) - chart_top - pad - caption_h);
    let chart_x = p.x + (p.col_w - chart_size) / 2.0;
    let chart = Rect::new(chart_x, chart_top, chart_size, chart_size);
    let pal = MinimapPalette::parchment();
    minimap::render(world, kin, wind, chart, &pal, chart_marks, race_marks, None, &[], None);

    // Name the local waters under the chart.
    let cluster = world.cluster_at(kin.pos);
    let cd = measure_text(&cluster.name, None, fs_small(), 1.0);
    draw_text(
        &cluster.name,
        chart_x + (chart_size - cd.width) / 2.0,
        chart_top + chart_size + px(22.0),
        fs_small() as f32,
        ink(),
    );
}

/// **The World** — a full spread (no centre spine): a fully zoomed-out, hand-drawn
/// chart of every archipelago at once, each named, with no ship marked. A keepsake map
/// rather than a live instrument (see [`crate::minimap::render_world`]).
fn page_world(world: &World, x0: f32, y0: f32, pw: f32, ph: f32, pad: f32, head_y: f32) {
    // Title centred across the whole spread, with a short rule under it.
    let title = "The World";
    let fs = fs_title();
    crate::font::heading(|| {
        let d = measure_text(title, None, fs, 1.0);
        draw_text(title, x0 + (pw - d.width) / 2.0, head_y, fs as f32, ink());
    });
    let under_y = head_y + px(10.0);
    let und_half = pw * 0.16;
    draw_line(x0 + pw / 2.0 - und_half, under_y, x0 + pw / 2.0 + und_half, under_y, px(1.5), dim_ink());

    // A 16:9 chart centred on the spread (matching the world's own 16:9 layout, and the
    // whole two-page spread it spans), leaving room for a caption and the footer below.
    // Sized off the spread width, then shrunk to the available height if that's tighter.
    let map_top = head_y + px(26.0);
    let map_bottom = y0 + ph - pad - px(30.0);
    let avail_h = (map_bottom - map_top).max(px(60.0));
    let mut mw = pw - 2.0 * pad;
    let mut mh = mw * 9.0 / 16.0;
    if mh > avail_h {
        mh = avail_h;
        mw = mh * 16.0 / 9.0;
    }
    let map_rect = Rect::new(x0 + (pw - mw) / 2.0, map_top, mw, mh);
    // Draw straight onto the logbook leaf (transparent panel) so it reads as inked into
    // the page rather than pasted over it.
    let mut pal = MinimapPalette::parchment();
    pal.panel = Color::new(0.0, 0.0, 0.0, 0.0);
    minimap::render_world(world, map_rect, &pal);

    // Caption under the chart.
    let cap = "All charted waters";
    let cd = measure_text(cap, None, fs_small(), 1.0);
    draw_text(cap, x0 + (pw - cd.width) / 2.0, map_rect.y + map_rect.h + px(20.0), fs_small() as f32, dim_ink());
}

/// **The Vessel** — purse, hull, larder, and the rig's figures. `sel` is the open
/// spread's button cursor (this page owns button 0, the **Caulk hull** repair).
fn page_vessel(p: &Page, gs: &GameState, sel: usize) {
    heading(p, "The Vessel");

    let fs = fs_body();
    let lh = line_h(fs);
    let mut y = p.body_y;

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

    // Field repair: a button that caulks the hull with a plank from the hold (see
    // `GameState::caulk_with_plank`) — a +10 mend at sea, no drydock. Kept right
    // under the hull readout so it stays in view however the condition list grows.
    // The planks tally sits beside it; the button is live only with timber aboard
    // and a hull worth mending.
    let planks = gs.quantity_of(Good::Plank);
    let can_caulk = planks > 0 && hull::damage(gs) > 0;
    row("Planks", &format!("{planks} aboard"), p.x, y, p.col_w, fs);
    y += lh * 0.25;
    let btn_w = p.col_w.min(px(220.0));
    let btn_h = px(26.0);
    let label = if planks <= 0 {
        "Caulk hull — no timber".to_string()
    } else if hull::damage(gs) <= 0 {
        "Caulk hull — sound".to_string()
    } else {
        format!("Caulk hull  +{}", GameState::HULL_PER_PLANK)
    };
    button(p.x, y, btn_w, btn_h, &label, sel == 0 && can_caulk, can_caulk);
    y += btn_h + lh * 0.5;
    draw_line(p.x, y, p.x + p.col_w, y, px(1.0), dim_ink());
    y += lh * 0.8;

    let top = upgrades::top_knots(gs.hull_level, gs.sail_level, 0);
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
    y += lh * 0.7;

    // Hull-condition handicaps: the no-go zone, helm and top-speed penalties a
    // battered hull suffers (see `game_state::hull::debuff`). Listed only when in
    // force, with the harbourmaster's job ban flagged below a quarter hull… er,
    // below 30% hull.
    draw_line(p.x, y, p.x + p.col_w, y, px(1.0), dim_ink());
    y += lh * 0.8;
    heading_minor(p, "Hull condition", y);
    y += lh * 0.9;
    let penalties = hull::penalty_lines(frac);
    if penalties.is_empty() {
        row_colored("Handling", "sound", ink(), p.x, y, p.col_w, fs);
        y += lh;
    } else {
        for (label, value) in &penalties {
            row_colored(label, value, warn_ink(), p.x, y, p.col_w, fs);
            y += lh;
        }
    }
    if frac <= hull::JOB_REFUSE_FRACTION {
        row_colored("Contracts & races", "refused", alarm_ink(), p.x, y, p.col_w, fs);
    }
}

/// A smaller sub-heading within a page body, used to group the hull-condition
/// readout below the rig figures.
fn heading_minor(p: &Page, text: &str, y: f32) {
    crate::font::heading(|| draw_text(text, p.x, y, fs_heading() as f32, dim_ink()));
}

/// **The Hold** — the laden fraction with a fill bar, then the manifest.
fn page_hold(p: &Page, gs: &GameState) {
    heading(p, "The Hold");

    let fs = fs_body();
    let lh = line_h(fs);
    let mut y = p.body_y;

    row("Gold", &format!("{} g", gs.gold), p.x, y, p.col_w, fs);
    y += lh * 0.6;
    draw_line(p.x, y, p.x + p.col_w, y, px(1.0), dim_ink());
    y += lh * 0.8;

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
    let bar_h = px(12.0);
    let bar_w = p.col_w;
    draw_rectangle(p.x, y, bar_w, bar_h, Color::new(0.0, 0.0, 0.0, 0.10));
    let frac = if cap > 0 {
        (used as f32 / cap as f32).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let fill_col = if full { alarm_ink() } else { parchment_edge() };
    draw_rectangle(p.x, y, bar_w * frac, bar_h, fill_col);
    draw_rectangle_lines(p.x, y, bar_w, bar_h, px(1.0), dim_ink());
    let haul = upgrades::max_haul(gs.sail_level);
    if cap > 0 && haul < cap {
        let nx = p.x + bar_w * (haul as f32 / cap as f32).clamp(0.0, 1.0);
        draw_line(nx, y - px(2.0), nx, y + bar_h + px(2.0), px(1.5), alarm_ink());
    }
    y += bar_h + lh * 0.8;
    draw_line(p.x, y, p.x + p.col_w, y, px(1.0), dim_ink());
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

    let fs = fs_body();
    let lh = line_h(fs);
    let mut y = p.body_y;

    // Contracts: each accepted haul's destination and how far it lies.
    crate::font::heading(|| draw_text("Contracts", p.x, y, fs_heading() as f32, dim_ink()));
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
    draw_line(p.x, y, p.x + p.col_w, y, px(1.0), dim_ink());
    y += lh * 0.9;

    // Race: the mark, its distance, and the speed made good toward it (VMG).
    crate::font::heading(|| draw_text("Race", p.x, y, fs_heading() as f32, dim_ink()));
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
    draw_line(p.x, y, p.x + p.col_w, y, px(1.0), dim_ink());
    y += lh * 0.9;

    // Nearest shipyard: where to mend the hull and buy fittings.
    crate::font::heading(|| draw_text("Nearest Shipyard", p.x, y, fs_heading() as f32, dim_ink()));
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

    let fs = fs_body();
    let lh = line_h(fs);
    let mut y = p.body_y;
    row("FPS", &format!("{}", get_fps()), p.x, y, p.col_w, fs);
    y += lh;
    row("Frame time", &format!("{:.1} ms", frame_dt * 1000.0), p.x, y, p.col_w, fs);
}

/// **The Ledger** — the lifetime tally of honest work: contracts honoured and the
/// sea-miles logged over the whole voyage (see [`crate::game_state::Stats`]).
fn page_ledger(p: &Page, gs: &GameState) {
    heading(p, "The Ledger");

    let fs = fs_body();
    let lh = line_h(fs);
    let mut y = p.body_y;

    let st = &gs.stats;
    row("Contracts fulfilled", &format!("{}", st.contracts_fulfilled), p.x, y, p.col_w, fs);
    y += lh;
    // Reward gold only: the returned deposit isn't a gain, so it isn't tallied.
    row("Contract earnings", &format!("{} g", st.contract_earnings), p.x, y, p.col_w, fs);
    y += lh * 0.6;
    draw_line(p.x, y, p.x + p.col_w, y, px(1.0), dim_ink());
    y += lh * 0.8;

    // Distance logged, shown in the same km/m form as the rest of the UI.
    row("Distance sailed", &format_dist(st.meters_traveled as f32), p.x, y, p.col_w, fs);
    y += lh;
    row("Days at sea", &format!("{}", st.days_passed), p.x, y, p.col_w, fs);
    y += lh * 0.6;
    draw_line(p.x, y, p.x + p.col_w, y, px(1.0), dim_ink());
    y += lh * 0.8;

    // Salvage recovered from the swell (count and the gold it fetched).
    row("Flotsam recovered", &format!("{}", st.flotsam_collected), p.x, y, p.col_w, fs);
    y += lh;
    row("Salvage gold", &format!("{} g", st.flotsam_gold), p.x, y, p.col_w, fs);
}

/// **The Wager Book** — the captain's racing record: wagers won, wagers lost, and
/// the tally of races sailed.
fn page_wagers(p: &Page, gs: &GameState) {
    heading(p, "The Wager Book");

    let fs = fs_body();
    let lh = line_h(fs);
    let mut y = p.body_y;

    let st = &gs.stats;
    row("Races won", &format!("{}", st.races_won), p.x, y, p.col_w, fs);
    y += lh;
    row("Races lost", &format!("{}", st.races_lost), p.x, y, p.col_w, fs);
    y += lh * 0.6;
    draw_line(p.x, y, p.x + p.col_w, y, px(1.0), dim_ink());
    y += lh * 0.8;
    let sailed = st.races_won + st.races_lost;
    row("Races sailed", &format!("{sailed}"), p.x, y, p.col_w, fs);
    y += lh;
    // Net stake gold across all wagers: green-ink in the black, alarm-ink in the red.
    let w = st.race_winnings;
    let (txt, col) = if w > 0 {
        (format!("+{w} g"), ink())
    } else if w < 0 {
        (format!("-{} g", -w), alarm_ink())
    } else {
        ("0 g".to_string(), dim_ink())
    };
    row_colored("Winnings", &txt, col, p.x, y, p.col_w, fs);
}
