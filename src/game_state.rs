//! The persisted voyage state and the port economy, ported from
//! `shared.{GameState, Goods, Trade, Upgrades, Hull}`.
//!
//! Per-frame ship kinematics stay in [`crate::sailing`]; only voyage *outcomes*
//! land here — gold, the hold's cargo, the rig/hold upgrades bought, the hull's
//! condition, and where the captain is (sailing or docked). Markets are
//! deterministic per island+good, so the same seed always offers the same
//! arbitrage. Haulage contracts (`shared.Mission`) ride here too — the accepted
//! contracts and their reserved hold. A booked wager race (`shared.Race`) rides
//! here too — the armed [`Race`] against the rival, settled when the player or the
//! rival reaches the mark (see [`crate::race`]).

use crate::mission::{self, Mission};
use crate::race::{self, Race};
use crate::rng::Rng;
use crate::world::{Island, World};

// --- Goods -------------------------------------------------------------------

/// A tradeable commodity with a baseline price in gold. Order matches Scala
/// `Good.values` (and so the cargo array's indexing) — do not reorder.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Good {
    Food,
    Rum,
    Sugar,
    Spice,
    Silk,
    Cotton,
    Tobacco,
}

impl Good {
    pub const ALL: [Good; 7] = [
        Good::Food,
        Good::Rum,
        Good::Sugar,
        Good::Spice,
        Good::Silk,
        Good::Cotton,
        Good::Tobacco,
    ];

    /// This good's slot in the cargo array (== Scala enum ordinal).
    #[inline]
    pub fn index(self) -> usize {
        self as usize
    }

    pub fn label(self) -> &'static str {
        match self {
            Good::Food => "Food",
            Good::Rum => "Rum",
            Good::Sugar => "Sugar",
            Good::Spice => "Spice",
            Good::Silk => "Silk",
            Good::Cotton => "Cotton",
            Good::Tobacco => "Tobacco",
        }
    }

    /// The good's baseline price before a port's ±45% jitter.
    pub fn base_price(self) -> i32 {
        match self {
            Good::Food => 4, // the galley's staple — cheap, eaten one ration per daytime
            Good::Rum => 18,
            Good::Sugar => 9,
            Good::Spice => 30,
            Good::Silk => 45,
            Good::Cotton => 12,
            Good::Tobacco => 24,
        }
    }
}

/// The price sheet at a single port: a price per good. Deterministic per
/// island+good (`Market.forIsland`).
#[derive(Clone, Debug)]
pub struct Market {
    pub prices: [i32; 7],
}

impl Market {
    /// Derive a port's prices by jittering each good's base price ±45%, in the
    /// same RNG draw order as Scala so a seed reproduces the chart's economy.
    pub fn for_island(island: &Island, seed: i64) -> Market {
        let mut rng = Rng::from_seed(seed ^ (island.id as i64).wrapping_mul(0x100000001b3));
        let mut prices = [0i32; 7];
        for (slot, good) in Good::ALL.iter().enumerate() {
            let factor = rng.between(0.55, 1.45);
            prices[slot] = ((good.base_price() as f64 * factor).round() as i32).max(1);
        }
        Market { prices }
    }

    pub fn price(&self, good: Good) -> i32 {
        self.prices[good.index()]
    }
}

// --- Trade / upgrade outcomes ------------------------------------------------

/// Why a trade or fitting could not be completed. The UI just no-ops on these,
/// so the message is only here for completeness / future surfacing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TradeError {
    NotDocked,
    NotEnoughGold,
    NotEnoughHold,
    NotEnoughCargo,
    NonPositive,
    NoShipyard,
    MaxUpgrade,
    HullSound,
    NoSuchMission,
    NoDelivery,
    RaceInProgress,
    NoSuchRace,
    NoRace,
}

/// Which fitting a shipyard upgrade improves.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UpgradeKind {
    Sail,
    Cargo,
}

impl UpgradeKind {
    pub fn label(self) -> &'static str {
        match self {
            UpgradeKind::Sail => "Sails",
            UpgradeKind::Cargo => "Hold",
        }
    }
}

