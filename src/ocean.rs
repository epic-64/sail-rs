//! The world-anchored ocean height field, ported from `shared.Ocean`.
//!
//! The surface is a sum of travelling sine swells — a pure function of world
//! position and time, fixed to the chart rather than the camera. The renderer
//! samples it to build the wave mesh; the hull samples it to sway with the water.

use crate::geometry::Vec2;
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

    let heave = (z_fore + z_aft + z_stbd + z_port) / 4.0;
    let pitch = (z_fore - z_aft).atan2(2.0 * HALF_LENGTH);
    let roll = (z_stbd - z_port).atan2(2.0 * HALF_BEAM);

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
