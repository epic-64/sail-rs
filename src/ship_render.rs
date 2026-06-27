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
use crate::ocean::{deck_heave_px, pitch_response, ShipMotion, HEAVE_CAMERA_SHARE};
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
const SET_EASE: f32 = 2.2; // 1/s the crew haul the canvas to its new set (furl/unfurl)

// --- How the swell's sway is split (the deck takes the bulk) ------------------
const DECK_SHARE: f32 = 0.6;
const YAW_SWAY_PX: f32 = 180.0; // px of pan per rad of hull yaw

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
// Tarred standing rigging (shrouds + ratlines).
const ROPE: [f32; 3] = [74.0, 60.0, 44.0];
const WHEEL_C: [f32; 3] = [134.0, 98.0, 58.0];
const WHEEL_DK: [f32; 3] = [96.0, 68.0, 40.0];
// Deck cargo: lashed crates. Top catches the sky, the side faces fall to shade.
const CRATE_TOP: [f32; 3] = [182.0, 148.0, 96.0];
const CRATE_MID: [f32; 3] = [150.0, 116.0, 70.0];
const CRATE_DK: [f32; 3] = [108.0, 80.0, 46.0];

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
    /// The bow's lift above the hull's mean this frame (metres) — drives the deck's
    /// heave bob (`crate::ocean::deck_heave_px`).
    pub bow_lift: f32,
}

/// Holds the eased animation state (wheel spin, yard brace, canvas set) between frames.
pub struct ShipRenderer {
    wheel_angle: f32,
    brace_angle: f32,
    /// Visually-eased sail set, chasing the chosen notch so the canvas furls/unfurls
    /// smoothly instead of teleporting between None/Half/Full.
    set: f32,
}

#[inline]
fn rgba(c: [f32; 3], shade: f32, a: f32) -> Color {
    Color::new(c[0] / 255.0 * shade, c[1] / 255.0 * shade, c[2] / 255.0 * shade, a)
}

impl ShipRenderer {
    pub fn new() -> Self {
        ShipRenderer {
            wheel_angle: 0.0,
            brace_angle: 0.0,
            set: 0.0,
        }
    }

    /// Advance the eased trim, then draw the deck and rig for this frame.
    #[allow(clippy::too_many_arguments)] // per-frame rig + swell + camera inputs
    pub fn render(
        &mut self,
        rig: &RigInput,
        dt: f32,
        t: f32,
        // How lit the deck is by the sky right now (1 = full noon, ~0.5 deep night),
        // tracked continuously off the day/night clock so the ship dims with the sea.
        day_lit: f32,
        storm: f32,
        w: f32,
        h: f32,
    ) {
        // Wheel chases the rudder; the yard hauls round toward the wind's bearing;
        // the canvas furls/unfurls toward the chosen notch.
        self.wheel_angle += (rig.turn * 2.4 - self.wheel_angle) * clamp(WHEEL_EASE * dt, 0.0, 1.0);
        let target_brace = clamp(-rig.wind_rel, -BRACE_LIMIT, BRACE_LIMIT);
        self.brace_angle += (target_brace - self.brace_angle) * clamp(BRACE_EASE * dt, 0.0, 1.0);
        self.set += (clamp(rig.set, 0.0, 1.0) - self.set) * clamp(SET_EASE * dt, 0.0, 1.0);

        // The storm drains the deck toward slate, so it sits in the same light as the
        // sea, on top of the clock's daylight already folded into `day_lit`.
        let lit = day_lit * (1.0 - 0.35 * clamp(storm, 0.0, 1.0));

        // --- Rigid sway shared by deck + rig -----------------------------------
        let m = rig.motion;
        let roll = m.roll * DECK_SHARE;
        let (sr, cr) = roll.sin_cos();
        // Fore-aft nod (radians): the bow climbs gently and dives hard, shared down
        // by the deck-share. This drives a real *tilt* of the deck plane and the rig
        // (handled in draw_deck / draw_rig), not a mere vertical bob, so the ship
        // pitches through the swell. Heave stays as the only pure vertical slide.
        let pitch_ang = pitch_response(m.pitch) * DECK_SHARE;
        let dx = m.yaw * YAW_SWAY_PX * DECK_SHARE;
        // The deck's heave bob is the deck's share of the bow's lift above the mean
        // (the camera cranes the rest — see main.rs). This replaces the old flat
        // `heave · 6px`, which was far too little and read as the planks flying over
        // the sea. Bow-up (positive lift) → negative px → the deck rises.
        let dy = deck_heave_px(rig.bow_lift) * (1.0 - HEAVE_CAMERA_SHARE);
        // Pivot well below the screen so the tall mast arcs as the hull rolls.
        let pvx = w * 0.5;
        let pvy = h * 1.15;
        let sway = move |x: f32, y: f32| -> Vec2 {
            let (ox, oy) = (x - pvx, y - pvy);
            let rx = ox * cr - oy * sr;
            let ry = ox * sr + oy * cr;
            vec2(pvx + rx + dx, pvy + ry + dy)
        };

        self.draw_deck(&sway, pitch_ang, lit, h, w);
        self.draw_rig(&sway, rig, pitch_ang, lit, t, h, w);
    }

