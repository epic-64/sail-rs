//! Haulage contracts taken at a port, ported faithfully from `shared.Mission`
//! (`Mission` + the `Missions` object).
//!
//! A contract carries `quantity` units of `good` to `target_id` for a `reward`.
//! Accepting one loads the goods into the hold (so it costs cargo space) and
//! requires a `deposit` of the goods' value plus 10%, returned alongside the
//! reward on delivery. The cargo is mission-bound — it cannot be sold at market,
//! only delivered (or abandoned, forfeiting the deposit).
//!
//! The board a port offers is derived deterministically from the world seed, so
//! the same chart always presents the same contracts. The state mutations
//! (accept / deliver / abandon) live on [`GameState`] in [`crate::game_state`];
//! the pure board derivation lives here.

use crate::game_state::{GameState, Good, Market};
use crate::race;
use crate::rng::Rng;
use crate::world::{Island, World};

/// A haulage contract taken at a port. Small and `Copy` (matching the Scala
/// `case class`), so the boards can be rebuilt and filtered freely.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Mission {
    pub id: i32,
    pub good: Good,
    pub quantity: i32,
    pub origin_id: i32,
    pub target_id: i32,
    pub reward: i32,
    pub deposit: i32,
}

/// How many contracts a port offers at once (`Missions.perPort`).
pub const PER_PORT: i32 = 6;

/// Cargo contracts are graded into four tiers by how far the haul runs, reusing
/// the very rungs the races step up on ([`race::HULL_REQ_KM`]): a tier-1 job is a
/// short hop, a tier-4 job a long ocean crossing. The tier sets only how much
/// cargo the job carries ([`tier_quantity_range`]); unlike a race it gates
/// nothing, so the only real bar is having the hold to stow it. Grading by
/// distance keeps the big, far hauls off a starter's board near the bottom of the
/// list, so a novice still finding their sea legs doesn't accept a 60-unit
/// crossing before they can sail one.
pub fn tier_for(distance_m: f32) -> i32 {
    race::required_level_for(distance_m) + 1
}

/// The inclusive cargo-quantity band a contract of the given (1-based) tier carries.
/// The bands step up with a clear gap between them, so a heavier haul always reads
/// as a longer one. A tier-4 64-unit haul fills a fully-upgraded hold.
fn tier_quantity_range(tier: i32) -> (f64, f64) {
    match tier {
        1 => (5.0, 13.0),
        2 => (17.0, 24.0),
        3 => (28.0, 40.0),
        _ => (45.0, 64.0),
    }
}

/// The contracts on the board at the captain's current port, with any already
/// accepted removed. Empty while at sea.
pub fn offered_at(state: &GameState, world: &World) -> Vec<Mission> {
    match state.docked_island(world) {
        None => Vec::new(),
        Some(isle) => generate(isle, world)
            .into_iter()
            .filter(|m| !state.active_missions.iter().any(|a| a.id == m.id))
            .collect(),
    }
}

/// The accepted contracts whose destination is the current port — i.e. the
/// deliveries the captain can hand in right now. Empty while at sea.
pub fn deliverable_at(state: &GameState, world: &World) -> Vec<Mission> {
    match state.docked_island(world) {
        None => Vec::new(),
        Some(isle) => state
            .active_missions
            .iter()
            .copied()
            .filter(|m| m.target_id == isle.id)
            .collect(),
    }
}

/// The accepted contracts still bound somewhere else — the reserved cargo riding
/// in the hold, shown on the manifest. Excludes anything deliverable at the
/// current port (those show, actionable, on the deliveries board). Empty at sea.
pub fn reserved_at(state: &GameState, world: &World) -> Vec<Mission> {
    match state.docked_island(world) {
        None => Vec::new(),
        Some(isle) => state
            .active_missions
            .iter()
            .copied()
            .filter(|m| m.target_id != isle.id)
            .collect(),
    }
}

