//! The captain's log: a parchment panel the captain flips open to read the ship's
//! state at leisure, ported in spirit from `client.SailingView`'s logbook spread.
//!
//! The original is a leather book that opens to two-page spreads. We keep its
//! *content* but not its DOM/CSS theatrics (the 3D `perspective()` page flips):
//! a flat parchment book, flipped open with **L** and paged with the **arrow
//! keys** (no mouse to click the original's nav arrows). The spreads in the book
//! (the count is derived; see [`active_spreads`]), in reading order:
//!
//! - **Course & Conditions** | **The Chart** — the live readouts beside the
//!   parchment [[crate::minimap]] (the opening spread, the look kept from before).
//! - **The Vessel** | **The Hold** — purse/hull/rig figures, and the manifest.
//! - **Bearings** | **Performance** — contract/race/shipyard headings, and FPS.
//! - **The Ledger** | **The Wager Book** — the captain's lifetime tally: contracts
//!   honoured and sea-miles logged, then the race record (see `game_state::Stats`).
//! - **The Almanac** — a full-spread price book across the local archipelago; present
//!   only once the captain buys the Trader's Almanac (see [[crate::tavern]]).
//! - **Legendary Trinkets** | **Trinket** — a selectable list of every trinket on the
//!   left, the chosen one's name, emblem and description on the right.
//! - **The World** — a full-spread (no spine), fully zoomed-out, hand-drawn chart of
//!   every archipelago at once, named, with no ship marked: the captain's keepsake
//!   map (see [[crate::minimap]] `render_world`); present only once the World Map is
//!   bought (see [[crate::tavern]]).

use macroquad::prelude::*;

use crate::game_state::{hull, upgrades, GameState, Good, Market};
use crate::minimap::{self, MinimapPalette};
use crate::tavern::{self, SpecialItem};
use crate::palette::Daytime;
use crate::sailing::{Kinematics, Wind, KNOT};
// The ink/parchment palette, the type scale and `format_dist` are shared with the
// port board; see `crate::ui`. Only the log's own warning inks live below.
use crate::ui::{
    alarm_ink, dim_ink, format_dist, fs_body, fs_chip, fs_heading, fs_small, fs_title, ink, line_h,
    parchment, parchment_edge, px,
};
use crate::world::World;

/// One two-page spread of the log. The first four always stand; the Almanac and the
/// World map appear only once the captain buys the tavern wares that unlock them (see
/// [`crate::tavern`]), so the book grows over a voyage.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Spread {
    CourseChart,
    VesselHold,
    BearingsPerformance,
    LedgerWagers,
    /// A price book across the local archipelago (the Trader's Almanac ware).
    Almanac,
    /// The captain's collection of legendary trinkets: a selectable list on the left,
    /// the chosen trinket's name/artwork/description on the right.
    Trinkets,
    /// The full-spread keepsake world map (the World Map ware): no spine, no second
    /// page, the whole leaf handed to the chart.
    World,
}

/// The spreads in the book, in reading order: the four standing spreads, then the
/// Almanac and the World map, each only when the captain owns the ware unlocking it.
fn active_spreads(gs: &GameState) -> Vec<Spread> {
    let mut v = vec![
        Spread::CourseChart,
        Spread::VesselHold,
        Spread::BearingsPerformance,
        Spread::LedgerWagers,
    ];
    if gs.owns(SpecialItem::TradersAlmanac) {
        v.push(Spread::Almanac);
    }
    // The trinket collection is always in the book (it lists every trinket, owned or
    // not), and sits just before the world map.
    v.push(Spread::Trinkets);
    if gs.owns(SpecialItem::WorldMap) {
        v.push(Spread::World);
    }
    v
}

/// How many spreads the book holds for this captain; `main` clamps the page cursor
/// to it.
pub fn num_spreads(gs: &GameState) -> usize {
    active_spreads(gs).len()
}

/// The page index of the world-map spread, if the captain owns the World Map, so
/// `main` can flip straight to it on **M**.
pub fn world_spread_index(gs: &GameState) -> Option<usize> {
    active_spreads(gs).iter().position(|s| *s == Spread::World)
}

