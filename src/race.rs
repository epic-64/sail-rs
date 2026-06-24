//! Wager races booked at a port, ported faithfully from `shared.Race`.
//!
//! A race is a dash from the docked port (`origin_id`) to another port
//! (`target_id`) in the same waters, against a computer-helmed rival, for a
//! `stake` of gold fixed by the length of the crossing (see [`stake_for`]). The
//! stake is paid up front when the race is booked; reaching the mark before the
//! rival pays it back doubled, trailing in forfeits it. Only one race runs at a
//! time. The rival is not spawned until the player next sets sail (see `main`),
//! so the captain can still trade and outfit before the off.
//!
//! As with [`crate::mission`], the *pure board derivation* and the *rival's
//! helmsman* live here; the state mutations (accept / win / lose / withdraw) are
//! methods on [`GameState`](crate::game_state::GameState).

use std::f32::consts::PI;

use crate::game_state::GameState;
use crate::geometry::{clamp, wrap_angle, Vec2};
use crate::rng::Rng;
use crate::sailing::{self, Helm, Kinematics, Wind};
use crate::world::{Island, World};

/// A wager race booked at a port: sail from `origin_id` to `target_id` against
/// the rival for `stake` gold. Small and `Copy`, like the Scala `case class`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Race {
    pub origin_id: i32,
    pub target_id: i32,
    pub stake: i32,
}

/// Gold staked per kilometre of the crossing, plus a gently quadratic bonus per
/// kilometre-squared so a longer leg is worth more than its length alone.
pub const GOLD_PER_KM: f64 = 30.0;
pub const BONUS_PER_KM_SQ: f64 = 6.0;

/// How many rival ports a harbour puts on offer at once (`Race.offerCount`).
pub const OFFER_COUNT: usize = 4;

/// How close to an island's shore a hull must come to take the race finish —
/// tighter than the (generous) docking range, but with room for a fast approach
/// to claim it without nosing onto the beach (`Race.finishMargin`).
pub const FINISH_MARGIN: f32 = 150.0;

/// How hard the rival puts the rudder over per radian off its lay.
const STEER_GAIN: f32 = 3.0;
/// Radians inside the no-go edge the rival points when beating, so a close-hauled
/// leg carries real drive rather than sitting on the zero-drive boundary.
const PINCH: f32 = 0.12;

/// The wager for a leg of `distance_m` metres, rounded to whole gold.
pub fn stake_for(distance_m: f32) -> i32 {
    let km = distance_m as f64 / 1000.0;
    (GOLD_PER_KM * km + BONUS_PER_KM_SQ * km * km).round() as i32
}

/// The wager for racing from `origin` to `target`.
pub fn stake_between(origin: &Island, target: &Island) -> i32 {
    stake_for(origin.pos.distance_to(target.pos))
}

/// The ports the docked port can offer a race to — every *other* port in the same
/// local cluster, nearest first. Empty while at sea. (`Race.targetsAt`.)
pub fn targets_at<'w>(state: &GameState, world: &'w World) -> Vec<&'w Island> {
    let Some(origin) = state.docked_island(world) else {
        return Vec::new();
    };
    let mut others: Vec<&Island> = match world
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
    others.sort_by(|a, b| {
        origin
            .pos
            .distance_to(a.pos)
            .partial_cmp(&origin.pos.distance_to(b.pos))
            .unwrap()
    });
    others
}

/// The harbour's card of [`OFFER_COUNT`] races: always the nearest and the
/// furthest reachable port — a short dash and a long haul — plus the rest filled
/// from those in between, then sorted nearest-first for a tidy board. Returns
/// every port when there are too few to choose from. Empty while at sea.
///
/// The original shuffles with a throwaway `Random`; we seed deterministically off
/// the world + origin (like [`crate::mission`]) so the day's card is stable for
/// the life of the port board rather than reshuffling every frame. (`Race.offers`.)
pub fn offers<'w>(state: &GameState, world: &'w World) -> Vec<&'w Island> {
    let all = targets_at(state, world); // nearest-first
    if all.len() <= OFFER_COUNT {
        return all;
    }
    let Some(origin) = state.docked_island(world) else {
        return Vec::new();
    };

    // Shuffle the middle (everything but the nearest & furthest) deterministically,
    // then take enough to fill the card alongside the head and tail.
    let mut middle: Vec<&Island> = all[1..all.len() - 1].to_vec();
    let mut rng = Rng::from_seed(
        world.seed
            ^ (origin.id as i64).wrapping_mul(0x9e3779b97f4a7c15u64 as i64)
            ^ 0x5f3759df,
    );
    for i in (1..middle.len()).rev() {
        let j = rng.int_between(0, (i + 1) as i32) as usize;
        middle.swap(i, j);
    }

    let mut chosen: Vec<&Island> = Vec::with_capacity(OFFER_COUNT);
    chosen.push(all[0]);
    chosen.push(all[all.len() - 1]);
    chosen.extend(middle.into_iter().take(OFFER_COUNT - 2));
    chosen.sort_by(|a, b| {
        origin
            .pos
            .distance_to(a.pos)
            .partial_cmp(&origin.pos.distance_to(b.pos))
            .unwrap()
    });
    chosen
}

