//! A small top-down chart, ported from `client.MinimapRenderer`.
//!
//! The ship's local waters are drawn zoomed so every island is just a dot (ports a
//! little brighter, shipyards ringed and lettered "S"), with the ship a heading
//! arrow at its world position. North is up. Faint wind streaks (with chevrons) flow
//! across the chart along the wind. As the ship sails away from the nearest
//! archipelago the frame widens, zooming out so the cluster recedes and the open sea
//! (with any neighbouring isles) comes into view; if it strays far its arrow clamps
//! to the frame edge rather than flying off the chart.
//!
//! Drawn straight to the screen (after `set_default_camera`), so the same renderer
//! serves both the always-on corner HUD map (`MinimapPalette::hud`) and the
//! captain's-log chart on parchment (`MinimapPalette::parchment`).

use macroquad::prelude::*;

use crate::geometry::Vec2;
use crate::sailing::{Kinematics, Wind};
use crate::world::{IsleKind, World};

/// Make a colour from 0–255 channels plus an alpha in [0,1].
fn rgba(r: u8, g: u8, b: u8, a: f32) -> Color {
    Color::new(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a)
}

/// The minimap's ink colours. The corner HUD map is drawn over a dark glass panel;
/// the logbook copy is inked onto parchment, so each wants its own scheme.
pub struct MinimapPalette {
    pub panel: Color,
    pub border: Color,
    pub wind_streak: Color,
    pub shipyard_ring: Color,
    pub port: Color,
    pub land: Color,
    pub ship: Color,
    pub mission_mark: Color,
    pub race_mark: Color,
    pub trader: Color,
}

impl MinimapPalette {
    /// Bright marks on a dark glass panel for the corner HUD map.
    pub fn hud() -> Self {
        MinimapPalette {
            panel: rgba(8, 16, 28, 0.55),
            border: rgba(150, 200, 255, 0.28),
            wind_streak: rgba(150, 200, 255, 0.20),
            shipyard_ring: rgba(96, 170, 255, 0.95),
            port: rgba(255, 224, 138, 0.95),
            land: rgba(176, 214, 210, 0.8),
            ship: rgba(255, 255, 255, 0.95),
            mission_mark: rgba(255, 210, 90, 0.95),
            race_mark: rgba(255, 92, 92, 0.95),
            trader: rgba(96, 210, 120, 0.95),
        }
    }

    /// Sepia inks for the logbook chart drawn on beige parchment. The panel itself
    /// is transparent here — the log draws the parchment leaf behind it.
    pub fn parchment() -> Self {
        MinimapPalette {
            panel: rgba(222, 205, 162, 1.0),
            border: rgba(120, 90, 55, 0.9),
            wind_streak: rgba(79, 47, 23, 0.16),
            shipyard_ring: rgba(47, 111, 158, 1.0),
            port: rgba(79, 47, 23, 0.7),
            land: rgba(42, 32, 24, 0.35),
            ship: rgba(79, 47, 23, 1.0),
            mission_mark: rgba(200, 150, 47, 1.0),
            race_mark: rgba(168, 40, 30, 1.0),
            trader: rgba(54, 96, 78, 0.9),
        }
    }
}

/// Liang–Barsky clip of a segment to `r`. `None` when the segment misses the rect
/// entirely. macroquad has no canvas clip, so we trim the wind streaks ourselves so
/// they never spill out past the chart frame.
fn clip_segment(x0: f32, y0: f32, x1: f32, y1: f32, r: Rect) -> Option<(f32, f32, f32, f32)> {
    let dx = x1 - x0;
    let dy = y1 - y0;
    let p = [-dx, dx, -dy, dy];
    let q = [x0 - r.x, r.x + r.w - x0, y0 - r.y, r.y + r.h - y0];
    let mut u0 = 0.0f32;
    let mut u1 = 1.0f32;
    for i in 0..4 {
        if p[i] == 0.0 {
            if q[i] < 0.0 {
                return None; // parallel and outside this edge
            }
        } else {
            let t = q[i] / p[i];
            if p[i] < 0.0 {
                if t > u1 {
                    return None;
                }
                if t > u0 {
                    u0 = t;
                }
            } else {
                if t < u0 {
                    return None;
                }
                if t < u1 {
                    u1 = t;
                }
            }
        }
    }
    Some((x0 + u0 * dx, y0 + u0 * dy, x0 + u1 * dx, y0 + u1 * dy))
}

/// Draw a dashed line from (x0,y0) to (x1,y1) — macroquad only draws solid lines,
/// so we lay down `dash`-long segments separated by `gap`.
#[allow(clippy::too_many_arguments)] // two endpoints + stroke style is inherent
fn draw_dashed_line(x0: f32, y0: f32, x1: f32, y1: f32, thick: f32, dash: f32, gap: f32, color: Color) {
    let dx = x1 - x0;
    let dy = y1 - y0;
    let len = (dx * dx + dy * dy).sqrt();
    if len <= 0.0 {
        return;
    }
    let (ux, uy) = (dx / len, dy / len);
    let step = dash + gap;
    let mut d = 0.0;
    while d < len {
        let a = d;
        let b = (d + dash).min(len);
        draw_line(x0 + ux * a, y0 + uy * a, x0 + ux * b, y0 + uy * b, thick, color);
        d += step;
    }
}

