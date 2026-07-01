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
//!
//! The whole assembly is lit by the scene's light ([`ShipLight`]): the coloured
//! key/ambient pair the islands and sea take, shaded per face against where the
//! sun or moon actually stands, so the deck reddens at dusk, cools under the
//! moon, drains to slate in a gale and flashes cold under lightning.

use macroquad::prelude::*;

use crate::geometry::clamp;
use crate::ocean::{deck_heave_px, pitch_response, ShipMotion, HEAVE_CAMERA_SHARE};
use crate::sailing::wind_factor_rel;

use std::f32::consts::TAU;

// --- Rig trim feel (ported from SailingView) ---------------------------------
const SAIL_PANELS: usize = 8; // vertical cloth panels the sail is built from
const BELLY_DEPTH: f32 = 0.37; // deepest draft, as a fraction of sail width
const FLAP_HZ: f32 = 1.6; // luff flutter rate
const FLAP_WAVES: f32 = 1.6; // ripple crests across the sail at once
const FLAP_DEPTH: f32 = 0.035; // deepest a flog throws a panel, fraction of width
const BRACE_LIMIT: f32 = 1.3; // hard brace (~75°) reached by a beam wind
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
// Tarred rigging (the forestay and the sheets working the sail).
const ROPE: [f32; 3] = [74.0, 60.0, 44.0];
// Kept darker than the deck planks so the rim reads against them.
const WHEEL_C: [f32; 3] = [104.0, 74.0, 44.0];
const WHEEL_DK: [f32; 3] = [76.0, 52.0, 30.0];
// Deck cargo: lashed crates. Top catches the sky, the side faces fall to shade.
const CRATE_TOP: [f32; 3] = [182.0, 148.0, 96.0];
const CRATE_MID: [f32; 3] = [150.0, 116.0, 70.0];
const CRATE_DK: [f32; 3] = [108.0, 80.0, 46.0];

/// The scene light the ship is shaded by this frame: the same coloured
/// key/ambient pair the islands and sea take (`OceanRenderer::scene_light`,
/// already storm-blended, so a gale drains the deck to slate with the water),
/// plus where the key light stands, so the woodwork shades directionally as the
/// sun arcs over and the ship turns beneath it.
pub struct ShipLight {
    /// Key light multiplier (sun by day, moon by night), brightness folded in.
    pub key: (f32, f32, f32),
    /// Ambient sky-fill multiplier, washing the shadowed faces.
    pub ambient: (f32, f32, f32),
    /// The key light's bearing relative to the bow (0 = dead ahead, + = starboard).
    pub light_rel: f32,
    /// Sine of the key light's altitude (0 on the horizon, 1 overhead).
    pub light_alt: f32,
    /// This frame's lightning glare, [0,1] (`clouds::StormSky::flash`): a brief
    /// cold flash thrown over the woodwork with the one lighting the sea.
    pub flash: f32,
}

/// Share of a face's shading that is ambient fill; the rest follows the key
/// light by the face's Lambert term. A touch above the islands' floor
/// (`islands_render::AMBIENT`) so the near woodwork stays readable on a
/// moonless deck.
const AMBIENT_SHARE: f32 = 0.5;
/// The cold blue-white a lightning strike throws over the deck, matched to the
/// sea's `LIGHTNING_COL`, and how strongly the flash relights the wood.
const FLASH_COL: [f32; 3] = [200.0, 216.0, 244.0];
const FLASH_GAIN: f32 = 0.5;

/// Per-frame shading context: the key light direction resolved into the rig's
/// local frame (+x starboard, +y up, +z aft toward the viewer) plus the coloured
/// key/ambient pair, applied Lambert-style per face. This is the ship's version
/// of the islands' `IslandView::lit`, so deck, land and sea all sit in one light.
struct Lume {
    key: (f32, f32, f32),
    ambient: (f32, f32, f32),
    l: (f32, f32, f32),
    flash: f32,
}

