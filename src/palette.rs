//! Time-of-day colour, ported from `shared.Daytime` plus the sea palette in
//! `OceanRenderer` and the sky gradient in `SailingView`.
//!
//! The sea palette is 21 channels in [0,255]:
//!   near, mid, far (the depth ramp), foam, sun (diffuse warmth),
//!   sky (Fresnel reflection), glint (specular sun-glitter).
//! The light colours carry most of the mood; the water hue shifts only gently
//! from phase to phase.

/// The time of day at sea, on a single dawn→day→dusk→night cycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Daytime {
    Dawn,
    Day,
    Dusk,
    Night,
}

impl Daytime {
    pub fn label(self) -> &'static str {
        match self {
            Daytime::Dawn => "Dawn",
            Daytime::Day => "Day",
            Daytime::Dusk => "Dusk",
            Daytime::Night => "Night",
        }
    }
}

/// Sea palette length (7 colours × 3 channels).
pub const PALETTE_LEN: usize = 21;
pub type Palette = [f32; PALETTE_LEN];

const fn pal(
    near: (f32, f32, f32),
    mid: (f32, f32, f32),
    far: (f32, f32, f32),
    foam: (f32, f32, f32),
    sun: (f32, f32, f32),
    sky: (f32, f32, f32),
    glint: (f32, f32, f32),
) -> Palette {
    [
        near.0, near.1, near.2, mid.0, mid.1, mid.2, far.0, far.1, far.2, foam.0, foam.1, foam.2,
        sun.0, sun.1, sun.2, sky.0, sky.1, sky.2, glint.0, glint.1, glint.2,
    ]
}

/// The sea at the height of a storm: near-black slate water, cold grey-white
/// foam, all warm light drained to pewter. The painted palette is blended toward
/// this by the gale's fury.
pub const STORM_PALETTE: Palette = pal(
    (7.0, 22.0, 30.0),
    (12.0, 38.0, 48.0),
    (34.0, 62.0, 72.0),
    (210.0, 222.0, 228.0),
    (104.0, 116.0, 126.0),
    (74.0, 92.0, 104.0),
    (150.0, 166.0, 178.0),
);

/// The target sea palette for a time of day — eased toward in the renderer.
pub fn palette_for(d: Daytime) -> Palette {
    match d {
        // Vivid tropical noon: a deep teal-blue trough rising through saturated
        // teal to a bright cyan-teal crest/horizon — the white highlights left to
        // the foam tips so the body stays richly teal rather than washing pale.
        Daytime::Day => pal(
            (6.0, 60.0, 82.0),
            (10.0, 165.0, 180.0),
            (104.0, 214.0, 226.0),
            (236.0, 253.0, 255.0),
            (255.0, 246.0, 222.0),
            (200.0, 236.0, 250.0),
            (255.0, 252.0, 240.0),
        ),
        // Golden dawn: near-black foreground water lifting through dusky purple to
        // a warm orange horizon where the sun breaks.
        Daytime::Dawn => pal(
            (18.0, 12.0, 30.0),
            (98.0, 50.0, 118.0),
            (226.0, 124.0, 70.0),
            (248.0, 226.0, 214.0),
            (255.0, 192.0, 142.0),
            (250.0, 198.0, 152.0),
            (255.0, 220.0, 172.0),
        ),
        // Fiery dusk: black foreground water set alight through deep red-purple to
        // a blazing orange horizon under the sinking blood-orange sun.
        Daytime::Dusk => pal(
            (16.0, 8.0, 22.0),
            (112.0, 30.0, 72.0),
            (242.0, 98.0, 46.0),
            (252.0, 210.0, 190.0),
            (255.0, 112.0, 60.0),
            (246.0, 120.0, 86.0),
            (255.0, 148.0, 92.0),
        ),
        // Deep night: black foreground water deepening through navy blue to a cool
        // cyan glow along the moonlit horizon.
        Daytime::Night => pal(
            (5.0, 9.0, 18.0),
            (16.0, 58.0, 112.0),
            (42.0, 152.0, 172.0),
            (198.0, 218.0, 236.0),
            (138.0, 170.0, 212.0),
            (90.0, 122.0, 172.0),
            (206.0, 222.0, 248.0),
        ),
    }
}

/// The fair-weather sky gradient (top, mid, horizon) for a time of day, from
/// `SailingView.fairSky`. Channels in [0,255].
pub fn fair_sky(d: Daytime) -> [(f32, f32, f32); 3] {
    match d {
        // Vivid blue zenith deepening to a luminous pale horizon.
        Daytime::Day => [(38.0, 150.0, 232.0), (104.0, 196.0, 242.0), (196.0, 236.0, 250.0)],
        // Violet pre-dawn up high, rose midband, warm gold at the horizon.
        Daytime::Dawn => [(66.0, 96.0, 156.0), (190.0, 150.0, 178.0), (252.0, 196.0, 140.0)],
        // Indigo dusk overhead, magenta-purple midband, a fiery red-orange horizon.
        Daytime::Dusk => [(44.0, 52.0, 104.0), (150.0, 82.0, 120.0), (244.0, 108.0, 66.0)],
        // Deep navy night, barely lifting toward the horizon.
        Daytime::Night => [(10.0, 20.0, 54.0), (22.0, 38.0, 78.0), (44.0, 60.0, 98.0)],
    }
}

