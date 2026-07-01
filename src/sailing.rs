//! Ship kinematics, helm, and the prevailing wind, ported from `shared.Sailing`
//! (`Kinematics`, `Helm`, `Ship.step`, `Wind`). This is the boat you sail around
//! the wave field.
//!
//! The sails harvest the wind by the bow's angle to it (`Wind::factor`): a beam
//! reach is fastest, a dead run a touch slower, and a 30°-either-side no-go zone
//! into the eye yields nothing — so making ground upwind forces the player to
//! tack. Drag, keel side-slip, rudder authority and yaw inertia all match the
//! original so the boat handles right.

use std::f32::consts::PI;

use crate::geometry::{clamp, wrap_angle, Vec2};
use crate::rng::Rng;
use crate::world::Island;

/// The prevailing wind, given as the compass heading it blows *toward* (0 = toward
/// north, increasing clockwise), so a ship on that same heading is running dead
/// before it. The wind shifts to a fresh random quarter every few minutes; the
/// sails are assumed perfectly trimmed, so all that matters for drive is the angle
/// between the hull's bow and the wind. Ported from `shared.Wind`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Wind {
    pub toward_rad: f32,
}

/// Top of the normalised drive scale, 1.0 = 100%. A beam reach — the wind square
/// on the beam, where the sails work hardest like wings — is the fastest point of
/// sail, so the curve peaks there; every other point of sail is a fraction of it.
pub const MAX_BOOST: f32 = 1.0;
/// Closer to the wind than this (measured off dead-downwind) the sails find no
/// drive at all — the ship is in irons and must tack. A 30° no-go zone either side
/// of the wind's eye. Public so the race's rival helm can beat to the same edge
/// (`shared.Wind.deadAngle`).
pub const DEAD_ANGLE: f32 = 5.0 / 6.0 * PI; // 150° off downwind = 30° off the wind
/// The drive snatched the instant the bow falls off out of irons: the curve jumps
/// straight to ~10% rather than building from nothing, so a hard beat crawls.
const FLOOR_DRIVE: f32 = 0.10;
/// Drive dead before the wind — fast, but a beam reach is faster, so the curve
/// eases back to this fraction at a dead run.
const RUN_DRIVE: f32 = 0.75;
/// The fastest point of sail: the wind square on the beam, 90° off the eye.
const BEAM_ANGLE: f32 = PI / 2.0;
/// Shapes the quick climb from `FLOOR_DRIVE` (just out of irons) to the peak.
const BEAT_EXP: f32 = 0.6;
/// Shapes the gentler fall from the peak (beam reach) back to `RUN_DRIVE` (a run).
const RUN_EXP: f32 = 1.4;

impl Wind {
    /// A fully random quarter — the whole sky's worth of choices when the wind
    /// backs or veers. `shared.Wind.random`.
    pub fn random(rng: &mut Rng) -> Wind {
        Wind {
            toward_rad: rng.between(-PI as f64, PI as f64) as f32,
        }
    }

    /// A first breeze that won't strand a new captain on spawn: the wind's quarter
    /// is rolled within 90° of the bow, so the ship begins on at least a beam reach
    /// — never in irons. Used only for the opening wind. `shared.Wind.favorable`.
    pub fn favorable(heading_rad: f32, rng: &mut Rng) -> Wind {
        let off = rng.between(-PI as f64 / 2.0, PI as f64 / 2.0) as f32;
        Wind {
            toward_rad: wrap_angle(heading_rad + off),
        }
    }

    /// Drive multiplier for a hull on `heading_rad` in this wind, in [0, MAX_BOOST].
    /// Zero inside the no-go zone; jumps to `FLOOR_DRIVE` the instant the bow falls
    /// off, climbs to the `MAX_BOOST` peak at a beam reach, then eases to `RUN_DRIVE`
    /// dead before the wind. `shared.Wind.factor`. The renderer reads the curve via
    /// `wind_factor_rel` directly, so this faithful-port wrapper is exercised only by
    /// the tests that pin the drive curve.
    #[allow(dead_code)]
    pub fn factor(&self, heading_rad: f32) -> f32 {
        wind_factor_rel(wrap_angle(self.toward_rad - heading_rad))
    }

