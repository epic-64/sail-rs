//! Per-island feature scatter, adapted from `shared.IsleFeatures`.
//!
//! The original placed billboard sprites (mountains, trees, huts…) to build each
//! island's silhouette. Here the island *body* is already a faceted mound (see
//! `islands_render`), so features are the scenery that dresses it: trees, bushes,
//! rocks, ruins and — on ports — docks, huts, a watchtower and flags. Each is a
//! deterministic function of the world seed and island, so a given chart always
//! grows the same flora. Features carry a world `offset` from the island centre
//! (so they fan out as you sail around — the parallax of the original) and an
//! absolute `height`; the renderer stands each on the mound's surface at its
//! offset.

use crate::geometry::Vec2;
use crate::isle_terrain::IsleTerrain;
use crate::rng::Rng;
use crate::world::{Island, IsleKind};
use std::f64::consts::TAU;

const GOLDEN: i64 = 0x9e3779b97f4a7c15u64 as i64;

/// Scenery-density levels offered by the performance setting (pause menu Options),
/// and the multiplier each applies to every scatter count. The scatters are tuned
/// at index 2 (`Medium` = ×1.0); lower levels thin the foliage to claw back frame
/// time on weak hardware, higher levels pack it in. `Very Low` lands near the
/// game's original sparse scatter (≈1/5 of Medium).
pub const DENSITY_LEVELS: usize = 5;

/// The on-screen name of a density level (`0..DENSITY_LEVELS`).
pub fn density_label(level: usize) -> &'static str {
    match level {
        0 => "Very Low",
        1 => "Low",
        2 => "Medium",
        3 => "High",
        _ => "Very High",
    }
}

