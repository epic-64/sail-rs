//! Drawing the racing rival and the wandering traders as true low-poly 3-D
//! miniatures. The hull is lofted from a [`crate::hull_shape`] station table
//! (the rival sails the tier her race demands, the traders their small
//! single-decked coasters) and the rig from [`crate::ship_render`]'s spar and
//! sail dimensions, so a ship crossing the bay is recognisably one of the
//! shipyard's own hulls seen from without, and reshaping a hull there
//! reshapes her here. Replaces the flat two-tone sloop billboard (itself a
//! stand-in for the original's `ship.svg`/`ship-bow.svg`/`ship-stern.svg`
//! sprites).
//!
//! Every vertex is projected through the same cylindrical map the waves and
//! islands use (column = bearing, row = depression angle; see
//! [`crate::islands_render`]), so the hull sits *in* the swell and turns
//! honestly as she and the player manoeuvre: bow-on she foreshortens,
//! broadside she shows her full run. macroquad's 2D pass has no depth buffer,
//! so her faces are painter-sorted by range and shaded per face against the
//! scene's active light, exactly like the island facets.
//!
//! The hull rides the local wave, and heels and pitches with the swell sampled
//! along her own beam and keel line, so she works through the same sea the
//! player feels rather than standing bolt upright on it.

use macroquad::prelude::*;

use crate::geometry::{clamp, wrap_angle, Vec2};
use crate::hull_shape::HullShape;
use crate::ocean;
use crate::ocean_renderer::WAVE_GAIN;
use crate::projection::{curve_dip, BASE_EYE, MAX_VIEW};
use crate::sailing::{wind_factor_rel, Kinematics};
use crate::scene::SceneView;
use crate::ship_render::{
    mast_top, sail_cuts, BELLY_DEPTH, BRACE_LIMIT, DECK_B, MAST_HW, RAIL, SAIL_CLOTH,
    SAIL_STANDOFF_M, SPAR,
};

use std::f32::consts::PI;

const RIVAL_MAG: f32 = 1.35; // drawn a touch larger than life
// Masthead floor (px) so a distant hull stays a readable fleck rather than a
// sub-pixel shimmer. Kept low: she now shrinks with true perspective like the
// islands and is removed by sinking under the horizon (see `dip`), so she no
// longer needs to hold a big fixed size to stay spottable while fading. A high
// floor froze her on-screen size (and fattened her footprint) just a few hundred
// metres out, which read as a far ship drawn weirdly huge.
const RIVAL_MIN_PX: f32 = 10.0;
const FOV_MARGIN: f32 = 1.12; // matches the wave mesh's column fan

// The exterior the player's first-person loft never shows, so these live here
// rather than in ship_render's palette (the freeboard is the hull shape's own).
const WL_TUCK: f32 = 0.62; // half-beam left at the waterline as the hull tucks toward the keel
const HULL: [f32; 3] = [96.0, 68.0, 42.0];
const HULL_DK: [f32; 3] = [62.0, 44.0, 28.0];


/// Floor under the per-face Lambert term, the sky-fill washing shadowed faces.
const AMBIENT: f32 = 0.45;

/// The racing rival flies a defiant red pennant; the wandering traders a calmer
/// sea-green one, so a sail crossing the bay reads as friendly traffic at a glance.
pub const RIVAL_PENNANT: [f32; 3] = [201.0, 62.0, 56.0];
pub const TRADER_PENNANT: [f32; 3] = [86.0, 158.0, 132.0];

/// A lofted vertex, carried from the projection into the face assembly: its
/// screen spot, its bearing off the bow and range (for clipping and the painter
/// sort), and its world position (chart x/y, height above her waterline) for
/// the face normals.
#[derive(Clone, Copy)]
struct P3 {
    sx: f32,
    sy: f32,
    phi: f32,
    d: f32,
    wx: f32,
    wy: f32,
    wz: f32,
}

/// glam's screen-space vector, under an explicit name: the world maths above
/// uses our [`crate::geometry::Vec2`], which shadows it.
type ScreenV = macroquad::math::Vec2;