// --- Economy / progression constants (match Scala) ---------------------------

pub const STARTING_GOLD: i32 = 200;
pub const BASE_CARGO: i32 = 16; // hold slots on a fresh ship
pub const CARGO_STEP: i32 = 4; // slots added per cargo upgrade
pub const STARTING_FOOD: i32 = 4;

const BASE_TOP_KNOTS: f64 = 24.0;
const KNOTS_PER_SAIL_LEVEL: f64 = 4.0;
const HAUL_BASE: i32 = 12;
const HAUL_PER_SAIL_LEVEL: i32 = 6;
const MAX_PENALTY: f64 = 0.66;

pub const MAX_LEVEL: i32 = 6;
const SAIL_BASE_COST: f64 = 300.0;
const CARGO_BASE_COST: f64 = 250.0;
const COST_GROWTH: f64 = 2.2;

const BASE_MAX_HULL: i32 = 180;
const HULL_PER_CARGO_LEVEL: i32 = 60;
const HULL_PER_SAIL_LEVEL: i32 = 40;
const REPAIR_COST_PER_HULL: i32 = 2;

// --- Upgrades (free functions; the logic of `shared.Upgrades`) ---------------

pub mod upgrades {
    use super::*;

    /// The cargo a rig at `sail_level` hauls at full speed before any penalty.
    pub fn max_haul(sail_level: i32) -> i32 {
        HAUL_BASE + sail_level * HAUL_PER_SAIL_LEVEL
    }

    /// Fraction of peak speed lost to an overladen hold, in [0, MAX_PENALTY].
    pub fn overload_penalty(sail_level: i32, load: i32) -> f64 {
        let haul = max_haul(sail_level);
        let over_ratio = (((load - haul) as f64) / haul as f64).clamp(0.0, 1.0);
        MAX_PENALTY * over_ratio
    }

    /// Peak speed (knots) for a rig at `sail_level` carrying `load` units.
    pub fn top_knots(sail_level: i32, load: i32) -> f64 {
        let peak = BASE_TOP_KNOTS + sail_level as f64 * KNOTS_PER_SAIL_LEVEL;
        peak * (1.0 - overload_penalty(sail_level, load))
    }

    /// Top speed relative to a fresh, lightly-laden ship — the multiplier the
    /// sailing engine scales its base top speed by, so a stronger rig runs
    /// faster and an overladen hull crawls. 1.0 for a bare ship within haulage.
    pub fn speed_scale(sail_level: i32, load: i32) -> f32 {
        (top_knots(sail_level, load) / BASE_TOP_KNOTS) as f32
    }

    pub fn cargo_capacity(cargo_level: i32) -> i32 {
        BASE_CARGO + cargo_level * CARGO_STEP
    }
    pub fn cargo_level_of(capacity: i32) -> i32 {
        (capacity - BASE_CARGO) / CARGO_STEP
    }

    pub fn sail_cost(level: i32) -> i32 {
        (SAIL_BASE_COST * COST_GROWTH.powi(level)).round() as i32
    }
    pub fn cargo_cost(level: i32) -> i32 {
        (CARGO_BASE_COST * COST_GROWTH.powi(level)).round() as i32
    }

    pub fn level_of(kind: UpgradeKind, s: &GameState) -> i32 {
        match kind {
            UpgradeKind::Sail => s.sail_level,
            UpgradeKind::Cargo => cargo_level_of(s.hold_capacity),
        }
    }

    /// The price of the next upgrade of `kind`, or None if maxed.
    pub fn next_cost(kind: UpgradeKind, s: &GameState) -> Option<i32> {
        let lvl = level_of(kind, s);
        if lvl >= MAX_LEVEL {
            None
        } else {
            Some(match kind {
                UpgradeKind::Sail => sail_cost(lvl),
                UpgradeKind::Cargo => cargo_cost(lvl),
            })
        }
    }

