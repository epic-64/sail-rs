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
    alt: f32, // sine of altitude; <0 dips below the horizon, >1 is past the zenith
    size: f32,
    color: (f32, f32, f32),
    phase: f32,
    rate: f32,
    /// A faint star that only emerges at the darkest of night: its alpha is gated by
    /// how far the sun has sunk, so it lifts the density past what dusk shows.
    faint: bool,
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
        r if r < 0.80 => (150.0, 188.0, 255.0), // blue
        r if r < 0.89 => (255.0, 190.0, 110.0), // amber
        r if r < 0.95 => (255.0, 120.0, 96.0),  // red
        _ => (120.0, 255.0, 188.0),             // cyan-green
    }
}

impl StarField {
    pub fn new(seed: i64, count: usize) -> Self {
        let mut rng = Rng::from_seed(seed);
        // The bright field shown all night, plus a denser pool of faint stars that
        // only fade in at the darkest of night (gated in `draw`).
        let faint_count = count * 3 / 4;
        let mut stars = Vec::with_capacity(count + faint_count);
        for i in 0..count + faint_count {
            let faint = i >= count;
            let az = rng.next_f64() as f32 * TAU;
            // Spread evenly from below the horizon up past the zenith, so the field
            // reaches the sea line (and keeps reaching it when the camera rolls or
            // pitches) and still fills the top corners when the helm cranes up. The
            // projection is linear in `alt`, so a uniform draw gives even screen
            // density. Stars that fall below the horizon are painted over by the sea;
            // those past the zenith sit over the over-scanned top sky.
            let alt = -0.18 + rng.next_f64() as f32 * 1.6;
            // Faint stars are the small pin-pricks that crowd a truly dark sky.
            let size = if faint {
                0.5 + rng.next_f64() as f32 * 0.4
            } else {
                0.8 + rng.next_f64() as f32 * 0.45
            };
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
                faint,
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

/// Linear blend between two RGB triples (0–255 components), `t` in [0, 1].
fn mix(a: (f32, f32, f32), b: (f32, f32, f32), t: f32) -> (f32, f32, f32) {
    (
        a.0 + (b.0 - a.0) * t,
        a.1 + (b.1 - a.1) * t,
        a.2 + (b.2 - a.2) * t,
    )
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
        // `star_alpha` is already full by early night, so it can't reveal the faint
        // pool. Gate those on how far the sun has sunk past it: 0 until well after
        // dusk, ramping to 1 at the darkest of night (sun at its lowest).
        let deep_night = clamp((-sky.sun_alt - 0.5) / 0.5, 0.0, 1.0);
        for s in &stars.stars {
            let base = if s.faint { star_a * deep_night } else { star_a };
            if base <= 0.01 {
                continue;
            }
            if let Some((x, y)) = project(s.az, s.alt, heading, half_fov_h, w, horizon) {
                // No horizon cull: stars run right down to (and past) the sea line so
                // a rolled or pitched view never bares a starless band at the horizon.
                // The far-water rectangle, drawn after this, hides the submerged ones.
                let tw = 0.82 + 0.18 * (t * s.rate + s.phase).sin();
                let a = clamp(base * tw, 0.0, 1.0);
                let c = Color::new(s.color.0 / 255.0, s.color.1 / 255.0, s.color.2 / 255.0, a);
                draw_circle(x, y, s.size, c);
            }
        }
    }

    if sky.moon_alt > -0.03 {
        // A touch dimmer than before, so the moon glows softly rather than glaring.
        let vis = clamp(sky.moon_alt * 2.0, 0.0, 1.0) * dim * 0.7;
        draw_body(sky.moon_az, sky.moon_alt, heading, half_fov_h, w, horizon, h * 0.038, (230.0, 234.0, 242.0), vis);
    }

    // Drawn until the whole disc has dipped below the horizon (the sea, painted
    // afterwards, hides the submerged part), so it completes its set instead of
    // vanishing while still half up.
    if sky.sun_alt > -0.2 {
        let low = clamp(sky.sun_alt, 0.0, 1.0);
        // How coloured the disc is: 1 right at the horizon, easing to 0 (natural
        // white) once the sun clears COLOR_SPAN. A small span confines the tint to
        // the brief sunrise/sunset window and leaves the sun white the rest of its
        // arc — so it neither reddens too early in the evening nor lingers orange
        // into the morning.
        const COLOR_SPAN: f32 = 0.32;
        let tint = 1.0 - clamp(low / COLOR_SPAN, 0.0, 1.0);
        // Sunrise climbs in the east (azimuth shy of due south); sunset sinks in the
        // west. They get different low-sun tints.
        let rising = sky.sun_az < NOON_AZ;
        let mut color = if rising {
            // Sunrise opens orange — never blood-red — through yellow to white.
            if tint > 0.5 {
                mix((255.0, 224.0, 130.0), (255.0, 150.0, 52.0), (tint - 0.5) * 2.0) // yellow → orange
            } else {
                mix((255.0, 244.0, 214.0), (255.0, 224.0, 130.0), tint * 2.0) // white → yellow
            }
        } else {
            // The setting sun burns deep blood-red right on the horizon, warming to
            // white as it lifts clear.
            mix((255.0, 244.0, 214.0), (228.0, 48.0, 28.0), tint)
        };
        // A touch smaller than before; still swells a little near the horizon, as it
        // really appears.
        let r = h * (0.045 + 0.022 * (1.0 - low));
        // Green flash: as the last sliver of the setting sun slips under the horizon
        // its top edge flares emerald for an instant (a real atmospheric effect). The
        // flash peaks the moment the disc's top edge meets the sea line — `set_alt`
        // is the altitude at which that happens, given the projection's `0.95` scale.
        let mut vis = dim;
        if !rising {
            let set_alt = -r / (horizon * 0.95);
            let flash = clamp(1.0 - (sky.sun_alt - set_alt).abs() / 0.035, 0.0, 1.0);
            if flash > 0.0 {
                color = mix(color, (70.0, 255.0, 130.0), flash);
                vis = (dim * (1.0 + 0.5 * flash)).min(1.0);
            }
        }
        draw_body(sky.sun_az, sky.sun_alt, heading, half_fov_h, w, horizon, r, color, vis);
    }
}
