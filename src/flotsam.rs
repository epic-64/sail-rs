//! Floating salvage adrift on the open sea, ported from `shared.Flotsam`.
//!
//! A crate, barrel or half-sunk strongbox bobs on the swell; the captain scoops
//! it up simply by sailing over it (see [`FlotsamField::collect_near`]), and the
//! gold it yields lands in the purse like any other voyage outcome. Salvage gives
//! the player something to chase on a long passage between ports instead of
//! staring at empty water.
//!
//! The field is **per-frame**, never persisted in [`GameState`](crate::game_state)
//! — only the gold a pickup yields is. It carries its own [`Rng`] so topping it up
//! stays deterministic from a seed: the same voyage always strews the same crates.
//! As in the original, the *pure* generation and collection live here; `main`
//! drives them each frame and the billboards are drawn by [`crate::flotsam_render`].

use std::f32::consts::{FRAC_PI_2, PI};

use crate::geometry::{wrap_angle, Vec2};
use crate::rng::Rng;
use crate::world::World;

/// The kinds of salvage that bob on the waves, each worth some gold when hauled
/// aboard. `weight` is how often the kind turns up: common crates, the odd
/// barrel, a rare strongbox. (`shared.FlotsamKind`.)
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FlotsamKind {
    Crate,
    Barrel,
    Chest,
}

impl FlotsamKind {
    /// In the Scala `enum`'s declaration order (the draw order [`pick`] walks).
    pub const ALL: [FlotsamKind; 3] = [FlotsamKind::Crate, FlotsamKind::Barrel, FlotsamKind::Chest];

    pub fn label(self) -> &'static str {
        match self {
            FlotsamKind::Crate => "Crate",
            FlotsamKind::Barrel => "Barrel",
            FlotsamKind::Chest => "Chest",
        }
    }

    /// The gold this kind is worth when scooped aboard.
    pub fn gold(self) -> i32 {
        match self {
            FlotsamKind::Crate => 12,
            FlotsamKind::Barrel => 28,
            FlotsamKind::Chest => 75,
        }
    }

    /// How often this kind turns up, as a relative weight (sums to 1.0).
    pub fn weight(self) -> f64 {
        match self {
            FlotsamKind::Crate => 0.56,
            FlotsamKind::Barrel => 0.32,
            FlotsamKind::Chest => 0.12,
        }
    }

    /// Draw a kind at random, weighted toward the common salvage. Advances `rng`
    /// by exactly one draw (so the draw order matches the Scala original).
    pub fn pick(rng: &mut Rng) -> FlotsamKind {
        let total: f64 = Self::ALL.iter().map(|k| k.weight()).sum();
        let mut rem = rng.between(0.0, total);
        for (i, k) in Self::ALL.iter().enumerate() {
            if i == Self::ALL.len() - 1 || rem < k.weight() {
                return *k;
            }
            rem -= k.weight();
        }
        Self::ALL[0]
    }
}

/// A floating collectible adrift on the open sea. (`shared.Flotsam`.)
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Flotsam {
    pub id: i32,
    pub pos: Vec2,
    pub kind: FlotsamKind,
}

/// What a sweep for salvage turned up: the gold scooped aboard and the items
/// lifted (so the renderer can flash a pickup). The field itself is mutated in
/// place by [`FlotsamField::collect_near`]. (`shared.Haul`.)
#[derive(Clone, Debug, Default)]
pub struct Haul {
    pub gold: i32,
    pub picked: Vec<Flotsam>,
}

