//! The player's own ship in the foreground: deck, bulwarks, wheel, mast, yard and
//! a square sail that braces, bellies and luffs. Flat-shaded low-poly geometry to
//! match the waves and islands — *not* the original's painted `deck*.png` bolted
//! to the camera with CSS `perspective()`/`rotateX` transforms.
//!
//! The whole assembly is pinned to the bottom-centre of the viewport and sways as
//! a rigid body with the swell (heave/pitch/roll/yaw from [`crate::ocean::ship_motion`]),
//! about a pivot below the screen so the masthead arcs as the hull rolls. On top
//! of that rigid sway the rig *articulates*:
//!
//! - the **yard** braces about the mast's vertical axis to trim to the wind,
//! - the **sail** bows into a parabolic belly out of plane (deepest amidships),
//! - and **luffs** — a travelling ripple flogs the cloth when starved of wind.
//!
//! The belly/brace/luff are built in a small local 3-D rig space (x across, y up,
//! z toward the viewer) and projected through a gentle fake perspective, so a
//! braced-and-bellied sail still reads as a curved surface from any angle. The
//! trim is driven by the real [`crate::sailing::Wind`]: the caller passes the
//! wind's bearing relative to the bow (`wind_rel`), and the sail bellies by the
//! same `Wind::factor` curve the physics uses, so it luffs exactly when the ship
//! is in irons.

use macroquad::prelude::*;

use crate::geometry::clamp;
use crate::ocean::ShipMotion;
use crate::palette::Daytime;
use crate::sailing::wind_factor_rel;

use std::f32::consts::TAU;

// --- Rig trim feel (ported from SailingView) ---------------------------------
const SAIL_PANELS: usize = 8; // vertical cloth panels the sail is built from
const PANEL_OVERLAP: f32 = 1.75; // each strip wider than its slot so neighbours overlap
const BELLY_DEPTH: f32 = 0.37; // deepest draft, as a fraction of sail width
const FLAP_HZ: f32 = 1.6; // luff flutter rate
const FLAP_WAVES: f32 = 1.6; // ripple crests across the sail at once
const FLAP_DEPTH: f32 = 0.035; // deepest a flog throws a panel, fraction of width
const BRACE_LIMIT: f32 = 1.4; // hard brace (~80°) reached by a beam wind
const BRACE_EASE: f32 = 2.5; // 1/s the crew haul the yard toward its trim
const WHEEL_EASE: f32 = 5.0; // 1/s the wheel chases the rudder input

// --- How the swell's sway is split (the deck takes the bulk) ------------------
const DECK_SHARE: f32 = 0.6;
const YAW_SWAY_PX: f32 = 180.0; // px of pan per rad of hull yaw
const PITCH_CLIMB: f32 = 1.3;
const PITCH_DIVE: f32 = 2.0;
const PITCH_DIVE_KNEE: f32 = 0.12; // rad of bow-down at which the dive boost is full

// Gentle perspective focal length (px) for the rig's local 3-D, matched to the
// original's 1600px so the belly and brace stay shallow, not fish-eyed.
const FOCAL: f32 = 1600.0;

// --- Wood / canvas palette (harmonises with the island features' wood tones) --
const SAIL_CLOTH: [f32; 3] = [226.0, 214.0, 188.0];
const DECK_A: [f32; 3] = [156.0, 120.0, 74.0];
const DECK_B: [f32; 3] = [138.0, 104.0, 62.0];
const RAIL: [f32; 3] = [120.0, 86.0, 52.0];
const RAIL_DK: [f32; 3] = [92.0, 64.0, 38.0];
const SPAR: [f32; 3] = [120.0, 88.0, 56.0];
const SPAR_DK: [f32; 3] = [90.0, 64.0, 40.0];
const WHEEL_C: [f32; 3] = [134.0, 98.0, 58.0];
const WHEEL_DK: [f32; 3] = [96.0, 68.0, 40.0];

/// Per-frame trim the rig is steered by. `wind_rel` is the prevailing wind's
/// bearing relative to the bow (0 = wind from dead astern, ±π = dead ahead).
pub struct RigInput {
    /// Hull sway this frame (roll/yaw already low-passed by the caller).
    pub motion: ShipMotion,
    /// Canvas set, 0 (furled) … 1 (full sail) — the chosen sail fraction.
    pub set: f32,
    /// Rudder demand, [-1, 1] — the wheel leads it.
    pub turn: f32,
    /// Wind bearing relative to the bow: `wrap(toward - heading)`, 0 = tailwind.
    pub wind_rel: f32,
}

