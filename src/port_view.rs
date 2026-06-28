//! Docking and the port overlay, ported in spirit from `client.PortView` (plus the
//! docking handshake from `client.SailingView`).
//!
//! Sail a port within its dock range with the bow pointed at it and the sails
//! struck, press **Space**, and the captain ties up: the world keeps running
//! underneath while a parchment board opens over it. The board has three tabs:
//!
//!   - **Market** — buy and sell the seven goods at this port's deterministic
//!     prices (Buy/Fill/Dump/Sell).
//!   - **Contracts** — accept haulage jobs out of this port, hand in deliveries
//!     owed here, and abandon reserved cargo (see [`crate::mission`]).
//!   - **Shipyard** / **Drydock** — mend the hull, and (at shipyard ports) buy
//!     sail and hold upgrades.
//!
//! The original's Racing tab waits on the Race port.
//!
//! Fully keyboard-driven: the cursor sits on the tab bar or on a row. Left/Right
//! switch tabs (or, on a commodity row, choose Buy/Fill/Dump/Sell); Up/Down move
//! through rows; Enter/Space commits; Esc (or "Set Sail") hands the helm back.

use std::cell::RefCell;

use macroquad::prelude::*;

use crate::game_state::{hull, upgrades, GameState, Good, Location, Market, TradeError, UpgradeKind};
use crate::geometry::{wrap_angle, Vec2};
use crate::minimap::{self, MinimapPalette};
use crate::mission;
use crate::race;
use crate::sailing::{Kinematics, Wind};
use crate::sound::SoundBank;
use crate::tavern;
use crate::touch::TouchState;
use crate::world::{Island, World};

// How far off the bow a port may sit and still raise the docking prompt: a
// forward arc of ±60°, so a port ahead offers to dock but one abeam or astern
// does not (`SailingView.dockFacingArc`).
const DOCK_FACING_ARC: f32 = std::f32::consts::PI / 3.0;

/// A port within dock range of `pos`, if any. The bow-facing check that decides
/// whether it can actually be entered lives in [`Harbor::update_dockable`].
pub fn port_at(world: &World, pos: Vec2) -> Option<&Island> {
    world
        .islands
        .iter()
        .filter(|i| i.is_port)
        .find(|i| i.pos.distance_to(pos) <= i.dock_range())
}

/// Manages the docking handshake and owns the open overlay, if any.
pub struct Harbor {
    /// The port in range and ahead this frame, eligible to dock (id).
    pub dockable: Option<i32>,
    /// The open trading board, while docked.
    pub screen: Option<PortScreen>,
}

impl Harbor {
    pub fn new() -> Harbor {
        Harbor {
            dockable: None,
            screen: None,
        }
    }

    pub fn is_open(&self) -> bool {
        self.screen.is_some()
    }

    /// Recompute the dockable port for this frame. Called while sailing. A port is
    /// dockable whenever it is in range and off the bow — so the prompt appears the
    /// moment the captain faces a port in range, and disappears the moment they turn
    /// away or sail clear. (Leaving the board hands the helm straight back, so a
    /// captain who casts off and at once faces the port can tie up again right away.)
    pub fn update_dockable(&mut self, world: &World, kin: &Kinematics) {
        self.dockable = port_at(world, kin.pos)
            .filter(|port| {
                wrap_angle(kin.pos.bearing_to(port.pos) - kin.heading_rad).abs() <= DOCK_FACING_ARC
            })
            .map(|p| p.id);
    }

    /// Tie up at the dockable port (Space, sails struck). Returns true if docked.
    pub fn try_dock(&mut self, gs: &mut GameState) -> bool {
        if let Some(id) = self.dockable {
            gs.location = Location::Docked(id);
            gs.stats.times_docked += 1;
            self.dockable = None;
            self.screen = Some(PortScreen::new(id));
            true
        } else {
            false
        }
    }

    /// Re-open the trading board at `id` for a voyage restored from a save that was
    /// docked when stored. The world is rebuilt fresh from the seed, so the
    /// [`PortScreen`] must be recreated; `gs.location` is already `Docked(id)`.
    pub fn reopen_docked(&mut self, id: i32) {
        self.screen = Some(PortScreen::new(id));
    }

    /// Cast off: close the board and hand the helm back.
    pub fn set_sail(&mut self, gs: &mut GameState) {
        gs.location = Location::Sailing;
        self.screen = None;
    }
}

// --- The overlay --------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Market,
    Contracts,
    Yard,
    Race,
    /// The shipyard tavern's one special ware (only present at shipyard ports).
    Tavern,
}

/// What the keyboard cursor rests on. Contracts and deliveries are keyed by
/// mission id (not list index) so the cursor survives the list reshuffling when
/// one is accepted, delivered, or abandoned.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    TabBar,
    Good(usize),
    Repair,
    Upgrade(UpgradeKind),
    Contract(i32),
    Delivery(i32),
    Manifest(i32),
    RaceTarget(i32),
    RaceWithdraw,
    /// The tavern's special ware (one per shipyard; resolved via [`tavern::item_at`]).
    TavernItem,
}

/// A constraint that an invalid action just bumped against, so the board can
/// flash it red and jiggle it once. Targets are keyed by the thing on screen
/// (good index / mission id / rival-port id) so the right cell lights up.
#[derive(Clone, Copy, PartialEq, Eq)]
enum FlashTarget {
    Gold,          // the purse in the header (out of coin)
    Hold,          // the hold tally in the header (out of room)
    Held(usize),   // a good's held-quantity cell (nothing to sell)
    Deposit(i32),  // a contract's deposit (can't cover the buy-in)
    Units(i32),    // a contract's haulage units (won't fit the hold)
    Stake(i32),    // a rival port's race stake (can't cover the wager)
    Tier(i32),     // a rival port's hull requirement (hull not sturdy enough)
    UpgradeCost(UpgradeKind), // a fitting's price (can't cover the refit)
    RepairCost,    // the drydock repair price (can't cover any mending)
    ItemCost,      // the tavern ware's price (can't cover the buy)
}

/// A live red-jiggle on one constraint, started at `start` (seconds, `get_time`).
struct Flash {
    target: FlashTarget,
    start: f64,
}

/// A tappable region recorded by `render` as it draws, hit-tested on the next
/// frame's `handle_input` (immediate-mode retained hitboxes). Geometry lives where
/// it's drawn — there is no second copy of the layout to keep in sync.
#[derive(Clone, Copy)]
struct HitRect {
    rect: Rect,
    effect: HitEffect,
}

#[derive(Clone, Copy)]
enum HitEffect {
    /// Switch to this tab (cursor to the bar), like tapping a tab label.
    Tab(Tab),
    /// Focus this row — setting the action column too, for a market chip — and,
    /// when `activate`, run it, so one tap on a chip / job row commits.
    Select {
        focus: Focus,
        column: Option<usize>,
        activate: bool,
    },
}

pub struct PortScreen {
    island_id: i32,
    tab: Tab,
    focus: Focus,
    column: usize, // commodity action column: 0 Buy · 1 Fill · 2 Dump · 3 Sell
    /// Constraints currently flashing from a rejected action (see [`FlashTarget`]).
    flashes: Vec<Flash>,
    /// Tappable regions from the last `render`, consumed by touch in `handle_input`.
    hits: RefCell<Vec<HitRect>>,
}

// Flash timing: a brief red jiggle that decays to nothing.
const FLASH_DUR: f32 = 0.42; // seconds the jiggle lasts
const FLASH_AMP: f32 = 4.0; // peak horizontal wobble, px
const FLASH_FREQ: f32 = 7.0; // wobble oscillations per second

/// The tabs every port shows. A shipyard adds [`Tab::Tavern`] after these (see
/// [`PortScreen::available_tabs`]).
const TABS: [Tab; 4] = [Tab::Market, Tab::Yard, Tab::Contracts, Tab::Race];
const LAST_COLUMN: usize = 3;

/// The port overlay's whole visual style in one place — every type size, spacing
/// step, column split and symbol the board draws is named here, so the render code
/// below carries no bare pixel literals. Sizes are deliberately compact.
mod style {
    // --- Type scale & line height — the one shared UI ladder ----------------
    // Lives in `crate::ui` so the captain's log draws on the very same scale.
    // Every pixel token below is multiplied by `scale()` for the screen, so the
    // board scales as one from a phone up to a 4K display (see `crate::ui::scale`).
    pub use crate::ui::{fs_body, fs_chip, fs_heading, fs_small, fs_title, line_h, px};

    // --- Spacing — every gap is a multiple of one base unit (design px) ------
    pub fn unit() -> f32 {
        px(6.0)
    }
    pub fn pad() -> f32 {
        unit() * 4.0 // panel inner margin (24)
    }
    pub fn gap() -> f32 {
        unit() * 2.0 // gap between groups (12)
    }
    pub fn rule_gap() -> f32 {
        unit() // heading baseline → its underline rule (6)
    }
    pub fn col_gap() -> f32 {
        unit() * 4.0 // chart column → board column (24)
    }

