//! The world-anchored ocean height field, ported from `shared.Ocean`.
//!
//! The surface is a sum of travelling sine swells — a pure function of world
//! position and time, fixed to the chart rather than the camera. The renderer
//! samples it to build the wave mesh; the hull samples it to sway with the water.

use crate::geometry::{clamp, Vec2};
use std::f32::consts::TAU;

/// How the deck reacts to the swell this frame (metres / radians). Bow-up pitch,
/// starboard-down roll and bow-to-starboard yaw are positive.
#[derive(Clone, Copy, Debug, Default)]
pub struct ShipMotion {
    pub heave: f32,
    pub pitch: f32,
    pub roll: f32,
    pub yaw: f32,
}

// One travelling wave each: unit direction, wavelength + amplitude (m), phase
// speed (m/s) and a phase offset. Order matches the Scala `swells`.
const N: usize = 4;
const DIR_X: [f32; N] = [1.0, 0.6, -0.3, 0.9];
const DIR_Y: [f32; N] = [0.0, 0.8, 0.95, -0.45];
const WAVELENGTH: [f32; N] = [130.0, 78.0, 44.0, 24.0];
const AMP: [f32; N] = [1.70, 1.05, 0.55, 0.28];
const SPEED: [f32; N] = [7.5, 5.5, 4.0, 3.0];
const PHASE: [f32; N] = [0.0, 1.7, 3.1, 5.2];

/// The crest height (m) at full sea state — the sum of every swell's amplitude.
/// Used to normalise foam/lighting against the tallest possible wave.
pub const MAX_AMPLITUDE: f32 = AMP[0] + AMP[1] + AMP[2] + AMP[3];

// Hull half-dimensions (m) the sway samples the surface across.
const HALF_LENGTH: f32 = 12.0;
const HALF_BEAM: f32 = 4.0;
// Where the helm — the first-person eye — sits aft of the hull's midpoint. The
// view's heave is sampled here (not at the centre) so it crests and dips in step
// with the observer rather than with a point a half-length ahead of them.
const HELM_AFT: f32 = 9.0;

// --- Deck / camera "ride" shaping (shared by main.rs and ship_render.rs) -------
// The bow's answer to the swell is *shaped* before it drives the camera look and
// the deck rake: amplified throughout (a swell should heave the bow, not just nod
// it) and amplified more nosing *down* the back of a crest than climbing its face,
// so a wave you slide down reads as a dive, not a glide. The dive boost eases in
// with depth (over `PITCH_DIVE_KNEE`) so the response is C1-continuous through the
// crest and never snaps as the pitch flips sign. Ported from `SailingView`.
pub const PITCH_CLIMB: f32 = 1.3;
pub const PITCH_DIVE: f32 = 2.0;
pub const PITCH_DIVE_KNEE: f32 = 0.12; // rad of bow-down at which the dive boost is full

/// The bow's shaped answer to the swell: climbs gently, noses down hard, eased in
/// with depth so it stays smooth through the crest. `SailingView.pitchResponse`.
#[inline]
pub fn pitch_response(pitch: f32) -> f32 {
    let dive = clamp(-pitch / PITCH_DIVE_KNEE, 0.0, 1.0);
    pitch * (PITCH_CLIMB + (PITCH_DIVE - PITCH_CLIMB) * dive)
}

/// Metres ahead of the hull centre the bow parts the water — where the bow's lift
/// above the hull's mean (`bow_lift`) is sampled for the deck's heave bob.
pub const BOW_REACH: f32 = 12.0;
/// How the bow's heave is split between craning the *camera* and bobbing the deck.
/// 0 = all deck (a violent slide); 1 = all camera (a level deck under a heaving
/// horizon). `SailingView.heaveCameraShare`.
pub const HEAVE_CAMERA_SHARE: f32 = 0.42;
const HEAVE_GAIN_PX: f32 = 27.0; // px the deck rises per metre the bow lifts above the mean
const HEAVE_MAX_PX: f32 = 44.0; // ceiling, so a steep crest can't fling the deck

