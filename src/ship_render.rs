//! The player's own ship in the foreground: hull, wheel, mast, yard and a square
//! sail that braces, bellies and luffs. Flat-shaded low-poly geometry to match
//! the waves and islands — *not* the original's painted `deck*.png` bolted to
//! the camera with CSS `perspective()`/`rotateX` transforms.
//!
//! The hull is a real loft: stations in metres ([`STATIONS`]) projected through
//! one perspective camera stood on the quarterdeck abaft the wheel
//! ([`helm_cam`]), so deck, bulwarks, rails and wheel all foreshorten
//! consistently. The whole assembly sways as a rigid body with the swell
//! (heave/pitch/roll/yaw from [`crate::ocean::ship_motion`]), about a pivot
//! below the screen so the masthead arcs as the hull rolls. On top of that
//! rigid sway the rig *articulates*:
//!
//! - the **yard** braces about the mast's vertical axis to trim to the wind,
//! - the **sail** bows into a belly out of plane, laced flat along the yard and
//!   deepest toward the free foot, so the curve runs down the cloth,
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
const SAIL_PANELS: usize = 8; // cloth panels across the sail's width
const SAIL_ROWS: usize = 6; // rows down its height (they resolve the vertical belly)
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

// --- The hull in 3-D ------------------------------------------------------------
// The hull is a real low-poly loft in metres, in the rig frame: +x starboard,
// +y up, +z aft toward the eye, origin on the waist deck under the mast. One
// perspective camera projects it all (see `helm_cam`), so the woodwork
// foreshortens consistently: the eye stands on the quarterdeck a stretch abaft
// the wheel. The rig (mast, yard, sail) keeps its own gentler anchored
// perspective — an honest camera would crop the sail out of a fixed forward
// view — pinned to the hull at the projected mast foot.
const CAM_AFT: f32 = 10.0; // the eye: metres abaft the mast
// Eye height above the waist deck: a helmsman's eye line (~1.65 m) stood on the
// quarterdeck (see the raised stations). Raising this reads as a taller viewer.
const CAM_UP: f32 = 2.45;
const CAM_F: f32 = 0.58; // focal length, ×w (~80° horizontal field of view)
const CAM_NEAR: f32 = 0.8; // metres; geometry nearer the eye than this is dropped
/// The horizon row the camera is levelled on, ×h (`ocean_renderer` draws the
/// sea's horizon at this same row).
const HORIZON: f32 = 0.54;

/// The hull's lofting stations, bow → stern: (z aft of the mast, half-beam,
/// deck height, bulwark height), all metres. This table *is* the ship's shape —
/// the sheer that dips amidships and climbs to the stemhead, the beam swelling
/// to its fullest at the mast, the raised quarterdeck aft (the doubled station
/// is its riser) — and everything drawn is lofted from it, so reshaping the
/// ship means editing numbers here and nowhere else. The transom lies behind
/// the eye (`CAM_AFT`), which is what keeps the woodwork running off-screen
/// through any sway: there is simply more ship back there.
const STATIONS: [(f32, f32, f32, f32); 13] = [
    (-15.0, 0.05, 1.55, 0.50), // stem tip
    (-13.5, 0.95, 1.22, 0.72),
    (-11.5, 1.95, 0.88, 0.70),
    (-9.0, 2.65, 0.55, 0.68),
    (-6.0, 3.15, 0.26, 0.66),
    (-3.0, 3.40, 0.10, 0.65),
    (0.0, 3.50, 0.02, 0.65), // the mast station: full beam
    (3.0, 3.45, 0.00, 0.66),
    (4.0, 3.40, 0.005, 0.68), // the sheer starts its climb to the quarterdeck...
    (QDECK_BREAK, 3.36, 0.01, 1.49), // ...topping out level with the platform's wall
    (QDECK_BREAK, 3.36, 0.82, 0.68), // quarterdeck side of the break (the riser)
    (9.0, 3.05, 0.88, 0.74),
    (11.0, 2.72, 0.92, 0.80), // transom, behind the eye
];

/// Where the deck steps up onto the helm's raised platform (the doubled
/// station above). The deck loft splits into waist and quarterdeck phases
/// here, with the companion stairs drawn between them.
const QDECK_BREAK: f32 = 5.0;

/// The breast rail across the quarterdeck's forward edge: where it stands and
/// how high its top rail rides off the platform. Shared by the rail itself and
/// the deck chart clipped to it (see [`DeckChart`]).
const BREAST_RAIL_Z: f32 = QDECK_BREAK + 0.1;
const BREAST_RAIL_H: f32 = 0.85;

/// Hull data (half-beam, deck height, bulwark height) interpolated at fore-aft
/// z, for placing furniture and rope feet between stations.
fn station_at(z: f32) -> (f32, f32, f32) {
    for pair in STATIONS.windows(2) {
        let (z0, b0, d0, w0) = pair[0];
        let (z1, b1, d1, w1) = pair[1];
        if z >= z0 && z <= z1 && z1 > z0 {
            let t = (z - z0) / (z1 - z0);
            return (b0 + (b1 - b0) * t, d0 + (d1 - d0) * t, w0 + (w1 - w0) * t);
        }
    }
    let (_, b, d, wh) = STATIONS[STATIONS.len() - 1];
    (b, d, wh)
}

/// A tiny deterministic hash to [0,1): per-slot jitter for the deck cargo's
/// sizes, offsets and stacking rolls. Render-side only; the world's seeded
/// RNG is never touched, and the same seed gives the same crate every frame.
fn slot_rand(seed: u32) -> f32 {
    let mut x = seed.wrapping_mul(0x9E37_79B9);
    x ^= x >> 16;
    x = x.wrapping_mul(0x85EB_CA6B);
    x ^= x >> 13;
    (x >> 8) as f32 / 16_777_216.0
}

/// Project a hull point (metres) through the helm camera: the fore-aft nod is a
/// true rotation about the mast foot (`sp`/`cp` its sin/cos, shared with the
/// rig), then a perspective divide from the eye. Returns the unswayed screen
/// point and the px-per-metre scale at that depth, or None inside the near
/// plane — such a point is below or behind the eye, always off-screen, so a
/// caller dropping its face never leaves a visible hole.
fn helm_cam(x: f32, y: f32, z: f32, sp: f32, cp: f32, w: f32, h: f32) -> Option<(Vec2, f32)> {
    let py = y * cp - z * sp;
    let pz = y * sp + z * cp;
    let d = CAM_AFT - pz;
    if d < CAM_NEAR {
        return None;
    }
    let s = w * CAM_F / d;
    Some((vec2(w * 0.5 + x * s, h * HORIZON + (CAM_UP - py) * s), s))
}

/// Screen anchors the rest of the frame hangs on, computed by `draw_deck` as it
/// projects the hull.
struct DeckPoints {
    /// Outer silhouette for the rain's occlusion test (see `deck_silhouette`).
    silhouette: Vec<Vec2>,
    /// Rope feet atop the rails, [port, starboard]: the sheets belay on the
    /// waist rail, the braces on the quarterdeck rail beside the helm.
    sheet_foot: [Vec2; 2],
    brace_foot: [Vec2; 2],
    /// The bowsprit's tip, where the forestay lands.
    bowsprit_tip: Vec2,
}

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
// Rigging: weathered hemp, light enough not to read as black lines on the sky.
const ROPE: [f32; 3] = [118.0, 98.0, 72.0];
// Kept darker than the deck planks so the rim reads against them.
const WHEEL_C: [f32; 3] = [104.0, 74.0, 44.0];
const WHEEL_DK: [f32; 3] = [76.0, 52.0, 30.0];
// Deck cargo: lashed crates. Top catches the sky, the side faces fall to shade.
const CRATE_TOP: [f32; 3] = [182.0, 148.0, 96.0];
const CRATE_MID: [f32; 3] = [150.0, 116.0, 70.0];
const CRATE_DK: [f32; 3] = [108.0, 80.0, 46.0];
// The deck chart: weathered parchment and its ink, kept close to the sail
// cloth's warmth so the little board sits in the same palette.
const CHART_PARCH: [f32; 3] = [216.0, 198.0, 158.0];
const CHART_INK: [f32; 3] = [66.0, 50.0, 34.0];
const CHART_PIN: [f32; 3] = [158.0, 42.0, 32.0];

// --- Deck cargo physics --------------------------------------------------------
// The crates are heavy and lashed: ordinary sailing never beats their static
// grip and the pile stands like furniture. What breaks them loose is the
// extreme stuff: a storm's deck angles, a frontal slam that floats weight off
// the planks, or the wheel hauled hard over at speed (the turn's centrifugal
// throw). A loose crate grinds (kinetic friction), slews as it goes, bangs off
// the bulwarks, the mast, the stairs and its neighbours; a stacked crate holds
// by less than a lashed one, so it lets go first, and one that slides past its
// base's edge tips off and tumbles to the deck. All of it is visual-only, in
// the ship frame the loft draws in; the world simulation never feels it.
const CRATE_G: f32 = 9.81;
const CRATE_MU_S: f32 = 0.34; // static grip: tan of the felt deck angle that breaks the lashings
const CRATE_MU_K: f32 = 0.24; // kinetic: a loose crate grinds to a stop, it doesn't glide
const TOP_GRIP: f32 = 0.7; // a stack top holds by this fraction of a lashed base's grip
// The drawn deck angles are kept gentle for the camera's comfort (`DECK_SHARE`
// of an already-shallow wave slope), far below what the same sea would do to a
// real deck. The cargo *feels* the sea, not the camera: its tilt is amplified
// by `CARGO_TILT`, and the turn's centrifugal throw by `CENTRI_GAIN` (standing
// in for the hard-over heel the visual deck doesn't take). Calibrated against
// measured motion (see the `storm_calibration` test): a full storm's worst
// rolls and slams must beat the grip, a working breeze must not come close.
const CARGO_TILT: f32 = 2.5;
const CENTRI_GAIN: f32 = 1.5;
const SLAM_KICK: f32 = 14.0; // m/s² of forward jolt a full frontal slam throws through the deck
const SLAM_LIGHTEN: f32 = 0.55; // share of weight a full slam floats off the crates
const CRATE_BOUNCE: f32 = 0.25; // speed kept (reversed) banging off a wall
const CRATE_SPIN: f32 = 0.6; // yaw per metre slid, scaled by each crate's signed jitter
const CRATE_STOP: f32 = 0.06; // m/s under which a sliding crate settles
const RESTOW_RATE: f32 = 0.04; // 1/s the crew ease shifted cargo back to stowage in a calm
const WALL_GAP: f32 = 0.12; // clearance kept off the bulwark's inboard face
const MAST_HW: f32 = 0.25; // the mast foot's half-extent crates shove against
// A crate slammed into the bulwark hard enough carries clean over the rail and
// is lost to the sea: the toll for keeping way on through a storm or hauling
// the wheel over at full speed. The impact speed needed scales with each
// crate's grip, and a cooldown spaces the losses out (a reckless helm bleeds
// cargo crate by crate rather than dumping the whole deck in one roll).
const OVERBOARD_SPEED: f32 = 2.8; // m/s into the wall that carries the rail, ×grip
const OVERBOARD_POP: f32 = 2.2; // m/s upward as the crate tips over the cap rail
const OVERBOARD_COOLDOWN: f32 = 2.0; // s between crates going over
const SINK_Y: f32 = -1.6; // m below the deck at which a floater is struck from the books

