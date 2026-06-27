//! Storm rain: thin streaks raking the whole viewport, the rings they punch in the
//! sea, and the little crowns they kick off the deck. Pure screen-space particles,
//! like the bow spray, but unbolted from the ship: the streaks fall across the whole
//! screen so the rain reads as weather around the captain rather than water off the
//! hull.
//!
//! Three layers, all driven by the same storm fury (so the squall fades in and out
//! with the signal that swells the thunder bed and darkens the sky):
//!   - **streaks** falling across the viewport, slanted by the apparent wind;
//!   - **ripples**: flat expanding rings on the sea, placed below the live sea line
//!     and rotated onto the rolled horizon so they lie on the water plane;
//!   - **splashes**: tiny upward crowns where the rain strikes the foreground deck
//!     (suppressed while glancing astern, since the forward deck is hidden then).

use macroquad::prelude::*;

use crate::geometry::clamp;

// --- Streaks ------------------------------------------------------------------
const DROP_RATE: f32 = 1300.0; // drops/sec spawned at full fury
const MAX_DROPS: usize = 1400; // hard cap so a long gale can't run away
const FALL_MIN: f32 = 1.9; // slowest fall, screen-heights/sec (resolution-free)
const FALL_MAX: f32 = 2.7; // fastest fall
const SLANT_FRAC: f32 = 0.55; // how much of the fall speed the wind throws sideways
const STREAK: f32 = 0.028; // streak length as a fraction of the per-second fall

// --- Ripples ------------------------------------------------------------------
const RIPPLE_RATE: f32 = 42.0; // rings/sec on the sea at full fury
const MAX_RIPPLES: usize = 130;
const RIPPLE_LIFE_MIN: f32 = 0.6;
const RIPPLE_LIFE_MAX: f32 = 1.05;
const RIPPLE_SEGS: usize = 16; // line segments per ring

// --- Splashes -----------------------------------------------------------------
const SPLASH_RATE: f32 = 95.0; // impacts/sec on the deck at full fury
const SPLASH_FLECKS: usize = 3; // flecks kicked up per impact
const MAX_SPLASH: usize = 450;
const SPLASH_GRAV: f32 = 1500.0; // px/s² pulling the crown back down
const SPLASH_LIFE_MIN: f32 = 0.16;
const SPLASH_LIFE_MAX: f32 = 0.34;

// Colours, lit by the day so they dim with the sea.
const RAIN: [f32; 3] = [188.0, 205.0, 220.0]; // cold blue-grey streaks
const RING: [f32; 3] = [202.0, 216.0, 228.0]; // a touch lighter for the sea rings
const CROWN: [f32; 3] = [224.0, 234.0, 242.0]; // brightest for the deck crowns

struct Drop {
    pos: Vec2,
    vel: Vec2,
    len: f32, // streak length (px), drawn back along the velocity
    width: f32,
    alpha: f32,
}

struct Ripple {
    pos: Vec2,
    rmax: f32,    // radius (px) the ring grows to over its life
    aspect: f32,  // ry/rx: how flat it lies (grazing view -> small)
    rot: f32,     // tilt onto the rolled sea (rad)
    age: f32,
    life: f32,
    alpha0: f32,
}

struct Fleck {
    pos: Vec2,
    vel: Vec2,
    life: f32,
    max_life: f32,
    size: f32,
}

/// Per-frame drivers for the rain, derived in the main loop.
pub struct RainInput {
    /// Gale fury, 0..1 (the eased storm signal): drives density and opacity.
    pub storm: f32,
    /// Apparent wind across the view, signed -1..1 (sin of the wind relative to the
    /// heading): positive drifts the rain to starboard. Slants the streaks.
    pub slant: f32,
    /// Daylight (1 = noon, ~0.5 deep night) so the rain dims with the sea.
    pub day_lit: f32,
    /// Screen y of the sea line (`horizon + camera pitch`): rings spawn below it.
    pub water_line: f32,
    /// Camera roll in degrees: the rings sit on this tilted horizon.
    pub roll_deg: f32,
    /// Forward deck visible (false while glancing astern): gates the deck splashes.
    pub deck: bool,
}

/// Holds the live particles and a small private RNG for spawn jitter.
pub struct Rain {
    drops: Vec<Drop>,
    ripples: Vec<Ripple>,
    flecks: Vec<Fleck>,
    rng: u32,
    drop_acc: f32, // fractional emission carried between frames
    ripple_acc: f32,
    splash_acc: f32,
}