/// Paint the chart into the square `rect` (screen space). `mission_targets` mark
/// the isles that hold an active contract's destination — a yellow ring with an
/// "M" (empty until missions land); `race_targets` mark the booked race's mark —
/// a red ring with an "R". `route`, if set, draws a dashed rhumb line between two
/// world points (the docked port and a highlighted contract's or race's other
/// port) so the captain can weigh a leg against the wind before taking it.
#[allow(clippy::too_many_arguments)]
pub fn render(
    world: &World,
    kin: &Kinematics,
    wind: Wind,
    rect: Rect,
    pal: &MinimapPalette,
    mission_targets: &[i32],
    race_targets: &[i32],
    route: Option<(Vec2, Vec2)>,
    // World positions of the local cluster's wandering traders, drawn as small
    // green triangles so the captain can spot the traffic crossing the bay. Empty on
    // the charts that don't track them (the log, the port board).
    traders: &[Vec2],
    // The racing rival's live world position and heading while a race is afoot,
    // drawn as a red heading-arrow (a twin of the player's) so the captain can see
    // where his opponent has got to and which way it's pointed. `None` off the
    // water (no race) and on the charts that don't track it (the log, the board).
    rival: Option<(Vec2, f32)>,
) {
    // Panel + frame. (Parchment's panel is opaque beige; the HUD's is dark glass.)
    if pal.panel.a > 0.0 {
        draw_rectangle(rect.x, rect.y, rect.w, rect.h, pal.panel);
    }
    draw_rectangle_lines(rect.x, rect.y, rect.w, rect.h, 2.0, pal.border);

    let size = rect.w.min(rect.h);
    let s = size / 168.0; // scale every CSS-pixel constant off the original 168px map
    let pad = 12.0 * s;
    let cx = rect.x + size / 2.0;
    let cy = rect.y + size / 2.0;

    // Frame the ship's local waters so the isles nearly fill the chart, then zoom out
    // as it leaves them: the farther the ship strays from the nearest archipelago the
    // wider the frame grows (up to a cap), so the cluster recedes and the open sea
    // (with any neighbouring isles) comes into view.
    //
    // To cross the water between two archipelagos without the view snapping the moment
    // the nearest one changes at the midline, we cross-fade: take the two nearest
    // clusters and slide the centre (and frame size) toward the midpoint as the second
    // draws level. At the midline the framing is the same whichever one counts as
    // "nearest", so the swap is seamless.
    const SHIP_FILL: f32 = 0.92; // keep the ship within this fraction of the half-frame
    const MAX_ZOOM_OUT: f32 = 3.0; // widest frame, as a multiple of the cluster's own
    const PULL_MAX: f32 = 0.65; // how far the centre slides from archipelago toward ship
    // Breathing room around the cluster's own span when it frames the chart. Kept tight
    // (just clear of the isles) so a captain sitting in an archipelago sees it fill the
    // chart; the pixel `pad` below still leaves room for the mark rings at the edge.
    const CLUSTER_MARGIN: f32 = 1.0;

    let a = world.cluster_at(kin.pos); // nearest archipelago
    let da = a.center.distance_to(kin.pos);
    // Second nearest (falls back to the nearest when the world holds a single cluster).
    let mut b = a;
    let mut db = f32::INFINITY;
    for c in &world.clusters {
        let d = c.center.distance_to(kin.pos);
        if c.id != a.id && d < db {
            db = d;
            b = c;
        }
    }
    let (ca, ha) = world.cluster_bounds(a);
    let (cb, hb) = world.cluster_bounds(b);

    // Blend weight: 0 deep in the nearest cluster's waters, ramping to 1 at the midline
    // where the second is just as close. `w` slides the centre at most halfway to the
    // other cluster, so both orderings meet at the midpoint and the swap is continuous.
    let ratio = if db.is_finite() { da / db } else { 0.0 };
    let t = ((ratio - 0.6) / 0.4).clamp(0.0, 1.0);
    let t = t * t * (3.0 - 2.0 * t); // smoothstep
    let w = t * 0.5;
    let anchor_x = ca.x * (1.0 - w) + cb.x * w;
    let anchor_y = ca.y * (1.0 - w) + cb.y * w;
    let half = ha * (1.0 - w) + hb * w; // blended so the frame size is seamless too

    // As the ship leaves the archipelago's footprint, slide the centre off the isles
    // and toward the ship, so the chart follows the captain instead of pinning the
    // isles dead-centre with the ship adrift at the edge. 0 inside the footprint,
    // easing to PULL_MAX once the ship is well clear of it.
    let stray = (kin.pos.x - anchor_x).hypot(kin.pos.y - anchor_y);
    let out = ((stray / half.max(1.0)) - 1.0).clamp(0.0, 1.5) / 1.5;
    let out = out * out * (3.0 - 2.0 * out); // smoothstep
    let pull = out * PULL_MAX;
    let view_x = anchor_x * (1.0 - pull) + kin.pos.x * pull;
    let view_y = anchor_y * (1.0 - pull) + kin.pos.y * pull;

    // Wide enough to keep both the ship and the (now off-centre) archipelago on the
    // chart, floored at the cluster's own span and capped so it never zooms out too far.
    let ship_off = (kin.pos.x - view_x).hypot(kin.pos.y - view_y);
    let cluster_reach = (ca.x - view_x).hypot(ca.y - view_y) + half * CLUSTER_MARGIN;
    let frame = (ship_off / SHIP_FILL)
        .max(cluster_reach)
        .min(half * CLUSTER_MARGIN * MAX_ZOOM_OUT);
    let scale = (size / 2.0 - pad) / frame;
    // World x (east) right, world y (north) up, so flip the screen y axis.
    let sx = |p: Vec2| cx + (p.x - view_x) * scale;
    let sy = |p: Vec2| cy - (p.y - view_y) * scale;

    // Wind streaks: faint parallel lines across the chart along the wind's flow,
    // each with a chevron pointing the way it blows. North-up, so they stay fixed
    // while the ship's arrow turns.
    let wdx = wind.toward_rad.sin(); // flow direction on the map (north up)
    let wdy = -wind.toward_rad.cos();
    let per_x = -wdy; // perpendicular: spacing between streaks
    let per_y = wdx;
    let span = size;
    let step = 24.0 * s;
    let n_lines = (span / step) as i32;
    for i in -n_lines..=n_lines {
        let o = i as f32 * step;
        let mx = cx + per_x * o;
        let my = cy + per_y * o;
        if let Some((ax, ay, bx, by)) =
            clip_segment(mx - wdx * span, my - wdy * span, mx + wdx * span, my + wdy * span, rect)
        {
            draw_line(ax, ay, bx, by, 1.0, pal.wind_streak);
        }
        // A chevron near the midpoint showing which way the wind flows — only when
        // its anchor sits inside the chart.
        let tx = mx + wdx * 5.0 * s;
        let ty = my + wdy * 5.0 * s;
        if rect.contains(vec2(tx, ty)) {
            let l1x = tx - wdx * 6.0 * s + per_x * 4.0 * s;
            let l1y = ty - wdy * 6.0 * s + per_y * 4.0 * s;
            let l2x = tx - wdx * 6.0 * s - per_x * 4.0 * s;
            let l2y = ty - wdy * 6.0 * s - per_y * 4.0 * s;
            draw_line(l1x, l1y, tx, ty, 1.0, pal.wind_streak);
            draw_line(l2x, l2y, tx, ty, 1.0, pal.wind_streak);
        }
    }

    // A dashed rhumb line between the docked port and the highlighted contract's
    // other port, drawn under the island dots so the markers sit on top.
    if let Some((from, to)) = route {
        if let Some((ax, ay, bx, by)) =
            clip_segment(sx(from), sy(from), sx(to), sy(to), rect)
        {
            draw_dashed_line(ax, ay, bx, by, 1.6, 6.0 * s, 4.0 * s, pal.mission_mark);
        }
    }

    // Every isle in view: a dot each (ports brighter), shipyards ringed. A mission
    // destination gets a yellow ring with an "M"; a race mark gets a red ring with an
    // "R" (both drawn on top of all). A small helper rings the isle and letters it
    // just above the ring, clear of it.
    let mark = |x: f32, y: f32, letter: &str, col: Color, rr: f32| {
        draw_circle_lines(x, y, rr, 2.0, col);
        let fs = (13.0 * s).max(11.0);
        let dims = measure_text(letter, None, fs as u16, 1.0);
        draw_text(letter, x - dims.width / 2.0, y - rr - 3.0 * s, fs, col);
    };
    for isle in &world.islands {
        let x = sx(isle.pos);
        let y = sy(isle.pos);
        // Cull isles whose mark falls outside the chart: macroquad has no canvas clip,
        // so we trim them ourselves rather than let them spill across the screen when
        // the frame slides as the ship sails far out.
        if !rect.contains(vec2(x, y)) {
            continue;
        }
        let r = if isle.is_port { 3.2 } else { 2.4 } * s;
        draw_circle(x, y, r, if isle.is_port { pal.port } else { pal.land });
        if isle.is_shipyard {
            let rr = 5.4 * s;
            draw_circle_lines(x, y, rr, 1.5, pal.shipyard_ring);
            // An "S" below the ring (clear of the M/R marks, which sit above).
            let fs = (13.0 * s).max(11.0);
            let dims = measure_text("S", None, fs as u16, 1.0);
            draw_text("S", x - dims.width / 2.0, y + rr + fs, fs, pal.shipyard_ring);
        }
        if mission_targets.contains(&isle.id) {
            mark(x, y, "M", pal.mission_mark, 5.5 * s);
        }
        // A race mark rings a touch wider so it reads distinctly even when the same
        // port also holds a contract.
        if race_targets.contains(&isle.id) {
            mark(x, y, "R", pal.race_mark, 7.2 * s);
        }
    }

    // The local traders: a small green triangle each at their world position, drawn
    // under the player's arrow. Sized at half the ship/rival arrow and pointed north
    // (the chart doesn't track a trader's heading), so the traffic reads as small
    // green darts without masquerading as a heading-arrow. Only those whose mark
    // falls inside the chart are shown.
    for &tp in traders {
        let x = sx(tp);
        let y = sy(tp);
        if rect.contains(vec2(x, y)) {
            let nose = 4.0 * s; // half the arrow's 8.0*s reach
            let tail = 2.25 * s; // half the arrow's 4.5*s base set-back
            let half = 1.2 * s; // half the arrow's 2.4*s half-width
            draw_triangle(
                vec2(x, y - nose),
                vec2(x - half, y + tail),
                vec2(x + half, y + tail),
                pal.trader,
            );
        }
    }

    // Draw a heading-arrow for a vessel at world `pos` pointing along `heading`,
    // clamped to the frame so it stays on the chart while crossing open sea. The
    // triangle is long and narrow (a slim spike) so its bearing reads clearly.
    let arrow = |pos: Vec2, heading: f32, col: Color| {
        let px = sx(pos).clamp(rect.x + pad, rect.x + size - pad);
        let py = sy(pos).clamp(rect.y + pad, rect.y + size - pad);
        let fx = heading.sin(); // forward dir on the map (north up)
        let fy = -heading.cos();
        let rx = -fy; // right = forward rotated 90°
        let ry = fx;
        let nose = 8.0 * s; // tip reach ahead
        let tail = 4.5 * s; // base set back
        let half = 2.4 * s; // half-width at the base (narrow → clear bearing)
        draw_triangle(
            vec2(px + fx * nose, py + fy * nose),
            vec2(px - fx * tail + rx * half, py - fy * tail + ry * half),
            vec2(px - fx * tail - rx * half, py - fy * tail - ry * half),
            col,
        );
    };

    // The racing rival, drawn first (under the player) as a red twin of the arrow
    // so the captain can see which way the one-to-beat is pointed.
    if let Some((rp, rh)) = rival {
        arrow(rp, rh, pal.race_mark);
    }

    // The player's ship, on top.
    arrow(kin.pos, kin.heading_rad, pal.ship);
}

