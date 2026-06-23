//! Low-poly island rendering. Replaces the original SVG billboards (which the
//! game's author disliked, and which let the waves clip through) with cohesive
//! flat-shaded geometry that matches the faceted wave mesh.
//!
//! Each island is a floor disc lying on the sea (a foreshortened ellipse, ported
//! from `IslandFloorRenderer`) plus a faceted "mound" body — concentric rings
//! from the shore up to a summit, triangulated and flat-shaded against the sun in
//! world space. Mechanics are unchanged: islands are placed and sized by
//! `WorldGen`, sit on the waterline at their own distance (so they parallax as
//! you sail around), and ride the swell by the same heave the sea uses.
//!
//! Correct wave occlusion is handled by the caller ([`OceanRenderer::render`]),
//! which draws each island *between* the wave bands by distance — so a near crest
//! rolls in front of a far island's base while its summit stands clear.

use macroquad::prelude::*;

use crate::geometry::{clamp, wrap_angle, Vec2};
use crate::projection::{BASE_EYE, EYE_HEIGHT, MAX_VIEW, SHORE_LIFT};
use crate::sailing::Kinematics;
use crate::world::{Island, IsleKind};

const FLOOR_SEG: usize = 44; // floor ellipse: smooth
const MOUND_SEG: usize = 22; // landmass body: chunky low-poly facets
const SIDE_CULL: f32 = 1.6; // how far off-axis an isle may sit before it's skipped
const AMBIENT: f32 = 0.45; // floor of the directional shading

// Mound profile: (radius fraction of shore, height fraction of summit). The foot
// ring sits on the waterline; inner rings climb to the apex.
const RINGS: [(f32, f32); 3] = [(0.98, 0.0), (0.62, 0.46), (0.32, 0.80)];

const SHADOW: [f32; 3] = [8.0, 40.0, 30.0];

/// Camera/view parameters shared with the wave renderer for one frame.
pub struct IslandView {
    pub w: f32,
    pub horizon: f32,
    pub px_per_rad: f32,
    pub px_per_rad_h: f32,
    pub half_fov_h_view: f32,
    pub eye_rise: f32,
    /// World-space unit vector pointing toward the sun (x, y on chart; z up).
    pub sun: (f32, f32, f32),
}

/// Sand-rim and foliage-interior colours per archetype (matches the original).
fn palette(isle: &Island) -> ([f32; 3], [f32; 3]) {
    match isle.terrain {
        IsleKind::Volcanic => ([78.0, 72.0, 68.0], [58.0, 52.0, 48.0]),
        IsleKind::Rocky => ([150.0, 146.0, 138.0], [96.0, 92.0, 84.0]),
        IsleKind::Jungle => ([232.0, 217.0, 168.0], [31.0, 104.0, 55.0]),
        IsleKind::Green => ([232.0, 217.0, 168.0], [47.0, 143.0, 78.0]),
    }
}

#[inline]
fn col(base: [f32; 3], shade: f32, alpha: f32) -> Color {
    Color::new(
        base[0] / 255.0 * shade,
        base[1] / 255.0 * shade,
        base[2] / 255.0 * shade,
        alpha,
    )
}

/// Project a world point at elevation `z` (m). When `waterline`, use the low
/// waterline eye (so the shore matches the floor disc and the sea); otherwise the
/// real eye height (so summits sit where the billboards used to).
#[inline]
fn project(
    wp: Vec2,
    z: f32,
    waterline: bool,
    kin: &Kinematics,
    v: &IslandView,
) -> (f32, f32) {
    let d = kin.pos.distance_to(wp).max(1.0);
    let rp = wrap_angle(kin.pos.bearing_to(wp) - kin.heading_rad);
    let sx = v.w * 0.5 + rp * v.px_per_rad_h;
    let sy = if waterline {
        v.horizon
            + (((BASE_EYE + v.eye_rise) / d).atan() - (SHORE_LIFT / d).atan()) * v.px_per_rad
    } else {
        v.horizon - ((z - EYE_HEIGHT - v.eye_rise) / d).atan() * v.px_per_rad
    };
    (sx, sy)
}

/// Fill a closed screen polygon (triangle fan from vertex 0).
fn fill_poly(xs: &[f32], ys: &[f32], color: Color) {
    for i in 1..xs.len() - 1 {
        draw_triangle(
            vec2(xs[0], ys[0]),
            vec2(xs[i], ys[i]),
            vec2(xs[i + 1], ys[i + 1]),
            color,
        );
    }
}

/// Flat-shade a triangle from its world-space (x, y, z) corners against the sun.
fn shade(a: (f32, f32, f32), b: (f32, f32, f32), c: (f32, f32, f32), v: &IslandView) -> f32 {
    let (ux, uy, uz) = (b.0 - a.0, b.1 - a.1, b.2 - a.2);
    let (vx, vy, vz) = (c.0 - a.0, c.1 - a.1, c.2 - a.2);
    let (mut nx, mut ny, mut nz) = (uy * vz - uz * vy, uz * vx - ux * vz, ux * vy - uy * vx);
    if nz < 0.0 {
        nx = -nx;
        ny = -ny;
        nz = -nz;
    }
    let len = (nx * nx + ny * ny + nz * nz).sqrt().max(1e-6);
    let diff = ((nx * v.sun.0 + ny * v.sun.1 + nz * v.sun.2) / len).max(0.0);
    AMBIENT + (1.0 - AMBIENT) * diff
}

