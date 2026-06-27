//! Shipyard taverns and their special wares.
//!
//! Every shipyard port (one per archipelago, see [`crate::world`]) has a tavern
//! that stocks a single special item: a one-off purchase that, once bought, rides
//! with the captain for the rest of the voyage. Some are **passive** keepsakes that
//! simply change how the ship sails (a chart unlocked in the log, richer salvage, a
//! price almanac); others are **active** abilities the captain invokes at the helm,
//! each recharging once a day (see [`crate::game_state::Inventory`]).
//!
//! Which tavern sells what is fixed per chart (so a captain can sail back for a ware
//! they passed up): the **home** shipyard — the one in the archipelago nearest the
//! origin, where the voyage begins — always stocks the World Map; the rest cycle
//! through the remaining wares in shipyard-id order. Ownership and the daily
//! cooldown live on [`crate::game_state::GameState`]; this module is just the
//! catalogue and the per-port assignment.

use crate::geometry::Vec2;
use crate::world::World;

/// A special ware sold at a shipyard tavern. Fieldless and ordered, so `as usize`
/// is its slot in the inventory arrays (see [`crate::game_state::Inventory`]) and in
/// the save — **do not reorder** without bumping the save format.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SpecialItem {
    /// A keepsake chart of the whole world, unlocking the captain's-log world-map
    /// spread and the **M** shortcut to it. The home tavern's ware.
    WorldMap,
    /// Pipe up a fresh wind on command, once a day: the prevailing breeze backs or
    /// veers to a new random quarter (the same shift the weather rolls on its own).
    WindWhistle,
    /// A swig that drives the ship on: after a moment's charge it hauls her up past her
    /// top speed (to her best plus a margin) on any point of sail, as long as canvas is
    /// set (full at full sail, nil with sails struck). Once a day.
    DolphinsDraught,
    /// Read the glass and calm the seas: the weather eases back to a flat calm,
    /// once a day.
    StormGlass,
    /// A trader's price book: the captain's log gains an Almanac spread comparing
    /// every port's prices across the local archipelago. Passive.
    TradersAlmanac,
    /// A lucky figurehead that draws coin from the sea: salvaged flotsam is worth
    /// half again as much. Passive.
    LuckyFigurehead,
}

use SpecialItem::*;

impl SpecialItem {
    /// Every ware, in inventory-slot order.
    pub const ALL: [SpecialItem; 6] =
        [WorldMap, WindWhistle, DolphinsDraught, StormGlass, TradersAlmanac, LuckyFigurehead];

    /// How many wares there are; the inventory arrays are this wide.
    pub const COUNT: usize = Self::ALL.len();

    /// This ware's slot in the inventory arrays / save (== its enum ordinal).
    #[inline]
    pub fn index(self) -> usize {
        self as usize
    }

    /// Tavern display name.
    pub fn name(self) -> &'static str {
        match self {
            WorldMap => "World Map",
            WindWhistle => "Wind Whistle",
            DolphinsDraught => "Dolphin's Draught",
            StormGlass => "Storm Glass",
            TradersAlmanac => "Trader's Almanac",
            LuckyFigurehead => "Lucky Figurehead",
        }
    }

    /// The price in gold the tavern asks for it.
    pub fn price(self) -> i32 {
        match self {
            WorldMap => 1000,
            WindWhistle => 5000,
            DolphinsDraught => 5000,
            StormGlass => 5000,
            TradersAlmanac => 5000,
            LuckyFigurehead => 5000,
        }
    }

    /// A short line of flavour + what the ware does, shown under its name on the
    /// tavern board.
    pub fn blurb(self) -> &'static str {
        match self {
            WorldMap => "A chart of every archipelago (press M), unveiling where each \
                         legendary trinket can be bought.",
            WindWhistle => "Pipe up a fresh wind from a new quarter. Once a day.",
            DolphinsDraught => "A swig that charges, then drives the ship on. Needs sail set. Once a day.",
            StormGlass => "Read the glass and calm the seas to a flat calm. Once a day.",
            TradersAlmanac => "Compare every port's prices in these waters, from your log.",
            LuckyFigurehead => "Draws coin from the sea: salvage is worth half again as much.",
        }
    }

    /// Active wares are invoked at the helm and recharge once a day; passive wares
    /// simply take effect once owned.
    pub fn is_active(self) -> bool {
        self.active_slot().is_some()
    }

    /// The helm slot (0-based) of an active ware, fixing its keybind and HUD button:
    /// slot `n` is triggered by the number key `n + 1`. `None` for passive wares.
    pub fn active_slot(self) -> Option<usize> {
        match self {
            WindWhistle => Some(0),
            DolphinsDraught => Some(1),
            StormGlass => Some(2),
            _ => None,
        }
    }

    /// The active ware bound to helm slot `slot` (the inverse of [`active_slot`]).
    pub fn from_active_slot(slot: usize) -> Option<SpecialItem> {
        Self::ALL.into_iter().find(|w| w.active_slot() == Some(slot))
    }

    /// The number-key hint for an active ware (e.g. `"1"`), for the tavern board and
    /// the helm HUD. `None` for passive wares.
    pub fn key_hint(self) -> Option<&'static str> {
        match self.active_slot()? {
            0 => Some("1"),
            1 => Some("2"),
            _ => Some("3"),
        }
    }

    /// A short glyph/label for the helm HUD button of an active ware.
    pub fn hud_label(self) -> &'static str {
        match self {
            WindWhistle => "WND",
            DolphinsDraught => "DASH",
            StormGlass => "CALM",
            _ => "",
        }
    }
}

