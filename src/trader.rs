//! Wandering merchant NPCs — small trading craft that ply the local waters.
//!
//! Each cluster keeps a little fleet of [`Trader`]s (see [`TRADERS_PER_CLUSTER`]),
//! but only the fleet of the *current* cluster is ever simulated or drawn: as the
//! captain crosses the open sea to fresh waters the old fleet is forgotten and a
//! new one spun up (`TraderFleet::update`). They never stray far — there is no
//! point sailing a trader the player will never see.
//!
//! A trader runs a fixed circuit of three or four ports in its cluster, cycling
//! between them forever. It helms itself with the very same close-hauled tacking
//! logic the racing rival uses ([`crate::race::rival_helm`]) — beating up to a
//! mark that lies upwind rather than stalling in irons — and grazes off shores
//! like the player ([`crate::sailing::resolve_grounding`]). On reaching a port it
//! lies to for a minute or so ([`WAIT_MIN`]…[`WAIT_MAX`]) before setting off for
//! the next, so the waters always have a few sails crossing them.
//!
//! The route, the layover times and the starting berths are all drawn from a
//! deterministic [`Rng`] seeded off the world and cluster, so a given chart always
//! has the same traders running the same circuits.

use crate::geometry::Vec2;
use crate::race;
use crate::rng::Rng;
use crate::sailing::{self, Kinematics, Wind};
use crate::world::{Cluster, World};

/// How many traders ply each cluster's waters at once.
pub const TRADERS_PER_CLUSTER: usize = 3;

/// A trader's circuit is this many ports (clamped to however many the cluster
/// actually has — never fewer than two, or there is nowhere to sail).
const ROUTE_MIN: usize = 3;
const ROUTE_MAX: usize = 4;

/// Seconds a trader lies to in port before setting off for the next leg.
const WAIT_MIN: f32 = 60.0;
const WAIT_MAX: f32 = 90.0;

/// How close to a port's shore a trader must come to count as arrived — well
/// outside the hull-clearance keep-out so grounding never stops it short of the
/// berth, but tighter than the player's generous docking range.
const ARRIVE_MARGIN: f32 = 140.0;

/// Where a trader sits relative to a port's shore while loitering / at spawn.
const OFFSHORE: f32 = 90.0;

/// SplitMix64's golden gamma, reused (as in `world`/`race`) to spread seeds.
const GOLDEN: i64 = 0x9e3779b97f4a7c15u64 as i64;

/// A single merchant craft running a fixed circuit of ports in one cluster.
pub struct Trader {
    /// Live kinematics on the water (position, heading, velocity, yaw).
    pub kin: Kinematics,
    /// The island ids on the circuit, in visiting order. Length 2…[`ROUTE_MAX`].
    route: Vec<i32>,
    /// Index into `route` of the port currently being sailed to.
    leg: usize,
    /// Seconds left lying to in port (`> 0` = berthed, `0` = under way).
    wait: f32,
    /// Per-trader stream for layover lengths, kept deterministic per chart.
    rng: Rng,
}

impl Trader {
    /// The port this trader is presently making for.
    fn target<'w>(&self, world: &'w World) -> &'w crate::world::Island {
        &world.islands[self.route[self.leg] as usize]
    }

    /// Advance the trader by `dt` seconds: lie to while the layover runs, else
    /// beat toward the next port (tacking when it lies upwind) and graze off any
    /// shores. On fetching the port, start a fresh layover and turn for the leg
    /// after it.
    pub fn update(&mut self, world: &World, wind: Wind, dt: f32) {
        if self.route.len() < 2 {
            return;
        }
        if self.wait > 0.0 {
            self.wait -= dt;
            self.kin.vel = Vec2::ZERO;
            self.kin.yaw_rate = 0.0;
            return;
        }

        let target = self.target(world).clone();
        let helm = race::rival_helm(&self.kin, target.pos, wind);
        let stepped = sailing::step(self.kin, helm, wind, dt);
        let near = world.islands_near(stepped.pos, 400.0);
        self.kin = sailing::resolve_grounding(stepped, &near);

        if self.kin.pos.distance_to(target.pos) <= target.radius + ARRIVE_MARGIN {
            self.wait = self.rng.between(WAIT_MIN as f64, WAIT_MAX as f64) as f32;
            self.leg = (self.leg + 1) % self.route.len();
            // Point at the next mark so she sets off cleanly when the layover ends.
            self.kin.heading_rad = self.kin.pos.bearing_to(self.target(world).pos);
            self.kin.vel = Vec2::ZERO;
            self.kin.yaw_rate = 0.0;
        }
    }
}

