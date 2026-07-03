//! The treasure dig minigame played when the captain goes ashore on an
//! uninhabited isle.
//!
//! A square field of buried tiles (`GRID` on a side) hides loose coins and
//! chests. Each chest spans `CHEST_TILES` adjacent tiles and pays out *only*
//! once every one of them is cleared, so it costs a big slice of the move
//! budget: the whole game is deciding whether the chest you glimpsed is worth
//! finishing or whether to keep grabbing loose coins with your remaining digs.
//!
//! This module is pure rules: no drawing, no input. It seeds its own [`Rng`]
//! from the world seed, the island id and the day, so a given isle shows the
//! same field all day and a fresh one each morning. That seed is independent of
//! the world-generation draw sequence, so nothing here can shift island layout.

use crate::rng::Rng;

/// Tiles along one edge of the field; the field is `GRID * GRID` tiles.
pub const GRID: usize = 6;
/// Total buried tiles in a field.
pub const TILES: usize = GRID * GRID;
/// Digs the captain gets before the field is spent.
pub const MOVES: u32 = 10;
/// Tiles a single chest occupies (a 2x3 / 3x2 footprint).
pub const CHEST_TILES: usize = 6;

// Reward tuning. Kept here so the payout curve is one edit away; the dig rules
// below never bake in a magnitude.
const COIN_MIN: i32 = 15;
const COIN_MAX: i32 = 60;
const CHEST_MIN: i32 = 150;
const CHEST_MAX: i32 = 320;
// How many loose coins get scattered across the non-chest tiles.
const COIN_MIN_COUNT: i32 = 5;
const COIN_MAX_COUNT: i32 = 9;
// Chance the field hides a second chest as well as the first.
const SECOND_CHEST_CHANCE: f64 = 0.35;
// Bounded rejection-sampling attempts when seeking a free chest footprint.
const PLACE_ATTEMPTS: i32 = 24;

/// What lies under a tile before it is dug.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Buried {
    /// Nothing but dirt.
    Dirt,
    /// A loose coin worth this much gold, banked the moment it is dug.
    Coin(i32),
    /// Part of the chest with this index in [`DigSite::chests`].
    Chest(usize),
}

/// One buried chest: the tiles it covers and the gold it pays once they are all
/// cleared.
#[derive(Clone, Debug)]
pub struct Chest {
    pub tiles: [usize; CHEST_TILES],
    pub reward: i32,
    pub claimed: bool,
}

/// The outcome of a single [`DigSite::dig`], for the UI to react to (sound,
/// flash, banner).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DigResult {
    /// The dig did nothing: the field was spent or the tile was already open.
    Spent,
    /// Turned up plain dirt.
    Dirt,
    /// Banked a loose coin worth this much gold.
    Coin(i32),
    /// Uncovered a piece of a chest that still needs more digging.
    ChestPiece,
    /// This dig cleared the last tile of a chest and banked its reward.
    ChestClaimed(i32),
}

/// A single ashore dig field and the captain's progress through it.
#[derive(Clone, Debug)]
pub struct DigSite {
    tiles: [Buried; TILES],
    revealed: [bool; TILES],
    pub chests: Vec<Chest>,
    pub moves_left: u32,
    pub gold_found: i32,
}

/// The tile index of a row/column, so chest footprints are easy to lay out.
fn idx(row: usize, col: usize) -> usize {
    row * GRID + col
}

/// Blend the three inputs into one PRNG seed. Distinct multipliers keep
/// neighbouring islands and consecutive days from aliasing onto the same field.
fn field_seed(world_seed: i64, island_id: i32, day: u32) -> i64 {
    world_seed
        .wrapping_mul(0x2f9d)
        .wrapping_add((island_id as i64).wrapping_mul(0x9e37_79b1))
        .wrapping_add((day as i64).wrapping_mul(0x85eb_ca77))
}