/// Vertical screen shift (px) bobbing the deck/camera with the bow's lift above the
/// hull's mean (`bow_lift`, metres). A plain linear gain, clamped so even a freak
/// crest stays a bob, never a launch. Bow-up lifts the deck → negative (up) px.
/// `SailingView.deckHeavePx`.
#[inline]
pub fn deck_heave_px(bow_lift: f32) -> f32 {
    clamp(-bow_lift * HEAVE_GAIN_PX, -HEAVE_MAX_PX, HEAVE_MAX_PX)
}

/// Sea-surface elevation (m) at world point `p` and time `t` seconds.
#[inline]
pub fn height(p: Vec2, t: f32, sea: f32) -> f32 {
    let mut acc = 0.0;
    let mut i = 0;
    while i < N {
        let k = TAU / WAVELENGTH[i];
        let omega = SPEED[i] * k;
        acc += AMP[i] * ((p.x * DIR_X[i] + p.y * DIR_Y[i]) * k - omega * t + PHASE[i]).sin();
        i += 1;
    }
    sea * acc
}

/// How the ship sways this frame: heave from the mean surface under the hull,
/// pitch from the fore-aft slope, roll from the port-starboard slope, and yaw
/// from the surface's twist along the hull.
pub fn ship_motion(pos: Vec2, heading: f32, t: f32, sea: f32) -> ShipMotion {
    let fwd = Vec2::from_heading(heading);
    let right = Vec2::new(heading.cos(), -heading.sin()); // 90° to starboard of the bow

    let z_fore = height(pos + fwd * HALF_LENGTH, t, sea);
    let z_aft = height(pos - fwd * HALF_LENGTH, t, sea);
    let z_stbd = height(pos + right * HALF_BEAM, t, sea);
    let z_port = height(pos - right * HALF_BEAM, t, sea);

    // Heave at the helm, where the observer actually stands — averaged across the
    // beam (the wheel is on the centreline, so roll lifts it none) for steadiness.
    // Sampling it at the hull's midpoint made the crest peak, and the dip begin,
    // while the helm a half-length astern was still climbing, so the view hung at
    // the top and dipped late. Pitch and roll stay the hull's rigid slopes about
    // its centre: the deck is a stiff body that tilts whole, not with the local
    // water at the wheel.
    let helm = pos - fwd * HELM_AFT;
    let heave =
        (height(helm + right * HALF_BEAM, t, sea) + height(helm - right * HALF_BEAM, t, sea)) / 2.0;
    let pitch = (z_fore - z_aft).atan2(2.0 * HALF_LENGTH);
    // Side buoyancy: water risen on one beam lifts that side and rolls the ship
    // *away* from it — a crest on port heels her to starboard, and vice versa. This
    // matches the pitch convention above (the high side lifts, like a bow over a
    // crest); the earlier `z_stbd - z_port` rolled her *into* the swell, backwards.
    let roll = (z_port - z_stbd).atan2(2.0 * HALF_BEAM);

    // Yaw torque from the swell's twist: port-starboard slope at the bow vs the
    // stern; their difference slews the bow off course. Halved to keep it gentle.
    let bow_roll = (height(pos + fwd * HALF_LENGTH + right * HALF_BEAM, t, sea)
        - height(pos + fwd * HALF_LENGTH - right * HALF_BEAM, t, sea))
    .atan2(2.0 * HALF_BEAM);
    let stern_roll = (height(pos - fwd * HALF_LENGTH + right * HALF_BEAM, t, sea)
        - height(pos - fwd * HALF_LENGTH - right * HALF_BEAM, t, sea))
    .atan2(2.0 * HALF_BEAM);
    let yaw = (bow_roll - stern_roll) / 2.0;

    ShipMotion {
        heave,
        pitch,
        roll,
        yaw,
    }
}
