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

use crate::game_state::{hull, upgrades, GameState, Good, Location, Market, UpgradeKind};
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
    RaceChallenge,
    RaceWithdraw,
}

pub struct PortScreen {
    island_id: i32,
    tab: Tab,
    focus: Focus,
    column: usize, // commodity action column: 0 Buy · 1 Fill · 2 Dump · 3 Sell
    /// The Racing tab's working choice: which rival port to wager on. `None` falls
    /// back to the nearest on the day's card. Only matters until a race is booked.
    race_choice: Option<i32>,
}

const TABS: [Tab; 4] = [Tab::Market, Tab::Contracts, Tab::Yard, Tab::Race];
const LAST_COLUMN: usize = 3;

impl PortScreen {
    fn new(island_id: i32) -> PortScreen {
        PortScreen {
            island_id,
            tab: Tab::Market,
            focus: Focus::TabBar,
            column: 0,
            race_choice: None,
        }
    }

    /// The rival port currently chosen to race (the cursor's pick, or the nearest
    /// on the card by default). `None` only when there are no ports to race to.
    fn race_chosen(&self, gs: &GameState, world: &World) -> Option<i32> {
        self.race_choice
            .or_else(|| race::offers(gs, world).first().map(|p| p.id))
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
                v.extend(mission::offered_at(gs, world).iter().map(|m| Focus::Contract(m.id)));
                v.extend(mission::deliverable_at(gs, world).iter().map(|m| Focus::Delivery(m.id)));
                v.extend(mission::reserved_at(gs, world).iter().map(|m| Focus::Manifest(m.id)));
                v
            }
            Tab::Yard => {
                let mut v = vec![Focus::Repair];
                if self.is_shipyard(world) {
                    v.push(Focus::Upgrade(UpgradeKind::Sail));
                    v.push(Focus::Upgrade(UpgradeKind::Cargo));
                }
                v
            }
            // While a race is booked the tab is just the armed race + a withdraw;
            // with none booked it is the day's rival ports and the challenge button.
            Tab::Race => {
                if gs.race.is_some() {
                    vec![Focus::RaceWithdraw]
                } else {
                    let mut v: Vec<Focus> = race::offers(gs, world)
                        .iter()
                        .map(|p| Focus::RaceTarget(p.id))
                        .collect();
                    v.push(Focus::RaceChallenge);
                    v
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
                let done = match self.column {
                    0 => gs.buy(market, good, 1),
                    1 => gs.fill(market, good),
                    2 => gs.dump(market, good),
                    _ => gs.sell(market, good, 1),
                };
                if done.is_ok() {
                    sounds.transaction();
                }
            }
            Focus::Repair => {
                if gs.repair().is_ok() {
                    sounds.transaction();
                }
            }
            Focus::Upgrade(kind) => {
                if gs.buy_upgrade(world, kind).is_ok() {
                    sounds.transaction();
                }
            }
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
                if done.is_ok() {
                    sounds.transaction();
                    let after = self.rows_of(gs, world, self.tab);
                    self.focus = if after.is_empty() {
                        Focus::TabBar
                    } else {
                        after[slot.min(after.len() - 1)]
                    };
                }
            }
            // Picking a target just arms the chart preview; the challenge books the
            // race (charging the stake), and withdraw drops a booked one.
            Focus::RaceTarget(id) => self.race_choice = Some(id),
            Focus::RaceChallenge => {
                if let Some(id) = self.race_chosen(gs, world) {
                    if gs.accept_race(world, id).is_ok() {
                        sounds.transaction();
                        // The picker rows are gone; land on the new withdraw row.
                        self.focus = Focus::RaceWithdraw;
                    }
                }
            }
            Focus::RaceWithdraw => {
                if gs.withdraw_race(world).is_ok() {
                    sounds.transaction();
                    self.focus = self
                        .rows_of(gs, world, self.tab)
                        .first()
                        .copied()
                        .unwrap_or(Focus::TabBar);
                }
            }
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
        let port = &world.islands[self.island_id as usize];

        // Dim the world so the board reads as the captain's focus.
        draw_rectangle(0.0, 0.0, w, h, Color::new(0.0, 0.0, 0.0, 0.5));

        let pw = (w * 0.92).min(940.0);
        let ph = (h * 0.9).min(560.0);
        let x0 = (w - pw) / 2.0;
        let y0 = (h - ph) / 2.0;
        draw_rectangle(x0, y0, pw, ph, parchment());
        draw_rectangle_lines(x0, y0, pw, ph, 3.0, parchment_edge());