/// A cheap, deterministic hash in [0,1] keyed off an integer, used to wobble the
/// hand-drawn coastlines on the world map. Deterministic so the chart looks the same
/// every time the captain flips to it (no RNG draws, so world generation is untouched).
fn hash01(n: u32) -> f32 {
    let mut x = n.wrapping_mul(0x9e3779b1);
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb352d);
    x ^= x >> 15;
    (x & 0xffff) as f32 / 65535.0
}

/// Ink a small hand-drawn kraken (a mantle, two eyes, and curling tentacles) centred
/// at (`cx`,`cy`) and sized to `size`, for the empty-quarter flourish on the world map.
/// `s` is the map's glyph scale (for stroke width); `col` the chart ink.
fn draw_kraken(cx: f32, cy: f32, size: f32, s: f32, col: Color) {
    let th = (1.3 * s).max(1.0);
    let hr = size * 0.42; // mantle radius

    // The mantle: a wobbled blob outline, taller than wide with a slight crown.
    const N: usize = 12;
    let mut px = [0.0f32; N];
    let mut py = [0.0f32; N];
    for k in 0..N {
        let ang = k as f32 / N as f32 * std::f32::consts::TAU;
        let rr = hr * (1.0 + 0.12 * hash01(k as u32 * 41 + 7));
        px[k] = cx + ang.sin() * rr;
        py[k] = cy - ang.cos() * rr * 1.15;
    }
    for k in 0..N {
        let n = (k + 1) % N;
        draw_line(px[k], py[k], px[n], py[n], th, col);
    }

    // Two eyes on the upper mantle.
    let er = (hr * 0.16).max(1.0);
    draw_circle(cx - hr * 0.36, cy - hr * 0.22, er, col);
    draw_circle(cx + hr * 0.36, cy - hr * 0.22, er, col);

    // Tentacles fanning across the lower half, each a curling, tapering polyline.
    let base = (cx, cy + hr * 0.55);
    const N_TENT: usize = 8;
    let tl = size;
    for i in 0..N_TENT {
        let frac = i as f32 / (N_TENT - 1) as f32;
        let ang = std::f32::consts::PI * (0.10 + 0.80 * frac);
        let (dx, dy) = (ang.cos(), ang.sin()); // dy > 0 → downward (screen y down)
        let (perpx, perpy) = (-dy, dx);
        let curl = if i % 2 == 0 { 1.0 } else { -1.0 };
        let mut prev = base;
        const SEG: i32 = 7;
        for j in 1..=SEG {
            let t = j as f32 / SEG as f32;
            let along = tl * t;
            let wob = (t * std::f32::consts::PI * 1.5).sin() * size * 0.18 * curl;
            let nx = base.0 + dx * along + perpx * wob;
            let ny = base.1 + dy * along + perpy * wob;
            let w = (th * (1.05 - 0.6 * t)).max(0.8); // taper toward the tip
            draw_line(prev.0, prev.1, nx, ny, w, col);
            prev = (nx, ny);
        }
    }
}