impl DigSite {
    /// Build the field an isle shows on a given day. Deterministic in its
    /// inputs, and seeded off its own [`Rng`] so it never perturbs world gen.
    pub fn generate(world_seed: i64, island_id: i32, day: u32) -> DigSite {
        let mut rng = Rng::from_seed(field_seed(world_seed, island_id, day));
        let mut tiles = [Buried::Dirt; TILES];
        let mut chests: Vec<Chest> = Vec::new();

        // Always at least one chest; sometimes a second. Placement is rejection
        // sampling within a bounded attempt count so a crowded field can quietly
        // settle for fewer chests rather than loop forever.
        let want = if rng.next_f64() < SECOND_CHEST_CHANCE { 2 } else { 1 };
        for _ in 0..want {
            if let Some(footprint) = place_chest(&mut rng, &tiles) {
                let ci = chests.len();
                for &t in &footprint {
                    tiles[t] = Buried::Chest(ci);
                }
                let reward = rng.int_between(CHEST_MIN, CHEST_MAX + 1);
                chests.push(Chest {
                    tiles: footprint,
                    reward,
                    claimed: false,
                });
            }
        }

        // Scatter loose coins over the tiles the chests did not take.
        let coins = rng.int_between(COIN_MIN_COUNT, COIN_MAX_COUNT + 1);
        for _ in 0..coins {
            // A bounded probe for a free dirt tile; if the field is packed we
            // simply place fewer coins.
            for _ in 0..PLACE_ATTEMPTS {
                let t = rng.int_between(0, TILES as i32) as usize;
                if tiles[t] == Buried::Dirt {
                    tiles[t] = Buried::Coin(rng.int_between(COIN_MIN, COIN_MAX + 1));
                    break;
                }
            }
        }

        DigSite {
            tiles,
            revealed: [false; TILES],
            chests,
            moves_left: MOVES,
            gold_found: 0,
        }
    }

    /// Whether the tile at `i` has been dug open.
    pub fn is_open(&self, i: usize) -> bool {
        self.revealed[i]
    }

    /// What is buried at `i` (regardless of whether it has been revealed). The
    /// UI uses this only for already-open tiles.
    pub fn buried_at(&self, i: usize) -> Buried {
        self.tiles[i]
    }

    /// True once the move budget is exhausted.
    pub fn finished(&self) -> bool {
        self.moves_left == 0
    }

    /// Dig the tile at `i`. Costs one move unless the field is spent or the tile
    /// is already open (both no-ops). Coins bank immediately; a chest banks its
    /// reward on the dig that clears its final tile.
    pub fn dig(&mut self, i: usize) -> DigResult {
        if self.moves_left == 0 || self.revealed[i] {
            return DigResult::Spent;
        }
        self.revealed[i] = true;
        self.moves_left -= 1;

        match self.tiles[i] {
            Buried::Dirt => DigResult::Dirt,
            Buried::Coin(g) => {
                self.gold_found += g;
                DigResult::Coin(g)
            }
            Buried::Chest(ci) => {
                let done = self.chests[ci].tiles.iter().all(|&t| self.revealed[t]);
                if done && !self.chests[ci].claimed {
                    self.chests[ci].claimed = true;
                    let reward = self.chests[ci].reward;
                    self.gold_found += reward;
                    DigResult::ChestClaimed(reward)
                } else {
                    DigResult::ChestPiece
                }
            }
        }
    }
}

