//! Drawing the racing rival: a small low-poly sloop billboard standing on the
//! swell, projected from its live position and shrinking with distance — the same
//! treatment the islands and their scenery get. Replaces the original's
//! `ship.svg`/`ship-bow.svg`/`ship-stern.svg` sprites (the port dropped the SVG
//! billboards for flat-shaded geometry; see [`crate::islands_render`]).
//!
//! The hull rides the local wave so the rival heaves with the same sea the player
//! does; the mast stands a fixed height of real metres above it. The whole sprite
//! is mirrored so her bow points the way she is actually sailing.

use macroquad::prelude::*;

use crate::geometry::{clamp, wrap_angle, Vec2};
use crate::ocean;
use crate::ocean_renderer::WAVE_GAIN;
use crate::projection::{BASE_EYE, MAX_VIEW};
use crate::sailing::Kinematics;

const RIVAL_TOP_M: f32 = 15.0; // metres from the waterline to the masthead
const RIVAL_MAG: f32 = 1.35; // drawn a touch larger than life
const RIVAL_MIN_PX: f32 = 26.0; // floor so a distant sail stays spottable
const FOV_MARGIN: f32 = 1.12; // matches the wave mesh's column fan

/// Draw the rival on the water, or nothing if she is out of view. Called inside
/// the world-camera pass (after the wave mesh) so she rides the camera ride and
/// sits on the painted sea. `heave` is the player's heave (the camera's rise);
/// `light` dims her into the night with the rest of the scene.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    rk: &Kinematics,
    kin: &Kinematics,
    t: f32,
    sea: f32,
    heave: f32,
    light: f32,
    horizon: f32,
    px_per_rad: f32,
    half_fov_h_view: f32,
    w: f32,
) {
    let d = kin.pos.distance_to(rk.pos);
    if d < 1.0 || d > MAX_VIEW {
        return;
    }
    let phi = wrap_angle(kin.pos.bearing_to(rk.pos) - kin.heading_rad);
    if phi.abs() > half_fov_h_view * FOV_MARGIN {
        return;
    }
    let px_per_rad_h = (w * 0.5) / half_fov_h_view;
    let sx = w * 0.5 + phi * px_per_rad_h;

    // Foot on the local wave surface (gained like the sea), masthead a fixed run of
    // real metres above it. Project both through the same cylindrical map the waves
    // use, so the rival sits *in* the swell rather than floating over it.
    let wave = ocean::height(rk.pos, t, sea);
    let foot_disp = (wave - heave) * WAVE_GAIN;
    let cphi = phi.cos();
    let foot_y = horizon + ((BASE_EYE - foot_disp) * cphi / d).atan() * px_per_rad;
    let top_y = horizon + ((BASE_EYE - foot_disp - RIVAL_TOP_M) * cphi / d).atan() * px_per_rad;
    let raw_h = (foot_y - top_y).max(0.0);
    let height = (raw_h * RIVAL_MAG).max(RIVAL_MIN_PX);
    let alpha = clamp((1.0 - d / MAX_VIEW) * 1.6, 0.18, 1.0);

    // Which way she shows: her bow points the way she is going, so mirror the
    // sprite when she is crossing to our left (heading left of our line of sight).
    let rel = wrap_angle(rk.heading_rad - kin.pos.bearing_to(rk.pos));
    let flip = if rel.sin() < 0.0 { -1.0 } else { 1.0 };

    // Heel her with the local swell so she rides the waves rather than standing
    // bolt upright on them: sample the sea a few metres to each beam (across our
    // line of sight) and tilt toward the lower side, by the same gain the mesh uses.
    let dir = (rk.pos - kin.pos) * (1.0 / d);
    let beam = Vec2::new(dir.y, -dir.x);
    let span = 7.0;
    let z_r = ocean::height(rk.pos + beam * span, t, sea);
    let z_l = ocean::height(rk.pos - beam * span, t, sea);
    let roll = clamp(((z_r - z_l) * WAVE_GAIN / (2.0 * span)).atan() * 0.7, -0.4, 0.4);

    draw_sloop(sx, foot_y, height, flip, roll, alpha, light);
}