    /// A one-line description of the fitting's current → next effect.
    pub fn effect(kind: UpgradeKind, s: &GameState) -> String {
        match kind {
            UpgradeKind::Sail => {
                let now = BASE_TOP_KNOTS as i32 + s.sail_level * KNOTS_PER_SAIL_LEVEL as i32;
                let next = now + KNOTS_PER_SAIL_LEVEL as i32;
                format!(
                    "{now}->{next} kn  haul {}->{}",
                    max_haul(s.sail_level),
                    max_haul(s.sail_level + 1)
                )
            }
            UpgradeKind::Cargo => {
                format!("{}->{} slots", s.hold_capacity, s.hold_capacity + CARGO_STEP)
            }
        }
    }
}

// --- Hull (free functions; the logic of `shared.Hull`) -----------------------

pub mod hull {
    use super::*;

    /// Maximum hull for a ship at the given rig and hold levels.
    pub fn max_hull(sail_level: i32, cargo_level: i32) -> i32 {
        BASE_MAX_HULL + cargo_level * HULL_PER_CARGO_LEVEL + sail_level * HULL_PER_SAIL_LEVEL
    }

    pub fn fraction(s: &GameState) -> f64 {
        s.hull as f64 / s.max_hull() as f64
    }

    /// Hull points missing from a full hull.
    pub fn damage(s: &GameState) -> i32 {
        s.max_hull() - s.hull
    }

    /// Gold to mend the hull all the way back to sound.
    pub fn repair_cost(s: &GameState) -> i32 {
        damage(s) * REPAIR_COST_PER_HULL
    }
}

// --- Location & GameState ----------------------------------------------------

/// Where the captain currently is in the world.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Location {
    Docked(i32),
    Sailing,
}

/// The discrete, persisted voyage state. The world itself is owned separately
/// (the renderers borrow it); here we keep only what a voyage mutates.
#[derive(Clone, Debug)]
pub struct GameState {
    pub gold: i32,
    /// Units of each good in the hold, indexed by [`Good::index`].
    pub cargo: [i32; 7],
    pub hold_capacity: i32,
    pub location: Location,
    pub sail_level: i32, // rig upgrades bought; drives top speed (see `upgrades`)
    pub hull: i32,       // current hull integrity; worn by storms & starvation
    /// Accepted haulage contracts riding in the hold until delivered (see
    /// [`crate::mission`]). Their goods occupy hold space but cannot be sold.
    pub active_missions: Vec<Mission>,
    /// The wager race booked at a port, if any — armed until the player or the
    /// rival reaches the mark, or the captain withdraws (see [`crate::race`]).
    pub race: Option<Race>,
}

impl GameState {
    /// A fresh voyage with a starting purse, a small larder, and a modest hold.
    /// The captain begins sailing just off the home port (the view is set up by
    /// `main`); the home shipyard is found the same way the original did.
    pub fn start() -> GameState {
        let mut cargo = [0i32; 7];
        cargo[Good::Food.index()] = STARTING_FOOD;
        GameState {
            gold: STARTING_GOLD,
            cargo,
            hold_capacity: BASE_CARGO,
            location: Location::Sailing,
            sail_level: 0,
            hull: hull::max_hull(0, 0),
            active_missions: Vec::new(),
            race: None,
        }
    }

    pub fn quantity_of(&self, good: Good) -> i32 {
        self.cargo[good.index()]
    }
    pub fn cargo_used(&self) -> i32 {
        self.cargo.iter().sum()
    }
    /// Hold taken by mission goods in transit — they ride along until delivered.
    pub fn mission_hold(&self) -> i32 {
        self.active_missions.iter().map(|m| m.quantity).sum()
    }
    /// Hold space in use: ordinary cargo plus reserved mission cargo.
    pub fn hold_used(&self) -> i32 {
        self.cargo_used() + self.mission_hold()
    }
    pub fn hold_free(&self) -> i32 {
        self.hold_capacity - self.hold_used()
    }
    pub fn food(&self) -> i32 {
        self.quantity_of(Good::Food)
    }

    pub fn max_hull(&self) -> i32 {
        hull::max_hull(self.sail_level, upgrades::cargo_level_of(self.hold_capacity))
    }

