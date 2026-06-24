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
use crate::isle_features::{FeatureKind, IsleFeature};
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

/// Surface elevation fraction (0..1 of summit height) at radial distance `t`
/// (0..1 of shore radius) on the mound — used to stand features on the slope.
fn mound_h_frac(t: f32) -> f32 {
    let lerp = |a: f32, b: f32, x: f32| a + (b - a) * x;
    if t >= 0.98 {
        0.0
    } else if t >= 0.62 {
        lerp(0.0, 0.46, (0.98 - t) / (0.98 - 0.62))
    } else if t >= 0.32 {
        lerp(0.46, 0.80, (0.62 - t) / (0.62 - 0.32))
    } else {
        lerp(0.80, 1.0, (0.32 - t) / 0.32)
    }
}

/// Draw one island (floor disc + faceted mound + features), or nothing if out of
/// view.
pub fn paint_island(isle: &Island, features: &[IsleFeature], kin: &Kinematics, v: &IslandView) {
    let dist = kin.pos.distance_to(isle.pos);
    let rel = wrap_angle(kin.pos.bearing_to(isle.pos) - kin.heading_rad);
    // Skip isles faded out, off to the side, or ones we've sailed inside of (the
    // ring projection degenerates there). Grounding keeps the hull just outside the
    // shore radius, so culling only strictly-inside keeps big isles visible up close.
    if dist > MAX_VIEW || dist < isle.radius || rel.abs() > v.half_fov_h_view * SIDE_CULL {
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

    // --- Features ------------------------------------------------------------
    // Stand each on the mound's surface at its offset and draw back-to-front so
    // nearer scenery overlays farther.
    let mut order: Vec<usize> = (0..features.len()).collect();
    order.sort_by(|&a, &b| {
        let da = kin.pos.distance_to(isle.pos + features[a].offset);
        let db = kin.pos.distance_to(isle.pos + features[b].offset);
        db.partial_cmp(&da).unwrap()
    });
    // On a tall isle the mound (drawn before the features) would otherwise let
    // far-side scenery show through its peak; cull features beyond the centre.
    // Flat isles occlude nothing, so they keep every feature.
    let to_isle = isle.pos - kin.pos;
    for &fi in &order {
        let f = &features[fi];
        if isle.height > 40.0 && f.offset.dot(to_isle) > 0.0 {
            continue;
        }
        let wp = isle.pos + f.offset;
        let t = (f.offset.length() / isle.radius).min(1.0);
        let base = isle.height * mound_h_frac(t);
        let (fx, fy) = project(wp, base, base < 0.5, kin, v);
        let (_, ty) = project(wp, base + f.height, false, kin, v);
        let h_px = fy - ty;
        if h_px < 1.0 {
            continue;
        }
        let w_px = (h_px * feature_aspect(f.kind) * f.size).max(2.0);
        draw_feature(f.kind, fx, fy, w_px, h_px, alpha);
    }
}

/// Width-to-height ratio of each feature's billboard.
fn feature_aspect(kind: FeatureKind) -> f32 {
    match kind {
        FeatureKind::Tree => 0.85,
        FeatureKind::Palm => 0.8,
        FeatureKind::Bush => 1.7,
        FeatureKind::Rock => 1.4,
        FeatureKind::Ruin => 1.2,
        FeatureKind::Hut => 1.35,
        FeatureKind::Tower => 0.5,
        FeatureKind::Dock => 2.6,
        FeatureKind::Flag => 0.6,
        FeatureKind::Shipwreck => 1.9,
    }
}

// --- Feature billboards (flat-shaded vector shapes) --------------------------
// Drawn in a local space where x ∈ [-0.5, 0.5] and y ∈ [0, 1] (0 = foot at the
// ground, 1 = top), mapped to screen at (cx + lx·w, foot − ly·h). Two-tone where
// it helps imply form, matching the faceted low-poly look.

#[inline]
fn rgba(c: [f32; 3], a: f32) -> Color {
    Color::new(c[0] / 255.0, c[1] / 255.0, c[2] / 255.0, a)
}

fn draw_feature(kind: FeatureKind, cx: f32, foot: f32, w: f32, h: f32, alpha: f32) {
    // Local→screen.
    let p = |lx: f32, ly: f32| vec2(cx + lx * w, foot - ly * h);
    let quad = |x0: f32, y0: f32, x1: f32, y1: f32, c: Color| {
        draw_triangle(p(x0, y0), p(x1, y0), p(x1, y1), c);
        draw_triangle(p(x0, y0), p(x1, y1), p(x0, y1), c);
    };
    let tri = |a: (f32, f32), b: (f32, f32), cc: (f32, f32), c: Color| {
        draw_triangle(p(a.0, a.1), p(b.0, b.1), p(cc.0, cc.1), c);
    };

    const TRUNK: [f32; 3] = [92.0, 64.0, 40.0];
    const CANOPY: [f32; 3] = [52.0, 132.0, 64.0];
    const CANOPY_DK: [f32; 3] = [34.0, 96.0, 48.0];
    const FROND: [f32; 3] = [60.0, 140.0, 72.0];
    const FROND_DK: [f32; 3] = [40.0, 104.0, 56.0];
    const BUSH: [f32; 3] = [66.0, 138.0, 72.0];
    const BUSH_DK: [f32; 3] = [46.0, 106.0, 56.0];
    const ROCK: [f32; 3] = [126.0, 122.0, 114.0];
    const ROCK_DK: [f32; 3] = [94.0, 90.0, 84.0];
    const STONE: [f32; 3] = [156.0, 150.0, 140.0];
    const STONE_DK: [f32; 3] = [116.0, 110.0, 102.0];
    const WALL: [f32; 3] = [198.0, 172.0, 132.0];
    const WALL_DK: [f32; 3] = [160.0, 136.0, 100.0];
    const ROOF: [f32; 3] = [150.0, 72.0, 52.0];
    const ROOF_DK: [f32; 3] = [120.0, 56.0, 40.0];
    const WOOD: [f32; 3] = [126.0, 90.0, 56.0];
    const WOOD_DK: [f32; 3] = [96.0, 66.0, 40.0];
    const FLAGC: [f32; 3] = [205.0, 64.0, 58.0];
    const POLE: [f32; 3] = [82.0, 72.0, 60.0];
    const WRECK: [f32; 3] = [74.0, 56.0, 42.0];
    const WRECK_DK: [f32; 3] = [52.0, 40.0, 30.0];

    match kind {
        FeatureKind::Tree => {
            quad(-0.08, 0.0, 0.08, 0.42, rgba(TRUNK, alpha));
            tri((-0.45, 0.3), (0.0, 0.3), (0.0, 1.0), rgba(CANOPY, alpha));
            tri((0.0, 0.3), (0.45, 0.3), (0.0, 1.0), rgba(CANOPY_DK, alpha));
        }
        FeatureKind::Palm => {
            quad(-0.05, 0.0, 0.06, 0.6, rgba(TRUNK, alpha));
            // Fronds fanning from the crown.
            tri((0.0, 0.55), (-0.5, 0.78), (-0.28, 0.92), rgba(FROND, alpha));
            tri((0.0, 0.55), (-0.18, 0.95), (0.04, 1.02), rgba(FROND, alpha));
            tri((0.0, 0.55), (0.5, 0.8), (0.28, 0.92), rgba(FROND_DK, alpha));
            tri((0.0, 0.55), (0.2, 0.98), (0.0, 1.0), rgba(FROND_DK, alpha));
        }
        FeatureKind::Bush => {
            tri((-0.5, 0.0), (0.5, 0.0), (-0.12, 0.95), rgba(BUSH, alpha));
            tri((0.5, 0.0), (0.12, 0.95), (-0.12, 0.95), rgba(BUSH_DK, alpha));
            tri((-0.5, 0.0), (-0.12, 0.95), (0.12, 0.95), rgba(BUSH, alpha));
        }
        FeatureKind::Rock => {
            // Irregular faceted boulder.
            tri((-0.5, 0.0), (-0.28, 0.7), (0.06, 0.95), rgba(ROCK, alpha));
            tri((-0.5, 0.0), (0.06, 0.95), (0.5, 0.45), rgba(ROCK, alpha));
            tri((0.06, 0.95), (0.5, 0.45), (0.5, 0.0), rgba(ROCK_DK, alpha));
            tri((-0.5, 0.0), (0.5, 0.45), (0.5, 0.0), rgba(ROCK_DK, alpha));
        }
        FeatureKind::Ruin => {
            // A few broken columns on a low base.
            quad(-0.5, 0.0, 0.5, 0.14, rgba(STONE_DK, alpha));
            quad(-0.4, 0.1, -0.22, 0.78, rgba(STONE, alpha));
            quad(-0.08, 0.1, 0.1, 1.0, rgba(STONE, alpha));
            quad(0.24, 0.1, 0.42, 0.55, rgba(STONE_DK, alpha));
        }
        FeatureKind::Hut => {
            quad(-0.4, 0.0, 0.4, 0.6, rgba(WALL, alpha));
            quad(0.0, 0.0, 0.4, 0.6, rgba(WALL_DK, alpha));
            tri((-0.5, 0.55), (0.5, 0.55), (-0.05, 1.0), rgba(ROOF, alpha));
            tri((0.5, 0.55), (-0.05, 1.0), (0.05, 1.0), rgba(ROOF_DK, alpha));
        }
        FeatureKind::Tower => {
            quad(-0.26, 0.0, 0.26, 0.82, rgba(STONE, alpha));
            quad(0.04, 0.0, 0.26, 0.82, rgba(STONE_DK, alpha));
            // Crenellations.
            quad(-0.3, 0.82, -0.12, 1.0, rgba(STONE, alpha));
            quad(-0.06, 0.82, 0.12, 1.0, rgba(STONE_DK, alpha));
            quad(0.18, 0.82, 0.3, 1.0, rgba(STONE_DK, alpha));
        }
        FeatureKind::Dock => {
            quad(-0.5, 0.3, 0.5, 0.62, rgba(WOOD, alpha));
            // Pilings.
            quad(-0.42, 0.0, -0.3, 0.4, rgba(WOOD_DK, alpha));
            quad(0.3, 0.0, 0.42, 0.4, rgba(WOOD_DK, alpha));
        }
        FeatureKind::Flag => {
            quad(-0.04, 0.0, 0.04, 1.0, rgba(POLE, alpha));
            tri((0.04, 0.66), (0.5, 0.82), (0.04, 1.0), rgba(FLAGC, alpha));
        }
        FeatureKind::Shipwreck => {
            // A broken hull canted on the beach with a snapped mast.
            tri((-0.5, 0.1), (0.45, 0.0), (0.5, 0.5), rgba(WRECK, alpha));
            tri((-0.5, 0.1), (0.5, 0.5), (-0.34, 0.55), rgba(WRECK_DK, alpha));
            quad(0.02, 0.45, 0.14, 1.0, rgba(WOOD_DK, alpha));
        }
    }
}