/// Whether a hull has reached `island` for the purposes of the race — within
/// [`FINISH_MARGIN`] of its shore. (`Race.reached`.)
pub fn reached(kin: &Kinematics, island: &Island) -> bool {
    kin.pos.distance_to(island.pos) <= island.radius + FINISH_MARGIN
}

/// Place the rival right alongside the player at the off: a short way abeam, dead
/// in the water, bow already pointed at the mark. The player only has to heave to
/// (sails struck, dead slow) to arm the start. (`SailingView.rivalStart`.)
pub fn rival_start(kin: &Kinematics, target: &Island, gap: f32) -> Kinematics {
    let to_target = target.pos - kin.pos;
    let len = to_target.length();
    let dir = if len < 1.0 {
        Vec2::new(0.0, 1.0)
    } else {
        to_target * (1.0 / len)
    };
    let abeam = Vec2::new(dir.y, -dir.x); // 90° off the course to the mark
    let spawn = kin.pos + abeam * gap;
    Kinematics::still(spawn, kin.pos.bearing_to(target.pos))
}

// --- the rival's helmsman ----------------------------------------------------

/// The heading the rival should steer to make for `desired` without sailing into
/// the wind: `desired` itself when it is outside the no-go zone, otherwise the
/// nearest close-hauled lay (just inside the sailable edge) on the tack that
/// points more toward the mark. (`Race.layHeading`.)
pub fn lay_heading(desired: f32, wind: Wind) -> f32 {
    let upwind = wrap_angle(wind.toward_rad + PI); // dead into the wind
    let off = wrap_angle(desired - upwind); // signed offset of the mark from dead upwind
    let edge = (PI - sailing::DEAD_ANGLE) + PINCH; // min sailable angle off dead upwind
    if off.abs() >= edge {
        desired
    } else {
        wrap_angle(upwind + if off >= 0.0 { edge } else { -edge })
    }
}