/// Deterministically derive the contracts a port offers, so the same world
/// always presents the same board. Every contract targets a port *in the same
/// local cluster* (hauls stay within the waters the captain is sailing), and the
/// reward scales with both the goods' value and the distance to haul them.
/// (Cross-cluster long-haul contracts are left for later.)
///
/// The targets are *anchored* across the distance range rather than drawn purely
/// at random: the two nearest ports, a middling one and the far reaches are always
/// represented, so every board carries a beginner-friendly short hop as well as a
/// long, lucrative crossing. The remaining slot is a free random pick.
pub fn generate(origin: &Island, world: &World) -> Vec<Mission> {
    let others: Vec<&Island> = match world
        .clusters
        .iter()
        .find(|c| c.island_ids.contains(&origin.id))
    {
        Some(c) => world
            .cluster_islands(c)
            .into_iter()
            .filter(|p| p.is_port && p.id != origin.id)
            .collect(),
        None => world
            .islands
            .iter()
            .filter(|p| p.is_port && p.id != origin.id)
            .collect(),
    };
    if others.is_empty() {
        return Vec::new();
    }

    // Value the cargo at the origin's own market, since that is what the captain
    // would actually pay (or recoup, on abandon) for these goods at this port.
    let market = Market::for_island(origin, world.seed);
    let mut rng = Rng::from_seed(
        world.seed
            ^ (origin.id as i64).wrapping_mul(0x9e3779b97f4a7c15u64 as i64)
            ^ 0x5f3759df,
    );

    // Other ports ranked nearest-first, so we can anchor the board's targets across
    // the distance range.
    let mut by_dist = others.clone();
    by_dist.sort_by(|a, b| {
        origin
            .pos
            .distance_to(a.pos)
            .total_cmp(&origin.pos.distance_to(b.pos))
    });
    let n = by_dist.len();
    // The "middling" haul: the port whose distance sits nearest the average.
    let mean = by_dist.iter().map(|p| origin.pos.distance_to(p.pos)).sum::<f32>() / n as f32;
    let avg = (0..n)
        .min_by(|&a, &b| {
            (origin.pos.distance_to(by_dist[a].pos) - mean)
                .abs()
                .total_cmp(&(origin.pos.distance_to(by_dist[b].pos) - mean).abs())
        })
        .unwrap();
    // Guaranteed targets: closest, second-closest, middling, farthest and
    // second-farthest (indices coincide and collapse on a small cluster). The
    // remaining slots beyond these anchors draw a target at random.
    let anchors = [0, 1.min(n - 1), avg, n - 1, n.saturating_sub(2)];
    let mut targets: Vec<&Island> = Vec::with_capacity(PER_PORT as usize);
    for &a in anchors.iter().take(PER_PORT as usize) {
        targets.push(by_dist[a]);
    }
    while targets.len() < PER_PORT as usize {
        targets.push(*rng.pick(&others));
    }

    let mut built = Vec::with_capacity(PER_PORT as usize);
    for (slot, target) in targets.iter().enumerate() {
        let good = *rng.pick(&Good::ALL);
        let distance = origin.pos.distance_to(target.pos) as f64;
        // The haul's distance grades it into a tier, and the tier alone sets how
        // much cargo it carries: a short hop is a light job, a long crossing a
        // heavy one.
        let (lo, hi) = tier_quantity_range(tier_for(distance as f32));
        let quantity = rng.between(lo, hi).round() as i32;
        let value = quantity * market.price(good);
        // The deposit is the goods' value plus 10%, so abandoning a contract (and
        // keeping the goods to sell) always costs at least that 10%, closing the
        // accept-abandon-sell arbitrage.
        let deposit = (value as f64 * 1.1).ceil() as i32;
        let reward = ((value as f64 * 0.3 + distance * quantity as f64 * 0.0025).round() as i32).max(1);
        built.push(Mission {
            id: origin.id * 100 + slot as i32,
            good,
            quantity,
            origin_id: origin.id,
            target_id: target.id,
            reward,
            deposit,
        });
    }
    // Present the board closest-first, so it reads top-down from the short, light
    // jobs to the long, heavy ones; the contract id keeps each slot identifiable
    // regardless of where the sort lands it.
    built.sort_by(|a, b| {
        let da = origin.pos.distance_to(world.islands[a.target_id as usize].pos);
        let db = origin.pos.distance_to(world.islands[b.target_id as usize].pos);
        da.total_cmp(&db)
    });
    built
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_state::{Location, TradeError};
    use crate::geometry::Vec2;
    use crate::world::{Cluster, IsleKind};

    // A little two-port cluster so contracts always have a destination.
    fn two_port_world() -> World {
        let mk = |id: i32, x: f32, y: f32| Island {
            id,
            name: format!("Isle {id}"),
            pos: Vec2::new(x, y),
            radius: 100.0,
            height: 20.0,
            terrain: IsleKind::Green,
            is_port: true,
            is_shipyard: true,
        };
        World {
            seed: 99,
            islands: vec![mk(0, 0.0, 0.0), mk(1, 2000.0, 0.0)],
            clusters: vec![Cluster {
                id: 0,
                name: "Waters".into(),
                center: Vec2::ZERO,
                island_ids: vec![0, 1],
            }],
        }
    }

    // A flush captain, docked at port 0, with a roomy hold so a contract can
    // always be funded and loaded.
    fn flush_state() -> GameState {
        let mut gs = GameState::start();
        gs.gold = 100_000;
        gs.hold_capacity = 1000;
        // A roomy hold raises max hull, so fill it: a flush captain's hull is sound
        // (else the 30%-hull job ban would refuse these contracts). See
        // `game_state::hull::can_take_jobs`.
        gs.hull = gs.max_hull();
        gs.location = Location::Docked(0);
        gs
    }

    #[test]
    fn accept_pays_deposit_loads_goods_and_activates() {
        let world = two_port_world();
        let mut gs = flush_state();
        let contract = offered_at(&gs, &world)[0];

        let gold_before = gs.gold;
        gs.accept(&world, contract.id).unwrap();
        assert_eq!(gs.gold, gold_before - contract.deposit);
        assert_eq!(gs.active_missions, vec![contract]);
        assert_eq!(gs.mission_hold(), contract.quantity);
    }

    #[test]
    fn accept_rejects_underfunded_contract() {
        let world = two_port_world();
        let mut gs = flush_state();
        let contract = offered_at(&gs, &world)[0];
        gs.gold = contract.deposit - 1;
        assert_eq!(gs.accept(&world, contract.id), Err(TradeError::NotEnoughGold));
    }

    #[test]
    fn accept_rejects_contract_that_will_not_fit() {
        let world = two_port_world();
        let mut gs = flush_state();
        let contract = offered_at(&gs, &world)[0];
        gs.hold_capacity = contract.quantity - 1;
        assert_eq!(gs.accept(&world, contract.id), Err(TradeError::NotEnoughHold));
    }

    #[test]
    fn accept_rejects_unknown_contract() {
        let world = two_port_world();
        let mut gs = flush_state();
        assert_eq!(gs.accept(&world, -1), Err(TradeError::NoSuchMission));
    }

    #[test]
    fn accept_refused_while_at_sea() {
        let world = two_port_world();
        let mut gs = flush_state();
        let contract = offered_at(&gs, &world)[0];
        gs.location = Location::Sailing;
        assert_eq!(gs.accept(&world, contract.id), Err(TradeError::NotDocked));
    }

    #[test]
    fn deliver_returns_deposit_pays_reward_and_frees_hold() {
        let world = two_port_world();
        let mut gs = flush_state();
        let contract = offered_at(&gs, &world)[0];
        gs.active_missions = vec![contract];
        gs.location = Location::Docked(contract.target_id);

        let gold_before = gs.gold;
        gs.deliver(&world, contract.id).unwrap();
        assert_eq!(gs.gold, gold_before + contract.deposit + contract.reward);
        assert!(gs.active_missions.is_empty());
    }

    #[test]
    fn deliver_refused_anywhere_but_the_destination() {
        let world = two_port_world();
        let mut gs = flush_state();
        let contract = offered_at(&gs, &world)[0];
        gs.active_missions = vec![contract];
        gs.location = Location::Docked(contract.origin_id);
        assert_eq!(gs.deliver(&world, contract.id), Err(TradeError::NoDelivery));
    }

    #[test]
    fn abandon_drops_contract_and_keeps_goods_as_sellable_cargo() {
        let world = two_port_world();
        let mut gs = flush_state();
        let contract = offered_at(&gs, &world)[0];
        gs.cargo = [0; 8];
        gs.active_missions = vec![contract];

        let hold_before = gs.hold_used();
        let gold_before = gs.gold;
        gs.abandon(&world, contract.id).unwrap();
        assert!(gs.active_missions.is_empty());
        assert_eq!(gs.gold, gold_before);
        assert_eq!(gs.quantity_of(contract.good), contract.quantity);
        assert_eq!(gs.hold_used(), hold_before);
    }

    #[test]
    fn offered_at_hides_a_contract_once_accepted() {
        let world = two_port_world();
        let mut gs = flush_state();
        let contract = offered_at(&gs, &world)[0];
        gs.accept(&world, contract.id).unwrap();
        assert!(!offered_at(&gs, &world).contains(&contract));
    }

    // A spread-out cluster of ports at a range of distances, so the generated
    // board carries contracts of several tiers to sort and grade.
    fn spread_world() -> World {
        let mk = |id: i32, x: f32, y: f32| Island {
            id,
            name: format!("Isle {id}"),
            pos: Vec2::new(x, y),
            radius: 100.0,
            height: 20.0,
            terrain: IsleKind::Green,
            is_port: true,
            is_shipyard: true,
        };
        // Origin at 0, targets a short hop to a long crossing away (metres).
        let islands = vec![
            mk(0, 0.0, 0.0),
            mk(1, 1500.0, 0.0),
            mk(2, 5000.0, 0.0),
            mk(3, 8000.0, 0.0),
            mk(4, 12000.0, 0.0),
        ];
        let island_ids = islands.iter().map(|i| i.id).collect();
        World {
            seed: 7,
            islands,
            clusters: vec![Cluster {
                id: 0,
                name: "Waters".into(),
                center: Vec2::ZERO,
                island_ids,
            }],
        }
    }

    #[test]
    fn tier_steps_up_with_distance() {
        // The race rungs (4.0, 6.5, 9.0 km) graduate the four tiers.
        assert_eq!(tier_for(1_500.0), 1);
        assert_eq!(tier_for(5_000.0), 2);
        assert_eq!(tier_for(8_000.0), 3);
        assert_eq!(tier_for(12_000.0), 4);
    }

    #[test]
    fn board_is_sorted_closest_first() {
        let world = spread_world();
        let board = generate(&world.islands[0], &world);
        let dists: Vec<f32> = board
            .iter()
            .map(|m| world.islands[0].pos.distance_to(world.islands[m.target_id as usize].pos))
            .collect();
        assert!(
            dists.windows(2).all(|w| w[0] <= w[1]),
            "contracts should be ordered nearest first, got {dists:?}"
        );
    }

    #[test]
    fn board_anchors_span_nearest_to_farthest() {
        let world = spread_world();
        let board = generate(&world.islands[0], &world);
        assert_eq!(board.len(), PER_PORT as usize);
        let targets: Vec<i32> = board.iter().map(|m| m.target_id).collect();
        // The two nearest (1, 2), the middling (3) and the farthest (4) are all
        // guaranteed a contract, whatever the lone random slot lands on.
        for id in [1, 2, 3, 4] {
            assert!(targets.contains(&id), "board should anchor a contract to isle {id}, got {targets:?}");
        }
    }

    #[test]
    fn quantity_stays_within_its_distance_tier_band() {
        let world = spread_world();
        for m in generate(&world.islands[0], &world) {
            let dist = world.islands[0].pos.distance_to(world.islands[m.target_id as usize].pos);
            let (lo, hi) = tier_quantity_range(tier_for(dist));
            assert!(
                (m.quantity as f64) >= lo.round() && (m.quantity as f64) <= hi.round(),
                "tier-{} contract carried {} units, outside {lo}..={hi}",
                tier_for(dist),
                m.quantity,
            );
        }
    }
}
