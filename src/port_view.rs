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

use macroquad::prelude::*;

use crate::game_state::{hull, upgrades, GameState, Good, Location, Market, TradeError, UpgradeKind};
use crate::geometry::{wrap_angle, Vec2};
use crate::minimap::{self, MinimapPalette};
use crate::mission;
use crate::race;
use crate::sailing::{Kinematics, Wind};
use crate::sound::SoundBank;
use crate::world::{Island, World};

// How far off the bow a port may sit and still raise the docking prompt: a
// forward arc of ±60°, so a port ahead offers to dock but one abeam or astern
// does not (`SailingView.dockFacingArc`).
const DOCK_FACING_ARC: f32 = std::f32::consts::PI / 3.0;

/// A port within dock range of `pos`, if any. The bow-facing check that decides
/// whether it can actually be entered lives in [`Harbor::update_dockable`].
pub fn port_at<'w>(world: &'w World, pos: Vec2) -> Option<&'w Island> {
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
}

/// A live red-jiggle on one constraint, started at `start` (seconds, `get_time`).
struct Flash {
    target: FlashTarget,
    start: f64,
}

pub struct PortScreen {
    island_id: i32,
    tab: Tab,
    focus: Focus,
    column: usize, // commodity action column: 0 Buy · 1 Fill · 2 Dump · 3 Sell
    /// Constraints currently flashing from a rejected action (see [`FlashTarget`]).
    flashes: Vec<Flash>,
}

// Flash timing: a brief red jiggle that decays to nothing.
const FLASH_DUR: f32 = 0.42; // seconds the jiggle lasts
const FLASH_AMP: f32 = 4.0; // peak horizontal wobble, px
const FLASH_FREQ: f32 = 7.0; // wobble oscillations per second

const TABS: [Tab; 4] = [Tab::Market, Tab::Contracts, Tab::Yard, Tab::Race];
const LAST_COLUMN: usize = 3;

/// The port overlay's whole visual style in one place — every type size, spacing
/// step, column split and symbol the board draws is named here, so the render code
/// below carries no bare pixel literals. Sizes are deliberately compact.
mod style {
    // --- Type scale & line height — the one shared UI ladder ----------------
    // Lives in `crate::ui` so the captain's log draws on the very same scale.
    pub use crate::ui::{line_h, FS_BODY, FS_CHIP, FS_HEADING, FS_SMALL, FS_TITLE};

    // --- Spacing — every gap is a multiple of one base unit -----------------
    pub const UNIT: f32 = 6.0;
    pub const PAD: f32 = UNIT * 4.0; // panel inner margin (24)
    pub const GAP: f32 = UNIT * 2.0; // gap between groups (12)
    pub const RULE_GAP: f32 = UNIT; // heading baseline → its underline rule (6)
    pub const COL_GAP: f32 = UNIT * 4.0; // chart column → board column (24)

    // --- The panel itself ---------------------------------------------------
    pub const SCRIM: f32 = 0.5; // alpha of the dim behind the board
    pub const PANEL_W_FRAC: f32 = 0.92; // panel size as a fraction of the screen…
    pub const PANEL_H_FRAC: f32 = 0.9;
    pub const PANEL_MAX_W: f32 = 940.0; // …capped here
    pub const PANEL_MAX_H: f32 = 560.0;
    pub const PANEL_BORDER: f32 = UNIT * 0.5; // panel edge stroke (3)
    /// Tab-bar button height.
    pub fn tab_h() -> f32 {
        line_h(FS_BODY) + UNIT
    }

