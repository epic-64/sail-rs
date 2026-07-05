//! Going ashore on an uninhabited isle, and the treasure-dig overlay it opens.
//!
//! The rules live in [`crate::dig_site`]; this module is the shore handshake
//! (sail up to a portless isle, bow on, sails furled, Space) plus the parchment
//! board the captain digs on. It mirrors [`crate::port_view`]'s shapes: a
//! [`Shore`] that owns the open [`DigScreen`], immediate-mode retained hitboxes
//! recorded as the grid draws and tapped next frame, and one set of verbs the
//! keyboard, the on-screen nav cluster and direct taps all feed.

use std::cell::RefCell;

use macroquad::prelude::*;

use crate::dig_site::{Buried, DigResult, DigSite, GRID, MOVES, TILES};
use crate::font;
use crate::game_state::GameState;
use crate::geometry::{wrap_angle, Vec2};
use crate::sailing::Kinematics;
use crate::pad::Pad;
use crate::sound::SoundBank;
use crate::touch::TouchState;
use crate::ui::{dim_ink, fs_body, fs_heading, fs_small, fs_title, ink, line_h, parchment, parchment_edge, px};
use crate::world::{Island, World};

// The forward arc within which a shore offers to be landed on, matching the
// docking arc so the two prompts feel the same (`port_view::DOCK_FACING_ARC`).
const LAND_FACING_ARC: f32 = std::f32::consts::PI / 3.0;

/// The uninhabited isle within landing range and off the bow, if any. Ports are
/// excluded: those are for [`crate::port_view`].
pub fn shore_at(world: &World, pos: Vec2) -> Option<&Island> {
    world
        .islands
        .iter()
        .filter(|i| !i.is_port)
        .find(|i| i.pos.distance_to(pos) <= i.dock_range())
}

/// Manages the shore handshake and owns the open dig board, if any. The parallel
/// to [`crate::port_view::Harbor`] for portless isles.
pub struct Shore {
    /// The isle in range and ahead this frame, eligible to land on (id).
    pub landable: Option<i32>,
    /// The open dig board, while ashore.
    pub screen: Option<DigScreen>,
}

impl Shore {
    pub fn new() -> Shore {
        Shore {
            landable: None,
            screen: None,
        }
    }

    pub fn is_open(&self) -> bool {
        self.screen.is_some()
    }

    /// Recompute the landable isle for this frame. `blocked` suppresses the offer
    /// (a port is dockable here, or this isle's field is already worked today), so
    /// the shore prompt never fights the docking prompt.
    pub fn update_landable(&mut self, world: &World, kin: &Kinematics, blocked: bool) {
        self.landable = if blocked {
            None
        } else {
            shore_at(world, kin.pos)
                .filter(|isle| {
                    wrap_angle(kin.pos.bearing_to(isle.pos) - kin.heading_rad).abs() <= LAND_FACING_ARC
                })
                .map(|i| i.id)
        };
    }

    /// Go ashore on the landable isle and lay out today's field. Returns the isle
    /// id if the captain landed, so the caller can mark it worked for the day.
    pub fn try_land(&mut self, world: &World, day: u32) -> Option<i32> {
        let id = self.landable?;
        let site = DigSite::generate(world.seed, id, day);
        self.landable = None;
        self.screen = Some(DigScreen::new(id, site));
        Some(id)
    }

    /// Put back to sea: close the board.
    pub fn cast_off(&mut self) {
        self.screen = None;
    }
}

/// The dig board over the sea: a grid of buried tiles, a cursor, and this
/// frame's tap regions.
pub struct DigScreen {
    island_id: i32,
    site: DigSite,
    /// The keyboard/d-pad cursor, a tile index in `0..TILES`.
    cursor: usize,
    /// Tile hit regions from the last `render`, tapped in `handle_input`.
    hits: RefCell<Vec<(Rect, usize)>>,
    /// The most recent dig's result and when it happened (`get_time`), for a
    /// short "found" banner under the grid.
    banner: Option<(DigResult, f64)>,
}

