//! Docking and the port overlay, ported in spirit from `client.PortView` (plus the
//! docking handshake from `client.SailingView`).
//!
//! Sail a port within its dock range with the bow pointed at it and the sails
//! struck, press **Space**, and the captain ties up: the world keeps running
//! underneath while a parchment board opens over it. The board has two tabs:
//!
//!   - **Market** — buy and sell the seven goods at this port's deterministic
//!     prices (Buy/Fill/Dump/Sell).
//!   - **Shipyard** / **Drydock** — mend the hull, and (at shipyard ports) buy
//!     sail and hold upgrades.
//!
//! The original's Contracts and Racing tabs wait on the Mission/Race ports.
//!
//! Fully keyboard-driven: the cursor sits on the tab bar or on a row. Left/Right
//! switch tabs (or, on a commodity row, choose Buy/Fill/Dump/Sell); Up/Down move
//! through rows; Enter/Space commits; Esc (or "Set Sail") hands the helm back.

use macroquad::prelude::*;

use crate::game_state::{hull, upgrades, GameState, Good, Location, Market, UpgradeKind};
use crate::geometry::{wrap_angle, Vec2};
use crate::minimap::{self, MinimapPalette};
use crate::sailing::{Kinematics, Wind};
use crate::world::{Island, World};

// How far off the bow a port may sit and still raise the docking prompt: a
// forward arc of ±60°, so a port ahead offers to dock but one abeam or astern
// does not (`SailingView.dockFacingArc`).
const DOCK_FACING_ARC: f32 = std::f32::consts::PI / 3.0;

/// The port a captain may enter right now: a port within dock range with the bow
/// pointed at it. `armed` gates the prompt so leaving port (or passing a
/// neighbour) doesn't instantly re-prompt — it re-arms once back in open water.
pub fn port_at<'w>(world: &'w World, pos: Vec2) -> Option<&'w Island> {
    world
        .islands
        .iter()
        .filter(|i| i.is_port)
        .find(|i| i.pos.distance_to(pos) <= i.dock_range())
}

/// Manages the docking handshake and owns the open overlay, if any.
pub struct Harbor {
    armed: bool,
    /// The port in range and ahead this frame, eligible to dock (id).
    pub dockable: Option<i32>,
    /// The open trading board, while docked.
    pub screen: Option<PortScreen>,
}

impl Harbor {
    pub fn new() -> Harbor {
        Harbor {
            armed: true,
            dockable: None,
            screen: None,
        }
    }

    pub fn is_open(&self) -> bool {
        self.screen.is_some()
    }

    /// Recompute the dockable port for this frame. Called while sailing.
    pub fn update_dockable(&mut self, world: &World, kin: &Kinematics) {
        // Re-arm once we've cleared open water, so leaving port doesn't re-prompt.
        if !self.armed && port_at(world, kin.pos).is_none() {
            self.armed = true;
        }
        self.dockable = if !self.armed {
            None
        } else {
            port_at(world, kin.pos)
                .filter(|port| {
                    wrap_angle(kin.pos.bearing_to(port.pos) - kin.heading_rad).abs()
                        <= DOCK_FACING_ARC
                })
                .map(|p| p.id)
        };
    }

    /// Tie up at the dockable port (Space, sails struck). Returns true if docked.
    pub fn try_dock(&mut self, gs: &mut GameState) -> bool {
        if let Some(id) = self.dockable {
            gs.location = Location::Docked(id);
            self.armed = false;
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
        // Stay disarmed until we've sailed clear, so we don't re-dock instantly.
    }
}

// --- The overlay --------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Market,
    Yard,
}

/// What the keyboard cursor rests on.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    TabBar,
    Good(usize),
    Repair,
    Upgrade(UpgradeKind),
}

pub struct PortScreen {
    island_id: i32,
    tab: Tab,
    focus: Focus,
    column: usize, // commodity action column: 0 Buy · 1 Fill · 2 Dump · 3 Sell
}

const TABS: [Tab; 2] = [Tab::Market, Tab::Yard];
const LAST_COLUMN: usize = 3;

impl PortScreen {
    fn new(island_id: i32) -> PortScreen {
        PortScreen {
            island_id,
            tab: Tab::Market,
            focus: Focus::TabBar,
            column: 0,
        }
    }