/// Ink a small hand-drawn whale (a humpbacked body facing left, with eye, flipper,
/// tail flukes, and a spout) centred at (`cx`,`cy`) and sized to `size`, the companion
/// flourish to the kraken on the world map. `s` is the glyph scale; `col` the ink.
fn draw_whale(cx: f32, cy: f32, size: f32, s: f32, col: Color) {
    let th = (1.2 * s).max(1.0);
    let l = size * 1.5; // body length
    let h = size * 0.78; // body height

    // The body: a tapered, humped oval outline (head left, narrowing to the tail right).
    const N: usize = 16;
    let mut bx = [0.0f32; N];
    let mut by = [0.0f32; N];
    for k in 0..N {
        let ang = k as f32 / N as f32 * std::f32::consts::TAU;
        let ex = ang.cos();
        let ey = ang.sin();
        let taper = 1.0 - 0.6 * ex.max(0.0); // narrow toward the tail
        let hump = if ey < 0.0 { -0.12 * h * (ex * 0.5 + 0.5) } else { 0.0 };
        bx[k] = cx + ex * l * 0.5;
        by[k] = cy + ey * h * 0.5 * taper + hump;
    }
    for k in 0..N {
        let n = (k + 1) % N;
        draw_line(bx[k], by[k], bx[n], by[n], th, col);
    }

    // Tail flukes at the right tip.
    let tx = cx + l * 0.5;
    let f = size * 0.5;
    draw_line(tx, cy, tx + f, cy - f * 0.8, th, col);
    draw_line(tx + f, cy - f * 0.8, tx + f * 0.35, cy, th, col);
    draw_line(tx, cy, tx + f, cy + f * 0.8, th, col);
    draw_line(tx + f, cy + f * 0.8, tx + f * 0.35, cy, th, col);

    // Eye near the head, a small flipper under the belly, and a spout above the head.
    draw_circle(cx - l * 0.34, cy - h * 0.08, (size * 0.05).max(1.0), col);
    draw_line(cx - l * 0.1, cy + h * 0.32, cx + l * 0.05, cy + h * 0.55, th, col);
    draw_line(cx + l * 0.05, cy + h * 0.55, cx + l * 0.12, cy + h * 0.3, th, col);
    let (spx, spy) = (cx - l * 0.3, cy - h * 0.5);
    for &dxs in &[-0.18f32, 0.0, 0.18] {
        draw_line(spx, spy, spx + dxs * size, spy - size * 0.55, th * 0.9, col);
    }
}

