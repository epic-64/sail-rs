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
use crate::rng::Rng;
use crate::world::{Island, IsleKind};
use std::f64::consts::TAU;

const GOLDEN: i64 = 0x9e3779b97f4a7c15u64 as i64;

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
pub fn generate(seed: i64, isle: &Island) -> Vec<IsleFeature> {
    let mut rng = Rng::from_seed(seed ^ (isle.id as i64 + 1).wrapping_mul(GOLDEN));
    let mut feats = match isle.terrain {
        IsleKind::Green => green(&mut rng, isle),
        IsleKind::Jungle => jungle(&mut rng, isle),
        IsleKind::Rocky => rocky(&mut rng, isle),
        IsleKind::Volcanic => volcanic(&mut rng, isle),
    };
    if isle.is_port {
        let mut prng = Rng::from_seed(
            (seed.wrapping_mul(0x100000001b3) ^ (isle.id as i64 + 1)) ^ 0x504f5254,
        );
        feats.extend(port_structures(&mut prng, isle));
    }
    feats
}

// --- per-terrain scatters ----------------------------------------------------

fn green(rng: &mut Rng, isle: &Island) -> Vec<IsleFeature> {
    use FeatureKind::*;
    let r = isle.radius;
    let n = rng.int_between(12, 20);
    let mut v = scatter(rng, n, r * 0.82, &[Tree, Palm, Tree, Pine], 6.0, 11.0, 0.8, 1.3);
    let n = rng.int_between(8, 14);
    v.extend(scatter(rng, n, r * 0.86, &[Bush, Fern, Bush], 2.0, 3.5, 0.8, 1.4));
    let n = rng.int_between(0, 4);
    v.extend(scatter(rng, n, r * 0.78, &[Rock], 3.0, 6.0, 0.7, 1.1));
    let n = rng.int_between(0, 2);
    v.extend(scatter(rng, n, r * 0.6, &[Ruin], 4.0, 7.0, 0.8, 1.2));
    maybe_shore(rng, isle, Shipwreck, 0.25, 3.0, 5.0, &mut v);
    v
}

fn jungle(rng: &mut Rng, isle: &Island) -> Vec<IsleFeature> {
    use FeatureKind::*;
    let r = isle.radius;
    let n = rng.int_between(18, 28);
    let mut v = scatter(rng, n, r * 0.86, &[Tree, Palm, Tree, Bush, Pine], 7.0, 12.0, 0.8, 1.3);
    let n = rng.int_between(12, 18);
    v.extend(scatter(rng, n, r * 0.88, &[Bush, Fern, Fern], 2.0, 4.0, 0.9, 1.4));
    let n = rng.int_between(1, 4);
    v.extend(scatter(rng, n, r * 0.6, &[Ruin], 4.0, 7.0, 0.9, 1.3));
    maybe_shore(rng, isle, Shipwreck, 0.35, 3.0, 5.0, &mut v);
    v
}

fn rocky(rng: &mut Rng, isle: &Island) -> Vec<IsleFeature> {
    use FeatureKind::*;
    let r = isle.radius;
    let n = rng.int_between(8, 14);
    let mut v = scatter(rng, n, r * 0.78, &[Rock, Rock, Pine], 4.0, 9.0, 0.7, 1.3);
    let n = rng.int_between(2, 6);
    v.extend(scatter(rng, n, r * 0.74, &[Pine, Tree, Bush], 5.0, 8.0, 0.6, 0.9));
    maybe_shore(rng, isle, Shipwreck, 0.3, 3.0, 5.0, &mut v);
    v
}

fn volcanic(rng: &mut Rng, isle: &Island) -> Vec<IsleFeature> {
    use FeatureKind::*;
    let r = isle.radius;
    let n = rng.int_between(6, 12);
    let mut v = scatter(rng, n, r * 0.74, &[Rock, Rock, Pine], 4.0, 9.0, 0.7, 1.3);
    let n = rng.int_between(1, 4);
    v.extend(scatter(rng, n, r * 0.74, &[Pine, Bush], 4.0, 7.0, 0.5, 0.8));
    v
}

// --- port settlement ---------------------------------------------------------

fn port_structures(rng: &mut Rng, isle: &Island) -> Vec<IsleFeature> {
    use FeatureKind::*;
    let r = isle.radius;
    let n = rng.int_between(5, 9);
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
    let n = rng.int_between(1, 3);
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
    v
}

// --- placement helpers -------------------------------------------------------

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
