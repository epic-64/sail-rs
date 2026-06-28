//! Storm clouds: big, low, looming masses that gather as the gale builds. Each mass
//! is built from many overlapping translucent puffs (a soft base, a denser middle and
//! small lumps) so it reads as a layered volume of cloud rather than a flat blob: the
//! overlaps darken the core toward black while the thin edges let the storm sky show
//! through. The whole field drifts slowly and is projected through the same sky dome
//! as the stars, so it rolls, pitches and pans with the view.
//!
//! Lightning no longer washes the screen. Instead a charge runs quickly *across* one
//! cloud and lights it from within: a bright point sweeps the mass over a fraction of
//! a second, swelling the puffs it passes, so the cloud flickers from the inside the
//! way real sheet lightning lights a thunderhead.

use macroquad::prelude::*;

use crate::geometry::{clamp, wrap_angle};
use crate::scene::SkyView;

const NUM_CLOUDS: usize = 22;

// Clouds gather across this fury band: nothing until the weather sours, full overcast
// by the time it is blowing hard.
const AMT_LO: f32 = 0.06;
const AMT_HI: f32 = 0.62;

// Slow drift of the whole field across the sky (rad/sec), quicker in a hard blow.
const DRIFT: f32 = 0.012;

// Cloud tone, lerped by `gale` (how storm-like the fury is): a lighter grey in a
// squall (less ominous, no lightning) deepening to a cold dark slate in a full storm.
// Overlapping puffs build the core further toward black.
const CLOUD_SOFT: [f32; 3] = [80.0, 90.0, 106.0];
const CLOUD: [f32; 3] = [34.0, 39.0, 49.0];
// The fury band over which a squall's soft overcast hardens into a storm's dark slate
// (and, above `FURY_FLOOR`, starts throwing lightning). A squall sits near the low end.
const GALE_LO: f32 = 0.5;
const GALE_HI: f32 = 0.9;

// --- Internal lightning -------------------------------------------------------
// Only a real storm throws lightning, never a squall: the floor sits above the squall
// fury so the bolts hold off until the weather has built well past it.
const FURY_FLOOR: f32 = 0.62;
const GAP_CALM: f32 = 18.0; // seconds between strikes at the floor
const GAP_PEAK: f32 = 4.0; // at full fury
const GAP_JITTER: f32 = 0.5; // ± fraction of the gap
const STRIKE_MIN: f32 = 0.30; // how long a charge takes to cross a cloud
const STRIKE_MAX: f32 = 0.55;
const GLOW_MAX: f32 = 1.0; // peak lightening of a puff at the charge
const GLOW: [f32; 3] = [196.0, 214.0, 242.0]; // cold blue-white of the lit cloud

const BOLT_CHANCE: f32 = 0.5; // odds any strike draws a visible zig-zag bolt
const JUMP_CHANCE: f32 = 0.6; // odds a strike forks to a neighbouring cloud
const JUMP_MAX_AZ: f32 = 0.7; // furthest bearing gap a fork will leap (rad)
const ARC_SEGS: usize = 7; // segments in the jagged bolt
const ARC_LIFE: f32 = 0.16; // how long a bolt lingers (s)

/// One translucent puff of a cloud. Geometry is stored as fractions of screen height
/// so it scales with the viewport; `a` is its base opacity.
struct Puff {
    fx: f32,
    fy: f32,
    fr: f32,
    a: f32,
    tier: u8, // 0 = big soft base (front), 1 = body, 2 = small deep lumps (back, lit)
}

struct Cloud {
    az: f32,       // bearing across the sky (rad)
    alt: f32,      // altitude as a sine, 0 = sea line .. 1 = overhead (clouds sit low)
    parallax: f32, // drift multiplier, for a little depth between masses
    litspan: f32,  // widest deep-lump offset (fraction of h): the lightning's reach
    puffs: Vec<Puff>,
    active: bool,    // a charge is live in this cloud
    strike_age: f32, // seconds into the charge; <0 while a forked strike waits its turn
    strike_life: f32,
    sweep: f32, // +1 / -1: which way the charge runs across the mass
}

/// A jagged lightning bolt, either leaping from one cloud to another (`a` != `b`) or
/// streaking across a single mass along the charge's path (`a` == `b`). Each end is a
/// cloud centre plus a fractional offset (so it re-projects as the clouds drift), and
/// `offs` jitters the path into a zig-zag line.
struct Arc {
    a: usize,
    b: usize,
    a_off: (f32, f32),
    b_off: (f32, f32),
    age: f32, // <0 while a forked bolt is still building in the first cloud
    offs: [f32; ARC_SEGS],
}