/// The id of the home shipyard: the shipyard port in the archipelago nearest the
/// origin, where the voyage begins (see `main`'s start-isle pick). `None` only for a
/// pathological world with no shipyard at all.
pub fn home_shipyard_id(world: &World) -> Option<i32> {
    let home = world.cluster_at(Vec2::ZERO);
    world
        .cluster_islands(home)
        .into_iter()
        .find(|i| i.is_shipyard)
        .map(|i| i.id)
}

/// The special ware stocked by the tavern at island `id`, or `None` when it is not a
/// shipyard port. The home shipyard always sells the World Map; the other shipyards,
/// taken in id order, cycle through the remaining wares — so a given chart always
/// sells the same ware at the same harbour, and a captain can sail back for one.
pub fn item_at(world: &World, id: i32) -> Option<SpecialItem> {
    let isle = world.islands.get(id as usize)?;
    if !isle.is_shipyard {
        return None;
    }
    let home = home_shipyard_id(world);
    if Some(id) == home {
        return Some(WorldMap);
    }
    // The non-home shipyards, in id order, get the remaining wares (cycling if there
    // are more shipyards than wares).
    const WARES: [SpecialItem; 5] =
        [WindWhistle, DolphinsDraught, StormGlass, TradersAlmanac, LuckyFigurehead];
    let pos = world
        .islands
        .iter()
        .filter(|i| i.is_shipyard && Some(i.id) != home)
        .position(|i| i.id == id)?;
    Some(WARES[pos % WARES.len()])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_home_tavern_sells_the_world_map() {
        let world = crate::world::generate(1);
        let home = home_shipyard_id(&world).expect("a world has a home shipyard");
        assert_eq!(item_at(&world, home), Some(WorldMap));
    }

    #[test]
    fn every_shipyard_has_a_ware_and_only_home_sells_the_map() {
        let world = crate::world::generate(7);
        let home = home_shipyard_id(&world);
        let mut maps = 0;
        for isle in &world.islands {
            let ware = item_at(&world, isle.id);
            assert_eq!(
                ware.is_some(),
                isle.is_shipyard,
                "exactly the shipyards stock a ware"
            );
            if ware == Some(WorldMap) {
                maps += 1;
                assert_eq!(Some(isle.id), home, "only the home tavern sells the map");
            }
        }
        assert_eq!(maps, 1, "exactly one tavern sells the world map");
    }

    #[test]
    fn active_slots_round_trip_to_distinct_keys() {
        for slot in 0..3 {
            let w = SpecialItem::from_active_slot(slot).expect("a ware per active slot");
            assert_eq!(w.active_slot(), Some(slot));
            assert!(w.is_active());
            assert!(w.key_hint().is_some());
        }
        assert!(!WorldMap.is_active());
        assert!(WorldMap.key_hint().is_none());
    }
}