/// Spin up a cluster's fleet of traders, each on its own deterministic circuit.
/// Returns an empty fleet for a cluster with too few ports to sail between.
fn spawn_fleet(world: &World, cluster: &Cluster) -> Vec<Trader> {
    let ports: Vec<&crate::world::Island> = world
        .cluster_islands(cluster)
        .into_iter()
        .filter(|i| i.is_port)
        .collect();
    if ports.len() < 2 {
        return Vec::new();
    }

    let mut traders = Vec::with_capacity(TRADERS_PER_CLUSTER);
    for ti in 0..TRADERS_PER_CLUSTER {
        let mut rng = Rng::from_seed(
            world.seed
                ^ (cluster.id as i64 + 1).wrapping_mul(GOLDEN)
                ^ (ti as i64 + 1).wrapping_mul(0x2545_f491_4f6c_dd1du64 as i64),
        );

        // A circuit of 3–4 ports (or however many the cluster has), chosen by a
        // partial Fisher–Yates shuffle of the cluster's ports.
        let want = rng.int_between(ROUTE_MIN as i32, ROUTE_MAX as i32 + 1) as usize;
        let len = want.min(ports.len()).max(2);
        let mut order: Vec<usize> = (0..ports.len()).collect();
        for i in (1..order.len()).rev() {
            let j = rng.int_between(0, (i + 1) as i32) as usize;
            order.swap(i, j);
        }
        let route: Vec<i32> = order[..len].iter().map(|&k| ports[k].id).collect();

        // Start her berthed off a random port on the circuit, bow already pointed
        // at the next, so the fleet is scattered and under way from the first frame.
        let start = rng.int_between(0, len as i32) as usize;
        let leg = (start + 1) % len;
        let from = &world.islands[route[start] as usize];
        let to = &world.islands[route[leg] as usize];
        let bearing = from.pos.bearing_to(to.pos);
        let pos = from.pos + Vec2::from_heading(bearing) * (from.radius + OFFSHORE);
        let kin = Kinematics::still(pos, pos.bearing_to(to.pos));

        traders.push(Trader {
            kin,
            route,
            leg,
            wait: 0.0,
            rng,
        });
    }
    traders
}

/// The traders of whatever cluster the player is currently in. Re-spawns its fleet
/// whenever the ship crosses into a new cluster, so only local traffic is ever
/// simulated.
pub struct TraderFleet {
    cluster_id: i32,
    traders: Vec<Trader>,
}

impl TraderFleet {
    /// Build the fleet for the cluster nearest `p`.
    pub fn new(world: &World, p: Vec2) -> Self {
        let cluster = world.cluster_at(p);
        TraderFleet {
            cluster_id: cluster.id,
            traders: spawn_fleet(world, cluster),
        }
    }

    /// Step every local trader; re-spawn the fleet first if the ship has crossed
    /// into a different cluster's waters.
    pub fn update(&mut self, world: &World, p: Vec2, wind: Wind, dt: f32) {
        let cluster = world.cluster_at(p);
        if cluster.id != self.cluster_id {
            self.cluster_id = cluster.id;
            self.traders = spawn_fleet(world, cluster);
        }
        for tr in &mut self.traders {
            tr.update(world, wind, dt);
        }
    }

    /// The live kinematics of every local trader (for the world renderer).
    pub fn kinematics(&self) -> Vec<Kinematics> {
        self.traders.iter().map(|t| t.kin).collect()
    }

    /// The chart positions of every local trader (for the minimap).
    pub fn positions(&self) -> Vec<Vec2> {
        self.traders.iter().map(|t| t.kin.pos).collect()
    }
}
