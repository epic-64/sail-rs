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
    /// Ship's timber: an ordinary, tradeable commodity that doubles as a field
    /// repair — caulk the hull with a plank at sea for [`GameState::HULL_PER_PLANK`]
    /// points (see [`GameState::caulk_with_plank`]). Appended last so the existing
    /// goods keep their cargo-array ordinals.
    Plank,
}

impl Good {
    pub const ALL: [Good; 8] = [
        Good::Food,
        Good::Rum,
        Good::Sugar,
        Good::Spice,
        Good::Silk,
        Good::Cotton,
        Good::Tobacco,
        Good::Plank,
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
            Good::Plank => "Planks",
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
            // A plank mends 10 hull points; the drydock charges 2 g/point (20 g for
            // the same), so timber priced just under that is a worthwhile thing to
            // stock for repairs far from a shipyard.
            Good::Plank => 15,
        }
    }
}

/// The price sheet at a single port: a price per good. Deterministic per
/// island+good (`Market.forIsland`).
#[derive(Clone, Debug)]
pub struct Market {
    pub prices: [i32; 8],
}

impl Market {
    /// Derive a port's prices by jittering each good's base price ±45%, in the
    /// same RNG draw order as Scala so a seed reproduces the chart's economy.
    pub fn for_island(island: &Island, seed: i64) -> Market {
        let mut rng = Rng::from_seed(seed ^ (island.id as i64).wrapping_mul(0x100000001b3));
        let mut prices = [0i32; 8];
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
    /// The hull is too battered to honourably take on a race or contract.
    HullTooDamaged,
    NoSuchMission,
    NoDelivery,
    RaceInProgress,
    NoSuchRace,
    NoRace,
    /// The race demands a sturdier hull than the captain has fitted.
    HullTierTooLow,
}

/// Which fitting a shipyard upgrade improves.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UpgradeKind {
    /// A sturdier hull — the *only* fitting that raises top speed (and with it the
    /// hull's points, hence its upkeep). See [`upgrades::peak_knots`].
    Hull,
    /// A taller rig — raises only the haul the ship carries before the overload
    /// penalty. See [`upgrades::max_haul`].
    Sail,
    /// A deeper hold — more cargo slots.
    Cargo,
}

impl UpgradeKind {
    pub fn label(self) -> &'static str {
        match self {
            UpgradeKind::Hull => "Hull",
            UpgradeKind::Sail => "Sails",
            UpgradeKind::Cargo => "Hold",
        }
    }
}

// --- Economy / progression constants (match Scala) ---------------------------

pub const STARTING_GOLD: i32 = 200;
pub const BASE_CARGO: i32 = 16; // hold slots on a fresh ship
pub const CARGO_STEP: i32 = 8; // slots added per cargo upgrade

// The single source of the base top speed is the sailing engine's
// `BASE_TOP_KNOTS` (an exact integer-valued f32, so the `as f64` is lossless); the
// economy reads it back here in knots. Don't redefine it — keep one number.
const BASE_TOP_KNOTS: f64 = crate::sailing::BASE_TOP_KNOTS as f64;
/// Top speed gained per hull tier — the hull upgrade *alone* drives speed now:
/// 24 / 29 / 34 / 39 kn across the four tiers. Sails no longer touch speed.
const KNOTS_PER_HULL_LEVEL: f64 = 5.0;
const HAUL_BASE: i32 = 12;
/// Haul tolerance gained per sail tier — sails *only* raise the load the rig can
/// carry before the overload penalty bites. Deliberately less than the +8 slots a
/// cargo upgrade adds, so haulers must over-invest in sails to keep a growing hold
/// at full speed; a racer (light hold) barely needs them.
const HAUL_PER_SAIL_LEVEL: i32 = 4;
const MAX_PENALTY: f64 = 0.66;

/// Top tier (0-indexed) for the sails and the hold — six steps each.
pub const MAX_LEVEL: i32 = 6;
/// Top hull tier (0-indexed): four tiers in all, Lv 1–4 to the captain.
pub const HULL_MAX_LEVEL: i32 = 3;
const SAIL_BASE_COST: f64 = 300.0;
const CARGO_BASE_COST: f64 = 250.0;
/// The hull is the premium fitting (it buys speed *and* hull points), so it is the
/// dearest to step up.
const HULL_BASE_COST: f64 = 500.0;
const COST_GROWTH: f64 = 2.2;