/// How many Up/Down-navigable items the spread at `index` carries — `main` uses it to
/// clamp the selection cursor. The Vessel spread has one (the **Caulk hull** button);
/// the Trinkets spread has one row per trinket (the selectable list).
pub fn button_count(gs: &GameState, index: usize) -> usize {
    match active_spreads(gs).get(index) {
        Some(Spread::VesselHold) => 1,
        Some(Spread::Trinkets) => SpecialItem::COUNT,
        _ => 0,
    }
}

/// Whether the spread at `index` is the Legendary Trinkets spread — so `main` can
/// hit-test pointer taps on its rows (see [`trinket_row_rect`]).
pub fn is_trinkets(gs: &GameState, index: usize) -> bool {
    matches!(active_spreads(gs).get(index), Some(Spread::Trinkets))
}

/// The book panel's outer rect `(x0, y0, pw, ph)` for a `w`×`h` screen. The one place
/// the panel size is set, shared by the render and the pointer hit-tests.
fn book_panel(w: f32, h: f32) -> (f32, f32, f32, f32) {
    let pw = (w * 0.86).min(px(760.0));
    let ph = (h * 0.90).min(px(470.0));
    ((w - pw) / 2.0, (h - ph) / 2.0, pw, ph)
}

/// Layout of the Legendary Trinkets list on the left page: the row left edge `x`, the
/// column width, the first row's text baseline, and the per-row step. Shared by the
/// page's draw and [`trinket_row_rect`] so the highlight and the hit-test never drift.
fn trinket_list_geom(w: f32, h: f32) -> (f32, f32, f32, f32) {
    let (x0, y0, pw, _ph) = book_panel(w, h);
    let pad = px(28.0);
    let x = x0 + pad;
    let col_w = pw / 2.0 - pad - px(22.0);
    let body_y = y0 + px(98.0);
    (x, col_w, body_y, line_h(fs_body()))
}

