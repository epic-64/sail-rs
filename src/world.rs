//! Procedural world generation, ported from `shared.World` / `shared.WorldGen`.
//!
//! The world is many local archipelagos (clusters) scattered far apart across
//! open sea. We reproduce the exact `Rng` draw order so a given seed rebuilds the
//! same chart the Scala game did. Generation math is in f64 (like Scala `Double`)
//! and positions are stored as our f32 `Vec2`.

use crate::geometry::Vec2;
use crate::rng::Rng;

/// The terrain archetype of a landmass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IsleKind {
    Green,
    Rocky,
    Jungle,
    Volcanic,
}

/// A single landmass on the chart.
#[derive(Clone, Debug)]
pub struct Island {
    pub id: i32,
    pub name: String,
    pub pos: Vec2,    // metres on the chart
    pub radius: f32,  // metres (shore radius)
    pub height: f32,  // metres (summit above sea level)
    pub terrain: IsleKind,
    pub is_port: bool,
    pub is_shipyard: bool,
}

impl Island {
    /// How close (m) the ship must come to dock.
    pub fn dock_range(&self) -> f32 {
        self.radius + 250.0
    }
}

/// A local archipelago: a tight knot of isles separated from its neighbours by
/// many miles of open sea.
#[derive(Clone, Debug)]
pub struct Cluster {
    pub id: i32,
    pub name: String,
    pub center: Vec2,
    pub island_ids: Vec<i32>,
}

/// The whole procedurally generated world.
pub struct World {
    pub seed: i64,
    pub islands: Vec<Island>,
    pub clusters: Vec<Cluster>,
}

impl World {
    /// The cluster whose centre is nearest — the local waters the ship is in.
    pub fn cluster_at(&self, p: Vec2) -> &Cluster {
        self.clusters
            .iter()
            .min_by(|a, b| {
                a.center
                    .distance_to(p)
                    .partial_cmp(&b.center.distance_to(p))
                    .unwrap()
            })
            .expect("world always has at least the centre cluster")
    }

    /// Every island within `range` metres of `p` (for rendering / collision).
    pub fn islands_near(&self, p: Vec2, range: f32) -> Vec<&Island> {
        self.islands
            .iter()
            .filter(|i| i.pos.distance_to(p) <= range + i.radius)
            .collect()
    }

    /// The isles belonging to a cluster, in id order. `islands` is in id order
    /// (index == id), so each id indexes straight in. (`World.clusterIslands`.)
    pub fn cluster_islands(&self, c: &Cluster) -> Vec<&Island> {
        c.island_ids
            .iter()
            .map(|&id| &self.islands[id as usize])
            .collect()
    }

    /// How far `p` lies past the edge of the nearest archipelago, in metres (negative
    /// while still within the isles' bounding square). The "high sea" the weather
    /// leans stormier on begins a kilometre or two beyond this edge (see `weather`).
    pub fn dist_outside_archipelago(&self, p: Vec2) -> f32 {
        let (centre, half) = self.cluster_bounds(self.cluster_at(p));
        centre.distance_to(p) - half
    }

    /// Tight square framing of a cluster's isles: the centre of their bounding box
    /// and the half-width of its larger axis. Lets a chart fill the frame with the
    /// isles themselves rather than the cluster's generous (corner-safe) radius.
    /// (`World.clusterBounds`.)
    pub fn cluster_bounds(&self, c: &Cluster) -> (Vec2, f32) {
        let isles = self.cluster_islands(c);
        let mut min_x = f32::MAX;
        let mut max_x = f32::MIN;
        let mut min_y = f32::MAX;
        let mut max_y = f32::MIN;
        for i in &isles {
            min_x = min_x.min(i.pos.x);
            max_x = max_x.max(i.pos.x);
            min_y = min_y.min(i.pos.y);
            max_y = max_y.max(i.pos.y);
        }
        let centre = Vec2::new((min_x + max_x) / 2.0, (min_y + max_y) / 2.0);
        let half = ((max_x - min_x).max(max_y - min_y)) / 2.0;
        (centre, half)
    }
}

// --- WorldGen constants (match Scala `WorldGen`) -----------------------------

const EXTENT: f64 = 10400.0;
const GRID_COLS: usize = 5;
const GRID_ROWS: usize = 5;
const ISLES_PER_CLUSTER: i32 = (GRID_COLS * GRID_ROWS) as i32;
const CLUSTER_COLS: usize = 4;
const CLUSTER_ROWS: usize = 3;
/// The world is laid out 16:9 (wider than tall) to match the captain's-log map, so the
/// cluster grid steps wider horizontally than vertically: the two spacings are sized so
/// the grid's whole bounding box ((cols-1) by (rows-1) steps) keeps that ratio.
const WORLD_ASPECT: f64 = 16.0 / 9.0;
const CLUSTER_SPACING_X: f64 = 21000.0;
const CLUSTER_SPACING_Y: f64 =
    CLUSTER_SPACING_X * (CLUSTER_COLS - 1) as f64 / ((CLUSTER_ROWS - 1) as f64 * WORLD_ASPECT);