/// Try to find a free 2x3 or 3x2 block of dirt for a chest. Returns the six tile
/// indices, or `None` if no free footprint turned up within the attempt budget.
fn place_chest(rng: &mut Rng, tiles: &[Buried; TILES]) -> Option<[usize; CHEST_TILES]> {
    for _ in 0..PLACE_ATTEMPTS {
        // Orientation: tall (3 rows x 2 cols) or wide (2 rows x 3 cols).
        let (h, w) = if rng.next_f64() < 0.5 { (3, 2) } else { (2, 3) };
        let top = rng.int_between(0, (GRID - h + 1) as i32) as usize;
        let left = rng.int_between(0, (GRID - w + 1) as i32) as usize;

        let mut footprint = [0usize; CHEST_TILES];
        let mut k = 0;
        let mut clear = true;
        for r in 0..h {
            for c in 0..w {
                let t = idx(top + r, left + c);
                if tiles[t] != Buried::Dirt {
                    clear = false;
                }
                footprint[k] = t;
                k += 1;
            }
        }
        if clear {
            return Some(footprint);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_is_deterministic_in_its_inputs() {
        let a = DigSite::generate(12345, 7, 3);
        let b = DigSite::generate(12345, 7, 3);
        assert_eq!(a.tiles, b.tiles);
        assert_eq!(a.chests.len(), b.chests.len());
    }

    #[test]
    fn a_new_day_reshuffles_the_field() {
        let today = DigSite::generate(12345, 7, 3);
        let tomorrow = DigSite::generate(12345, 7, 4);
        // Overwhelmingly the layouts differ; guard the seed-mixing wiring.
        assert_ne!(today.tiles, tomorrow.tiles);
    }

    #[test]
    fn every_chest_covers_exactly_its_footprint() {
        // Sweep a range of seeds so both orientations and both chest counts show.
        for seed in 0..200i64 {
            let site = DigSite::generate(seed, 1, 1);
            assert!(!site.chests.is_empty(), "a field always hides a chest");
            for (ci, chest) in site.chests.iter().enumerate() {
                let covered = site
                    .tiles
                    .iter()
                    .filter(|&&t| t == Buried::Chest(ci))
                    .count();
                assert_eq!(covered, CHEST_TILES);
                assert_eq!(chest.tiles.len(), CHEST_TILES);
            }
        }
    }

    #[test]
    fn a_dig_spends_one_move_and_reveals_the_tile() {
        let mut site = DigSite::generate(1, 1, 1);
        assert_eq!(site.moves_left, MOVES);
        assert!(!site.is_open(0));
        site.dig(0);
        assert_eq!(site.moves_left, MOVES - 1);
        assert!(site.is_open(0));
    }

    #[test]
    fn redigging_an_open_tile_is_a_free_no_op() {
        let mut site = DigSite::generate(1, 1, 1);
        site.dig(0);
        let moves = site.moves_left;
        let gold = site.gold_found;
        assert_eq!(site.dig(0), DigResult::Spent);
        assert_eq!(site.moves_left, moves);
        assert_eq!(site.gold_found, gold);
    }

    #[test]
    fn digging_stops_when_the_budget_is_spent() {
        let mut site = DigSite::generate(1, 1, 1);
        // Dig distinct tiles until out of moves.
        let mut i = 0;
        while !site.finished() {
            site.dig(i);
            i += 1;
        }
        assert_eq!(site.moves_left, 0);
        // A further dig on a fresh tile changes nothing.
        assert_eq!(site.dig(TILES - 1), DigResult::Spent);
        assert!(!site.is_open(TILES - 1));
    }

    #[test]
    fn a_coin_banks_its_gold_once() {
        let mut site = DigSite::generate(1, 1, 1);
        let coin = (0..TILES)
            .find(|&t| matches!(site.tiles[t], Buried::Coin(_)))
            .expect("a field scatters coins");
        let Buried::Coin(value) = site.tiles[coin] else {
            unreachable!()
        };
        assert_eq!(site.dig(coin), DigResult::Coin(value));
        assert_eq!(site.gold_found, value);
    }

    #[test]
    fn a_chest_pays_only_once_every_tile_is_cleared() {
        // Give the field a generous budget by rebuilding rules-free: dig the six
        // chest tiles directly and confirm the reward lands on the last one.
        let mut site = DigSite::generate(1, 1, 1);
        let chest = site.chests[0].clone();
        for (n, &t) in chest.tiles.iter().enumerate() {
            // Ensure we never run dry mid-excavation for this accounting test.
            site.moves_left = MOVES;
            let result = site.dig(t);
            if n + 1 < CHEST_TILES {
                assert_eq!(result, DigResult::ChestPiece);
                assert_eq!(site.gold_found, 0, "no gold until the chest is finished");
            } else {
                assert_eq!(result, DigResult::ChestClaimed(chest.reward));
                assert_eq!(site.gold_found, chest.reward);
                assert!(site.chests[0].claimed);
            }
        }
    }
}
