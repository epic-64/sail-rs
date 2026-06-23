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
    pub const CYCLE: [Daytime; 4] = [Daytime::Dawn, Daytime::Day, Daytime::Dusk, Daytime::Night];

    pub fn label(self) -> &'static str {
        match self {
            Daytime::Dawn => "Dawn",
            Daytime::Day => "Day",
            Daytime::Dusk => "Dusk",
            Daytime::Night => "Night",
        }
    }

    /// The next daytime in the cycle, wrapping from night back to dawn.
    pub fn next(self) -> Daytime {
        let i = Self::CYCLE.iter().position(|&d| d == self).unwrap_or(0);
        Self::CYCLE[(i + 1) % Self::CYCLE.len()]
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
            (10.0, 111.0, 124.0),
            (17.0, 160.0, 168.0),
            (60.0, 184.0, 182.0),
            (234.0, 253.0, 255.0),
            (255.0, 247.0, 224.0),
            (214.0, 241.0, 251.0),
            (255.0, 252.0, 240.0),
        ),
        Daytime::Dawn => pal(
            (12.0, 104.0, 120.0),
            (24.0, 150.0, 160.0),
            (78.0, 180.0, 180.0),
            (240.0, 238.0, 235.0),
            (255.0, 206.0, 150.0),
            (250.0, 205.0, 180.0),
            (255.0, 224.0, 176.0),
        ),
        Daytime::Dusk => pal(
            (14.0, 92.0, 108.0),
            (26.0, 135.0, 142.0),
            (95.0, 165.0, 168.0),
            (250.0, 225.0, 215.0),
            (255.0, 150.0, 92.0),
            (244.0, 150.0, 118.0),
            (255.0, 184.0, 120.0),
        ),
        Daytime::Night => pal(
            (8.0, 70.0, 90.0),
            (12.0, 98.0, 118.0),
            (34.0, 128.0, 140.0),
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
