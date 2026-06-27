//! Drawing floating salvage: small low-poly crates, barrels and strongboxes
//! bobbing on the swell, projected from their world positions and shrinking with
//! distance — the same billboard treatment the islands, their scenery and the
//! racing rival get (see [`crate::rival_render`]). Replaces the original's
//! `crate.svg` / `barrel.svg` / `chest.svg` sprites with flat-shaded geometry to
//! match the rest of the port's look.
//!
//! Each piece rides the local wave so it heaves with the same sea the player
//! does, and heels with the swell so it rocks on the water rather than standing
//! bolt upright. Drawn inside the wave march (see [`crate::ocean_renderer`]) so
//! nearer crests and islands occlude it like any other world object.

use macroquad::prelude::*;

use crate::flotsam::FlotsamKind;
use crate::geometry::{clamp, Vec2};
use crate::ocean;
use crate::ocean_renderer::WAVE_GAIN;
use crate::projection::BASE_EYE;
use crate::scene::SceneView;

const TOP_M: f32 = 3.0; // metres from the waterline to the top of the object
const MAG: f32 = 1.6; // drawn a touch larger than life so it stays spottable
const MIN_PX: f32 = 5.0; // floor so a distant speck of salvage stays visible
const FOV_MARGIN: f32 = 1.12; // matches the wave mesh's column fan
// Salvage you can realistically reach sits within a few hundred metres of the bow,
// so a piece beyond that should read as out of reach: it fades from full opacity at
// FADE_NEAR to nothing by FADE_FAR (and is not drawn past it), dissolving into the
// haze instead of hanging on the horizon as a crisp, never-closing speck.
const FADE_NEAR: f32 = 800.0;
const FADE_FAR: f32 = 1400.0;

/// Draw one piece of salvage on the water, or nothing if it is out of view.
/// Called inside the world-camera pass (interleaved with the wave bands) so it
/// rides the camera ride and sits on the painted sea. `heave` is the player's
/// heave (the camera's rise); `light` dims it into the night with the scene.
pub fn draw(pos: Vec2, kind: FlotsamKind, view: &SceneView) {
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
        ..
    } = *view;
    let d = kin.pos.distance_to(pos);
    if !(1.0..=FADE_FAR).contains(&d) {
        return;
    }
    let phi = crate::geometry::wrap_angle(kin.pos.bearing_to(pos) - kin.heading_rad);
    if phi.abs() > half_fov_h_view * FOV_MARGIN {
        return;
    }
    let px_per_rad_h = (w * 0.5) / half_fov_h_view;
    let sx = w * 0.5 + phi * px_per_rad_h;

    // Foot on the local wave surface (gained like the sea), top a fixed run of real
    // metres above it — projected through the same cylindrical map the waves use so
    // the piece sits *in* the swell rather than floating over it.
    let wave = ocean::height(pos, t, sea);
    let foot_disp = (wave - heave) * WAVE_GAIN;
    let cphi = phi.cos();
    let foot_y = horizon + ((BASE_EYE - foot_disp) * cphi / d).atan() * px_per_rad;
    let top_y = horizon + ((BASE_EYE - foot_disp - TOP_M) * cphi / d).atan() * px_per_rad;
    let raw_h = (foot_y - top_y).max(0.0);
    let height = (raw_h * MAG).max(MIN_PX);
    // Fade out with distance so far, unreachable salvage dissolves into the haze
    // rather than reading as a crisp speck that never grows.
    let alpha = clamp((FADE_FAR - d) / (FADE_FAR - FADE_NEAR), 0.0, 1.0);

    // Heel it with the local swell (sample the sea to each beam and tilt toward the
    // lower side) so it bobs on the waves, by the same gain the mesh uses.
    let dir = (pos - kin.pos) * (1.0 / d);
    let beam = Vec2::new(dir.y, -dir.x);
    let span = 6.0;
    let z_r = ocean::height(pos + beam * span, t, sea);
    let z_l = ocean::height(pos - beam * span, t, sea);
    let roll = clamp(((z_r - z_l) * WAVE_GAIN / (2.0 * span)).atan() * 0.8, -0.5, 0.5);

    draw_piece(kind, sx, foot_y, height, roll, alpha, light);
}