// How long a dig's result banner lingers, seconds.
const BANNER_DUR: f64 = 1.6;

impl DigScreen {
    fn new(island_id: i32, site: DigSite) -> DigScreen {
        DigScreen {
            island_id,
            site,
            cursor: 0,
            hits: RefCell::new(Vec::new()),
            banner: None,
        }
    }

    /// Dig the tile at `i`, bank any gold onto the purse, and cue a sound.
    fn dig(&mut self, i: usize, gs: &mut GameState, sounds: &SoundBank) {
        let result = self.site.dig(i);
        match result {
            DigResult::Spent => {
                sounds.invalid();
                return; // no banner for a wasted tap
            }
            DigResult::Dirt => {}
            DigResult::Coin(g) => {
                gs.gold += g;
                sounds.trade_one();
            }
            DigResult::ChestPiece => sounds.trade_one(),
            DigResult::ChestClaimed(g) => {
                gs.gold += g;
                sounds.salvage();
            }
        }
        self.banner = Some((result, get_time()));
    }

    /// Read input and drive the board. Returns true when the captain puts to sea.
    pub fn handle_input(&mut self, gs: &mut GameState, sounds: &SoundBank, touch: &TouchState, pad: &Pad) -> bool {
        let n = crate::touch_ui::nav_cluster(screen_width(), screen_height(), true);

        if is_key_pressed(KeyCode::Escape) || pad.back() || touch.tapped_in(n.back) {
            return true;
        }

        // Move the cursor over the grid: up/down a row, left/right a column,
        // each clamped to the field edges.
        let (mut row, mut col) = (self.cursor / GRID, self.cursor % GRID);
        if is_key_pressed(KeyCode::Up) || pad.up() || touch.tapped_in(n.up) {
            row = row.saturating_sub(1);
        }
        if is_key_pressed(KeyCode::Down) || pad.down() || touch.tapped_in(n.down) {
            row = (row + 1).min(GRID - 1);
        }
        if is_key_pressed(KeyCode::Left) || pad.left() || touch.tapped_in(n.left) {
            col = col.saturating_sub(1);
        }
        if is_key_pressed(KeyCode::Right) || pad.right() || touch.tapped_in(n.right) {
            col = (col + 1).min(GRID - 1);
        }
        self.cursor = row * GRID + col;

        // Dig the cursor tile: Enter/Space or the cluster's ✓.
        if is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::Space) || pad.confirm() || touch.tapped_in(n.confirm) {
            self.dig(self.cursor, gs, sounds);
        }