        let pad = 30.0;

        // --- Header: eyebrow, port name, purse ------------------------------
        let eyebrow = if port.is_shipyard {
            "SHIPYARD · PORT OF CALL"
        } else {
            "PORT OF CALL"
        };
        draw_text(eyebrow, x0 + pad, y0 + 34.0, 18.0, dim_ink());
        draw_text(&port.name, x0 + pad, y0 + 62.0, 32.0, ink());

        // Purse, right-aligned in the header.
        let gold = format!("Gold {}", gs.gold);
        let hold = format!("Hold {}/{}", gs.hold_used(), gs.hold_capacity);
        right_text(&gold, x0 + pw - pad, y0 + 40.0, 24);
        right_text(&hold, x0 + pw - pad, y0 + 64.0, 20);

        let bar_y = y0 + 86.0;
        draw_line(x0 + pad, bar_y, x0 + pw - pad, bar_y, 1.0, dim_ink());

        // --- Tab bar --------------------------------------------------------
        let yard_label = if port.is_shipyard { "Shipyard" } else { "Drydock" };
        let on_bar = self.focus == Focus::TabBar;
        let tab_y = bar_y + 12.0;
        let mut tx = x0 + pad;
        tx = self.tab_button("Market", Tab::Market, tx, tab_y, on_bar);
        tx = self.tab_button("Contracts", Tab::Contracts, tx, tab_y, on_bar);
        tx = self.tab_button(yard_label, Tab::Yard, tx, tab_y, on_bar);
        let _ = self.tab_button("Racing", Tab::Race, tx, tab_y, on_bar);

        // --- Body: chart on the left, the active board on the right ----------
        let body_top = tab_y + 44.0;
        let chart_size = (ph - (body_top - y0) - pad).min(pw * 0.34);
        let chart = Rect::new(x0 + pad, body_top, chart_size, chart_size);
        let cpal = MinimapPalette::parchment();
        // Mark every accepted contract's destination and the race mark (booked, or
        // the one being eyed on the Racing tab), and draw a dashed route from this
        // port out to the highlighted contract's or race's other port.
        let mut marks: Vec<i32> = gs.active_missions.iter().map(|m| m.target_id).collect();
        if let Some(r) = gs.race {
            marks.push(r.target_id);
        } else if self.tab == Tab::Race {
            if let Some(id) = self.race_chosen(gs, world) {
                marks.push(id);
            }
        }
        let route = self.route_line(gs, world);
        minimap::render(world, kin, wind, chart, &cpal, &marks, route, &[]);
        // Name the local waters under the chart.
        let waters = &world.cluster_at(kin.pos).name;
        let cd = measure_text(waters, None, 18, 1.0);
        draw_text(
            waters,
            chart.x + (chart_size - cd.width) / 2.0,
            chart.y + chart_size + 22.0,
            18.0,
            ink(),
        );

        let board_x = chart.x + chart_size + 28.0;
        let board_w = x0 + pw - pad - board_x;
        match self.tab {
            Tab::Market => self.render_market(gs, market, board_x, body_top, board_w),
            Tab::Contracts => self.render_contracts(gs, world, board_x, body_top, board_w),
            Tab::Yard => self.render_yard(gs, world, board_x, body_top, board_w),
            Tab::Race => self.render_race(gs, world, board_x, body_top, board_w),
        }