/// Ink a small hand-drawn breaking wave (a swell curling into a foam-tipped lip over a
/// gently rippled surface) centred at (`cx`,`cy`) and sized to `size`, the third of the
/// world map's empty-quarter flourishes. `s` is the glyph scale; `col` the ink.
fn draw_wave(cx: f32, cy: f32, size: f32, s: f32, col: Color) {
    let th = (1.2 * s).max(1.0);
    // A quadratic bezier, laid down as short segments (macroquad draws only straight
    // lines) so the swell and lip read as smooth hand-drawn curves.
    let bez = |p0: (f32, f32), p1: (f32, f32), p2: (f32, f32)| {
        const ST: i32 = 14;
        let mut prev = p0;
        for i in 1..=ST {
            let t = i as f32 / ST as f32;
            let u = 1.0 - t;
            let x = u * u * p0.0 + 2.0 * u * t * p1.0 + t * t * p2.0;
            let y = u * u * p0.1 + 2.0 * u * t * p1.1 + t * t * p2.1;
            draw_line(prev.0, prev.1, x, y, th, col);
            prev = (x, y);
        }
    };

    // Back swell rising from the trough to the crest, the curling lip, and the inner curl.
    let crest = (cx + 0.15 * size, cy - 0.6 * size);
    let lip = (cx + 0.7 * size, cy - 0.05 * size);
    bez((cx - size, cy + 0.45 * size), (cx - 0.5 * size, cy - 0.55 * size), crest);
    bez(crest, (cx + 0.75 * size, cy - 0.75 * size), lip);
    bez(lip, (cx + 0.55 * size, cy + 0.2 * size), (cx + 0.2 * size, cy + 0.05 * size));
    bez(crest, (cx + 0.45 * size, cy - 0.4 * size), (cx + 0.35 * size, cy - 0.05 * size));

    // Foam droplets flung off the lip.
    for &(fx, fy, fr) in &[(0.4f32, -0.7f32, 0.06f32), (0.6, -0.55, 0.05), (0.25, -0.78, 0.045)] {
        draw_circle(cx + fx * size, cy + fy * size, (size * fr).max(1.0), col);
    }

    // The water surface: a gentle ripple beneath the swell.
    let base_y = cy + 0.5 * size;
    const STEPS: i32 = 24;
    let mut prev = (cx - size, base_y);
    for i in 1..=STEPS {
        let t = i as f32 / STEPS as f32;
        let x = cx - size + t * 2.0 * size;
        let y = base_y + (t * std::f32::consts::PI * 3.0).sin() * size * 0.07;
        draw_line(prev.0, prev.1, x, y, th * 0.9, col);
        prev = (x, y);
    }
}

