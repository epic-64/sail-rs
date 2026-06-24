//! The sky's moving bodies: a sun and moon that arc over the dome on the
//! day/night clock, and a world-anchored field of stars that fades in after dusk.
//!
//! Everything here is driven by a single clock value `tod` in [0,1): the sun
//! rides an arc that crests at noon (`tod` ½) and dips below the horizon at night,
//! the moon rides the opposite arc, and the stars wheel with the heading like the
//! sun does — all drawn through the same camera the sea and islands use, so they
//! roll together with the swell.

use macroquad::prelude::*;
use std::f32::consts::{PI, TAU};

use crate::geometry::{clamp, wrap_angle};
use crate::rng::Rng;

/// The world bearing the sun transits at noon: due south (0 = N, CW), so it climbs
/// the southern sky like the real northern-hemisphere sun.
const NOON_AZ: f32 = PI;
/// Radians either side of due south the sun (and moon) tracks between rise and set.
/// At 1.25 it rises in the east-south-east and sets in the west-south-west, sweeping
/// the sky rather than rising and setting on one spot.
const AZ_SPREAD: f32 = 1.25;

/// The sky's lighting and bodies at clock `tod`. Altitudes are the *sine* of the
/// body's angle above the horizon (so >0 is up, and they double as the vertical
/// component of the light direction); bearings are world chart angles.
pub struct Sky {
    pub sun_alt: f32,
    pub sun_az: f32,
    pub moon_alt: f32,
    pub moon_az: f32,
    /// 0 in daylight, easing to 1 once the sun is well below the horizon.
    pub star_alpha: f32,
    /// Bearing and altitude of whichever body lights the scene (sun by day, moon
    /// by night), plus how brightly it does so — feeds the wave/island shading.
    pub light_az: f32,
    pub light_alt: f32,
    pub light_strength: f32,
}

/// Resolve the sky state for a clock value. `tod`: 0 = midnight, ¼ = sunrise,
/// ½ = noon, ¾ = sunset.
pub fn sky_state(tod: f32) -> Sky {
    // Arc angle: 0 at sunrise, π/2 at noon, π at sunset, 3π/2 at midnight.
    let a = (tod.rem_euclid(1.0) - 0.25) * TAU;
    let sun_alt = a.sin();
    // cos(a): +1 at sunrise (east of south) → −1 at sunset (west of south).
    let sun_az = NOON_AZ - a.cos() * AZ_SPREAD;
    // The moon runs half a cycle behind the sun, so it climbs as the sun sets.
    let am = a + PI;
    let moon_alt = am.sin();
    let moon_az = NOON_AZ - am.cos() * AZ_SPREAD;

    let sun_light = clamp(sun_alt * 1.4 + 0.05, 0.0, 1.0);
    let moon_light = clamp(moon_alt, 0.0, 1.0) * 0.16;
    // While the sun is up it owns the lighting; once it dips the moon takes over.
    let (light_az, light_alt) = if sun_alt > -0.02 {
        (sun_az, sun_alt.max(0.0))
    } else {
        (moon_az, moon_alt.max(0.0))
    };
    let light_strength = sun_light.max(moon_light);
    let star_alpha = clamp(-sun_alt * 1.8 - 0.05, 0.0, 1.0);

    Sky {
        sun_alt,
        sun_az,
        moon_alt,
        moon_az,
        star_alpha,
        light_az,
        light_alt,
        light_strength,
    }
}

/// One fixed star: a world bearing + altitude, a size, a tint, and its own twinkle.
struct Star {
    az: f32,
    alt: f32, // sine of altitude above the horizon, in (0, 1]
    size: f32,
    color: (f32, f32, f32),
    phase: f32,
    rate: f32,
}

/// A deterministic dome of stars, generated once from the world seed.
pub struct StarField {
    stars: Vec<Star>,
}

/// Pick a star tint: most are white / blue-white, with a scatter of warm amber,
/// ruddy red, and cool cyan-green ones for colour among the field.
fn star_color(rng: &mut Rng) -> (f32, f32, f32) {
    match rng.next_f64() {
        r if r < 0.68 => {
            let b = 0.85 + rng.next_f64() as f32 * 0.15;
            (228.0 * b, 234.0 * b, 255.0)
        }
        r if r < 0.80 => (170.0, 198.0, 255.0), // blue
        r if r < 0.89 => (255.0, 216.0, 168.0), // amber
        r if r < 0.95 => (255.0, 176.0, 162.0), // red
        _ => (176.0, 255.0, 214.0),             // cyan-green
    }
}