const BASE_MAX_HULL: i32 = 180;
/// Hull points gained per hull tier: 180 / 240 / 300 / 360. Because wear is a
/// *fraction* of the (now larger) hull, a sturdier ship costs proportionally more
/// to keep mended — the higher upkeep the captain trades for the speed.
const HULL_PER_HULL_LEVEL: i32 = 60;
const REPAIR_COST_PER_HULL: i32 = 2;
/// Hull points mended by caulking with a single plank from the hold (a field
/// repair the captain makes from the log; see [`GameState::caulk_with_plank`]).
const HULL_PER_PLANK: i32 = 10;

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

    /// The hull tier's peak speed (knots) before any overload penalty — the headline
    /// number the hull upgrade buys: 24 / 29 / 34 / 39 across the four tiers.
    pub fn peak_knots(hull_level: i32) -> f64 {
        BASE_TOP_KNOTS + hull_level as f64 * KNOTS_PER_HULL_LEVEL
    }

    /// Peak speed (knots) for a ship at `hull_level` whose `sail_level` rig is
    /// carrying `load` units. Speed comes from the hull tier; the sails only set how
    /// much can ride before [`overload_penalty`] trims that peak.
    pub fn top_knots(hull_level: i32, sail_level: i32, load: i32) -> f64 {
        peak_knots(hull_level) * (1.0 - overload_penalty(sail_level, load))
    }

    /// The ship's peak speed in engine units (m/s) — [`top_knots`] converted for
    /// the sailing engine, which the ship hands to [`crate::sailing::step_with`] /
    /// [`crate::sailing::step_debuffed`] as its own ceiling. A better hull runs
    /// faster, an overladen hold crawls; a bare ship within haulage makes
    /// [`crate::sailing::BASE_TOP_SPEED`].
    pub fn top_speed(hull_level: i32, sail_level: i32, load: i32) -> f32 {
        (top_knots(hull_level, sail_level, load) * crate::sailing::KNOT as f64) as f32
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
    pub fn hull_cost(level: i32) -> i32 {
        (HULL_BASE_COST * COST_GROWTH.powi(level)).round() as i32
    }

    pub fn level_of(kind: UpgradeKind, s: &GameState) -> i32 {
        match kind {
            UpgradeKind::Hull => s.hull_level,
            UpgradeKind::Sail => s.sail_level,
            UpgradeKind::Cargo => cargo_level_of(s.hold_capacity),
        }
    }

    /// The top tier a fitting can reach (0-indexed). The hull stops at four tiers;
    /// the sails and hold run the full ladder.
    pub fn max_level(kind: UpgradeKind) -> i32 {
        match kind {
            UpgradeKind::Hull => HULL_MAX_LEVEL,
            UpgradeKind::Sail | UpgradeKind::Cargo => MAX_LEVEL,
        }
    }

    /// The price of the next upgrade of `kind`, or None if maxed.
    pub fn next_cost(kind: UpgradeKind, s: &GameState) -> Option<i32> {
        let lvl = level_of(kind, s);
        if lvl >= max_level(kind) {
            None
        } else {
            Some(match kind {
                UpgradeKind::Hull => hull_cost(lvl),
                UpgradeKind::Sail => sail_cost(lvl),
                UpgradeKind::Cargo => cargo_cost(lvl),
            })
        }
    }
}

// --- Hull (free functions; the logic of `shared.Hull`) -----------------------

pub mod hull {
    use super::*;