/// One crate of deck cargo: its stowage slot, its live pose and motion, and the
/// per-crate jitter that keeps the pile from letting go all at once. Positions
/// are ship-frame metres (+x starboard, +z aft), `y` the height of its bottom.
struct DeckCrate {
    hw: f32,
    hd: f32,
    ht: f32,
    /// Static-friction jitter: how well this crate's lashings hold.
    grip: f32,
    /// Signed yaw tendency while sliding (which way it slews).
    spin: f32,
    /// The stowage slot the crew re-stow it toward in a calm.
    home: (f32, f32),
    x: f32,
    z: f32,
    y: f32,
    yaw: f32,
    vx: f32,
    vz: f32,
    vy: f32,
    /// The crate this one is stacked on, if any (always a lower index, so a
    /// base is stepped before the crates riding it).
    base: Option<usize>,
    /// Sliding rather than moving with the deck (or with its base).
    loose: bool,
    /// Mid-air, tumbling off a stack.
    fall: bool,
    /// Carried over the rail: ballistic outside the hull, fenced by nothing,
    /// bound for the sea.
    over: bool,
    /// Sunk from view: struck from the books, skipped by physics and drawing
    /// until the next re-stow rebuilds the pile.
    gone: bool,
}

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
/// How far to drain the chill from the ambient sky-fill *for the ship only*. The
/// woodwork is warm brown, so a clear midday sky's blue ambient (half of every
/// face's light, see `AMBIENT_SHARE`) tints the planks a cold green the islands'
/// greens never show. This eases the fill back toward neutral grey, scaled by how
/// cool it actually is, so a blue daytime sky stops chilling the deck while a warm
/// dusk fill is left alone (and the *key* light still carries the hour's colour).
const AMBIENT_WARMTH: f32 = 0.8;
/// The cold blue-white a lightning strike throws over the deck, matched to the
/// sea's `LIGHTNING_COL`, and how strongly the flash relights the wood.
const FLASH_COL: [f32; 3] = [200.0, 216.0, 244.0];
const FLASH_GAIN: f32 = 0.5;

/// Ease an ambient fill toward neutral grey in proportion to how cool it is
/// (blue over red), preserving its overall brightness. A warm fill (red ≥ blue)
/// passes through untouched; a cold blue fill loses `AMBIENT_WARMTH` of its
/// chroma, so daylight stops washing the warm deck cold.
fn warm_ambient(a: (f32, f32, f32)) -> (f32, f32, f32) {
    let grey = (a.0 + a.1 + a.2) / 3.0;
    let cool = ((a.2 - a.0) / grey.max(1e-3)).clamp(0.0, 1.0);
    let k = AMBIENT_WARMTH * cool;
    let mix = |c: f32| c + (grey - c) * k;
    (mix(a.0), mix(a.1), mix(a.2))
}

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
            ambient: warm_ambient(light.ambient),
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

/// The world chart pinned to the breast rail beside the wheel, once the
/// captain owns the World Map ware: a keepsake miniature of the whole world,
/// readable from the helm without opening the captain's log. Positions are
/// normalized chart space, [0,1] on each axis (u east, v north).
pub struct DeckChart<'a> {
    /// Every isle's plot: (u, v, is_port).
    pub isles: &'a [(f32, f32, bool)],
    /// The ship's own plot in the same space: the "you are here" pin.
    pub ship: (f32, f32),
}