/// Holds the eased animation state (wheel spin, yard brace) between frames.
pub struct ShipRenderer {
    wheel_angle: f32,
    brace_angle: f32,
}

#[inline]
fn rgba(c: [f32; 3], shade: f32, a: f32) -> Color {
    Color::new(c[0] / 255.0 * shade, c[1] / 255.0 * shade, c[2] / 255.0 * shade, a)
}

/// The bow's shaped answer to the swell: it climbs a wave gently but noses down
/// hard into the trough, eased in with depth so it stays smooth through the crest.
fn pitch_response(pitch: f32) -> f32 {
    let dive = clamp(-pitch / PITCH_DIVE_KNEE, 0.0, 1.0);
    pitch * (PITCH_CLIMB + (PITCH_DIVE - PITCH_CLIMB) * dive)
}


impl ShipRenderer {
    pub fn new() -> Self {
        ShipRenderer {
            wheel_angle: 0.0,
            brace_angle: 0.0,
        }
    }

    /// Advance the eased trim, then draw the deck and rig for this frame.
    pub fn render(
        &mut self,
        rig: &RigInput,
        dt: f32,
        t: f32,
        day: Daytime,
        storm: f32,
        w: f32,
        h: f32,
    ) {
        // Wheel chases the rudder; the yard hauls round toward the wind's bearing.
        self.wheel_angle += (rig.turn * 2.4 - self.wheel_angle) * clamp(WHEEL_EASE * dt, 0.0, 1.0);
        let target_brace = clamp(-rig.wind_rel, -BRACE_LIMIT, BRACE_LIMIT);
        self.brace_angle += (target_brace - self.brace_angle) * clamp(BRACE_EASE * dt, 0.0, 1.0);

        // Daylight knocks the whole ship down a touch at dusk/night and the storm
        // drains it toward slate, so the deck sits in the same light as the sea.
        let day_lit = match day {
            Daytime::Day => 1.0,
            Daytime::Dawn => 0.9,
            Daytime::Dusk => 0.82,
            Daytime::Night => 0.5,
        };
        let lit = day_lit * (1.0 - 0.35 * clamp(storm, 0.0, 1.0));

        // --- Rigid sway shared by deck + rig -----------------------------------
        let m = rig.motion;
        let roll = m.roll * DECK_SHARE;
        let (sr, cr) = roll.sin_cos();
        let pitch_px = pitch_response(m.pitch) * 90.0;
        let dx = m.yaw * YAW_SWAY_PX * DECK_SHARE;
        let dy = (pitch_px + m.heave * 6.0) * DECK_SHARE;
        // Pivot well below the screen so the tall mast arcs as the hull rolls.
        let pvx = w * 0.5;
        let pvy = h * 1.15;
        let sway = move |x: f32, y: f32| -> Vec2 {
            let (ox, oy) = (x - pvx, y - pvy);
            let rx = ox * cr - oy * sr;
            let ry = ox * sr + oy * cr;
            vec2(pvx + rx + dx, pvy + ry + dy)
        };

        self.draw_deck(&sway, lit, h, w);
        self.draw_rig(&sway, rig, lit, t, h, w);
    }

    /// Deck floor, bulwarks and the ship's wheel — the static woodwork the camera
    /// is bolted to. A planked perspective trapezoid that just sways with the hull.
    fn draw_deck(&self, sway: &impl Fn(f32, f32) -> Vec2, lit: f32, h: f32, w: f32) {
        let cx = w * 0.5;
        // Far (toward the bow) and near (under the helm) edges of the deck plank.
        let far_y = h * 0.70;
        let near_y = h * 1.08; // off the bottom edge so the deck fills the foreground
        let far_hw = w * 0.16;
        let near_hw = w * 0.62;

        let quad = |a: Vec2, b: Vec2, c: Vec2, d: Vec2, col: Color| {
            draw_triangle(a, b, c, col);
            draw_triangle(a, c, d, col);
        };

        // Planks: vertical strips running fore-aft, converging toward the bow, in
        // two alternating tones so the boards read.
        let planks = 9;
        for i in 0..planks {
            let u0 = i as f32 / planks as f32 * 2.0 - 1.0;
            let u1 = (i + 1) as f32 / planks as f32 * 2.0 - 1.0;
            let a = sway(cx + u0 * far_hw, far_y);
            let b = sway(cx + u1 * far_hw, far_y);
            let c = sway(cx + u1 * near_hw, near_y);
            let d = sway(cx + u0 * near_hw, near_y);
            let tone = if i % 2 == 0 { DECK_A } else { DECK_B };
            quad(a, b, c, d, rgba(tone, lit, 1.0));
        }

        // Bulwarks: a raised rail up each side, darker on the inboard face.
        let rail_h = h * 0.10;
        for side in [-1.0f32, 1.0] {
            let fb = sway(cx + side * far_hw, far_y);
            let nb = sway(cx + side * near_hw, near_y);
            let ft = sway(cx + side * far_hw, far_y - rail_h * 0.5);
            let nt = sway(cx + side * near_hw, near_y - rail_h);
            quad(fb, nb, nt, ft, rgba(RAIL, lit, 1.0));
            // A thin cap line on top of the rail.
            let fc = sway(cx + side * far_hw * 1.04, far_y - rail_h * 0.5);
            let nc = sway(cx + side * near_hw * 1.02, near_y - rail_h);
            quad(ft, nt, nc, fc, rgba(RAIL_DK, lit, 1.0));
        }

        self.draw_wheel(sway, lit, h, w);
    }

