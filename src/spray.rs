//! Bow spray: foam flung up at the stem and off the two bow shoulders as the ship
//! drives through the swell. Pure screen-space particles bolted to the foreground
//! ship (like the deck and rig) — emitted from the bow, launched up and outward,
//! then left to arc under gravity and fade.
//!
//! Emission strengthens with **speed** (the standing bow wave) and **bursts** when
//! the bow slams into a wave — frontally (the stem plunging into a face, read off
//! the bow's downward heave) or from the side (a hard, fast roll into a crest). The
//! emitters are static in screen space; once thrown, each droplet flies on its own,
//! so the sheet reads as water torn loose from the hull rather than glued to it.

use macroquad::prelude::*;

use crate::geometry::clamp;

// --- Emission rates (particles/sec at full strength) --------------------------
const BOW_RATE: f32 = 95.0; // standing bow plume at full speed
const SIDE_RATE: f32 = 75.0; // bow-wave shoulders at full speed (split both sides)
const SLAM_RATE: f32 = 750.0; // frontal slam burst (brief, so it reads as a sheet)
const HEEL_RATE: f32 = 360.0; // extra off the lee shoulder when she rolls into a sea

// --- Particle flight ----------------------------------------------------------
const GRAVITY: f32 = 1150.0; // px/s² pulling droplets back down
const LIFE_MIN: f32 = 0.35;
const LIFE_MAX: f32 = 0.85;
const MAX_PARTICLES: usize = 700; // hard cap so a long slam can't run away
const SPIN_MAX: f32 = 9.0; // rad/s a fleck can tumble (sign random)

// Foam colour (lit by the day), faintly blue-white.
const FOAM: [f32; 3] = [236.0, 244.0, 248.0];

struct Particle {
    pos: Vec2,
    vel: Vec2,
    life: f32,
    max_life: f32,
    size: f32,
    rot: f32,  // current rotation (rad)
    spin: f32, // rad/s the fleck tumbles
}

/// Per-frame drivers for the spray, all derived in the main loop.
pub struct SprayInput {
    /// Ship speed as a fraction of top speed, 0..1 — the standing bow wave.
    pub speed_frac: f32,
    /// Frontal wave impact this frame, 0..1 — the bow plunging into a face.
    pub slam: f32,
    /// Signed roll rate (rad/s): positive heels to starboard. Drives the lee
    /// shoulder's extra spray when she rolls hard into a sea.
    pub heel_rate: f32,
    /// Daylight (1 = noon, ~0.5 deep night) so the foam dims with the sea.
    pub day_lit: f32,
}

/// Holds the live droplets and a small private RNG for launch jitter.
pub struct Spray {
    parts: Vec<Particle>,
    rng: u32,
    // Fractional emission carried between frames so low rates still drip.
    acc_bow: f32,
    acc_left: f32,
    acc_right: f32,
}

impl Spray {
    pub fn new() -> Self {
        Spray {
            parts: Vec::new(),
            rng: 0x9e3779b9,
            acc_bow: 0.0,
            acc_left: 0.0,
            acc_right: 0.0,
        }
    }

    /// xorshift32 → [0, 1).
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