/// Per-frame trim the rig is steered by. `wind_rel` is the prevailing wind's
/// bearing relative to the bow (0 = wind from dead astern, ±π = dead ahead).
pub struct RigInput<'a> {
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
    /// Units in the hold (sellable cargo plus reserved mission goods, i.e.
    /// `hold_used`): the waist shows one lashed crate per unit.
    pub cargo: i32,
    /// Speed through the water (m/s) and the hull's yaw rate (rad/s): their
    /// product is the turn's centrifugal throw on the deck cargo.
    pub speed: f32,
    pub yaw_rate: f32,
    /// Frontal slam this frame, [0,1] (the spray's input): a plunge jolts the
    /// cargo forward and floats weight off the planks for a moment.
    pub slam: f32,
    /// The world chart pinned by the wheel, `None` until the captain owns the
    /// World Map ware (the board simply isn't aboard yet).
    pub chart: Option<DeckChart<'a>>,
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
    /// The deck cargo's live state, stepped by `step_cargo` each frame and
    /// rebuilt (re-stowed) whenever the hold's unit count changes.
    crates: Vec<DeckCrate>,
    /// The hold count the live crates represent, to notice an outside change
    /// (a trade at port re-stows the deck). Kept in step with overboard losses
    /// by `cargo_washed_overboard`, so a loss the game has been told about
    /// does *not* re-stow the pile mid-storm.
    stowed: i32,
    /// Crates lost over the rail since the game last collected them
    /// (see `cargo_washed_overboard`).
    washed: i32,
    /// Seconds before the sea may take another crate (see OVERBOARD_COOLDOWN).
    over_cooldown: f32,
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
            crates: Vec::new(),
            stowed: -1,
            washed: 0,
            over_cooldown: 0.0,
        }
    }

    /// Crates lost over the rail since the last call: the game applies them to
    /// the hold (see `GameState::lose_cargo`). Collecting them also squares
    /// `stowed`, so the already-reported loss doesn't read as an outside cargo
    /// change and re-stow the deck mid-storm.
    pub fn cargo_washed_overboard(&mut self) -> i32 {
        let n = self.washed;
        self.washed = 0;
        self.stowed -= n;
        n
    }

    /// Lay out the deck cargo afresh for `target` hold units: one crate per
    /// unit. Slots fill from the quarterdeck break forward toward the bow (the
    /// slots nearest the helm are stowed first); sizes and offsets jitter by a
    /// per-slot hash (`slot_rand`), so the pile reads as stowed by hand yet
    /// lays out identically for the same count. A first pass lays one crate
    /// per slot with some rolling a second stacked on top straight away; a
    /// second pass returns aft to top up the slots left single, so a full hold
    /// buries the waist two crates deep. The sequence is fixed and only ever
    /// *extended* by more cargo: crates already stowed keep their slots.
    fn rebuild_crates(&mut self, target: usize) {
        self.crates.clear();
        // Fill-order slots: (hash key, centre x, centre z). Rows march from
        // just short of the break toward the bow; the columns are shuffled
        // per row so the pile doesn't fill in tidy stripes.
        let cols = [-2.4f32, -1.2, 0.0, 1.2]; // clear of the stairs at x 2.0
        let mut slots: Vec<(u32, f32, f32)> = Vec::new();
        let mut zrow = 4.2f32;
        let mut row = 0u32;
        while zrow > -6.0 {
            let spin = (slot_rand(row.wrapping_mul(31).wrapping_add(7)) * cols.len() as f32)
                as usize;
            for c in 0..cols.len() {
                let k = row * cols.len() as u32 + c as u32;
                let j = |n: u32| slot_rand(k.wrapping_mul(97).wrapping_add(n));
                let x = cols[(c + spin) % cols.len()] + 0.20 * (j(0) - 0.5);
                let z = zrow + 0.20 * (j(1) - 0.5);
                // The mast wants clear deck around its foot.
                if x.abs() < 0.7 && z.abs() < 1.1 {
                    continue;
                }
                slots.push((k, x, z));
            }
            zrow -= 1.1;
            row += 1;
        }
        let crate_at = |k: u32, x: f32, z: f32, hw: f32, hd: f32, ht: f32, y: f32| {
            let j = |n: u32| slot_rand(k.wrapping_mul(97).wrapping_add(n));
            DeckCrate {
                hw,
                hd,
                ht,
                grip: 0.85 + 0.3 * j(10),
                spin: 2.0 * (j(11) - 0.5),
                home: (x, z),
                x,
                z,
                y,
                yaw: 0.0,
                vx: 0.0,
                vz: 0.0,
                vy: 0.0,
                base: None,
                loose: false,
                fall: false,
                over: false,
                gone: false,
            }
        };
        // The pass-0 crate of each slot, for pass 1 to stack on.
        let mut base_of: Vec<usize> = Vec::with_capacity(slots.len());
        for pass in 0..2 {
            for (si, &(k, x, z)) in slots.iter().enumerate() {
                if self.crates.len() >= target {
                    break;
                }
                let j = |n: u32| slot_rand(k.wrapping_mul(97).wrapping_add(n));
                let (hw, hd) = (0.40 + 0.16 * j(2), 0.35 + 0.15 * j(3));
                let ht = 0.55 + 0.40 * j(4);
                let deck = station_at(z).1; // the deck rises toward the bow
                let stacked_early = j(5) < 0.38;
                let top = crate_at(
                    k.wrapping_mul(5).wrapping_add(3),
                    x + 0.12 * (j(6) - 0.5),
                    // Jittered only *toward* the eye (never aft of its base),
                    // so the far-to-near sort always draws it after.
                    z + 0.06 * j(7),
                    hw * (0.60 + 0.30 * j(8)),
                    hd * 0.8,
                    0.45 + 0.25 * j(9),
                    deck + ht,
                );
                if pass == 0 {
                    base_of.push(self.crates.len());
                    self.crates.push(crate_at(k, x, z, hw, hd, ht, deck));
                    if stacked_early && self.crates.len() < target {
                        self.crates.push(DeckCrate { base: Some(base_of[si]), ..top });
                    }
                } else if !stacked_early {
                    self.crates.push(DeckCrate { base: Some(base_of[si]), ..top });
                }
            }
        }
    }

    /// Step the deck cargo's physics for this frame. `roll` / `pitch_ang` are
    /// the *deck's* tilt (the share of the swell it takes, wind heel folded
    /// in), so the crates feel exactly the slopes the planks are drawn at.
    /// Gravity down those slopes, the turn's centrifugal throw and a frontal
    /// slam's jolt make an acceleration field; a crate holds fast until the
    /// field beats its static grip, slides under kinetic friction once loose,
    /// and is fenced by the bulwarks, the quarterdeck riser, the stairs, the
    /// mast and its neighbours. Stack tops grip less, ride their base while
    /// they hold, and tumble off its edge when they don't.
    fn step_cargo(&mut self, rig: &RigInput, roll: f32, pitch_ang: f32, dt: f32) {
        if self.stowed != rig.cargo {
            self.rebuild_crates(rig.cargo.max(0) as usize);
            self.stowed = rig.cargo;
        }
        let n = self.crates.len();
        if n == 0 {
            return;
        }
        let dt = dt.clamp(0.0, 0.05); // a hitch must not explode the pile
        if dt <= 0.0 {
            return;
        }
        let inv_dt = 1.0 / dt;
        // Where the crates stand: the slam floats weight off the planks, so
        // everything holds by less just as the jolt arrives.
        let g_eff = CRATE_G * (1.0 - SLAM_LIGHTEN * clamp(rig.slam, 0.0, 1.0)).max(0.2);
        // The deck-plane acceleration field (+x starboard, +z aft): gravity
        // down the tilted planks (amplified past the camera-gentle drawn tilt,
        // see CARGO_TILT), the turn's centrifugal throw (a starboard turn
        // slings cargo to port), and the slam's forward jolt.
        let ax = CRATE_G * (roll * CARGO_TILT).sin()
            - rig.speed * rig.yaw_rate * CENTRI_GAIN;
        let az = CRATE_G * (pitch_ang * CARGO_TILT).sin()
            - SLAM_KICK * clamp(rig.slam, 0.0, 1.0);
        let field = (ax * ax + az * az).sqrt();

        self.over_cooldown = (self.over_cooldown - dt).max(0.0);
        let prev: Vec<(f32, f32)> = self.crates.iter().map(|c| (c.x, c.z)).collect();
        for i in 0..n {
            if self.crates[i].gone {
                continue;
            }
            // A base is always a lower index, so it has already moved this
            // frame; copy what its rider needs before borrowing mutably.
            let carried = self.crates[i].base.map(|b| {
                let bc = &self.crates[b];
                (
                    bc.x,
                    bc.z,
                    bc.y + bc.ht,
                    bc.hw,
                    bc.hd,
                    (bc.x - prev[b].0, bc.z - prev[b].1),
                )
            });
            let c = &mut self.crates[i];
            let hold =
                CRATE_MU_S * c.grip * if c.base.is_some() { TOP_GRIP } else { 1.0 } * g_eff;
            if c.fall {
                // Tumbling: ballistic, slewing as it goes.
                c.vy -= CRATE_G * dt;
                c.y += c.vy * dt;
                c.x += c.vx * dt;
                c.z += c.vz * dt;
                c.yaw += c.spin * 2.5 * dt;
                if c.over {
                    // Past the rail there is only the sea: the ship sails on
                    // underneath, so the crate sweeps aft relative to the deck,
                    // and is struck from the books once it sinks from view.
                    c.vz += rig.speed * 0.5 * dt;
                    if c.y < SINK_Y {
                        c.gone = true;
                        self.washed += 1;
                    }
                    continue;
                }
                let deck = station_at(c.z).1;
                if c.y <= deck {
                    c.y = deck;
                    c.vy = 0.0;
                    c.vx *= 0.5;
                    c.vz *= 0.5;
                    c.fall = false;
                    c.home = (c.x, c.z); // re-stowed where it landed, until port
                }
                continue;
            }
            if let Some((bx, bz, btop, bhw, bhd, (bdx, bdz))) = carried {
                // Riding a stack: carried by the base while it holds.
                c.y = btop;
                if !c.loose {
                    c.x += bdx;
                    c.z += bdz;
                    if field > hold {
                        c.loose = true;
                        c.vx = bdx * inv_dt;
                        c.vz = bdz * inv_dt;
                    } else if field < hold * 0.5 {
                        let k = clamp(RESTOW_RATE * dt, 0.0, 1.0);
                        c.x += (c.home.0 - c.x) * k;
                        c.z += (c.home.1 - c.z) * k;
                        c.yaw -= c.yaw * k;
                    }
                }
                if c.loose {
                    // Kinetic friction drags it toward its base's own motion.
                    let (rvx, rvz) = (c.vx - bdx * inv_dt, c.vz - bdz * inv_dt);
                    let rv = (rvx * rvx + rvz * rvz).sqrt();
                    let (fx, fz) = if rv > 1e-4 { (-rvx / rv, -rvz / rv) } else { (0.0, 0.0) };
                    c.vx += (ax + fx * CRATE_MU_K * g_eff) * dt;
                    c.vz += (az + fz * CRATE_MU_K * g_eff) * dt;
                    c.x += c.vx * dt;
                    c.z += c.vz * dt;
                    c.yaw += c.spin * CRATE_SPIN * rv * dt;
                    if rv < CRATE_STOP && field < hold {
                        c.loose = false;
                        c.vx = 0.0;
                        c.vz = 0.0;
                    }
                    // Slid past the base's edge: over it goes.
                    if (c.x - bx).abs() > bhw || (c.z - bz).abs() > bhd {
                        c.base = None;
                        c.fall = true;
                        c.vy = 0.0;
                    }
                }
                continue;
            }
            // On the deck.
            if !c.loose {
                if field > hold {
                    c.loose = true;
                } else if field < hold * 0.5 {
                    // Calm: the crew ease shifted cargo back to its stowage.
                    let k = clamp(RESTOW_RATE * dt, 0.0, 1.0);
                    c.x += (c.home.0 - c.x) * k;
                    c.z += (c.home.1 - c.z) * k;
                    c.yaw -= c.yaw * k;
                }
            }
            if c.loose {
                let v = (c.vx * c.vx + c.vz * c.vz).sqrt();
                let (fx, fz) = if v > 1e-4 { (-c.vx / v, -c.vz / v) } else { (0.0, 0.0) };
                c.vx += (ax + fx * CRATE_MU_K * g_eff) * dt;
                c.vz += (az + fz * CRATE_MU_K * g_eff) * dt;
                c.x += c.vx * dt;
                c.z += c.vz * dt;
                c.yaw += c.spin * CRATE_SPIN * v * dt;
                if v < CRATE_STOP && field < hold {
                    c.loose = false;
                    c.vx = 0.0;
                    c.vz = 0.0;
                }
            }
            c.y = station_at(c.z).1;
        }

        // --- Neighbours: keep grounded crates from interpenetrating -----------
        // One cheap AABB separation pass (yaw ignored): push overlapping pairs
        // apart along the shallower axis and kill their approach there. Stack
        // tops and falling crates are skipped; a top touching its base's top
        // face doesn't overlap it (strict test). Runs *before* the fences so a
        // pile pressed against the wall can't be shoved back through it.
        for i in 0..n {
            for k in i + 1..n {
                let (a_ok, b_ok) = {
                    let (a, b) = (&self.crates[i], &self.crates[k]);
                    (
                        a.base.is_none() && !a.fall,
                        b.base.is_none() && !b.fall,
                    )
                };
                if !a_ok || !b_ok {
                    continue;
                }
                let (l, r) = self.crates.split_at_mut(k);
                let (a, b) = (&mut l[i], &mut r[0]);
                if a.y >= b.y + b.ht || b.y >= a.y + a.ht {
                    continue;
                }
                let (dx, dz) = (b.x - a.x, b.z - a.z);
                let px = a.hw + b.hw - dx.abs();
                let pz = a.hd + b.hd - dz.abs();
                if px <= 0.0 || pz <= 0.0 {
                    continue;
                }
                if px < pz {
                    let s = if dx >= 0.0 { 1.0 } else { -1.0 };
                    a.x -= s * px * 0.5;
                    b.x += s * px * 0.5;
                    let v = (a.vx + b.vx) * 0.5;
                    a.vx = v;
                    b.vx = v;
                } else {
                    let s = if dz >= 0.0 { 1.0 } else { -1.0 };
                    a.z -= s * pz * 0.5;
                    b.z += s * pz * 0.5;
                    let v = (a.vz + b.vz) * 0.5;
                    a.vz = v;
                    b.vz = v;
                }
            }
        }

        // --- Fences: the woodwork a sliding crate fetches up against ----------
        // Everything not riding a stack is fenced, a tumbling crate included
        // (it may be mid-air, but the bulwarks still stand in its way) — only a
        // crate already carried over the rail is past fencing. Last, so nothing
        // a slide or a shove did this frame leaves a crate outside the hull. A
        // crate stopped short flings the speed it lost into any crate riding it
        // (inertia); one carried overboard spills its riders where it stood.
        // Both are collected and applied after the borrow ends.
        let mut kicks: Vec<(usize, f32, f32)> = Vec::new();
        let mut spills: Vec<(usize, f32, f32)> = Vec::new();
        for i in 0..n {
            {
                let c = &self.crates[i];
                if c.base.is_some() || c.gone || c.over {
                    continue;
                }
            }
            let c = &mut self.crates[i];
            let mut kick = (0.0f32, 0.0f32);
            // The bulwarks, following the hull's curve at this station.
            let limit = station_at(c.z).0 - WALL_GAP - c.hw;
            if c.x.abs() > limit {
                let s = c.x.signum();
                // Slammed in hard enough, the crate carries the rail instead
                // of fetching up against the wall: over it goes.
                if c.vx * s > OVERBOARD_SPEED * c.grip && self.over_cooldown <= 0.0 {
                    self.over_cooldown = OVERBOARD_COOLDOWN;
                    c.over = true;
                    c.fall = true;
                    c.loose = true;
                    c.vy = OVERBOARD_POP;
                    spills.push((i, c.vx, c.vz));
                    continue;
                }
                c.x = s * limit;
                if c.vx * s > 0.0 {
                    kick.0 = c.vx;
                    c.vx = -c.vx * CRATE_BOUNCE;
                }
            }
            // The quarterdeck riser aft; the rising bow forward.
            let z_max = QDECK_BREAK - WALL_GAP - c.hd;
            let z_min = -6.5 + c.hd;
            if c.z > z_max {
                c.z = z_max;
                if c.vz > 0.0 {
                    kick.1 = c.vz;
                    c.vz = -c.vz * CRATE_BOUNCE;
                }
            } else if c.z < z_min {
                c.z = z_min;
                if c.vz < 0.0 {
                    kick.1 = c.vz;
                    c.vz = -c.vz * CRATE_BOUNCE;
                }
            }
            // The companion stairs block the starboard run abaft the waist.
            let stair_x = 2.0 - WALL_GAP - c.hw;
            if c.z + c.hd > QDECK_BREAK - 4.0 && c.x > stair_x {
                c.x = stair_x;
                if c.vx > 0.0 {
                    kick.0 = c.vx;
                    c.vx = -c.vx * CRATE_BOUNCE;
                }
            }
            // The mast's foot: shove out along the shallower overlap.
            let px = MAST_HW + c.hw - c.x.abs();
            let pz = MAST_HW + c.hd - c.z.abs();
            if px > 0.0 && pz > 0.0 {
                if px < pz {
                    c.x += c.x.signum() * px;
                    c.vx = -c.vx * CRATE_BOUNCE;
                } else {
                    c.z += c.z.signum() * pz;
                    c.vz = -c.vz * CRATE_BOUNCE;
                }
            }
            if kick != (0.0, 0.0) {
                kicks.push((i, kick.0, kick.1));
            }
        }
        for &(b, kx, kz) in &kicks {
            for i in 0..n {
                let c = &mut self.crates[i];
                if c.base == Some(b) && !c.loose && !c.fall {
                    c.loose = true;
                    c.vx = kx;
                    c.vz = kz;
                }
            }
        }
        // A crate riding one that went over the rail is left mid-air where its
        // base was: it tumbles to the deck with the base's last motion (and may
        // well follow it over next).
        for &(b, vx, vz) in &spills {
            for i in 0..n {
                let c = &mut self.crates[i];
                if c.base == Some(b) {
                    c.base = None;
                    c.loose = true;
                    c.fall = true;
                    c.vx = vx;
                    c.vz = vz;
                    c.vy = 0.0;
                }
            }
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

        self.step_cargo(rig, roll, pitch_ang, dt);
        let pts = self.draw_deck(&sway, pitch_ang, &lume, h, w);
        self.draw_rig(&sway, rig, pitch_ang, &lume, t, h, w, &pts);
        // The chart board after the rig, so no rope paints across the parchment;
        // it stands on the breast rail, nearer the eye than everything forward.
        if let Some(chart) = &rig.chart {
            self.draw_chart(&sway, chart, pitch_ang, &lume, h, w);
        }
        // The wheel last: it is the nearest thing on the ship, standing between
        // the eye and everything else.
        self.draw_wheel(&sway, pitch_ang, &lume, h, w);
        self.deck_silhouette = pts.silhouette;
    }

    /// The hull: deck floor, quarterdeck, bulwarks, railing, cargo and bowsprit,
    /// lofted from [`STATIONS`] through the helm camera and drawn bow → stern so
    /// nearer woodwork paints over farther (macroquad has no depth buffer).
    /// Returns the screen anchors the rig and the rain hang on.
    fn draw_deck(
        &self,
        sway: &impl Fn(f32, f32) -> Vec2,
        pitch_ang: f32,
        lume: &Lume,
        h: f32,
        w: f32,
    ) -> DeckPoints {
        let (sp, cp) = pitch_ang.sin_cos();
        let cam = |x: f32, y: f32, z: f32| {
            helm_cam(x, y, z, sp, cp, w, h).map(|(p, s)| (sway(p.x, p.y), s))
        };
        let pt = |x: f32, y: f32, z: f32| cam(x, y, z).map(|(p, _)| p);
        let quad = |a: Vec2, b: Vec2, c: Vec2, d: Vec2, col: Color| {
            draw_triangle(a, b, c, col);
            draw_triangle(a, c, d, col);
        };
        // Draw a lofted face only when every corner clears the near plane.
        let try_quad =
            |a: Option<Vec2>, b: Option<Vec2>, c: Option<Vec2>, d: Option<Vec2>, col: Color| {
                if let (Some(a), Some(b), Some(c), Some(d)) = (a, b, c, d) {
                    quad(a, b, c, d, col);
                }
            };

        // Face normals in the rig frame (+x starboard, +y up, +z aft): the deck
        // plane tips a little aft toward the eye; a bulwark's inboard face leans
        // in over the deck, so it takes the light from the *opposite* rail's side.
        let n_deck = (0.0, 0.94, 0.34);
        let n_wall = |side: f32| (-side * 0.72, 0.42, 0.55);

        let rail_col = lume.face(RAIL, (0.0, 0.25, 0.97));
        let board_col = lume.face(RAIL_DK, (0.0, 0.8, 0.6));

        // --- Deck floor: planks lofted station to station. Strips between fixed
        // fractions of the half-beam in two alternating tones, so every board
        // springs from the stem and follows the hull's plan curve. A segment
        // standing more up than along (the quarterdeck riser) faces away from an
        // eye abaft it, so it is skipped; the quarterdeck floor then overpaints
        // exactly the run of waist its edge hides, which is the correct
        // painter's-order occlusion. The loft runs in two phases, waist then
        // quarterdeck, with the companion stairs drawn between them: they stand
        // on the one and duck under the other.
        let planks = 9;
        let floor_pair = |pair: &[(f32, f32, f32, f32)]| {
            let (z0, b0, d0, _) = pair[0];
            let (z1, b1, d1, _) = pair[1];
            if (d1 - d0).abs() > (z1 - z0).abs() {
                return;
            }
            for i in 0..planks {
                let u0 = i as f32 / planks as f32 * 2.0 - 1.0;
                let u1 = (i + 1) as f32 / planks as f32 * 2.0 - 1.0;
                let tone = if i % 2 == 0 { DECK_A } else { DECK_B };
                try_quad(
                    pt(u0 * b0, d0, z0),
                    pt(u1 * b0, d0, z0),
                    pt(u1 * b1, d1, z1),
                    pt(u0 * b1, d1, z1),
                    lume.face(tone, n_deck),
                );
            }
        };
        // A station belongs to the quarterdeck run if it lies aft of the break
        // (the raised twin of the doubled break station included).
        let is_aft = |z: f32, d: f32| z > QDECK_BREAK || (z == QDECK_BREAK && d > 0.4);
        for pair in STATIONS.windows(2).filter(|p| !is_aft(p[0].0, p[0].2)) {
            floor_pair(pair);
        }

        // --- Companion stairs: the way down from the quarterdeck to the waist,
        // starboard, under the breast rail's open end. A long shallow flight,
        // so it emerges from behind the platform edge into view (a steep ladder
        // would hide entirely inside the wedge the edge masks from the eye).
        // Only the treads and the inboard carriage face the eye; the risers
        // face the bow. The sloped handrail pokes above the platform edge, so
        // the flight reads even where its top treads are rightly hidden.
        // Deferred until the waist walls and railing are down: the flight
        // stands inboard of them, so it must paint over the starboard wall,
        // never hide behind it.
        let companion_stairs = || {
            let (x0, x1) = (2.0f32, 3.3); // inboard / outboard edges
            let steps = 6;
            let run = 4.0; // fore-aft reach of the flight
            let (_, qd_y, _) = station_at(QDECK_BREAK + 0.1);
            let side_col = lume.col(RAIL_DK, lume.diff(n_wall(1.0)), 0.9);
            for k in (1..steps).rev() {
                let y = qd_y * (1.0 - k as f32 / steps as f32);
                let za = QDECK_BREAK - run * k as f32 / steps as f32;
                let zb = QDECK_BREAK - run * (k as f32 - 1.0) / steps as f32;
                // The carriage: the solid side wall under this tread.
                try_quad(
                    pt(x0, 0.0, za),
                    pt(x0, 0.0, zb),
                    pt(x0, y, zb),
                    pt(x0, y, za),
                    side_col,
                );
                // The tread.
                let tone = if k % 2 == 0 { DECK_A } else { DECK_B };
                try_quad(
                    pt(x0, y, za),
                    pt(x1, y, za),
                    pt(x1, y, zb),
                    pt(x0, y, zb),
                    lume.face(tone, n_deck),
                );
            }
            // The handrail down the inboard carriage: head post height off the
            // platform, foot height off the lowest tread.
            let rail_at = |f: f32| (QDECK_BREAK - run * f, (qd_y + 0.8) + (0.75 - (qd_y + 0.8)) * f);
            for f in [0.3f32, 0.7] {
                let (z, ry) = rail_at(f);
                let ty = qd_y * (1.0 - f); // the tread the post stands on
                if let (Some((b, s)), Some((t, _))) = (cam(x0, ty, z), cam(x0, ry, z)) {
                    let pw = (0.05 * s).max(1.0);
                    quad(
                        vec2(b.x - pw, b.y),
                        vec2(b.x + pw, b.y),
                        vec2(t.x + pw, t.y),
                        vec2(t.x - pw, t.y),
                        rail_col,
                    );
                }
            }
            let (hz, hy) = rail_at(0.0);
            let (fz, fy) = rail_at(0.95);
            if let (Some((a, s)), Some((b, _))) = (cam(x0, hy, hz), cam(x0, fy, fz)) {
                let th = 0.07 * s;
                quad(a, b, vec2(b.x, b.y + th), vec2(a.x, a.y + th), board_col);
            }
        };

        // The rest of the woodwork draws in two depth phases around the
        // quarterdeck floor: everything at waist level first, then the platform
        // over it, then the quarterdeck's own walls and rails. That is what
        // makes the platform opaque: the waist walls, rails and cargo inside
        // the wedge its edge masks are painted first and covered by it, rather
        // than painting through it.

        // --- The mast's shadow, thrown across the deck by the key light --------
        // The mast stands at the origin; each metre of it lands on the deck
        // offset along the light's horizontal throw, so the shadow swings round
        // the deck with the hour and the ship's heading, and stretches as the
        // sun or moon sinks. A translucent strip drawn in segments, each kept
        // inside the hull's footprint so it never paints on the sea; its
        // darkness is the key light's share of a lit deck face, so it fades
        // away under a gale's overcast or a bare night sky. Called once per
        // deck level with that level's fore-aft range.
        let mast_shadow = |z_lo: f32, z_hi: f32| {
            let (lx, ly, lz) = lume.l;
            if ly <= 0.05 {
                return;
            }
            // Horizontal throw per metre of mast height; the pole length is
            // capped so a low light runs the shadow off the hull, not forever.
            let (tx, tz) = (-lx / ly, -lz / ly);
            let throw = (tx * tx + tz * tz).sqrt().max(1e-3);
            let mast_h = 14.0f32.min(30.0 / throw);
            let key_l = (lume.key.0 + lume.key.1 + lume.key.2) / 3.0;
            let amb_l = (lume.ambient.0 + lume.ambient.1 + lume.ambient.2) / 3.0;
            let key_part = (1.0 - AMBIENT_SHARE) * ly * key_l;
            let shade = key_part / (key_part + AMBIENT_SHARE * amb_l + 1e-4);
            let col = Color::new(0.0, 0.0, 0.0, 0.5 * shade);
            // Perpendicular of the shadow's run, for the strip's width.
            let (qx, qz) = (-tz / throw, tx / throw);
            let segs = 12;
            for i in 0..segs {
                let (t0, t1) = (i as f32 / segs as f32, (i + 1) as f32 / segs as f32);
                // The pole tapers, so its shadow thins toward the tip.
                let (w0, w1) = (0.17 * (1.0 - 0.45 * t0), 0.17 * (1.0 - 0.45 * t1));
                let (x0, z0) = (t0 * mast_h * tx, t0 * mast_h * tz);
                let (x1, z1) = (t1 * mast_h * tx, t1 * mast_h * tz);
                let (zm, xm) = ((z0 + z1) * 0.5, (x0 + x1) * 0.5);
                let (bm, _, _) = station_at(zm);
                if xm.abs() > bm - 0.1 || !(z_lo..z_hi).contains(&zm) {
                    continue;
                }
                let (_, d0, _) = station_at(z0);
                let (_, d1, _) = station_at(z1);
                try_quad(
                    pt(x0 - qx * w0, d0 + 0.02, z0 - qz * w0),
                    pt(x0 + qx * w0, d0 + 0.02, z0 + qz * w0),
                    pt(x1 + qx * w1, d1 + 0.02, z1 + qz * w1),
                    pt(x1 - qx * w1, d1 + 0.02, z1 - qz * w1),
                    col,
                );
            }
        };

        // --- Bulwarks: a planked wall up each side riding the deck edge, its
        // height running the sheer in `STATIONS`, with strakes and a cap board.
        // Called once per deck level.
        let bulwarks = |aft: bool| {
            for side in [-1.0f32, 1.0] {
                let wall_col = lume.col(RAIL, lume.diff(n_wall(side)), 0.9);
                let seam_col = lume.col(RAIL_DK, lume.diff(n_wall(side)), 0.9);
                for pair in STATIONS.windows(2).filter(|p| is_aft(p[0].0, p[0].2) == aft) {
                    let (z0, b0, d0, w0) = pair[0];
                    let (z1, b1, d1, w1) = pair[1];
                    let base0 = pt(side * b0, d0, z0);
                    let base1 = pt(side * b1, d1, z1);
                    let top1 = pt(side * b1, d1 + w1, z1);
                    let top0 = pt(side * b0, d0 + w0, z0);
                    try_quad(base0, base1, top1, top0, wall_col);
                    // Strakes: seam lines along the inboard face.
                    for fr in [0.34f32, 0.67] {
                        if let (Some(s0), Some(s1)) =
                            (pt(side * b0, d0 + w0 * fr, z0), pt(side * b1, d1 + w1 * fr, z1))
                        {
                            draw_line(s0.x, s0.y, s1.x, s1.y, (h * 0.0022).max(1.0), seam_col);
                        }
                    }
                    // A thin cap board on top, flared a touch outboard.
                    let c0 = pt(side * b0 * 1.04, d0 + w0 + 0.07, z0);
                    let c1 = pt(side * b1 * 1.04, d1 + w1 + 0.07, z1);
                    try_quad(top0, top1, c1, c0, lume.face(RAIL_DK, (0.0, 0.9, 0.44)));
                }
            }
        };

        // --- Open railing: stanchions on the bulwark cap joined by a rail board
        // riding their tops, so the topsides read as guarded rather than a bare
        // wall. One post per station; perspective spaces and sizes them. Called
        // once per deck level, so each level's rail run ends at the break and
        // the quarterdeck's begins on its own corner post.
        let post_h = 0.42; // m above the cap
        let post_hw = 0.05;
        let railing = |aft: bool| {
            for side in [-1.0f32, 1.0] {
                let mut prev: Option<(Vec2, f32)> = None; // previous rail top + px/m
                for &(z, b, d, wh) in STATIONS.iter().filter(|s| is_aft(s.0, s.2) == aft) {
                    let (Some((cap_p, s)), Some((rail_p, _))) =
                        (cam(side * b, d + wh, z), cam(side * b, d + wh + post_h, z))
                    else {
                        continue;
                    };
                    // The rail board from the previous post's top.
                    if let Some((pr, ps)) = prev {
                        quad(
                            pr,
                            rail_p,
                            vec2(rail_p.x, rail_p.y + 0.07 * s),
                            vec2(pr.x, pr.y + 0.07 * ps),
                            board_col,
                        );
                    }
                    prev = Some((rail_p, s));
                    // The stanchion itself.
                    let pw = (post_hw * s).max(1.0);
                    quad(
                        vec2(cap_p.x - pw, cap_p.y),
                        vec2(cap_p.x + pw, cap_p.y),
                        vec2(rail_p.x + pw, rail_p.y),
                        vec2(rail_p.x - pw, rail_p.y),
                        rail_col,
                    );
                }
            }
        };

        // Waist level: shadow on its planks, then its walls and rails, then the
        // companion stairs over them (the flight is inboard of the walls).
        mast_shadow(-14.5, QDECK_BREAK);
        bulwarks(false);
        railing(false);
        companion_stairs();

        // --- Deck cargo: the lashed crates. Their layout and motion live in
        // `self.crates` (one per hold unit, stowed helm-first; stepped by
        // `step_cargo`, which lets extreme weather and violent turns shift
        // them). Drawn far → near so nearer crates overlap those behind; each
        // side face is culled and shaded by its yawed outward normal, so a
        // crate slewed by a slide keeps honest light. Drawn before the
        // quarterdeck floor, so a crate reaching into the wedge the platform
        // edge masks is rightly covered by it.
        let mut idx: Vec<usize> = (0..self.crates.len()).collect();
        idx.sort_by(|&a, &b| {
            // Far (small z) first; a stacked crate (greater height) over its base.
            let (ca, cb) = (&self.crates[a], &self.crates[b]);
            (ca.z, ca.y).partial_cmp(&(cb.z, cb.y)).unwrap()
        });
        // Outward normals of the four side faces, before the crate's yaw.
        const SIDE_N: [(f32, f32); 4] = [(0.0, -1.0), (1.0, 0.0), (0.0, 1.0), (-1.0, 0.0)];
        for &k in &idx {
            let c = &self.crates[k];
            if c.gone {
                continue;
            }
            let (ys, yc) = c.yaw.sin_cos();
            let plan = |sx: f32, sz: f32| {
                let (ox, oz) = (sx * c.hw, sz * c.hd);
                (c.x + ox * yc - oz * ys, c.z + ox * ys + oz * yc)
            };
            let corners = [plan(-1.0, -1.0), plan(1.0, -1.0), plan(1.0, 1.0), plan(-1.0, 1.0)];
            let bot = corners.map(|(x, z)| pt(x, c.y, z));
            let top = corners.map(|(x, z)| pt(x, c.y + c.ht, z));
            for f in 0..4 {
                let g2 = (f + 1) % 4;
                let n = (
                    SIDE_N[f].0 * yc - SIDE_N[f].1 * ys,
                    SIDE_N[f].0 * ys + SIDE_N[f].1 * yc,
                );
                // Cull faces turned away from the eye (it stands at the origin
                // of x, CAM_AFT aft; height doesn't matter for vertical faces).
                let fcx = (corners[f].0 + corners[g2].0) * 0.5;
                let fcz = (corners[f].1 + corners[g2].1) * 0.5;
                if n.0 * -fcx + n.1 * (CAM_AFT - fcz) <= 0.0 {
                    continue;
                }
                let tone = if n.1 > 0.55 { CRATE_MID } else { CRATE_DK };
                let norm = (n.0 * 0.95, 0.15, n.1 * 0.95);
                try_quad(bot[f], bot[g2], top[g2], top[f], lume.face(tone, norm));
                // A batten across the eye-facing face so the box reads as planked.
                if n.1 > 0.55 {
                    let (lo, hi) = (c.y + c.ht * 0.42, c.y + c.ht * 0.52);
                    try_quad(
                        pt(corners[f].0, lo, corners[f].1),
                        pt(corners[g2].0, lo, corners[g2].1),
                        pt(corners[g2].0, hi, corners[g2].1),
                        pt(corners[f].0, hi, corners[f].1),
                        lume.face(CRATE_DK, norm),
                    );
                }
            }
            // The lit top, always visible from a helmsman's eye above the pile.
            try_quad(top[0], top[1], top[2], top[3], lume.face(CRATE_TOP, (0.0, 0.97, 0.26)));
        }

        // Quarterdeck level: the platform floor over the waist detail, then its
        // own shadow run, walls and rails.
        for pair in STATIONS.windows(2).filter(|p| is_aft(p[0].0, p[0].2)) {
            floor_pair(pair);
        }
        mast_shadow(QDECK_BREAK, 8.5);
        bulwarks(true);
        railing(true);

        // --- Breast rail: a railing across the quarterdeck's forward edge, so
        // the raised platform the helmsman stands on actually reads from the
        // helm: you look over it, down onto the waist where the cargo rides.
        // It spans port to just past the centreline; the starboard end stays
        // open where the companion stairs come up. Drawn after the crates,
        // since it stands nearer the eye than everything forward of the break.
        {
            let brk = BREAST_RAIL_Z; // just aft of the quarterdeck break
            let (_, qd_y, _) = station_at(brk);
            let rail_y = qd_y + BREAST_RAIL_H; // waist-high off the platform
            let (rail_l, rail_r) = (-3.25f32, 1.6); // port bulwark → the stair head
            let posts = 6;
            for i in 0..posts {
                let x = rail_l + (rail_r - rail_l) * (i as f32 / (posts - 1) as f32);
                if let (Some((b0, s)), Some((t0, _))) = (cam(x, qd_y, brk), cam(x, rail_y, brk))
                {
                    let pw = (0.05 * s).max(1.0);
                    quad(
                        vec2(b0.x - pw, b0.y),
                        vec2(b0.x + pw, b0.y),
                        vec2(t0.x + pw, t0.y),
                        vec2(t0.x - pw, t0.y),
                        rail_col,
                    );
                }
            }
            // The rail board along the post tops, and a mid rail below it.
            for (y, th_m) in [(rail_y, 0.07f32), (qd_y + 0.45, 0.045)] {
                if let (Some((l, s)), Some((r, _))) = (cam(rail_l, y, brk), cam(rail_r, y, brk))
                {
                    let th = th_m * s;
                    quad(l, r, vec2(r.x, r.y + th), vec2(l.x, l.y + th), board_col);
                }
            }
        }

        // --- Bowsprit: a tapered spar from the stemhead out toward the horizon.
        // It anchors the forestay and closes the ship's profile so the prow
        // reads as a ship's, not a raft's. Two-tone halves, matching the mast.
        let sprit_tip = (2.7f32, -18.2f32); // (height, z) of the tip
        {
            let base = (1.5f32, -14.6f32);
            let lit_l = lume.face(SPAR, (-0.66, 0.4, 0.64));
            let lit_r = lume.face(SPAR_DK, (0.66, 0.4, 0.64));
            if let (Some(b0), Some(b1), Some(t1), Some(t0), Some(mb), Some(mt)) = (
                pt(-0.18, base.0, base.1),
                pt(0.18, base.0, base.1),
                pt(0.08, sprit_tip.0, sprit_tip.1),
                pt(-0.08, sprit_tip.0, sprit_tip.1),
                pt(0.0, base.0, base.1),
                pt(0.0, sprit_tip.0, sprit_tip.1),
            ) {
                draw_triangle(b0, mb, mt, lit_l);
                draw_triangle(b0, mt, t0, lit_l);
                draw_triangle(mb, b1, t1, lit_r);
                draw_triangle(mb, t1, mt, lit_r);
            }
        }

        // --- Screen anchors for the rig's ropes and the rain -------------------
        // A point atop the rail (the stanchion tops) at fore-aft z.
        let rail_top = |side: f32, z: f32| -> Option<Vec2> {
            let (b, d, wh) = station_at(z);
            pt(side * b, d + wh + post_h, z)
        };
        let off = vec2(w * 0.5, h * 2.0); // fallback: parked far off-screen
        let foot = |side: f32, z: f32| rail_top(side, z).unwrap_or(off);
        let sheet_foot = [foot(-1.0, 3.5), foot(1.0, 3.5)];
        let brace_foot = [foot(-1.0, 6.5), foot(1.0, 6.5)];
        let bowsprit_tip = pt(0.0, sprit_tip.0, sprit_tip.1).unwrap_or(off);

        // Outer silhouette for rain occlusion: down each rail bow → stern (as
        // far aft as the near plane allows), then straight off the bottom of the
        // screen, so the polygon encloses every plank, wall and rail drawn.
        // Built through the same projection and `sway`, so it tracks the hull.
        let rail_line = |side: f32| -> Vec<Vec2> {
            STATIONS
                .iter()
                .filter_map(|&(z, b, d, wh)| pt(side * b * 1.04, d + wh + post_h, z))
                .collect()
        };
        let port = rail_line(-1.0);
        let stbd = rail_line(1.0);
        let mut silhouette = port.clone();
        if let (Some(lp), Some(ls)) = (port.last(), stbd.last()) {
            silhouette.push(vec2(lp.x, h * 1.5)); // down off the bottom edge
            silhouette.push(vec2(ls.x, h * 1.5));
        }
        silhouette.extend(stbd.iter().rev());

        DeckPoints { silhouette, sheet_foot, brace_foot, bowsprit_tip }
    }

    /// The ship's wheel, standing on the quarterdeck just ahead of the eye and
    /// spun toward the rudder: a spoked ring on a short pedestal. The pedestal
    /// is projected through the helm camera like the deck it stands on; the
    /// ring itself is drawn flat at the hub's projected scale (it faces the
    /// helmsman, so its plane is all but parallel to the screen).
    fn draw_wheel(
        &self,
        sway: &impl Fn(f32, f32) -> Vec2,
        pitch_ang: f32,
        lume: &Lume,
        h: f32,
        w: f32,
    ) {
        // Where the helm stands: the wheel's plane, hub height and radius (m).
        // Sized so the rim stays under the horizon: the wheel should command the
        // foreground without blocking the view dead ahead you navigate by.
        const WHEEL_Z: f32 = 6.6;
        // Hub about waist height off the quarterdeck, far under the eye line:
        // the helmsman looks down onto the wheel, its lower rim running off the
        // bottom of the screen.
        const HUB_Y: f32 = 1.25; // above the waist deck; the pedestal adds the rest
        const WHEEL_R: f32 = 0.52;
        let (sp, cp) = pitch_ang.sin_cos();
        let cam = |x: f32, y: f32, z: f32| {
            helm_cam(x, y, z, sp, cp, w, h).map(|(p, s)| (sway(p.x, p.y), s))
        };
        let Some((hub, s)) = cam(0.0, HUB_Y, WHEEL_Z) else {
            return; // nod swung the wheel inside the near plane: nothing to draw
        };
        let a = self.wheel_angle;
        // The wheel stands upright facing the helmsman (aft), so its whole face
        // shares one normal.
        let rim_col = lume.face(WHEEL_C, (0.0, 0.2, 0.98));
        let spoke_col = lume.face(WHEEL_DK, (0.0, 0.2, 0.98));

        // Pedestal: a tapered post from the quarterdeck up to the hub.
        let (_, deck_y, _) = station_at(WHEEL_Z);
        if let Some((base, _)) = cam(0.0, deck_y, WHEEL_Z) {
            let (bw, tw) = (0.14 * s, 0.09 * s);
            let (b0, b1) = (vec2(base.x - bw, base.y), vec2(base.x + bw, base.y));
            let (t1, t0) = (vec2(hub.x + tw, hub.y), vec2(hub.x - tw, hub.y));
            draw_triangle(b0, b1, t1, spoke_col);
            draw_triangle(b0, t1, t0, spoke_col);
        }

        let r = WHEEL_R * s;
        // Rim: a ring approximated by a fan of short trapezoids.
        let seg = 24;
        for i in 0..seg {
            let t0 = i as f32 / seg as f32 * TAU + a;
            let t1 = (i + 1) as f32 / seg as f32 * TAU + a;
            let inner = r * 0.78;
            let p0o = vec2(hub.x + t0.cos() * r, hub.y + t0.sin() * r);
            let p1o = vec2(hub.x + t1.cos() * r, hub.y + t1.sin() * r);
            let p1i = vec2(hub.x + t1.cos() * inner, hub.y + t1.sin() * inner);
            let p0i = vec2(hub.x + t0.cos() * inner, hub.y + t0.sin() * inner);
            draw_triangle(p0o, p1o, p1i, rim_col);
            draw_triangle(p0o, p1i, p0i, rim_col);
        }
        // Spokes radiating past the rim into handles.
        for k in 0..8 {
            let ta = k as f32 / 8.0 * TAU + a;
            let (sn, cs) = ta.sin_cos();
            let dir = vec2(cs, sn);
            let n = vec2(-sn, cs) * (r * 0.06); // perpendicular, for spoke thickness
            let outer = r * 1.18;
            let p0 = hub + n;
            let p1 = hub + dir * outer + n;
            let p2 = hub + dir * outer - n;
            let p3 = hub - n;
            draw_triangle(p0, p1, p2, spoke_col);
            draw_triangle(p0, p2, p3, spoke_col);
        }
        // Hub.
        draw_circle(hub.x, hub.y, r * 0.22, rim_col);
    }

    /// The world chart aboard: a small parchment board clipped to the breast
    /// rail just port of the wheel, leaned like a chart desk so its face tips
    /// up toward the helmsman's eye. Inked with the isles and a red pin for the
    /// ship (see [`DeckChart`]), so the whole world is a glance away from the
    /// helm without opening the captain's log.
    fn draw_chart(
        &self,
        sway: &impl Fn(f32, f32) -> Vec2,
        chart: &DeckChart,
        pitch_ang: f32,
        lume: &Lume,
        h: f32,
        w: f32,
    ) {
        // The board in metres: bottom edge resting on the breast rail's top,
        // port of the wheel so it never blocks the view dead ahead; the top
        // edge leans toward the bow so the face reads from above.
        const XL: f32 = -2.4;
        const XR: f32 = -1.45;
        const BOARD_H: f32 = 0.68;
        const LEAN: f32 = 0.5; // radians off vertical, top toward the bow

        let (_, qd_y, _) = station_at(BREAST_RAIL_Z);
        let y0 = qd_y + BREAST_RAIL_H - 0.04; // clipped onto the top rail board
        let (ls, lc) = LEAN.sin_cos();
        let (y1, z1) = (y0 + BOARD_H * lc, BREAST_RAIL_Z - BOARD_H * ls);

        let (sp, cp) = pitch_ang.sin_cos();
        let cam = |x: f32, y: f32, z: f32| {
            helm_cam(x, y, z, sp, cp, w, h).map(|(p, s)| (sway(p.x, p.y), s))
        };
        // Every corner must clear the near plane, or the board is off-screen.
        let (Some((bl, s)), Some((br, _)), Some((tl, _)), Some((tr, _))) = (
            cam(XL, y0, BREAST_RAIL_Z),
            cam(XR, y0, BREAST_RAIL_Z),
            cam(XL, y1, z1),
            cam(XR, y1, z1),
        ) else {
            return;
        };

        // A point on the face by chart coordinates, bilinear across the
        // projected corners: u west → east, v south → north (up the board).
        let at = |u: f32, v: f32| -> Vec2 {
            let bot = bl + (br - bl) * u;
            let top = tl + (tr - tl) * u;
            bot + (top - bot) * v
        };
        let quad = |a: Vec2, b: Vec2, c: Vec2, d: Vec2, col: Color| {
            draw_triangle(a, b, c, col);
            draw_triangle(a, c, d, col);
        };

        // One shade for the whole face: it tips up toward the aft sky.
        let diff = lume.diff((0.0, ls, lc));
        let parch = lume.col(CHART_PARCH, diff, 1.0);
        let ink = lume.col(CHART_INK, diff, 1.0);
        let frame_col = lume.col(RAIL_DK, diff, 0.9);
        let faint = Color::new(ink.r, ink.g, ink.b, ink.a * 0.3);
        let line_w = (0.012 * s).max(1.0);

        // The wooden backing board, a whisker proud of the parchment all round,
        // then the sheet itself and a sketched double border.
        let f = 0.06;
        quad(at(-f, -f), at(1.0 + f, -f), at(1.0 + f, 1.0 + f), at(-f, 1.0 + f), frame_col);
        quad(at(0.0, 0.0), at(1.0, 0.0), at(1.0, 1.0), at(0.0, 1.0), parch);
        let border = |inset: f32, col: Color| {
            let pts = [
                at(inset, inset),
                at(1.0 - inset, inset),
                at(1.0 - inset, 1.0 - inset),
                at(inset, 1.0 - inset),
            ];
            for i in 0..4 {
                let (a, b) = (pts[i], pts[(i + 1) % 4]);
                draw_line(a.x, a.y, b.x, b.y, line_w, col);
            }
        };
        border(0.035, ink);
        // A faint graticule so the open sea reads as charted, not blank.
        for t in [1.0 / 3.0, 2.0 / 3.0] {
            let (a, b) = (at(t, 0.05), at(t, 0.95));
            draw_line(a.x, a.y, b.x, b.y, line_w, faint);
            let (a, b) = (at(0.05, t), at(0.95, t));
            draw_line(a.x, a.y, b.x, b.y, line_w, faint);
        }

        // The isles, inked inside a margin so none kisses the border. Ports get
        // the heavier blot, the rest a fleck.
        let plot = |u: f32, v: f32| at(0.08 + u * 0.84, 0.08 + v * 0.84);
        for &(u, v, is_port) in chart.isles {
            let p = plot(u, v);
            let r = if is_port { 0.020 * s } else { 0.013 * s };
            draw_circle(p.x, p.y, r.max(1.0), ink);
        }
        // The ship's pin: a red-headed tack over the plots, ringed so it reads
        // against a crowded archipelago.
        let (su, sv) = chart.ship;
        let p = plot(clamp(su, 0.0, 1.0), clamp(sv, 0.0, 1.0));
        draw_circle_lines(p.x, p.y, (0.030 * s).max(2.0), line_w, ink);
        draw_circle(p.x, p.y, (0.018 * s).max(1.5), lume.col(CHART_PIN, diff, 1.0));

        // Two clips lashing the board to the rail, so it hangs rather than floats.
        for u in [0.14f32, 0.86] {
            quad(
                at(u - 0.035, -0.10),
                at(u + 0.035, -0.10),
                at(u + 0.035, 0.05),
                at(u - 0.035, 0.05),
                lume.col(RAIL, diff, 0.9),
            );
        }
    }

    /// Mast, yard and the square sail: the articulating rig. The sail is built
    /// from a grid of cloth cells, each vertex given an out-of-plane depth
    /// (belly + luff), then the whole yard rotated about the mast (the brace)
    /// before projecting through the fake perspective. Cells draw back-to-front
    /// so the curved surface overlaps correctly.
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
        deck: &DeckPoints,
    ) {
        // Rope is round and matte: no face to turn to the light, so it takes
        // a fixed half-diffuse of the hour's colour everywhere.
        let rope_col = lume.col(ROPE, 0.5, 1.0);
        let cx = w * 0.5;
        // The rig is anchored where the helm camera puts the hull's mast station.
        let foot_y = h * HORIZON + w * CAM_F * CAM_UP / CAM_AFT;
        let mast_len = h * 0.74; // tall enough to tower off the top of the screen
        // The bare pole runs 3 m above the yard/sail rigging (the engine's metre
        // scale is ocean::HEAVE_GAIN_PX = 27 px/m). The shrouds make for this very
        // top; the yard and sail stay pinned to `mast_len` below.
        let mast_top = mast_len + 3.0 * 27.0;
        let yard_y = mast_len * 0.90; // yard crosses near the masthead
        let sail_w = w * 0.44;
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
        let stand_off = w * 0.020; // base depth of the sail behind the mast plane
        let depth = -fill * BELLY_DEPTH * sail_w; // belly draft (px); negative = away
        let phase = t * FLAP_HZ * TAU;

        let sail_top = yard_y;
        let sail_bot = yard_y - sail_h * furl;

        // The belly's draft profiles. Down the height the cloth is laced flat
        // along the yard, bows to its deepest about two thirds down, and is
        // hauled partway back at the foot by the sheets (a 3/4 sine arc gives
        // exactly that). Across the width it stays nearly uniform, with just a
        // gentle ease toward the leeches, so the sail reads as one deep fold
        // between yard and foot rather than a bulge between its sides.
        let vert = |v: f32| (v * 0.75 * std::f32::consts::PI).sin();
        let horiz = |u: f32| 1.0 - 0.3 * (2.0 * u).powi(2);
        // The out-of-plane offset of a cloth point at across-fraction `u`
        // (-0.5..0.5) and down-fraction `v` (0 = the head, 1 = the foot).
        // The belly and the flog both fade to nothing at the head, so the cloth
        // stays pinned along the yard and swings out toward the free foot.
        let panel_z = |u: f32, v: f32| -> f32 {
            let belly = depth * vert(v) * horiz(u);
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
        // bowsprit tip (projected by draw_deck). The one piece of standing
        // rigging forward of the canvas, so it draws *before* the sail and the
        // cloth hides its upper run; only the lower reach to the bow shows.
        {
            let tip = deck.bowsprit_tip;
            let head = project(0.0, mast_top, 0.0);
            let thick = (h * 0.0028).max(1.0);
            draw_line(head.x, head.y, tip.x, tip.y, thick, rope_col);
        }

        // --- Sail cloth, a continuous mesh drawn back-to-front by depth --------
        // A grid of cells: columns across the width, rows down the height. The
        // rows are what let the vertical belly actually bow in projection; a
        // single head-to-foot quad would interpolate the arc away. Adjacent
        // cells share their seam vertices exactly, so the cloth reads as one
        // watertight surface from any brace angle. Drawn *before* the spars so
        // the mast and yard (at the rig's z≈0 plane, nearest the viewer) always
        // part the cloth instead of the cloth painting over them.
        let n = SAIL_PANELS;
        let m = SAIL_ROWS;
        let sail_y = |v: f32| sail_top + (sail_bot - sail_top) * v;
        // Every grid vertex, computed once so the cells meeting at a vertex
        // use the very same projected point.
        let grid: Vec<Vec<Vec2>> = (0..=n)
            .map(|j| {
                let u = j as f32 / n as f32 - 0.5;
                (0..=m)
                    .map(|k| {
                        let v = k as f32 / m as f32;
                        let (x, z) = braced(u, panel_z(u, v));
                        project(x, sail_y(v), z)
                    })
                    .collect()
            })
            .collect();
        let cell_z = |i: usize, k: usize| {
            let u = (i as f32 + 0.5) / n as f32 - 0.5;
            let v = (k as f32 + 0.5) / m as f32;
            braced(u, panel_z(u, v)).1
        };
        let mut order: Vec<(usize, usize)> =
            (0..n).flat_map(|i| (0..m).map(move |k| (i, k))).collect();
        // Farthest (most negative z at the cell's centre) first.
        order.sort_by(|&a, &b| cell_z(a.0, a.1).partial_cmp(&cell_z(b.0, b.1)).unwrap());

        for &(i, k) in &order {
            let u = (i as f32 + 0.5) / n as f32 - 0.5;
            let v = (k as f32 + 0.5) / m as f32;
            let tl = grid[i][k];
            let tr = grid[i + 1][k];
            let br = grid[i + 1][k + 1];
            let bl = grid[i][k + 1];
            // The belly catches the light at its deepest reach and falls to
            // shade where the cloth flattens back toward its pinned edges (the
            // yard above, the sheeted foot, the leeches at the sides); a cell
            // braced edge-on (small horizontal span) also dims.
            let belly_lit = 1.0 - 0.28 * fill * (1.0 - vert(v) * horiz(u));
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

        // --- Running rigging: the ropes led aft to the railing. Each side
        // carries a sheet from the clew (the foot's free corner) and, belayed
        // further astern, a brace from the yardarm. The sheet's clew end rides
        // the same brace/belly/luff transform as the cloth's own panels, so it
        // leads wherever the sail swings and trembles with it when it flogs;
        // the brace follows the rigid yard tip. Braced hard on the wind an
        // attachment swings abaft the mast plane; that rope must hide behind
        // the spars, so each rope draws before or after them by its end's depth.
        let rope_thick = (h * 0.0028).max(1.0);
        let ropes: Vec<(Vec<Vec2>, bool)> = {
            let sag = h * 0.035; // the rope's own weight bows the run a little
            let segs = 8;
            // A rope from a rig point (already projected, with its pre-projection
            // depth `z`) down to a belay point on the hull's railing (projected
            // by draw_deck, so the feet ride the woodwork exactly).
            let lead = |top: Vec2, z: f32, foot: Vec2| -> (Vec<Vec2>, bool) {
                let run: Vec<Vec2> = (0..=segs)
                    .map(|i| {
                        let t = i as f32 / segs as f32;
                        let mut p = top.lerp(foot, t);
                        p.y += sag * (t * std::f32::consts::PI).sin();
                        p
                    })
                    .collect();
                (run, z < 0.0)
            };
            let mut ropes = Vec::new();
            for (si, &side) in [-1.0f32, 1.0].iter().enumerate() {
                // The sheet: from the sail mesh's outermost seam at the foot.
                let u = side * 0.5;
                let (kx, kz) = braced(u, panel_z(u, 1.0));
                ropes.push(lead(project(kx, sail_bot, kz), kz, deck.sheet_foot[si]));
                // The brace: from the yard's tip (matching the spar's own span).
                let (ax, az) = braced(side * 0.54, -stand_off);
                ropes.push(lead(project(ax, sail_top, az), az, deck.brace_foot[si]));
                // The leech line: strung corner to corner, from the sail's head
                // at the yard straight down to the clew, the tackle the furl
                // hauls on. It spans free of the cloth (no belly), with just a
                // little slack of its own that shrinks as the furl draws the
                // corners together.
                let (hx, hz) = braced(u, panel_z(u, 0.0));
                let head = project(hx, sail_top, hz);
                let clew = project(kx, sail_bot, kz);
                let slack = head.distance(clew) * 0.05;
                let leech: Vec<Vec2> = (0..=segs)
                    .map(|i| {
                        let t = i as f32 / segs as f32;
                        let mut p = head.lerp(clew, t);
                        p.y += slack * (t * std::f32::consts::PI).sin();
                        p
                    })
                    .collect();
                ropes.push((leech, kz < 0.0));
            }
            ropes
        };
        let draw_rope = |pts: &[Vec2]| {
            for w2 in pts.windows(2) {
                draw_line(w2[0].x, w2[0].y, w2[1].x, w2[1].y, rope_thick, rope_col);
            }
        };
        // The rope(s) whose rig end lies abaft the mast plane, hidden by the spars.
        for (pts, behind) in &ropes {
            if *behind {
                draw_rope(pts);
            }
        }

        // --- Yard: a spar along the braced across-axis at the sail's head -------
        // Drawn over the panels so it crosses ahead of the cloth it carries.
        {
            let (lx, lz) = braced(-0.54, -stand_off);
            let (rx, rz) = braced(0.54, -stand_off);
            let th = h * 0.011;
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
            let bw = w * 0.016; // base half-width
            let tw = w * 0.010; // taper to the masthead
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

        // The remaining ropes, their rig ends riding forward of the mast plane,
        // drawn nearest so they lead over the spars toward the rail.
        for (pts, behind) in &ropes {
            if !*behind {
                draw_rope(pts);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rig(cargo: i32, speed: f32, yaw_rate: f32, slam: f32) -> RigInput<'static> {
        RigInput {
            motion: ShipMotion::default(),
            set: 1.0,
            turn: 0.0,
            wind_rel: 0.0,
            bow_lift: 0.0,
            cargo,
            speed,
            yaw_rate,
            slam,
            chart: None,
        }
    }

    fn step_for(r: &mut ShipRenderer, input: &RigInput, roll: f32, pitch: f32, secs: f32) {
        let dt = 1.0 / 60.0;
        let mut t = 0.0;
        while t < secs {
            r.step_cargo(input, roll, pitch, dt);
            t += dt;
        }
    }

    fn poses(r: &ShipRenderer) -> Vec<(f32, f32)> {
        r.crates.iter().map(|c| (c.x, c.z)).collect()
    }

    fn max_shift(a: &[(f32, f32)], b: &[(f32, f32)]) -> f32 {
        a.iter()
            .zip(b)
            .map(|(p, q)| ((p.0 - q.0).powi(2) + (p.1 - q.1).powi(2)).sqrt())
            .fold(0.0, f32::max)
    }

    /// Calibration probe for the cargo-physics constants: the deck motion a
    /// full storm actually produces, sailed hard into the sea. Run with
    /// --nocapture to read the numbers when retuning CARGO_TILT / SLAM_KICK.
    #[test]
    fn storm_calibration() {
        use crate::geometry::Vec2;
        use crate::ocean;
        let sea = 1.3f32;
        let pos = Vec2::new(1000.0, 2000.0);
        let heading = std::f32::consts::PI * 1.5; // driving into the dominant train
        let (mut max_roll, mut max_pitch, mut max_bow_vel) = (0.0f32, 0.0f32, 0.0f32);
        let dt = 1.0 / 60.0;
        let mut prev_bow = 0.0f32;
        for i in 0..(180 * 60) {
            let t = i as f32 * dt;
            let p = pos + Vec2::from_heading(heading) * (12.0 * t); // under way
            let m = ocean::ship_motion(p, heading, t, sea);
            let bow = ocean::height(p + Vec2::from_heading(heading) * ocean::BOW_REACH, t, sea)
                - m.heave;
            if i > 0 {
                max_bow_vel = max_bow_vel.max((prev_bow - bow) / dt);
            }
            prev_bow = bow;
            max_roll = max_roll.max(m.roll.abs());
            max_pitch = max_pitch.max(m.pitch.abs());
        }
        println!(
            "storm max |roll| {max_roll:.3} rad, max |pitch| {max_pitch:.3} rad, max bow drop {max_bow_vel:.2} m/s (slam {:.2})",
            max_bow_vel / 7.0
        );
        // The storm must at least rock the hull hard enough that the tuned
        // cargo constants have something to bite on.
        assert!(max_roll > 0.05 && max_bow_vel > 0.3);
    }

    /// A full storm's worst rolls and slams (magnitudes from the calibration
    /// probe above, deck-shared) must visibly shift cargo even on a straight
    /// course: lashings give in punches on the big seas.
    #[test]
    fn cargo_shifts_in_a_storm() {
        let mut r = ShipRenderer::new();
        let mut input = rig(24, 12.0, 0.0, 0.0);
        r.step_cargo(&input, 0.0, 0.0, 1.0 / 60.0);
        let before = poses(&r);
        let dt = 1.0 / 60.0;
        let mut t = 0.0f32;
        while t < 90.0 {
            // Deck-share of the measured storm roll/pitch, at swell periods,
            // with a slam pulse as the bow drives into a face every few
            // seconds (encounter rate at speed).
            let roll = 0.080 * (t * 0.9).sin();
            let pitch = 0.080 * (t * 0.7 + 1.0).sin();
            input.slam = if t % 4.0 < 0.5 { 0.35 } else { 0.0 };
            r.step_cargo(&input, roll, pitch, dt);
            t += dt;
        }
        let shift = max_shift(&before, &poses(&r));
        assert!(shift > 0.15, "a full storm shifted no cargo (max shift {shift})");
    }

    /// One crate per hold unit, and the same count lays out identically.
    #[test]
    fn crate_count_matches_cargo() {
        let mut r = ShipRenderer::new();
        r.step_cargo(&rig(24, 0.0, 0.0, 0.0), 0.0, 0.0, 1.0 / 60.0);
        assert_eq!(r.crates.len(), 24);
        let first = poses(&r);
        let mut r2 = ShipRenderer::new();
        r2.step_cargo(&rig(24, 0.0, 0.0, 0.0), 0.0, 0.0, 1.0 / 60.0);
        assert_eq!(first, poses(&r2));
        // A maxed hold still fits on deck.
        let mut r3 = ShipRenderer::new();
        r3.step_cargo(&rig(64, 0.0, 0.0, 0.0), 0.0, 0.0, 1.0 / 60.0);
        assert_eq!(r3.crates.len(), 64);
    }

    /// Ordinary sailing (a working deck angle, a leisurely turn) must not
    /// budge the cargo at all: the crates are heavy and lashed.
    #[test]
    fn cargo_holds_fast_in_ordinary_sailing() {
        let mut r = ShipRenderer::new();
        let input = rig(24, 8.0, 0.10, 0.1); // ~16 kn, half rudder, light chop
        r.step_cargo(&input, 0.0, 0.0, 1.0 / 60.0);
        let before = poses(&r);
        step_for(&mut r, &input, 0.10, 0.05, 10.0); // ~6° heel, ~3° pitch
        assert_eq!(before, poses(&r), "lashed cargo shifted in ordinary sailing");
    }

    /// A hard turn at a top-tier hull's full speed slings crates loose.
    #[test]
    fn cargo_slides_in_a_violent_turn() {
        let mut r = ShipRenderer::new();
        let input = rig(24, 20.0, crate::sailing::MAX_YAW_RATE, 0.0); // ~39 kn, wheel hard over
        r.step_cargo(&input, 0.0, 0.0, 1.0 / 60.0);
        let before = poses(&r);
        step_for(&mut r, &input, 0.15, 0.0, 6.0); // heeling hard through the turn
        let shift = max_shift(&before, &poses(&r));
        assert!(shift > 0.3, "no crate slid in a violent turn (max shift {shift})");
    }

    /// However hard the shaking, every crate stays inside the bulwarks, off
    /// the stairs, clear of the mast and on (or above) the deck.
    #[test]
    fn shaken_cargo_stays_on_deck() {
        let mut r = ShipRenderer::new();
        let mut input = rig(64, 20.0, crate::sailing::MAX_YAW_RATE, 0.8);
        r.step_cargo(&input, 0.0, 0.0, 1.0 / 60.0);
        // A storm's worth of violence, swinging both ways.
        for k in 0..8 {
            let side = if k % 2 == 0 { 1.0 } else { -1.0 };
            input.yaw_rate = crate::sailing::MAX_YAW_RATE * side;
            step_for(&mut r, &input, 0.25 * side, -0.15 * side, 2.0);
        }
        for c in &r.crates {
            if c.base.is_some() || c.over || c.gone {
                continue; // riding a stack (fenced through its base), or lost to the sea
            }
            let (b, d, _) = station_at(c.z);
            assert!(c.x.abs() + c.hw <= b + 1e-3, "crate through the bulwark");
            assert!(c.z + c.hd <= QDECK_BREAK + 1e-3, "crate through the riser");
            assert!(c.z - c.hd >= -6.5 - c.hd * 2.0, "crate off the bow");
            assert!(c.y >= d - 1e-3, "crate under the deck");
            if c.z + c.hd > QDECK_BREAK - 4.0 {
                assert!(c.x + c.hw <= 2.0 + 1e-3, "crate inside the stairs");
            }
        }
        // The books stay square: live crates plus the reported losses cover
        // every unit the hold started with.
        let washed = r.cargo_washed_overboard();
        let live = r.crates.iter().filter(|c| !c.gone).count() as i32;
        assert_eq!(live + washed, 64);
    }

    /// Keeping full way on through a storm with the wheel hard over is exactly
    /// the recklessness that puts cargo over the rail — but as a drip, one
    /// crate every few seconds, never the whole deck in one roll.
    #[test]
    fn reckless_storm_turn_washes_cargo_overboard() {
        let mut r = ShipRenderer::new();
        let mut input = rig(24, 20.0, crate::sailing::MAX_YAW_RATE, 0.0);
        r.step_cargo(&input, 0.0, 0.0, 1.0 / 60.0);
        let dt = 1.0 / 60.0;
        let mut t = 0.0f32;
        while t < 20.0 {
            let roll = 0.080 * (t * 0.9).sin();
            input.slam = if t % 4.0 < 0.5 { 0.35 } else { 0.0 };
            r.step_cargo(&input, roll, -0.02, dt);
            t += dt;
        }
        let washed = r.cargo_washed_overboard();
        assert!(washed > 0, "a reckless storm turn lost no cargo");
        assert!(washed < 24, "the whole deck emptied at once (lost {washed})");
        let live = r.crates.iter().filter(|c| !c.gone).count() as i32;
        assert_eq!(live + washed, 24);
        assert_eq!(r.stowed, 24 - washed);
        // With the loss collected, the same (reduced) hold count does not
        // re-stow the deck: the pile stays where the storm left it.
        input.cargo = 24 - washed;
        input.slam = 0.0;
        let live_before: Vec<bool> = r.crates.iter().map(|c| c.gone).collect();
        r.step_cargo(&input, 0.0, 0.0, dt);
        assert_eq!(live_before, r.crates.iter().map(|c| c.gone).collect::<Vec<bool>>());
    }

    /// Once the violence ends the pile settles: nothing keeps sliding, and
    /// the crew slowly haul shifted crates back toward their stowage.
    #[test]
    fn cargo_settles_and_restows_after_the_storm() {
        let mut r = ShipRenderer::new();
        let input = rig(24, 20.0, crate::sailing::MAX_YAW_RATE, 0.0);
        r.step_cargo(&input, 0.0, 0.0, 1.0 / 60.0);
        step_for(&mut r, &input, 0.2, 0.0, 5.0); // shift the pile
        let calm = rig(24, 5.0, 0.0, 0.0);
        step_for(&mut r, &calm, 0.0, 0.0, 5.0); // let it settle
        let settled = poses(&r);
        assert!(
            r.crates.iter().filter(|c| !c.gone).all(|c| !c.loose && !c.fall),
            "still sliding in a calm"
        );
        // Ten more calm seconds: everything eases toward home, nothing runs away.
        step_for(&mut r, &calm, 0.0, 0.0, 10.0);
        let dist_home = |p: &[(f32, f32)]| -> f32 {
            p.iter()
                .zip(&r.crates)
                .map(|(q, c)| ((q.0 - c.home.0).powi(2) + (q.1 - c.home.1).powi(2)).sqrt())
                .sum()
        };
        assert!(dist_home(&poses(&r)) <= dist_home(&settled) + 1e-3);
    }
}