/// Exactly how many archipelagos the world holds: the centre cell plus the
/// highest-rolling others on the cluster grid (so the world stays small and the
/// count is fixed rather than a fill probability).
const NUM_CLUSTERS: usize = 7;
const CLUSTER_RADIUS: f64 = EXTENT * 0.7;
const MIN_CLUSTER_GAP: f64 = CLUSTER_RADIUS * 2.2;

const PREFIXES: [&str; 6] = ["Port", "Isla", "Cape", "Saint", "Cabo", "Fort"];
const STARTS: [&str; 10] = [
    "Tor", "Cala", "Bar", "Mor", "Pala", "Cor", "Nag", "Vel", "Dra", "Mar",
];
const MIDDLES: [&str; 10] = ["tu", "ve", "ba", "que", "ri", "lo", "san", "ga", "do", "mi"];
const ENDS: [&str; 10] = [
    "ga", "vera", "moor", "nada", "cay", "reef", "rock", "haven", "doza", "sol",
];
const CLUSTER_ADJ: [&str; 16] = [
    "Coral", "Saltspit", "Tempest", "Verdant", "Ashen", "Sapphire", "Broken", "Mistral", "Sunken",
    "Golden", "Iron", "Pearl", "Storm", "Drowned", "Amber", "Obsidian",
];
const CLUSTER_NOUN: [&str; 10] = [
    "Reaches", "Shoals", "Expanse", "Straits", "Atolls", "Banks", "Sound", "Chain", "Narrows",
    "Deeps",
];
const TERRAINS: [IsleKind; 5] = [
    IsleKind::Green,
    IsleKind::Green,
    IsleKind::Rocky,
    IsleKind::Jungle,
    IsleKind::Volcanic,
];

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

pub fn generate(seed: i64) -> World {
    let mut rng0 = Rng::from_seed(seed);
    // for r <- 0 until rows; c <- 0 until cols  (r-major)
    let mut full_grid: Vec<(usize, usize)> = Vec::new();
    for r in 0..CLUSTER_ROWS {
        for c in 0..CLUSTER_COLS {
            full_grid.push((r, c));
        }
    }
    let centre_cell = (CLUSTER_ROWS / 2, CLUSTER_COLS / 2);
    let origin_x = -((CLUSTER_COLS - 1) as f64) * CLUSTER_SPACING_X / 2.0;
    let origin_y = -((CLUSTER_ROWS - 1) as f64) * CLUSTER_SPACING_Y / 2.0;

    // Roll one key per cell (keeping the per-cell draw so the RNG sequence stays put),
    // then keep exactly NUM_CLUSTERS archipelagos. Three placements are guaranteed so the
    // world is never lopsided: the centre cell, plus the highest-rolling cell in the
    // far-left and far-right columns (so an archipelago always sits at each end and the
    // world spans the grid's full width). The remaining slots go to the highest-rolling
    // cells left. Kept cells stay in grid (r-major) order so island ids run in reading order.
    let rolls: Vec<f64> = full_grid.iter().map(|_| rng0.next_f64()).collect();
    let want = NUM_CLUSTERS.min(full_grid.len());
    let last_col = CLUSTER_COLS - 1;
    let best = |pred: &dyn Fn((usize, usize)) -> bool, keep_set: &[bool]| -> Option<usize> {
        (0..full_grid.len())
            .filter(|&i| !keep_set[i] && pred(full_grid[i]))
            .max_by(|&a, &b| rolls[a].partial_cmp(&rolls[b]).unwrap())
    };

    let mut keep_set = vec![false; full_grid.len()];
    let mut kept = 0usize;
    for sel in [
        best(&|c| c == centre_cell, &keep_set),
        best(&|c| c.1 == 0, &keep_set),
        best(&|c| c.1 == last_col, &keep_set),
    ] {
        if let Some(i) = sel {
            if !keep_set[i] {
                keep_set[i] = true;
                kept += 1;
            }
        }
    }
    // Fill the remaining slots with the highest-rolling cells left.
    let mut rest: Vec<usize> = (0..full_grid.len()).filter(|&i| !keep_set[i]).collect();
    rest.sort_by(|&a, &b| rolls[b].partial_cmp(&rolls[a]).unwrap());
    for &i in &rest {
        if kept >= want {
            break;
        }
        keep_set[i] = true;
        kept += 1;
    }
    let grid: Vec<(usize, usize)> = full_grid
        .iter()
        .enumerate()
        .filter(|(i, _)| keep_set[*i])
        .map(|(_, c)| *c)
        .collect();

    let mut clusters: Vec<Cluster> = Vec::new();
    let mut islands: Vec<Island> = Vec::new();
    for (c_idx, (r, c)) in grid.iter().enumerate() {
        let c_idx = c_idx as i32;
        // Roll 16 candidate spots; take the first clear of every cluster so far,
        // else the roomiest.
        let mut cands: Vec<Vec2> = Vec::new();
        for _ in 0..16 {
            let jx = rng0.between(-0.35, 0.35);
            let jy = rng0.between(-0.35, 0.35);
            let p = Vec2::new(
                (origin_x + (*c as f64 + jx) * CLUSTER_SPACING_X) as f32,
                (origin_y + (*r as f64 + jy) * CLUSTER_SPACING_Y) as f32,
            );
            cands.push(p);
        }
        let gap = |p: Vec2, cs: &[Cluster]| -> f32 {
            cs.iter()
                .map(|cl| cl.center.distance_to(p))
                .fold(f32::MAX, f32::min)
        };
        let center = cands
            .iter()
            .find(|p| gap(**p, &clusters) >= MIN_CLUSTER_GAP as f32)
            .copied()
            .unwrap_or_else(|| {
                *cands
                    .iter()
                    .max_by(|a, b| {
                        gap(**a, &clusters)
                            .partial_cmp(&gap(**b, &clusters))
                            .unwrap()
                    })
                    .unwrap()
            });
        let cname = cluster_name(&mut rng0);
        let id_offset = c_idx * ISLES_PER_CLUSTER;
        let cluster_seed = seed ^ ((c_idx as i64 + 1).wrapping_mul(0x9e3779b97f4a7c15u64 as i64));
        let isles = generate_cluster(cluster_seed, center, id_offset);
        let island_ids: Vec<i32> = isles.iter().map(|i| i.id).collect();
        clusters.push(Cluster {
            id: c_idx,
            name: cname,
            center,
            island_ids,
        });
        islands.extend(isles);
    }

    World {
        seed,
        islands,
        clusters,
    }
}