/// A stylised square-rigged sloop in a local space where x ∈ [-0.5, 0.5] (bow to
/// the right before mirroring) and y ∈ [0, 1] (0 = waterline foot, 1 = masthead),
/// mapped to screen at (cx + flip·lx·w, foot − ly·h). Two-tone to imply a bellied
/// sail and a rounded hull, matching the faceted low-poly look of the isles.
fn draw_sloop(cx: f32, foot: f32, h: f32, flip: f32, roll: f32, alpha: f32, light: f32) {
    let w = h * 0.92;
    // Local (lx, ly) → an offset from the foot, rotated by the swell heel, then
    // anchored at the foot point on the wave so she rocks about her waterline.
    let (sr, cr) = roll.sin_cos();
    let p = |lx: f32, ly: f32| {
        let dx = flip * lx * w;
        let dy = -ly * h;
        vec2(cx + dx * cr - dy * sr, foot + dx * sr + dy * cr)
    };
    let tri = |a: (f32, f32), b: (f32, f32), c: (f32, f32), col: Color| {
        draw_triangle(p(a.0, a.1), p(b.0, b.1), p(c.0, c.1), col);
    };
    let quad = |a: (f32, f32), b: (f32, f32), c: (f32, f32), dd: (f32, f32), col: Color| {
        draw_triangle(p(a.0, a.1), p(b.0, b.1), p(c.0, c.1), col);
        draw_triangle(p(a.0, a.1), p(c.0, c.1), p(dd.0, dd.1), col);
    };
    let rgba = |c: [f32; 3], a: f32| {
        Color::new(c[0] / 255.0 * light, c[1] / 255.0 * light, c[2] / 255.0 * light, a)
    };

    const HULL: [f32; 3] = [86.0, 60.0, 38.0];
    const HULL_DK: [f32; 3] = [58.0, 40.0, 26.0];
    const DECK: [f32; 3] = [128.0, 94.0, 58.0];
    const MAST: [f32; 3] = [74.0, 56.0, 38.0];
    const SAIL: [f32; 3] = [236.0, 228.0, 204.0];
    const SAIL_DK: [f32; 3] = [200.0, 190.0, 162.0];
    const PENNANT: [f32; 3] = [201.0, 62.0, 56.0];

    // Mast, then the two sails braced about it (drawn before the hull so the hull's
    // bulwark tucks over their foot).
    quad((-0.03, 0.18), (0.03, 0.18), (0.02, 0.99), (-0.02, 0.99), rgba(MAST, alpha));
    // Mainsail abaft the mast (toward the stern, to the left), bellied.
    tri((-0.02, 0.24), (-0.02, 0.95), (-0.46, 0.36), rgba(SAIL, alpha));
    tri((-0.02, 0.24), (-0.46, 0.36), (-0.40, 0.24), rgba(SAIL_DK, alpha));
    // Headsail forward of the mast (toward the bow, to the right).
    tri((0.0, 0.30), (0.0, 0.90), (0.46, 0.26), rgba(SAIL, alpha));
    tri((0.0, 0.30), (0.46, 0.26), (0.40, 0.24), rgba(SAIL_DK, alpha));

    // Hull: a rounded body sitting at the waterline, transom to the left, bow
    // sweeping out to the right, with a lighter deck strake along the top.
    quad((-0.40, 0.04), (0.40, 0.04), (0.40, 0.18), (-0.40, 0.18), rgba(HULL, alpha));
    tri((0.40, 0.04), (0.52, 0.13), (0.40, 0.18), rgba(HULL, alpha)); // bow
    tri((-0.40, 0.04), (-0.40, 0.18), (-0.48, 0.13), rgba(HULL_DK, alpha)); // transom
    tri((-0.40, 0.04), (0.40, 0.04), (0.30, 0.0), rgba(HULL_DK, alpha)); // keel shadow
    quad((-0.40, 0.16), (0.40, 0.16), (0.40, 0.20), (-0.40, 0.20), rgba(DECK, alpha)); // deck line

    // A pennant streaming from the masthead.
    tri((0.0, 0.99), (0.20, 0.95), (0.0, 0.90), rgba(PENNANT, alpha));
}