    /// The ship's wheel at the helm, spun toward the rudder. A spoked ring with a
    /// hub, standing proud of the deck at the bottom-centre.
    fn draw_wheel(&self, sway: &impl Fn(f32, f32) -> Vec2, lit: f32, h: f32, w: f32) {
        let cx = w * 0.5;
        let cy = h * 0.92;
        let r = h * 0.085;
        let a = self.wheel_angle;

        // Rim: a ring approximated by a fan of short trapezoids.
        let seg = 24;
        for i in 0..seg {
            let t0 = i as f32 / seg as f32 * TAU + a;
            let t1 = (i + 1) as f32 / seg as f32 * TAU + a;
            let inner = r * 0.78;
            let p0o = sway(cx + t0.cos() * r, cy + t0.sin() * r);
            let p1o = sway(cx + t1.cos() * r, cy + t1.sin() * r);
            let p1i = sway(cx + t1.cos() * inner, cy + t1.sin() * inner);
            let p0i = sway(cx + t0.cos() * inner, cy + t0.sin() * inner);
            draw_triangle(p0o, p1o, p1i, rgba(WHEEL_C, lit, 1.0));
            draw_triangle(p0o, p1i, p0i, rgba(WHEEL_C, lit, 1.0));
        }
        // Spokes radiating past the rim into handles.
        for k in 0..8 {
            let ta = k as f32 / 8.0 * TAU + a;
            let (s, c) = ta.sin_cos();
            let nx = -s; // perpendicular, for spoke thickness
            let ny = c;
            let hw = r * 0.06;
            let inner = 0.0;
            let outer = r * 1.18;
            let p0 = sway(cx + c * inner + nx * hw, cy + s * inner + ny * hw);
            let p1 = sway(cx + c * outer + nx * hw, cy + s * outer + ny * hw);
            let p2 = sway(cx + c * outer - nx * hw, cy + s * outer - ny * hw);
            let p3 = sway(cx + c * inner - nx * hw, cy + s * inner - ny * hw);
            draw_triangle(p0, p1, p2, rgba(WHEEL_DK, lit, 1.0));
            draw_triangle(p0, p2, p3, rgba(WHEEL_DK, lit, 1.0));
        }
        // Hub.
        let hub = sway(cx, cy);
        draw_circle(hub.x, hub.y, r * 0.22, rgba(WHEEL_C, lit, 1.0));
    }

