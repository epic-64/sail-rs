//! A small top-down chart, ported from `client.MinimapRenderer`.
//!
//! The local cluster — the waters the ship is currently in — is drawn zoomed right
//! out so every island is just a dot (ports a little brighter, shipyards ringed and
//! lettered "S"),
//! with the ship a heading arrow at its world position. North is up. Faint wind
//! streaks (with chevrons) flow across the chart along the wind. When the ship
//! strays out toward open sea its arrow clamps to the frame edge rather than flying
//! off the chart.
//!
//! Drawn straight to the screen (after `set_default_camera`), so the same renderer
//! serves both the always-on corner HUD map (`MinimapPalette::hud`) and the
//! captain's-log chart on parchment (`MinimapPalette::parchment`).

use macroquad::prelude::*;

use crate::geometry::Vec2;
use crate::sailing::{Kinematics, Wind};
use crate::world::World;

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

    // Frame the ship's current local waters so its isles nearly fill the chart.
    let cluster = world.cluster_at(kin.pos);
    let (bbox_c, half_span) = world.cluster_bounds(cluster);
    let frame = half_span * 1.06;
    let scale = (size / 2.0 - pad) / frame;
    // Centre on the cluster, but slide the view to keep the ship on the chart when
    // it strays toward (or past) the cluster's edge — never more than a frame-half
    // from the ship.
    let view_x = bbox_c
        .x
        .min(kin.pos.x + frame)
        .max(kin.pos.x - frame);
    let view_y = bbox_c
        .y
        .min(kin.pos.y + frame)
        .max(kin.pos.y - frame);
    // World x → right, world y (north) → up, so flip the screen y axis.
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

    // The isles of the local cluster: a dot each (ports brighter), shipyards ringed.
    // A mission destination gets a yellow ring with an "M"; a race mark gets a red
    // ring with an "R" — both drawn on top of all. A small helper rings the isle and
    // letters it just above the ring, clear of it.
    let mark = |x: f32, y: f32, letter: &str, col: Color, rr: f32| {
        draw_circle_lines(x, y, rr, 2.0, col);
        let fs = (13.0 * s).max(11.0);
        let dims = measure_text(letter, None, fs as u16, 1.0);
        draw_text(letter, x - dims.width / 2.0, y - rr - 3.0 * s, fs, col);
    };
    for isle in world.cluster_islands(cluster) {
        let x = sx(isle.pos);
        let y = sy(isle.pos);
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