    /// Which point of sail the bow's angle to the wind puts the ship on, for the
    /// HUD. `shared.Wind.pointOfSail`.
    pub fn point_of_sail(&self, heading_rad: f32) -> PointOfSail {
        let theta = wrap_angle(heading_rad - self.toward_rad).abs(); // 0 = running downwind
        let off_wind = (PI - theta) * 180.0 / PI; // degrees off the wind's eye
        if off_wind < 30.0 {
            PointOfSail::IntoWind
        } else if off_wind < 60.0 {
            PointOfSail::CloseHauled
        } else if off_wind < 80.0 {
            PointOfSail::CloseReach
        } else if off_wind < 100.0 {
            PointOfSail::BeamReach
        } else if off_wind < 130.0 {
            PointOfSail::Reaching
        } else if off_wind < 160.0 {
            PointOfSail::BroadReach
        } else {
            PointOfSail::Running
        }
    }
}

/// The drive multiplier as a function of the wind's bearing *relative to the bow*
/// (`wrap(toward - heading)`, 0 = tailwind from astern). The whole curve depends
/// only on the magnitude of this angle, so the sail renderer can read its belly
/// straight off the relative wind without the world heading. See `Wind::factor`.
pub fn wind_factor_rel(wind_rel: f32) -> f32 {
    wind_factor_rel_widened(wind_rel, 0.0)
}

/// As [`wind_factor_rel`], but widening the no-go zone by `dead_extra` radians on
/// each side — a battered hull can't point as high, so its irons edge creeps out
/// (see `game_state::hull::debuff`). `dead_extra` of 0 is the undamaged curve.
pub fn wind_factor_rel_widened(wind_rel: f32, dead_extra: f32) -> f32 {
    let theta = wind_rel.abs(); // 0 = running downwind
    let off_wind = PI - theta; // 0 = into the eye, π = dead run
    // 30° off the eye plus the damage's widening, clamped short of the beam so the
    // climb to the peak never inverts.
    let irons = (PI - DEAD_ANGLE + dead_extra).min(BEAM_ANGLE - 0.01);
    if off_wind <= irons {
        0.0
    } else if off_wind <= BEAM_ANGLE {
        let t = (off_wind - irons) / (BEAM_ANGLE - irons); // 0 at no-go edge, 1 at the beam
        FLOOR_DRIVE + (MAX_BOOST - FLOOR_DRIVE) * t.powf(BEAT_EXP)
    } else {
        let t = (off_wind - BEAM_ANGLE) / (PI - BEAM_ANGLE); // 0 at the beam, 1 dead astern
        MAX_BOOST - (MAX_BOOST - RUN_DRIVE) * t.powf(RUN_EXP)
    }
}

/// Handling penalties a damaged hull suffers, stacked from its condition by
/// `game_state::hull::debuff`. A sound hull uses [`HullDebuff::NONE`], which
/// leaves the boat's feel exactly as the original.
#[derive(Clone, Copy, Debug)]
pub struct HullDebuff {
    /// Radians added to each side of the no-go zone (she can't point as high).
    pub dead_angle_extra: f32,
    /// Multiplier on rudder turn rate (1.0 = full bite, < 1 sluggish helm).
    pub turn_mult: f32,
    /// Multiplier on top speed (1.0 = full, < 1 a tired hull).
    pub speed_mult: f32,
}

impl HullDebuff {
    pub const NONE: HullDebuff = HullDebuff {
        dead_angle_extra: 0.0,
        turn_mult: 1.0,
        speed_mult: 1.0,
    };
}

/// Where the present heading sits on the points of sail, coarse-grained for the
/// HUD. `shared.PointOfSail`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PointOfSail {
    IntoWind,
    CloseHauled,
    CloseReach,
    BeamReach,
    Reaching,
    BroadReach,
    Running,
}