/// The frame's growing pile of shaded faces: (painter depth, screen triangle,
/// lit colour), sorted far-to-near and drawn once the whole ship is assembled.
type Prims = Vec<(f32, [ScreenV; 3], Color)>;

/// Draw the rival on the water, or nothing if she is out of view. Called inside
/// the world-camera pass (after the wave mesh) so she rides the camera ride and
/// sits on the painted sea. `heave` is the player's heave (the camera's rise);
/// the scene's `light` and `sun` shade her into the night with everything else.
/// `hull` is the shape she sails (see [`crate::hull_shape`]): the racing rival
/// lofts the tier her race demands, the traders their small coasters.
pub fn draw(rk: &Kinematics, view: &SceneView, pennant: [f32; 3], hull: &HullShape) {
    let SceneView {
        kin,
        t,
        sea,
        heave,
        light,
        horizon,
        px_per_rad,
        half_fov_h_view,
        w,
        sun,
        wind_toward,
        ..
    } = *view;
    let d = kin.pos.distance_to(rk.pos);
    if !(1.0..=MAX_VIEW).contains(&d) {
        return;
    }

    // Foot on the local wave surface (gained like the sea); the hull's own
    // metres are not gained, matching how the islands stand on the water.
    let wave = ocean::height(rk.pos, t, sea);
    let foot_disp = (wave - heave) * WAVE_GAIN;

    // The model magnification: larger than life, grown further when the true
    // perspective masthead (the tallest, on a multi-master) would drop under
    // the visibility floor, so a far rival scales up as one shape instead of
    // degenerating to a smear.
    let tallest = hull
        .masts
        .iter()
        .map(|m| hull.station_at(m.z).1 + mast_top(m))
        .fold(0.0f32, f32::max);
    let mast_m = (hull.freeboard + tallest) * RIVAL_MAG;
    // Fake planetary curvature: past `CURVE_START` a distant sail sinks hull-first
    // under the swell (the nearer opaque water swallows her from the waterline up)
    // rather than fading to a ghost. One depression drops every lofted point alike,
    // so `raw_h` (and the magnification) is untouched: she sinks whole, not shrinks.
    let dip = curve_dip(d) * px_per_rad;
    let foot_y = horizon + ((BASE_EYE - foot_disp) / d).atan() * px_per_rad + dip;
    let top_y = horizon + ((BASE_EYE - foot_disp - mast_m) / d).atan() * px_per_rad + dip;
    let raw_h = (foot_y - top_y).max(0.1);
    let s = RIVAL_MAG * (RIVAL_MIN_PX / raw_h).max(1.0);

    // View-cone gate on her centre, slackened by her own longest reach so she
    // doesn't pop while partly on screen.
    let phi_c = wrap_angle(kin.pos.bearing_to(rk.pos) - kin.heading_rad);
    let slack = (hull.sprit_tip.1.abs() * s / d).atan();
    if phi_c.abs() > half_fov_h_view * FOV_MARGIN + slack {
        return;
    }
    let px_per_rad_h = (w * 0.5) / half_fov_h_view;
    // Fully opaque to the cull now: distance removes her by sinking her under the
    // horizon (see `dip` above), not by fading her toward transparent.
    let alpha = 1.0;

    // Her attitude in the swell: heel from the sea sampled off each beam, pitch
    // from ahead and astern along her keel line, by the same gain the mesh uses.
    // The spans follow her own hull, so a short coaster answers chop a longer
    // ship's baselines average out (the exterior echo of the buoyancy probes
    // in `ocean::ship_motion`).
    let fwd = Vec2::from_heading(rk.heading_rad);
    let right = Vec2::new(rk.heading_rad.cos(), -rk.heading_rad.sin());
    let bspan = hull.half_beam() * 2.0;
    let heel = {
        let z_r = ocean::height(rk.pos + right * bspan, t, sea);
        let z_l = ocean::height(rk.pos - right * bspan, t, sea);
        clamp(((z_l - z_r) * WAVE_GAIN / (2.0 * bspan)).atan() * 0.7, -0.4, 0.4)
    };
    let lspan = hull.half_length() * 0.77;
    let pitch = {
        let z_f = ocean::height(rk.pos + fwd * lspan, t, sea);
        let z_a = ocean::height(rk.pos - fwd * lspan, t, sea);
        clamp(((z_a - z_f) * WAVE_GAIN / (2.0 * lspan)).atan() * 0.5, -0.25, 0.25)
    };
    let (sp, cp) = pitch.sin_cos();
    let (sr, cr) = heel.sin_cos();

    // A loft point (rig frame: x starboard, y up from the waist deck, z aft,
    // metres) through her attitude and heading into the world, then through the
    // cylindrical map. The depression angle is purely `atan(height / range)`,
    // a function of range alone, *not* of the bearing (column and row are
    // independent axes, exactly as `islands_render::project` does it).
    let vert = |x: f32, y: f32, z: f32| -> P3 {
        let y1 = y * cp + z * sp; // pitch about the beam axis (bow-down positive)
        let z1 = z * cp - y * sp;
        let x1 = x * cr + y1 * sr; // heel about the keel line (starboard positive)
        let y2 = y1 * cr - x * sr;
        let wp = rk.pos + fwd * (-z1 * s) + right * (x1 * s);
        let elev = (y2 + hull.freeboard) * s;
        let dv = kin.pos.distance_to(wp).max(1.0);
        let phi = wrap_angle(kin.pos.bearing_to(wp) - kin.heading_rad);
        P3 {
            sx: w * 0.5 + phi * px_per_rad_h,
            sy: horizon + ((BASE_EYE - foot_disp - elev) / dv).atan() * px_per_rad
                + curve_dip(dv) * px_per_rad,
            phi,
            d: dv,
            wx: wp.x,
            wy: wp.y,
            wz: elev,
        }
    };

    // Face clipping: a vertex swung far outside the drawn fan (she is nearly
    // alongside) would smear its triangle across the screen, so the face is
    // dropped instead; likewise anything hard against or behind the camera.
    let phi_clip = half_fov_h_view * 1.35;
    let clipped = |a: &P3, b: &P3, c: &P3| {
        a.d.min(b.d).min(c.d) < 2.0 || a.phi.abs().max(b.phi.abs()).max(c.phi.abs()) > phi_clip
    };

    // One face: two-sided (the normal is turned toward the eye, so the far side
    // of the hull shades as its outside), lit by the scene's active light with
    // an ambient floor, like the island facets. `lam` overrides the Lambert
    // term for faces whose true normal is meaningless (spars, the pennant).
    let tri = |prims: &mut Prims, a: P3, b: P3, c: P3, base: [f32; 3], lam: f32| {
        if clipped(&a, &b, &c) {
            return;
        }
        let e1 = (b.wx - a.wx, b.wy - a.wy, b.wz - a.wz);
        let e2 = (c.wx - a.wx, c.wy - a.wy, c.wz - a.wz);
        let mut n = (
            e1.1 * e2.2 - e1.2 * e2.1,
            e1.2 * e2.0 - e1.0 * e2.2,
            e1.0 * e2.1 - e1.1 * e2.0,
        );
        let cen = (
            (a.wx + b.wx + c.wx) / 3.0,
            (a.wy + b.wy + c.wy) / 3.0,
            (a.wz + b.wz + c.wz) / 3.0,
        );
        let to_eye = (kin.pos.x - cen.0, kin.pos.y - cen.1, BASE_EYE - cen.2);
        if n.0 * to_eye.0 + n.1 * to_eye.1 + n.2 * to_eye.2 < 0.0 {
            n = (-n.0, -n.1, -n.2);
        }
        let diff = if lam >= 0.0 {
            lam
        } else {
            let nl = (n.0 * n.0 + n.1 * n.1 + n.2 * n.2).sqrt().max(1e-6);
            ((n.0 * sun.0 + n.1 * sun.1 + n.2 * sun.2) / nl).max(0.0)
        };
        let m = light * (AMBIENT + (1.0 - AMBIENT) * diff);
        let col = Color::new(
            base[0] / 255.0 * m,
            base[1] / 255.0 * m,
            base[2] / 255.0 * m,
            alpha,
        );
        let depth = (a.d + b.d + c.d) / 3.0;
        prims.push((depth, [vec2(a.sx, a.sy), vec2(b.sx, b.sy), vec2(c.sx, c.sy)], col));
    };
    let quad = |prims: &mut Prims, a: P3, b: P3, c: P3, dd: P3, base: [f32; 3], lam: f32| {
        tri(prims, a, b, c, base, lam);
        tri(prims, a, c, dd, base, lam);
    };

    // A round spar as a screen-space ribbon between two lofted points, its
    // width the spar's true projected diameter (floored so a distant mast
    // stays a visible stroke). A fixed mid Lambert stands in for the roundness
    // (a cylinder always shows some lit and some shaded run).
    let line3 = |prims: &mut Prims, a: P3, b: P3, r_m: f32| {
        if clipped(&a, &b, &b) {
            return;
        }
        let pa = vec2(a.sx, a.sy);
        let pb = vec2(b.sx, b.sy);
        let ab = pb - pa;
        let n = vec2(-ab.y, ab.x) / ab.length().max(1e-3);
        let ha = (r_m * s * px_per_rad / a.d).max(0.6);
        let hb = (r_m * s * px_per_rad / b.d).max(0.6);
        let depth = (a.d + b.d) * 0.5;
        let m = light * (AMBIENT + (1.0 - AMBIENT) * 0.55);
        let col = Color::new(
            SPAR[0] / 255.0 * m,
            SPAR[1] / 255.0 * m,
            SPAR[2] / 255.0 * m,
            alpha,
        );
        prims.push((depth, [pa + n * ha, pb + n * hb, pb - n * hb], col));
        prims.push((depth, [pa + n * ha, pb - n * hb, pa - n * ha], col));
    };

    let mut prims: Prims = Vec::with_capacity(192);

    // --- Hull: the player's own lofting stations, seen from without. Each
    // side runs a topside strake from the waterline up to the deck edge and a
    // lighter wale on up to the cap rail, so the sheer line reads; the deck
    // spans between the sheer lines (the doubled break station makes the
    // quarterdeck riser of its own accord), and a dark transom closes the stern.
    let mut port: Vec<(P3, P3, P3)> = Vec::with_capacity(hull.stations.len());
    let mut stbd: Vec<(P3, P3, P3)> = Vec::with_capacity(hull.stations.len());
    for &(z, b, dk, wall) in hull.stations.iter() {
        for (side, out) in [(-1.0f32, &mut port), (1.0f32, &mut stbd)] {
            out.push((
                vert(side * b * WL_TUCK, -hull.freeboard, z),
                vert(side * b, dk, z),
                vert(side * b, dk + wall, z),
            ));
        }
    }
    for i in 0..hull.stations.len() - 1 {
        for side in [&port, &stbd] {
            let (wl0, dk0, cap0) = side[i];
            let (wl1, dk1, cap1) = side[i + 1];
            quad(&mut prims, wl0, wl1, dk1, dk0, HULL, -1.0);
            quad(&mut prims, dk0, dk1, cap1, cap0, RAIL, -1.0);
        }
        quad(&mut prims, port[i].1, stbd[i].1, stbd[i + 1].1, port[i + 1].1, DECK_B, -1.0);
    }
    let aftmost = hull.stations.len() - 1;
    quad(
        &mut prims,
        port[aftmost].0,
        stbd[aftmost].0,
        stbd[aftmost].2,
        port[aftmost].2,
        HULL_DK,
        -1.0,
    );

    // --- Rig: masts, yards and bowsprit at the player's dimensions, trimmed
    // to the same prevailing wind the player's rig is. The yards brace toward
    // the wind's bearing off her own bow by the player's rule (hard over on a
    // beam wind, see `BRACE_LIMIT`); her helm holds course rather than chasing
    // trim, so no easing is needed.
    let wind_rel = wrap_angle(wind_toward - rk.heading_rad);
    let brace = clamp(-wind_rel, -BRACE_LIMIT, BRACE_LIMIT);
    let (sb, cb) = brace.sin_cos();
    line3(
        &mut prims,
        vert(0.0, hull.sprit_base.0, hull.sprit_base.1),
        vert(0.0, hull.sprit_tip.0, hull.sprit_tip.1),
        0.13,
    );
    for mast in hull.masts {
        let foot_y = hull.station_at(mast.z).1;
        let mast_top = foot_y + mast_top(mast);
        line3(
            &mut prims,
            vert(0.0, foot_y, mast.z),
            vert(0.0, mast_top, mast.z),
            MAST_HW * 0.8,
        );

        // The mast's sails, course upward, each its yard and cloth at the
        // player's own dimensions (the canvas plan shared via `sail_cuts`):
        // the course square, the topsail above it tapering toward its head.
        for &(yard_y, hoist, w_head, w_foot) in &sail_cuts(mast, foot_y) {
            let yhw = w_head * 0.5 + 0.4; // the yardarms run a touch past the cloth
            line3(
                &mut prims,
                vert(-cb * yhw, yard_y, mast.z + sb * yhw),
                vert(cb * yhw, yard_y, mast.z - sb * yhw),
                0.11,
            );

            // --- Sail: a coarse grid of cloth panels laced flat along the yard,
            // bellying by how much wind she actually harvests on this point of sail
            // (the physics' own curve, so the cloth hangs slack in irons), deepest
            // toward the free foot, with the player's own draft profiles. The panel's
            // out-of-plane offset rides in front of the mast (`z0` negative = forward)
            // and rotates with the brace, as `ship_render::draw_rig` does it.
            const COLS: usize = 4;
            const ROWS: usize = 2;
            let depth = -wind_factor_rel(wind_rel) * BELLY_DEPTH * w_foot;
            let sail_pt = |i: usize, j: usize| {
                let u = i as f32 / COLS as f32 - 0.5;
                let v = j as f32 / ROWS as f32;
                let x0 = u * (w_head + (w_foot - w_head) * v);
                let z0 = -SAIL_STANDOFF_M
                    + depth * (v * 0.75 * PI).sin() * (1.0 - 0.3 * (2.0 * u).powi(2));
                vert(
                    x0 * cb + z0 * sb,
                    yard_y - v * hoist,
                    mast.z - x0 * sb + z0 * cb,
                )
            };
            for i in 0..COLS {
                for j in 0..ROWS {
                    quad(
                        &mut prims,
                        sail_pt(i, j),
                        sail_pt(i + 1, j),
                        sail_pt(i + 1, j + 1),
                        sail_pt(i, j + 1),
                        SAIL_CLOTH,
                        -1.0,
                    );
                }
            }
        }
    }

    // --- The pennant off the main masthead (red for a rival, green for a
    // trader), streaming dead downwind: unlike the yards it knows no brace
    // limit, so it shows the true wind even when the sail is hauled hard over.
    // A fixed bright term keeps its signal colour saturated.
    let main = hull.masts.last().unwrap();
    let main_top = hull.station_at(main.z).1 + mast_top(main);
    let (pw_s, pw_c) = wind_rel.sin_cos();
    tri(
        &mut prims,
        vert(0.0, main_top, main.z),
        vert(0.0, main_top - 0.45, main.z),
        vert(pw_s * 1.4, main_top - 0.15, main.z - pw_c * 1.4),
        pennant,
        0.85,
    );

    // Painter order: farthest faces first, so the near side of the hull covers
    // the far, and the sail covers the mast's run behind it.
    prims.sort_by(|x, y| y.0.total_cmp(&x.0));
    for (_, p, col) in prims {
        draw_triangle(p[0], p[1], p[2], col);
    }
}