/// Paint one piece of salvage in a local space where x ∈ [-0.5, 0.5] and
/// y ∈ [0, 1] (0 = waterline foot, 1 = top), rotated by the swell heel and
/// anchored at the foot point so it rocks about its waterline.
fn draw_piece(kind: FlotsamKind, cx: f32, foot: f32, h: f32, roll: f32, alpha: f32, light: f32) {
    let bw = h * 0.95; // billboard width tracks its drawn height
    let (sr, cr) = roll.sin_cos();
    let p = |lx: f32, ly: f32| {
        let dx = lx * bw;
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

    match kind {
        FlotsamKind::Crate => draw_crate(&quad, &rgba, alpha),
        FlotsamKind::Barrel => draw_barrel(&tri, &quad, &rgba, alpha),
        FlotsamKind::Chest => draw_chest(&tri, &quad, &rgba, alpha),
    }
}

// --- the three kinds, each a squat flat-shaded billboard ----------------------

/// A planked wooden crate: a lit front face, a darker shaded side, and a pale top
/// lid, with two cross-battens nailed across the face.
fn draw_crate(
    quad: &impl Fn((f32, f32), (f32, f32), (f32, f32), (f32, f32), Color),
    rgba: &impl Fn([f32; 3], f32) -> Color,
    a: f32,
) {
    const FACE: [f32; 3] = [150.0, 108.0, 64.0];
    const SIDE: [f32; 3] = [104.0, 74.0, 42.0];
    const LID: [f32; 3] = [176.0, 132.0, 84.0];
    const BATTEN: [f32; 3] = [86.0, 60.0, 34.0];

    // Body sits low on the water (a little awash at the foot), a near-cube.
    quad((-0.34, 0.06), (0.30, 0.06), (0.30, 0.74), (-0.34, 0.74), rgba(FACE, a));
    // Shaded right side, drawn as a thin slanted strake to fake a turned corner.
    quad((0.30, 0.06), (0.42, 0.14), (0.42, 0.80), (0.30, 0.74), rgba(SIDE, a));
    // Pale lid catching the sky.
    quad((-0.34, 0.74), (0.30, 0.74), (0.42, 0.80), (-0.22, 0.80), rgba(LID, a));
    // Cross-battens nailed across the face (an X of two thin planks).
    quad((-0.34, 0.16), (-0.22, 0.06), (0.30, 0.64), (0.18, 0.74), rgba(BATTEN, a));
    quad((0.18, 0.06), (0.30, 0.16), (-0.22, 0.74), (-0.34, 0.64), rgba(BATTEN, a));
}

/// An oak cask: a bulged body (wider amidships than at the chimes) shaded down one
/// side, banded by two dark iron hoops.
fn draw_barrel(
    tri: &impl Fn((f32, f32), (f32, f32), (f32, f32), Color),
    quad: &impl Fn((f32, f32), (f32, f32), (f32, f32), (f32, f32), Color),
    rgba: &impl Fn([f32; 3], f32) -> Color,
    a: f32,
) {
    const STAVE: [f32; 3] = [168.0, 120.0, 66.0];
    const STAVE_DK: [f32; 3] = [120.0, 84.0, 46.0];
    const HOOP: [f32; 3] = [70.0, 58.0, 48.0];

    // Lower and upper trapezoids meeting at the bulge (y≈0.45): narrow chimes,
    // fat belly. Lit left half then a darker right half down the turn of the staves.
    quad((-0.20, 0.05), (0.04, 0.05), (0.04, 0.45), (-0.34, 0.45), rgba(STAVE, a));
    quad((-0.34, 0.45), (0.04, 0.45), (0.04, 0.86), (-0.20, 0.86), rgba(STAVE, a));
    quad((0.04, 0.05), (0.22, 0.05), (0.36, 0.45), (0.04, 0.45), rgba(STAVE_DK, a));
    quad((0.04, 0.45), (0.36, 0.45), (0.22, 0.86), (0.04, 0.86), rgba(STAVE_DK, a));
    // The pale end-grain of the cask head peeking over the top chime.
    tri((-0.20, 0.86), (0.04, 0.90), (0.22, 0.86), rgba(STAVE, a));
    // Two iron hoops banding the staves.
    quad((-0.30, 0.22), (0.30, 0.22), (0.30, 0.29), (-0.30, 0.29), rgba(HOOP, a));
    quad((-0.30, 0.61), (0.30, 0.61), (0.30, 0.68), (-0.30, 0.68), rgba(HOOP, a));
}

/// A half-sunk strongbox: a dark hardwood chest with a domed lid, a brass band up
/// the front and a brass lock-plate — the rare, valuable salvage.
fn draw_chest(
    tri: &impl Fn((f32, f32), (f32, f32), (f32, f32), Color),
    quad: &impl Fn((f32, f32), (f32, f32), (f32, f32), (f32, f32), Color),
    rgba: &impl Fn([f32; 3], f32) -> Color,
    a: f32,
) {
    const BODY: [f32; 3] = [92.0, 60.0, 36.0];
    const BODY_DK: [f32; 3] = [62.0, 40.0, 24.0];
    const LID: [f32; 3] = [110.0, 74.0, 46.0];
    const BRASS: [f32; 3] = [206.0, 166.0, 74.0];

    // Box body, awash at the foot, with a darker right edge for depth.
    quad((-0.40, 0.05), (0.30, 0.05), (0.30, 0.46), (-0.40, 0.46), rgba(BODY, a));
    quad((0.30, 0.05), (0.42, 0.12), (0.42, 0.52), (0.30, 0.46), rgba(BODY_DK, a));
    // Domed lid: a trapezoid capped by a low arc of two triangles.
    quad((-0.40, 0.46), (0.30, 0.46), (0.26, 0.66), (-0.36, 0.66), rgba(LID, a));
    tri((-0.36, 0.66), (0.26, 0.66), (-0.06, 0.76), rgba(LID, a));
    tri((0.30, 0.46), (0.42, 0.52), (0.26, 0.66), rgba(BODY_DK, a));
    // Brass: a band up the front and a square lock-plate at the lid seam.
    quad((-0.06, 0.05), (0.04, 0.05), (0.04, 0.66), (-0.06, 0.66), rgba(BRASS, a));
    quad((-0.10, 0.40), (0.08, 0.40), (0.08, 0.52), (-0.10, 0.52), rgba(BRASS, a));
}