        // Footer hint.
        draw_text(
            "Arrows move · Tab switches board · Enter trades · Esc sets sail",
            x0 + pad,
            y0 + ph - 16.0,
            18.0,
            dim_ink(),
        );
    }

    /// Draw a tab button; returns the x where the next one should start.
    fn tab_button(&self, label: &str, tab: Tab, x: f32, y: f32, on_bar: bool) -> f32 {
        let dims = measure_text(label, None, 22, 1.0);
        let bw = dims.width + 28.0;
        let bh = 30.0;
        let active = self.tab == tab;
        if active {
            draw_rectangle(x, y, bw, bh, Color::new(0.31, 0.19, 0.09, 1.0));
            // A lit ring while the cursor rests on the bar.
            if on_bar {
                draw_rectangle_lines(x, y, bw, bh, 2.5, Color::new(0.85, 0.66, 0.30, 1.0));
            }
        } else {
            draw_rectangle_lines(x, y, bw, bh, 1.5, parchment_edge());
        }
        let c = if active { parchment() } else { ink() };
        draw_text(label, x + 14.0, y + 21.0, 22.0, c);
        x + bw + 12.0
    }

    fn render_market(&self, gs: &GameState, market: &Market, x: f32, y: f32, w: f32) {
        let fs = 20;
        // Column anchors within the board.
        let name_x = x;
        let price_r = x + w * 0.42; // right edge of the price column
        let held_r = x + w * 0.56; // right edge of the hold column
        let actions_x = x + w * 0.60;

        draw_text("Commodity", name_x, y, fs as f32, dim_ink());
        right_text("Price", price_r, y, fs);
        right_text("Hold", held_r, y, fs);
        draw_text("Trade", actions_x, y, fs as f32, dim_ink());
        draw_line(x, y + 8.0, x + w, y + 8.0, 1.0, dim_ink());

        const ACTIONS: [&str; 4] = ["Buy", "Fill", "Dump", "Sell"];
        let row_h = 34.0;
        let mut ry = y + 34.0;
        for (i, good) in Good::ALL.iter().enumerate() {
            let active_row = self.focus == Focus::Good(i);
            if active_row {
                draw_rectangle(x - 6.0, ry - 18.0, w + 12.0, row_h - 4.0, row_highlight());
            }
            draw_text(good.label(), name_x, ry, fs as f32, ink());
            right_text(&market.price(*good).to_string(), price_r, ry, fs);
            right_text(&gs.quantity_of(*good).to_string(), held_r, ry, fs);

            // Four action chips.
            let chip_w = (x + w - actions_x) / 4.0;
            for (c, label) in ACTIONS.iter().enumerate() {
                let cx = actions_x + c as f32 * chip_w;
                let focused = active_row && self.column == c;
                button(cx + 2.0, ry - 17.0, chip_w - 4.0, 24.0, label, focused);
            }
            ry += row_h;
        }
    }

    fn render_yard(&self, gs: &GameState, world: &World, x: f32, y: f32, w: f32) {
        let fs = 20;
        let label_x = x;
        let detail_r = x + w * 0.66;
        let cost_r = x + w * 0.80;
        let action_x = x + w * 0.82;

        draw_text("Drydock & Shipyard", label_x, y, fs as f32, dim_ink());
        draw_line(x, y + 8.0, x + w, y + 8.0, 1.0, dim_ink());

        let row_h = 40.0;
        let mut ry = y + 38.0;

        // A right-aligned action chip for a yard row (styling kept simple: a
        // disabled row just no-ops when activated).
        let chip = |ry: f32, focused: bool, label: &str| {
            button(action_x, ry - 18.0, x + w - action_x, 26.0, label, focused);
        };

        // Drydock: mend the hull.
        {
            let active = self.focus == Focus::Repair;
            if active {
                draw_rectangle(x - 6.0, ry - 20.0, w + 12.0, row_h - 6.0, row_highlight());
            }
            let dmg = hull::damage(gs);
            draw_text("Hull · Mend", label_x, ry, fs as f32, ink());
            let cond = format!(
                "{} / {} ({}%)",
                gs.hull,
                gs.max_hull(),
                (hull::fraction(gs) * 100.0).round() as i32
            );
            right_text(&cond, detail_r, ry, fs);
            let cost = if dmg <= 0 {
                "—".to_string()
            } else {
                format!("{}", hull::repair_cost(gs))
            };
            right_text(&cost, cost_r, ry, fs);
            chip(ry, active, if dmg <= 0 { "Sound" } else { "Repair" });
            ry += row_h;
        }

        // Shipyard: sail & cargo upgrades.
        if self.is_shipyard(world) {
            for kind in [UpgradeKind::Sail, UpgradeKind::Cargo] {
                let active = self.focus == Focus::Upgrade(kind);
                if active {
                    draw_rectangle(x - 6.0, ry - 20.0, w + 12.0, row_h - 6.0, row_highlight());
                }
                let name = format!("{} · Lv {}", kind.label(), upgrades::level_of(kind, gs));
                draw_text(&name, label_x, ry, fs as f32, ink());
                right_text(&upgrades::effect(kind, gs), detail_r, ry, 16);
                let (cost, label) = match upgrades::next_cost(kind, gs) {
                    None => ("MAX".to_string(), "Maxed"),
                    Some(c) => (c.to_string(), "Fit"),
                };
                right_text(&cost, cost_r, ry, fs);
                chip(ry, active, label);
                ry += row_h;
            }
        } else {
            draw_text(
                "No shipyard here — find a shipyard port to outfit.",
                label_x,
                ry + 6.0,
                16.0,
                dim_ink(),
            );
        }
    }

    /// The Racing board: wager on beating a computer-helmed rival to another port.
    /// With no race booked it shows the day's rival ports (each with its stake) and
    /// a challenge button; with one booked it shows the armed race and a withdraw.
    fn render_race(&self, gs: &GameState, world: &World, x: f32, y: f32, w: f32) {
        let origin = &world.islands[self.island_id as usize];
        draw_text("Harbour Race · Wager", x, y, 20.0, ink());
        let mut ry = y + 30.0;

        // Label (dim, left) + value (right-aligned) line within the board.
        let line = |label: &str, value: &str, ry: f32| {
            draw_text(label, x, ry, 18.0, dim_ink());
            right_text(value, x + w, ry, 18);
        };

        if let Some(race) = gs.race {
            let target = &world.islands[race.target_id as usize];
            draw_text("Race booked", x, ry, 22.0, ink());
            ry += 32.0;
            line("Race to", &target.name, ry);
            ry += 26.0;
            line("Stake", &race.stake.to_string(), ry);
            ry += 26.0;
            line("On winning", &(race.stake * 2).to_string(), ry);
            ry += 38.0;
            draw_text(
                "Set sail and the rival draws up alongside.",
                x,
                ry,
                16.0,
                dim_ink(),
            );
            ry += 20.0;
            draw_text(
                "Heave to, then raise sail to start level.",
                x,
                ry,
                16.0,
                dim_ink(),
            );
            ry += 34.0;
            let focused = self.focus == Focus::RaceWithdraw;
            button(x, ry, w.min(300.0), 28.0, "Withdraw (forfeit stake)", focused);
            return;
        }

        let offers = race::offers(gs, world);
        if offers.is_empty() {
            draw_text(
                "No rival ports in these waters to race to.",
                x,
                ry,
                18.0,
                dim_ink(),
            );
            return;
        }

        draw_text(
            "Beat a rival sloop to another port. The stake rises",
            x,
            ry,
            16.0,
            dim_ink(),
        );
        ry += 19.0;
        draw_text("with the distance of the leg.", x, ry, 16.0, dim_ink());
        ry += 28.0;

        draw_text("Race to", x, ry, 18.0, dim_ink());
        right_text("Stake", x + w, ry, 18);
        draw_line(x, ry + 8.0, x + w, ry + 8.0, 1.0, dim_ink());
        ry += 32.0;

        let chosen = self.race_chosen(gs, world);
        let row_h = 30.0;
        for p in &offers {
            let active = self.focus == Focus::RaceTarget(p.id);
            if active {
                draw_rectangle(x - 6.0, ry - 16.0, w + 12.0, row_h - 4.0, row_highlight());
            }
            let is_chosen = chosen == Some(p.id);
            let name = if is_chosen {
                format!("{}  (chosen)", p.name)
            } else {
                p.name.clone()
            };
            draw_text(&name, x, ry, 18.0, ink());
            right_text(&race::stake_between(origin, p).to_string(), x + w, ry, 18);
            ry += row_h;
        }

        ry += 12.0;
        let focused = self.focus == Focus::RaceChallenge;
        let label = match chosen {
            Some(id) => {
                let port = &world.islands[id as usize];
                format!("Challenge to {} · {}", port.name, race::stake_between(origin, port))
            }
            None => "Challenge".to_string(),
        };
        button(x, ry, w.min(420.0), 30.0, &label, focused);
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
        let fs = 18;
        let row_h = 30.0;
        let mut ry = y;

        // --- Cargo Contracts (jobs offered here) -----------------------------
        draw_text("Cargo Contracts", x, ry, 20.0, ink());
        ry += 18.0;
        draw_text("Cargo", x, ry, fs as f32, dim_ink());
        draw_text("To", x + w * 0.27, ry, fs as f32, dim_ink());
        right_text("Deposit", x + w * 0.585, ry, fs);
        right_text("Reward", x + w * 0.73, ry, fs);
        draw_line(x, ry + 8.0, x + w, ry + 8.0, 1.0, dim_ink());
        ry += 30.0;

        let offered = mission::offered_at(gs, world);
        if offered.is_empty() {
            draw_text("No contracts on the board.", x, ry, fs as f32, dim_ink());
            ry += row_h;
        } else {
            for m in &offered {
                let to = world.islands[m.target_id as usize].name.clone();
                let active = self.focus == Focus::Contract(m.id);
                self.contract_line(m, &to, true, "Accept", active, x, ry, w, row_h);
                ry += row_h;
            }
        }

        // --- Deliveries owed at this very port -------------------------------
        let deliveries = mission::deliverable_at(gs, world);
        if !deliveries.is_empty() {
            ry += 12.0;
            draw_text("Deliveries Awaiting", x, ry, 20.0, ink());
            ry += 24.0;
            for m in &deliveries {
                let from = format!("from {}", world.islands[m.origin_id as usize].name);
                let active = self.focus == Focus::Delivery(m.id);
                self.contract_line(m, &from, true, "Deliver", active, x, ry, w, row_h);
                ry += row_h;
            }
        }

        // --- Reserved cargo bound elsewhere (the hold manifest) --------------
        let reserved = mission::reserved_at(gs, world);
        if !reserved.is_empty() {
            ry += 12.0;
            draw_text("Reserved Cargo · Hold Manifest", x, ry, 20.0, ink());
            ry += 24.0;
            for m in &reserved {
                let to = format!("-> {}", world.islands[m.target_id as usize].name);
                let active = self.focus == Focus::Manifest(m.id);
                self.contract_line(m, &to, false, "Abandon", active, x, ry, w, row_h);
                ry += row_h;
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
        row_h: f32,
    ) {
        let fs = 18;
        if active {
            draw_rectangle(x - 6.0, ry - 16.0, w + 12.0, row_h - 4.0, row_highlight());
        }
        draw_text(&format!("{} {}", m.quantity, m.good.label()), x, ry, fs as f32, ink());
        draw_text(to_text, x + w * 0.27, ry, fs as f32, ink());
        if show_money {
            right_text(&m.deposit.to_string(), x + w * 0.585, ry, fs);
            right_text(&m.reward.to_string(), x + w * 0.73, ry, fs);
        }
        let action_x = x + w * 0.77;
        button(action_x, ry - 16.0, x + w - action_x, 24.0, chip, active);
    }
}

/// Render the "press Space to dock" call-to-action when a port is in range,
/// drawn in screen space over the sea. `sail_struck` gates the action text.
pub fn render_prompt(harbor: &Harbor, world: &World, sail_struck: bool, w: f32, h: f32) {
    let Some(id) = harbor.dockable else { return };
    if harbor.is_open() {
        return;
    }
    let name = &world.islands[id as usize].name;
    let msg = if sail_struck {
        format!("Press  Space  to dock at {name}")
    } else {
        format!("Strike sail (S) to enter {name}")
    };
    let fs = 26;
    let dims = measure_text(&msg, None, fs, 1.0);
    let bx = w * 0.5 - dims.width / 2.0;
    let by = h * 0.80;
    draw_rectangle(
        bx - 18.0,
        by - 28.0,
        dims.width + 36.0,
        42.0,
        Color::new(0.0, 0.0, 0.0, 0.55),
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

fn ink() -> Color {
    Color::new(79.0 / 255.0, 47.0 / 255.0, 23.0 / 255.0, 1.0)
}
fn dim_ink() -> Color {
    Color::new(79.0 / 255.0, 47.0 / 255.0, 23.0 / 255.0, 0.62)
}
fn parchment() -> Color {
    Color::new(230.0 / 255.0, 216.0 / 255.0, 176.0 / 255.0, 1.0)
}
fn parchment_edge() -> Color {
    Color::new(120.0 / 255.0, 90.0 / 255.0, 55.0 / 255.0, 0.9)
}
fn row_highlight() -> Color {
    Color::new(150.0 / 255.0, 110.0 / 255.0, 60.0 / 255.0, 0.28)
}

/// Draw `text` right-aligned so its right edge sits at `right_x`.
fn right_text(text: &str, right_x: f32, y: f32, fs: u16) {
    let dims = measure_text(text, None, fs, 1.0);
    draw_text(text, right_x - dims.width, y, fs as f32, ink());
}

/// A small action chip: filled when focused, outlined otherwise, label centred.
fn button(x: f32, y: f32, w: f32, h: f32, label: &str, focused: bool) {
    if focused {
        draw_rectangle(x, y, w, h, Color::new(0.31, 0.19, 0.09, 1.0));
    } else {
        draw_rectangle_lines(x, y, w, h, 1.5, parchment_edge());
    }
    let fs = 18;
    let dims = measure_text(label, None, fs, 1.0);
    let c = if focused { parchment() } else { ink() };
    draw_text(
        label,
        x + (w - dims.width) / 2.0,
        y + h / 2.0 + fs as f32 * 0.35,
        fs as f32,
        c,
    );
}