    fn is_shipyard(&self, world: &World) -> bool {
        world.islands[self.island_id as usize].is_shipyard
    }

    /// The navigable rows of the active tab, top to bottom (the tab bar is its
    /// own focus, above these).
    fn rows_of(&self, world: &World, tab: Tab) -> Vec<Focus> {
        match tab {
            Tab::Market => (0..Good::ALL.len()).map(Focus::Good).collect(),
            Tab::Yard => {
                let mut v = vec![Focus::Repair];
                if self.is_shipyard(world) {
                    v.push(Focus::Upgrade(UpgradeKind::Sail));
                    v.push(Focus::Upgrade(UpgradeKind::Cargo));
                }
                v
            }
        }
    }

    fn enter_rows(&mut self, world: &World) {
        if let Some(&first) = self.rows_of(world, self.tab).first() {
            self.focus = first;
        }
    }

    /// Up/Down within the active tab; from the tab bar, Down enters the rows;
    /// from the topmost row, Up returns to the tab bar.
    fn move_cursor(&mut self, world: &World, delta: i32) {
        match self.focus {
            Focus::TabBar => {
                if delta > 0 {
                    self.enter_rows(world);
                }
            }
            here => {
                let list = self.rows_of(world, self.tab);
                match list.iter().position(|f| *f == here) {
                    None => self.enter_rows(world),
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
    fn slide_tab(&mut self, world: &World, delta: i32) {
        let i = TABS.iter().position(|t| *t == self.tab).unwrap_or(0);
        let n = TABS.len() as i32;
        self.tab = TABS[((i as i32 + delta).rem_euclid(n)) as usize];
        self.focus = self
            .rows_of(world, self.tab)
            .first()
            .copied()
            .unwrap_or(Focus::TabBar);
    }

    fn activate(&mut self, gs: &mut GameState, world: &World, market: &Market) {
        match self.focus {
            Focus::TabBar => self.enter_rows(world),
            Focus::Good(i) => {
                let good = Good::ALL[i];
                let _ = match self.column {
                    0 => gs.buy(market, good, 1),
                    1 => gs.fill(market, good),
                    2 => gs.dump(market, good),
                    _ => gs.sell(market, good, 1),
                };
            }
            Focus::Repair => {
                let _ = gs.repair();
            }
            Focus::Upgrade(kind) => {
                let _ = gs.buy_upgrade(world, kind);
            }
        }
    }

    /// Read keys and drive the board. Returns true when the captain sets sail.
    pub fn handle_input(&mut self, gs: &mut GameState, world: &World, market: &Market) -> bool {
        if is_key_pressed(KeyCode::Escape) {
            return true;
        }
        if is_key_pressed(KeyCode::Tab) {
            let back = is_key_down(KeyCode::LeftShift) || is_key_down(KeyCode::RightShift);
            self.cycle_tab(if back { -1 } else { 1 });
        }
        if is_key_pressed(KeyCode::Up) {
            self.move_cursor(world, -1);
        }
        if is_key_pressed(KeyCode::Down) {
            self.move_cursor(world, 1);
        }
        if is_key_pressed(KeyCode::Left) {
            match self.focus {
                Focus::TabBar => self.cycle_tab(-1),
                Focus::Good(_) if self.column > 0 => self.column -= 1,
                _ => self.slide_tab(world, -1),
            }
        }
        if is_key_pressed(KeyCode::Right) {
            match self.focus {
                Focus::TabBar => self.cycle_tab(1),
                Focus::Good(_) if self.column < LAST_COLUMN => self.column += 1,
                _ => self.slide_tab(world, 1),
            }
        }
        if is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::Space) {
            self.activate(gs, world, market);
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
        let _ = self.tab_button(yard_label, Tab::Yard, tx, tab_y, on_bar);

        // --- Body: chart on the left, the active board on the right ----------
        let body_top = tab_y + 44.0;
        let chart_size = (ph - (body_top - y0) - pad).min(pw * 0.34);
        let chart = Rect::new(x0 + pad, body_top, chart_size, chart_size);
        let cpal = MinimapPalette::parchment();
        minimap::render(world, kin, wind, chart, &cpal, &[]);
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
            Tab::Yard => self.render_yard(gs, world, board_x, body_top, board_w),
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