/// How many pieces of salvage drift within reach of the ship at once.
pub const TARGET: usize = 3;
/// Nearest a fresh piece spawns (m) — well clear of the bow so a replacement
/// never winks into existence right on top of the player, and far enough that a
/// new piece reads as something to steer toward rather than a freebie underfoot.
pub const MIN_SPAWN: f32 = 400.0;
/// Farthest a fresh piece spawns (m) — kept inside the renderer's full-opacity
/// range (`flotsam_render::FADE_NEAR`) so a fresh piece appears crisp and ready to
/// chase rather than already dissolving into the haze.
pub const MAX_SPAWN: f32 = 800.0;
/// Items farther than this from the ship are forgotten as it sails on, so the
/// field never accumulates a wake of stale crates across the whole chart.
pub const CULL_DIST: f32 = 3200.0;
/// Once a piece has fallen abaft the beam (the ship has sailed past it) and is at
/// least this far clear, its slot is recycled into fresh salvage ahead of the bow
/// rather than left dragging astern for the whole [`CULL_DIST`] run: a passed-by
/// crate sits outside the forward view, never closes the distance, and would
/// otherwise hog one of the [`TARGET`] slots so no new salvage spawns ahead. (New:
/// the original kept every piece until `CULL_DIST`, which let passed salvage starve
/// the field of anything reachable.) Set generously so a piece is only forgotten
/// once it has genuinely fallen behind, not the moment it slips out of view.
pub const ASTERN_CULL: f32 = 500.0;
/// Metres of open water kept around an island when strewing salvage, so crates
/// float offshore rather than on the beach.
pub const SHORE_CLEARANCE: f32 = 120.0;
/// How close the ship must come (m) to scoop a piece aboard — a touch wider than
/// the hull (~24 m bow-to-stern), so sailing *over* a crate lifts it without
/// hoovering up salvage from a boat-length away. (New: the original left the reach
/// to the caller; this is the value `main` sweeps with.)
pub const REACH: f32 = 30.0;
/// Tries to find open water for one piece before giving up this round.
const MAX_ATTEMPTS: i32 = 8;

/// The drifting salvage currently at sea. The field carries its own RNG so
/// topping it up stays pure and deterministic from a seed. (`shared.FlotsamField`.)
#[derive(Clone, Debug)]
pub struct FlotsamField {
    pub items: Vec<Flotsam>,
    pub next_id: i32,
    pub rng: Rng,
}

impl FlotsamField {
    /// A fresh, empty field seeded deterministically. (`FlotsamField.fromSeed`.)
    pub fn from_seed(seed: i64) -> FlotsamField {
        FlotsamField {
            items: Vec::new(),
            next_id: 0,
            rng: Rng::from_seed(seed),
        }
    }

    /// Scoop up every floating item within `reach` metres of the ship, removing
    /// them from the field and returning the gold gained and the items lifted.
    /// (`FlotsamField.collectNear`.)
    pub fn collect_near(&mut self, pos: Vec2, reach: f32) -> Haul {
        let mut picked = Vec::new();
        let mut kept = Vec::with_capacity(self.items.len());
        for f in self.items.drain(..) {
            if f.pos.distance_to(pos) <= reach {
                picked.push(f);
            } else {
                kept.push(f);
            }
        }
        self.items = kept;
        let gold = picked.iter().map(|f| f.kind.gold()).sum();
        Haul { gold, picked }
    }

    /// Top the field back up to [`TARGET`] items drifting within sight of the
    /// ship, so there is always fresh salvage on the horizon to chase.
    ///
    /// Items fallen far astern (beyond [`CULL_DIST`]) are dropped first, along with
    /// any the ship has already sailed past (abaft the beam and clear by
    /// [`ASTERN_CULL`]), so passed-by salvage frees its slot instead of receding out
    /// of reach forever. New ones then appear in a ring around the ship — far enough
    /// out not to pop in underfoot, close enough to spot — and never atop an island.
    /// Fresh salvage is biased to lie *ahead* of the bow (`heading`), so it drifts
    /// into the player's course rather than scattering evenly behind. Threads the
    /// field's own RNG. (`FlotsamField.replenish`.)
    pub fn replenish(&mut self, center: Vec2, heading: f32, world: &World) {
        self.items.retain(|f| {
            let d = f.pos.distance_to(center);
            if d > CULL_DIST {
                return false;
            }
            // Abaft the beam means the ship has passed it; recycle it once clear so
            // a fresh piece can spawn ahead. (Abaft pieces are outside the forward
            // view, so this never blinks a visible crate out of existence.)
            let abaft = wrap_angle(center.bearing_to(f.pos) - heading).abs() > FRAC_PI_2;
            !(abaft && d > ASTERN_CULL)
        });
        let need = TARGET.saturating_sub(self.items.len());
        for _ in 0..need {
            if let Some(item) = draw_item(center, heading, world, self.next_id, &mut self.rng) {
                self.items.push(item);
                self.next_id += 1;
            }
        }
    }
}