impl Lume {
    fn new(light: &ShipLight) -> Self {
        // Altitude arrives as its sine; the horizontal reach is the matching
        // cosine, split across the bearing relative to the bow. The camera looks
        // along the bow, so a light dead ahead shines *toward* the viewer (-z).
        let horiz = (1.0 - light.light_alt * light.light_alt).max(0.0).sqrt();
        Lume {
            key: light.key,
            ambient: light.ambient,
            l: (
                light.light_rel.sin() * horiz,
                light.light_alt.max(0.0),
                -light.light_rel.cos() * horiz,
            ),
            flash: clamp(light.flash, 0.0, 1.0),
        }
    }

    /// Lambert term of a face normal (rig frame, roughly unit length): 0 turned
    /// away from the key light, 1 face-on to it.
    fn diff(&self, n: (f32, f32, f32)) -> f32 {
        clamp(n.0 * self.l.0 + n.1 * self.l.1 + n.2 * self.l.2, 0.0, 1.0)
    }

    /// Shade a base colour by a diffuse term: ambient fill plus the key light by
    /// `diff`, a material multiplier `mul` on top, and the lightning flash's cold
    /// boost over everything (so a strike relights the rig against the dark).
    fn col(&self, base: [f32; 3], diff: f32, mul: f32) -> Color {
        let k = (1.0 - AMBIENT_SHARE) * diff;
        let f = self.flash * FLASH_GAIN;
        let ch = |b: f32, amb: f32, key: f32, fc: f32| {
            b / 255.0 * ((amb * AMBIENT_SHARE + key * k) * mul + fc / 255.0 * f)
        };
        Color::new(
            ch(base[0], self.ambient.0, self.key.0, FLASH_COL[0]),
            ch(base[1], self.ambient.1, self.key.1, FLASH_COL[1]),
            ch(base[2], self.ambient.2, self.key.2, FLASH_COL[2]),
            1.0,
        )
    }

    /// Shade a face directly by its normal.
    fn face(&self, base: [f32; 3], n: (f32, f32, f32)) -> Color {
        self.col(base, self.diff(n), 1.0)
    }
}

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
    /// Screen-space outline of the deck as drawn this frame (bow tip + both cap
    /// rails down off the bottom edge), recorded by `draw_deck`. The foreground
    /// rain queries it (`deck_covers`) so its sea rings hide behind the deck
    /// instead of painting over the planks. Empty while the deck isn't drawn
    /// (glancing astern), so it then covers nothing.
    deck_silhouette: Vec<Vec2>,
}

/// Even-odd ray cast: is point `p` inside the (possibly non-convex) polygon `poly`?
/// Used for the deck silhouette so the rain can tell sea behind the deck from sea
/// in the clear. A polygon of fewer than three points covers nothing.
fn point_in_poly(poly: &[Vec2], p: Vec2) -> bool {
    if poly.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[j]);
        if (a.y > p.y) != (b.y > p.y) {
            let cross = a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x);
            if p.x < cross {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}

impl ShipRenderer {
    pub fn new() -> Self {
        ShipRenderer {
            wheel_angle: 0.0,
            brace_angle: 0.0,
            set: 0.0,
            deck_silhouette: Vec::new(),
        }
    }

    /// True if screen point (`x`, `y`) lies under the deck as drawn this frame, so
    /// foreground rain rings behind the deck can be hidden by it. False when the
    /// deck wasn't drawn (empty silhouette), so nothing is occluded then.
    pub fn deck_covers(&self, x: f32, y: f32) -> bool {
        point_in_poly(&self.deck_silhouette, vec2(x, y))
    }

