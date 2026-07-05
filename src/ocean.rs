//! The world-anchored ocean height field, ported from `shared.Ocean`.
//!
//! The surface is a sum of travelling sine swells — a pure function of world
//! position and time, fixed to the chart rather than the camera. The renderer
//! samples it to build the wave mesh; the hull samples it to sway with the water.

use crate::geometry::{clamp, Vec2};
use crate::hull_shape::HullShape;
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

// Where the hull floats relative to the ship's world position: the first-person
// eye stands *at* `pos` (the wave mesh marches from there; see `ocean_renderer`),
// and the lofted hull runs from the transom just behind the eye to the stem far
// ahead of it. The sway must be sampled under *that* hull (sampling about the eye
// instead put every probe half a ship length astern of the ship the player sees,
// so the felt pitch lagged the visible ride). Each hull tier carries its own
// probe layout and dims (`crate::hull_shape::HullShape`), so where the water is
// sampled always matches the hull as drawn.

// --- Deck / camera "ride" shaping (shared by main.rs and ship_render.rs) -------
// The bow's answer to the swell is *shaped* before it drives the camera look and
// the deck rake. The gains sit below 1: a swell should mostly *heave* the ship
// (see `deck_heave_px`) with the nod kept secondary, so a wave reads as the water
// rising and falling under the hull rather than a seesaw. `PITCH_DIVE` can weight
// nosing *down* the back of a crest differently from climbing its face; the
// difference eases in with depth (over `PITCH_DIVE_KNEE`) so the response is
// C1-continuous through the crest and never snaps as the pitch flips sign.
// Currently the two are equal: the nod is symmetric, so the view rests on the
// horizon on average instead of biasing downward. Ported from `SailingView`.
pub const PITCH_CLIMB: f32 = 0.9;
pub const PITCH_DIVE: f32 = 0.9;
pub const PITCH_DIVE_KNEE: f32 = 0.12; // rad of bow-down at which the dive boost is full

/// The bow's shaped answer to the swell: scaled by the climb/dive gains, the dive
/// side eased in with depth so it stays smooth through the crest. Symmetric while
/// the two gains are equal. `SailingView.pitchResponse`.
#[inline]
pub fn pitch_response(pitch: f32) -> f32 {
    let dive = clamp(-pitch / PITCH_DIVE_KNEE, 0.0, 1.0);
    pitch * (PITCH_CLIMB + (PITCH_DIVE - PITCH_CLIMB) * dive)
}

/// How the bow's heave is split between craning the *camera* and bobbing the deck.
/// 0 = all deck (a violent slide); 1 = all camera (a level deck under a heaving
/// horizon). `SailingView.heaveCameraShare`.
pub const HEAVE_CAMERA_SHARE: f32 = 0.42;
const HEAVE_GAIN_PX: f32 = 31.0; // px the deck rises per metre the bow lifts above the mean
const HEAVE_MAX_PX: f32 = 51.0; // ceiling, so a steep crest can't fling the deck

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

/// How the ship sways this frame: a least-squares plane fit of the swell over
/// the hull's own buoyancy probes (see [`HullShape::probes`]). Heave is the
/// fitted plane under the eye (`pos`), pitch its fore-aft slope, roll its
/// port-starboard slope, and yaw the surface's twist along the hull.
///
/// The plane fit is the hulls' wave filter: a swell longer than the probed
/// waterline lifts every probe together and tilts the plane whole, while a
/// wave the waterline spans averages out across the probes. A long hull thus
/// rides out the chop a short one is tossed by, and both answer a real storm
/// swell alike. Note the deck no longer chases the water right at the eye: on
/// a big hull small seas visibly run past the hull instead of bobbing it.
pub fn ship_motion(pos: Vec2, heading: f32, t: f32, sea: f32, hull: &HullShape) -> ShipMotion {
    let fwd = Vec2::from_heading(heading);
    let right = Vec2::new(heading.cos(), -heading.sin()); // 90° to starboard of the bow

    // The probes anchor to the *drawn* hull's waterline midpoint, ahead of the
    // eye, so the deck answers the water the player sees under the bow, in
    // phase with the visible ride.
    let centre_ahead = hull.centre_ahead();
    let centre = pos + fwd * centre_ahead;

    let mut n = 0.0f32;
    let mut sum_h = 0.0f32; // probe heights, and their moments along each axis
    let mut sum_a = 0.0f32;
    let mut sum_aa = 0.0f32;
    let mut sum_ah = 0.0f32;
    let mut sum_xx = 0.0f32;
    let mut sum_xh = 0.0f32;
    for &(a, x) in hull.probes {
        let z = height(centre + fwd * a + right * x, t, sea);
        n += 1.0;
        sum_h += z;
        sum_a += a;
        sum_aa += a * a;
        sum_ah += a * z;
        sum_xx += x * x;
        sum_xh += x * z;
    }
    let mean = sum_h / n;
    let a_bar = sum_a / n;
    // Fore-aft slope (m of rise per m toward the bow), centred so a probe
    // layout with an unpaired stem probe doesn't bias it. The athwart slope
    // needs no centring: the starboard offsets cancel by construction.
    let slope_fore = (sum_ah - a_bar * sum_h) / (sum_aa - n * a_bar * a_bar).max(1e-6);
    let slope_stbd = sum_xh / sum_xx.max(1e-6);

    let pitch = slope_fore.atan();
    // Side buoyancy: water risen on one beam lifts that side and rolls the
    // ship *away* from it (a crest on port heels her to starboard), matching
    // the pitch convention (the high side lifts, like a bow over a crest).
    let roll = (-slope_stbd).atan();
    // Heave: the rigid plane evaluated under the eye. The wave mesh marches
    // from the eye and projects the sea relative to this heave, so the water
    // right at the hull stays visually anchored; what the plane fit filtered
    // out of the heave is exactly the chop that then runs past the hull.
    let heave = mean + slope_fore * (-centre_ahead - a_bar);

    ShipMotion {
        heave,
        pitch,
        roll,
        yaw: swell_yaw(pos, heading, t, sea, hull),
    }
}