        // Direct tap on a tile digs it (unless the tap already worked a nav-cluster
        // button, so a cluster press over the grid doesn't also dig beneath it).
        let cluster_used = touch.tapped_in(n.up)
            || touch.tapped_in(n.down)
            || touch.tapped_in(n.left)
            || touch.tapped_in(n.right)
            || touch.tapped_in(n.confirm);
        if !cluster_used {
            let tapped = self
                .hits
                .borrow()
                .iter()
                .find(|(rect, _)| touch.tapped_in(*rect))
                .map(|(_, i)| *i);
            if let Some(i) = tapped {
                self.cursor = i;
                self.dig(i, gs, sounds);
            }
        }
        false
    }

    pub fn render(&self, world: &World, w: f32, h: f32) {
        let isle = &world.islands[self.island_id as usize];

        self.hits.borrow_mut().clear();

        // Dim the sea behind the board.
        draw_rectangle(0.0, 0.0, w, h, Color::new(0.0, 0.0, 0.0, 0.5));

        // The card: sized to hold a header, the square field, and a footer, and
        // capped so a big screen gets a big board (all lengths via `px`).
        let pad = px(24.0);
        let pw = (w * 0.9).min(px(520.0));
        let ph = (h * 0.92).min(px(600.0));
        let x0 = (w - pw) / 2.0;
        let y0 = (h - ph) / 2.0;
        draw_rectangle(x0, y0, pw, ph, parchment());
        draw_rectangle_lines(x0, y0, pw, ph, px(3.0), parchment_edge());

        let left = x0 + pad;
        let right = x0 + pw - pad;
        let inner_w = pw - 2.0 * pad;

        // --- Header: eyebrow, isle name, tally --------------------------------
        let eyebrow_y = y0 + pad + fs_small() as f32;
        let name_y = eyebrow_y + line_h(fs_title());
        font::heading(|| {
            draw_text("UNCHARTED ISLE", left, eyebrow_y, fs_small() as f32, dim_ink());
            draw_text(&isle.name, left, name_y, fs_title() as f32, ink());
        });
        let digs = format!("Digs left {}/{}", self.site.moves_left, MOVES);
        let gold = format!("Found {} gold", self.site.gold_found);
        let digs_y = y0 + pad + fs_heading() as f32;
        let gold_y = digs_y + line_h(fs_small());
        right_text(&digs, right, digs_y, fs_heading());
        right_text(&gold, right, gold_y, fs_small());

        let rule_y = name_y + px(12.0);
        draw_line(left, rule_y, left + inner_w, rule_y, px(1.0), parchment_edge());

        // --- The field: a centred square grid ---------------------------------
        let footer_h = line_h(fs_small()) * 2.0 + pad;
        let grid_top = rule_y + px(16.0);
        let avail_h = (y0 + ph) - grid_top - footer_h;
        let board = avail_h.min(inner_w);
        let gx = left + (inner_w - board) / 2.0;
        let cell = board / GRID as f32;
        let inset = cell * 0.10; // gap between neighbouring tiles

        for i in 0..TILES {
            let (r, c) = (i / GRID, i % GRID);
            let tx = gx + c as f32 * cell;
            let ty = grid_top + r as f32 * cell;
            let rect = Rect::new(tx, ty, cell, cell);
            self.hits.borrow_mut().push((rect, i));
            self.draw_tile(tx + inset, ty + inset, cell - 2.0 * inset, i);
            if i == self.cursor && !self.site.finished() {
                // The cursor: a bold ink outline over the tile.
                draw_rectangle_lines(tx + inset * 0.5, ty + inset * 0.5, cell - inset, cell - inset, px(3.0), ink());
            }
        }

        // --- Footer: a result banner, then the controls hint ------------------
        let footer_y = y0 + ph - pad;
        if let Some((result, at)) = self.banner {
            if get_time() - at < BANNER_DUR {
                let (msg, col) = banner_text(result);
                let d = measure_text(msg, None, fs_body(), 1.0);
                draw_text(msg, x0 + (pw - d.width) / 2.0, footer_y - line_h(fs_small()), fs_body() as f32, col);
            }
        }
        let hint = if self.site.finished() {
            crate::device::hint("The field is spent · Esc puts to sea", "The field is spent · B puts to sea")
        } else {
            crate::device::hint("Arrows move · Enter digs · Esc puts to sea", "D-pad move · A digs · B puts to sea")
        };
        draw_text(hint, left, footer_y, fs_small() as f32, dim_ink());
    }

    /// Draw one tile's face: an unturned mound while buried, or what the dig
    /// turned up once open.
    fn draw_tile(&self, x: f32, y: f32, s: f32, i: usize) {
        if !self.site.is_open(i) {
            // Unturned earth: a filled mound with a lighter domed highlight.
            draw_rectangle(x, y, s, s, MOUND);
            draw_rectangle(x + s * 0.14, y + s * 0.14, s * 0.72, s * 0.4, MOUND_LIT);
            draw_rectangle_lines(x, y, s, s, px(1.5), parchment_edge());
            return;
        }
        // A dug tile is a recessed pit; its contents sit in the hollow.
        draw_rectangle(x, y, s, s, PIT);
        match self.site.buried_at(i) {
            Buried::Dirt => {}
            Buried::Coin(_) => {
                let cx = x + s / 2.0;
                let cy = y + s / 2.0;
                draw_circle(cx, cy, s * 0.28, COIN);
                draw_circle_lines(cx, cy, s * 0.28, px(1.5), COIN_EDGE);
                draw_circle(cx - s * 0.08, cy - s * 0.08, s * 0.07, COIN_LIT);
            }
            Buried::Chest(_) => {
                // A slab of chest planking with a metal band. Neighbouring cleared
                // chest tiles share the fill, so the chest reads as one body.
                draw_rectangle(x, y, s, s, CHEST_WOOD);
                draw_rectangle(x, y + s * 0.44, s, s * 0.16, CHEST_BAND);
                draw_rectangle_lines(x, y, s, s, px(1.5), CHEST_EDGE);
            }
        }
        draw_rectangle_lines(x, y, s, s, px(1.0), PIT_EDGE);
    }
}