    /// Advance the eased trim, then draw the deck and rig for this frame.
    pub fn render(
        &mut self,
        rig: &RigInput,
        dt: f32,
        t: f32,
        // The hour's scene light (key + ambient, storm-blended) and where the key
        // stands, so the deck shades with the sea and islands: warm-white at noon,
        // blood-orange at dusk, dim cool-blue under the moon, slate in a gale.
        light: &ShipLight,
        w: f32,
        h: f32,
    ) {
        // Wheel chases the rudder; the yard hauls round toward the wind's bearing;
        // the canvas furls/unfurls toward the chosen notch.
        self.wheel_angle += (rig.turn * 2.4 - self.wheel_angle) * clamp(WHEEL_EASE * dt, 0.0, 1.0);
        let target_brace = clamp(-rig.wind_rel, -BRACE_LIMIT, BRACE_LIMIT);
        self.brace_angle += (target_brace - self.brace_angle) * clamp(BRACE_EASE * dt, 0.0, 1.0);
        self.set += (clamp(rig.set, 0.0, 1.0) - self.set) * clamp(SET_EASE * dt, 0.0, 1.0);

        let lume = Lume::new(light);

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

        self.deck_silhouette = self.draw_deck(&sway, pitch_ang, &lume, h, w);
        self.draw_rig(&sway, rig, pitch_ang, &lume, t, h, w);
    }

    /// Deck floor, bulwarks and the ship's wheel — the static woodwork the camera
    /// is bolted to. A planked perspective trapezoid that just sways with the hull.
    /// Draws the deck and returns its screen-space outer silhouette (bow tip, both
    /// cap rails, down past the bottom edge) for the rain's occlusion test.
    fn draw_deck(
        &self,
        sway: &impl Fn(f32, f32) -> Vec2,
        pitch_ang: f32,
        lume: &Lume,
        h: f32,
        w: f32,
    ) -> Vec<Vec2> {
        let cx = w * 0.5;
        // Face normals in the rig frame (+x starboard, +y up, +z aft): the deck
        // plane tips a little aft toward the eye; a bulwark's inboard face leans
        // in over the deck, so it takes the light from the *opposite* rail's side.
        let n_deck = (0.0, 0.94, 0.34);
        let n_wall = |side: f32| (-side * 0.72, 0.42, 0.55);
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
            quad(a, b, c, d, lume.face(tone, n_deck));
        }

        // Foredeck: the planking carries on past the far edge and pinches to the
        // stemhead, a fan of converging triangles forming the pointed bow. Same
        // plank count, tone parity and shade as the main deck, so every board
        // runs unbroken from the helm to the stem.
        let stem = sway(cx, stem_y);
        for i in 0..planks {
            let u0 = i as f32 / planks as f32 * 2.0 - 1.0;
            let u1 = (i + 1) as f32 / planks as f32 * 2.0 - 1.0;
            let a = sway(cx + u0 * far_hw, far_y);
            let b = sway(cx + u1 * far_hw, far_y);
            let tone = if i % 2 == 0 { DECK_A } else { DECK_B };
            draw_triangle(a, b, stem, lume.face(tone, n_deck));
        }

        // Bulwarks: one continuous planked wall up each side, running from the
        // stemhead along the foredeck and aft to the helm. Its height grows with
        // nearness the way the deck's width does, so the wall stands waist-high
        // amidships and towers by the helm. One surface with one shade, its
        // strakes and cap board carried the whole sheer, so nothing kinks where
        // the foredeck meets the main deck.
        let rail_h = h * 0.10;
        let wall_stem = rail_h * 0.55; // a touch of sheer rise at the stemhead
        let wall_far = rail_h * 0.45; // where the foredeck meets the main deck
        let wall_near = rail_h * 1.6; // ...towering by the helm
        for side in [-1.0f32, 1.0] {
            // Sheer stations bow → helm: (x, wall base y, wall height, cap flare).
            let sts = [
                (cx, stem_y, wall_stem, 1.0f32),
                (cx + side * far_hw, far_y, wall_far, 1.04),
                (cx + side * near_hw, near_y, wall_near, 1.02),
            ];
            for seg in sts.windows(2) {
                let (x0, y0, h0, f0) = seg[0];
                let (x1, y1, h1, f1) = seg[1];
                let b0 = sway(x0, y0);
                let b1 = sway(x1, y1);
                let t1 = sway(x1, y1 - h1);
                let t0 = sway(x0, y0 - h0);
                quad(b0, b1, t1, t0, lume.col(RAIL, lume.diff(n_wall(side)), 0.9));
                // Strakes: seam lines along the inboard face.
                for f in [0.34f32, 0.67] {
                    let s0 = sway(x0, y0 - h0 * f);
                    let s1 = sway(x1, y1 - h1 * f);
                    draw_line(
                        s0.x,
                        s0.y,
                        s1.x,
                        s1.y,
                        (h * 0.0022).max(1.0),
                        lume.col(RAIL_DK, lume.diff(n_wall(side)), 0.9),
                    );
                }
                // A thin cap board on top, flared a touch outboard.
                let c0 = sway(cx + (x0 - cx) * f0, y0 - h0);
                let c1 = sway(cx + (x1 - cx) * f1, y1 - h1);
                quad(t0, t1, c1, c0, lume.face(RAIL_DK, (0.0, 0.9, 0.44)));
            }
        }