impl PointOfSail {
    pub fn label(self) -> &'static str {
        match self {
            PointOfSail::IntoWind => "Into the wind",
            PointOfSail::CloseHauled => "Close-hauled",
            PointOfSail::CloseReach => "Close reach",
            PointOfSail::BeamReach => "Beam reach",
            PointOfSail::Reaching => "Reaching",
            PointOfSail::BroadReach => "Broad reach",
            PointOfSail::Running => "Running",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Kinematics {
    pub pos: Vec2,
    pub heading_rad: f32,
    pub vel: Vec2,
    pub yaw_rate: f32,
}

impl Kinematics {
    pub fn still(pos: Vec2, heading_rad: f32) -> Self {
        Kinematics {
            pos,
            heading_rad,
            vel: Vec2::ZERO,
            yaw_rate: 0.0,
        }
    }

    pub fn speed(&self) -> f32 {
        self.vel.length()
    }
}

/// Player helm input for a frame. `turn` in [-1, 1], `throttle` in [0, 1].
#[derive(Clone, Copy, Debug)]
pub struct Helm {
    pub turn: f32,
    pub throttle: f32,
}

impl Helm {
    pub const IDLE: Helm = Helm {
        turn: 0.0,
        throttle: 0.0,
    };
}

pub const KNOT: f32 = 0.5144; // m/s per knot
/// The base hull's top speed in knots — a fresh, lightly-laden ship on a beam reach
/// (its fastest point of sail). This is only the *baseline*: the engine itself
/// imposes no ceiling. The actual top speed is whatever the ship hands
/// [`step_with`]/[`step_debuffed`], which its rig upgrades raise and a laden hold
/// lowers (see `game_state::upgrades::top_speed`). Kept here, beside `KNOT`, as the
/// single source the economy reads back in knots (`game_state::BASE_TOP_KNOTS`).
pub const BASE_TOP_KNOTS: f32 = 24.0;
/// [`BASE_TOP_KNOTS`] in engine units (m/s) — the default ceiling for the
/// parameterless [`step`] (NPCs) and the reference the bow-spray normalises against.
pub const BASE_TOP_SPEED: f32 = BASE_TOP_KNOTS * KNOT; // ~12.35 m/s
// The bow speed chases the angle's max with two *separate* rates, so building way and
// shedding it are decoupled — a brisk climb you can read, a lazy coast you can tack on.
pub const ACCEL_GAIN: f32 = 0.75; // 1/s building way: τ ≈ 1/ACCEL_GAIN ≈ 1.3 s, so she
// makes the angle's full speed in ~8 s — quick enough to tell whether a new heading pays.
pub const COAST_DRAG: f32 = 0.14; // 1/s shedding way (sail can't hold her speed): τ ≈ 7 s,
// well below ACCEL_GAIN so a heavy hull glides through the wind's eye on a tack/jibe
// rather than stalling dead. This is the *only* knob on how far she carries through irons.
const SETTLE_EPS: f32 = 0.03; // m/s: within this of the angle's max, snap to it exactly, so
// a steady beam reach reads a clean 24.0 kn instead of asymptoting to 23.9 (≈ 0.06 kn).
pub const KEEL: f32 = 0.9; // 1/s how strongly the keel bleeds side-slip
pub const MAX_YAW_RATE: f32 = 0.24; // rad/s heading change at full rudder once up to speed
pub const REF_SPEED: f32 = 7.0; // m/s at which the rudder reaches full bite
pub const MIN_AUTHORITY: f32 = 0.4; // steerage even dead in the water: keeps her handy with sails furled / no wind
pub const YAW_INERTIA: f32 = 0.3; // 1/s how quickly yaw-rate eases toward the rudder's command

/// Advance the ship by `dt` seconds. The helm sets a *rudder* angle (`turn`) and a
/// *sail* setting (`throttle`); the rudder only turns the hull when water flows
/// over it (authority ∝ speed) and the yaw-rate eases in/out with inertia, so the
/// wheel arcs the ship rather than pivoting it, and velocity lags the bow.
///
/// The sails harvest the `wind` by the bow's angle to it (`Wind::factor`): a beam
/// reach is fastest, pointing into the eye removes drive entirely, so making ground
/// upwind forces the player to tack. Ported from `Ship.step`.
pub fn step(kin: Kinematics, helm: Helm, wind: Wind, dt: f32) -> Kinematics {
    step_with(kin, helm, wind, dt, BASE_TOP_SPEED)
}

/// As [`step`], but with the ship's own `top_speed` (m/s) on a beam reach — set by
/// its rig upgrades and the weight in its hold (see `game_state::upgrades::top_speed`).
/// The engine carries no top speed of its own; the ship is the authority. A bare hull
/// passes [`BASE_TOP_SPEED`], so the baseline feel is unchanged.
pub fn step_with(kin: Kinematics, helm: Helm, wind: Wind, dt: f32, top_speed: f32) -> Kinematics {
    step_debuffed(kin, helm, wind, dt, top_speed, HullDebuff::NONE)
}

/// As [`step_with`], but with a damaged hull's handling penalties folded in: a
/// wider no-go zone, a slower-answering helm, and a lower top speed (see
/// [`HullDebuff`]). [`step_with`] is this with [`HullDebuff::NONE`].
pub fn step_debuffed(
    kin: Kinematics,
    helm: Helm,
    wind: Wind,
    dt: f32,
    top_speed: f32,
    debuff: HullDebuff,
) -> Kinematics {
    let rudder = clamp(helm.turn, -1.0, 1.0);
    let throttle = clamp(helm.throttle, 0.0, 1.0);
    let top = top_speed.max(0.05 * BASE_TOP_SPEED) * debuff.speed_mult.max(0.05);

    let authority = clamp(kin.speed() / REF_SPEED, MIN_AUTHORITY, 1.0);
    let target_yaw = rudder * MAX_YAW_RATE * debuff.turn_mult * authority;
    let yaw_rate = kin.yaw_rate + (target_yaw - kin.yaw_rate) * clamp(YAW_INERTIA * dt, 0.0, 1.0);
    let heading = wrap_angle(kin.heading_rad + yaw_rate * dt);
    let fwd = Vec2::from_heading(heading);

    // How much wind the bow's angle harvests — through a no-go zone widened by any hull
    // damage — sets the speed this point of sail can hold under the present sail trim. A
    // beam reach at full sail is the hull's top speed exactly (factor = MAX_BOOST = 1).
    let factor = wind_factor_rel_widened(wrap_angle(wind.toward_rad - heading), debuff.dead_angle_extra);
    let target = top * factor * throttle;

    // The along-bow way homes on that target: briskly while building (ACCEL_GAIN), gently
    // while she's carrying more than the sail holds (COAST_DRAG) — bearing into the eye on
    // a tack, or with the sheet eased — so she glides through the no-go zone instead of
    // stalling. Snap the last sliver so a steady reach makes the number exactly.
    let bow = fwd.dot(kin.vel);
    let bow = if bow < target {
        let gained = bow + (target - bow) * clamp(ACCEL_GAIN * dt, 0.0, 1.0);
        if target - gained < SETTLE_EPS {
            target
        } else {
            gained
        }
    } else {
        bow - (bow - target) * clamp(COAST_DRAG * dt, 0.0, 1.0)
    };

    // Sideways slip is bled by the keel; a little persists so a heavy hull skids through
    // her turns rather than tracking on rails.
    let slip = (kin.vel - fwd * fwd.dot(kin.vel)) * (1.0 - clamp(KEEL * dt, 0.0, 1.0));

    // Recompose. The bow term never exceeds `top`; cap the whole so transient slip in a
    // hard turn can't push her past the hull's max either.
    let vel = fwd * bow + slip;
    let sp = vel.length();
    let vel = if sp > top { vel * (top / sp) } else { vel };
    Kinematics {
        pos: kin.pos + vel * dt,
        heading_rad: heading,
        vel,
        yaw_rate,
    }
}

/// Metres of open water kept between the hull and an island's shore.
pub const HULL_CLEARANCE: f32 = 8.0;

/// Keep the hull out of every island's shore — and nothing more. A hard ring sits
/// at `radius + HULL_CLEARANCE`: cross it and the ship is unstuck back to the ring
/// with only her *inward* (shoreward) way cancelled, so sailing straight at a shore
/// stops her dead at the beach (a "crash") while the along-shore (tangential) way is
/// left untouched. There is **no** cushion or scrape — she keeps full speed grazing
/// the coast, so the captain can zoom past a shore as close as she likes.
pub fn resolve_grounding(kin: Kinematics, islands: &[&Island]) -> Kinematics {
    let mut k = kin;
    for isle in islands {
        let keep_out = isle.radius + HULL_CLEARANCE;
        let delta = k.pos - isle.pos;
        let d = delta.length();
        if d >= keep_out {
            continue;
        }
        let n = if d > 1e-6 {
            delta * (1.0 / d)
        } else {
            Vec2::from_heading(k.heading_rad) * -1.0
        };
        // Unstick to the ring; if still closing, cancel only the shoreward way and
        // keep the along-shore way (no speed lost grazing parallel to the coast).
        k.pos = isle.pos + n * keep_out;
        let inward = k.vel.dot(n); // < 0 while sailing toward the shore
        if inward < 0.0 {
            k.vel = k.vel - n * inward;
        }
    }
    k
}

#[cfg(test)]
mod tests {
    use super::*;

    // A steady wind toward the north, so a ship heading north runs before it.
    const NORTHERLY: Wind = Wind { toward_rad: 0.0 };

    fn still(heading: f32) -> Kinematics {
        Kinematics::still(Vec2::ZERO, heading)
    }

    #[test]
    fn beam_reach_is_the_strongest_drive() {
        assert!((NORTHERLY.factor(PI / 2.0) - MAX_BOOST).abs() < 1e-6);
    }

    #[test]
    fn a_dead_run_eases_back_to_run_drive() {
        assert!((NORTHERLY.factor(0.0) - RUN_DRIVE).abs() < 1e-6);
    }

    #[test]
    fn no_drive_straight_into_the_wind() {
        assert!(NORTHERLY.factor(PI).abs() < 1e-6);
    }

    #[test]
    fn no_headway_pointing_into_the_wind_however_full_the_sail() {
        let into_wind = still(PI);
        let after = step(into_wind, Helm { turn: 0.0, throttle: 1.0 }, NORTHERLY, 0.5);
        assert!(after.speed() < 1e-6);
    }

    #[test]
    fn beam_reach_settles_faster_than_a_dead_run() {
        let across = step(still(PI / 2.0), Helm { turn: 0.0, throttle: 1.0 }, NORTHERLY, 2.0).speed();
        let downwind = step(still(0.0), Helm { turn: 0.0, throttle: 1.0 }, NORTHERLY, 2.0).speed();
        assert!(across > downwind);
    }

    #[test]
    fn a_long_beam_reach_climbs_to_top_speed_without_exceeding_it() {
        let mut k = still(PI / 2.0);
        // A beam reach at full sail targets exactly `top`; she climbs in ~8 s and snaps
        // onto the number (SETTLE_EPS), so after this she sits at `top`, never above it.
        for _ in 0..600 {
            k = step(k, Helm { turn: 0.0, throttle: 1.0 }, NORTHERLY, 0.1);
        }
        assert!(k.speed() > BASE_TOP_SPEED * 0.95);
        assert!(k.speed() <= BASE_TOP_SPEED + 1e-6);
    }

    #[test]
    fn a_settled_beam_reach_makes_top_speed_exactly() {
        // The captain means 24 kn, not 23.9: a steady beam reach must land *on* the
        // number, not asymptote short of it.
        let mut k = still(PI / 2.0);
        for _ in 0..600 {
            k = step(k, Helm { turn: 0.0, throttle: 1.0 }, NORTHERLY, 0.1);
        }
        assert!((k.speed() - BASE_TOP_SPEED).abs() < 1e-6);
    }

    #[test]
    fn reaches_the_angle_max_within_ten_seconds() {
        // Building way is brisk enough to read an angle: ≥ 99% of the beam-reach max
        // after 10 s of full sail.
        let mut k = still(PI / 2.0);
        for _ in 0..100 {
            k = step(k, Helm { turn: 0.0, throttle: 1.0 }, NORTHERLY, 0.1);
        }
        assert!(k.speed() >= BASE_TOP_SPEED * 0.99);
    }

    #[test]
    fn she_coasts_far_slower_than_she_builds_way() {
        // Decoupled rates: from rest, one second of full sail on a beam reach builds
        // more way than one second drifting into irons sheds — so she carries her
        // momentum through the wind's eye on a tack.
        let built = step(still(PI / 2.0), Helm { turn: 0.0, throttle: 1.0 }, NORTHERLY, 1.0).speed();
        // Start at that speed but pointed into the wind's eye (no drive), sails furled.
        let moving = Kinematics {
            vel: Vec2::from_heading(PI) * built,
            ..still(PI)
        };
        let kept = step(moving, Helm::IDLE, NORTHERLY, 1.0).speed();
        let shed = built - kept;
        assert!(shed < built, "she should keep most of her way through the eye");
        assert!(kept > built * 0.85, "a one-second coast should barely slow her");
    }

    #[test]
    fn favorable_opening_wind_is_never_worse_than_a_dead_run() {
        let heading = 1.0;
        let mut rng = Rng::from_seed(42);
        let w = Wind::favorable(heading, &mut rng);
        assert!(w.factor(heading) >= RUN_DRIVE - 1e-6);
    }

    #[test]
    fn points_of_sail_are_named_like_the_original() {
        assert_eq!(NORTHERLY.point_of_sail(0.0), PointOfSail::Running);
        assert_eq!(NORTHERLY.point_of_sail(35f32.to_radians()), PointOfSail::BroadReach);
        assert_eq!(NORTHERLY.point_of_sail(PI / 2.0), PointOfSail::BeamReach);
        assert_eq!(NORTHERLY.point_of_sail(135f32.to_radians()), PointOfSail::CloseHauled);
        assert_eq!(NORTHERLY.point_of_sail(PI), PointOfSail::IntoWind);
    }
}