/// The scatter-count multiplier for a density level (`0..DENSITY_LEVELS`).
pub fn density_mul(level: usize) -> f32 {
    match level {
        0 => 0.2,
        1 => 0.5,
        2 => 1.0,
        3 => 1.6,
        _ => 2.4,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeatureKind {
    Tree,
    Palm,
    Pine,
    Fern,
    Bush,
    Rock,
    Ruin,
    Hut,
    Cottage,
    Tower,
    Dock,
    Flag,
    Shipwreck,
    // Foliage and ground cover.
    DeadTree,
    FlowerPatch,
    Reeds,
    Cactus,
    FallenLog,
    // Stone: piled, standing, and molten.
    Cairn,
    StoneArch,
    LavaVent,
    // Human traces beyond the harbour.
    Campfire,
    Windmill,
    Lighthouse,
}

/// One element of an island, placed at a world `offset` (m) from the centre.
/// `height` is the feature's own height (m) above the ground it stands on; `size`
/// is a horizontal width multiplier.
#[derive(Clone, Copy, Debug)]
pub struct IsleFeature {
    pub kind: FeatureKind,
    pub offset: Vec2,
    pub height: f32,
    pub size: f32,
}

/// All features for one island, deterministic from the world seed + island id.
/// `density` is the scenery multiplier from the performance setting (see
/// [`density_mul`]); 1.0 is the tuned baseline.
pub fn generate(seed: i64, isle: &Island, density: f32) -> Vec<IsleFeature> {
    let mut rng = Rng::from_seed(seed ^ (isle.id as i64 + 1).wrapping_mul(GOLDEN));
    let mut feats = match isle.terrain {
        IsleKind::Green => green(&mut rng, isle, density),
        IsleKind::Jungle => jungle(&mut rng, isle, density),
        IsleKind::Rocky => rocky(&mut rng, isle, density),
        IsleKind::Volcanic => volcanic(&mut rng, isle, density),
    };
    if isle.is_port {
        let mut prng = Rng::from_seed(
            (seed.wrapping_mul(0x100000001b3) ^ (isle.id as i64 + 1)) ^ 0x504f5254,
        );
        feats.extend(port_structures(&mut prng, isle, density));
    }
    // Cull anything that fell in the water. With the lobed coastline a scatter point
    // can land in a bay or out past a headland; a tree or house there would float on
    // the sea beside the visible land. Keep only features on dry land (the same
    // coastline the renderer draws and collision grounds against). Shore dwellers are
    // exempt: a dock reaches out over the water and a wreck lies canted in the
    // shallows, so both belong at the waterline.
    let terrain = IsleTerrain::for_island(isle);
    feats.retain(|f| {
        matches!(f.kind, FeatureKind::Dock | FeatureKind::Shipwreck)
            || terrain.on_land(isle.pos + f.offset, SHORE_INSET)
    });
    feats
}

/// How far inside the shoreline (m) a placed feature must sit, so foliage and
/// buildings stand clear on dry land rather than awash at the very waterline.
const SHORE_INSET: f32 = 6.0;

// --- per-terrain scatters ----------------------------------------------------

fn green(rng: &mut Rng, isle: &Island, d: f32) -> Vec<IsleFeature> {
    use FeatureKind::*;
    let r = isle.radius;
    let n = scaled(rng, 60, 100, d);
    let mut v = scatter(rng, n, r * 0.82, &[Tree, Palm, Tree, Pine], 6.0, 11.0, 0.8, 1.3);
    let n = scaled(rng, 40, 70, d);
    v.extend(scatter(rng, n, r * 0.86, &[Bush, Fern, Bush], 2.0, 3.5, 0.8, 1.4));
    let n = scaled(rng, 0, 20, d);
    v.extend(scatter(rng, n, r * 0.78, &[Rock], 3.0, 6.0, 0.7, 1.1));
    let n = scaled(rng, 0, 10, d);
    v.extend(scatter(rng, n, r * 0.6, &[Ruin], 4.0, 7.0, 0.8, 1.2));
    // Wildflower meadows and the odd weathered snag.
    let n = scaled(rng, 15, 35, d);
    v.extend(scatter(rng, n, r * 0.8, &[FlowerPatch, FlowerPatch, DeadTree], 1.5, 4.0, 0.9, 1.4));
    // Reeds at the water's edge.
    let n = scaled(rng, 8, 20, d);
    v.extend(scatter(rng, n, r * 0.9, &[Reeds], 2.0, 3.5, 0.8, 1.3));
    // A lone landmark: a standing arch or an old windmill.
    maybe_one(rng, isle, &[StoneArch, Windmill], 0.4, 12.0, 18.0, 0.55, &mut v);
    maybe_one(rng, isle, &[Campfire], 0.2, 2.5, 4.0, 0.78, &mut v);
    maybe_shore(rng, isle, Shipwreck, 0.25, 3.0, 5.0, &mut v);
    v
}

fn jungle(rng: &mut Rng, isle: &Island, d: f32) -> Vec<IsleFeature> {
    use FeatureKind::*;
    let r = isle.radius;
    let n = scaled(rng, 90, 140, d);
    let mut v = scatter(rng, n, r * 0.86, &[Tree, Palm, Tree, Bush, Pine], 7.0, 12.0, 0.8, 1.3);
    let n = scaled(rng, 60, 90, d);
    v.extend(scatter(rng, n, r * 0.88, &[Bush, Fern, Fern], 2.0, 4.0, 0.9, 1.4));
    let n = scaled(rng, 5, 20, d);
    v.extend(scatter(rng, n, r * 0.6, &[Ruin], 4.0, 7.0, 0.9, 1.3));
    // Fallen logs on the jungle floor; reeds and blooms in the clearings.
    let n = scaled(rng, 10, 25, d);
    v.extend(scatter(rng, n, r * 0.85, &[FallenLog, Fern, Bush], 2.0, 4.0, 0.9, 1.4));
    let n = scaled(rng, 8, 18, d);
    v.extend(scatter(rng, n, r * 0.9, &[Reeds, Reeds, FlowerPatch], 2.0, 3.5, 0.8, 1.3));
    maybe_one(rng, isle, &[StoneArch], 0.5, 12.0, 18.0, 0.5, &mut v);
    maybe_one(rng, isle, &[Campfire], 0.2, 2.5, 4.0, 0.78, &mut v);
    maybe_shore(rng, isle, Shipwreck, 0.35, 3.0, 5.0, &mut v);
    v
}

fn rocky(rng: &mut Rng, isle: &Island, d: f32) -> Vec<IsleFeature> {
    use FeatureKind::*;
    let r = isle.radius;
    let n = scaled(rng, 40, 70, d);
    let mut v = scatter(rng, n, r * 0.78, &[Rock, Rock, Pine], 4.0, 9.0, 0.7, 1.3);
    let n = scaled(rng, 10, 30, d);
    v.extend(scatter(rng, n, r * 0.74, &[Pine, Tree, Bush], 5.0, 8.0, 0.6, 0.9));
    // Stone cairns, dead snags and the hardy cactus of a barren isle.
    let n = scaled(rng, 8, 20, d);
    v.extend(scatter(rng, n, r * 0.72, &[Cairn, Rock, DeadTree], 3.0, 6.0, 0.8, 1.2));
    let n = scaled(rng, 3, 10, d);
    v.extend(scatter(rng, n, r * 0.7, &[Cactus, DeadTree], 3.0, 6.0, 0.7, 1.0));
    maybe_one(rng, isle, &[StoneArch, Cairn], 0.4, 8.0, 14.0, 0.5, &mut v);
    maybe_one(rng, isle, &[Campfire], 0.18, 2.5, 4.0, 0.78, &mut v);
    maybe_shore(rng, isle, Shipwreck, 0.3, 3.0, 5.0, &mut v);
    v
}

fn volcanic(rng: &mut Rng, isle: &Island, d: f32) -> Vec<IsleFeature> {
    use FeatureKind::*;
    let r = isle.radius;
    let n = scaled(rng, 30, 60, d);
    let mut v = scatter(rng, n, r * 0.74, &[Rock, Rock, Pine], 4.0, 9.0, 0.7, 1.3);
    let n = scaled(rng, 5, 20, d);
    v.extend(scatter(rng, n, r * 0.74, &[Pine, Bush], 4.0, 7.0, 0.5, 0.8));
    // Glowing lava vents, cinder cairns and a scatter of hardy cactus.
    let n = scaled(rng, 6, 16, d);
    v.extend(scatter(rng, n, r * 0.6, &[LavaVent, Rock, DeadTree], 2.5, 5.0, 0.8, 1.2));
    let n = scaled(rng, 2, 8, d);
    v.extend(scatter(rng, n, r * 0.68, &[Cactus, Cairn], 3.0, 5.0, 0.6, 0.9));
    maybe_one(rng, isle, &[LavaVent], 0.6, 4.0, 7.0, 0.3, &mut v);
    v
}

// --- port settlement ---------------------------------------------------------

fn port_structures(rng: &mut Rng, isle: &Island, d: f32) -> Vec<IsleFeature> {
    use FeatureKind::*;
    let r = isle.radius;
    let n = scaled(rng, 25, 45, d);
    let mut v = scatter(rng, n, r * 0.55, &[Hut, Hut, Cottage], 4.0, 6.0, 0.9, 1.2);
    // A watchtower set back from the water.
    let ang = rng.between(0.0, TAU);
    let rad = r as f64 * 0.42;
    v.push(IsleFeature {
        kind: Tower,
        offset: Vec2::new((ang.sin() * rad) as f32, (ang.cos() * rad) as f32),
        height: rng.between(9.0, 14.0) as f32,
        size: rng.between(0.8, 1.1) as f32,
    });
    let n = scaled(rng, 5, 15, d);
    v.extend(scatter(rng, n, r * 0.5, &[Flag], 4.0, 6.0, 0.8, 1.1));
    // A shoreline dock.
    let ang = rng.between(0.0, TAU);
    let rad = r as f64 * 0.85;
    v.push(IsleFeature {
        kind: Dock,
        offset: Vec2::new((ang.sin() * rad) as f32, (ang.cos() * rad) as f32),
        height: 2.5,
        size: 1.4,
    });
    // A lighthouse on a headland, its lantern lit after dusk.
    let ang = rng.between(0.0, TAU);
    let rad = r as f64 * 0.8;
    v.push(IsleFeature {
        kind: Lighthouse,
        offset: Vec2::new((ang.sin() * rad) as f32, (ang.cos() * rad) as f32),
        height: rng.between(14.0, 20.0) as f32,
        size: rng.between(0.9, 1.1) as f32,
    });
    v
}

// --- placement helpers -------------------------------------------------------

/// Draw a scatter count in `[lo, hi]`, then scale it by the scenery-density setting
/// (rounding, floored at 0). The base draw always happens, so the per-island RNG
/// stream stays a function of seed + density alone (same inputs, same island).
fn scaled(rng: &mut Rng, lo: i32, hi: i32, density: f32) -> i32 {
    ((rng.int_between(lo, hi) as f32 * density).round() as i32).max(0)
}

#[allow(clippy::too_many_arguments)] // scatter parameters (bounds + density knobs)
fn scatter(
    rng: &mut Rng,
    count: i32,
    max_r: f32,
    kinds: &[FeatureKind],
    h_lo: f64,
    h_hi: f64,
    s_lo: f64,
    s_hi: f64,
) -> Vec<IsleFeature> {
    let mut out = Vec::new();
    for _ in 0..count {
        let ang = rng.between(0.0, TAU);
        let rad = rng.between(max_r as f64 * 0.15, max_r as f64);
        let h = rng.between(h_lo, h_hi) as f32;
        let size = rng.between(s_lo, s_hi) as f32;
        let kind = *rng.pick(kinds);
        out.push(IsleFeature {
            kind,
            offset: Vec2::new((ang.sin() * rad) as f32, (ang.cos() * rad) as f32),
            height: h,
            size,
        });
    }
    out
}

/// With probability `prob`, place a single landmark (one kind picked from `kinds`)
/// at `rad_frac` of the island radius, e.g. a standing arch or a windmill. Like
/// [`maybe_shore`] it always draws the roll/angle/height/kind/size so the RNG stream
/// stays a pure function of the inputs whether or not the landmark lands.
#[allow(clippy::too_many_arguments)]
fn maybe_one(
    rng: &mut Rng,
    isle: &Island,
    kinds: &[FeatureKind],
    prob: f64,
    h_lo: f64,
    h_hi: f64,
    rad_frac: f64,
    out: &mut Vec<IsleFeature>,
) {
    let roll = rng.next_f64();
    let ang = rng.between(0.0, TAU);
    let h = rng.between(h_lo, h_hi) as f32;
    let size = rng.between(0.9, 1.2) as f32;
    let kind = *rng.pick(kinds);
    if roll < prob {
        let rad = isle.radius as f64 * rad_frac;
        out.push(IsleFeature {
            kind,
            offset: Vec2::new((ang.sin() * rad) as f32, (ang.cos() * rad) as f32),
            height: h,
            size,
        });
    }
}

/// With probability `prob`, place one feature out on the shoreline (e.g. a wreck).
fn maybe_shore(
    rng: &mut Rng,
    isle: &Island,
    kind: FeatureKind,
    prob: f64,
    h_lo: f64,
    h_hi: f64,
    out: &mut Vec<IsleFeature>,
) {
    let roll = rng.next_f64();
    let ang = rng.between(0.0, TAU);
    let h = rng.between(h_lo, h_hi) as f32;
    if roll < prob {
        let rad = isle.radius as f64 * 0.9;
        out.push(IsleFeature {
            kind,
            offset: Vec2::new((ang.sin() * rad) as f32, (ang.cos() * rad) as f32),
            height: h,
            size: 1.2,
        });
    }
}