    // --- The panel itself ---------------------------------------------------
    pub const SCRIM: f32 = 0.5; // alpha of the dim behind the board (ratio)
    pub const PANEL_W_FRAC: f32 = 0.92; // panel size as a fraction of the screen…
    pub const PANEL_H_FRAC: f32 = 0.9;
    pub fn panel_max_w() -> f32 {
        px(940.0) // …capped here (scaled, so a big screen gets a big board)
    }
    pub fn panel_max_h() -> f32 {
        px(560.0)
    }
    pub fn panel_border() -> f32 {
        unit() * 0.5 // panel edge stroke (3)
    }
    /// Tab-bar button height.
    pub fn tab_h() -> f32 {
        line_h(fs_body()) + unit()
    }

    // --- Rules & chips ------------------------------------------------------
    pub fn rule_w() -> f32 {
        px(1.0) // hairline divider thickness
    }
    pub fn border_w() -> f32 {
        px(1.5) // chip / outline stroke thickness
    }
    pub fn tab_underline() -> f32 {
        px(3.5) // the thick rule under the entered (active) tab
    }
    /// Baseline drop from a line's vertical centre, as a fraction of font size —
    /// used to vertically centre text in a chip (a ratio, not scaled).
    pub const CAP_RATIO: f32 = 0.35;
    pub fn chip_h() -> f32 {
        unit() * 4.0 // action chip / button height (24)
    }
    pub fn chip_w() -> f32 {
        unit() * 17.0 // a fitting's fixed chip width (102)
    }
    pub fn chip_gap() -> f32 {
        unit() // gap a cost keeps left of its chip (6)
    }
    pub fn chip_inner() -> f32 {
        unit() * 0.5 // gap between packed action chips (3)
    }
    pub fn row_pad_x() -> f32 {
        unit() // a focus highlight's horizontal overhang
    }
    /// Height of a list row (a chip plus breathing room).
    pub fn row_h() -> f32 {
        chip_h() + unit()
    }
    pub fn tab_pad_x() -> f32 {
        unit() * 2.0 // padding either side of a tab label
    }
    pub fn tab_gap() -> f32 {
        unit() * 2.0 // gap between tabs
    }
    pub fn btn_wide() -> f32 {
        unit() * 52.0 // a full-width action button's cap (312)
    }

    // --- Table column splits (fractions of the board width) -----------------
    // Market: name | price | held | four action chips.
    pub const MKT_PRICE_R: f32 = 0.42;
    pub const MKT_HELD_R: f32 = 0.56;
    pub const MKT_ACTIONS_X: f32 = 0.60;
    // Contracts: cargo | to | deposit | reward | action.
    pub const CON_TO_X: f32 = 0.27;
    pub const CON_DEP_R: f32 = 0.585;
    pub const CON_REW_R: f32 = 0.73;
    pub const CON_ACT_X: f32 = 0.77;
    // Racing: name | hull tier | stake | action.
    pub const RACE_TIER_R: f32 = 0.56;
    pub const RACE_STAKE_R: f32 = 0.74;
    pub const RACE_ACT_X: f32 = 0.78;
    // Shipyard fittings: where a data value's column begins.
    pub const YARD_VAL_X: f32 = 0.34;
    /// The chart fills this fraction of the board width down the left side.
    pub const CHART_FRAC: f32 = 0.34;

    // --- Symbols — one consistent set --------------------------------------
    // Inline separators are the middle dot "·" (baked into the header/title
    // literals); a transition or route uses the arrow below (never "->").
    pub const ARROW: &str = " → ";
}

impl PortScreen {
    fn new(island_id: i32) -> PortScreen {
        PortScreen {
            island_id,
            tab: Tab::Market,
            focus: Focus::TabBar,
            column: 0,
            flashes: Vec::new(),
            hits: RefCell::new(Vec::new()),
        }
    }

    /// Record a tappable region for this frame (see [`HitRect`]).
    fn record_hit(&self, rect: Rect, effect: HitEffect) {
        self.hits.borrow_mut().push(HitRect { rect, effect });
    }

    /// Start (or restart) a red jiggle on `target`. Drops any expired flashes and
    /// re-arms a still-running one so a repeated rejected press jiggles afresh.
    fn flash(&mut self, target: FlashTarget) {
        let now = get_time();
        self.flashes
            .retain(|f| f.target != target && ((now - f.start) as f32) < FLASH_DUR);
        self.flashes.push(Flash { target, start: now });
    }

    /// The current jiggle on `target`, if any: a `(dx, redness)` pair where `dx`
    /// is the horizontal wobble (px) and `redness` (1→0) blends the ink to red.
    fn flash_of(&self, target: FlashTarget) -> Option<(f32, f32)> {
        let now = get_time();
        self.flashes.iter().find(|f| f.target == target).and_then(|f| {
            let age = (now - f.start) as f32;
            if age >= FLASH_DUR {
                return None;
            }
            let decay = 1.0 - age / FLASH_DUR;
            let dx = FLASH_AMP * (age * FLASH_FREQ * std::f32::consts::TAU).sin() * decay;
            Some((dx, decay))
        })
    }

    fn is_shipyard(&self, world: &World) -> bool {
        world.islands[self.island_id as usize].is_shipyard
    }

    /// The tabs this port shows, in order: the common four, plus the tavern at a
    /// shipyard port (the only port with a tavern to call at).
    fn available_tabs(&self, world: &World) -> Vec<Tab> {
        let mut v = TABS.to_vec();
        if self.is_shipyard(world) {
            v.push(Tab::Tavern);
        }
        v
    }

    /// The navigable rows of the active tab, top to bottom (the tab bar is its
    /// own focus, above these). Derived from the live state so it always matches
    /// what's on screen as contracts come and go.
    fn rows_of(&self, gs: &GameState, world: &World, tab: Tab) -> Vec<Focus> {
        match tab {
            Tab::Market => (0..Good::ALL.len()).map(Focus::Good).collect(),
            Tab::Contracts => {
                let mut v = Vec::new();
                // Follow the on-screen order so the cursor walks the board top to
                // bottom: deliveries owed here are drawn first, then the offered
                // contracts, then the reserved manifest.
                v.extend(mission::deliverable_at(gs, world).iter().map(|m| Focus::Delivery(m.id)));
                // A hull too battered to be hired can't take on *new* contracts, so
                // those rows aren't focusable; deliveries owed and abandoning
                // reserved cargo stay open.
                if hull::can_take_jobs(gs) {
                    v.extend(mission::offered_at(gs, world).iter().map(|m| Focus::Contract(m.id)));
                }
                v.extend(mission::reserved_at(gs, world).iter().map(|m| Focus::Manifest(m.id)));
                v
            }
            Tab::Yard => {
                let mut v = vec![Focus::Repair];
                if self.is_shipyard(world) {
                    v.push(Focus::Upgrade(UpgradeKind::Hull));
                    v.push(Focus::Upgrade(UpgradeKind::Sail));
                    v.push(Focus::Upgrade(UpgradeKind::Cargo));
                }
                v
            }
            // While a race is booked the tab is just the armed race + an abandon;
            // with none booked it is the day's rival ports, each accepted by pressing
            // Enter on it — the same flow as accepting a contract.
            Tab::Race => {
                if gs.race.is_some() {
                    vec![Focus::RaceWithdraw]
                } else if !hull::can_take_jobs(gs) {
                    // Too battered to be staked in a race — no rival rows to pick.
                    Vec::new()
                } else {
                    race::offers(gs, world)
                        .iter()
                        .map(|p| Focus::RaceTarget(p.id))
                        .collect()
                }
            }
            // One ware per tavern; the row is focusable whether or not it's owned (so
            // the captain can read it either way), and is absent only at a port with
            // no tavern at all.
            Tab::Tavern => {
                if tavern::item_at(world, self.island_id).is_some() {
                    vec![Focus::TavernItem]
                } else {
                    Vec::new()
                }
            }
        }
    }

    fn enter_rows(&mut self, gs: &GameState, world: &World) {
        if let Some(&first) = self.rows_of(gs, world, self.tab).first() {
            self.focus = first;
        }
    }

    /// Up/Down within the active tab; from the tab bar, Down enters the rows;
    /// from the topmost row, Up returns to the tab bar.
    fn move_cursor(&mut self, gs: &GameState, world: &World, delta: i32) {
        match self.focus {
            Focus::TabBar => {
                if delta > 0 {
                    self.enter_rows(gs, world);
                }
            }
            here => {
                let list = self.rows_of(gs, world, self.tab);
                match list.iter().position(|f| *f == here) {
                    None => self.enter_rows(gs, world),
                    Some(0) if delta < 0 => self.focus = Focus::TabBar,
                    Some(i) => {
                        let j = (i as i32 + delta).clamp(0, list.len() as i32 - 1) as usize;
                        self.focus = list[j];
                    }
                }
            }
        }
    }

    fn cycle_tab(&mut self, world: &World, delta: i32) {
        let tabs = self.available_tabs(world);
        let i = tabs.iter().position(|t| *t == self.tab).unwrap_or(0);
        let n = tabs.len() as i32;
        self.tab = tabs[((i as i32 + delta).rem_euclid(n)) as usize];
        self.focus = Focus::TabBar;
    }

