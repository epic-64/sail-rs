//! Storm rain: thin streaks raking the whole viewport, thickening with the gale's
//! fury and slanting with the wind. Pure screen-space particles, like the bow spray,
//! but unbolted from the ship: they fall across the entire screen so the rain reads
//! as weather around the captain rather than water off the hull.
//!
//! Each drop spawns just above the top edge, falls fast (canted by the apparent wind)
//! and is culled once it crosses the bottom. The live population scales with the
//! storm fury, so the squall fades in and out with the same signal that swells the
//! thunder bed and darkens the sky.

use macroquad::prelude::*;

use crate::geometry::clamp;

// --- Emission -----------------------------------------------------------------
const DROP_RATE: f32 = 1300.0; // drops/sec spawned at full fury
const MAX_DROPS: usize = 1400; // hard cap so a long gale can't run away

// --- Flight (all speeds are fractions of screen height/sec, so resolution-free) -
const FALL_MIN: f32 = 1.9; // slowest fall, screen-heights/sec
const FALL_MAX: f32 = 2.7; // fastest fall
const SLANT_FRAC: f32 = 0.55; // how much of the fall speed the wind throws sideways
const STREAK: f32 = 0.028; // streak length as a fraction of the per-second fall

// Rain colour: a cold blue-grey, lit by the day so it dims with the sea.
const RAIN: [f32; 3] = [188.0, 205.0, 220.0];

struct Drop {
    pos: Vec2,
    vel: Vec2,
    len: f32, // streak length (px), drawn back along the velocity
    width: f32,
    alpha: f32,
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
}

/// Holds the live drops and a small private RNG for spawn jitter.
pub struct Rain {
    drops: Vec<Drop>,
    rng: u32,
    acc: f32, // fractional emission carried between frames
}

impl Rain {
    pub fn new() -> Self {
        Rain {
            drops: Vec::new(),
            rng: 0x1f123bb5,
            acc: 0.0,
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

    /// Advance the live drops, spawn fresh rain for this frame's fury, and draw the
    /// lot. Screen-space; call after the world camera is reset, under the HUD.
    pub fn render(&mut self, input: &RainInput, dt: f32, w: f32, h: f32) {
        let storm = clamp(input.storm, 0.0, 1.0);
        let slant = clamp(input.slant, -1.0, 1.0);

        // --- Integrate + cull --------------------------------------------------
        let off = h + 0.05 * h;
        self.drops.retain_mut(|d| {
            d.pos.x += d.vel.x * dt;
            d.pos.y += d.vel.y * dt;
            d.pos.y < off
        });

        // --- Spawn -------------------------------------------------------------
        // Density ramps with fury; nothing falls in fair weather.
        self.acc += DROP_RATE * storm * dt;
        let n = self.acc.floor();
        self.acc -= n;
        let lit = clamp(input.day_lit, 0.0, 1.0);
        for _ in 0..(n as usize) {
            if self.drops.len() >= MAX_DROPS {
                break;
            }
            let fall = self.range(FALL_MIN, FALL_MAX) * h;
            let vx = slant * fall * SLANT_FRAC;
            let vy = fall;
            // Enter from above; bias the spawn band toward the windward side so the
            // slanted streaks still cover the screen rather than leaving a dry edge.
            let pad = w * (0.15 + 0.15 * slant.abs());
            let px = self.range(-pad, w + pad) - slant * pad;
            let py = self.range(-0.1 * h, 0.0);
            // Nearer drops (longer/brighter) versus far drizzle, for a little depth.
            let near = self.range(0.0, 1.0);
            let speed = (vx * vx + vy * vy).sqrt();
            let len = speed * STREAK * (0.7 + 0.6 * near);
            let scale = (h / 720.0).max(0.6);
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

        // --- Draw --------------------------------------------------------------
        for d in &self.drops {
            let dir = d.vel.normalize_or_zero();
            let tail = d.pos - dir * d.len;
            let col = Color::new(
                RAIN[0] / 255.0 * lit,
                RAIN[1] / 255.0 * lit,
                RAIN[2] / 255.0 * lit,
                d.alpha,
            );
            draw_line(d.pos.x, d.pos.y, tail.x, tail.y, d.width, col);
        }
    }
}