/// Right-align `text` so its end sits at `right`, on the `y` baseline.
fn right_text(text: &str, right: f32, y: f32, fs: u16) {
    let d = measure_text(text, None, fs, 1.0);
    draw_text(text, right - d.width, y, fs as f32, ink());
}

/// The banner line and colour for a dig outcome (only the rewarding ones show).
fn banner_text(result: DigResult) -> (&'static str, Color) {
    match result {
        DigResult::Coin(_) => ("A coin!", COIN_EDGE),
        DigResult::ChestPiece => ("Timber... a chest is buried here.", ink()),
        DigResult::ChestClaimed(_) => ("Chest unearthed!", COIN_EDGE),
        DigResult::Dirt | DigResult::Spent => ("", ink()),
    }
}

// --- Tile palette (soil / gold / chest), keyed off the parchment inks ---------
const MOUND: Color = Color::new(161.0 / 255.0, 120.0 / 255.0, 74.0 / 255.0, 1.0);
const MOUND_LIT: Color = Color::new(184.0 / 255.0, 146.0 / 255.0, 98.0 / 255.0, 1.0);
const PIT: Color = Color::new(52.0 / 255.0, 33.0 / 255.0, 18.0 / 255.0, 1.0);
const PIT_EDGE: Color = Color::new(30.0 / 255.0, 19.0 / 255.0, 10.0 / 255.0, 1.0);
const COIN: Color = Color::new(214.0 / 255.0, 178.0 / 255.0, 74.0 / 255.0, 1.0);
const COIN_LIT: Color = Color::new(245.0 / 255.0, 224.0 / 255.0, 150.0 / 255.0, 1.0);
const COIN_EDGE: Color = Color::new(150.0 / 255.0, 110.0 / 255.0, 30.0 / 255.0, 1.0);
const CHEST_WOOD: Color = Color::new(110.0 / 255.0, 72.0 / 255.0, 38.0 / 255.0, 1.0);
const CHEST_BAND: Color = Color::new(214.0 / 255.0, 178.0 / 255.0, 74.0 / 255.0, 1.0);
const CHEST_EDGE: Color = Color::new(60.0 / 255.0, 38.0 / 255.0, 18.0 / 255.0, 1.0);

/// The shore call-to-action while sailing: shown when an isle is landable (bow
/// on, in range). Mirrors [`crate::port_view::render_prompt`] byte-for-byte in
/// shape (same two-state wording, same [`crate::ui::sea_prompt`] styling, same
/// [`crate::device::hint`]-driven keyboard/gamepad text) so a port and an
/// uninhabited isle read as one system rather than two lookalikes.
pub fn render_prompt(shore: &Shore, world: &World, sails_furled: bool, w: f32, h: f32) {
    let Some(id) = shore.landable else { return };
    if shore.is_open() {
        return;
    }
    let name = &world.islands[id as usize].name;
    let msg = if sails_furled {
        let key = crate::device::hint("Space", "X");
        format!("Press  {key}  to go ashore on {name}")
    } else {
        let key = crate::device::hint("S", "B");
        format!("Furl sail ({key}) to land on {name}")
    };
    crate::ui::sea_prompt(&msg, w, h);
}
