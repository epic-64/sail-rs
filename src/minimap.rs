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
//! Drawn straight to the screen (after `set_default_camera`), on parchment
//! (`MinimapPalette::parchment`): it serves the captain's-log chart and the port
//! board's leg preview. (The always-on chart the captain steers by is the deck
//! chart at the wheel, see [`crate::ship_render::DeckChart`].)

use macroquad::prelude::*;

use crate::font::{draw_text, measure_text};
use crate::geometry::Vec2;
use crate::sailing::{Kinematics, Wind};
use crate::world::{IsleKind, World};

/// Make a colour from 0–255 channels plus an alpha in [0,1].
fn rgba(r: u8, g: u8, b: u8, a: f32) -> Color {
    Color::new(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, a)
}

/// The minimap's ink colours, inked onto the logbook / port-board parchment.
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

/// Paint the chart into the square `rect` (screen space). `mission_targets` mark
/// the isles that hold an active contract's destination — a yellow ring with an
/// "M" (empty until missions land); `race_targets` mark the booked race's mark —
/// a red ring with an "R". `route`, if set, draws a straight rhumb line between two
/// world points (the docked port and a highlighted contract's or race's other port),
/// ringing the target in brown, so the captain can weigh a leg against the wind before
/// taking it.
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
    // The port the captain is docked at, ringed in brown as a "you are here" mark.
    // `None` off the water (the HUD map and the log, where the ship isn't in port).
    home: Option<Vec2>,
) {
    // Panel + frame. (Parchment's panel is opaque beige; the HUD's is dark glass.)
    if pal.panel.a > 0.0 {
        draw_rectangle(rect.x, rect.y, rect.w, rect.h, pal.panel);
    }
    draw_rectangle_lines(rect.x, rect.y, rect.w, rect.h, 2.0, pal.border);

    let size = rect.w.min(rect.h);
    let s = size / 168.0; // scale every CSS-pixel constant off the original 168px map
    // The isle marks (dots, rings, letters) and the route line size off `ms`, a scale
    // capped just above `ui::scale`. On the corner minimap `s` already tracks
    // `ui::scale`, so this leaves it untouched; the captain's-log chart is drawn ~2x
    // larger, where an uncapped `s` would bloat the whole cluster (dots swallowing their
    // own rings). Capping keeps the marks the same crisp absolute size on both charts.
    // The rest of the chart (ship arrow, wind streaks) still tracks the full `s`.
    let ms = s.min(1.2 * crate::ui::scale());
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

    // The docked port, ringed brown as a "you are here" mark (drawn under the island
    // dots so its own marker sits on top).
    if let Some(h) = home {
        let (hx, hy) = (sx(h), sy(h));
        if rect.contains(vec2(hx, hy)) {
            draw_circle_lines(hx, hy, SELECT_RING * ms, (1.6 * ms).max(1.2), pal.border);
        }
    }

    // A straight rhumb line between the docked port and the highlighted contract's
    // other port, drawn under the island dots so the markers sit on top, with a brown
    // ring round the selected target so it reads as the picked destination.
    if let Some((from, to)) = route {
        if let Some((ax, ay, bx, by)) =
            clip_segment(sx(from), sy(from), sx(to), sy(to), rect)
        {
            draw_line(ax, ay, bx, by, (1.6 * ms).max(1.2), pal.mission_mark);
        }
        let (tx, ty) = (sx(to), sy(to));
        if rect.contains(vec2(tx, ty)) {
            draw_circle_lines(tx, ty, SELECT_RING * ms, (1.6 * ms).max(1.2), pal.border);
        }
    }

    // Isle-fitting ring radii, in design pixels (scaled by `ms`). A lone fitting sits at
    // the base radius; when an isle carries several, each successive ring steps out by
    // `RING_STEP` so they nest visibly rather than one hiding another.
    const RING_BASE: f32 = 3.5;
    const RING_STEP: f32 = 2.0;
    const LETTER_DIST: f32 = 11.0;
    // A brown ring marking the docked port and the selected target, hugging the isle as a
    // highlight of the leg's end.
    const SELECT_RING: f32 = 7.0;
    // Every isle in view: a dot each (ports brighter). Its fittings (a shipyard, a
    // booked mission, a booked race) each add a ring and a letter. A lone fitting rings
    // the isle at the base radius; stacked fittings step outward (blue shipyard, yellow
    // mission, red race), drawn smallest first so a larger never hides a smaller. Each
    // letter sits the same distance from the isle: a lone mark rides straight above,
    // while two or more fan out to the sides so they never collide (angles by count).
    let fs = (11.0 * ms).max(9.0);
    let letter = |x: f32, y: f32, glyph: &str, col: Color, ang_deg: f32| {
        let a = ang_deg.to_radians();
        let lx = x + LETTER_DIST * ms * a.sin();
        let ly = y - LETTER_DIST * ms * a.cos();
        let dims = measure_text(glyph, None, fs as u16, 1.0);
        draw_text(glyph, lx - dims.width / 2.0, ly + dims.offset_y / 2.0, fs, col);
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
        let r = if isle.is_port { 3.2 } else { 2.4 } * ms;
        draw_circle(x, y, r, if isle.is_port { pal.port } else { pal.land });

        let is_mission = mission_targets.contains(&isle.id);
        let is_race = race_targets.contains(&isle.id);
        // Rings, smallest first: each present fitting steps out one slot from the base.
        let mut slot = 0;
        for (present, col, th) in [
            (isle.is_shipyard, pal.shipyard_ring, 1.5),
            (is_mission, pal.mission_mark, 2.0),
            (is_race, pal.race_mark, 2.0),
        ] {
            if present {
                draw_circle_lines(x, y, (RING_BASE + slot as f32 * RING_STEP) * ms, th, col);
                slot += 1;
            }
        }
        // Letters: gather whichever marks this isle carries, then fan them across the
        // top by count. One mark sits dead centre above; two or more spread to the
        // sides, symmetric about the top.
        let mut marks: Vec<(&str, Color)> = Vec::new();
        if isle.is_shipyard {
            marks.push(("S", pal.shipyard_ring));
        }
        if is_mission {
            marks.push(("M", pal.mission_mark));
        }
        if is_race {
            marks.push(("R", pal.race_mark));
        }
        let angles: &[f32] = match marks.len() {
            0 | 1 => &[0.0],
            2 => &[-35.0, 35.0],
            _ => &[-52.0, 0.0, 52.0],
        };
        for (i, (glyph, col)) in marks.iter().enumerate() {
            letter(x, y, glyph, *col, angles[i]);
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
/// hand-drawn coastlines on the world map (and the kraken flourish in [`crate::map_kraken`]).
/// Deterministic so the chart looks the same every time the captain flips to it (no RNG
/// draws, so world generation is untouched).
pub(crate) fn hash01(n: u32) -> f32 {
    let mut x = n.wrapping_mul(0x9e3779b1);
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb352d);
    x ^= x >> 15;
    (x & 0xffff) as f32 / 65535.0
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
    // roughly square cells. The lines are laid out symmetrically outward from the chart's
    // exact centre so that a crossing always lands dead centre, where the compass is pinned.
    let (mx, my) = (rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
    let cell = rect.h / 6.0;
    let mut xs: Vec<f32> = vec![mx];
    let mut off = cell;
    while mx - off > rect.x + 1.0 || mx + off < rect.x + rect.w - 1.0 {
        if mx - off > rect.x + 1.0 {
            xs.push(mx - off);
        }
        if mx + off < rect.x + rect.w - 1.0 {
            xs.push(mx + off);
        }
        off += cell;
    }
    let mut ys: Vec<f32> = vec![my];
    let mut off = cell;
    while my - off > rect.y + 1.0 || my + off < rect.y + rect.h - 1.0 {
        if my - off > rect.y + 1.0 {
            ys.push(my - off);
        }
        if my + off < rect.y + rect.h - 1.0 {
            ys.push(my + off);
        }
        off += cell;
    }
    for &gx in &xs {
        draw_line(gx, rect.y, gx, rect.y + rect.h, 1.0, grid_col);
    }
    for &gy in &ys {
        draw_line(rect.x, gy, rect.x + rect.w, gy, 1.0, grid_col);
    }

    // The compass hub sits on the central grid crossing, dead centre of the chart, so the
    // rose lands squarely on an intersection with the rhumb lines fanning out symmetrically.
    let hub = (mx, my);

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
    // A small sepia glyph drawn over the blob, telling the isle's terrain apart at
    // a glance (one arm per `IsleKind` below: a grass tuft, a ridge, a palm...).
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
        IsleKind::Tropical => {
            // A palm leaning over its beach: slanted trunk, fronds drooping seaward.
            let (tx, ty) = (x + 0.35 * r, y - 0.45 * r);
            draw_line(x - 0.25 * r, y + 0.6 * r, tx, ty, 1.1, col); // leaning trunk
            for &(fx, fy) in &[(-0.75f32, 0.05f32), (0.7, 0.2), (-0.35, 0.45), (0.4, 0.5)] {
                draw_line(tx, ty, tx + fx * r, ty + fy * r, 1.0, col); // fronds
            }
        }
        IsleKind::Desert => {
            // A saguaro cactus: upright trunk with two upturned arms.
            draw_line(x, y + 0.6 * r, x, y - 0.7 * r, 1.1, col); // trunk
            draw_line(x, y + 0.05 * r, x - 0.4 * r, y + 0.05 * r, 1.0, col);
            draw_line(x - 0.4 * r, y + 0.05 * r, x - 0.4 * r, y - 0.35 * r, 1.0, col);
            draw_line(x, y - 0.15 * r, x + 0.4 * r, y - 0.15 * r, 1.0, col);
            draw_line(x + 0.4 * r, y - 0.15 * r, x + 0.4 * r, y - 0.5 * r, 1.0, col);
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
        draw_circle_lines(spx, spy, 3.2 * s, (1.2 * s).max(1.0), col);
    }

    // Sea-monsters to fill the open water, the way old charts crammed a kraken or a
    // whale into their empty quarters. Each is inked in the roomiest empty pocket of
    // sea: the point farthest from every charted port, the compass hub, an archipelago
    // name, the sheet border, and the beasts already placed (see [`ornament_slots`]), so
    // the beasts settle into the widest gaps of open water instead of crowding the isles.
    {
        // The drawable sea, inset a little off the double frame line. Kept generous (only
        // `pad`, not a wider berth) so a broad empty margin still earns an ornament rather
        // than being left blank.
        let sea = Rect::new(
            rect.x + pad,
            rect.y + pad,
            (rect.w - 2.0 * pad).max(1.0),
            (rect.h - 2.0 * pad).max(1.0),
        );

        // What the beasts must stand clear of, each a centre + keep-out radius: every
        // charted port (a touch past its blob so a harbour is never crowded), the compass
        // rose at the hub, and the name inked beneath each archipelago, so no ornament is
        // drawn across a label. The name geometry mirrors the label pass further below.
        let mut blocked: Vec<((f32, f32), f32)> = ports_xy.iter().map(|&p| (p, 3.6 * s)).collect();
        blocked.push((hub, 15.0 * s));
        let name_fs = ((11.0 * s) as u16).max(10);
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
                cminx = cminx.min(sx(p));
                cmaxx = cmaxx.max(sx(p));
                cmaxy = cmaxy.max(sy(p));
            }
            let mid = (cminx + cmaxx) / 2.0;
            let ty = (cmaxy + name_fs as f32 + 3.0 * s).min(rect.y + rect.h - 4.0 * s);
            let half_w = crate::font::heading(|| measure_text(&c.name, None, name_fs, 1.0).width) * 0.5;
            blocked.push(((mid, ty - name_fs as f32 * 0.35), half_w + 2.0 * s));
        }

        // One pocket per beast, roomiest first. A beast's bulk spans about `FILL_REACH`
        // times its `size`, so a pocket of radius `clear` draws one of `clear /
        // FILL_REACH` to fill it (its thinnest fluke/arm tips may reach a touch further,
        // into the berth already baked into each blocked radius), capped so a vast empty
        // sheet doesn't blow one monster out of all proportion, and skipped when a gap is
        // too cramped to read the flourish. The beasts share a signature (`cx, cy, size,
        // s, col`), so one loop inks them all.
        const FILL_REACH: f32 = 1.25;
        let draw: [(fn(f32, f32, f32, f32, Color), f32, f32); 3] = [
            (crate::map_kraken::draw_kraken, 52.0, 22.0),
            (crate::map_whale::draw_whale, 46.0, 20.0),
            (crate::map_wave::draw_wave, 40.0, 18.0),
        ];
        let slots = ornament_slots(sea, &blocked, draw.len());
        for (&((cx, cy), clear), &(ink, cap, floor)) in slots.iter().zip(&draw) {
            if clear > floor * s {
                ink(cx, cy, (clear / FILL_REACH).min(cap * s), s, pal.ship);
            }
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

    // The compass rose sits at the hub where the rhumb lines converge, north up: a
    // sixteen-point star inside a double ring, each point split down its axis into a
    // parchment-lit face and a sepia-shaded face so the rose reads as embossed on the
    // sheet. Cardinal points reach farthest, then the intercardinals, then the short
    // half-wind points, with the four cardinals lettered in the heading face.
    let (ox, oy) = hub;
    let r = 14.0 * s;
    let dark = pal.ship;
    let light = pal.panel;
    let ring_o = r * 0.52;
    let ring_i = r * 0.32;

    // A star point: a slim rhombus from the hub out to `len`, split lengthwise into a
    // lit half (`light`) and a shaded half (`dark`). Cardinal points are outlined so
    // their edges stay crisp on the beige; the shorter points are left un-outlined so
    // the centre of the rose doesn't clot with ink.
    let point = |ang: f32, len: f32, wide: f32, outline: bool| {
        let (dx, dy) = (ang.sin(), -ang.cos()); // north up, clockwise from N
        let (px, py) = (-dy, dx); // perpendicular
        let hub_v = vec2(ox, oy);
        let tip = vec2(ox + dx * len, oy + dy * len);
        let waist = len * 0.30;
        let s1 = vec2(ox + dx * waist + px * wide, oy + dy * waist + py * wide);
        let s2 = vec2(ox + dx * waist - px * wide, oy + dy * waist - py * wide);
        draw_triangle(tip, s1, hub_v, light);
        draw_triangle(tip, hub_v, s2, dark);
        draw_line(hub_v.x, hub_v.y, tip.x, tip.y, 1.0, pal.border);
        if outline {
            draw_line(tip.x, tip.y, s1.x, s1.y, 1.0, pal.border);
            draw_line(tip.x, tip.y, s2.x, s2.y, 1.0, pal.border);
            draw_line(hub_v.x, hub_v.y, s1.x, s1.y, 1.0, pal.border);
            draw_line(hub_v.x, hub_v.y, s2.x, s2.y, 1.0, pal.border);
        }
    };

    // Faint degree ticks around the rim, then the double ring the star sits within.
    for i in 0..32 {
        let a = i as f32 / 32.0 * std::f32::consts::TAU;
        let (dx, dy) = (a.sin(), -a.cos());
        let t1 = ring_o + (if i % 4 == 0 { 2.6 } else { 1.4 }) * s;
        draw_line(ox + dx * ring_o, oy + dy * ring_o, ox + dx * t1, oy + dy * t1, 1.0, pal.wind_streak);
    }
    draw_circle_lines(ox, oy, ring_o, 1.0, pal.border);
    draw_circle_lines(ox, oy, ring_i, 1.0, pal.wind_streak);

    // Sixteen points: half-winds and intercardinals first, the four cardinals last so
    // they sit on top and reach farthest.
    for i in 0..16 {
        if i % 4 == 0 {
            continue; // cardinals drawn below
        }
        let ang = i as f32 / 16.0 * std::f32::consts::TAU;
        if i % 4 == 2 {
            point(ang, r * 0.60, r * 0.070, false); // intercardinal (NE, SE, SW, NW)
        } else {
            point(ang, r * 0.44, r * 0.045, false); // half-wind
        }
    }
    for i in 0..4 {
        point(i as f32 / 4.0 * std::f32::consts::TAU, r, r * 0.10, true);
    }

    // A filled bead where the points meet, ringed by a parchment pip.
    draw_circle(ox, oy, r * 0.09, dark);
    draw_circle(ox, oy, r * 0.04, light);

    // Cardinal letters just outside the rim, in the heading face; N a touch larger so
    // north reads at a glance.
    let lr = r + 3.0 * s;
    let letter = |txt: &str, cx: f32, cy: f32, fs: u16| {
        crate::font::heading(|| {
            let d = measure_text(txt, None, fs, 1.0);
            draw_text(txt, cx - d.width / 2.0, cy + d.offset_y / 2.0, fs as f32, pal.ship);
        });
    };
    let cfs = ((9.0 * s) as u16).max(8);
    let nfs = ((11.0 * s) as u16).max(9);
    letter("N", ox, oy - lr, nfs);
    letter("E", ox + lr, oy, cfs);
    letter("S", ox, oy + lr, cfs);
    letter("W", ox - lr, oy, cfs);
}

/// Find the large empty areas of the chart to drop the map's ornaments (kraken, whale,
/// wave) into, so they fill open water instead of crowding the isles, and return `n`
/// slots as (centre, radius). For each ornament the method is the plain one:
///
///   1. **Identify a large empty area.** Rasterise the drawable `sea` into a grid and
///      score every cell by how open it is (its distance to the nearest island, the
///      compass hub, a name in `blocked`, or the sea border). The open water is the
///      positive-scoring cells; the emptiest area is the connected patch of them around
///      the single widest cell (cells nearly as open as that peak).
///   2. **Determine the centre of that area.** Take the patch's centroid, so a broad or
///      long pocket is centred in its middle rather than pinned to one end.
///   3. **Place the artwork there.** The slot's `radius` is the openness at that centre
///      (how big an ornament fits); the caller sizes and skips off it.
///
/// The chosen area is then struck off so the next ornament lands in a different pocket.
/// `radius` can come back small (a tight chart) or `0` (no room); the caller decides
/// what is roomy enough to draw. Deterministic (a fixed grid, no RNG), so the chart
/// looks the same every time the captain opens the log.
pub(crate) fn ornament_slots(
    sea: Rect,
    blocked: &[((f32, f32), f32)],
    n: usize,
) -> Vec<((f32, f32), f32)> {
    // Rasterise the sea. `cols` sets the resolution across the wider run of the sheet;
    // the cell is square, so the row count follows from the sheet's height.
    let cols = 64usize;
    let cell = (sea.w / cols as f32).max(1.0);
    let rows = ((sea.h / cell).round() as usize).max(1);
    let ccx = |c: usize| sea.x + (c as f32 + 0.5) * cell;
    let ccy = |r: usize| sea.y + (r as f32 + 0.5) * cell;

    // How open a point is: its distance to the nearest island/hub/name rim or the sea
    // border. Positive is open water; negative is inside something an ornament must miss.
    let openness = |px: f32, py: f32| -> f32 {
        let mut c = (px - sea.x).min(sea.x + sea.w - px).min(py - sea.y).min(sea.y + sea.h - py);
        for &((ox, oy), rad) in blocked {
            c = c.min((px - ox).hypot(py - oy) - rad);
        }
        c
    };
    let score: Vec<f32> = (0..rows * cols).map(|k| openness(ccx(k % cols), ccy(k / cols))).collect();
    // Which cells are still free to take an ornament (open water, not yet struck off).
    let mut free: Vec<bool> = score.iter().map(|&s| s > 0.0).collect();

    let neighbours = |k: usize| {
        let (r, c) = (k / cols, k % cols);
        let mut v = [usize::MAX; 4];
        let mut i = 0;
        for (ok, nb) in [(r > 0, k.wrapping_sub(cols)), (r + 1 < rows, k + cols), (c > 0, k.wrapping_sub(1)), (c + 1 < cols, k + 1)] {
            if ok {
                v[i] = nb;
                i += 1;
            }
        }
        (v, i)
    };

    let mut slots = Vec::with_capacity(n);
    for _ in 0..n {
        // (1) The widest point of the open water still free.
        let peak = (0..rows * cols).filter(|&k| free[k]).max_by(|&a, &b| score[a].total_cmp(&score[b]));
        let Some(peak) = peak else {
            slots.push(((sea.x + sea.w / 2.0, sea.y + sea.h / 2.0), 0.0));
            continue;
        };
        // The large empty area around it: the connected patch of free cells nearly as
        // open as the peak (a broad pocket, not just that one cell).
        let thresh = score[peak] * 0.85;
        let mut area = Vec::new();
        let mut seen = vec![false; rows * cols];
        let mut stack = vec![peak];
        seen[peak] = true;
        while let Some(k) = stack.pop() {
            area.push(k);
            let (nb, cnt) = neighbours(k);
            for &m in &nb[..cnt] {
                if free[m] && !seen[m] && score[m] >= thresh {
                    seen[m] = true;
                    stack.push(m);
                }
            }
        }
        // (2) Its centre: the centroid of the patch.
        let (mut sxp, mut syp) = (0.0f32, 0.0f32);
        for &k in &area {
            sxp += ccx(k % cols);
            syp += ccy(k / cols);
        }
        let centre = (sxp / area.len() as f32, syp / area.len() as f32);
        let radius = openness(centre.0, centre.1).max(0.0);
        // (3) The slot. Then strike off the pocket around it (the inscribed circle) so
        // the next ornament finds a different empty area.
        slots.push((centre, radius));
        for k in 0..rows * cols {
            if free[k] && (ccx(k % cols) - centre.0).hypot(ccy(k / cols) - centre.1) <= radius {
                free[k] = false;
            }
        }
    }
    slots
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each placement sits in open water (its reported `radius` is the real clearance to
    /// the border and every blocked rim), and the three land in distinct areas rather
    /// than piling into one.
    #[test]
    fn ornaments_fill_the_gaps() {
        let sea = Rect::new(0.0, 0.0, 300.0, 200.0);
        // Two clumps of "ports" near the left and right, leaving open water down the
        // middle and top/bottom bands.
        let blocked = [
            ((60.0, 100.0), 10.0),
            ((80.0, 90.0), 10.0),
            ((240.0, 110.0), 10.0),
            ((220.0, 95.0), 10.0),
        ];
        let slots = ornament_slots(sea, &blocked, 3);
        assert_eq!(slots.len(), 3);
        for &((x, y), radius) in &slots {
            // Inside the sea.
            assert!(x >= sea.x && x <= sea.x + sea.w && y >= sea.y && y <= sea.y + sea.h);
            // The reported radius is real: the centre truly is that far from the border
            // and every blocked circle's rim.
            let edge = (x - sea.x).min(sea.x + sea.w - x).min(y - sea.y).min(sea.y + sea.h - y);
            assert!(radius <= edge + 1e-3);
            for &((ox, oy), rad) in &blocked {
                assert!((x - ox).hypot(y - oy) - rad >= radius - 1e-3);
            }
            // A pocket wide enough to be worth an ornament was found.
            assert!(radius > 20.0, "pocket too small: {radius}");
        }
        // The three don't stack: each pair is well separated (a struck-off pocket bars
        // the next ornament from landing on top of it).
        for i in 0..slots.len() {
            for j in i + 1..slots.len() {
                let (a, b) = (slots[i], slots[j]);
                let d = (a.0 .0 - b.0 .0).hypot(a.0 .1 - b.0 .1);
                assert!(d > a.1.min(b.1), "slots {i}/{j} too close: {d}");
            }
        }
    }
}