        // Bowsprit: a tapered spar running out over the stem toward the horizon.
        // It anchors the forestay (see draw_rig) and closes the ship's profile so
        // the prow reads as a ship's, not a raft's. Two-tone halves for round form,
        // matching the mast.
        {
            let base_y = stem_y - rail_h * 0.25;
            let tip_y = stem_y - h * 0.115; // reaches out past the stemhead
            let bw = w * 0.0065; // half-width at the knightheads
            let tw = bw * 0.45; // taper to the tip
            let b0 = sway(cx - bw, base_y);
            let b1 = sway(cx + bw, base_y);
            let t1 = sway(cx + tw, tip_y);
            let t0 = sway(cx - tw, tip_y);
            let mb = sway(cx, base_y);
            let mt = sway(cx, tip_y);
            let lit_l = lume.face(SPAR, (-0.66, 0.4, 0.64));
            let lit_r = lume.face(SPAR_DK, (0.66, 0.4, 0.64));
            draw_triangle(b0, mb, mt, lit_l);
            draw_triangle(b0, mt, t0, lit_l);
            draw_triangle(mb, b1, t1, lit_r);
            draw_triangle(mb, t1, mt, lit_r);
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
        let cap_far = far_y - wall_far;
        let cap_near = near_y - wall_near;
        let stem_cap_y = stem_y - wall_stem;
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
                    lume.face(RAIL_DK, (0.0, 0.8, 0.6)),
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
                    lume.face(RAIL, (0.0, 0.25, 0.97)),
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
            quad(bnl, bfl, tfl, tnl, lume.face(CRATE_DK, (-0.92, 0.0, 0.4))); // left side
            quad(bfr, bnr, tnr, tfr, lume.face(CRATE_DK, (0.92, 0.0, 0.4))); // right side
            quad(bnl, bnr, tnr, tnl, lume.face(CRATE_MID, (0.0, 0.15, 0.99))); // near face
            quad(tfl, tfr, tnr, tnl, lume.face(CRATE_TOP, (0.0, 0.97, 0.26))); // lit top
            // A batten across the near face so the box reads as planked.
            let nf = |f: f32| {
                (
                    sway(nlx, nly - lift - ph * f),
                    sway(nrx, nry - lift - ph * f),
                )
            };
            let (lo_l, lo_r) = nf(0.42);
            let (hi_l, hi_r) = nf(0.52);
            quad(lo_l, lo_r, hi_r, hi_l, lume.face(CRATE_DK, (0.0, 0.15, 0.99)));
        }

        self.draw_wheel(sway, lume, h, w);