/// Yaw torque from the swell's twist: port-starboard slope at the bow vs the
/// stern; their difference slews the bow off course. Halved to keep it gentle.
/// The camera/deck sway reads it via [`ship_motion`]; in a gale the sailing loop
/// also feeds it back into the hull's *actual* heading (scaled by the storm's
/// fury), so heavy seas genuinely shove the bow and the helm must answer.
pub fn swell_yaw(pos: Vec2, heading: f32, t: f32, sea: f32, hull: &HullShape) -> f32 {
    let fwd = Vec2::from_heading(heading);
    let right = Vec2::new(heading.cos(), -heading.sin()); // 90° to starboard of the bow
    let (hl, hb) = (hull.half_length(), hull.half_beam());
    let centre = pos + fwd * hull.centre_ahead();
    let bow = centre + fwd * hl;
    let stern = centre - fwd * hl;
    let bow_roll = (height(bow + right * hb, t, sea) - height(bow - right * hb, t, sea))
        .atan2(2.0 * hb);
    let stern_roll = (height(stern + right * hb, t, sea)
        - height(stern - right * hb, t, sea))
    .atan2(2.0 * hb);
    (bow_roll - stern_roll) / 2.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hull_shape::{BRIG, GALLEON, INDIAMAN, SLOOP};

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
                    let yaw = swell_yaw(pos, heading, ti as f32 * 0.171, 1.3, &BRIG).abs();
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

    /// RMS pitch *rate* of a hull over a sweep of positions, headings and
    /// times. Pitch is where the probes' plane fit bites (a wave dead abeam
    /// spans no waterline, so geometric filtering can't touch beam roll; the
    /// mass side of that lives in `HullShape::sway_response`), and the rate
    /// weights the short waves by their high encounter frequency; plain
    /// amplitude is dominated by the long swells both hulls ride alike.
    fn pitch_rate_rms(hull: &HullShape, sea: f32) -> f64 {
        let dt = 1.0 / 30.0;
        let mut sum = 0.0f64;
        let mut n = 0u32;
        for pi in 0..12 {
            let pos = Vec2::new(pi as f32 * 211.7 - 1700.0, pi as f32 * 149.3 - 1100.0);
            for hi in 0..8 {
                let heading = hi as f32 / 8.0 * TAU;
                let mut prev = ship_motion(pos, heading, 0.0, sea, hull);
                for ti in 1..180 {
                    let m = ship_motion(pos, heading, ti as f32 * dt, sea, hull);
                    let dp = ((m.pitch - prev.pitch) / dt) as f64;
                    sum += dp * dp;
                    n += 1;
                    prev = m;
                }
            }
        }
        (sum / n as f64).sqrt()
    }

    /// The point of per-hull buoyancy probes: in a light sea (short calm-train
    /// waves) the sloop's short waterline is worked noticeably harder than the
    /// brig's, while a full storm's long swells even the two out.
    #[test]
    fn small_hull_answers_the_chop_a_big_hull_filters() {
        let calm_ratio = pitch_rate_rms(&SLOOP, 0.35) / pitch_rate_rms(&BRIG, 0.35);
        let storm_ratio = pitch_rate_rms(&SLOOP, 1.3) / pitch_rate_rms(&BRIG, 1.3);
        println!("sloop/brig pitch-rate ratio: calm {calm_ratio:.2}, storm {storm_ratio:.2}");
        assert!(calm_ratio > 1.2, "the sloop should feel light chop the brig filters");
        assert!(
            storm_ratio < calm_ratio,
            "long storm swells should even the hulls out relative to the chop"
        );
    }

    /// The same ordering carried up a tier: the galleon's still longer probed
    /// waterline rides light chop easier than the brig (her heavier feel on
    /// top of that lives in `sway_response`, outside this fit).
    #[test]
    fn the_galleon_filters_what_still_works_the_brig() {
        let calm_ratio = pitch_rate_rms(&BRIG, 0.35) / pitch_rate_rms(&GALLEON, 0.35);
        println!("brig/galleon pitch-rate ratio: calm {calm_ratio:.2}");
        assert!(calm_ratio > 1.1, "the galleon should filter chop the brig still feels");
    }

    /// And once more up the ladder: the indiaman's still longer probed
    /// waterline rides light chop easier than the galleon.
    #[test]
    fn the_indiaman_filters_what_still_works_the_galleon() {
        let calm_ratio = pitch_rate_rms(&GALLEON, 0.35) / pitch_rate_rms(&INDIAMAN, 0.35);
        println!("galleon/indiaman pitch-rate ratio: calm {calm_ratio:.2}");
        assert!(calm_ratio > 1.05, "the indiaman should filter chop the galleon still feels");
    }
}