/// A fully zoomed-out, hand-drawn chart of the whole world for the captain's log:
/// every archipelago at once, inked as little knots of irregular isles on parchment,
/// each cluster named. Unlike [`render`] this is a *static keepsake map*, not a live
/// instrument: it frames the entire world (not the ship's local waters) and draws no
/// wind, no marks, and **no player**, so it reads as a chart pinned in the logbook.
///
/// `wares` is indexed by cluster id (`clusters[i].id == i`): the legendary trinket the
/// archipelago's shipyard tavern sells (its name + whether it's already in the kit),
/// or `None` for a cluster with no shipyard. Its name is inked under the cluster's, a
/// checkmark beside the ones owned — the very thing the World Map unveils.
pub fn render_world(world: &World, rect: Rect, pal: &MinimapPalette, wares: &[Option<(&str, bool)>]) {
    if pal.panel.a > 0.0 {
        draw_rectangle(rect.x, rect.y, rect.w, rect.h, pal.panel);
    }
    let s = rect.w.min(rect.h) / 300.0; // scale glyph constants off a 300px baseline

    // A double, slightly inset frame for a sketched cartouche look.
    draw_rectangle_lines(rect.x, rect.y, rect.w, rect.h, 2.0, pal.border);
    let inset = 4.0 * s;
    draw_rectangle_lines(rect.x + inset, rect.y + inset, rect.w - 2.0 * inset, rect.h - 2.0 * inset, 1.0, pal.border);

    if world.islands.is_empty() {
        return;
    }

    // Purely a visual breathing-room hack: push each archipelago's *centre* outward
    // from the world's middle by `CLUSTER_SPREAD` (so the clusters drift apart), and
    // push each isle outward from its own cluster centre by `ISLE_SPREAD` (so the isles
    // within a cluster stop overlapping). Both keep the overall layout; the auto-fit
    // below reframes the result.
    const CLUSTER_SPREAD: f32 = 1.6;
    const ISLE_SPREAD: f32 = 1.45;

    // Each isle's cluster centre, indexed by id (islands are in id order, index == id).
    let mut center_of = vec![Vec2::new(0.0, 0.0); world.islands.len()];
    for c in &world.clusters {
        for &id in &c.island_ids {
            center_of[id as usize] = c.center;
        }
    }
    // The anchor we spread away from: the mean of the cluster centres.
    let mut ax = 0.0f32;
    let mut ay = 0.0f32;
    for c in &world.clusters {
        ax += c.center.x;
        ay += c.center.y;
    }
    let n = world.clusters.len().max(1) as f32;
    let (ax, ay) = (ax / n, ay / n);
    // The display position of every isle, with clusters spread apart.
    let disp_pos: Vec<Vec2> = world
        .islands
        .iter()
        .map(|isle| {
            let c = center_of[isle.id as usize];
            Vec2::new(
                ax + (c.x - ax) * CLUSTER_SPREAD + (isle.pos.x - c.x) * ISLE_SPREAD,
                ay + (c.y - ay) * CLUSTER_SPREAD + (isle.pos.y - c.y) * ISLE_SPREAD,
            )
        })
        .collect();

    // Frame the whole (spread-out) world: the bounding box of every isle, centred, with
    // a margin so the outermost archipelagos don't kiss the frame.
    let mut min_x = f32::MAX;
    let mut max_x = f32::MIN;
    let mut min_y = f32::MAX;
    let mut max_y = f32::MIN;
    for p in &disp_pos {
        min_x = min_x.min(p.x);
        max_x = max_x.max(p.x);
        min_y = min_y.min(p.y);
        max_y = max_y.max(p.y);
    }
    let span_x = (max_x - min_x).max(1.0);
    let span_y = (max_y - min_y).max(1.0);
    let world_cx = (min_x + max_x) / 2.0;
    let world_cy = (min_y + max_y) / 2.0;
    let pad = 18.0 * s;
    let avail_w = (rect.w - 2.0 * pad).max(1.0);
    let avail_h = (rect.h - 2.0 * pad).max(1.0);
    // Fit each axis independently so the chart fills the (16:9) frame instead of being
    // letterboxed. A single uniform scale would fit the world's bounding box without
    // clipping, but that box is a touch narrower than 16:9 once each cluster's own girth
    // is added to both axes, so it would become height-bound and leave wide gaps of open
    // sea east and west with the isles bunched in the middle. The small horizontal
    // stretch this introduces is invisible on a stylised chart (the blobs are drawn at a
    // fixed screen radius, so only their spacing shifts). 1.12 keeps a little open sea
    // around the outermost isles.
    let scale_x = avail_w / (span_x * 1.12);
    let scale_y = avail_h / (span_y * 1.12);
    let cx = rect.x + rect.w / 2.0;
    let cy = rect.y + rect.h / 2.0;
    // World x (east) right, world y (north) up, so flip the screen y axis.
    let sx = |p: Vec2| cx + (p.x - world_cx) * scale_x;
    let sy = |p: Vec2| cy - (p.y - world_cy) * scale_y;

    // Port and shipyard screen positions, gathered up front so the nautic lines below
    // can be laid under the isles. Only ports are charted; one shipyard per cluster.
    let mut ports_xy: Vec<(f32, f32)> = Vec::new();
    let mut shipyards_xy: Vec<(f32, f32)> = Vec::new();
    for (idx, isle) in world.islands.iter().enumerate() {
        if !isle.is_port {
            continue;
        }
        let p = (sx(disp_pos[idx]), sy(disp_pos[idx]));
        ports_xy.push(p);
        if isle.is_shipyard {
            shipyards_xy.push(p);
        }
    }

    let rhumb = Color::new(pal.border.r, pal.border.g, pal.border.b, pal.border.a * 0.45);
    let grid_col = Color::new(pal.border.r, pal.border.g, pal.border.b, pal.border.a * 0.22);

    // A faint rectangular grid (graticule) over the chart: evenly spaced lines making
    // roughly square cells. The interior line positions are kept so the compass can be
    // pinned to a grid crossing.
    let cell = rect.h / 6.0;
    let mut xs: Vec<f32> = Vec::new();
    let mut x = rect.x + cell;
    while x < rect.x + rect.w - 1.0 {
        xs.push(x);
        x += cell;
    }
    let mut ys: Vec<f32> = Vec::new();
    let mut y = rect.y + cell;
    while y < rect.y + rect.h - 1.0 {
        ys.push(y);
        y += cell;
    }
    for &gx in &xs {
        draw_line(gx, rect.y, gx, rect.y + rect.h, 1.0, grid_col);
    }
    for &gy in &ys {
        draw_line(rect.x, gy, rect.x + rect.w, gy, 1.0, grid_col);
    }

    // The compass hub, pinned to whichever grid crossing near the chart's middle sits
    // farthest from any port, so the rose lands on an intersection and in free water
    // rather than atop the central archipelago.
    let (mx, my) = (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
    let mut hub = (mx, my);
    let mut best_clear = -1.0f32;
    for &hx in &xs {
        for &hy in &ys {
            if (hx - mx).abs() > rect.w * 0.28 || (hy - my).abs() > rect.h * 0.32 {
                continue;
            }
            let mut clear = f32::MAX;
            for &(ox, oy) in &ports_xy {
                clear = clear.min((hx - ox).hypot(hy - oy));
            }
            if clear > best_clear {
                best_clear = clear;
                hub = (hx, hy);
            }
        }
    }

    // Where a ray from `o` along `d` leaves the rect: the smallest positive crossing of
    // the four edges, so each line of bearing reaches the frame.
    let ray_to_edge = |o: (f32, f32), d: (f32, f32), r: Rect| -> (f32, f32) {
        let mut t = f32::INFINITY;
        if d.0 > 1e-4 {
            t = t.min((r.x + r.w - o.0) / d.0);
        } else if d.0 < -1e-4 {
            t = t.min((r.x - o.0) / d.0);
        }
        if d.1 > 1e-4 {
            t = t.min((r.y + r.h - o.1) / d.1);
        } else if d.1 < -1e-4 {
            t = t.min((r.y - o.1) / d.1);
        }
        if !t.is_finite() {
            return o;
        }
        (o.0 + d.0 * t, o.1 + d.1 * t)
    };
    // Lines of bearing fanning from the compass hub out to the frame edge along the 16
    // points of the compass (every 22.5 degrees, north up), the way a chart's rhumb lines
    // radiate from the rose across the whole sheet.
    for i in 0..16 {
        let theta = i as f32 / 16.0 * std::f32::consts::TAU; // clockwise from north
        let d = (theta.sin(), -theta.cos()); // north up (screen y points down)
        let (ex, ey) = ray_to_edge(hub, d, rect);
        draw_line(hub.0, hub.1, ex, ey, (1.0 * s).max(1.0), rhumb);
    }

    // Each isle: an irregular hand-inked blob (a triangle fan with a wobbled rim, the
    // wobble seeded off the isle id) for the landmass body.
    let blob = |x: f32, y: f32, r: f32, seed: u32| {
        const N: usize = 9;
        let mut vx = [0.0f32; N];
        let mut vy = [0.0f32; N];
        for k in 0..N {
            let ang = k as f32 / N as f32 * std::f32::consts::TAU;
            let rr = r * (0.72 + 0.55 * hash01(seed.wrapping_mul(977).wrapping_add(k as u32)));
            vx[k] = x + ang.cos() * rr;
            vy[k] = y + ang.sin() * rr;
        }
        for k in 0..N {
            let n = (k + 1) % N;
            draw_triangle(vec2(x, y), vec2(vx[k], vy[k]), vec2(vx[n], vy[n]), pal.land);
        }
        for k in 0..N {
            let n = (k + 1) % N;
            draw_line(vx[k], vy[k], vx[n], vy[n], 1.2, pal.border);
        }
    };
    // A small sepia glyph drawn over the blob, telling the isle's terrain apart at a
    // glance: a grass tuft (green), a jagged ridge (rocky), a palm (jungle), or a
    // crater cone (volcanic).
    let col = pal.ship;
    let feature = |x: f32, y: f32, r: f32, kind: IsleKind| match kind {
        IsleKind::Green => {
            for &dx in &[-0.32f32, 0.0, 0.32] {
                let h = if dx == 0.0 { 0.7 } else { 0.5 };
                draw_line(x + dx * r, y + 0.25 * r, x + dx * r, y - h * r, 1.0, col);
            }
        }
        IsleKind::Rocky => {
            let pts = [
                (x - 0.95 * r, y + 0.4 * r),
                (x - 0.45 * r, y - 0.5 * r),
                (x - 0.05 * r, y - 0.05 * r),
                (x + 0.4 * r, y - 0.85 * r),
                (x + 0.95 * r, y + 0.4 * r),
            ];
            for w in pts.windows(2) {
                draw_line(w[0].0, w[0].1, w[1].0, w[1].1, 1.1, col);
            }
        }
        IsleKind::Jungle => {
            let (tx, ty) = (x, y - 0.5 * r);
            draw_line(x, y + 0.6 * r, tx, ty, 1.1, col); // trunk
            for &(fx, fy) in &[(-0.8f32, -0.15f32), (0.8, -0.15), (-0.5, 0.35), (0.5, 0.35)] {
                draw_line(tx, ty, tx + fx * r, ty + fy * r, 1.0, col); // fronds
            }
        }
        IsleKind::Volcanic => {
            let base_y = y + 0.5 * r;
            let rim_y = y - 0.7 * r;
            draw_line(x - 0.9 * r, base_y, x - 0.35 * r, rim_y, 1.1, col);
            draw_line(x + 0.9 * r, base_y, x + 0.35 * r, rim_y, 1.1, col);
            draw_line(x - 0.35 * r, rim_y, x + 0.35 * r, rim_y, 1.1, col); // crater rim
        }
    };
    // Only the ports are charted (the trading isles the captain actually visits): a
    // hand-inked blob plus its terrain glyph (positions gathered in the pre-pass above).
    for (idx, isle) in world.islands.iter().enumerate() {
        if !isle.is_port {
            continue;
        }
        let x = sx(disp_pos[idx]);
        let y = sy(disp_pos[idx]);
        let r = 2.7 * s;
        blob(x, y, r, isle.id as u32 + 1);
        feature(x, y, r, isle.terrain);
    }

    // A ring round each shipyard, over the isles (the bearing lines no longer run through
    // them, but the harbours stay marked).
    for &(spx, spy) in &shipyards_xy {
        draw_circle_lines(spx, spy, 4.0 * s, (1.2 * s).max(1.0), col);
    }

    // Sea-monsters to fill the open water, the way old charts crammed a kraken or a
    // whale into their empty quarters. `find` returns the roomiest spot (farthest from
    // every port, the compass hub, the frame, and any beast already placed) by sampling
    // a grid; we put the kraken in the best gap, then the whale in the next.
    {
        let m = pad + 8.0 * s;
        let (ix, iy) = (rect.x + m, rect.y + m);
        let (iw, ih) = ((rect.w - 2.0 * m).max(1.0), (rect.h - 2.0 * m).max(1.0));
        // `extra` holds (centre, radius) of beasts already drawn, so the clearance keeps
        // its distance from their whole footprint, not just their centre point.
        let find = |extra: &[((f32, f32), f32)]| -> ((f32, f32), f32) {
            let mut best = (ix + iw / 2.0, iy + ih / 2.0);
            let mut best_clear = -1.0f32;
            const G: i32 = 12;
            for gy in 0..=G {
                for gx in 0..=G {
                    let px = ix + iw * gx as f32 / G as f32;
                    let py = iy + ih * gy as f32 / G as f32;
                    let mut clear = (px - ix).min(ix + iw - px).min(py - iy).min(iy + ih - py);
                    for &(ox, oy) in &ports_xy {
                        clear = clear.min((px - ox).hypot(py - oy));
                    }
                    for &((ox, oy), rad) in extra {
                        clear = clear.min((px - ox).hypot(py - oy) - rad);
                    }
                    if clear > best_clear {
                        best_clear = clear;
                        best = (px, py);
                    }
                }
            }
            (best, best_clear)
        };
        // The kraken, in the roomiest quarter (the compass hub is treated as occupied too).
        let (kspot, kclear) = find(&[(hub, 16.0 * s)]);
        let ksize = (kclear * 0.78).min(46.0 * s);
        if kclear > 22.0 * s {
            draw_kraken(kspot.0, kspot.1, ksize, s, pal.ship);
        }
        // The whale, in the next roomiest, kept clear of the kraken's footprint.
        let (wspot, wclear) = find(&[(hub, 16.0 * s), (kspot, ksize)]);
        let wsize = (wclear * 0.8).min(40.0 * s);
        if wclear > 20.0 * s {
            draw_whale(wspot.0, wspot.1, wsize, s, pal.ship);
        }
        // A breaking wave, in the third roomiest gap, clear of both beasts.
        let (vspot, vclear) = find(&[(hub, 16.0 * s), (kspot, ksize), (wspot, wsize)]);
        if vclear > 18.0 * s {
            draw_wave(vspot.0, vspot.1, (vclear * 0.8).min(34.0 * s), s, pal.ship);
        }
    }

    // Name each archipelago beneath its knot of isles, in the heading face so the map
    // reads like a drawn chart. Clamped to stay clear of the frame.
    let fs = ((11.0 * s) as u16).max(10);
    for c in &world.clusters {
        let isles = world.cluster_islands(c);
        if isles.is_empty() {
            continue;
        }
        let mut cminx = f32::MAX;
        let mut cmaxx = f32::MIN;
        let mut cmaxy = f32::MIN;
        for i in &isles {
            let p = disp_pos[i.id as usize];
            let x = sx(p);
            let y = sy(p);
            cminx = cminx.min(x);
            cmaxx = cmaxx.max(x);
            cmaxy = cmaxy.max(y);
        }
        let mid = (cminx + cmaxx) / 2.0;
        let ty = (cmaxy + fs as f32 + 3.0 * s).min(rect.y + rect.h - 4.0 * s);
        crate::font::heading(|| {
            let d = measure_text(&c.name, None, fs, 1.0);
            let tx = (mid - d.width / 2.0).clamp(rect.x + 4.0 * s, rect.x + rect.w - d.width - 4.0 * s);
            draw_text(&c.name, tx, ty, fs as f32, pal.ship);
        });
        // Below the name, the legendary trinket this archipelago's tavern sells (sans
        // face, smaller and dimmer than the name), with a checkmark on the ones owned.
        if let Some(Some((ware, owned))) = wares.get(c.id as usize) {
            let wfs = ((9.0 * s) as u16).max(8);
            let text = if *owned { format!("\u{2713} {ware}") } else { ware.to_string() };
            let d = measure_text(&text, None, wfs, 1.0);
            let wx = (mid - d.width / 2.0).clamp(rect.x + 4.0 * s, rect.x + rect.w - d.width - 4.0 * s);
            let wy = (ty + wfs as f32 + 2.0 * s).min(rect.y + rect.h - 3.0 * s);
            draw_text(&text, wx, wy, wfs as f32, pal.port);
        }
    }

    // The compass rose sits at the hub where the rhumb lines converge, north up: faint
    // cross and saltire with a filled north spike, the flourish of an old chart.
    let rr = 13.0 * s;
    let (ox, oy) = hub;
    draw_line(ox - rr, oy, ox + rr, oy, 1.0, pal.wind_streak);
    draw_line(ox, oy - rr, ox, oy + rr, 1.0, pal.wind_streak);
    let d = rr * 0.42;
    draw_line(ox - d, oy - d, ox + d, oy + d, 1.0, pal.wind_streak);
    draw_line(ox - d, oy + d, ox + d, oy - d, 1.0, pal.wind_streak);
    draw_triangle(vec2(ox, oy - rr), vec2(ox - 2.4 * s, oy), vec2(ox + 2.4 * s, oy), pal.border);
    let nfs = ((10.0 * s) as u16).max(9);
    let nd = measure_text("N", None, nfs, 1.0);
    draw_text("N", ox - nd.width / 2.0, oy - rr - 2.0 * s, nfs as f32, pal.ship);
}