impl StarField {
    pub fn new(seed: i64, count: usize) -> Self {
        let mut rng = Rng::from_seed(seed);
        let mut stars = Vec::with_capacity(count);
        for _ in 0..count {
            let az = rng.next_f64() as f32 * TAU;
            // Bias toward the upper sky so the field thins near the horizon haze.
            let alt = 0.05 + (rng.next_f64() as f32).powf(0.7) * 0.95;
            let size = 0.6 + rng.next_f64() as f32 * 1.7;
            let color = star_color(&mut rng);
            let phase = rng.next_f64() as f32 * TAU;
            let rate = 1.4 + rng.next_f64() as f32 * 3.2;
            stars.push(Star {
                az,
                alt,
                size,
                color,
                phase,
                rate,
            });
        }
        StarField { stars }
    }
}

/// Map a body's bearing + altitude to a screen point in the world camera, the same
/// way the sun used to be placed: bearing across the view, altitude up from the
/// horizon. Returns `None` when it's off to the side.
fn project(az: f32, alt: f32, heading: f32, half_fov_h: f32, w: f32, horizon: f32) -> Option<(f32, f32)> {
    let rel = wrap_angle(az - heading);
    if rel.abs() > half_fov_h * 1.15 {
        return None;
    }
    let x = w * 0.5 + (rel / half_fov_h) * (w * 0.5);
    let y = horizon - alt * horizon * 0.95;
    Some((x, y))
}

/// Draw a soft glowing disc (sun or moon) with a faint halo.
#[allow(clippy::too_many_arguments)]
fn draw_body(
    az: f32,
    alt: f32,
    heading: f32,
    half_fov_h: f32,
    w: f32,
    horizon: f32,
    r: f32,
    color: (f32, f32, f32),
    vis: f32,
) {
    if vis <= 0.01 {
        return;
    }
    let Some((x, y)) = project(az, alt, heading, half_fov_h, w, horizon) else {
        return;
    };
    let core = Color::new(color.0 / 255.0, color.1 / 255.0, color.2 / 255.0, vis);
    let halo = Color::new(color.0 / 255.0, color.1 / 255.0, color.2 / 255.0, vis * 0.16);
    draw_circle(x, y, r * 2.1, halo);
    draw_circle(x, y, r * 1.4, halo);
    draw_circle(x, y, r, core);
}

/// Paint the stars (behind), then the moon, then the sun — all above the horizon,
/// so the sea drawn afterwards covers anything dipping below it. `storm` dims the
/// whole sky toward overcast.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    sky: &Sky,
    stars: &StarField,
    t: f32,
    heading: f32,
    half_fov_h: f32,
    w: f32,
    h: f32,
    horizon: f32,
    storm: f32,
) {
    let dim = 1.0 - clamp(storm, 0.0, 1.0); // overcast swallows the stars and moon

    let star_a = sky.star_alpha * dim;
    if star_a > 0.01 {
        for s in &stars.stars {
            if let Some((x, y)) = project(s.az, s.alt, heading, half_fov_h, w, horizon) {
                if y > horizon {
                    continue;
                }
                let tw = 0.62 + 0.38 * (t * s.rate + s.phase).sin();
                let a = clamp(star_a * tw, 0.0, 1.0);
                let c = Color::new(s.color.0 / 255.0, s.color.1 / 255.0, s.color.2 / 255.0, a);
                draw_circle(x, y, s.size, c);
            }
        }
    }

    if sky.moon_alt > -0.03 {
        let vis = clamp(sky.moon_alt * 2.0, 0.0, 1.0) * dim;
        draw_body(sky.moon_az, sky.moon_alt, heading, half_fov_h, w, horizon, h * 0.045, (230.0, 234.0, 242.0), vis);
    }

    if sky.sun_alt > -0.06 {
        // The setting sun burns deep blood-red on the horizon and warms to white at
        // noon. `warm` is held low across the low sun (squared) so the red lingers
        // through the whole sunset rather than only at the instant it touches down.
        let warm = clamp(sky.sun_alt, 0.0, 1.0);
        let warm = warm * warm;
        let color = (
            228.0 + (255.0 - 228.0) * warm,
            48.0 + (244.0 - 48.0) * warm,
            28.0 + (214.0 - 28.0) * warm,
        );
        // The low sun also swells larger near the horizon, as it really appears.
        let r = h * (0.055 + 0.03 * (1.0 - warm));
        draw_body(sky.sun_az, sky.sun_alt, heading, half_fov_h, w, horizon, r, color, dim);
    }
}