    /// Maximum hull for a ship at the given hull tier. Only the hull upgrade enlarges
    /// the planking now — a taller rig or deeper hold leave the structure unchanged.
    pub fn max_hull(hull_level: i32) -> i32 {
        BASE_MAX_HULL + hull_level * HULL_PER_HULL_LEVEL
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

    // --- Condition penalties --------------------------------------------------
    // As the hull rots, handicaps stack one notch for every 10% of hull lost
    // below 90%, cycling through three kinds: a wider no-go zone, a slower helm,
    // then a lower top speed — and repeating down the scale. So the first bite at
    // 90% widens the no-go zone, 80% slows the turn, 70% trims top speed, 60%
    // widens the no-go again, and so on to 10%.

    /// Degrees added to each side of the no-go zone per deadzone notch.
    pub const DEADZONE_STEP_DEG: f32 = 5.0;
    /// Top-speed lost per speed notch (fraction).
    pub const SPEED_STEP: f32 = 0.05;
    /// Turn-rate lost per helm notch (fraction).
    pub const TURN_STEP: f32 = 0.10;
    /// At or below this fraction of hull the harbourmaster won't let the captain
    /// take on races or haulage — sailing a job in such a wreck is unseemly.
    pub const JOB_REFUSE_FRACTION: f64 = 0.30;

    /// How many notches of each penalty kind the hull's `fraction` has earned,
    /// returned as `(deadzone, turn, speed)`. The single source of truth for both
    /// the physics ([`debuff`]) and the captain's log ([`penalty_lines`]).
    pub fn penalty_counts(fraction: f64) -> (i32, i32, i32) {
        let pct = fraction * 100.0;
        if pct > 90.0 {
            return (0, 0, 0);
        }
        // Notch 0 bites at 90%, one more for every further 10% lost, down to 10%.
        let crossed = (((90.0 - pct) / 10.0).floor() as i32 + 1).clamp(0, 9);
        let mut counts = [0i32; 3];
        for i in 0..crossed {
            counts[(i % 3) as usize] += 1;
        }
        (counts[0], counts[1], counts[2])
    }

    /// The handling penalties currently in force, for the helm/physics.
    pub fn debuff(fraction: f64) -> crate::sailing::HullDebuff {
        let (dz, turn, spd) = penalty_counts(fraction);
        crate::sailing::HullDebuff {
            dead_angle_extra: (dz as f32) * DEADZONE_STEP_DEG.to_radians(),
            turn_mult: (1.0 - TURN_STEP * turn as f32).max(0.1),
            speed_mult: (1.0 - SPEED_STEP * spd as f32).max(0.1),
        }
    }

    /// Human-readable lines describing the penalties in force, for the captain's
    /// log. Empty while the hull is sound (above 90%).
    pub fn penalty_lines(fraction: f64) -> Vec<(String, String)> {
        let (dz, turn, spd) = penalty_counts(fraction);
        let mut lines = Vec::new();
        if dz > 0 {
            lines.push((
                "No-go zone".to_string(),
                format!("+{}°", (dz as f32 * DEADZONE_STEP_DEG) as i32),
            ));
        }
        if turn > 0 {
            lines.push(("Turn rate".to_string(), format!("-{}%", turn * 10)));
        }
        if spd > 0 {
            lines.push(("Top speed".to_string(), format!("-{}%", spd * 5)));
        }
        lines
    }

    /// Whether the hull is sound enough to take on races and haulage contracts.
    pub fn can_take_jobs(s: &GameState) -> bool {
        fraction(s) > JOB_REFUSE_FRACTION
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
    pub cargo: [i32; 8],
    pub hold_capacity: i32,
    pub location: Location,
    pub hull_level: i32, // hull tier bought; drives top speed & max hull (see `upgrades`)
    pub sail_level: i32, // sail tier bought; drives haul tolerance only (see `upgrades`)
    pub hull: i32,       // current hull integrity; worn by storms & starvation
    /// Fractional hull wear sailed off but not yet a whole point. The hull is
    /// integer-valued, so distance decay (see [`GameState::wear_distance`])
    /// banks its remainder here and spends it a point at a time.
    pub hull_wear: f64,
    /// Accepted haulage contracts riding in the hold until delivered (see
    /// [`crate::mission`]). Their goods occupy hold space but cannot be sold.
    pub active_missions: Vec<Mission>,
    /// The wager race booked at a port, if any — armed until the player or the
    /// rival reaches the mark, or the captain withdraws (see [`crate::race`]).
    pub race: Option<Race>,
    /// The captain's lifetime tally, kept across the whole voyage and shown on the
    /// captain's log's record page. Persisted with the rest of the state.
    pub stats: Stats,
}

/// The running ledger of a captain's career: contracts honoured, wagers won and
/// lost, and the sea-miles logged. Accumulated for the life of a voyage (never
/// reset by a refit or a trade) and surfaced on the captain's log's record spread.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stats {
    /// Haulage contracts delivered to their destination (see [`GameState::deliver`]).
    pub contracts_fulfilled: u32,
    /// Gold earned from contract *rewards* over the voyage (the returned deposit is
    /// not a gain, so it isn't counted) (see [`GameState::deliver`]).
    pub contract_earnings: i64,
    /// Wager races reached first (see [`GameState::win_race`]).
    pub races_won: u32,
    /// Wager races the rival reached first (see [`GameState::lose_race`]).
    pub races_lost: u32,
    /// Net gold from racing: the stake won on each victory, less the stake forfeit
    /// on each defeat. Signed, so a losing record shows red (see [`GameState::win_race`]
    /// / [`GameState::lose_race`]).
    pub race_winnings: i64,
    /// Whole metres sailed over the voyage, banked from [`GameState::wear_distance`]
    /// and shown as kilometres. Held as a float so short hops accumulate exactly.
    pub meters_traveled: f64,
}

impl GameState {
    /// A fresh voyage with a starting purse, an empty hold, and a modest hull.
    /// The captain begins sailing just off the home port (the view is set up by
    /// `main`); the home shipyard is found the same way the original did.
    pub fn start() -> GameState {
        GameState {
            gold: STARTING_GOLD,
            cargo: [0i32; 8],
            hold_capacity: BASE_CARGO,
            location: Location::Sailing,
            hull_level: 0,
            sail_level: 0,
            hull: hull::max_hull(0),
            hull_wear: 0.0,
            active_missions: Vec::new(),
            race: None,
            stats: Stats::default(),
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

    pub fn max_hull(&self) -> i32 {
        hull::max_hull(self.hull_level)
    }

    /// Wear the hull from distance sailed: every kilometre under way costs 1%
    /// of a full hull. Damage is accrued continuously and banked in
    /// `hull_wear`, spending whole points off `hull` as they accumulate, so
    /// even a string of short hops grinds the planking down over a voyage.
    /// `meters` is the world-distance (1 unit = 1 m) covered this step.
    pub fn wear_distance(&mut self, meters: f64) {
        if meters <= 0.0 {
            return;
        }
        // Log the sea-miles first, before the hull guard: distance is tallied for
        // the captain's record even on a wrecked (0-hull) hull that's still adrift.
        self.stats.meters_traveled += meters;
        if self.hull <= 0 {
            return;
        }
        let km = meters / 1000.0;
        self.hull_wear += km * self.max_hull() as f64 * 0.01;
        if self.hull_wear >= 1.0 {
            let points = self.hull_wear.floor() as i32;
            self.hull_wear -= points as f64;
            self.hull = (self.hull - points).max(0);
        }
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
    ///
    /// Every upgrade enlarges the hull ([`max_hull`](GameState::max_hull) grows
    /// with the rig and hold), and the new fittings come sound: the yard hauls her
    /// out and patches the hull to full as part of the work, so an upgrade never
    /// leaves the captain a smaller *fraction* of hull than before.
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
            UpgradeKind::Hull => self.hull_level += 1,
            UpgradeKind::Sail => self.sail_level += 1,
            UpgradeKind::Cargo => self.hold_capacity += CARGO_STEP,
        }
        // Fresh, sound planking: the refit patches the hull all the way back up.
        self.hull = self.max_hull();
        self.hull_wear = 0.0;
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

    /// Hull points mended by a single plank — surfaced so the captain's log can
    /// caption the field repair.
    pub const HULL_PER_PLANK: i32 = HULL_PER_PLANK;

    /// Caulk the hull at sea with one plank from the hold, mending
    /// [`HULL_PER_PLANK`](Self::HULL_PER_PLANK) points (capped at the hull's
    /// maximum). This is the captain's-log field repair — no gold and no drydock,
    /// just timber spent from the cargo. Refused with no planks aboard or a hull
    /// already sound (so a plank is never wasted patching a full hull).
    pub fn caulk_with_plank(&mut self) -> Result<(), TradeError> {
        if self.quantity_of(Good::Plank) <= 0 {
            return Err(TradeError::NotEnoughCargo);
        }
        if hull::damage(self) <= 0 {
            return Err(TradeError::HullSound);
        }
        self.cargo[Good::Plank.index()] -= 1;
        self.hull = (self.hull + HULL_PER_PLANK).min(self.max_hull());
        Ok(())
    }

    // --- Missions (the logic of `shared.Missions`) ---------------------------

    /// Take a contract from the board: pay the deposit and load the goods into
    /// the hold. (`Missions.accept`.)
    pub fn accept(&mut self, world: &World, mission_id: i32) -> Result<(), TradeError> {
        if self.docked_island_id().is_none() {
            return Err(TradeError::NotDocked);
        }
        if !hull::can_take_jobs(self) {
            return Err(TradeError::HullTooDamaged);
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
        self.stats.contracts_fulfilled += 1;
        self.stats.contract_earnings += mission.reward as i64;
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
        if !hull::can_take_jobs(self) {
            return Err(TradeError::HullTooDamaged);
        }
        let target = race::targets_at(self, world)
            .into_iter()
            .find(|p| p.id == target_id)
            .ok_or(TradeError::NoSuchRace)?;
        let distance = origin_pos.distance_to(target.pos);
        // The longer the leg the sturdier the hull the harbour demands — and the
        // tougher the rival and the richer the purse (see `race::offer_terms`).
        let required_level = race::required_level_for(distance);
        if self.hull_level < required_level {
            return Err(TradeError::HullTierTooLow);
        }
        let wager = race::stake_for(distance);
        if wager > self.gold {
            return Err(TradeError::NotEnoughGold);
        }
        self.gold -= wager;
        self.race = Some(Race {
            origin_id,
            target_id,
            stake: wager,
            required_level,
        });
        Ok(())
    }

    /// Settle a race the player has won: hand back the stake doubled (the wager
    /// plus its match) and clear the race. A no-op when no race runs. (`Race.win`.)
    pub fn win_race(&mut self) {
        if let Some(r) = self.race {
            self.gold += r.stake * 2;
            self.race = None;
            self.stats.races_won += 1;
            // The stake was forfeit on booking; winning hands back double, so the
            // net gain is the stake itself.
            self.stats.race_winnings += r.stake as i64;
        }
    }

    /// Settle a race the player has lost — the stake was already forfeited when the
    /// race was booked, so this only clears it. (`Race.lose`.)
    pub fn lose_race(&mut self) {
        if let Some(r) = self.race.take() {
            self.stats.races_lost += 1;
            // The stake was already forfeit when the race was booked: a clear loss.
            self.stats.race_winnings -= r.stake as i64;
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_wear_costs_one_percent_of_full_hull_per_km() {
        let mut gs = GameState::start();
        let full = gs.max_hull();
        // 10 km should knock off ~10% of a full hull.
        gs.wear_distance(10_000.0);
        let lost = full - gs.hull;
        let expected = (full as f64 * 0.10).floor() as i32;
        assert_eq!(lost, expected);
    }

    #[test]
    fn distance_wear_banks_the_fractional_remainder() {
        let mut gs = GameState::start();
        let full = gs.max_hull();
        // A string of short hops totalling 1 km still wears a whole ~1% off, even
        // though no single hop crosses a full hull point on its own.
        for _ in 0..100 {
            gs.wear_distance(10.0); // 10 m each, 1 km total
        }
        assert_eq!(full - gs.hull, (full as f64 * 0.01).floor() as i32);
    }

    #[test]
    fn penalties_stack_one_notch_per_ten_percent_cycling_kinds() {
        use hull::penalty_counts;
        // Sound above 90%.
        assert_eq!(penalty_counts(1.00), (0, 0, 0));
        assert_eq!(penalty_counts(0.95), (0, 0, 0));
        // 90% widens the no-go zone; 80% adds a slow helm; 70% trims top speed.
        assert_eq!(penalty_counts(0.90), (1, 0, 0));
        assert_eq!(penalty_counts(0.80), (1, 1, 0));
        assert_eq!(penalty_counts(0.70), (1, 1, 1));
        // …then the cycle repeats down the scale.
        assert_eq!(penalty_counts(0.60), (2, 1, 1));
        assert_eq!(penalty_counts(0.30), (3, 2, 2));
        assert_eq!(penalty_counts(0.10), (3, 3, 3));
    }

    #[test]
    fn debuff_translates_notches_into_handling_numbers() {
        // At 70% hull: +5° no-go, -10% turn, -5% top speed.
        let d = hull::debuff(0.70);
        assert!((d.dead_angle_extra - 5f32.to_radians()).abs() < 1e-6);
        assert!((d.turn_mult - 0.90).abs() < 1e-6);
        assert!((d.speed_mult - 0.95).abs() < 1e-6);
    }

    #[test]
    fn buying_an_upgrade_refits_the_hull_to_full() {
        let world = crate::world::generate(1);
        // Dock at a shipyard so the upgrade is allowed.
        let yard = world.islands.iter().find(|i| i.is_shipyard).unwrap();
        let mut gs = GameState::start();
        gs.gold = 1_000_000;
        gs.location = Location::Docked(yard.id);
        gs.hull = gs.max_hull() / 4; // battered before the refit
        let before = gs.max_hull();
        gs.buy_upgrade(&world, UpgradeKind::Hull).unwrap();
        // The hull tier enlarges the planking, and it comes sound — full, not the
        // old quarter, and bigger than before.
        assert!(gs.max_hull() > before);
        assert_eq!(gs.hull, gs.max_hull());
        assert_eq!(gs.hull_wear, 0.0);
    }

    #[test]
    fn a_plank_caulks_ten_hull_points_and_is_spent() {
        let mut gs = GameState::start();
        gs.cargo[Good::Plank.index()] = 2;
        gs.hull = gs.max_hull() - 25;
        gs.caulk_with_plank().unwrap();
        // One plank spent, ten points mended.
        assert_eq!(gs.quantity_of(Good::Plank), 1);
        assert_eq!(gs.hull, gs.max_hull() - 15);
    }

    #[test]
    fn caulking_never_overfills_the_hull_and_refuses_a_sound_one() {
        let mut gs = GameState::start();
        gs.cargo[Good::Plank.index()] = 1;
        // Only 4 points down: the plank tops it off without overshooting.
        gs.hull = gs.max_hull() - 4;
        gs.caulk_with_plank().unwrap();
        assert_eq!(gs.hull, gs.max_hull());
        assert_eq!(gs.quantity_of(Good::Plank), 0);
        // A second caulk on a sound hull is refused — and no plank is wasted.
        gs.cargo[Good::Plank.index()] = 1;
        assert_eq!(gs.caulk_with_plank(), Err(TradeError::HullSound));
        assert_eq!(gs.quantity_of(Good::Plank), 1);
    }

    #[test]
    fn caulking_without_timber_is_refused() {
        let mut gs = GameState::start();
        gs.hull = gs.max_hull() / 2;
        assert_eq!(gs.caulk_with_plank(), Err(TradeError::NotEnoughCargo));
    }

    #[test]
    fn lifetime_stats_tally_distance_races_and_contracts() {
        let mut gs = GameState::start();
        // Distance is logged in metres, even once the hull is wrecked and adrift.
        gs.wear_distance(2_500.0);
        gs.hull = 0;
        gs.wear_distance(500.0);
        assert!((gs.stats.meters_traveled - 3_000.0).abs() < 1e-9);

        // A won race counts once, banks the stake as winnings, and clears the wager.
        gs.race = Some(Race { origin_id: 0, target_id: 1, stake: 100, required_level: 0 });
        gs.win_race();
        assert_eq!(gs.stats.races_won, 1);
        assert_eq!(gs.stats.race_winnings, 100);
        assert!(gs.race.is_none());
        // A loss forfeits the stake: net winnings fall back to zero.
        gs.race = Some(Race { origin_id: 0, target_id: 1, stake: 100, required_level: 0 });
        gs.lose_race();
        assert_eq!(gs.stats.races_lost, 1);
        assert_eq!(gs.stats.race_winnings, 0);
        // With no race afoot, settling again is a no-op and doesn't pad the tally.
        gs.win_race();
        gs.lose_race();
        assert_eq!((gs.stats.races_won, gs.stats.races_lost), (1, 1));
        assert_eq!(gs.stats.race_winnings, 0);
    }

    #[test]
    fn a_battered_hull_is_barred_from_jobs_at_thirty_percent() {
        let mut gs = GameState::start();
        gs.location = Location::Docked(0);
        gs.hull = (gs.max_hull() as f64 * 0.31).round() as i32;
        assert!(hull::can_take_jobs(&gs));
        gs.hull = (gs.max_hull() as f64 * 0.30).round() as i32;
        assert!(!hull::can_take_jobs(&gs));
    }
}
