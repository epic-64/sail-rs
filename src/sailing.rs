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
    /// dead before the wind. `shared.Wind.factor`.
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
    let theta = wind_rel.abs(); // 0 = running downwind
    let off_wind = PI - theta; // 0 = into the eye, π = dead run
    let irons = PI - DEAD_ANGLE; // 30° off the eye: edge of the no-go zone
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
pub const MAX_SPEED: f32 = 23.0 * KNOT; // ~11.83 m/s top speed (a dead run = 100%)
pub const DRAG: f32 = 0.28; // 1/s water resistance while the sails are working
pub const GLIDE_DRAG: f32 = 0.10; // 1/s reduced resistance when starved of drive, so she keeps her way
pub const GLIDE_FACTOR: f32 = FLOOR_DRIVE; // wind factor below which drag eases toward GLIDE_DRAG
pub const KEEL: f32 = 0.9; // 1/s how strongly the keel bleeds side-slip
pub const MAX_YAW_RATE: f32 = 0.24; // rad/s heading change at full rudder once up to speed
pub const REF_SPEED: f32 = 7.0; // m/s at which the rudder reaches full bite
pub const MIN_AUTHORITY: f32 = 0.4; // steerage even dead in the water — keeps her handy with sails struck / no wind
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
    step_scaled(kin, helm, wind, dt, 1.0)
}

/// As [`step`], but scaling the rig's top speed by `speed_scale` — the ship's
/// upgraded rig and the weight of its cargo (see `game_state::upgrades`). A bare
/// ship within its haulage uses 1.0, so the baseline feel is unchanged.
pub fn step_scaled(kin: Kinematics, helm: Helm, wind: Wind, dt: f32, speed_scale: f32) -> Kinematics {
    let rudder = clamp(helm.turn, -1.0, 1.0);
    let throttle = clamp(helm.throttle, 0.0, 1.0);
    let top = MAX_SPEED * speed_scale.max(0.05);

    let authority = clamp(kin.speed() / REF_SPEED, MIN_AUTHORITY, 1.0);
    let target_yaw = rudder * MAX_YAW_RATE * authority;
    let yaw_rate = kin.yaw_rate + (target_yaw - kin.yaw_rate) * clamp(YAW_INERTIA * dt, 0.0, 1.0);
    let heading = wrap_angle(kin.heading_rad + yaw_rate * dt);
    let fwd = Vec2::from_heading(heading);

    // Sails push along the bow, scaled by how much wind the bow's angle harvests.
    let factor = wind.factor(heading);
    let drive = throttle * (top * DRAG) * factor;
    let thrust_v = kin.vel + fwd * (drive * dt);
    // Water resistance: full DRAG at any working point of sail, easing to the lower
    // GLIDE_DRAG as the drive falls away in the no-go zone — so a ship that shoots
    // into irons keeps her way and can coast through the wind's eye rather than
    // stalling at once. Real points of sail (factor ≥ GLIDE_FACTOR) are unaffected,
    // so steady-state sailing speeds are unchanged.
    let glide = clamp(factor / GLIDE_FACTOR, 0.0, 1.0);
    let decay = GLIDE_DRAG + (DRAG - GLIDE_DRAG) * glide;
    let dragged = thrust_v * (1.0 - decay * dt).max(0.0);
    let fwd_comp = fwd.dot(dragged);
    let lateral = dragged - fwd * fwd_comp; // sideways slip
    let gripped = dragged - lateral * clamp(KEEL * dt, 0.0, 1.0);

    // Full drive on a beam reach = the top speed.
    let ceiling = top * MAX_BOOST;
    let sp = gripped.length();
    let capped = if sp > ceiling {
        gripped * (ceiling / sp)
    } else {
        gripped
    };
    Kinematics {
        pos: kin.pos + capped * dt,
        heading_rad: heading,
        vel: capped,
        yaw_rate,
    }
}

/// Metres of open water kept between the hull and an island's shore.
pub const HULL_CLEARANCE: f32 = 8.0;
const SCRAPE_LOSS: f32 = 0.06;
const SCRAPE_RETAIN: f32 = 0.2;

/// Keep the hull out of every island after a step: if the new position crossed
/// inside an island's keep-out radius, slide it back to the boundary and strip
/// the inward velocity, so a ship driven at the coast grazes along it and slides
/// round instead of ploughing through. Remaining along-shore way is bled down by
/// a *scrape* proportional to how hard the hull struck. Ported from
/// `Ship.resolveGrounding`.
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
        let pushed = isle.pos + n * keep_out;
        let inward = k.vel.dot(n); // < 0 while sailing into the shore
        if inward >= 0.0 {
            k.pos = pushed; // already sailing back out — just unstick
        } else {
            let tangential = k.vel - n * inward; // strip the shoreward component
            let retain = clamp(1.0 + SCRAPE_LOSS * inward, SCRAPE_RETAIN, 1.0);
            k.pos = pushed;
            k.vel = tangential * retain;
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
        for _ in 0..200 {
            k = step(k, Helm { turn: 0.0, throttle: 1.0 }, NORTHERLY, 0.1);
        }
        assert!(k.speed() > MAX_SPEED * 0.95);
        assert!(k.speed() <= MAX_SPEED + 1e-6);
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