/// Build one local archipelago: a jittered 5×5 grid of isles around `center`.
fn generate_cluster(seed: i64, center: Vec2, id_offset: i32) -> Vec<Island> {
    let mut rng = Rng::from_seed(seed);
    let cell_w = EXTENT / GRID_COLS as f64;
    let cell_h = EXTENT / GRID_ROWS as f64;

    let mut built: Vec<Island> = Vec::new();
    let mut idx: i32 = 0;
    for r in 0..GRID_ROWS {
        for c in 0..GRID_COLS {
            let jx = rng.between(0.15, 0.85);
            let jy = rng.between(0.15, 0.85);
            let radius = rng.between(140.0, 235.0);
            let relief = rng.between(0.28, 0.5);
            let terrain = *rng.pick(&TERRAINS);
            let port_roll = rng.next_f64();
            let is_port = idx <= 1 || port_roll < 0.45;
            let name = island_name(&mut rng);
            let pos = Vec2::new(
                center.x + ((c as f64 + jx) * cell_w - EXTENT / 2.0) as f32,
                center.y + ((r as f64 + jy) * cell_h - EXTENT / 2.0) as f32,
            );
            // Summit elevation by terrain: green & jungle shores lie almost flat on
            // the water (low beaches/atolls), while rocky and volcanic isles rear up
            // into proper hills and cones.
            // Islands are relatively flat by default: green/jungle are low cays a few
            // metres proud of the water; rocky and volcanic rear up a little more into
            // modest hills and cones, but nothing towering.
            let height = match terrain {
                IsleKind::Green | IsleKind::Jungle => 5.0 + radius * relief * 0.10,
                IsleKind::Rocky => 14.0 + radius * relief * 0.35,
                IsleKind::Volcanic => 20.0 + radius * relief * 0.40,
            };
            built.push(Island {
                id: id_offset + idx,
                name,
                pos,
                radius: radius as f32,
                height: height as f32,
                terrain,
                is_port,
                is_shipyard: false,
            });
            idx += 1;
        }
    }

    // One shipyard per cluster: its first port.
    if let Some(yard) = built.iter().position(|i| i.is_port) {
        built[yard].is_shipyard = true;
    }
    built
}

fn cluster_name(rng: &mut Rng) -> String {
    let a = *rng.pick(&CLUSTER_ADJ);
    let n = *rng.pick(&CLUSTER_NOUN);
    format!("{a} {n}")
}

fn island_name(rng: &mut Rng) -> String {
    let pre = *rng.pick(&PREFIXES);
    let s = *rng.pick(&STARTS);
    let m = *rng.pick(&MIDDLES);
    let e = *rng.pick(&ENDS);
    let drop_mid = rng.next_f64();
    let core = if drop_mid < 0.4 {
        format!("{s}{e}")
    } else {
        format!("{s}{m}{e}")
    };
    format!("{pre} {}", capitalize(&core))
}