/// The rival's helm this frame: full sail, steering for `target` but never dead
/// into the wind — when the mark lies upwind it falls off to the nearest
/// close-hauled lay and beats up to it instead. (`Race.rivalHelm`.)
pub fn rival_helm(kin: &Kinematics, target: Vec2, wind: Wind) -> Helm {
    let desired = kin.pos.bearing_to(target);
    let heading = lay_heading(desired, wind);
    let err = wrap_angle(heading - kin.heading_rad);
    Helm {
        turn: clamp(err * STEER_GAIN, -1.0, 1.0),
        throttle: 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_state::{Location, TradeError};
    use crate::world::{Cluster, IsleKind};

    // A little cluster of well-separated ports so a race always has somewhere to
    // run to, and `offers` has to choose.
    fn race_world() -> World {
        let mk = |id: i32, x: f32, y: f32| Island {
            id,
            name: format!("Isle {id}"),
            pos: Vec2::new(x, y),
            radius: 100.0,
            height: 20.0,
            terrain: IsleKind::Green,
            is_port: true,
            is_shipyard: id == 0,
        };
        // Origin at 0; five other ports at increasing distances east.
        let islands = vec![
            mk(0, 0.0, 0.0),
            mk(1, 1000.0, 0.0),
            mk(2, 2000.0, 0.0),
            mk(3, 3000.0, 0.0),
            mk(4, 4000.0, 0.0),
            mk(5, 6000.0, 0.0),
        ];
        World {
            seed: 7,
            islands,
            clusters: vec![Cluster {
                id: 0,
                name: "Waters".into(),
                center: Vec2::ZERO,
                radius: 20000.0,
                island_ids: vec![0, 1, 2, 3, 4, 5],
            }],
        }
    }

    fn flush_docked() -> GameState {
        let mut gs = GameState::start();
        gs.gold = 100_000;
        gs.location = Location::Docked(0);
        gs
    }

    #[test]
    fn stake_for_charges_per_km_plus_a_quadratic_bonus() {
        // 4 km: 30·4 + 6·4² = 120 + 96 = 216.
        assert_eq!(stake_for(4000.0), 216);
    }

    #[test]
    fn stake_rises_more_than_linearly_with_the_leg() {
        assert!(stake_for(6000.0) - stake_for(3000.0) > stake_for(3000.0) - stake_for(0.0));
    }

    #[test]
    fn offers_put_at_most_four_ports_on_the_card() {
        let world = race_world();
        let gs = flush_docked();
        assert!(offers(&gs, &world).len() <= OFFER_COUNT);
    }

    #[test]
    fn offers_always_include_the_nearest_and_the_furthest() {
        let world = race_world();
        let gs = flush_docked();
        let reachable = targets_at(&gs, &world); // nearest-first
        let offered: Vec<i32> = offers(&gs, &world).iter().map(|p| p.id).collect();
        assert!(offered.contains(&reachable.first().unwrap().id));
        assert!(offered.contains(&reachable.last().unwrap().id));
    }

    #[test]
    fn accept_charges_the_stake_up_front_and_arms_the_race() {
        let world = race_world();
        let mut gs = flush_docked();
        let target = targets_at(&gs, &world)[0];
        let stake = stake_between(&world.islands[0], target);
        let gold = gs.gold;
        gs.accept_race(&world, target.id).unwrap();
        assert_eq!(gs.gold, gold - stake);
        assert_eq!(
            gs.race,
            Some(Race {
                origin_id: 0,
                target_id: target.id,
                stake
            })
        );
    }

    #[test]
    fn accept_refuses_a_race_the_captain_cannot_stake() {
        let world = race_world();
        let mut gs = flush_docked();
        let target = targets_at(&gs, &world)[0];
        gs.gold = 1;
        assert_eq!(gs.accept_race(&world, target.id), Err(TradeError::NotEnoughGold));
    }

    #[test]
    fn accept_refuses_a_second_race_while_one_is_booked() {
        let world = race_world();
        let mut gs = flush_docked();
        let target = targets_at(&gs, &world)[0];
        gs.accept_race(&world, target.id).unwrap();
        assert_eq!(
            gs.accept_race(&world, target.id),
            Err(TradeError::RaceInProgress)
        );
    }

    #[test]
    fn accept_refuses_an_unknown_target() {
        let world = race_world();
        let mut gs = flush_docked();
        assert_eq!(gs.accept_race(&world, -1), Err(TradeError::NoSuchRace));
    }

    #[test]
    fn accept_refused_while_at_sea() {
        let world = race_world();
        let mut gs = flush_docked();
        let target = targets_at(&gs, &world)[0];
        gs.location = Location::Sailing;
        assert_eq!(gs.accept_race(&world, target.id), Err(TradeError::NotDocked));
    }

    #[test]
    fn win_hands_back_the_stake_doubled_and_clears_the_race() {
        let world = race_world();
        let mut gs = flush_docked();
        let target = targets_at(&gs, &world)[0];
        gs.accept_race(&world, target.id).unwrap();
        let gold = gs.gold;
        let stake = gs.race.unwrap().stake;
        gs.win_race();
        assert_eq!(gs.gold, gold + stake * 2);
        assert_eq!(gs.race, None);
    }

    #[test]
    fn lose_clears_the_race_without_refunding_the_stake() {
        let world = race_world();
        let mut gs = flush_docked();
        let target = targets_at(&gs, &world)[0];
        gs.accept_race(&world, target.id).unwrap();
        let gold = gs.gold;
        gs.lose_race();
        assert_eq!(gs.gold, gold);
        assert_eq!(gs.race, None);
    }

    #[test]
    fn withdraw_drops_a_booked_race_forfeiting_the_stake() {
        let world = race_world();
        let mut gs = flush_docked();
        let target = targets_at(&gs, &world)[0];
        gs.accept_race(&world, target.id).unwrap();
        let gold = gs.gold;
        gs.withdraw_race(&world).unwrap();
        assert_eq!(gs.race, None);
        assert_eq!(gs.gold, gold);
    }

    #[test]
    fn withdraw_refused_when_no_race_is_booked() {
        let world = race_world();
        let mut gs = flush_docked();
        assert_eq!(gs.withdraw_race(&world), Err(TradeError::NoRace));
    }

    #[test]
    fn rival_steers_straight_for_a_mark_it_can_fetch_off_the_wind() {
        // Mark due north, wind blowing toward the south-east — the rival can lay it.
        let kin = Kinematics::still(Vec2::ZERO, 0.0);
        let mark = Vec2::new(0.0, 1000.0);
        let helm = rival_helm(&kin, mark, Wind { toward_rad: -3.0 * PI / 4.0 });
        assert_eq!(helm.throttle, 1.0);
        assert!(helm.turn.abs() < 0.05);
    }

    #[test]
    fn rival_never_points_dead_into_the_wind_for_a_mark_dead_upwind() {
        // Mark due north, wind blowing toward the south (from the north) — straight
        // upwind to the mark. The rival must fall off to a sailable lay.
        let kin = Kinematics::still(Vec2::ZERO, 0.0);
        let mark = Vec2::new(0.0, 1000.0);
        let wind = Wind { toward_rad: PI };
        let helm = rival_helm(&kin, mark, wind);
        let lay = lay_heading(kin.pos.bearing_to(mark), wind);
        assert!(wind.factor(lay) > 0.0);
        assert!(helm.turn != 0.0);
    }
}