    /// Switch to the adjacent tab but stay down in its rows (Left/Right paging
    /// once the cursor runs off the end of a row).
    fn slide_tab(&mut self, gs: &GameState, world: &World, delta: i32) {
        let tabs = self.available_tabs(world);
        let i = tabs.iter().position(|t| *t == self.tab).unwrap_or(0);
        let n = tabs.len() as i32;
        self.tab = tabs[((i as i32 + delta).rem_euclid(n)) as usize];
        self.focus = self
            .rows_of(gs, world, self.tab)
            .first()
            .copied()
            .unwrap_or(Focus::TabBar);
    }

    fn activate(
        &mut self,
        gs: &mut GameState,
        world: &World,
        market: &Market,
        sounds: &SoundBank,
    ) {
        match self.focus {
            Focus::TabBar => self.enter_rows(gs, world),
            Focus::Good(i) => {
                let good = Good::ALL[i];
                // Whether the hold was already full going in tells a failed Fill
                // (which only reports `NonPositive`) apart from being out of coin.
                let hold_full = gs.hold_free() <= 0;
                let buying = self.column <= 1; // Buy / Fill
                // Fill/Dump move the hold in bulk (coin pour); Buy/Sell are
                // per-unit (a single coin).
                let bulk = self.column == 1 || self.column == 2;
                let done = match self.column {
                    0 => gs.buy(market, good, 1),
                    1 => gs.fill(market, good),
                    2 => gs.dump(market, good),
                    _ => gs.sell(market, good, 1),
                };
                match done {
                    Ok(()) => {
                        if bulk {
                            sounds.trade_bulk();
                        } else {
                            sounds.trade_one();
                        }
                    }
                    Err(e) => {
                        sounds.invalid();
                        if buying {
                            // Out of coin lights the purse; out of room lights the
                            // hold tally. A Fill that bought nothing is one or the
                            // other depending on which ran out.
                            match e {
                                TradeError::NotEnoughHold => self.flash(FlashTarget::Hold),
                                TradeError::NotEnoughGold => self.flash(FlashTarget::Gold),
                                TradeError::NonPositive if hold_full => {
                                    self.flash(FlashTarget::Hold)
                                }
                                TradeError::NonPositive => self.flash(FlashTarget::Gold),
                                _ => {}
                            }
                        } else {
                            // Nothing to sell/dump — light the held-quantity cell.
                            self.flash(FlashTarget::Held(i));
                        }
                    }
                }
            }
            Focus::Repair => match gs.repair() {
                Ok(()) => sounds.trade_bulk(),
                Err(e) => {
                    sounds.invalid();
                    if e == TradeError::NotEnoughGold {
                        // Price and purse are the two ends of the same shortfall: jiggle both.
                        self.flash(FlashTarget::RepairCost);
                        self.flash(FlashTarget::Gold);
                    }
                }
            },
            Focus::Upgrade(kind) => match gs.buy_upgrade(world, kind) {
                Ok(()) => sounds.trade_bulk(),
                Err(e) => {
                    sounds.invalid();
                    if e == TradeError::NotEnoughGold {
                        // Price and purse are the two ends of the same shortfall: jiggle both.
                        self.flash(FlashTarget::UpgradeCost(kind));
                        self.flash(FlashTarget::Gold);
                    }
                }
            },
            // Contract actions reshuffle the rows. Keep the cursor at the same
            // vertical slot — the acted-on row has vanished, so we land on
            // whatever took its place (or the tab bar if the tab emptied out).
            here @ (Focus::Contract(_) | Focus::Delivery(_) | Focus::Manifest(_)) => {
                let before = self.rows_of(gs, world, self.tab);
                let slot = before.iter().position(|f| *f == here).unwrap_or(0);
                let done = match here {
                    Focus::Contract(id) => gs.accept(world, id),
                    Focus::Delivery(id) => gs.deliver(world, id),
                    Focus::Manifest(id) => gs.abandon(world, id),
                    _ => unreachable!(),
                };
                match done {
                    Ok(()) => {
                        // Accepting/abandoning a contract stamps; a delivery pays
                        // out a purse, so it rings the coin pour.
                        match here {
                            Focus::Delivery(_) => sounds.trade_bulk(),
                            _ => sounds.accept(),
                        }
                        let after = self.rows_of(gs, world, self.tab);
                        self.focus = if after.is_empty() {
                            Focus::TabBar
                        } else {
                            after[slot.min(after.len() - 1)]
                        };
                    }
                    Err(e) => {
                        sounds.invalid();
                        // Only accepting a contract has a buy-in to fall short on:
                        // no coin lights its deposit; no room lights both the hold
                        // tally and the contract's haulage units.
                        if let Focus::Contract(id) = here {
                            match e {
                                TradeError::NotEnoughGold => self.flash(FlashTarget::Deposit(id)),
                                TradeError::NotEnoughHold => {
                                    self.flash(FlashTarget::Hold);
                                    self.flash(FlashTarget::Units(id));
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
            // Enter on a rival port books the race outright (charging the stake) —
            // no separate confirm step, mirroring how a contract is accepted. The
            // picker rows then vanish, so land on the new abandon row.
            Focus::RaceTarget(id) => match gs.accept_race(world, id) {
                Ok(()) => {
                    sounds.accept();
                    self.focus = Focus::RaceWithdraw;
                }
                Err(e) => {
                    sounds.invalid();
                    match e {
                        TradeError::NotEnoughGold => self.flash(FlashTarget::Stake(id)),
                        TradeError::HullTierTooLow => self.flash(FlashTarget::Tier(id)),
                        _ => {}
                    }
                }
            },
            Focus::RaceWithdraw => match gs.withdraw_race(world) {
                Ok(()) => {
                    sounds.accept();
                    self.focus = self
                        .rows_of(gs, world, self.tab)
                        .first()
                        .copied()
                        .unwrap_or(Focus::TabBar);
                }
                Err(_) => sounds.invalid(),
            },
            // Buy the tavern's one ware (resolved from the port). A short purse jiggles
            // the price and the header gold; an already-owned ware just no-ops.
            Focus::TavernItem => {
                let Some(item) = tavern::item_at(world, self.island_id) else {
                    return;
                };
                // Already in the kit: nothing to buy, so don't buzz at the captain.
                if gs.owns(item) {
                    return;
                }
                match gs.buy_item(world, item) {
                    Ok(()) => sounds.trade_bulk(),
                    Err(e) => {
                        sounds.invalid();
                        if e == TradeError::NotEnoughGold {
                            self.flash(FlashTarget::ItemCost);
                            self.flash(FlashTarget::Gold);
                        }
                    }
                }
            }
        }
    }

    /// Read input and drive the board. Returns true when the captain sets sail.
    ///
    /// Three ways in, all converging on the same `cycle_tab`/`move_cursor`/
    /// `activate`: the keyboard; the on-screen nav cluster (an emulated d-pad +
    /// ✓/✕ for captains who'd rather not tap precisely); and direct taps on the
    /// tabs/rows/chips themselves (the regions `render` recorded last frame).
    pub fn handle_input(
        &mut self,
        gs: &mut GameState,
        world: &World,
        market: &Market,
        sounds: &SoundBank,
        touch: &TouchState,
    ) -> bool {
        // The board always has a focused, actionable cell (a tab to drill into, or a
        // row to trade / commit), so the ✓ stands.
        let n = crate::touch_ui::nav_cluster(screen_width(), screen_height(), true);

        // Back out: Esc, or the cluster's ✕. With the cursor down in a tab's rows
        // it first lifts back up to the tab bar; pressed again on the bar it casts
        // off and closes the board.
        if is_key_pressed(KeyCode::Escape) || touch.tapped_in(n.back) {
            if self.focus == Focus::TabBar {
                return true;
            }
            self.focus = Focus::TabBar;
            return false;
        }
        // Tab cycles the board (keyboard only — touch taps the tab labels direct).
        if is_key_pressed(KeyCode::Tab) {
            let back = is_key_down(KeyCode::LeftShift) || is_key_down(KeyCode::RightShift);
            self.cycle_tab(world, if back { -1 } else { 1 });
        }

        // Directional verbs: keyboard arrows or the nav cluster's d-pad / ✓.
        let up = is_key_pressed(KeyCode::Up) || touch.tapped_in(n.up);
        let down = is_key_pressed(KeyCode::Down) || touch.tapped_in(n.down);
        let left = is_key_pressed(KeyCode::Left) || touch.tapped_in(n.left);
        let right = is_key_pressed(KeyCode::Right) || touch.tapped_in(n.right);
        let confirm =
            is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::Space) || touch.tapped_in(n.confirm);

        if up {
            self.move_cursor(gs, world, -1);
        }
        if down {
            self.move_cursor(gs, world, 1);
        }
        if left {
            match self.focus {
                Focus::TabBar => self.cycle_tab(world, -1),
                Focus::Good(_) if self.column > 0 => self.column -= 1,
                _ => self.slide_tab(gs, world, -1),
            }
        }
        if right {
            match self.focus {
                Focus::TabBar => self.cycle_tab(world, 1),
                Focus::Good(_) if self.column < LAST_COLUMN => self.column += 1,
                _ => self.slide_tab(gs, world, 1),
            }
        }
        if confirm {
            self.activate(gs, world, market, sounds);
        }

        // Direct tap-to-activate on the board — tabs, rows, chips — *unless* this
        // tap already worked a nav-cluster button (so a cluster press sitting over a
        // row doesn't also fire the row beneath it).
        let cluster_used = touch.tapped_in(n.up)
            || touch.tapped_in(n.down)
            || touch.tapped_in(n.left)
            || touch.tapped_in(n.right)
            || touch.tapped_in(n.confirm);
        if !cluster_used {
            // Resolve to the *smallest* region the tap falls in, so an action chip
            // nested inside its select-only row wins over the row.
            let hit = self
                .hits
                .borrow()
                .iter()
                .filter(|hr| touch.tapped_in(hr.rect))
                .min_by(|a, b| (a.rect.w * a.rect.h).total_cmp(&(b.rect.w * b.rect.h)))
                .map(|hr| hr.effect);
            if let Some(effect) = hit {
                match effect {
                    HitEffect::Tab(tab) => {
                        self.tab = tab;
                        self.focus = Focus::TabBar;
                    }
                    HitEffect::Select { focus, column, activate } => {
                        self.focus = focus;
                        if let Some(c) = column {
                            self.column = c;
                        }
                        if activate {
                            self.activate(gs, world, market, sounds);
                        }
                    }
                }
            }
        }
        false
    }

    #[allow(clippy::too_many_arguments)] // the board needs the full world/frame context
    pub fn render(
        &self,
        gs: &GameState,
        world: &World,
        market: &Market,
        kin: &Kinematics,
        wind: Wind,
        w: f32,
        h: f32,
    ) {
        use style::*;
        let port = &world.islands[self.island_id as usize];

        // Fresh hit regions for this layout; the render below repopulates them as it
        // draws, and `handle_input` taps them next frame.
        self.hits.borrow_mut().clear();

        // Dim the world so the board reads as the captain's focus.
        draw_rectangle(0.0, 0.0, w, h, Color::new(0.0, 0.0, 0.0, SCRIM));

        let pw = (w * PANEL_W_FRAC).min(panel_max_w());
        let ph = (h * PANEL_H_FRAC).min(panel_max_h());
        let x0 = (w - pw) / 2.0;
        let y0 = (h - ph) / 2.0;
        draw_rectangle(x0, y0, pw, ph, parchment());
        draw_rectangle_lines(x0, y0, pw, ph, panel_border(), parchment_edge());

        let left = x0 + pad();
        let right = x0 + pw - pad();
        let inner_w = pw - 2.0 * pad();

        // --- Header: eyebrow, port name, purse ------------------------------
        let eyebrow = if port.is_shipyard {
            "SHIPYARD · PORT OF CALL"
        } else {
            "PORT OF CALL"
        };
        let eyebrow_y = y0 + pad() + fs_small() as f32;
        let name_y = eyebrow_y + line_h(fs_title());
        crate::font::heading(|| {
            draw_text(eyebrow, left, eyebrow_y, fs_small() as f32, dim_ink());
            draw_text(&port.name, left, name_y, fs_title() as f32, ink());
        });

        // Purse + hold tally, right-aligned in the header. The gold lights red and
        // jiggles when an action runs out of coin; the hold tally does the same when
        // it runs out of room. The hold line also carries the sail's haul tolerance.
        let used = gs.hold_used();
        let cap = gs.hold_capacity;
        let haul = upgrades::max_haul(gs.sail_level);
        let gold = format!("Gold {}", gs.gold);
        let hold = format!("Hold {used}/{cap} · Sail tolerance {haul}");
        let gold_y = y0 + pad() + fs_heading() as f32;
        let hold_y = gold_y + line_h(fs_small());
        right_text_flash(&gold, right, gold_y, fs_heading(), self.flash_of(FlashTarget::Gold));
        right_text_flash(&hold, right, hold_y, fs_small(), self.flash_of(FlashTarget::Hold));

        // A laden-fill bar beneath the tally, mirroring the captain's-log hold bar:
        // the hold's laden fraction, with a notch at the sail's haul tolerance. It
        // turns red once the load passes that tolerance — i.e. once she's overladen
        // and taking the overload penalty.
        let bar_w = px(150.0);
        let bar_h = px(7.0);
        let bar_x = right - bar_w;
        let fill_top = hold_y + line_h(fs_small()) * 0.4;
        let overladen = used > haul;
        draw_rectangle(bar_x, fill_top, bar_w, bar_h, Color::new(0.0, 0.0, 0.0, 0.10));
        let frac = if cap > 0 { (used as f32 / cap as f32).clamp(0.0, 1.0) } else { 0.0 };
        let fill_col = if overladen { alarm_ink() } else { parchment_edge() };
        draw_rectangle(bar_x, fill_top, bar_w * frac, bar_h, fill_col);
        draw_rectangle_lines(bar_x, fill_top, bar_w, bar_h, px(1.0), dim_ink());
        if cap > 0 && haul < cap {
            let nx = bar_x + bar_w * (haul as f32 / cap as f32).clamp(0.0, 1.0);
            draw_line(nx, fill_top - px(2.0), nx, fill_top + bar_h + px(2.0), px(1.5), alarm_ink());
        }

        let bar_y = name_y + gap();
        rule(left, bar_y, inner_w);

        // --- Tab bar --------------------------------------------------------
        let yard_label = if port.is_shipyard { "Shipyard" } else { "Drydock" };
        let on_bar = self.focus == Focus::TabBar;
        let tab_y = bar_y + gap();
        let mut tx = left;
        tx = self.tab_button("Market", Tab::Market, tx, tab_y, on_bar);
        tx = self.tab_button(yard_label, Tab::Yard, tx, tab_y, on_bar);
        tx = self.tab_button("Contracts", Tab::Contracts, tx, tab_y, on_bar);
        tx = self.tab_button("Racing", Tab::Race, tx, tab_y, on_bar);
        // Only a shipyard has a tavern to call at.
        if port.is_shipyard {
            let _ = self.tab_button("Tavern", Tab::Tavern, tx, tab_y, on_bar);
        }

        // --- Body: chart on the left, the active board on the right ----------
        let body_top = tab_y + tab_h() + gap();
        let chart_size = (ph - (body_top - y0) - pad()).min(pw * CHART_FRAC);
        let chart = Rect::new(left, body_top, chart_size, chart_size);
        let cpal = MinimapPalette::parchment();
        // Mark every accepted contract's destination ("M") and the booked race's
        // mark ("R"), and draw a dashed route from this port out to the highlighted
        // contract's or race's other port (so a rival port eyed on the Racing tab
        // previews its leg before it is booked, just as a contract does).
        let marks: Vec<i32> = gs.active_missions.iter().map(|m| m.target_id).collect();
        let race_marks: Vec<i32> = gs.race.iter().map(|r| r.target_id).collect();
        let route = self.route_line(gs, world);
        minimap::render(world, kin, wind, chart, &cpal, &marks, &race_marks, route, &[], None);
        // Name the local waters under the chart.
        let waters = &world.cluster_at(kin.pos).name;
        let cd = measure_text(waters, None, fs_small(), 1.0);
        draw_text(
            waters,
            chart.x + (chart_size - cd.width) / 2.0,
            chart.y + chart_size + line_h(fs_small()),
            fs_small() as f32,
            ink(),
        );

        let board_x = chart.x + chart_size + col_gap();
        let board_w = right - board_x;
        // The chart on the left is a rect whose *top edge* sits at `body_top`, but the
        // boards lead with text drawn on a *baseline* — so starting them at `body_top`
        // floats their first line a full ascent above it, riding up against the tab bar.
        // Drop the board down by one heading ascent so its first line's cap-height lines
        // up with the chart's top edge.
        let board_top = body_top + fs_heading() as f32;
        match self.tab {
            Tab::Market => self.render_market(gs, market, board_x, board_top, board_w),
            Tab::Contracts => self.render_contracts(gs, world, board_x, board_top, board_w),
            Tab::Yard => self.render_yard(gs, world, board_x, board_top, board_w),
            Tab::Race => self.render_race(gs, world, board_x, board_top, board_w),
            Tab::Tavern => self.render_tavern(gs, world, board_x, board_top, board_w),
        }

        // Footer hint.
        draw_text(
            "Arrows move · Tab switches board · Enter trades · Esc backs out",
            left,
            y0 + ph - pad(),
            fs_small() as f32,
            dim_ink(),
        );
    }

    /// Draw a tab button; returns the x where the next one should start.
    fn tab_button(&self, label: &str, tab: Tab, x: f32, y: f32, on_bar: bool) -> f32 {
        use style::*;
        let dims = measure_text(label, None, fs_body(), 1.0);
        let bw = dims.width + 2.0 * tab_pad_x();
        let bh = tab_h();
        self.record_hit(Rect::new(x, y, bw, bh), HitEffect::Tab(tab));
        let active = self.tab == tab; // the board this tab shows is the one on screen
        let highlighted = active && on_bar; // the cursor is resting on this tab
        if highlighted {
            // The cursor highlight: filled, the same as a focused chip.
            draw_rectangle(x, y, bw, bh, chip_fill());
        } else {
            draw_rectangle_lines(x, y, bw, bh, border_w(), parchment_edge());
        }
        let c = if highlighted { parchment() } else { ink() };
        draw_text(label, x + tab_pad_x(), y + bh / 2.0 + fs_body() as f32 * CAP_RATIO, fs_body() as f32, c);
        // Once a tab is entered (the cursor has dropped into its rows) it sheds the
        // highlight and is marked active by a thick underline instead, so the cursor
        // down in the body is never confused with the tab it came from.
        if active && !on_bar {
            let uy = y + bh;
            draw_line(x, uy, x + bw, uy, tab_underline(), ink());
        }
        x + bw + tab_gap()
    }

    fn render_market(&self, gs: &GameState, market: &Market, x: f32, y: f32, w: f32) {
        use style::*;
        // Column anchors within the board.
        let price_r = x + w * MKT_PRICE_R; // right edge of the price column
        let held_r = x + w * MKT_HELD_R; // right edge of the hold column
        let actions_x = x + w * MKT_ACTIONS_X;

        draw_text("Commodity", x, y, fs_small() as f32, dim_ink());
        right_text("Price", price_r, y, fs_small());
        right_text("Hold", held_r, y, fs_small());
        draw_text("Trade", actions_x, y, fs_small() as f32, dim_ink());
        rule(x, y + rule_gap(), w);

        const ACTIONS: [&str; 4] = ["Buy", "Fill", "Dump", "Sell"];
        let step = row_h();
        let mut ry = y + step;
        for (i, good) in Good::ALL.iter().enumerate() {
            let active_row = self.focus == Focus::Good(i);
            if active_row {
                highlight_row(x, ry, w);
            }
            // Tapping the commodity (left of the chips) just highlights its row.
            self.record_hit(
                Rect::new(x - row_pad_x(), row_center(ry) - row_h() / 2.0, actions_x - x, row_h()),
                HitEffect::Select { focus: Focus::Good(i), column: None, activate: false },
            );
            draw_text(good.label(), x, ry, fs_body() as f32, ink());
            right_text(&market.price(*good).to_string(), price_r, ry, fs_body());
            // The held tally jiggles red on a Sell/Dump with nothing to sell.
            right_text_flash(
                &gs.quantity_of(*good).to_string(),
                held_r,
                ry,
                fs_body(),
                self.flash_of(FlashTarget::Held(i)),
            );

            // Four action chips.
            let chip_w = (x + w - actions_x) / 4.0;
            for (c, label) in ACTIONS.iter().enumerate() {
                let cx = actions_x + c as f32 * chip_w;
                let focused = active_row && self.column == c;
                let chip = Rect::new(cx + chip_inner(), chip_y(ry), chip_w - 2.0 * chip_inner(), chip_h());
                button(chip.x, chip.y, chip.w, chip.h, label, focused);
                // Tapping a chip selects this good + column and commits the trade.
                self.record_hit(
                    chip,
                    HitEffect::Select { focus: Focus::Good(i), column: Some(c), activate: true },
                );
            }
            ry += step;
        }
    }

    /// Two sections: **Drydock** (hull repair — a single one-line row at the top) and
    /// **Shipyard** (the three orthogonal fittings, each a compact block: a title, one
    /// or two data lines, and a cost/chip). Every size and gap comes from [`style`].
    fn render_yard(&self, gs: &GameState, world: &World, x: f32, y: f32, w: f32) {
        use style::*;
        let val_x = x + w * YARD_VAL_X; // inner column where stat values begin
        let chip_x = x + w - chip_w();
        let cost_r = chip_x - chip_gap(); // cost right-aligned just left of the chip
        let step = line_h(fs_body());

        // A cost right-aligned left of the action chip, both centred in a block of
        // height `bh` whose top is `ry`. The chip is the *only* commit hitbox (the row
        // body just selects, like every other board row); a rejected buy jiggles the
        // price red, so the constraint the captain bumped is the thing that flashes.
        let cost_chip = |ry: f32, bh: f32, cost: &str, label: &str, focus: Focus, focused: bool, activatable: bool| {
            let chip_top = ry + (bh - chip_h()) / 2.0;
            let chip_rect = Rect::new(chip_x, chip_top, chip_w(), chip_h());
            button(chip_rect.x, chip_rect.y, chip_rect.w, chip_rect.h, label, focused);
            let flash = match focus {
                Focus::Repair => self.flash_of(FlashTarget::RepairCost),
                Focus::Upgrade(k) => self.flash_of(FlashTarget::UpgradeCost(k)),
                _ => None,
            };
            right_text_flash(cost, cost_r, chip_top + chip_h() / 2.0 + fs_body() as f32 * CAP_RATIO, fs_body(), flash);
            if activatable {
                self.record_hit(chip_rect, HitEffect::Select { focus, column: None, activate: true });
            }
        };
        let highlight = |ry: f32, bh: f32| {
            draw_rectangle(x - row_pad_x(), ry, w + 2.0 * row_pad_x(), bh, row_highlight());
        };

        // ===== Drydock — hull repair: one line ==============================
        // Pull the first row up snug under the heading rule (the section advance
        // plus the row's own top pad otherwise stack ~two blank lines).
        let mut ry = section("Drydock · Hull Repair", x, y, w) - gap();
        {
            let active = self.focus == Focus::Repair;
            let bh = step + gap();
            if active {
                highlight(ry, bh);
            }
            self.record_hit(
                Rect::new(x - row_pad_x(), ry, w + 2.0 * row_pad_x(), bh),
                HitEffect::Select { focus: Focus::Repair, column: None, activate: false },
            );
            let base = ry + step;
            let cond = format!(
                "{} / {} ({}%)",
                gs.hull,
                gs.max_hull(),
                (hull::fraction(gs) * 100.0).round() as i32
            );
            draw_text("Hull status", x, base, fs_body() as f32, ink());
            draw_text(&cond, val_x, base, fs_body() as f32, ink());
            // Only a damaged hull shows a price and a Repair chip; a sound hull
            // (100%) has nothing to mend, so we drop both.
            if hull::damage(gs) > 0 {
                cost_chip(ry, bh, &hull::repair_cost(gs).to_string(), "Repair", Focus::Repair, active, true);
            }
            // Extra breathing room before the next section heading.
            ry += bh + gap() * 2.0;
        }

        // ===== Shipyard — hull / sails / hold fittings ======================
        ry = section("Shipyard · Outfitting", x, ry, w) - gap();
        if self.is_shipyard(world) {
            for kind in [UpgradeKind::Hull, UpgradeKind::Sail, UpgradeKind::Cargo] {
                let active = self.focus == Focus::Upgrade(kind);
                let lvl0 = upgrades::level_of(kind, gs);
                let maxed = upgrades::next_cost(kind, gs).is_none();

                // Levels shown 1-indexed (the starter ship is Lv 1); the hull reads as
                // a mark (Mk I..IV) to match the captain's parlance.
                let title = match kind {
                    UpgradeKind::Hull => format!("Hull · {}", hull_mark(lvl0)),
                    _ => format!("{} · Lv {}", kind.label(), lvl0 + 1),
                };

                // The fitting's data lines: the current→next gain, plus (for the hull)
                // a live readout of how the ship stands today. The sail and hold show
                // only their tolerance/slot count — live "in use" is on the purse line
                // and the hold-vs-tolerance bar in the header.
                let mut lines: Vec<(&str, String)> = Vec::new();
                match kind {
                    UpgradeKind::Hull => {
                        let s0 = upgrades::peak_knots(lvl0) as i32;
                        let h0 = hull::max_hull(lvl0);
                        if maxed {
                            lines.push(("Top speed", format!("{s0} kn")));
                            lines.push(("Hull points", h0.to_string()));
                        } else {
                            let s1 = upgrades::peak_knots(lvl0 + 1) as i32;
                            let h1 = hull::max_hull(lvl0 + 1);
                            lines.push(("Top speed", format!("{s0}{ARROW}{s1} kn")));
                            lines.push(("Hull points", format!("{h0}{ARROW}{h1}")));
                        }
                    }
                    UpgradeKind::Sail => {
                        let haul = upgrades::max_haul(gs.sail_level);
                        if maxed {
                            lines.push(("Cargo tolerance", format!("{haul} units")));
                        } else {
                            let h1 = upgrades::max_haul(gs.sail_level + 1);
                            lines.push(("Cargo tolerance", format!("{haul}{ARROW}{h1} units")));
                        }
                    }
                    UpgradeKind::Cargo => {
                        let cap = gs.hold_capacity;
                        if maxed {
                            lines.push(("Cargo slots", cap.to_string()));
                        } else {
                            let next = upgrades::cargo_capacity(lvl0 + 1);
                            lines.push(("Cargo slots", format!("{cap}{ARROW}{next}")));
                        }
                    }
                }

                // Title line + data lines, plus a little breathing room.
                let bh = (1 + lines.len()) as f32 * step + gap();
                if active {
                    highlight(ry, bh);
                }
                self.record_hit(
                    Rect::new(x - row_pad_x(), ry, w + 2.0 * row_pad_x(), bh),
                    HitEffect::Select { focus: Focus::Upgrade(kind), column: None, activate: false },
                );
                draw_text(&title, x, ry + step, fs_body() as f32, ink());
                for (i, (label, value)) in lines.iter().enumerate() {
                    stat(label, value, x, val_x, ry + step * (i as f32 + 2.0));
                }

                let (cost, label) = match upgrades::next_cost(kind, gs) {
                    None => ("MAX".to_string(), "Maxed"),
                    Some(c) => (c.to_string(), "Fit"),
                };
                cost_chip(ry, bh, &cost, label, Focus::Upgrade(kind), active, !maxed);
                // `bh` already carries a trailing `gap()`; don't double it between rows.
                ry += bh;
            }
        } else {
            // Every archipelago has exactly one shipyard (its first port), so there's
            // always one in these waters — name it, and how far it lies, rather than
            // leaving the captain to hunt blind.
            let here = &world.islands[self.island_id as usize];
            let cluster = world.cluster_at(here.pos);
            let yard = world.cluster_islands(cluster).into_iter().find(|i| i.is_shipyard);
            let sh = line_h(fs_small());
            let mut fy = ry + sh;
            draw_text(
                "No shipyard here. A shipyard outfits your vessel: a sturdier hull,",
                x,
                fy,
                fs_small() as f32,
                dim_ink(),
            );
            fy += sh;
            draw_text("more sail, and a larger hold.", x, fy, fs_small() as f32, dim_ink());
            if let Some(y) = yard {
                fy += sh;
                let to_yard = here.pos.bearing_to(y.pos);
                let msg = format!(
                    "The {}'s shipyard lies at {} ({} {}).",
                    cluster.name,
                    y.name,
                    format_dist(here.pos.distance_to(y.pos)),
                    crate::geometry::compass(to_yard),
                );
                draw_text(&msg, x, fy, fs_small() as f32, dim_ink());
            }
        }
    }

    /// The Racing board: wager on beating a computer-helmed rival to another port.
    /// With no race booked it shows the day's rival ports (each with its stake) and
    /// a challenge button; with one booked it shows the armed race and a withdraw.
    fn render_race(&self, gs: &GameState, world: &World, x: f32, y: f32, w: f32) {
        use style::*;
        let origin = &world.islands[self.island_id as usize];
        crate::font::heading(|| draw_text("Harbour Race · Wager", x, y, fs_heading() as f32, ink()));
        let mut ry = y + line_h(fs_heading());
        let step = line_h(fs_body());

        // Label (dim, left) + value (right-aligned) line within the board.
        let line = |label: &str, value: &str, ry: f32| {
            draw_text(label, x, ry, fs_body() as f32, dim_ink());
            right_text(value, x + w, ry, fs_body());
        };

        if let Some(race) = gs.race {
            let target = &world.islands[race.target_id as usize];
            crate::font::heading(|| draw_text("Race booked", x, ry, fs_heading() as f32, ink()));
            ry += line_h(fs_heading());
            line("Race to", &target.name, ry);
            ry += step;
            line("Rival hull", &hull_mark(race.required_level), ry);
            ry += step;
            line("Stake", &race.stake.to_string(), ry);
            ry += step;
            line("On winning", &(race.stake * 2).to_string(), ry);
            ry += step + gap();
            draw_text("Set sail and the rival draws up alongside.", x, ry, fs_small() as f32, dim_ink());
            ry += line_h(fs_small());
            draw_text("Heave to, then raise sail to start level.", x, ry, fs_small() as f32, dim_ink());
            ry += line_h(fs_small()) + gap();
            let focused = self.focus == Focus::RaceWithdraw;
            button(x, ry, w.min(btn_wide()), chip_h(), "Abandon race (stake refunded)", focused);
            self.record_hit(
                Rect::new(x, ry, w.min(btn_wide()), chip_h()),
                HitEffect::Select { focus: Focus::RaceWithdraw, column: None, activate: true },
            );
            return;
        }

        if !hull::can_take_jobs(gs) {
            // No harbour will stake a wreck — flag it where the rival card would be.
            draw_text(
                "Hull too battered — no harbour will stake you in a race.",
                x,
                ry,
                fs_body() as f32,
                flash_red(),
            );
            return;
        }

        let offers = race::offers(gs, world);
        if offers.is_empty() {
            draw_text("No rival ports in these waters to race to.", x, ry, fs_body() as f32, dim_ink());
            return;
        }

        draw_text("Beat a rival sloop to another port. The stake rises", x, ry, fs_small() as f32, dim_ink());
        ry += line_h(fs_small());
        draw_text("with the distance of the leg. Enter to take one on.", x, ry, fs_small() as f32, dim_ink());
        ry += line_h(fs_small()) + gap();

        // Columns: port name, the required hull tier, the stake right-aligned, then
        // an Accept chip — the same shape as a contract row.
        let tier_r = x + w * RACE_TIER_R;
        let stake_r = x + w * RACE_STAKE_R;
        let action_x = x + w * RACE_ACT_X;
        draw_text("Race to", x, ry, fs_small() as f32, dim_ink());
        right_text("Hull", tier_r, ry, fs_small());
        right_text("Stake", stake_r, ry, fs_small());
        rule(x, ry + rule_gap(), w);
        ry += line_h(fs_heading());

        // Each rival port is its own row: highlight it and Enter (or the Accept
        // chip) books the race — none is marked until then, the same as contracts.
        for p in &offers {
            let active = self.focus == Focus::RaceTarget(p.id);
            if active {
                highlight_row(x, ry, w);
            }
            // Row selects (previews the leg on the chart); the Accept chip below
            // books it — the same row-vs-chip split as the market and contracts.
            self.record_hit(
                row_rect(x, ry, w),
                HitEffect::Select { focus: Focus::RaceTarget(p.id), column: None, activate: false },
            );
            draw_text(&p.name, x, ry, fs_body() as f32, ink());
            let (stake, required) = race::offer_terms(origin, p);
            // The hull tier the leg demands, always shown (`Mk I` for an open race):
            // normal ink when the captain can meet it, alarm-red (and jiggling on a
            // rejected Accept) when his hull is too light for the leg.
            let tier_txt = hull_mark(required);
            if required > 0 && gs.hull_level < required {
                let dx = self
                    .flash_of(FlashTarget::Tier(p.id))
                    .map(|(dx, _)| dx)
                    .unwrap_or(0.0);
                let dims = measure_text(&tier_txt, None, fs_body(), 1.0);
                draw_text(&tier_txt, tier_r - dims.width + dx, ry, fs_body() as f32, flash_red());
            } else {
                right_text(&tier_txt, tier_r, ry, fs_body());
            }
            // The stake jiggles red when the purse can't cover the wager.
            right_text_flash(
                &stake.to_string(),
                stake_r,
                ry,
                fs_body(),
                self.flash_of(FlashTarget::Stake(p.id)),
            );
            let chip_rect = Rect::new(action_x, chip_y(ry), x + w - action_x, chip_h());
            button(chip_rect.x, chip_rect.y, chip_rect.w, chip_rect.h, "Accept", active);
            self.record_hit(
                chip_rect,
                HitEffect::Select { focus: Focus::RaceTarget(p.id), column: None, activate: true },
            );
            ry += row_h();
        }
    }

    /// The Tavern board: the shipyard tavern's one special ware. Shows its name,
    /// what it does, and a price + Buy chip; once owned, it reads as in the kit, and
    /// an active ware spells out its helm key and once-a-day recharge.
    fn render_tavern(&self, gs: &GameState, world: &World, x: f32, y: f32, w: f32) {
        use style::*;
        crate::font::heading(|| draw_text("The Tavern", x, y, fs_heading() as f32, ink()));
        let mut ry = y + line_h(fs_heading());

        let Some(item) = tavern::item_at(world, self.island_id) else {
            draw_text("This tavern has nothing for sale.", x, ry, fs_body() as f32, dim_ink());
            return;
        };

        // A line of flavour above the ware itself.
        draw_text(
            "The taverner keeps one curio behind the bar, sold but once.",
            x,
            ry,
            fs_small() as f32,
            dim_ink(),
        );
        ry += line_h(fs_small()) + gap();

        // Word-wrap a blurb into lines no wider than `w` at font size `fs`.
        let wrap = |text: &str, fs: u16| -> Vec<String> {
            let mut lines = Vec::new();
            let mut cur = String::new();
            for word in text.split_whitespace() {
                let trial = if cur.is_empty() { word.to_string() } else { format!("{cur} {word}") };
                if !cur.is_empty() && measure_text(&trial, None, fs, 1.0).width > w {
                    lines.push(std::mem::replace(&mut cur, word.to_string()));
                } else {
                    cur = trial;
                }
            }
            if !cur.is_empty() {
                lines.push(cur);
            }
            lines
        };

        let owned = gs.owns(item);
        let active = self.focus == Focus::TavernItem;
        let blurb = wrap(item.blurb(), fs_small());
        // A closing line for an owned active ware: while it's charged, its helm key; while
        // it's spent, the keybind is hidden (it would do nothing) and only the recharge is
        // noted. The daily charge comes back at sunrise.
        let status: Option<String> = if owned && item.is_active() {
            Some(if gs.item_ready(item) {
                let key = item.key_hint().unwrap_or("");
                format!("Press {key} at the helm to use. Recharges at sunrise.")
            } else {
                "Spent for the day. Recharges at sunrise.".to_string()
            })
        } else {
            None
        };

        // The ware sits in one block: a title row (name left, price+chip right), the
        // blurb, then the optional status line. Size the block so the focus highlight
        // and the touch hit-region cover the whole thing.
        let step = line_h(fs_body());
        let blurb_h = blurb.len() as f32 * line_h(fs_small());
        let status_h = if status.is_some() { line_h(fs_small()) } else { 0.0 };
        let bh = step + blurb_h + status_h + gap();
        if active {
            draw_rectangle(x - row_pad_x(), ry, w + 2.0 * row_pad_x(), bh, row_highlight());
        }
        self.record_hit(
            Rect::new(x - row_pad_x(), ry, w + 2.0 * row_pad_x(), bh),
            HitEffect::Select { focus: Focus::TavernItem, column: None, activate: false },
        );

        // Title row: ware name on the left.
        let title_base = ry + step;
        crate::font::heading(|| draw_text(item.name(), x, title_base, fs_heading() as f32, ink()));

        // Right side of the title row: a Buy chip with the price, or an "In kit" tag.
        let chip_x = x + w - chip_w();
        if owned {
            right_text("In your kit", x + w, title_base, fs_body());
        } else {
            let chip_top = ry + (step - chip_h()) / 2.0;
            let chip_rect = Rect::new(chip_x, chip_top, chip_w(), chip_h());
            button(chip_rect.x, chip_rect.y, chip_rect.w, chip_rect.h, "Buy", active);
            self.record_hit(
                chip_rect,
                HitEffect::Select { focus: Focus::TavernItem, column: None, activate: true },
            );
            // The price jiggles red (with the header gold) when the purse falls short.
            right_text_flash(
                &item.price().to_string(),
                chip_x - chip_gap(),
                chip_top + chip_h() / 2.0 + fs_body() as f32 * CAP_RATIO,
                fs_body(),
                self.flash_of(FlashTarget::ItemCost),
            );
        }

        // The blurb, then the status line.
        let mut by = title_base + line_h(fs_small());
        for line in &blurb {
            draw_text(line, x, by, fs_small() as f32, dim_ink());
            by += line_h(fs_small());
        }
        if let Some(s) = status {
            draw_text(&s, x, by, fs_small() as f32, ink());
        }
    }

    /// The dashed route the chart should draw: from this port to the other port
    /// of the highlighted contract (its destination), delivery (its origin), or
    /// manifest row (its destination). `None` with the cursor anywhere else.
    fn route_line(&self, gs: &GameState, world: &World) -> Option<(Vec2, Vec2)> {
        let here = world.islands[self.island_id as usize].pos;
        let other_id = match self.focus {
            Focus::Contract(id) => mission::offered_at(gs, world)
                .iter()
                .find(|m| m.id == id)
                .map(|m| m.target_id),
            Focus::Delivery(id) => gs
                .active_missions
                .iter()
                .find(|m| m.id == id)
                .map(|m| m.origin_id),
            Focus::Manifest(id) => gs
                .active_missions
                .iter()
                .find(|m| m.id == id)
                .map(|m| m.target_id),
            Focus::RaceTarget(id) => Some(id),
            Focus::RaceWithdraw => gs.race.map(|r| r.target_id),
            _ => None,
        }?;
        Some((here, world.islands[other_id as usize].pos))
    }

    /// The contracts board: haulage jobs out of this port, the deliveries owed
    /// here, and the reserved cargo riding in the hold bound elsewhere.
    fn render_contracts(&self, gs: &GameState, world: &World, x: f32, y: f32, w: f32) {
        use style::*;
        let step = row_h();
        let mut ry = y;

        // --- Deliveries owed at this very port (top: the most actionable line,
        // so a captain who just made port sees the payout waiting first) ------
        let deliveries = mission::deliverable_at(gs, world);
        if !deliveries.is_empty() {
            crate::font::heading(|| draw_text("Deliveries Awaiting", x, ry, fs_heading() as f32, ink()));
            ry += line_h(fs_heading());
            for m in &deliveries {
                let from = format!("from {}", world.islands[m.origin_id as usize].name);
                let focus = Focus::Delivery(m.id);
                self.contract_line(m, &from, true, "Deliver", focus, x, ry, w);
                ry += step;
            }
            ry += gap();
        }

        // --- Cargo Contracts (jobs offered here) -----------------------------
        crate::font::heading(|| draw_text("Cargo Contracts", x, ry, fs_heading() as f32, ink()));
        ry += line_h(fs_heading());
        // The deposit is a refundable surety, not a fee: the shipper posts it as a
        // pledge, then on delivery gets it back *and* the reward on top. Spell it out
        // — test players read the payout as reward-minus-deposit.
        for flavor in [
            "The shipper posts a deposit as surety that the cargo reaches port.",
            "On delivery your deposit is returned in full, plus the reward on top.",
        ] {
            draw_text(flavor, x, ry, fs_small() as f32, dim_ink());
            ry += line_h(fs_small());
        }
        // Breathing room before the column labels / contract rows.
        ry += gap();
        draw_text("Cargo", x, ry, fs_small() as f32, dim_ink());
        draw_text("To", x + w * CON_TO_X, ry, fs_small() as f32, dim_ink());
        right_text("Deposit", x + w * CON_DEP_R, ry, fs_small());
        right_text("Reward", x + w * CON_REW_R, ry, fs_small());
        rule(x, ry + rule_gap(), w);
        ry += line_h(fs_heading());

        let offered = mission::offered_at(gs, world);
        if !hull::can_take_jobs(gs) {
            // A hull below 30% is too disreputable to be hired — flag it plainly so
            // the failed Accept isn't a mystery.
            draw_text(
                "Hull too battered — no cargo will be entrusted to you.",
                x,
                ry,
                fs_body() as f32,
                flash_red(),
            );
            ry += step;
        } else if offered.is_empty() {
            draw_text("No contracts on the board.", x, ry, fs_body() as f32, dim_ink());
            ry += step;
        } else {
            for m in &offered {
                let to = world.islands[m.target_id as usize].name.clone();
                let focus = Focus::Contract(m.id);
                self.contract_line(m, &to, true, "Accept", focus, x, ry, w);
                ry += step;
            }
        }

        // --- Reserved cargo bound elsewhere (the hold manifest) --------------
        let reserved = mission::reserved_at(gs, world);
        if !reserved.is_empty() {
            ry += gap();
            crate::font::heading(|| draw_text("Reserved Cargo · Hold Manifest", x, ry, fs_heading() as f32, ink()));
            ry += line_h(fs_heading());
            for m in &reserved {
                let to = format!("→ {}", world.islands[m.target_id as usize].name);
                let focus = Focus::Manifest(m.id);
                self.contract_line(m, &to, false, "Abandon", focus, x, ry, w);
                ry += step;
            }
        }
    }

    /// One contract/delivery/manifest row: cargo, the other port, optionally the
    /// deposit & reward, and the action chip.
    #[allow(clippy::too_many_arguments)]
    fn contract_line(
        &self,
        m: &mission::Mission,
        to_text: &str,
        show_money: bool,
        chip: &str,
        focus: Focus,
        x: f32,
        ry: f32,
        w: f32,
    ) {
        use style::*;
        let active = self.focus == focus;
        if active {
            highlight_row(x, ry, w);
        }
        // Tapping the row body selects it (so the chart previews the leg); only the
        // action chip commits — mirroring the market's row-vs-chip split.
        self.record_hit(
            row_rect(x, ry, w),
            HitEffect::Select { focus, column: None, activate: false },
        );
        // The haulage units jiggle red when accepting would overflow the hold.
        let (udx, ured) = self.flash_of(FlashTarget::Units(m.id)).unwrap_or((0.0, 0.0));
        draw_text(
            format!("{} {}", m.quantity, m.good.label()),
            x + udx,
            ry,
            fs_body() as f32,
            flash_ink(ured),
        );
        draw_text(to_text, x + w * CON_TO_X, ry, fs_body() as f32, ink());
        if show_money {
            // The deposit jiggles red when the purse can't cover the buy-in.
            right_text_flash(
                &m.deposit.to_string(),
                x + w * CON_DEP_R,
                ry,
                fs_body(),
                self.flash_of(FlashTarget::Deposit(m.id)),
            );
            right_text(&m.reward.to_string(), x + w * CON_REW_R, ry, fs_body());
        }
        let action_x = x + w * CON_ACT_X;
        let chip_rect = Rect::new(action_x, chip_y(ry), x + w - action_x, chip_h());
        button(chip_rect.x, chip_rect.y, chip_rect.w, chip_rect.h, chip, active);
        self.record_hit(
            chip_rect,
            HitEffect::Select { focus, column: None, activate: true },
        );
    }
}

/// Render the "press Space to dock" call-to-action when a port is in range,
/// drawn in screen space over the sea. `sail_struck` gates the action text.
/// `race_target` is the finish-line island of a race in progress (if any): we
/// suppress the prompt there so a novice doesn't strike sail short of the line.
pub fn render_prompt(
    harbor: &Harbor,
    world: &World,
    sail_struck: bool,
    race_target: Option<i32>,
    w: f32,
    h: f32,
) {
    let Some(id) = harbor.dockable else { return };
    if harbor.is_open() {
        return;
    }
    // Don't urge "strike sail to enter port" at the very island we're racing to.
    if race_target == Some(id) {
        return;
    }
    let name = &world.islands[id as usize].name;
    let msg = if sail_struck {
        format!("Press  Space  to dock at {name}")
    } else {
        format!("Strike sail (S) to enter {name}")
    };
    // Drawn over the open sea (not the parchment board), so it keeps the larger
    // title size to catch the eye; sizes/spacing still come from `style`.
    use style::*;
    let fs = fs_title();
    let dims = measure_text(&msg, None, fs, 1.0);
    let bx = w * 0.5 - dims.width / 2.0;
    let by = h * 0.80;
    let pill_h = line_h(fs) + gap();
    let center = by - fs as f32 * CAP_RATIO;
    draw_rectangle(
        bx - gap(),
        center - pill_h / 2.0,
        dims.width + 2.0 * gap(),
        pill_h,
        Color::new(0.0, 0.0, 0.0, SCRIM),
    );
    draw_text(&msg, bx, by, fs as f32, WHITE);
}

// --- Small drawing helpers ----------------------------------------------------

// The ink / parchment palette and the type scale are shared with the captain's log;
// see `crate::ui`.
use crate::ui::{alarm_ink, dim_ink, format_dist, ink, parchment, parchment_edge};

fn row_highlight() -> Color {
    Color::new(150.0 / 255.0, 110.0 / 255.0, 60.0 / 255.0, 0.28)
}

/// Draw `text` right-aligned so its right edge sits at `right_x`.
fn right_text(text: &str, right_x: f32, y: f32, fs: u16) {
    let dims = measure_text(text, None, fs, 1.0);
    draw_text(text, right_x - dims.width, y, fs as f32, ink());
}

/// A hairline divider across `w` at `y`.
fn rule(x: f32, y: f32, w: f32) {
    draw_line(x, y, x + w, y, style::rule_w(), dim_ink());
}

/// A section header in the display face, with a hairline rule under it spanning `w`.
/// Returns the baseline `y` for the first body line beneath it.
fn section(text: &str, x: f32, y: f32, w: f32) -> f32 {
    crate::font::heading(|| draw_text(text, x, y, style::fs_heading() as f32, ink()));
    rule(x, y + style::rule_gap(), w);
    y + style::line_h(style::fs_heading())
}

/// One data line: a dim label on the left, its value at column `val_x`.
fn stat(label: &str, value: &str, x: f32, val_x: f32, y: f32) {
    draw_text(label, x, y, style::fs_body() as f32, dim_ink());
    draw_text(value, val_x, y, style::fs_body() as f32, ink());
}

/// The vertical centre of a list row whose text baseline is `ry`.
fn row_center(ry: f32) -> f32 {
    ry - style::fs_body() as f32 * style::CAP_RATIO
}

/// The rect of a list row whose text baseline is `ry`: `w` plus a small overhang
/// either side. The focus highlight and the touch hit-region share it, so they
/// can't drift apart.
fn row_rect(x: f32, ry: f32, w: f32) -> Rect {
    let h = style::row_h();
    Rect::new(
        x - style::row_pad_x(),
        row_center(ry) - h / 2.0,
        w + 2.0 * style::row_pad_x(),
        h,
    )
}

/// The focus highlight behind a list row (text baseline `ry`).
fn highlight_row(x: f32, ry: f32, w: f32) {
    let r = row_rect(x, ry, w);
    draw_rectangle(r.x, r.y, r.w, r.h, row_highlight());
}

/// The top-left `y` for a `chip_h()`-tall chip centred on a row whose baseline is `ry`.
fn chip_y(ry: f32) -> f32 {
    row_center(ry) - style::chip_h() / 2.0
}

/// Roman numeral for a small tier (covers every fitting ladder).
fn roman(n: i32) -> &'static str {
    match n {
        1 => "I",
        2 => "II",
        3 => "III",
        4 => "IV",
        5 => "V",
        6 => "VI",
        7 => "VII",
        _ => "VIII",
    }
}

/// A hull tier as the captain reads it: a 0-indexed level shown as `Mk I`..`Mk IV`.
fn hull_mark(level0: i32) -> String {
    format!("Mk {}", roman(level0 + 1))
}

/// The alarm red an out-of-bounds constraint flashes toward.
fn flash_red() -> Color {
    Color::new(0.80, 0.13, 0.10, 1.0)
}

/// Ink blended toward the alarm red by `red` (0 = normal ink, 1 = full red).
fn flash_ink(red: f32) -> Color {
    let (a, b) = (ink(), flash_red());
    Color::new(
        a.r + (b.r - a.r) * red,
        a.g + (b.g - a.g) * red,
        a.b + (b.b - a.b) * red,
        1.0,
    )
}

/// Right-aligned text with an optional `(dx, redness)` jiggle: shifted by `dx`
/// and tinted toward red, so a violated constraint shakes and lights up.
fn right_text_flash(text: &str, right_x: f32, y: f32, fs: u16, flash: Option<(f32, f32)>) {
    let (dx, red) = flash.unwrap_or((0.0, 0.0));
    let dims = measure_text(text, None, fs, 1.0);
    draw_text(text, right_x - dims.width + dx, y, fs as f32, flash_ink(red));
}

/// The filled colour of a focused chip / active tab.
fn chip_fill() -> Color {
    Color::new(0.31, 0.19, 0.09, 1.0)
}

/// A small action chip: filled when focused, outlined otherwise, label centred.
fn button(x: f32, y: f32, w: f32, h: f32, label: &str, focused: bool) {
    if focused {
        draw_rectangle(x, y, w, h, chip_fill());
    } else {
        draw_rectangle_lines(x, y, w, h, style::border_w(), parchment_edge());
    }
    let fs = style::fs_chip();
    let dims = measure_text(label, None, fs, 1.0);
    let c = if focused { parchment() } else { ink() };
    draw_text(
        label,
        x + (w - dims.width) / 2.0,
        y + h / 2.0 + fs as f32 * style::CAP_RATIO,
        fs as f32,
        c,
    );
}

#[cfg(test)]
mod dock_cycle_tests {
    use super::*;
    use crate::world::{Cluster, IsleKind};

    fn one_port_world() -> World {
        let isle = Island {
            id: 0,
            name: "Test Port".into(),
            pos: Vec2::new(0.0, 0.0),
            radius: 100.0,
            height: 20.0,
            terrain: IsleKind::Green,
            is_port: true,
            is_shipyard: true,
        };
        World {
            seed: 1,
            islands: vec![isle],
            clusters: vec![Cluster {
                id: 0,
                name: "C".into(),
                center: Vec2::ZERO,
                island_ids: vec![0],
            }],
        }
    }

    // Park the ship `dist` metres due south of the port, bow swung to `heading`
    // (0 = north, straight at the port). Drives `update_dockable` for one frame.
    fn at(harbor: &mut Harbor, world: &World, dist: f32, heading: f32) {
        let kin = Kinematics::still(Vec2::new(0.0, -dist), heading);
        harbor.update_dockable(world, &kin);
    }

    // The bug: after docking and leaving, the port could not be re-entered. Casting
    // off and at once facing the port (in range) must let the captain tie up again.
    #[test]
    fn can_re_enter_a_port_right_after_casting_off() {
        let world = one_port_world();
        let mut harbor = Harbor::new();
        let mut gs = GameState::start();

        // Approach within range, bow on the port — the prompt is offered.
        at(&mut harbor, &world, 300.0, 0.0);
        assert!(harbor.dockable.is_some(), "approach should offer the dock");

        // Dock, then cast off. Still in range with the bow on the port, the prompt
        // is back at once — the captain can re-enter right away.
        assert!(harbor.try_dock(&mut gs));
        harbor.set_sail(&mut gs);
        at(&mut harbor, &world, 300.0, 0.0);
        assert!(
            harbor.dockable.is_some(),
            "facing the port in range should offer the dock again immediately"
        );
        assert!(harbor.try_dock(&mut gs), "and the captain can re-enter");
    }

    // The prompt is gone only when the bow is off the port, or it is out of range.
    #[test]
    fn no_prompt_when_facing_away_or_out_of_range() {
        let world = one_port_world();
        let mut harbor = Harbor::new();

        // In range but bow pointed away — no prompt.
        at(&mut harbor, &world, 300.0, std::f32::consts::PI);
        assert!(harbor.dockable.is_none(), "no prompt while pointed away");

        // Bow on the port but out of dock range — no prompt.
        at(&mut harbor, &world, 1000.0, 0.0);
        assert!(harbor.dockable.is_none(), "no prompt out of range");

        // In range and facing it — prompt.
        at(&mut harbor, &world, 300.0, 0.0);
        assert!(harbor.dockable.is_some(), "in range and facing offers the dock");
    }
}