    // --- Rules & chips ------------------------------------------------------
    pub const RULE_W: f32 = 1.0; // hairline divider thickness
    pub const BORDER_W: f32 = 1.5; // chip / outline stroke thickness
    /// Baseline drop from a line's vertical centre, as a fraction of font size —
    /// used to vertically centre text in a chip.
    pub const CAP_RATIO: f32 = 0.35;
    pub const CHIP_H: f32 = UNIT * 4.0; // action chip / button height (24)
    pub const CHIP_W: f32 = UNIT * 17.0; // a fitting's fixed chip width (102)
    pub const CHIP_GAP: f32 = UNIT; // gap a cost keeps left of its chip (6)
    pub const CHIP_INNER: f32 = UNIT * 0.5; // gap between packed action chips (3)
    pub const ROW_PAD_X: f32 = UNIT; // a focus highlight's horizontal overhang
    /// Height of a list row (a chip plus breathing room).
    pub fn row_h() -> f32 {
        CHIP_H + UNIT
    }
    pub const TAB_PAD_X: f32 = UNIT * 2.0; // padding either side of a tab label
    pub const TAB_GAP: f32 = UNIT * 2.0; // gap between tabs
    pub const BTN_WIDE: f32 = UNIT * 52.0; // a full-width action button's cap (312)

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
        }
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

    /// The navigable rows of the active tab, top to bottom (the tab bar is its
    /// own focus, above these). Derived from the live state so it always matches
    /// what's on screen as contracts come and go.
    fn rows_of(&self, gs: &GameState, world: &World, tab: Tab) -> Vec<Focus> {
        match tab {
            Tab::Market => (0..Good::ALL.len()).map(Focus::Good).collect(),
            Tab::Contracts => {
                let mut v = Vec::new();
                // A hull too battered to be hired can't take on *new* contracts, so
                // those rows aren't focusable — but deliveries owed and abandoning
                // reserved cargo stay open.
                if hull::can_take_jobs(gs) {
                    v.extend(mission::offered_at(gs, world).iter().map(|m| Focus::Contract(m.id)));
                }
                v.extend(mission::deliverable_at(gs, world).iter().map(|m| Focus::Delivery(m.id)));
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

    fn cycle_tab(&mut self, delta: i32) {
        let i = TABS.iter().position(|t| *t == self.tab).unwrap_or(0);
        let n = TABS.len() as i32;
        self.tab = TABS[((i as i32 + delta).rem_euclid(n)) as usize];
        self.focus = Focus::TabBar;
    }

    /// Switch to the adjacent tab but stay down in its rows (Left/Right paging
    /// once the cursor runs off the end of a row).
    fn slide_tab(&mut self, gs: &GameState, world: &World, delta: i32) {
        let i = TABS.iter().position(|t| *t == self.tab).unwrap_or(0);
        let n = TABS.len() as i32;
        self.tab = TABS[((i as i32 + delta).rem_euclid(n)) as usize];
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
                let done = match self.column {
                    0 => gs.buy(market, good, 1),
                    1 => gs.fill(market, good),
                    2 => gs.dump(market, good),
                    _ => gs.sell(market, good, 1),
                };
                match done {
                    Ok(()) => sounds.transaction(),
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
                Ok(()) => sounds.transaction(),
                Err(e) => {
                    sounds.invalid();
                    if e == TradeError::NotEnoughGold {
                        self.flash(FlashTarget::Gold);
                    }
                }
            },
            Focus::Upgrade(kind) => match gs.buy_upgrade(world, kind) {
                Ok(()) => sounds.transaction(),
                Err(e) => {
                    sounds.invalid();
                    if e == TradeError::NotEnoughGold {
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
                        sounds.transaction();
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
                    sounds.transaction();
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
                    sounds.transaction();
                    self.focus = self
                        .rows_of(gs, world, self.tab)
                        .first()
                        .copied()
                        .unwrap_or(Focus::TabBar);
                }
                Err(_) => sounds.invalid(),
            },
        }
    }

    /// Read keys and drive the board. Returns true when the captain sets sail.
    pub fn handle_input(
        &mut self,
        gs: &mut GameState,
        world: &World,
        market: &Market,
        sounds: &SoundBank,
    ) -> bool {
        if is_key_pressed(KeyCode::Escape) {
            return true;
        }
        if is_key_pressed(KeyCode::Tab) {
            let back = is_key_down(KeyCode::LeftShift) || is_key_down(KeyCode::RightShift);
            self.cycle_tab(if back { -1 } else { 1 });
        }
        if is_key_pressed(KeyCode::Up) {
            self.move_cursor(gs, world, -1);
        }
        if is_key_pressed(KeyCode::Down) {
            self.move_cursor(gs, world, 1);
        }
        if is_key_pressed(KeyCode::Left) {
            match self.focus {
                Focus::TabBar => self.cycle_tab(-1),
                Focus::Good(_) if self.column > 0 => self.column -= 1,
                _ => self.slide_tab(gs, world, -1),
            }
        }
        if is_key_pressed(KeyCode::Right) {
            match self.focus {
                Focus::TabBar => self.cycle_tab(1),
                Focus::Good(_) if self.column < LAST_COLUMN => self.column += 1,
                _ => self.slide_tab(gs, world, 1),
            }
        }
        if is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::Space) {
            self.activate(gs, world, market, sounds);
        }
        false
    }

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

        // Dim the world so the board reads as the captain's focus.
        draw_rectangle(0.0, 0.0, w, h, Color::new(0.0, 0.0, 0.0, SCRIM));

        let pw = (w * PANEL_W_FRAC).min(PANEL_MAX_W);
        let ph = (h * PANEL_H_FRAC).min(PANEL_MAX_H);
        let x0 = (w - pw) / 2.0;
        let y0 = (h - ph) / 2.0;
        draw_rectangle(x0, y0, pw, ph, parchment());
        draw_rectangle_lines(x0, y0, pw, ph, PANEL_BORDER, parchment_edge());

        let left = x0 + PAD;
        let right = x0 + pw - PAD;
        let inner_w = pw - 2.0 * PAD;

        // --- Header: eyebrow, port name, purse ------------------------------
        let eyebrow = if port.is_shipyard {
            "SHIPYARD · PORT OF CALL"
        } else {
            "PORT OF CALL"
        };
        let eyebrow_y = y0 + PAD + FS_SMALL as f32;
        let name_y = eyebrow_y + line_h(FS_TITLE);
        crate::font::heading(|| {
            draw_text(eyebrow, left, eyebrow_y, FS_SMALL as f32, dim_ink());
            draw_text(&port.name, left, name_y, FS_TITLE as f32, ink());
        });

        // Purse, right-aligned in the header. Either lights red and jiggles when an
        // action runs out of coin (the purse) or out of hold room (the tally).
        let gold = format!("Gold {}", gs.gold);
        let hold = format!("Hold {}/{}", gs.hold_used(), gs.hold_capacity);
        let gold_y = y0 + PAD + FS_HEADING as f32;
        let hold_y = gold_y + line_h(FS_SMALL);
        right_text_flash(&gold, right, gold_y, FS_HEADING, self.flash_of(FlashTarget::Gold));
        right_text_flash(&hold, right, hold_y, FS_SMALL, self.flash_of(FlashTarget::Hold));

        let bar_y = name_y + GAP;
        rule(left, bar_y, inner_w);

        // --- Tab bar --------------------------------------------------------
        let yard_label = if port.is_shipyard { "Shipyard" } else { "Drydock" };
        let on_bar = self.focus == Focus::TabBar;
        let tab_y = bar_y + GAP;
        let mut tx = left;
        tx = self.tab_button("Market", Tab::Market, tx, tab_y, on_bar);
        tx = self.tab_button("Contracts", Tab::Contracts, tx, tab_y, on_bar);
        tx = self.tab_button(yard_label, Tab::Yard, tx, tab_y, on_bar);
        let _ = self.tab_button("Racing", Tab::Race, tx, tab_y, on_bar);

        // --- Body: chart on the left, the active board on the right ----------
        let body_top = tab_y + tab_h() + GAP;
        let chart_size = (ph - (body_top - y0) - PAD).min(pw * CHART_FRAC);
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
        let cd = measure_text(waters, None, FS_SMALL, 1.0);
        draw_text(
            waters,
            chart.x + (chart_size - cd.width) / 2.0,
            chart.y + chart_size + line_h(FS_SMALL),
            FS_SMALL as f32,
            ink(),
        );

        let board_x = chart.x + chart_size + COL_GAP;
        let board_w = right - board_x;
        // The chart on the left is a rect whose *top edge* sits at `body_top`, but the
        // boards lead with text drawn on a *baseline* — so starting them at `body_top`
        // floats their first line a full ascent above it, riding up against the tab bar.
        // Drop the board down by one heading ascent so its first line's cap-height lines
        // up with the chart's top edge.
        let board_top = body_top + FS_HEADING as f32;
        match self.tab {
            Tab::Market => self.render_market(gs, market, board_x, board_top, board_w),
            Tab::Contracts => self.render_contracts(gs, world, board_x, board_top, board_w),
            Tab::Yard => self.render_yard(gs, world, board_x, board_top, board_w),
            Tab::Race => self.render_race(gs, world, board_x, board_top, board_w),
        }

        // Footer hint.
        draw_text(
            "Arrows move · Tab switches board · Enter trades · Esc sets sail",
            left,
            y0 + ph - PAD,
            FS_SMALL as f32,
            dim_ink(),
        );
    }

    /// Draw a tab button; returns the x where the next one should start.
    fn tab_button(&self, label: &str, tab: Tab, x: f32, y: f32, on_bar: bool) -> f32 {
        use style::*;
        let dims = measure_text(label, None, FS_BODY, 1.0);
        let bw = dims.width + 2.0 * TAB_PAD_X;
        let bh = tab_h();
        let active = self.tab == tab;
        if active {
            draw_rectangle(x, y, bw, bh, chip_fill());
            // A lit ring while the cursor rests on the bar.
            if on_bar {
                draw_rectangle_lines(x, y, bw, bh, BORDER_W + 1.0, tab_ring());
            }
        } else {
            draw_rectangle_lines(x, y, bw, bh, BORDER_W, parchment_edge());
        }
        let c = if active { parchment() } else { ink() };
        draw_text(label, x + TAB_PAD_X, y + bh / 2.0 + FS_BODY as f32 * CAP_RATIO, FS_BODY as f32, c);
        x + bw + TAB_GAP
    }

    fn render_market(&self, gs: &GameState, market: &Market, x: f32, y: f32, w: f32) {
        use style::*;
        // Column anchors within the board.
        let price_r = x + w * MKT_PRICE_R; // right edge of the price column
        let held_r = x + w * MKT_HELD_R; // right edge of the hold column
        let actions_x = x + w * MKT_ACTIONS_X;

        draw_text("Commodity", x, y, FS_SMALL as f32, dim_ink());
        right_text("Price", price_r, y, FS_SMALL);
        right_text("Hold", held_r, y, FS_SMALL);
        draw_text("Trade", actions_x, y, FS_SMALL as f32, dim_ink());
        rule(x, y + RULE_GAP, w);

        const ACTIONS: [&str; 4] = ["Buy", "Fill", "Dump", "Sell"];
        let step = row_h();
        let mut ry = y + step;
        for (i, good) in Good::ALL.iter().enumerate() {
            let active_row = self.focus == Focus::Good(i);
            if active_row {
                highlight_row(x, ry, w);
            }
            draw_text(good.label(), x, ry, FS_BODY as f32, ink());
            right_text(&market.price(*good).to_string(), price_r, ry, FS_BODY);
            // The held tally jiggles red on a Sell/Dump with nothing to sell.
            right_text_flash(
                &gs.quantity_of(*good).to_string(),
                held_r,
                ry,
                FS_BODY,
                self.flash_of(FlashTarget::Held(i)),
            );

            // Four action chips.
            let chip_w = (x + w - actions_x) / 4.0;
            for (c, label) in ACTIONS.iter().enumerate() {
                let cx = actions_x + c as f32 * chip_w;
                let focused = active_row && self.column == c;
                button(cx + CHIP_INNER, chip_y(ry), chip_w - 2.0 * CHIP_INNER, CHIP_H, label, focused);
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
        let chip_x = x + w - CHIP_W;
        let cost_r = chip_x - CHIP_GAP; // cost right-aligned just left of the chip
        let step = line_h(FS_BODY);

        // A cost right-aligned left of the action chip, both centred in a block of
        // height `bh` whose top is `ry`.
        let cost_chip = |ry: f32, bh: f32, cost: &str, label: &str, focused: bool| {
            let chip_top = ry + (bh - CHIP_H) / 2.0;
            button(chip_x, chip_top, CHIP_W, CHIP_H, label, focused);
            right_text(cost, cost_r, chip_top + CHIP_H / 2.0 + FS_BODY as f32 * CAP_RATIO, FS_BODY);
        };
        let highlight = |ry: f32, bh: f32| {
            draw_rectangle(x - ROW_PAD_X, ry, w + 2.0 * ROW_PAD_X, bh, row_highlight());
        };

        // ===== Drydock — hull repair: one line ==============================
        let mut ry = section("DRYDOCK · HULL REPAIR", x, y, w);
        {
            let active = self.focus == Focus::Repair;
            let bh = step + GAP;
            if active {
                highlight(ry, bh);
            }
            let base = ry + step;
            let cond = format!(
                "{} / {} ({}%)",
                gs.hull,
                gs.max_hull(),
                (hull::fraction(gs) * 100.0).round() as i32
            );
            draw_text("Hull · Mend", x, base, FS_BODY as f32, ink());
            draw_text(&cond, val_x, base, FS_BODY as f32, ink());
            let dmg = hull::damage(gs);
            let cost = if dmg <= 0 { "—".to_string() } else { hull::repair_cost(gs).to_string() };
            cost_chip(ry, bh, &cost, if dmg <= 0 { "Sound" } else { "Repair" }, active);
            ry += bh + GAP;
        }

        // ===== Shipyard — hull / sails / hold fittings ======================
        ry = section("SHIPYARD · OUTFITTING", x, ry, w);
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

                // The fitting's data lines: the current→next gain, and (for hull and
                // sails) a live readout of how the ship stands today. The hold shows
                // only its slot count — its live "in use" is already on the purse line.
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
                            lines.push(("Haul capacity", format!("{haul} units")));
                        } else {
                            let h1 = upgrades::max_haul(gs.sail_level + 1);
                            lines.push(("Haul capacity", format!("{haul}{ARROW}{h1} units")));
                        }
                        lines.push(("Now carrying", format!("{} / {} units", gs.hold_used(), haul)));
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
                let bh = (1 + lines.len()) as f32 * step + GAP;
                if active {
                    highlight(ry, bh);
                }
                draw_text(&title, x, ry + step, FS_BODY as f32, ink());
                for (i, (label, value)) in lines.iter().enumerate() {
                    stat(label, value, x, val_x, ry + step * (i as f32 + 2.0));
                }

                let (cost, label) = match upgrades::next_cost(kind, gs) {
                    None => ("MAX".to_string(), "Maxed"),
                    Some(c) => (c.to_string(), "Fit"),
                };
                cost_chip(ry, bh, &cost, label, active);
                ry += bh + GAP;
            }
        } else {
            draw_text(
                "No shipyard here — find a shipyard port to outfit.",
                x,
                ry + step,
                FS_BODY as f32,
                dim_ink(),
            );
        }
    }

    /// The Racing board: wager on beating a computer-helmed rival to another port.
    /// With no race booked it shows the day's rival ports (each with its stake) and
    /// a challenge button; with one booked it shows the armed race and a withdraw.
    fn render_race(&self, gs: &GameState, world: &World, x: f32, y: f32, w: f32) {
        use style::*;
        let origin = &world.islands[self.island_id as usize];
        crate::font::heading(|| draw_text("Harbour Race · Wager", x, y, FS_HEADING as f32, ink()));
        let mut ry = y + line_h(FS_HEADING);
        let step = line_h(FS_BODY);

        // Label (dim, left) + value (right-aligned) line within the board.
        let line = |label: &str, value: &str, ry: f32| {
            draw_text(label, x, ry, FS_BODY as f32, dim_ink());
            right_text(value, x + w, ry, FS_BODY);
        };

        if let Some(race) = gs.race {
            let target = &world.islands[race.target_id as usize];
            crate::font::heading(|| draw_text("Race booked", x, ry, FS_HEADING as f32, ink()));
            ry += line_h(FS_HEADING);
            line("Race to", &target.name, ry);
            ry += step;
            line("Rival hull", &hull_mark(race.required_level), ry);
            ry += step;
            line("Stake", &race.stake.to_string(), ry);
            ry += step;
            line("On winning", &(race.stake * 2).to_string(), ry);
            ry += step + GAP;
            draw_text("Set sail and the rival draws up alongside.", x, ry, FS_SMALL as f32, dim_ink());
            ry += line_h(FS_SMALL);
            draw_text("Heave to, then raise sail to start level.", x, ry, FS_SMALL as f32, dim_ink());
            ry += line_h(FS_SMALL) + GAP;
            let focused = self.focus == Focus::RaceWithdraw;
            button(x, ry, w.min(BTN_WIDE), CHIP_H, "Abandon race (stake refunded)", focused);
            return;
        }

        if !hull::can_take_jobs(gs) {
            // No harbour will stake a wreck — flag it where the rival card would be.
            draw_text(
                "Hull too battered — no harbour will stake you in a race.",
                x,
                ry,
                FS_BODY as f32,
                flash_red(),
            );
            return;
        }

        let offers = race::offers(gs, world);
        if offers.is_empty() {
            draw_text("No rival ports in these waters to race to.", x, ry, FS_BODY as f32, dim_ink());
            return;
        }

        draw_text("Beat a rival sloop to another port. The stake rises", x, ry, FS_SMALL as f32, dim_ink());
        ry += line_h(FS_SMALL);
        draw_text("with the distance of the leg. Enter to take one on.", x, ry, FS_SMALL as f32, dim_ink());
        ry += line_h(FS_SMALL) + GAP;

        // Columns: port name, the required hull tier, the stake right-aligned, then
        // an Accept chip — the same shape as a contract row.
        let tier_r = x + w * RACE_TIER_R;
        let stake_r = x + w * RACE_STAKE_R;
        let action_x = x + w * RACE_ACT_X;
        draw_text("Race to", x, ry, FS_SMALL as f32, dim_ink());
        right_text("Hull", tier_r, ry, FS_SMALL);
        right_text("Stake", stake_r, ry, FS_SMALL);
        rule(x, ry + RULE_GAP, w);
        ry += line_h(FS_HEADING);

        // Each rival port is its own row: highlight it and Enter (or the Accept
        // chip) books the race — none is marked until then, the same as contracts.
        for p in &offers {
            let active = self.focus == Focus::RaceTarget(p.id);
            if active {
                highlight_row(x, ry, w);
            }
            draw_text(&p.name, x, ry, FS_BODY as f32, ink());
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
                let dims = measure_text(&tier_txt, None, FS_BODY, 1.0);
                draw_text(&tier_txt, tier_r - dims.width + dx, ry, FS_BODY as f32, flash_red());
            } else {
                right_text(&tier_txt, tier_r, ry, FS_BODY);
            }
            // The stake jiggles red when the purse can't cover the wager.
            right_text_flash(
                &stake.to_string(),
                stake_r,
                ry,
                FS_BODY,
                self.flash_of(FlashTarget::Stake(p.id)),
            );
            button(action_x, chip_y(ry), x + w - action_x, CHIP_H, "Accept", active);
            ry += row_h();
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

        // --- Cargo Contracts (jobs offered here) -----------------------------
        crate::font::heading(|| draw_text("Cargo Contracts", x, ry, FS_HEADING as f32, ink()));
        ry += line_h(FS_HEADING);
        draw_text("Cargo", x, ry, FS_SMALL as f32, dim_ink());
        draw_text("To", x + w * CON_TO_X, ry, FS_SMALL as f32, dim_ink());
        right_text("Deposit", x + w * CON_DEP_R, ry, FS_SMALL);
        right_text("Reward", x + w * CON_REW_R, ry, FS_SMALL);
        rule(x, ry + RULE_GAP, w);
        ry += line_h(FS_HEADING);

        let offered = mission::offered_at(gs, world);
        if !hull::can_take_jobs(gs) {
            // A hull below 30% is too disreputable to be hired — flag it plainly so
            // the failed Accept isn't a mystery.
            draw_text(
                "Hull too battered — no cargo will be entrusted to you.",
                x,
                ry,
                FS_BODY as f32,
                flash_red(),
            );
            ry += step;
        } else if offered.is_empty() {
            draw_text("No contracts on the board.", x, ry, FS_BODY as f32, dim_ink());
            ry += step;
        } else {
            for m in &offered {
                let to = world.islands[m.target_id as usize].name.clone();
                let active = self.focus == Focus::Contract(m.id);
                self.contract_line(m, &to, true, "Accept", active, x, ry, w);
                ry += step;
            }
        }

        // --- Deliveries owed at this very port -------------------------------
        let deliveries = mission::deliverable_at(gs, world);
        if !deliveries.is_empty() {
            ry += GAP;
            crate::font::heading(|| draw_text("Deliveries Awaiting", x, ry, FS_HEADING as f32, ink()));
            ry += line_h(FS_HEADING);
            for m in &deliveries {
                let from = format!("from {}", world.islands[m.origin_id as usize].name);
                let active = self.focus == Focus::Delivery(m.id);
                self.contract_line(m, &from, true, "Deliver", active, x, ry, w);
                ry += step;
            }
        }

        // --- Reserved cargo bound elsewhere (the hold manifest) --------------
        let reserved = mission::reserved_at(gs, world);
        if !reserved.is_empty() {
            ry += GAP;
            crate::font::heading(|| draw_text("Reserved Cargo · Hold Manifest", x, ry, FS_HEADING as f32, ink()));
            ry += line_h(FS_HEADING);
            for m in &reserved {
                let to = format!("→ {}", world.islands[m.target_id as usize].name);
                let active = self.focus == Focus::Manifest(m.id);
                self.contract_line(m, &to, false, "Abandon", active, x, ry, w);
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
        active: bool,
        x: f32,
        ry: f32,
        w: f32,
    ) {
        use style::*;
        if active {
            highlight_row(x, ry, w);
        }
        // The haulage units jiggle red when accepting would overflow the hold.
        let (udx, ured) = self.flash_of(FlashTarget::Units(m.id)).unwrap_or((0.0, 0.0));
        draw_text(
            &format!("{} {}", m.quantity, m.good.label()),
            x + udx,
            ry,
            FS_BODY as f32,
            flash_ink(ured),
        );
        draw_text(to_text, x + w * CON_TO_X, ry, FS_BODY as f32, ink());
        if show_money {
            // The deposit jiggles red when the purse can't cover the buy-in.
            right_text_flash(
                &m.deposit.to_string(),
                x + w * CON_DEP_R,
                ry,
                FS_BODY,
                self.flash_of(FlashTarget::Deposit(m.id)),
            );
            right_text(&m.reward.to_string(), x + w * CON_REW_R, ry, FS_BODY);
        }
        let action_x = x + w * CON_ACT_X;
        button(action_x, chip_y(ry), x + w - action_x, CHIP_H, chip, active);
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
    let fs = FS_TITLE;
    let dims = measure_text(&msg, None, fs, 1.0);
    let bx = w * 0.5 - dims.width / 2.0;
    let by = h * 0.80;
    let pill_h = line_h(fs) + GAP;
    let center = by - fs as f32 * CAP_RATIO;
    draw_rectangle(
        bx - GAP,
        center - pill_h / 2.0,
        dims.width + 2.0 * GAP,
        pill_h,
        Color::new(0.0, 0.0, 0.0, SCRIM),
    );
    draw_text(&msg, bx, by, fs as f32, WHITE);
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
                radius: 1000.0,
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

// --- Small drawing helpers ----------------------------------------------------

// The ink / parchment palette and the type scale are shared with the captain's log;
// see `crate::ui`.
use crate::ui::{dim_ink, ink, parchment, parchment_edge};

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
    draw_line(x, y, x + w, y, style::RULE_W, dim_ink());
}

/// A section header in the display face, with a hairline rule under it spanning `w`.
/// Returns the baseline `y` for the first body line beneath it.
fn section(text: &str, x: f32, y: f32, w: f32) -> f32 {
    crate::font::heading(|| draw_text(text, x, y, style::FS_HEADING as f32, ink()));
    rule(x, y + style::RULE_GAP, w);
    y + style::line_h(style::FS_HEADING)
}

/// One data line: a dim label on the left, its value at column `val_x`.
fn stat(label: &str, value: &str, x: f32, val_x: f32, y: f32) {
    draw_text(label, x, y, style::FS_BODY as f32, dim_ink());
    draw_text(value, val_x, y, style::FS_BODY as f32, ink());
}

/// The vertical centre of a list row whose text baseline is `ry`.
fn row_center(ry: f32) -> f32 {
    ry - style::FS_BODY as f32 * style::CAP_RATIO
}

/// The focus highlight behind a list row (text baseline `ry`), spanning `w` plus a
/// small overhang either side.
fn highlight_row(x: f32, ry: f32, w: f32) {
    let h = style::row_h();
    draw_rectangle(
        x - style::ROW_PAD_X,
        row_center(ry) - h / 2.0,
        w + 2.0 * style::ROW_PAD_X,
        h,
        row_highlight(),
    );
}

/// The top-left `y` for a `CHIP_H`-tall chip centred on a row whose baseline is `ry`.
fn chip_y(ry: f32) -> f32 {
    row_center(ry) - style::CHIP_H / 2.0
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

/// The lit gold ring around the active tab while the cursor rests on the bar.
fn tab_ring() -> Color {
    Color::new(0.85, 0.66, 0.30, 1.0)
}

/// A small action chip: filled when focused, outlined otherwise, label centred.
fn button(x: f32, y: f32, w: f32, h: f32, label: &str, focused: bool) {
    if focused {
        draw_rectangle(x, y, w, h, chip_fill());
    } else {
        draw_rectangle_lines(x, y, w, h, style::BORDER_W, parchment_edge());
    }
    let fs = style::FS_CHIP;
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