/// Draw one island (floor disc + faceted mound), or nothing if out of view.
pub fn paint_island(isle: &Island, kin: &Kinematics, v: &IslandView) {
    let dist = kin.pos.distance_to(isle.pos);
    let rel = wrap_angle(kin.pos.bearing_to(isle.pos) - kin.heading_rad);
    // Skip isles faded out, off to the side, or ones we've sailed onto.
    if dist > MAX_VIEW || dist < isle.radius * 1.1 || rel.abs() > v.half_fov_h_view * SIDE_CULL {
        return;
    }
    let alpha = clamp((1.0 - dist / MAX_VIEW) * 1.5, 0.0, 1.0);
    let (sand, foliage) = palette(isle);

    // --- Floor disc: shadow, sand rim, foliage interior (waterline rings) ----
    let mut xs = [0.0f32; FLOOR_SEG];
    let mut ys = [0.0f32; FLOOR_SEG];
    let mut floor_ring = |r: f32, color: Color| {
        for i in 0..FLOOR_SEG {
            let a = i as f32 / FLOOR_SEG as f32 * std::f32::consts::TAU;
            let wp = Vec2::new(isle.pos.x + a.cos() * r, isle.pos.y + a.sin() * r);
            let (sx, sy) = project(wp, 0.0, true, kin, v);
            xs[i] = sx;
            ys[i] = sy;
        }
        fill_poly(&xs, &ys, color);
    };
    floor_ring(isle.radius * 1.06, col(SHADOW, 1.0, alpha * 0.45));
    floor_ring(isle.radius * 0.98, col(sand, 1.0, alpha));
    floor_ring(isle.radius * 0.70, col(foliage, 1.0, alpha));

    // --- Faceted mound body --------------------------------------------------
    let h = isle.height;
    // Screen + world coords of each ring's vertices, plus the apex.
    let mut ring_sx = [[0.0f32; MOUND_SEG]; 3];
    let mut ring_sy = [[0.0f32; MOUND_SEG]; 3];
    let mut ring_w = [[(0.0f32, 0.0f32, 0.0f32); MOUND_SEG]; 3];
    for (ri, &(rfrac, hfrac)) in RINGS.iter().enumerate() {
        let r = isle.radius * rfrac;
        let z = h * hfrac;
        for i in 0..MOUND_SEG {
            let a = i as f32 / MOUND_SEG as f32 * std::f32::consts::TAU;
            let wp = Vec2::new(isle.pos.x + a.cos() * r, isle.pos.y + a.sin() * r);
            let (sx, sy) = project(wp, z, hfrac == 0.0, kin, v);
            ring_sx[ri][i] = sx;
            ring_sy[ri][i] = sy;
            ring_w[ri][i] = (wp.x, wp.y, z);
        }
    }
    let (apex_sx, apex_sy) = project(isle.pos, h, false, kin, v);
    let apex_w = (isle.pos.x, isle.pos.y, h);

    // Painter's order within the island: draw slices back-to-front so the front
    // facets overlay the far side without any depth buffer.
    let mut order: [usize; MOUND_SEG] = [0; MOUND_SEG];
    for (i, slot) in order.iter_mut().enumerate() {
        *slot = i;
    }
    order.sort_by(|&a, &b| {
        let da = kin.pos.distance_to(Vec2::new(ring_w[0][a].0, ring_w[0][a].1));
        let db = kin.pos.distance_to(Vec2::new(ring_w[0][b].0, ring_w[0][b].1));
        db.partial_cmp(&da).unwrap()
    });

    for &i in order.iter() {
        let j = (i + 1) % MOUND_SEG;
        // Bottom strip (shore→ring1): sandy. Upper strip (ring1→ring2): foliage.
        for (lvl, base) in [(0usize, sand), (1usize, foliage)] {
            let a = ring_w[lvl][i];
            let b = ring_w[lvl][j];
            let c = ring_w[lvl + 1][j];
            let d = ring_w[lvl + 1][i];
            let sh = shade(a, b, c, v);
            let color = col(base, sh, alpha);
            draw_triangle(
                vec2(ring_sx[lvl][i], ring_sy[lvl][i]),
                vec2(ring_sx[lvl][j], ring_sy[lvl][j]),
                vec2(ring_sx[lvl + 1][j], ring_sy[lvl + 1][j]),
                color,
            );
            draw_triangle(
                vec2(ring_sx[lvl][i], ring_sy[lvl][i]),
                vec2(ring_sx[lvl + 1][j], ring_sy[lvl + 1][j]),
                vec2(ring_sx[lvl + 1][i], ring_sy[lvl + 1][i]),
                color,
            );
        }
        // Cap (ring2→apex).
        let sh = shade(ring_w[2][i], ring_w[2][j], apex_w, v);
        draw_triangle(
            vec2(ring_sx[2][i], ring_sy[2][i]),
            vec2(ring_sx[2][j], ring_sy[2][j]),
            vec2(apex_sx, apex_sy),
            col(foliage, sh, alpha),
        );
    }
}