    /// Mast, yard and the square sail — the articulating rig. The sail is built
    /// from overlapping vertical panels, each given an out-of-plane depth (belly +
    /// luff), then the whole yard rotated about the mast (the brace) before
    /// projecting through the fake perspective. Panels draw back-to-front so the
    /// curved surface overlaps correctly.
    fn draw_rig(
        &self,
        sway: &impl Fn(f32, f32) -> Vec2,
        rig: &RigInput,
        lit: f32,
        t: f32,
        h: f32,
        w: f32,
    ) {
        let cx = w * 0.5;
        let foot_y = h * 0.78; // mast steps into the deck here
        let mast_len = h * 0.82; // tall enough to tower off the top of the screen
        let yard_y = mast_len * 0.82; // yard crosses near the masthead
        let sail_w = w * 0.46;
        let sail_h = mast_len * 0.50;

        // Project a rig-local point (across x, up y, depth z toward viewer) to a
        // swayed screen point. The mast foot is the local origin on the deck.
        let project = |x: f32, y: f32, z: f32| -> Vec2 {
            let persp = FOCAL / (FOCAL - z);
            sway(cx + x * persp, foot_y - y * persp)
        };

        // --- Mast: a slightly tapered vertical post, two-tone for round form ----
        {
            let bw = w * 0.018; // base half-width
            let tw = w * 0.011; // taper to the masthead
            let b0 = project(-bw, 0.0, 0.0);
            let b1 = project(bw, 0.0, 0.0);
            let t1 = project(tw, mast_len, 0.0);
            let t0 = project(-tw, mast_len, 0.0);
            let mid0 = project(0.0, 0.0, 0.0);
            let mid1 = project(0.0, mast_len, 0.0);
            // Left half lit, right half shaded.
            draw_triangle(b0, mid0, mid1, rgba(SPAR, lit, 1.0));
            draw_triangle(b0, mid1, t0, rgba(SPAR, lit, 1.0));
            draw_triangle(mid0, b1, t1, rgba(SPAR_DK, lit, 1.0));
            draw_triangle(mid0, t1, mid1, rgba(SPAR_DK, lit, 1.0));
        }

        // --- Sail trim --------------------------------------------------------
        let draw_f = wind_factor_rel(rig.wind_rel); // wind harvested, 0..1 (same curve as the physics)
        let set = clamp(rig.set, 0.0, 1.0);
        let fill = draw_f * set; // belly amount
        let luff = (1.0 - draw_f).powi(3) * set; // flog amount
        let furl = set.max(0.05); // a struck sail keeps a thin rolled sliver
        let brace = self.brace_angle;
        let (sb, cb) = brace.sin_cos();

        let depth = -fill * BELLY_DEPTH * sail_w; // belly draft (px); negative = away
        let phase = t * FLAP_HZ * TAU;

        let sail_top = yard_y;
        let sail_bot = yard_y - sail_h * furl;

        // The out-of-plane offset of a panel edge at across-fraction `u` (-0.5..0.5).
        let panel_z = |u: f32| -> f32 {
            let belly = depth * (1.0 - (2.0 * u).powi(2)); // parabolic bulge
            let wave = (phase - u * FLAP_WAVES * TAU).sin();
            let flog = luff * FLAP_DEPTH * sail_w * wave * (0.3 + u.abs());
            belly + flog
        };
        // Rotate a panel edge (across x0, out-of-plane z0) about the mast (the brace).
        let braced = |u: f32| -> (f32, f32) {
            let x0 = u * sail_w;
            let z0 = panel_z(u);
            (x0 * cb + z0 * sb, -x0 * sb + z0 * cb)
        };

        // --- Yard: a spar along the braced across-axis at the sail's head -------
        {
            let (lx, lz) = braced(-0.54);
            let (rx, rz) = braced(0.54);
            let th = h * 0.012;
            let a = project(lx, sail_top + th, lz);
            let b = project(rx, sail_top + th, rz);
            let c = project(rx, sail_top - th, rz);
            let d = project(lx, sail_top - th, lz);
            draw_triangle(a, b, c, rgba(SPAR, lit, 1.0));
            draw_triangle(a, c, d, rgba(SPAR, lit, 1.0));
        }

        // --- Sail panels, drawn back-to-front by depth -------------------------
        let n = SAIL_PANELS;
        let half_strip = (PANEL_OVERLAP / n as f32) * 0.5; // overlapping half-width in u
        let mut order: Vec<usize> = (0..n).collect();
        let panel_u = |i: usize| (i as f32 + 0.5) / n as f32 - 0.5;
        order.sort_by(|&a, &b| {
            // Farthest (most negative z at the panel centre) first.
            let za = braced(panel_u(a)).1;
            let zb = braced(panel_u(b)).1;
            za.partial_cmp(&zb).unwrap()
        });

        for &i in &order {
            let u = panel_u(i);
            let ul = u - half_strip;
            let ur = u + half_strip;
            let (lx, lz) = braced(ul);
            let (rx, rz) = braced(ur);
            let tl = project(lx, sail_top, lz);
            let tr = project(rx, sail_top, rz);
            let br = project(rx, sail_bot, rz);
            let bl = project(lx, sail_bot, lz);
            // The belly catches the light amidships and falls to shade at the edges;
            // a panel braced edge-on (small horizontal span) also dims.
            let belly_lit = 1.0 - 0.28 * fill * (2.0 * u).powi(2);
            let face = ((tr.x - tl.x).abs() / (sail_w / n as f32 + 1.0)).min(1.0);
            let shade = (0.55 + 0.45 * face) * belly_lit;
            let col = rgba(SAIL_CLOTH, lit * shade, 1.0);
            draw_triangle(tl, tr, br, col);
            draw_triangle(tl, br, bl, col);
        }
    }
}
