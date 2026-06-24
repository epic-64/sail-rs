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
pub const PER_PORT: i32 = 5;

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
/// always presents the same board. Each contract targets a different port *in
/// the same local cluster* — hauls stay within the waters the captain is sailing
/// — and the reward scales with both the goods' value and the distance to haul
/// them. (Cross-cluster long-haul contracts are left for later.)
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

    let mut built = Vec::with_capacity(PER_PORT as usize);
    for slot in 0..PER_PORT {
        let good = *rng.pick(&Good::ALL);
        let target: &Island = *rng.pick(&others);
        let qf = rng.between(5.0, 15.0);
        let quantity = qf.round() as i32;
        let value = quantity * market.price(good);
        // The deposit is the goods' value plus 10%, so abandoning a contract (and
        // keeping the goods to sell) always costs at least that 10% — closing the
        // accept-abandon-sell arbitrage.
        let deposit = (value as f64 * 1.1).ceil() as i32;
        let distance = origin.pos.distance_to(target.pos) as f64;
        let reward = ((value as f64 * 0.3 + distance * quantity as f64 * 0.0025).round() as i32).max(1);
        built.push(Mission {
            id: origin.id * 100 + slot,
            good,
            quantity,
            origin_id: origin.id,
            target_id: target.id,
            reward,
            deposit,
        });
    }
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
                radius: 4000.0,
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
        gs.cargo = [0; 7];
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
        assert!(!offered_at(&gs, &world).iter().any(|m| *m == contract));
    }
}
