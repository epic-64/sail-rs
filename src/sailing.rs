//! Ship kinematics and helm, ported from `shared.Sailing` (`Kinematics`, `Helm`,
//! `Ship.step`). This is the camera you drive around the wave field.
//!
//! Wind/sail harvesting is not modelled yet — `drive` is simply throttle × thrust
//! so you can sail around and watch the swell. The rest (drag, keel side-slip,
//! rudder authority, yaw inertia) matches the original so the boat handles right.

use crate::geometry::{clamp, wrap_angle, Vec2};
use crate::world::Island;

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
pub const DRAG: f32 = 0.28; // 1/s water resistance
pub const KEEL: f32 = 0.9; // 1/s how strongly the keel bleeds side-slip
pub const MAX_YAW_RATE: f32 = 0.24; // rad/s heading change at full rudder once up to speed
pub const REF_SPEED: f32 = 7.0; // m/s at which the rudder reaches full bite
pub const MIN_AUTHORITY: f32 = 0.15; // a sliver of steering even dead in the water
pub const YAW_INERTIA: f32 = 0.3; // 1/s how quickly yaw-rate eases toward the rudder's command

/// Advance the ship by `dt` seconds. The rudder only turns the hull when water
/// flows over it (authority ∝ speed) and the yaw-rate eases in/out with inertia,
/// so the wheel arcs the ship rather than pivoting it. Velocity lags the bow.
pub fn step(kin: Kinematics, helm: Helm, dt: f32) -> Kinematics {
    let rudder = clamp(helm.turn, -1.0, 1.0);
    let throttle = clamp(helm.throttle, 0.0, 1.0);

    let authority = clamp(kin.speed() / REF_SPEED, MIN_AUTHORITY, 1.0);
    let target_yaw = rudder * MAX_YAW_RATE * authority;
    let yaw_rate = kin.yaw_rate + (target_yaw - kin.yaw_rate) * clamp(YAW_INERTIA * dt, 0.0, 1.0);
    let heading = wrap_angle(kin.heading_rad + yaw_rate * dt);
    let fwd = Vec2::from_heading(heading);

    // Sails (no wind model yet): full drive straight along the bow.
    let drive = throttle * (MAX_SPEED * DRAG);
    let thrust_v = kin.vel + fwd * (drive * dt);
    let dragged = thrust_v * (1.0 - DRAG * dt).max(0.0);
    let fwd_comp = fwd.dot(dragged);
    let lateral = dragged - fwd * fwd_comp; // sideways slip
    let gripped = dragged - lateral * clamp(KEEL * dt, 0.0, 1.0);

    let sp = gripped.length();
    let capped = if sp > MAX_SPEED {
        gripped * (MAX_SPEED / sp)
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