/// The storm sky gradient (top, mid, horizon), from `SailingView`.
pub const STORM_SKY: [(f32, f32, f32); 3] = [
    (14.0, 19.0, 26.0),
    (45.0, 58.0, 66.0),
    (69.0, 86.0, 92.0),
];

// --- Continuous time-of-day -------------------------------------------------
// The four palettes above are keyframes spaced evenly around a 24-hour ring:
// midnight sits at phase 0, dawn at ¼, noon at ½, dusk at ¾. A clock value
// `tod` in [0,1) is blended between the two bracketing keyframes so the sky and
// sea slide smoothly through the day rather than snapping between four states.
const RING: [Daytime; 4] = [Daytime::Night, Daytime::Dawn, Daytime::Day, Daytime::Dusk];

/// The two keyframes bracketing `tod` and the smoothed blend factor between them.
fn ring_blend(tod: f32) -> (Daytime, Daytime, f32) {
    let s = tod.rem_euclid(1.0) * 4.0;
    let i = (s.floor() as usize) % 4;
    let f = s - s.floor();
    let fs = f * f * (3.0 - 2.0 * f); // smoothstep: ease through each transition
    (RING[i], RING[(i + 1) % 4], fs)
}

/// The discrete phase nearest `tod`, for the HUD label and log readout.
pub fn daytime_at(tod: f32) -> Daytime {
    let s = (tod.rem_euclid(1.0) * 4.0).round() as usize;
    RING[s % 4]
}

/// Smoothstep: 0 below `e0`, 1 above `e1`, eased in between.
#[inline]
fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How fully night has fallen, from the sun's altitude-sine (`sun_alt` per
/// `celestial::sky_state`: >0 up, <0 below the horizon): 0 in daylight, easing to 1
/// once the sun is well below the horizon. The sea palette, the painted sky
/// (`main::draw_sky`) and the sky the water reflects (`main`) all key off this, so
/// they drop to the dark-blue/cyan night palette together — no warm tint lingering on
/// the water once the sky has gone dark. The window holds the full sunset colour
/// through the moment the sun touches the horizon, then fades to full night over the
/// following dusk (~25 s at the day's pace) — long enough to enjoy the red glow off
/// the water without it overstaying into the night.
pub fn night_factor(sun_alt: f32) -> f32 {
    1.0 - smoothstep(-0.32, -0.02, sun_alt)
}

/// The sun's altitude-sine at clock `tod` (0 = midnight, ¼ sunrise, ½ noon, ¾ sunset).
fn sun_alt_at(tod: f32) -> f32 {
    ((tod.rem_euclid(1.0) - 0.25) * std::f32::consts::TAU).sin()
}

/// The sea palette at clock `tod`, blended across the day's keyframes.
pub fn sea_palette(tod: f32) -> Palette {
    let (a, b, f) = ring_blend(tod);
    let pa = palette_for(a);
    let pb = palette_for(b);
    let mut out = [0.0; PALETTE_LEN];
    for i in 0..PALETTE_LEN {
        out[i] = pa[i] + (pb[i] - pa[i]) * f;
    }
    // Once the sun is below the horizon, ease the whole palette quickly to night so
    // the fiery dusk water doesn't stay red long after sunset (and so dawn's warmth
    // holds off until the sun is about to rise), keeping sea and sky in step.
    let night_pull = night_factor(sun_alt_at(tod));
    if night_pull > 0.0 {
        let night = palette_for(Daytime::Night);
        for i in 0..PALETTE_LEN {
            out[i] += (night[i] - out[i]) * night_pull;
        }
    }
    out
}

/// The fair-weather sky gradient (top, mid, horizon) at clock `tod`, blended.
pub fn sky_gradient(tod: f32) -> [(f32, f32, f32); 3] {
    let (a, b, f) = ring_blend(tod);
    let sa = fair_sky(a);
    let sb = fair_sky(b);
    let mut out = [(0.0, 0.0, 0.0); 3];
    for i in 0..3 {
        out[i] = (
            sa[i].0 + (sb[i].0 - sa[i].0) * f,
            sa[i].1 + (sb[i].1 - sa[i].1) * f,
            sa[i].2 + (sb[i].2 - sa[i].2) * f,
        );
    }
    out
}