fn open_water(p: Vec2, world: &World) -> bool {
    world
        .islands
        .iter()
        .all(|isle| p.distance_to(isle.pos) > isle.radius + SHORE_CLEARANCE)
}

/// Try to place one piece in open water in the spawn ring around `center`, biased
/// to lie ahead of `heading`. The bearing offset is a cubic of a uniform draw,
/// which crowds spawns into a forward cone while still letting the odd piece drift
/// in from the beam or quarter — natural-looking, not a rigid arc. Advances `rng`
/// once per attempt (two `between` draws + one [`FlotsamKind::pick`]), matching the
/// Scala draw order. (`FlotsamField.drawItem`.)
fn draw_item(center: Vec2, heading: f32, world: &World, id: i32, rng: &mut Rng) -> Option<Flotsam> {
    for _ in 0..MAX_ATTEMPTS {
        let u = rng.between(-1.0, 1.0) as f32;
        let dist = rng.between(MIN_SPAWN as f64, MAX_SPAWN as f64) as f32;
        let kind = FlotsamKind::pick(rng);
        let ang = heading + u * u * u * PI; // cubic bias toward dead ahead
        let p = center + Vec2::from_heading(ang) * dist;
        if open_water(p, world) {
            return Some(Flotsam { id, pos: p, kind });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::wrap_angle;
    use std::f32::consts::PI;

    fn world() -> World {
        crate::world::generate(7)
    }

    #[test]
    fn collect_near_scoops_an_item_the_ship_sails_over_and_awards_its_gold() {
        let mut field = FlotsamField {
            items: vec![Flotsam {
                id: 0,
                pos: Vec2::new(100.0, 100.0),
                kind: FlotsamKind::Barrel,
            }],
            next_id: 1,
            rng: Rng::from_seed(99),
        };
        let haul = field.collect_near(Vec2::new(110.0, 95.0), 40.0);
        assert_eq!(haul.gold, FlotsamKind::Barrel.gold());
        assert_eq!(haul.picked.iter().map(|f| f.id).collect::<Vec<_>>(), vec![0]);
        assert!(field.items.is_empty());
    }

    #[test]
    fn collect_near_leaves_salvage_beyond_reach_untouched() {
        let mut field = FlotsamField {
            items: vec![Flotsam {
                id: 0,
                pos: Vec2::new(1000.0, 0.0),
                kind: FlotsamKind::Crate,
            }],
            next_id: 1,
            rng: Rng::from_seed(99),
        };
        let haul = field.collect_near(Vec2::ZERO, 40.0);
        assert_eq!(haul.gold, 0);
        assert!(haul.picked.is_empty());
        assert_eq!(field.items.iter().map(|f| f.id).collect::<Vec<_>>(), vec![0]);
    }

    #[test]
    fn collect_near_takes_only_what_is_in_range() {
        let mut field = FlotsamField {
            items: vec![
                Flotsam { id: 0, pos: Vec2::new(5.0, 0.0), kind: FlotsamKind::Crate },
                Flotsam { id: 1, pos: Vec2::new(5000.0, 0.0), kind: FlotsamKind::Chest },
            ],
            next_id: 2,
            rng: Rng::from_seed(99),
        };
        let haul = field.collect_near(Vec2::ZERO, 40.0);
        assert_eq!(haul.picked.iter().map(|f| f.id).collect::<Vec<_>>(), vec![0]);
        assert_eq!(field.items.iter().map(|f| f.id).collect::<Vec<_>>(), vec![1]);
    }

    #[test]
    fn replenish_strews_a_full_quota_within_sight_in_open_water() {
        let world = world();
        let mut field = FlotsamField::from_seed(3);
        field.replenish(Vec2::ZERO, 0.0, &world);
        assert_eq!(field.items.len(), TARGET);
        let dists: Vec<f32> = field.items.iter().map(|f| f.pos.distance_to(Vec2::ZERO)).collect();
        assert!(dists.iter().cloned().fold(0.0_f32, f32::max) <= MAX_SPAWN);
        assert!(dists.iter().cloned().fold(f32::MAX, f32::min) >= MIN_SPAWN);
        let nearest_shore = field
            .items
            .iter()
            .map(|f| {
                world
                    .islands
                    .iter()
                    .map(|i| i.pos.distance_to(f.pos))
                    .fold(f32::MAX, f32::min)
            })
            .fold(f32::MAX, f32::min);
        assert!(nearest_shore > SHORE_CLEARANCE);
    }

    #[test]
    fn replenish_favours_salvage_ahead_of_the_bow() {
        // Heading due east (+x): most of the quota should lie in the forward hemisphere.
        let world = world();
        let mut field = FlotsamField::from_seed(3);
        field.replenish(Vec2::ZERO, PI / 2.0, &world);
        let ahead = field
            .items
            .iter()
            .filter(|f| wrap_angle(Vec2::ZERO.bearing_to(f.pos) - PI / 2.0).abs() < PI / 2.0)
            .count();
        assert!(ahead > TARGET / 2);
    }

    #[test]
    fn replenish_forgets_salvage_fallen_far_astern() {
        let world = world();
        let mut field = FlotsamField {
            items: vec![Flotsam {
                id: 0,
                pos: Vec2::new(CULL_DIST + 500.0, 0.0),
                kind: FlotsamKind::Crate,
            }],
            next_id: 1,
            rng: Rng::from_seed(5),
        };
        field.replenish(Vec2::ZERO, 0.0, &world);
        assert!(!field.items.iter().any(|f| f.id == 0));
    }

    #[test]
    fn replenish_recycles_salvage_the_ship_has_sailed_past() {
        // A piece dead astern (heading due north, piece to the south) and well clear
        // of pickup reach should be recycled rather than left receding out of reach.
        let world = world();
        let mut field = FlotsamField {
            items: vec![Flotsam {
                id: 0,
                pos: Vec2::new(0.0, -(ASTERN_CULL + 200.0)),
                kind: FlotsamKind::Crate,
            }],
            next_id: 1,
            rng: Rng::from_seed(5),
        };
        field.replenish(Vec2::ZERO, 0.0, &world);
        assert!(!field.items.iter().any(|f| f.id == 0));
        // ...but a piece the same distance ahead is kept (it is still reachable).
        let mut field = FlotsamField {
            items: vec![Flotsam {
                id: 0,
                pos: Vec2::new(0.0, ASTERN_CULL + 200.0),
                kind: FlotsamKind::Crate,
            }],
            next_id: 1,
            rng: Rng::from_seed(5),
        };
        field.replenish(Vec2::ZERO, 0.0, &world);
        assert!(field.items.iter().any(|f| f.id == 0));
    }

    #[test]
    fn pick_is_weighted_toward_common_salvage() {
        // Over many draws the common crate should outnumber the rare chest.
        let mut rng = Rng::from_seed(123);
        let mut crates = 0;
        let mut chests = 0;
        for _ in 0..2000 {
            match FlotsamKind::pick(&mut rng) {
                FlotsamKind::Crate => crates += 1,
                FlotsamKind::Chest => chests += 1,
                FlotsamKind::Barrel => {}
            }
        }
        assert!(crates > chests);
    }
}