    /// Advance the existing droplets, emit fresh spray for this frame's drivers,
    /// and draw the lot. Screen-space; call after the world camera is reset and the
    /// ship is drawn, so the foam sits in front of the bow.
    pub fn render(&mut self, input: &SprayInput, dt: f32, w: f32, h: f32) {
        // --- Integrate + cull the live droplets --------------------------------
        let grav = GRAVITY * dt;
        self.parts.retain_mut(|p| {
            p.vel.y += grav;
            p.pos.x += p.vel.x * dt;
            p.pos.y += p.vel.y * dt;
            p.rot += p.spin * dt;
            p.life -= dt;
            p.life > 0.0
        });

        // --- Emission strengths ------------------------------------------------
        let speed = clamp(input.speed_frac, 0.0, 1.0);
        // The standing bow wave only really stands up once she has way on.
        let way = clamp((speed - 0.12) / 0.88, 0.0, 1.0);
        // A slam throws spray even at modest speed, but a dead-in-the-water hull
        // bobbing in swell shouldn't fountain — floor it on a little way.
        let slam = clamp(input.slam, 0.0, 1.0) * (0.35 + 0.65 * speed);
        // How hard she is rolling, split to the lee shoulder by sign.
        let heel = clamp(input.heel_rate.abs(), 0.0, 2.5);

        // Launch energy scales with whichever is driving hardest.
        let drive = way.max(slam);

        // Emitter anchors (screen space). The bow sits up-screen at the stem; the
        // shoulders a touch aft and out to either side where the bow wave parts.
        let cx = w * 0.5;
        let bow_y = h * 0.72;
        let shoulder_y = h * 0.80;
        let shoulder_dx = w * 0.15;

        // Bow plume: up the screen with a little spread, bursting on a slam.
        let bow_rate = BOW_RATE * way + SLAM_RATE * slam;
        self.acc_bow += bow_rate * dt;
        let n_bow = self.acc_bow.floor();
        self.acc_bow -= n_bow;
        for _ in 0..(n_bow as usize) {
            let energy = self.range(0.55, 1.0) * (0.45 + 0.55 * drive);
            let ang = self.range(-0.45, 0.45); // from straight up
            let speed_px = self.range(380.0, 720.0) * energy;
            let vx = speed_px * ang.sin() + self.range(-90.0, 90.0);
            let vy = -speed_px * ang.cos();
            let px = cx + self.range(-w * 0.035, w * 0.035);
            let py = bow_y + self.range(-h * 0.01, h * 0.01);
            let size = self.range(2.0, 4.6) * (0.7 + 0.5 * drive);
            self.push(vec2(px, py), vec2(vx, vy), size);
        }

        // Shoulders: outward + up off each bow. The lee shoulder (the side she is
        // rolling toward) gets the heel burst — positive heel_rate → starboard.
        let heel_r = if input.heel_rate > 0.0 { heel } else { 0.0 };
        let heel_l = if input.heel_rate < 0.0 { heel } else { 0.0 };
        let side_base = SIDE_RATE * 0.5 * way;
        let rate_l = side_base + HEEL_RATE * heel_l + SLAM_RATE * 0.35 * slam;
        let rate_r = side_base + HEEL_RATE * heel_r + SLAM_RATE * 0.35 * slam;

        self.acc_left += rate_l * dt;
        let n_left = self.acc_left.floor();
        self.acc_left -= n_left;
        self.emit_shoulder(n_left as usize, -1.0, cx - shoulder_dx, shoulder_y, drive.max(heel_l.min(1.0)), w, h);

        self.acc_right += rate_r * dt;
        let n_right = self.acc_right.floor();
        self.acc_right -= n_right;
        self.emit_shoulder(n_right as usize, 1.0, cx + shoulder_dx, shoulder_y, drive.max(heel_r.min(1.0)), w, h);

        // --- Draw --------------------------------------------------------------
        let lit = clamp(input.day_lit, 0.0, 1.0);
        for p in &self.parts {
            // Fade in fast off the hull, then ease out over the fleck's life.
            let f = p.life / p.max_life;
            let alpha = (f * 1.3).min(0.85) * 0.9;
            let col = Color::new(
                FOAM[0] / 255.0 * lit,
                FOAM[1] / 255.0 * lit,
                FOAM[2] / 255.0 * lit,
                alpha,
            );
            // A small tumbling square, shrinking a touch as it thins out. Built as
            // two triangles from the rotated corners so it spins as it flies.
            let s = p.size * (0.5 + 0.5 * f);
            let (sin, cos) = p.rot.sin_cos();
            let corner = |lx: f32, ly: f32| {
                vec2(p.pos.x + lx * cos - ly * sin, p.pos.y + lx * sin + ly * cos)
            };
            let a = corner(-s, -s);
            let b = corner(s, -s);
            let c = corner(s, s);
            let d = corner(-s, s);
            draw_triangle(a, b, c, col);
            draw_triangle(a, c, d, col);
        }
    }

    /// Emit `n` droplets off a bow shoulder. `side` is -1 (port) or +1 (starboard);
    /// they fly outward and up, fanning away from the hull.
    fn emit_shoulder(&mut self, n: usize, side: f32, ax: f32, ay: f32, drive: f32, w: f32, h: f32) {
        for _ in 0..n {
            let energy = self.range(0.5, 1.0) * (0.45 + 0.55 * drive);
            // Mostly outward, canted up — the sheet a bow wave throws to the side.
            let out = self.range(260.0, 560.0) * energy;
            let up = self.range(180.0, 460.0) * energy;
            let vx = side * out + self.range(-60.0, 60.0);
            let vy = -up;
            let px = ax + self.range(-w * 0.025, w * 0.025);
            let py = ay + self.range(-h * 0.012, h * 0.012);
            let size = self.range(1.8, 4.0) * (0.7 + 0.5 * drive);
            self.push(vec2(px, py), vec2(vx, vy), size);
        }
    }

    #[inline]
    fn push(&mut self, pos: Vec2, vel: Vec2, size: f32) {
        if self.parts.len() >= MAX_PARTICLES {
            return;
        }
        let max_life = self.range(LIFE_MIN, LIFE_MAX);
        let rot = self.range(0.0, std::f32::consts::TAU);
        let spin = self.range(-SPIN_MAX, SPIN_MAX);
        self.parts.push(Particle {
            pos,
            vel,
            life: max_life,
            max_life,
            size,
            rot,
            spin,
        });
    }
}
