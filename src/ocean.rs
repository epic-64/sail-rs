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

// As the sea builds, swells grow *longer* as well as taller: a gale rolls in long
// ridges, not tall chop. Naively this means stretching each swell's wavelength with
// the sea state — but that makes the spatial wavenumber `k` a function of time, and the
// phase `k·(p·dir) − SPEED·k·t` is evaluated at *world* coordinates and *absolute* time
// that are both large (the ship sails hundreds of km from the origin; `t` only grows).
// So while the weather eases and `k` drifts, the phase sweeps wildly — the whole sea
// appears to race for the few seconds the transition takes, worse the longer you've
// sailed. Instead we cross-fade between two *fixed* wave trains: a short-wavelength
// "calm" set and a long-wavelength "storm" set. Each `k` is constant, so neither train
// ever races; only the blend weight moves with the sea — a pure amplitude morph, always
// smooth. At `sea = 0` it is the calm train alone, at full storm the long train alone
// (matching the old extremes); in between the two superpose into natural swell groups.
const STORM_STRETCH: f32 = 1.71; // long-train wavelength multiplier at full storm

/// Blend weight (0 = calm/short train … 1 = storm/long train) for the current sea state.
#[inline]
fn storm_blend(sea: f32) -> f32 {
    clamp(sea / 1.3, 0.0, 1.0)
}

/// The crest height (m) at full sea state — the sum of every swell's amplitude.
/// Used to normalise foam/lighting against the tallest possible wave.
pub const MAX_AMPLITUDE: f32 = AMP[0] + AMP[1] + AMP[2] + AMP[3];

// Where the hull floats relative to the ship's world position. The first-person
// eye stands *at* `pos` (the wave mesh marches from there; see `ocean_renderer`),
// and the lofted hull runs from the transom just behind the eye to the stem far
// ahead of it (`ship_render::CAM_AFT` and `STATIONS`). The sway must be sampled
// under *that* hull: the old constants modelled a phantom hull centred on the eye,
// which put every sample half a ship length astern of the ship the player sees,
// so the felt pitch lagged the visible ride (bow-up on the crest, neutral riding
// down the face, the dive arriving only at the trough).
const CENTRE_AHEAD: f32 = 12.0; // hull midpoint, metres ahead of `pos` (the eye)
const HALF_LENGTH: f32 = 13.0; // half the waterline the loft draws
const HALF_BEAM: f32 = 4.0;

// --- Deck / camera "ride" shaping (shared by main.rs and ship_render.rs) -------
// The bow's answer to the swell is *shaped* before it drives the camera look and
// the deck rake: amplified throughout (a swell should heave the bow, not just nod
// it). `PITCH_DIVE` can amplify nosing *down* the back of a crest more than climbing
// its face — set above `PITCH_CLIMB` to make a wave you slide down read as a dive
// rather than a glide. That boost eases in with depth (over `PITCH_DIVE_KNEE`) so the
// response is C1-continuous through the crest and never snaps as the pitch flips sign.
// Currently the two are equal: the nod is symmetric, so the view rests on the horizon
// on average instead of biasing downward. Ported from `SailingView`.
pub const PITCH_CLIMB: f32 = 1.3;
pub const PITCH_DIVE: f32 = 1.3;
pub const PITCH_DIVE_KNEE: f32 = 0.12; // rad of bow-down at which the dive boost is full

/// The bow's shaped answer to the swell: amplified throughout, with an optional extra
/// dive gain (`PITCH_DIVE` > `PITCH_CLIMB`) eased in with depth so it stays smooth
/// through the crest. Symmetric while the two gains are equal. `SailingView.pitchResponse`.
#[inline]
pub fn pitch_response(pitch: f32) -> f32 {
    let dive = clamp(-pitch / PITCH_DIVE_KNEE, 0.0, 1.0);
    pitch * (PITCH_CLIMB + (PITCH_DIVE - PITCH_CLIMB) * dive)
}

/// Metres ahead of the eye (`pos`) where the stem parts the water: the fore end of
/// the drawn hull, where the bow's lift above the hull's mean (`bow_lift`) is
/// sampled for the deck's heave bob and the frontal slam.
pub const BOW_REACH: f32 = CENTRE_AHEAD + HALF_LENGTH;
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
    let b = storm_blend(sea);
    let mut i = 0;
    while i < N {
        let phase_pos = p.x * DIR_X[i] + p.y * DIR_Y[i];
        // Both trains share a crest speed (c = omega/k = SPEED) and phase offset; only
        // the wavelength differs, so the long train rolls in over the short one as the
        // sea builds. Each `k` is a compile-time constant — never a function of `t`.
        let kc = TAU / WAVELENGTH[i];
        let ks = TAU / (WAVELENGTH[i] * STORM_STRETCH);
        let calm = (phase_pos * kc - SPEED[i] * kc * t + PHASE[i]).sin();
        let storm = (phase_pos * ks - SPEED[i] * ks * t + PHASE[i]).sin();
        acc += AMP[i] * ((1.0 - b) * calm + b * storm);
        i += 1;
    }
    sea * acc
}