        // Outer silhouette for rain occlusion: the bow tip, then each cap rail's
        // top edge swept aft to the helm and down past the bottom of the screen, so
        // the polygon encloses every plank, bulwark and rail the deck paints. Built
        // through the same `sway`, so it tracks the deck's heel, pitch and bob.
        let cap_far_top = cap_far - post_h_at(cap_far);
        let cap_near_top = cap_near - post_h_at(cap_near);
        let out_far = far_hw * 1.04; // outer edge of the bulwark cap (matches above)
        let out_near = near_hw * 1.02;
        vec![
            sway(cx, stem_cap_y),            // bow tip (the deck's highest point)
            sway(cx - out_far, cap_far_top), // port: far cap top
            sway(cx - out_near, cap_near_top), // port: helm cap top
            sway(cx - out_near, near_y),     // port: helm foot (off the bottom edge)
            sway(cx + out_near, near_y),     // starboard: helm foot
            sway(cx + out_near, cap_near_top), // starboard: helm cap top
            sway(cx + out_far, cap_far_top), // starboard: far cap top
        ]
    }

    /// The ship's wheel at the helm, spun toward the rudder. A spoked ring with a
    /// hub, standing proud of the deck at the bottom-centre.
    fn draw_wheel(&self, sway: &impl Fn(f32, f32) -> Vec2, lume: &Lume, h: f32, w: f32) {
        let cx = w * 0.5;
        let cy = h * 0.99; // pulled back with the helm, half off the bottom edge
        let r = h * 0.12;
        let a = self.wheel_angle;
        // The wheel stands upright facing the helmsman (aft), so its whole face
        // shares one normal.
        let rim_col = lume.face(WHEEL_C, (0.0, 0.2, 0.98));
        let spoke_col = lume.face(WHEEL_DK, (0.0, 0.2, 0.98));

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
            draw_triangle(p0o, p1o, p1i, rim_col);
            draw_triangle(p0o, p1i, p0i, rim_col);
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
            draw_triangle(p0, p1, p2, spoke_col);
            draw_triangle(p0, p2, p3, spoke_col);
        }
        // Hub.
        let hub = sway(cx, cy);
        draw_circle(hub.x, hub.y, r * 0.22, rim_col);
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
        lume: &Lume,
        t: f32,
        h: f32,
        w: f32,
    ) {
        // Tarred rope is round and matte: no face to turn to the light, so it
        // takes a fixed half-diffuse of the hour's colour everywhere.
        let rope_col = lume.col(ROPE, 0.5, 1.0);
        let cx = w * 0.5;
        let foot_y = h * 0.82; // mast steps into the deck here (lowered with the deck)
        let mast_len = h * 0.82; // tall enough to tower off the top of the screen
        // The bare pole runs 3 m above the yard/sail rigging (the engine's metre
        // scale is ocean::HEAVE_GAIN_PX = 27 px/m). The shrouds make for this very
        // top; the yard and sail stay pinned to `mast_len` below.
        let mast_top = mast_len + 3.0 * 27.0;
        let yard_y = mast_len * 0.90; // yard crosses near the masthead
        let sail_w = w * 0.49;
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
        // Flog amount by point of sail: full flogging in irons, easing to its
        // weakest on the beam (the cleanest draw), then a lazy shiver building
        // again toward a dead run, where the following wind lets the cloth
        // slat. A small floor keeps the canvas alive at every angle.
        let x = (rig.wind_rel.abs() / std::f32::consts::PI).clamp(0.0, 1.0); // 0 = dead run, 1 = in irons
        let beam_dist = (2.0 * (x - 0.5)).abs(); // 0 on the beam, 1 at either extreme
        let flog = if x > 0.5 { 0.06 + 0.94 * beam_dist * beam_dist } else { 0.06 + 0.24 * beam_dist * beam_dist };
        let luff = flog * set;
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

        // The out-of-plane offset of a panel edge at across-fraction `u` (-0.5..0.5)
        // and down-fraction `v` (0 = the head, laced to the yard, 1 = the foot).
        // The belly and the flog both fade to nothing at the head, so the cloth
        // stays pinned along the yard and swings out toward the free foot.
        let panel_z = |u: f32, v: f32| -> f32 {
            let belly = depth * (1.0 - (2.0 * u).powi(2)) * v.powf(0.6); // parabolic bulge
            let wave = (phase - u * FLAP_WAVES * TAU).sin();
            let flog = luff * FLAP_DEPTH * sail_w * wave * (0.3 + u.abs()) * (0.25 + 0.75 * v);
            -stand_off + belly + flog
        };
        // Rotate a panel edge (across `u`, out-of-plane `z0`) about the mast (the brace).
        let braced = |u: f32, z0: f32| -> (f32, f32) {
            let x0 = u * sail_w;
            (x0 * cb + z0 * sb, -x0 * sb + z0 * cb)
        };

        // --- Forestay: the rope from the masthead down over the bow to the
        // bowsprit tip (matching draw_deck's spar). The one piece of standing
        // rigging forward of the canvas, so it draws *before* the sail and the
        // cloth hides its upper run; only the lower reach to the bow shows.
        {
            let nod = pitch_ang * h * 0.72;
            let far_y = h * 0.76 - nod;
            let stem_y = far_y - h * 0.09;
            let tip = sway(cx, stem_y - h * 0.115);
            let head = project(0.0, mast_top, 0.0);
            let thick = (h * 0.0028).max(1.0);
            draw_line(head.x, head.y, tip.x, tip.y, thick, rope_col);
        }

        // --- Sail panels, a continuous mesh drawn back-to-front by depth -------
        // Adjacent panels share their seam vertices exactly, so the cloth reads
        // as one watertight surface from any brace angle instead of overlapping
        // strips shingling at a slant. Drawn *before* the spars so the mast and
        // yard (at the rig's z≈0 plane, nearest the viewer) always part the
        // cloth instead of the cloth painting over them.
        let n = SAIL_PANELS;
        // The corner geometry at each of the n+1 seams: (head, foot), computed
        // once so both panels flanking a seam use the very same points.
        let seams: Vec<((f32, f32), (f32, f32))> = (0..=n)
            .map(|j| {
                let u = j as f32 / n as f32 - 0.5;
                (braced(u, panel_z(u, 0.0)), braced(u, panel_z(u, 1.0)))
            })
            .collect();
        let mut order: Vec<usize> = (0..n).collect();
        let panel_u = |i: usize| (i as f32 + 0.5) / n as f32 - 0.5;
        order.sort_by(|&a, &b| {
            // Farthest (most negative z at the panel's belly) first.
            let za = braced(panel_u(a), panel_z(panel_u(a), 0.7)).1;
            let zb = braced(panel_u(b), panel_z(panel_u(b), 0.7)).1;
            za.partial_cmp(&zb).unwrap()
        });

        for &i in &order {
            let u = panel_u(i);
            // Head corners ride the yard's plane; foot corners carry the full belly.
            let ((ltx, ltz), (lbx, lbz)) = seams[i];
            let ((rtx, rtz), (rbx, rbz)) = seams[i + 1];
            let tl = project(ltx, sail_top, ltz);
            let tr = project(rtx, sail_top, rtz);
            let br = project(rbx, sail_bot, rbz);
            let bl = project(lbx, sail_bot, lbz);
            // The belly catches the light amidships and falls to shade at the edges;
            // a panel braced edge-on (small horizontal span) also dims.
            let belly_lit = 1.0 - 0.28 * fill * (2.0 * u).powi(2);
            let face = ((tr.x - tl.x).abs() / (sail_w / n as f32 + 1.0)).min(1.0);
            let shade = (0.55 + 0.45 * face) * belly_lit;
            // Directional cloth: the braced plane's normal (the belly's edge
            // falloff is already in `belly_lit`) against the key light, with a
            // leak from the sky overhead. Canvas is translucent, so a back-lit
            // sail still glows through at three-quarters strength rather than
            // falling into flat shadow, and a small floor keeps a low sun from
            // blacking the cloth out.
            let toward = sb * lume.l.0 + cb * lume.l.2 + 0.35 * lume.l.1;
            let through = if toward >= 0.0 { toward } else { -toward * 0.75 };
            let cloth = 0.25 + 0.75 * through.min(1.0);
            let col = lume.col(SAIL_CLOTH, cloth, shade);
            draw_triangle(tl, tr, br, col);
            draw_triangle(tl, br, bl, col);
        }

        // --- Sheets: the two ropes working the sail, one from each clew (the
        // foot's free corners) hauled aft to the railing well astern. The clew
        // end rides the same brace/belly/luff transform as the cloth's own
        // panels, so the ropes lead wherever the sail swings and tremble with
        // it when it flogs. Braced hard on the wind a clew swings abaft the
        // mast plane; that side's rope must hide behind the spars, so each
        // rope draws before or after them by its clew's depth.
        let sheet_thick = (h * 0.0028).max(1.0);
        let sheets: Vec<(Vec<Vec2>, bool)> = {
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
                let cap_far = far_y - rail_h * 0.45;
                let cap_near = near_y - rail_h * 1.6;
                let cap = cap_far + (cap_near - cap_far) * v;
                // Sit a touch above the cap, on the stanchion tops.
                sway(cx + side * hw, cap - rail_h * (0.35 + 0.7 * v) * 0.9)
            };
            let sag = h * 0.035; // the rope's own weight bows the run a little
            let segs = 8;
            [-1.0f32, 1.0]
                .iter()
                .map(|&side| {
                    // The clew: the sail mesh's outermost seam at the foot.
                    let u = side * 0.5;
                    let (kx, kz) = braced(u, panel_z(u, 1.0));
                    let clew = project(kx, sail_bot, kz);
                    let foot = rail_top(side, 0.74); // belayed well astern
                    let pts: Vec<Vec2> = (0..=segs)
                        .map(|i| {
                            let t = i as f32 / segs as f32;
                            let mut p = clew.lerp(foot, t);
                            p.y += sag * (t * std::f32::consts::PI).sin();
                            p
                        })
                        .collect();
                    (pts, kz < 0.0)
                })
                .collect()
        };
        let draw_sheet = |pts: &[Vec2]| {
            for w2 in pts.windows(2) {
                draw_line(w2[0].x, w2[0].y, w2[1].x, w2[1].y, sheet_thick, rope_col);
            }
        };
        // The rope(s) whose clew lies abaft the mast plane, hidden by the spars.
        for (pts, behind) in &sheets {
            if *behind {
                draw_sheet(pts);
            }
        }

        // --- Yard: a spar along the braced across-axis at the sail's head -------
        // Drawn over the panels so it crosses ahead of the cloth it carries.
        {
            let (lx, lz) = braced(-0.54, -stand_off);
            let (rx, rz) = braced(0.54, -stand_off);
            let th = h * 0.012;
            let a = project(lx, sail_top + th, lz);
            let b = project(rx, sail_top + th, rz);
            let c = project(rx, sail_top - th, rz);
            let d = project(lx, sail_top - th, lz);
            // The yard swings with the brace, so its lit face follows the sail's
            // plane (plus a touch of sky from above).
            let yard_col = lume.face(SPAR, (sb * 0.95, 0.3, cb * 0.95));
            draw_triangle(a, b, c, yard_col);
            draw_triangle(a, c, d, yard_col);
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
            // Two-tone halves for round form, each half turned to its own side so
            // the shading flips as the light crosses the bow.
            let lit_l = lume.face(SPAR, (-0.7, 0.15, 0.7));
            let lit_r = lume.face(SPAR_DK, (0.7, 0.15, 0.7));
            draw_triangle(b0, mid0, mid1, lit_l);
            draw_triangle(b0, mid1, t0, lit_l);
            draw_triangle(mid0, b1, t1, lit_r);
            draw_triangle(mid0, t1, mid1, lit_r);
        }

        // The remaining sheet(s), their clews riding forward of the mast plane,
        // drawn nearest so the rope leads over the spars toward the rail.
        for (pts, behind) in &sheets {
            if !*behind {
                draw_sheet(pts);
            }
        }
    }
}