impl Rain {
    pub fn new() -> Self {
        Rain {
            drops: Vec::new(),
            ripples: Vec::new(),
            flecks: Vec::new(),
            rng: 0x1f123bb5,
            drop_acc: 0.0,
            ripple_acc: 0.0,
            splash_acc: 0.0,
        }
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

    /// Uniform in [a, b).
    #[inline]
    fn range(&mut self, a: f32, b: f32) -> f32 {
        a + (b - a) * self.rand()
    }

    /// Advance every layer, spawn fresh rain for this frame's fury, and draw the lot.
    /// Screen-space; call after the world camera is reset and the ship is drawn (so
    /// the deck crowns sit over the planks), under the HUD.
    ///
    /// `wave_lift` drops a ring's (still-unrolled) spawn point onto the live sea:
    /// given a screen (x, y) on the flat sea plane it returns the y the wavy surface
    /// actually reaches there, so rings ride crests and troughs instead of a flat
    /// band. `deck_covers` reports whether a final (rolled) screen point sits behind
    /// the foreground deck, so rings there are hidden rather than painted on the planks.
    pub fn render(
        &mut self,
        input: &RainInput,
        wave_lift: impl Fn(f32, f32) -> f32,
        deck_covers: impl Fn(f32, f32) -> bool,
        dt: f32,
        w: f32,
        h: f32,
    ) {
        let storm = clamp(input.storm, 0.0, 1.0);
        let slant = clamp(input.slant, -1.0, 1.0);
        let lit = clamp(input.day_lit, 0.0, 1.0);
        let scale = (h / 720.0).max(0.6);

        self.streaks(storm, slant, lit, scale, dt, w, h);
        self.sea_rings(
            storm,
            lit,
            scale,
            input.water_line,
            input.roll_deg,
            wave_lift,
            deck_covers,
            dt,
            w,
            h,
        );
        self.deck_crowns(storm, lit, scale, input.deck, dt, w, h);
    }

    /// Falling streaks across the whole viewport.
    fn streaks(&mut self, storm: f32, slant: f32, lit: f32, scale: f32, dt: f32, w: f32, h: f32) {
        let off = h * 1.05;
        self.drops.retain_mut(|d| {
            d.pos.x += d.vel.x * dt;
            d.pos.y += d.vel.y * dt;
            d.pos.y < off
        });

        self.drop_acc += DROP_RATE * storm * dt;
        let n = self.drop_acc.floor();
        self.drop_acc -= n;
        for _ in 0..(n as usize) {
            if self.drops.len() >= MAX_DROPS {
                break;
            }
            let fall = self.range(FALL_MIN, FALL_MAX) * h;
            let vx = slant * fall * SLANT_FRAC;
            let vy = fall;
            // Enter from above; bias the spawn band to windward so the slanted
            // streaks still cover the screen rather than leaving a dry edge.
            let pad = w * (0.15 + 0.15 * slant.abs());
            let px = self.range(-pad, w + pad) - slant * pad;
            let py = self.range(-0.1 * h, 0.0);
            // Nearer drops (longer/brighter) versus far drizzle, for a little depth.
            let near = self.rand();
            let speed = (vx * vx + vy * vy).sqrt();
            let len = speed * STREAK * (0.7 + 0.6 * near);
            let width = scale * (0.8 + 0.9 * near);
            let alpha = (0.18 + 0.22 * near) * (0.4 + 0.6 * storm) * lit;
            self.drops.push(Drop {
                pos: vec2(px, py),
                vel: vec2(vx, vy),
                len,
                width,
                alpha,
            });
        }

        for d in &self.drops {
            let dir = d.vel.normalize_or_zero();
            let tail = d.pos - dir * d.len;
            let col = Color::new(RAIN[0] / 255.0 * lit, RAIN[1] / 255.0 * lit, RAIN[2] / 255.0 * lit, d.alpha);
            draw_line(d.pos.x, d.pos.y, tail.x, tail.y, d.width, col);
        }
    }

    /// Expanding rings punched in the sea below the live, rolled sea line.
    #[allow(clippy::too_many_arguments)]
    fn sea_rings(
        &mut self,
        storm: f32,
        lit: f32,
        scale: f32,
        water_line: f32,
        roll_deg: f32,
        wave_lift: impl Fn(f32, f32) -> f32,
        deck_covers: impl Fn(f32, f32) -> bool,
        dt: f32,
        w: f32,
        h: f32,
    ) {
        self.ripples.retain_mut(|r| {
            r.age += dt;
            r.age < r.life
        });

        let roll = roll_deg.to_radians();
        let (rs, rc) = roll.sin_cos();
        // The water band runs from a little below the sea line down to the deck.
        let band_top = water_line + 0.05 * h;
        let band_h = (h - band_top).max(0.0);

        self.ripple_acc += RIPPLE_RATE * storm * dt;
        let n = self.ripple_acc.floor();
        self.ripple_acc -= n;
        for _ in 0..(n as usize) {
            if self.ripples.len() >= MAX_RIPPLES || band_h <= 0.0 {
                break;
            }
            // depth: 0 far (near the sea line) .. 1 near (toward the deck). Spread
            // across the whole band so the rings reach right out toward the horizon
            // rather than clustering in the near few metres; perspective already
            // shrinks and flattens the far ones (rmax/aspect below scale with depth).
            let depth = self.rand();
            let bx = self.range(-0.15 * w, 1.15 * w);
            // Drop the flat-band point onto the live wave surface, so a ring that
            // lands on a crest sits higher up the screen than one in a trough.
            let by = wave_lift(bx, band_top + band_h * depth);
            // Lay the point on the rolled sea: rotate about the screen centre.
            let ox = bx - w * 0.5;
            let oy = by - h * 0.5;
            let px = w * 0.5 + ox * rc - oy * rs;
            let py = h * 0.5 + ox * rs + oy * rc;
            // The foreground deck stands in front of the sea: drop rings that would
            // land behind it rather than paint them onto the planks.
            if deck_covers(px, py) {
                continue;
            }
            let rmax = (0.016 + 0.045 * depth) * h * self.range(0.7, 1.2);
            let aspect = 0.16 + 0.26 * depth;
            let life = self.range(RIPPLE_LIFE_MIN, RIPPLE_LIFE_MAX);
            self.ripples.push(Ripple {
                pos: vec2(px, py),
                rmax,
                aspect,
                rot: roll,
                age: 0.0,
                life,
                alpha0: 0.45 * (0.4 + 0.6 * storm) * lit,
            });
        }

        let thick = scale.max(0.7);
        for r in &self.ripples {
            // Re-test occlusion each frame: a ring spawned in the clear can fall
            // behind the deck as she heels and pitches over its short life.
            if deck_covers(r.pos.x, r.pos.y) {
                continue;
            }
            let f = r.age / r.life; // 0..1
            let rx = r.rmax * f;
            let ry = rx * r.aspect;
            let alpha = r.alpha0 * (1.0 - f);
            if alpha <= 0.0 {
                continue;
            }
            let col = Color::new(RING[0] / 255.0 * lit, RING[1] / 255.0 * lit, RING[2] / 255.0 * lit, alpha);
            draw_ring(r.pos, rx, ry, r.rot, thick, col);
        }
    }

    /// Tiny crowns kicked up where the rain strikes the foreground deck.
    #[allow(clippy::too_many_arguments)]
    fn deck_crowns(&mut self, storm: f32, lit: f32, scale: f32, deck: bool, dt: f32, w: f32, h: f32) {
        let grav = SPLASH_GRAV * dt;
        self.flecks.retain_mut(|f| {
            f.vel.y += grav;
            f.pos.x += f.vel.x * dt;
            f.pos.y += f.vel.y * dt;
            f.life -= dt;
            f.life > 0.0
        });

        if deck {
            self.splash_acc += SPLASH_RATE * storm * dt;
            let n = self.splash_acc.floor();
            self.splash_acc -= n;
            for _ in 0..(n as usize) {
                if self.flecks.len() + SPLASH_FLECKS > MAX_SPLASH {
                    break;
                }
                // Impact somewhere across the foreground deck band.
                let ix = self.range(0.20 * w, 0.80 * w);
                let iy = self.range(0.82 * h, 0.97 * h);
                for _ in 0..SPLASH_FLECKS {
                    let vx = self.range(-1.0, 1.0) * self.range(40.0, 130.0);
                    let vy = -self.range(120.0, 270.0);
                    let max_life = self.range(SPLASH_LIFE_MIN, SPLASH_LIFE_MAX);
                    let size = self.range(1.2, 2.6) * scale;
                    self.flecks.push(Fleck {
                        pos: vec2(ix, iy),
                        vel: vec2(vx, vy),
                        life: max_life,
                        max_life,
                        size,
                    });
                }
            }
        } else {
            self.splash_acc = 0.0;
        }

        if !deck {
            return;
        }
        for f in &self.flecks {
            let fade = (f.life / f.max_life).clamp(0.0, 1.0);
            let alpha = fade * 0.8 * (0.4 + 0.6 * storm) * lit;
            let col = Color::new(CROWN[0] / 255.0 * lit, CROWN[1] / 255.0 * lit, CROWN[2] / 255.0 * lit, alpha);
            draw_circle(f.pos.x, f.pos.y, f.size * (0.6 + 0.4 * fade), col);
        }
    }
}

/// Draw an ellipse outline as a short polyline, centred at `c`, with horizontal /
/// vertical radii `rx`/`ry`, rotated by `rot` (rad). Used for the flat sea rings so
/// they lie on the tilted water plane.
fn draw_ring(c: Vec2, rx: f32, ry: f32, rot: f32, thick: f32, col: Color) {
    let (s, cs) = rot.sin_cos();
    let mut prev = vec2(0.0, 0.0);
    for i in 0..=RIPPLE_SEGS {
        let a = std::f32::consts::TAU * i as f32 / RIPPLE_SEGS as f32;
        let ex = rx * a.cos();
        let ey = ry * a.sin();
        let p = vec2(c.x + ex * cs - ey * s, c.y + ex * s + ey * cs);
        if i > 0 {
            draw_line(prev.x, prev.y, p.x, p.y, thick, col);
        }
        prev = p;
    }
}