pub struct StormSky {
    clouds: Vec<Cloud>,
    rng: u32,
    phase: f32, // accumulated drift
    next: f32,  // seconds to the next strike
    arc: Option<Arc>,
    // This frame's overall lightning glare in [0,1]: the brightest live strike (and
    // any connecting bolt), so the sea renderer can flash the water as the sky lights.
    flash: f32,
    // World bearing of that brightest strike, so the sea flash falls on the water in
    // its direction rather than washing the whole sea (see `clouds::StormSky::flash_az`).
    flash_az: f32,
}

impl StormSky {
    pub fn new() -> Self {
        let mut s = StormSky {
            clouds: Vec::new(),
            arc: None,
            rng: 0x6d2b79f5,
            phase: 0.0,
            next: GAP_CALM,
            flash: 0.0,
            flash_az: 0.0,
        };
        for _ in 0..NUM_CLOUDS {
            let cloud = s.gen_cloud();
            s.clouds.push(cloud);
        }
        s
    }

    /// xorshift32 -> [0, 1).
    #[inline]
    fn rand(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.rng = x;
        (x >> 8) as f32 / (1u32 << 24) as f32
    }

    #[inline]
    fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.rand()
    }

    /// A roughly gaussian draw in [-1, 1] (two uniforms averaged) so puffs cluster
    /// toward the cloud's heart rather than spreading evenly.
    #[inline]
    fn bell(&mut self) -> f32 {
        self.rand() + self.rand() - 1.0
    }

    /// Build one cloud: a soft wide base, a denser middle and small lumps, stacked so
    /// the overlaps read as a layered, volumetric mass.
    fn gen_cloud(&mut self) -> Cloud {
        let mut puffs = Vec::new();
        let mut litspan: f32 = 0.0;
        // (count, half-width spread, half-height spread, radius lo/hi, alpha lo/hi).
        // Wide spreads and fat radii so a single mass spans much of the sky; many of
        // them overlapping blanket it into a roiling overcast. Tier 2 (the small
        // lumps) are the deep cores the lightning lights from within.
        let tiers: [(usize, f32, f32, f32, f32, f32, f32); 3] = [
            (6, 0.36, 0.13, 0.13, 0.24, 0.045, 0.080), // soft base
            (9, 0.28, 0.11, 0.08, 0.14, 0.070, 0.110), // body
            (18, 0.32, 0.11, 0.035, 0.075, 0.060, 0.110), // lumps: many, wide-spread cores
        ];
        for (ti, (n, sx, sy, rlo, rhi, alo, ahi)) in tiers.iter().enumerate() {
            let tier = ti as u8;
            for _ in 0..*n {
                let fx = self.bell() * sx;
                let fy = self.bell() * sy;
                let fr = self.range(*rlo, *rhi);
                let a = self.range(*alo, *ahi);
                if tier == 2 {
                    litspan = litspan.max(fx.abs());
                }
                puffs.push(Puff { fx, fy, fr, a, tier });
            }
        }
        // Spread the masses across the whole sky, sea line to overhead, so the overcast
        // is total rather than a low band.
        let alt = 0.05 + 0.92 * self.rand();
        Cloud {
            az: self.range(0.0, std::f32::consts::TAU),
            alt,
            parallax: self.range(0.75, 1.25),
            litspan,
            puffs,
            active: false,
            strike_age: 0.0,
            strike_life: 0.0,
            sweep: 1.0,
        }
    }

    /// Schedule the next strike, the gap shrinking as the fury climbs.
    fn arm(&mut self, fury: f32) {
        let f = clamp((fury - FURY_FLOOR) / (1.0 - FURY_FLOOR), 0.0, 1.0);
        let gap = GAP_CALM + (GAP_PEAK - GAP_CALM) * f;
        self.next = gap * self.range(1.0 - GAP_JITTER, 1.0 + GAP_JITTER);
    }

    /// Advance, fire any lightning, and draw the cloud field. Call in the world camera
    /// just after the stars/sun/moon (so the clouds sit over them) and before the sea.
    pub fn render(&mut self, view: &SkyView, dt: f32, fury: f32, day_lit: f32, h: f32) {
        let fury = clamp(fury, 0.0, 1.0);
        let amount = smoothstep(AMT_LO, AMT_HI, fury);
        // Reset this frame's glare; the strike loop below raises it. No clouds, no flash.
        self.flash = 0.0;
        if amount <= 0.0 {
            return;
        }

        // Drift the whole field, quicker in a hard blow.
        self.phase += DRIFT * (0.4 + 0.6 * fury) * dt;

        // Age the live charges (a forked strike runs from a negative age, so it waits
        // its turn before it begins glowing).
        for c in &mut self.clouds {
            if c.active {
                c.strike_age += dt;
                if c.strike_age >= c.strike_life {
                    c.active = false;
                }
            }
        }
        // Age the connecting bolt.
        if let Some(arc) = &mut self.arc {
            arc.age += dt;
            if arc.age >= ARC_LIFE {
                self.arc = None;
            }
        }

        // Fire the next strike into a cloud that is on screen.
        if fury > FURY_FLOOR {
            self.next -= dt;
            if self.next <= 0.0 {
                self.strike(view);
                self.arm(fury);
            }
        }

        // How storm-like the fury is: a squall keeps the soft, lighter overcast; a full
        // storm deepens it to dark slate. Drives the cloud tone (and, past `FURY_FLOOR`,
        // the lightning above).
        let gale = smoothstep(GALE_LO, GALE_HI, fury);
        let tone = [
            CLOUD_SOFT[0] + (CLOUD[0] - CLOUD_SOFT[0]) * gale,
            CLOUD_SOFT[1] + (CLOUD[1] - CLOUD_SOFT[1]) * gale,
            CLOUD_SOFT[2] + (CLOUD[2] - CLOUD_SOFT[2]) * gale,
        ];
        // Ambient light the clouds catch: dark at night, slate by day.
        let ambient = 0.35 + 0.65 * clamp(day_lit, 0.0, 1.0);
        let base = Color::new(
            tone[0] / 255.0 * ambient,
            tone[1] / 255.0 * ambient,
            tone[2] / 255.0 * ambient,
            1.0,
        );

        // The brightest live strike this frame and its bearing, fed to the sea renderer
        // so the water flashes with the sky, on the strike's side. Raised below.
        let mut flash = 0.0f32;
        let mut flash_az = 0.0f32;
        for c in &self.clouds {
            let az = wrap_angle(c.az + self.phase * c.parallax);
            let Some((cx, cy)) = project(az, c.alt, view) else {
                continue;
            };
            // Soft fade as a mass nears the edge of view, so clouds slide in rather
            // than pop.
            let rel = wrap_angle(az - view.heading).abs();
            let edge = 1.0 - smoothstep(view.half_fov_h * 0.85, view.half_fov_h * 1.15, rel);
            let vis = amount * edge;
            if vis <= 0.0 {
                continue;
            }

            // Draw back-to-front: the small deep lumps (tier 2) first, then the body
            // and the big soft base in front. The lightning lights *only* the deep
            // lumps, so it glows from inside the cloud and the puffs drawn over it
            // diffuse the light, rather than the whole mass flaring at once.
            let dark = |p: &Puff| {
                let col = Color::new(base.r, base.g, base.b, (p.a * vis).min(1.0));
                draw_circle(cx + p.fx * h, cy + p.fy * h, p.fr * h, col);
            };

            // Back: the deep lumps.
            for p in c.puffs.iter().filter(|p| p.tier == 2) {
                dark(p);
            }

            // Lightning inside the cloud: a charge sweeping the deep lumps lights the
            // ones it passes, so the glow travels through the cloud's core.
            if c.active && c.strike_age >= 0.0 && c.litspan > 0.0 {
                let q = clamp(c.strike_age / c.strike_life, 0.0, 1.0);
                let light_x = cx + (-c.litspan + 2.0 * c.litspan * q) * c.sweep * h;
                // Fast attack, slower decay, with a high-frequency flicker over the top.
                let env = (4.0 * q * (1.0 - q)).max(0.0);
                let flick = 0.7 + 0.3 * (q * 19.0).sin().abs();
                let intensity = env * flick;
                // This mass's contribution to the scene glare (faded with its on-screen
                // visibility), so a near, bright strike flashes the sea more than a
                // faint one sliding off the edge of view. The brightest sets the bearing.
                let contrib = intensity * vis;
                if contrib > flash {
                    flash = contrib;
                    flash_az = az;
                }
                let reach = (c.litspan * h * 0.5).max(1.0);
                let inv2 = 1.0 / (2.0 * reach * reach);
                for p in c.puffs.iter().filter(|p| p.tier == 2) {
                    let px = cx + p.fx * h;
                    let py = cy + p.fy * h;
                    let dx = px - light_x;
                    let dy = py - cy;
                    let fall = (-(dx * dx + dy * dy) * inv2).exp();
                    let ga = intensity * fall * GLOW_MAX * vis;
                    if ga <= 0.01 {
                        continue;
                    }
                    draw_circle(
                        px,
                        py,
                        p.fr * h * 1.15,
                        Color::new(GLOW[0] / 255.0, GLOW[1] / 255.0, GLOW[2] / 255.0, ga.min(0.95)),
                    );
                }
            }

            // Front: the body, then the big soft base, over the glow.
            for p in c.puffs.iter().filter(|p| p.tier == 1) {
                dark(p);
            }
            for p in c.puffs.iter().filter(|p| p.tier == 0) {
                dark(p);
            }
        }

        // The fork: a jagged bolt arcing from one cloud into the next, drawn over the
        // masses so it reads as the discharge leaping the gap.
        if let Some(arc) = &self.arc {
            if arc.age >= 0.0 {
                self.draw_arc(arc, view, amount, h);
                // The bolt's snap adds to the glare (matching `draw_arc`'s own fade),
                // its bearing taken from the cloud it leaps from.
                let q = clamp(arc.age / ARC_LIFE, 0.0, 1.0);
                let contrib = (1.0 - q) * amount;
                if contrib > flash {
                    flash = contrib;
                    let ca = &self.clouds[arc.a];
                    flash_az = wrap_angle(ca.az + self.phase * ca.parallax);
                }
            }
        }
        self.flash = flash;
        self.flash_az = flash_az;
    }

    /// This frame's overall lightning glare in [0,1] (0 when no strike is live), for
    /// the sea renderer to flash the water as the sky lights. Valid after [`render`].
    pub fn flash(&self) -> f32 {
        self.flash
    }

    /// The world bearing of this frame's brightest strike, so the sea flash falls on
    /// the water in its direction rather than everywhere. Valid after [`render`].
    pub fn flash_az(&self) -> f32 {
        self.flash_az
    }

    /// Draw a bolt as a jagged, fading line between its two (offset) endpoints.
    fn draw_arc(&self, arc: &Arc, view: &SkyView, amount: f32, h: f32) {
        let pa = self.cloud_screen(arc.a, view);
        let pb = self.cloud_screen(arc.b, view);
        let (Some(ca), Some(cb)) = (pa, pb) else {
            return;
        };
        // Each end rides its cloud's centre, offset within the mass (zero for the two
        // cloud centres of a leaping fork).
        let a = vec2(ca.x + arc.a_off.0 * h, ca.y + arc.a_off.1 * h);
        let b = vec2(cb.x + arc.b_off.0 * h, cb.y + arc.b_off.1 * h);
        let q = clamp(arc.age / ARC_LIFE, 0.0, 1.0);
        let bright = (1.0 - q) * amount; // a quick snap then fade
        if bright <= 0.01 {
            return;
        }
        // Perpendicular to the run, to jitter the path off the straight line.
        let dir = vec2(b.x - a.x, b.y - a.y);
        let span = (dir.x * dir.x + dir.y * dir.y).sqrt().max(1.0);
        let perp = vec2(-dir.y / span, dir.x / span);
        let jag = (span * 0.10).min(h * 0.05);
        let core = Color::new(GLOW[0] / 255.0, GLOW[1] / 255.0, GLOW[2] / 255.0, bright.min(0.95));
        let halo = Color::new(GLOW[0] / 255.0, GLOW[1] / 255.0, GLOW[2] / 255.0, (bright * 0.3).min(0.6));
        let mut prev = a;
        for i in 1..=ARC_SEGS {
            let f = i as f32 / ARC_SEGS as f32;
            // Zero the wander at the two ends, fullest in the middle.
            let taper = (f * (1.0 - f)) * 4.0;
            let off = if i < ARC_SEGS { arc.offs[i - 1] * jag * taper } else { 0.0 };
            let mid = vec2(a.x + dir.x * f + perp.x * off, a.y + dir.y * f + perp.y * off);
            draw_line(prev.x, prev.y, mid.x, mid.y, (h * 0.006).max(2.0), halo);
            draw_line(prev.x, prev.y, mid.x, mid.y, (h * 0.0022).max(1.0), core);
            prev = mid;
        }
    }

    /// The screen point of a cloud's centre this frame, or `None` if off to the side.
    fn cloud_screen(&self, i: usize, view: &SkyView) -> Option<Vec2> {
        let c = &self.clouds[i];
        let az = wrap_angle(c.az + self.phase * c.parallax);
        project(az, c.alt, view).map(|(x, y)| vec2(x, y))
    }

    /// Fire a strike into a cloud that is on screen, sometimes forking on into a
    /// neighbouring cloud with a bolt leaping the gap.
    fn strike(&mut self, view: &SkyView) {
        // Gather the in-view, idle clouds with their bearing and screen x.
        let mut shown: Vec<(usize, f32, f32)> = Vec::new();
        for (i, c) in self.clouds.iter().enumerate() {
            if c.active {
                continue;
            }
            let az = wrap_angle(c.az + self.phase * c.parallax);
            if let Some((x, _)) = project(az, c.alt, view) {
                shown.push((i, az, x));
            }
        }
        if shown.is_empty() {
            // Nothing to light; glance back soon.
            self.next = 0.5;
            return;
        }

        let (a_idx, a_az, a_x) = shown[(self.rand() * shown.len() as f32) as usize % shown.len()];
        let life_a = self.range(STRIKE_MIN, STRIKE_MAX);

        // Maybe fork to the nearest other in-view cloud within reach.
        let mut fork: Option<usize> = None;
        if self.rand() < JUMP_CHANCE && shown.len() > 1 {
            let mut best = JUMP_MAX_AZ;
            for &(j, jaz, _) in &shown {
                if j == a_idx {
                    continue;
                }
                let d = wrap_angle(jaz - a_az).abs();
                if d < best {
                    best = d;
                    fork = Some(j);
                }
            }
        }

        // The charge runs toward the cloud it forks into (if any).
        let toward = fork
            .and_then(|j| shown.iter().find(|&&(k, _, _)| k == j))
            .map(|&(_, _, bx)| if bx >= a_x { 1.0 } else { -1.0 });
        let a_sweep = toward.unwrap_or(if self.rand() < 0.5 { -1.0 } else { 1.0 });

        {
            let c = &mut self.clouds[a_idx];
            c.active = true;
            c.strike_age = 0.0;
            c.strike_life = life_a;
            c.sweep = a_sweep;
        }

        // Half of all strikes draw a visible zig-zag bolt (whether or not the charge
        // jumps); the rest light the cloud from within as a silent sheet flash. Roll it
        // and the jitter up front so both bolt kinds below share them.
        let show_bolt = self.rand() < BOLT_CHANCE;
        let mut offs = [0.0_f32; ARC_SEGS];
        if show_bolt {
            for o in &mut offs {
                *o = self.range(-1.0, 1.0);
            }
        }

        if let Some(b_idx) = fork {
            let life_b = self.range(STRIKE_MIN, STRIKE_MAX);
            // The fork waits until the first charge is most of the way across, then
            // the second cloud lights (and the bolt, if shown, leaps the gap).
            let delay = life_a * 0.55;
            {
                let c = &mut self.clouds[b_idx];
                c.active = true;
                c.strike_age = -delay;
                c.strike_life = life_b;
                c.sweep = a_sweep;
            }
            if show_bolt {
                self.arc = Some(Arc {
                    a: a_idx,
                    b: b_idx,
                    a_off: (0.0, 0.0),
                    b_off: (0.0, 0.0),
                    age: -delay,
                    offs,
                });
            }
        } else if show_bolt {
            // No jump: the bolt streaks across the striking cloud along the charge's own
            // path (the lit span, in the sweep direction), zig-zagging as it travels.
            let span = self.clouds[a_idx].litspan.max(0.08);
            self.arc = Some(Arc {
                a: a_idx,
                b: a_idx,
                a_off: (-span * a_sweep, 0.0),
                b_off: (span * a_sweep, 0.0),
                age: 0.0,
                offs,
            });
        }
    }
}

/// Map a cloud's bearing + altitude to a screen point in the world camera, the same
/// way the stars are placed. `None` when it is off to the side.
fn project(az: f32, alt: f32, view: &SkyView) -> Option<(f32, f32)> {
    let rel = wrap_angle(az - view.heading);
    if rel.abs() > view.half_fov_h * 1.15 {
        return None;
    }
    let x = view.w * 0.5 + (rel / view.half_fov_h) * (view.w * 0.5);
    let y = view.horizon - alt * view.horizon * 0.95;
    Some((x, y))
}

/// Smooth 0->1 ramp between `a` and `b`.
#[inline]
fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    if b <= a {
        return if x < a { 0.0 } else { 1.0 };
    }
    let t = clamp((x - a) / (b - a), 0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