/// How the ship sways this frame: heave from the mean surface at the eye (`pos`),
/// pitch from the fore-aft slope, roll from the port-starboard slope, and yaw
/// from the surface's twist along the hull, all sampled under the hull as drawn.
pub fn ship_motion(pos: Vec2, heading: f32, t: f32, sea: f32) -> ShipMotion {
    let fwd = Vec2::from_heading(heading);
    let right = Vec2::new(heading.cos(), -heading.sin()); // 90° to starboard of the bow

    // The rigid-body slopes are sampled about the *drawn* hull's midpoint, ahead
    // of the eye (see `CENTRE_AHEAD`), so the deck answers the water the player
    // sees under the bow, in phase with the visible ride.
    let centre = pos + fwd * CENTRE_AHEAD;
    let bow = centre + fwd * HALF_LENGTH;
    let stern = centre - fwd * HALF_LENGTH;
    let z_fore = height(bow, t, sea);
    let z_aft = height(stern, t, sea);
    let z_stbd = height(centre + right * HALF_BEAM, t, sea);
    let z_port = height(centre - right * HALF_BEAM, t, sea);

    // Heave at the eye itself, averaged across the beam (the wheel is on the
    // centreline, so roll lifts it none) for steadiness. The wave mesh marches
    // from the eye and projects the sea relative to this heave, so sampling it
    // anywhere else adds a spurious bob to the near water. Pitch and roll stay
    // the hull's rigid slopes about its centre: the deck is a stiff body that
    // tilts whole, not with the local water at the wheel.
    let heave =
        (height(pos + right * HALF_BEAM, t, sea) + height(pos - right * HALF_BEAM, t, sea)) / 2.0;
    let pitch = (z_fore - z_aft).atan2(2.0 * HALF_LENGTH);
    // Side buoyancy: water risen on one beam lifts that side and rolls the ship
    // *away* from it (a crest on port heels her to starboard, and vice versa). This
    // matches the pitch convention above (the high side lifts, like a bow over a
    // crest); the earlier `z_stbd - z_port` rolled her *into* the swell, backwards.
    let roll = (z_port - z_stbd).atan2(2.0 * HALF_BEAM);

    ShipMotion {
        heave,
        pitch,
        roll,
        yaw: swell_yaw(pos, heading, t, sea),
    }
}

/// Yaw torque from the swell's twist: port-starboard slope at the bow vs the
/// stern; their difference slews the bow off course. Halved to keep it gentle.
/// The camera/deck sway reads it via [`ship_motion`]; in a gale the sailing loop
/// also feeds it back into the hull's *actual* heading (scaled by the storm's
/// fury), so heavy seas genuinely shove the bow and the helm must answer.
pub fn swell_yaw(pos: Vec2, heading: f32, t: f32, sea: f32) -> f32 {
    let fwd = Vec2::from_heading(heading);
    let right = Vec2::new(heading.cos(), -heading.sin()); // 90° to starboard of the bow
    let centre = pos + fwd * CENTRE_AHEAD;
    let bow = centre + fwd * HALF_LENGTH;
    let stern = centre - fwd * HALF_LENGTH;
    let bow_roll = (height(bow + right * HALF_BEAM, t, sea)
        - height(bow - right * HALF_BEAM, t, sea))
    .atan2(2.0 * HALF_BEAM);
    let stern_roll = (height(stern + right * HALF_BEAM, t, sea)
        - height(stern - right * HALF_BEAM, t, sea))
    .atan2(2.0 * HALF_BEAM);
    (bow_roll - stern_roll) / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_swell_yaw_envelope() {
        // Sweep positions, headings and times at full-storm sea and report the
        // twist's envelope; calibrates how hard main.rs lets a gale shove the bow
        // (its storm yaw gain multiplies this). Also pins that the twist stays a
        // nudge, well under a radian, so the feedback can never spin the ship.
        let mut max = 0.0f32;
        let mut sum = 0.0f64;
        let mut n = 0u32;
        for pi in 0..24 {
            let pos = Vec2::new(pi as f32 * 137.3 - 1500.0, pi as f32 * 91.7 - 900.0);
            for hi in 0..8 {
                let heading = hi as f32 / 8.0 * TAU;
                for ti in 0..240 {
                    let yaw = swell_yaw(pos, heading, ti as f32 * 0.171, 1.3).abs();
                    max = max.max(yaw);
                    sum += yaw as f64;
                    n += 1;
                }
            }
        }
        println!("swell_yaw at sea=1.3: max {:.4} rad, mean {:.4} rad", max, sum / n as f64);
        assert!(max > 0.01, "storm seas must twist the hull at all");
        assert!(max < 1.0, "the twist is a nudge, not a spin");
    }
}
