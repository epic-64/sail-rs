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
        Daytime::Day => pal(
            (10.0, 100.0, 152.0),
            (18.0, 146.0, 200.0),
            (58.0, 172.0, 212.0),
            (234.0, 253.0, 255.0),
            (255.0, 247.0, 224.0),
            (214.0, 241.0, 251.0),
            (255.0, 252.0, 240.0),
        ),
        Daytime::Dawn => pal(
            (12.0, 94.0, 148.0),
            (24.0, 138.0, 192.0),
            (76.0, 168.0, 206.0),
            (240.0, 238.0, 235.0),
            (255.0, 206.0, 150.0),
            (250.0, 205.0, 180.0),
            (255.0, 224.0, 176.0),
        ),
        Daytime::Dusk => pal(
            (14.0, 84.0, 134.0),
            (26.0, 124.0, 174.0),
            (92.0, 154.0, 192.0),
            (250.0, 225.0, 215.0),
            (255.0, 150.0, 92.0),
            (244.0, 150.0, 118.0),
            (255.0, 184.0, 120.0),
        ),
        Daytime::Night => pal(
            (8.0, 62.0, 112.0),
            (12.0, 90.0, 148.0),
            (32.0, 120.0, 168.0),
            (200.0, 220.0, 236.0),
            (150.0, 178.0, 214.0),
            (96.0, 128.0, 176.0),
            (208.0, 224.0, 248.0),
        ),
    }
}

/// The fair-weather sky gradient (top, mid, horizon) for a time of day, from
/// `SailingView.fairSky`. Channels in [0,255].
pub fn fair_sky(d: Daytime) -> [(f32, f32, f32); 3] {
    match d {
        Daytime::Day => [(56.0, 169.0, 228.0), (127.0, 205.0, 240.0), (214.0, 241.0, 251.0)],
        Daytime::Dawn => [(96.0, 165.0, 199.0), (153.0, 194.0, 209.0), (222.0, 223.0, 218.0)],
        Daytime::Dusk => [(98.0, 140.0, 176.0), (148.0, 165.0, 184.0), (209.0, 190.0, 192.0)],
        Daytime::Night => [(26.0, 73.0, 115.0), (53.0, 87.0, 120.0), (86.0, 100.0, 124.0)],
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

/// The sea palette at clock `tod`, blended across the day's keyframes.
pub fn sea_palette(tod: f32) -> Palette {
    let (a, b, f) = ring_blend(tod);
    let pa = palette_for(a);
    let pb = palette_for(b);
    let mut out = [0.0; PALETTE_LEN];
    for i in 0..PALETTE_LEN {
        out[i] = pa[i] + (pb[i] - pa[i]) * f;
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