/// The clickable rect of trinket row `i` (0-based) on the Legendary Trinkets spread,
/// for a `w`×`h` screen — the same band the page highlights when it's selected.
pub fn trinket_row_rect(i: usize, w: f32, h: f32) -> Rect {
    let (x, col_w, body_y, lh) = trinket_list_geom(w, h);
    let baseline = body_y + i as f32 * lh;
    Rect::new(x - px(6.0), baseline - lh * 0.78, col_w + px(12.0), lh)
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
/// the page cursor (0..[`num_spreads`]); `frame_dt` is the last frame time, read on
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
    let (x0, y0, pw, ph) = book_panel(w, h);
    draw_rectangle(x0, y0, pw, ph, parchment());
    draw_rectangle_lines(x0, y0, pw, ph, px(3.0), parchment_edge());

    // Which spread is open. The book's pages depend on the wares owned (see
    // `active_spreads`); the cursor index is clamped to whatever is in the book now.
    let spreads = active_spreads(gs);
    let kind = spreads
        .get(spread.min(spreads.len().saturating_sub(1)))
        .copied()
        .unwrap_or(Spread::CourseChart);
    // The full-spread pages (the world map and the almanac grid) span the whole leaf,
    // so they get no centre spine; every other spread is a two-page split divided by one.
    let full_spread = matches!(kind, Spread::Almanac | Spread::World);
    let spine = x0 + pw / 2.0;
    if !full_spread {
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

    // The pages of the open spread.
    match kind {
        Spread::CourseChart => {
            page_course(&left, kin, wind, sail_name, day, weather_label, dev_mode);
            page_chart(&right, world, kin, wind, chart_marks, race_marks, y0, ph, pad);
        }
        Spread::VesselHold => {
            page_vessel(&left, gs, sel);
            page_hold(&right, gs);
        }
        Spread::BearingsPerformance => {
            page_bearings(&left, world, kin, gs);
            page_performance(&right, frame_dt);
        }
        Spread::LedgerWagers => {
            page_ledger(&left, gs);
            page_wagers(&right, gs);
        }
        Spread::Almanac => {
            page_almanac(world, kin, x0, pw, pad, head_y);
        }
        Spread::Trinkets => {
            page_trinkets(&left, gs, sel);
            page_trinket_detail(&right, gs, sel);
        }
        Spread::World => {
            page_world(world, gs, x0, y0, pw, ph, pad, head_y);
        }
    }

    // --- Footer: page dots + the navigation hint ---------------------------
    let foot_y = y0 + ph - px(16.0);
    // One dot per spread, centred on the spine, the current spread filled.
    let n_spreads = spreads.len();
    let cur = spread.min(n_spreads.saturating_sub(1));
    let gap = px(18.0);
    let dots_w = gap * (n_spreads as f32 - 1.0);
    let mut dx = spine - dots_w / 2.0;
    for i in 0..n_spreads {
        if i == cur {
            draw_circle(dx, foot_y - px(5.0), px(4.0), ink());
        } else {
            draw_circle_lines(dx, foot_y - px(5.0), px(4.0), px(1.0), dim_ink());
        }
        dx += gap;
    }
    // Left footer: paging, plus the cursor's keys on spreads that take one.
    let nav = match kind {
        Spread::VesselHold => "\u{25C4} \u{25BA} pages   \u{25B2} \u{25BC} Enter  use",
        Spread::Trinkets => "\u{25C4} \u{25BA} pages   \u{25B2} \u{25BC} choose a trinket",
        _ => "\u{25C4} \u{25BA} turn the page",
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
/// chart of every archipelago at once, each named (and labelled with the legendary
/// trinket its tavern sells), with no ship marked. A keepsake map rather than a live
/// instrument (see [`crate::minimap::render_world`]).
fn page_world(world: &World, gs: &GameState, x0: f32, y0: f32, pw: f32, ph: f32, pad: f32, head_y: f32) {
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
    // The trinket each archipelago's tavern sells (and whether it's already in the kit),
    // indexed by cluster id, so the chart can letter each cluster with its ware.
    let wares: Vec<Option<(&str, bool)>> = world
        .clusters
        .iter()
        .map(|c| {
            world
                .cluster_islands(c)
                .into_iter()
                .find(|i| i.is_shipyard)
                .and_then(|sy| tavern::item_at(world, sy.id))
                .map(|item| (item.name(), gs.owns(item)))
        })
        .collect();
    minimap::render_world(world, map_rect, &pal, &wares);

    // Caption under the chart.
    let cap = "All charted waters";
    let cd = measure_text(cap, None, fs_small(), 1.0);
    draw_text(cap, x0 + (pw - cd.width) / 2.0, map_rect.y + map_rect.h + px(20.0), fs_small() as f32, dim_ink());
}

/// **The Almanac** — a full-spread price book (the Trader's Almanac ware): for each
/// good, the cheapest port to buy it and the dearest to sell it across the local
/// archipelago, with the margin between, so the captain can plan a run without
/// sailing the cluster to read every board.
fn page_almanac(world: &World, kin: &Kinematics, x0: f32, pw: f32, pad: f32, head_y: f32) {
    // Title centred across the whole spread, underlined.
    let title = "The Almanac";
    let fs = fs_title();
    crate::font::heading(|| {
        let d = measure_text(title, None, fs, 1.0);
        draw_text(title, x0 + (pw - d.width) / 2.0, head_y, fs as f32, ink());
    });
    let under_y = head_y + px(10.0);
    let und_half = pw * 0.16;
    draw_line(x0 + pw / 2.0 - und_half, under_y, x0 + pw / 2.0 + und_half, under_y, px(1.5), dim_ink());

    let cluster = world.cluster_at(kin.pos);
    let cap = format!("Best prices across the {}.", cluster.name);
    let cd = measure_text(&cap, None, fs_small(), 1.0);
    draw_text(&cap, x0 + (pw - cd.width) / 2.0, head_y + px(30.0), fs_small() as f32, dim_ink());

    // Every port in these waters, with its (deterministic) price sheet.
    let ports: Vec<_> = world
        .cluster_islands(cluster)
        .into_iter()
        .filter(|i| i.is_port)
        .map(|i| (i, Market::for_island(i, world.seed)))
        .collect();

    let x = x0 + pad;
    let cw = pw - 2.0 * pad;
    let buy_x = x + cw * 0.22;
    let sell_x = x + cw * 0.54;
    let margin_r = x + cw; // right edge for the right-aligned margin column

    // Right-align `text` so its right edge sits at `rx`.
    let right_at = |text: &str, rx: f32, y: f32, col: Color| {
        let d = measure_text(text, None, fs_small(), 1.0);
        draw_text(text, rx - d.width, y, fs_small() as f32, col);
    };

    let mut ry = head_y + px(54.0);
    draw_text("Commodity", x, ry, fs_small() as f32, dim_ink());
    draw_text("Cheapest to buy", buy_x, ry, fs_small() as f32, dim_ink());
    draw_text("Dearest to sell", sell_x, ry, fs_small() as f32, dim_ink());
    right_at("Margin", margin_r, ry, dim_ink());
    draw_line(x, ry + px(6.0), x + cw, ry + px(6.0), px(1.0), dim_ink());
    ry += line_h(fs_heading());

    if ports.is_empty() {
        draw_text("No ports in these waters.", x, ry, fs_body() as f32, dim_ink());
        return;
    }

    let lh = line_h(fs_body());
    for good in Good::ALL {
        let lo = ports.iter().min_by_key(|(_, m)| m.price(good)).unwrap();
        let hi = ports.iter().max_by_key(|(_, m)| m.price(good)).unwrap();
        let lo_price = lo.1.price(good);
        let hi_price = hi.1.price(good);
        draw_text(good.label(), x, ry, fs_body() as f32, ink());
        draw_text(&format!("{} · {}", lo.0.name, lo_price), buy_x, ry, fs_small() as f32, ink());
        draw_text(&format!("{} · {}", hi.0.name, hi_price), sell_x, ry, fs_small() as f32, ink());
        // The margin a perfect round trip would clear; greyed when flat (one port, or
        // no spread to work).
        let margin = hi_price - lo_price;
        let (txt, col) = if margin > 0 { (format!("+{margin}"), ink()) } else { ("0".to_string(), dim_ink()) };
        right_at(&txt, margin_r, ry, col);
        ry += lh;
    }
}

/// Word-wrap `text` into lines no wider than `max_w` at font size `fs`.
fn wrap(text: &str, fs: u16, max_w: f32) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        let trial = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
        if !cur.is_empty() && measure_text(&trial, None, fs, 1.0).width > max_w {
            lines.push(std::mem::replace(&mut cur, word.to_string()));
        } else {
            cur = trial;
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

/// **Legendary Trinkets** (left page) — the captain's collection: every trinket, one
/// per row, by name, with a checkmark on the ones owned. `sel` is the highlighted row
/// (moved with Up/Down or a pointer tap), driving the detail on the facing page.
fn page_trinkets(p: &Page, gs: &GameState, sel: usize) {
    heading(p, "Legendary Trinkets");
    let fs = fs_body();
    let lh = line_h(fs);
    let sel = sel.min(SpecialItem::COUNT - 1);
    let mut y = p.body_y;
    for (i, item) in SpecialItem::ALL.iter().enumerate() {
        if i == sel {
            // Highlight the selected row (the same sepia band the board uses for a row).
            draw_rectangle(
                p.x - px(6.0),
                y - lh * 0.78,
                p.col_w + px(12.0),
                lh,
                Color::new(150.0 / 255.0, 110.0 / 255.0, 60.0 / 255.0, 0.28),
            );
        }
        let owned = gs.owns(*item);
        draw_text(item.name(), p.x, y, fs as f32, ink());
        // A checkmark, right-aligned in the column, on the trinkets already in the kit.
        if owned {
            let ck = "\u{2713}";
            let d = measure_text(ck, None, fs, 1.0);
            draw_text(ck, p.x + p.col_w - d.width, y, fs as f32, ink());
        }
        y += lh;
    }
}

/// **Trinket** (right page) — the selected trinket's name, a hand-inked emblem, and its
/// description, plus a line on whether it's in the kit (else where to buy it).
fn page_trinket_detail(p: &Page, gs: &GameState, sel: usize) {
    heading(p, "Trinket");
    let item = SpecialItem::ALL[sel.min(SpecialItem::COUNT - 1)];
    let lh = line_h(fs_body());
    let mut y = p.body_y;

    // The trinket's name as a sub-title in the display face.
    crate::font::heading(|| draw_text(item.name(), p.x, y, fs_heading() as f32, ink()));
    y += line_h(fs_heading());

    // A framed cartouche holding the hand-inked emblem, centred in the column.
    let art = p.col_w.min(px(150.0));
    let box_x = p.x + (p.col_w - art) / 2.0;
    let box_y = y;
    draw_rectangle_lines(box_x, box_y, art, art, px(1.5), parchment_edge());
    draw_trinket_art(item, box_x + art / 2.0, box_y + art / 2.0, art * 0.34, ink());
    y = box_y + art + lh * 0.9;

    // The description, word-wrapped to the column.
    for line in wrap(item.blurb(), fs_small(), p.col_w) {
        draw_text(&line, p.x, y, fs_small() as f32, dim_ink());
        y += line_h(fs_small());
    }
    y += lh * 0.5;

    // In the kit, or a note on where to acquire it.
    let (txt, col) = if gs.owns(item) {
        ("In your kit \u{2713}".to_string(), ink())
    } else {
        (format!("Sold at a shipyard tavern for {} gold.", item.price()), dim_ink())
    };
    draw_text(&txt, p.x, y, fs_small() as f32, col);

    // For an owned *active* ware (the ones invoked at the helm), note whether its daily
    // charge is ready or spent. The charge comes back at sunrise (the day/night clock
    // rolling into a new day, see `main`'s days-passed tally), so a spent ware is good
    // again at first light. Passive keepsakes have no charge, so they get no line.
    if gs.owns(item) && item.is_active() {
        y += line_h(fs_small());
        let (status, scol) = if gs.item_ready(item) {
            ("Charged: ready to use.".to_string(), ink())
        } else {
            ("Spent: recharges at sunrise.".to_string(), dim_ink())
        };
        draw_text(&status, p.x, y, fs_small() as f32, scol);
    }
}

/// Ink a small hand-drawn emblem for `item`, centred at (`cx`,`cy`) and sized so the
/// art spans roughly `2*size`, in chart ink `col`. Each trinket gets its own little
/// device, the way an old codex illustrated a curiosity beside its entry.
fn draw_trinket_art(item: SpecialItem, cx: f32, cy: f32, size: f32, col: Color) {
    let th = (size * 0.10).max(1.5);
    // A circular arc as a short polyline (macroquad draws only straight segments), for
    // the wind gusts and the horseshoe.
    let arc = |ccx: f32, ccy: f32, rad: f32, a0: f32, a1: f32, thick: f32| {
        const SEG: i32 = 24;
        let mut prev = (ccx + rad * a0.cos(), ccy + rad * a0.sin());
        for i in 1..=SEG {
            let t = i as f32 / SEG as f32;
            let a = a0 + (a1 - a0) * t;
            let cur = (ccx + rad * a.cos(), ccy + rad * a.sin());
            draw_line(prev.0, prev.1, cur.0, cur.1, thick, col);
            prev = cur;
        }
    };

    match item {
        SpecialItem::WorldMap => {
            // A rolled chart: a sheet with thick rolled edges, a dotted route, and an X.
            let w = size * 1.7;
            let hh = size * 1.15;
            let (l, r, t, b) = (cx - w / 2.0, cx + w / 2.0, cy - hh / 2.0, cy + hh / 2.0);
            draw_rectangle_lines(l, t, w, hh, th, col);
            draw_line(l, t, l, b, th * 1.8, col); // rolled left edge
            draw_line(r, t, r, b, th * 1.8, col); // rolled right edge
            // A dotted route across the sheet.
            let n = 7;
            for i in 0..n {
                let f0 = i as f32 / n as f32;
                let f1 = f0 + 0.5 / n as f32;
                let p0 = (l + w * (0.12 + 0.7 * f0), b - hh * (0.2 + 0.55 * f0));
                let p1 = (l + w * (0.12 + 0.7 * f1), b - hh * (0.2 + 0.55 * f1));
                draw_line(p0.0, p0.1, p1.0, p1.1, th * 0.8, col);
            }
            // The X marking the spot.
            let (xx, xy, e) = (cx + w * 0.22, cy + hh * 0.12, size * 0.16);
            draw_line(xx - e, xy - e, xx + e, xy + e, th, col);
            draw_line(xx - e, xy + e, xx + e, xy - e, th, col);
        }
        SpecialItem::WindWhistle => {
            // Two curling gusts of wind, each a line trailing into a near-full curl.
            let gust = |gy: f32, curl_r: f32| {
                let end_x = cx + size * 0.35;
                draw_line(cx - size, gy, end_x, gy, th, col);
                arc(end_x, gy - curl_r, curl_r, std::f32::consts::FRAC_PI_2, std::f32::consts::FRAC_PI_2 + 5.4, th);
            };
            gust(cy - size * 0.4, size * 0.24);
            gust(cy + size * 0.35, size * 0.32);
        }
        SpecialItem::DolphinsDraught => {
            // A bottle: a body, a narrow neck, a cork, and a line for the liquid within.
            let bw = size * 0.95;
            let bt = cy - size * 0.15;
            let bb = cy + size * 1.0;
            let (l, r) = (cx - bw / 2.0, cx + bw / 2.0);
            draw_rectangle_lines(l, bt, bw, bb - bt, th, col);
            let nw = size * 0.34;
            draw_line(cx - nw / 2.0, bt, cx - nw / 2.0, bt - size * 0.5, th, col);
            draw_line(cx + nw / 2.0, bt, cx + nw / 2.0, bt - size * 0.5, th, col);
            draw_rectangle_lines(cx - nw * 0.6, bt - size * 0.75, nw * 1.2, size * 0.26, th, col);
            draw_line(l + th, cy + size * 0.35, r - th, cy + size * 0.35, th * 0.8, col);
        }
        SpecialItem::StormGlass => {
            // A sealed glass vial with a liquid level and a few settling crystals.
            let gw = size * 0.62;
            let gh = size * 1.7;
            let (l, t) = (cx - gw / 2.0, cy - gh / 2.0);
            draw_rectangle_lines(l, t, gw, gh, th, col);
            draw_rectangle_lines(cx - gw * 0.36, t - size * 0.26, gw * 0.72, size * 0.26, th, col); // stopper
            let lvl = cy + gh * 0.1;
            draw_line(l + th, lvl, l + gw - th, lvl, th * 0.8, col);
            // Crystal fronds rising from the floor.
            for k in 0..3 {
                let bx = l + gw * (0.28 + 0.22 * k as f32);
                let by = t + gh - th;
                draw_line(bx, by, bx - gw * 0.08, by - gh * 0.22, th * 0.8, col);
                draw_line(bx, by, bx + gw * 0.08, by - gh * 0.18, th * 0.8, col);
            }
        }
        SpecialItem::TradersAlmanac => {
            // An open book: two pages flaring from a central spine, ruled with lines.
            let half = size * 1.05;
            let (st, sb) = (cy - size * 0.55, cy + size * 0.5);
            let (pt, pb) = (cy - size * 0.35, cy + size * 0.7); // outer page corners
            // Left and right page outlines.
            for s in [-1.0f32, 1.0] {
                let ox = cx + s * half;
                draw_line(cx, st, ox, pt, th, col);
                draw_line(ox, pt, ox, pb, th, col);
                draw_line(ox, pb, cx, sb, th, col);
                // Ruled text lines.
                for k in 0..3 {
                    let ly = cy - size * 0.1 + k as f32 * size * 0.26;
                    draw_line(cx + s * size * 0.2, ly, cx + s * half * 0.82, ly + s * size * 0.04, th * 0.7, col);
                }
            }
            draw_line(cx, st, cx, sb, th, col); // spine
        }
        SpecialItem::LuckyFigurehead => {
            // A horseshoe, open at the foot, with end caps and three nail holes — luck
            // drawn the way every sailor knows it.
            let rad = size * 0.85;
            let a0 = 120.0f32.to_radians();
            let a1 = 420.0f32.to_radians(); // up over the top, leaving a gap at the bottom
            arc(cx, cy, rad, a0, a1, th * 2.0);
            for &a in &[a0, a1] {
                draw_circle(cx + rad * a.cos(), cy + rad * a.sin(), th * 1.1, col);
            }
            for k in 0..3 {
                let a = a0 + (a1 - a0) * (0.2 + 0.3 * k as f32);
                draw_circle_lines(cx + rad * a.cos(), cy + rad * a.sin(), th * 0.6, th * 0.5, col);
            }
        }
    }
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