    pub fn docked_island_id(&self) -> Option<i32> {
        match self.location {
            Location::Docked(id) => Some(id),
            Location::Sailing => None,
        }
    }

    pub fn docked_island<'w>(&self, world: &'w World) -> Option<&'w Island> {
        self.docked_island_id().map(|id| &world.islands[id as usize])
    }

    // --- Trade (the logic of `shared.Trade`) ---------------------------------

    /// Buy `qty` units of `good` at the given port price. Mutates on success.
    pub fn buy(&mut self, market: &Market, good: Good, qty: i32) -> Result<(), TradeError> {
        if qty <= 0 {
            return Err(TradeError::NonPositive);
        }
        let cost = market.price(good) * qty;
        if cost > self.gold {
            return Err(TradeError::NotEnoughGold);
        }
        if qty > self.hold_free() {
            return Err(TradeError::NotEnoughHold);
        }
        self.gold -= cost;
        self.cargo[good.index()] += qty;
        Ok(())
    }

    /// Buy as much `good` as the hold and purse allow.
    pub fn fill(&mut self, market: &Market, good: Good) -> Result<(), TradeError> {
        let price = market.price(good);
        let affordable = if price > 0 {
            self.gold / price
        } else {
            self.hold_free()
        };
        let qty = self.hold_free().min(affordable);
        self.buy(market, good, qty)
    }

    /// Sell `qty` units of `good` at the given port price. Mutates on success.
    pub fn sell(&mut self, market: &Market, good: Good, qty: i32) -> Result<(), TradeError> {
        if qty <= 0 {
            return Err(TradeError::NonPositive);
        }
        if qty > self.quantity_of(good) {
            return Err(TradeError::NotEnoughCargo);
        }
        self.gold += market.price(good) * qty;
        self.cargo[good.index()] -= qty;
        Ok(())
    }

    /// Sell every unit of `good` held.
    pub fn dump(&mut self, market: &Market, good: Good) -> Result<(), TradeError> {
        self.sell(market, good, self.quantity_of(good))
    }

    // --- Shipyard / drydock (the logic of `shared.Upgrades` / `shared.Hull`) --

    /// Buy the next upgrade of `kind`. Requires being docked at a shipyard with
    /// the funds and the fitting not already maxed.
    pub fn buy_upgrade(&mut self, world: &World, kind: UpgradeKind) -> Result<(), TradeError> {
        let isle = self.docked_island(world).ok_or(TradeError::NotDocked)?;
        if !isle.is_shipyard {
            return Err(TradeError::NoShipyard);
        }
        let cost = upgrades::next_cost(kind, self).ok_or(TradeError::MaxUpgrade)?;
        if cost > self.gold {
            return Err(TradeError::NotEnoughGold);
        }
        self.gold -= cost;
        match kind {
            UpgradeKind::Sail => self.sail_level += 1,
            UpgradeKind::Cargo => self.hold_capacity += CARGO_STEP,
        }
        Ok(())
    }

    /// Mend the hull at the docked port, repairing as much as the purse allows.
    pub fn repair(&mut self) -> Result<(), TradeError> {
        if self.docked_island_id().is_none() {
            return Err(TradeError::NotDocked);
        }
        let damage = hull::damage(self);
        if damage <= 0 {
            return Err(TradeError::HullSound);
        }
        let affordable = self.gold / REPAIR_COST_PER_HULL;
        let points = damage.min(affordable);
        if points <= 0 {
            return Err(TradeError::NotEnoughGold);
        }
        self.gold -= points * REPAIR_COST_PER_HULL;
        self.hull += points;
        Ok(())
    }

    // --- Missions (the logic of `shared.Missions`) ---------------------------

    /// Take a contract from the board: pay the deposit and load the goods into
    /// the hold. (`Missions.accept`.)
    pub fn accept(&mut self, world: &World, mission_id: i32) -> Result<(), TradeError> {
        if self.docked_island_id().is_none() {
            return Err(TradeError::NotDocked);
        }
        let mission = mission::offered_at(self, world)
            .into_iter()
            .find(|m| m.id == mission_id)
            .ok_or(TradeError::NoSuchMission)?;
        if mission.deposit > self.gold {
            return Err(TradeError::NotEnoughGold);
        }
        if mission.quantity > self.hold_free() {
            return Err(TradeError::NotEnoughHold);
        }
        self.gold -= mission.deposit;
        self.active_missions.push(mission);
        Ok(())
    }

    /// Hand in a contract at its destination: return the deposit plus the reward
    /// and free the hold the goods occupied. (`Missions.deliver`.)
    pub fn deliver(&mut self, world: &World, mission_id: i32) -> Result<(), TradeError> {
        let isle_id = self.docked_island(world).ok_or(TradeError::NotDocked)?.id;
        let mission = self
            .active_missions
            .iter()
            .copied()
            .find(|m| m.id == mission_id && m.target_id == isle_id)
            .ok_or(TradeError::NoDelivery)?;
        self.gold += mission.deposit + mission.reward;
        self.active_missions.retain(|m| m.id != mission.id);
        Ok(())
    }

    /// Abandon an accepted contract: forfeit the deposit and reward, but keep the
    /// goods — they convert from mission-bound cargo into ordinary, sellable
    /// cargo. Hold usage is unchanged: the reserved space becomes free cargo.
    /// (`Missions.abandon`.)
    pub fn abandon(&mut self, world: &World, mission_id: i32) -> Result<(), TradeError> {
        if self.docked_island(world).is_none() {
            return Err(TradeError::NotDocked);
        }
        let mission = self
            .active_missions
            .iter()
            .copied()
            .find(|m| m.id == mission_id)
            .ok_or(TradeError::NoSuchMission)?;
        self.active_missions.retain(|m| m.id != mission.id);
        self.cargo[mission.good.index()] += mission.quantity;
        Ok(())
    }

    // --- Races (the logic of `shared.Race`) ----------------------------------

    /// Book a race: charge the distance-fixed stake up front and arm the race
    /// against the named target. Refused while at sea, while another race is
    /// already booked, or without the gold for the wager. (`Race.accept`.)
    pub fn accept_race(&mut self, world: &World, target_id: i32) -> Result<(), TradeError> {
        let origin = self.docked_island(world).ok_or(TradeError::NotDocked)?;
        let origin_id = origin.id;
        let origin_pos = origin.pos;
        if self.race.is_some() {
            return Err(TradeError::RaceInProgress);
        }
        let target = race::targets_at(self, world)
            .into_iter()
            .find(|p| p.id == target_id)
            .ok_or(TradeError::NoSuchRace)?;
        let wager = race::stake_for(origin_pos.distance_to(target.pos));
        if wager > self.gold {
            return Err(TradeError::NotEnoughGold);
        }
        self.gold -= wager;
        self.race = Some(Race {
            origin_id,
            target_id,
            stake: wager,
        });
        Ok(())
    }

    /// Settle a race the player has won: hand back the stake doubled (the wager
    /// plus its match) and clear the race. A no-op when no race runs. (`Race.win`.)
    pub fn win_race(&mut self) {
        if let Some(r) = self.race {
            self.gold += r.stake * 2;
            self.race = None;
        }
    }

    /// Settle a race the player has lost — the stake was already forfeited when the
    /// race was booked, so this only clears it. (`Race.lose`.)
    pub fn lose_race(&mut self) {
        self.race = None;
    }

    /// Abandon a booked race at port before it has started — the stake is handed
    /// back in full, so calling it off costs the captain nothing. (Once the rival is
    /// on the water the race is settled by win/lose instead.) Refused at sea or with
    /// no race booked. (`Race.withdraw`.)
    pub fn withdraw_race(&mut self, world: &World) -> Result<(), TradeError> {
        self.docked_island(world).ok_or(TradeError::NotDocked)?;
        let Some(r) = self.race else {
            return Err(TradeError::NoRace);
        };
        self.gold += r.stake;
        self.race = None;
        Ok(())
    }
}