    /// Deck floor, bulwarks and the ship's wheel — the static woodwork the camera
    /// is bolted to. A planked perspective trapezoid that just sways with the hull.
    fn draw_deck(&self, sway: &impl Fn(f32, f32) -> Vec2, pitch_ang: f32, lit: f32, h: f32, w: f32) {
        let cx = w * 0.5;
        // Far (toward the bow) and near (under the helm) edges of the deck plank.
        // The fore-aft nod tilts the plane about mid-deck: bow-up lifts the far edge
        // and settles the helm, so the deck rocks fore-and-aft through the swell
        // rather than just sliding up and down.
        let nod = pitch_ang * h * 0.72;
        let far_y = h * 0.76 - nod; // dropped so the bow covers less of the horizon
        let near_y = h * 1.22 + nod * 0.3; // helm pulled well back, off the bottom edge
        let far_hw = w * 0.12; // narrow bow + wide helm → stronger foreshortening
        let near_hw = w * 0.72;
        // The stem: the bow pinches to a point forward of (above) the far edge so
        // the hull reads as a pointed prow, not a raft's flat front.
        let stem_y = far_y - h * 0.09;

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

        // Foredeck: the planking carries on past the far edge and pinches to the
        // stemhead, a fan of converging triangles forming the pointed bow.
        let stem = sway(cx, stem_y);
        let fore_planks = 8;
        for i in 0..fore_planks {
            let u0 = i as f32 / fore_planks as f32 * 2.0 - 1.0;
            let u1 = (i + 1) as f32 / fore_planks as f32 * 2.0 - 1.0;
            let a = sway(cx + u0 * far_hw, far_y);
            let b = sway(cx + u1 * far_hw, far_y);
            // Slightly darker than the main deck so the raked foredeck reads apart.
            let tone = if i % 2 == 0 { DECK_B } else { DECK_A };
            draw_triangle(a, b, stem, rgba(tone, 0.92 * lit, 1.0));
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

        // Bow rails: the topside sweeps in from each far corner to the raised
        // stemhead, closing the prow and giving it a little sheer.
        let stem_cap = sway(cx, stem_y - rail_h * 0.55);
        for side in [-1.0f32, 1.0] {
            let fb = sway(cx + side * far_hw, far_y);
            let ft = sway(cx + side * far_hw, far_y - rail_h * 0.5);
            // Outer (lit) face down to the waterline, then the capped top edge.
            quad(fb, ft, stem_cap, stem, rgba(RAIL, lit, 1.0));
            draw_triangle(ft, stem_cap, stem, rgba(RAIL_DK, lit, 1.0));
        }

        // Open railing: stanchions standing along each topside, joined by a cap
        // rail above the bulwark, so the deck reads as guarded rather than a bare
        // wall. The sheer runs the whole side, from the stemhead forward along the
        // foredeck and aft to the helm. Posts and cap grow with nearness, so the
        // rail towers over the viewer at the helm and shrinks to the bow (true
        // perspective). Built far → near so nearer posts overlap those behind.
        let posts = 8; // along the main deck side (far corner → helm)
        let fore_posts = 4; // up the foredeck (stem → far corner)
        let post_hw = w * 0.006;
        let cap_far = far_y - rail_h * 0.5;
        let cap_near = near_y - rail_h;
        let stem_cap_y = stem_y - rail_h * 0.55;
        // Depth of a deck y in the far(0)→near(1) span; negative past the bow.
        let depth = |y: f32| (y - far_y) / (near_y - far_y);
        // Stanchion height and half-width scale with depth, so the rail grows
        // toward the viewer and pinches to nothing at the bow.
        let post_h_at = |y: f32| rail_h * (0.35 + 0.7 * depth(y)).max(0.12);
        let post_hw_at = |y: f32| post_hw * (0.7 + 1.3 * depth(y)).max(0.4);
        for side in [-1.0f32, 1.0] {
            // The full sheer line, ordered bow → helm so the draw goes far → near.
            let mut pts: Vec<(f32, f32)> = Vec::new();
            // Foredeck: stem (converged on centreline) out to the far corner.
            for i in 0..fore_posts {
                let a = i as f32 / fore_posts as f32;
                let x = cx + side * far_hw * a;
                let y = stem_cap_y + (cap_far - stem_cap_y) * a;
                pts.push((x, y));
            }
            // Main deck: far corner aft to the helm, inclusive of both ends.
            for i in 0..=posts {
                let t = i as f32 / posts as f32;
                let hw = far_hw + (near_hw - far_hw) * t;
                let cap = cap_far + (cap_near - cap_far) * t;
                pts.push((cx + side * hw, cap));
            }
            // Cap rail: a thin board riding the tops of the stanchions, its
            // thickness tracking the post height so it foreshortens too.
            for w2 in pts.windows(2) {
                let (x0, y0) = w2[0];
                let (x1, y1) = w2[1];
                let (t0, t1) = (y0 - post_h_at(y0), y1 - post_h_at(y1));
                let (b0, b1) = (post_h_at(y0) * 0.22, post_h_at(y1) * 0.22);
                quad(
                    sway(x0, t0),
                    sway(x1, t1),
                    sway(x1, t1 + b1),
                    sway(x0, t0 + b0),
                    rgba(RAIL_DK, lit, 1.0),
                );
            }
            // Stanchions: vertical posts from the cap up to the rail.
            for &(px, py) in &pts {
                let ph = post_h_at(py);
                let pw = post_hw_at(py);
                quad(
                    sway(px - pw, py),
                    sway(px + pw, py),
                    sway(px + pw, py - ph),
                    sway(px - pw, py - ph),
                    rgba(RAIL, lit, 1.0),
                );
            }
        }

        // --- Deck cargo: a few lashed crates riding the deck -------------------
        // Positioned in deck coords (u across ±1, v fore→aft 0..1), drawn far →
        // near so nearer crates overlap those behind. Each is a flat-shaded box:
        // the two side faces and near face in shade, the lit top catching the sky.
        // The far face is hidden, so it is never drawn.
        let deck_pt = |u: f32, v: f32| -> (f32, f32) {
            let hw = far_hw + (near_hw - far_hw) * v;
            (cx + u * hw, far_y + (near_y - far_y) * v)
        };
        // (centre u, centre v, half-width u, half-depth v, height px, base lift px)
        let crates: [(f32, f32, f32, f32, f32, f32); 5] = [
            (-0.40, 0.38, 0.16, 0.060, h * 0.085, 0.0),
            (-0.38, 0.38, 0.13, 0.050, h * 0.070, h * 0.085), // stacked on the first
            (0.46, 0.44, 0.17, 0.070, h * 0.100, 0.0),
            (0.22, 0.27, 0.11, 0.045, h * 0.065, 0.0),
            (-0.58, 0.50, 0.18, 0.075, h * 0.110, 0.0),
        ];
        let mut idx: Vec<usize> = (0..crates.len()).collect();
        idx.sort_by(|&a, &b| {
            // Far (small v) first; a stacked crate (greater lift) over its base.
            (crates[a].1, crates[a].5)
                .partial_cmp(&(crates[b].1, crates[b].5))
                .unwrap()
        });
        for &k in &idx {
            let (cu, cv, hu, hv, ph, lift) = crates[k];
            let (flx, fly) = deck_pt(cu - hu, cv - hv); // far-left footprint
            let (frx, fry) = deck_pt(cu + hu, cv - hv); // far-right
            let (nrx, nry) = deck_pt(cu + hu, cv + hv); // near-right
            let (nlx, nly) = deck_pt(cu - hu, cv + hv); // near-left
            let base = |x: f32, y: f32| sway(x, y - lift);
            let top = |x: f32, y: f32| sway(x, y - lift - ph);
            let (bfl, bfr, bnr, bnl) = (base(flx, fly), base(frx, fry), base(nrx, nry), base(nlx, nly));
            let (tfl, tfr, tnr, tnl) = (top(flx, fly), top(frx, fry), top(nrx, nry), top(nlx, nly));
            quad(bnl, bfl, tfl, tnl, rgba(CRATE_DK, lit, 1.0)); // left side
            quad(bfr, bnr, tnr, tfr, rgba(CRATE_DK, lit, 1.0)); // right side
            quad(bnl, bnr, tnr, tnl, rgba(CRATE_MID, lit, 1.0)); // near face
            quad(tfl, tfr, tnr, tnl, rgba(CRATE_TOP, lit, 1.0)); // lit top
            // A batten across the near face so the box reads as planked.
            let nf = |f: f32| {
                (
                    sway(nlx, nly - lift - ph * f),
                    sway(nrx, nry - lift - ph * f),
                )
            };
            let (lo_l, lo_r) = nf(0.42);
            let (hi_l, hi_r) = nf(0.52);
            quad(lo_l, lo_r, hi_r, hi_l, rgba(CRATE_DK, lit, 1.0));
        }

        self.draw_wheel(sway, lit, h, w);
    }

    /// The ship's wheel at the helm, spun toward the rudder. A spoked ring with a
    /// hub, standing proud of the deck at the bottom-centre.
    fn draw_wheel(&self, sway: &impl Fn(f32, f32) -> Vec2, lit: f32, h: f32, w: f32) {
        let cx = w * 0.5;
        let cy = h * 1.0; // pulled back with the helm, half off the bottom edge
        let r = h * 0.095;
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
    #[allow(clippy::too_many_arguments)] // sway/projection inputs for the rig
    fn draw_rig(
        &self,
        sway: &impl Fn(f32, f32) -> Vec2,
        rig: &RigInput,
        pitch_ang: f32,
        lit: f32,
        t: f32,
        h: f32,
        w: f32,
    ) {
        let cx = w * 0.5;
        let foot_y = h * 0.82; // mast steps into the deck here (lowered with the deck)
        let mast_len = h * 0.82; // tall enough to tower off the top of the screen
        // The bare pole runs 3 m above the yard/sail rigging (the engine's metre
        // scale is ocean::HEAVE_GAIN_PX = 27 px/m). The shrouds make for this very
        // top; the yard and sail stay pinned to `mast_len` below.
        let mast_top = mast_len + 3.0 * 27.0;
        let yard_y = mast_len * 0.90; // yard crosses near the masthead
        let sail_w = w * 0.38;
        let sail_h = mast_len * 0.42;

        // The fore-aft nod tips the whole rig about its foot: bow-up rocks the
        // masthead aft (toward the helm/viewer), bow-down throws it forward.
        let (sp, cp) = pitch_ang.sin_cos();
        // Project a rig-local point (across x, up y, depth z toward viewer) to a
        // swayed screen point. The mast foot is the local origin on the deck; (y, z)
        // are first rotated by the pitch so the rig nods through the swell.
        let project = |x: f32, y: f32, z: f32| -> Vec2 {
            let py = y * cp - z * sp;
            let pz = y * sp + z * cp;
            let persp = FOCAL / (FOCAL - pz);
            sway(cx + x * persp, foot_y - py * persp)
        };

        // --- Sail trim --------------------------------------------------------
        let draw_f = wind_factor_rel(rig.wind_rel); // wind harvested, 0..1 (same curve as the physics)
        let set = self.set; // visually-eased set, so the canvas furls/unfurls smoothly
        let fill = draw_f * set; // belly amount
        let luff = (1.0 - draw_f).powi(3) * set; // flog amount
        let furl = set.max(0.05); // a struck sail keeps a thin rolled sliver
        let brace = self.brace_angle;
        let (sb, cb) = brace.sin_cos();

        // The cloth hangs a touch abaft the mast (away from the viewer) so the spar
        // always parts it, never pokes through — on top of that sits the belly.
        let stand_off = w * 0.022; // base depth of the sail behind the mast plane
        let depth = -fill * BELLY_DEPTH * sail_w; // belly draft (px); negative = away
        let phase = t * FLAP_HZ * TAU;

        let sail_top = yard_y;
        let sail_bot = yard_y - sail_h * furl;

        // The out-of-plane offset of a panel edge at across-fraction `u` (-0.5..0.5).
        let panel_z = |u: f32| -> f32 {
            let belly = depth * (1.0 - (2.0 * u).powi(2)); // parabolic bulge
            let wave = (phase - u * FLAP_WAVES * TAU).sin();
            let flog = luff * FLAP_DEPTH * sail_w * wave * (0.3 + u.abs());
            -stand_off + belly + flog
        };
        // Rotate a panel edge (across x0, out-of-plane z0) about the mast (the brace).
        let braced = |u: f32| -> (f32, f32) {
            let x0 = u * sail_w;
            let z0 = panel_z(u);
            (x0 * cb + z0 * sb, -x0 * sb + z0 * cb)
        };

        // --- Sail panels, drawn back-to-front by depth -------------------------
        // Drawn *before* the spars so the mast and yard (at the rig's z≈0 plane,
        // nearest the viewer) always part the cloth instead of the cloth painting
        // over them.
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

        // --- Yard: a spar along the braced across-axis at the sail's head -------
        // Drawn over the panels so it crosses ahead of the cloth it carries.
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

        // --- Mast: a slightly tapered vertical post, two-tone for round form ----
        // Drawn last, at z=0 (nearest), so it stands in front of the sail and yard.
        {
            let bw = w * 0.018; // base half-width
            let tw = w * 0.011; // taper to the masthead
            let b0 = project(-bw, 0.0, 0.0);
            let b1 = project(bw, 0.0, 0.0);
            let t1 = project(tw, mast_top, 0.0);
            let t0 = project(-tw, mast_top, 0.0);
            let mid0 = project(0.0, 0.0, 0.0);
            let mid1 = project(0.0, mast_top, 0.0);
            // Left half lit, right half shaded.
            draw_triangle(b0, mid0, mid1, rgba(SPAR, lit, 1.0));
            draw_triangle(b0, mid1, t0, rgba(SPAR, lit, 1.0));
            draw_triangle(mid0, b1, t1, rgba(SPAR_DK, lit, 1.0));
            draw_triangle(mid0, t1, mid1, rgba(SPAR_DK, lit, 1.0));
        }

        // --- Shrouds: tarred ropes fanning from the hounds (just below the
        // masthead) down to the railing on each side, braced with ratline rungs
        // so they read as the ladder the crew climbs. Drawn last, over the sail
        // and mast: standing rigging stands nearest the viewer. The tops run off
        // the top of the screen, so mostly only the lower fan is in view.
        {
            // Recompute the deck's side geometry (matches draw_deck) so the feet
            // sit on the railing as the hull nods.
            let nod = pitch_ang * h * 0.72;
            let far_y = h * 0.76 - nod;
            let near_y = h * 1.22 + nod * 0.3;
            let far_hw = w * 0.12;
            let near_hw = w * 0.72;
            let rail_h = h * 0.10;
            // A point atop the railing cap at fore-aft fraction v (0=bow, 1=helm).
            let rail_top = |side: f32, v: f32| -> Vec2 {
                let hw = far_hw + (near_hw - far_hw) * v;
                let cap_far = far_y - rail_h * 0.5;
                let cap_near = near_y - rail_h;
                let cap = cap_far + (cap_near - cap_far) * v;
                // Sit a touch above the cap, on the stanchion tops.
                sway(cx + side * hw, cap - rail_h * (0.35 + 0.7 * v) * 0.9)
            };
            let thick = (h * 0.0028).max(1.0);
            // Fore-aft positions the shrouds land on the rail, set well abaft the
            // mast (which steps in around v ≈ 0.13) so the fan stands nearer the
            // viewer.
            let feet_v = [0.16f32, 0.31, 0.46];
            for side in [-1.0f32, 1.0] {
                // The masthead: all shrouds on a side gather at the very top.
                let hounds = project(side * w * 0.011, mast_top, 0.0);
                let feet: Vec<Vec2> = feet_v.iter().map(|&v| rail_top(side, v)).collect();
                for &foot in &feet {
                    draw_line(hounds.x, hounds.y, foot.x, foot.y, thick, rgba(ROPE, lit, 1.0));
                }
                // Ratline rungs: short ropes lacing adjacent shrouds at a few
                // heights, spaced wider toward the deck (perspective). Each rung
                // meets its neighbouring shroud at the *same* screen height, so it
                // lies level (facing the horizon) rather than raking up the mast.
                for &f in &[0.5f32, 0.7, 0.88] {
                    for pair in feet.windows(2) {
                        let a = hounds.lerp(pair[0], f);
                        let span = pair[1].y - hounds.y;
                        let tb = if span.abs() > 0.01 { (a.y - hounds.y) / span } else { f };
                        let b = hounds.lerp(pair[1], tb.clamp(0.0, 1.0));
                        draw_line(a.x, a.y, b.x, b.y, thick * 0.7, rgba(ROPE, lit * 0.92, 1.0));
                    }
                }
            }
        }
    }
}
