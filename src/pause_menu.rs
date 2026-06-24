//! The pause menu — a parchment overlay that freezes the voyage.
//!
//! Pressing **Esc** in open water (no log, no port board up) heaves the whole game
//! to: the world stops advancing and this menu opens over the frozen scene. It has
//! two views:
//!
//!   - **Main** — *Resume* (back to the helm), *Options*, *Quit*.
//!   - **Options** — a master-volume slider (in steps of 10%) that scales the whole
//!     mix, plus *Back*.
//!
//! Keyboard-driven like the rest of the game: Up/Down move the cursor, Left/Right
//! work the slider, Enter selects, Esc backs out (Options → Main, Main → Resume).

use macroquad::prelude::*;

use crate::sound::SoundBank;

/// Which page of the menu is showing.
#[derive(Clone, Copy, PartialEq)]
enum View {
    Main,
    Options,
}

/// The main-menu rows, in cursor order.
const MAIN_ITEMS: [&str; 3] = ["Resume", "Options", "Quit"];
/// The options rows: the master-volume slider, then Back.
const OPTIONS_ROWS: usize = 2; // 0 = master volume, 1 = back
const MASTER_STEP: f32 = 0.1; // the slider moves in 10% notches

/// What the main loop should do after handling a frame of pause-menu input.
pub enum PauseAction {
    /// Nothing changed; stay paused.
    None,
    /// Close the menu and hand the helm back.
    Resume,
    /// Quit the game.
    Quit,
}

/// The pause menu's state: whether it's up, which view, and the cursor row.
pub struct PauseMenu {
    pub open: bool,
    view: View,
    cursor: usize,
}

impl PauseMenu {
    pub fn new() -> PauseMenu {
        PauseMenu {
            open: false,
            view: View::Main,
            cursor: 0,
        }
    }

    /// Raise the menu, always opening on the main view with the cursor at the top.
    pub fn open(&mut self) {
        self.open = true;
        self.view = View::Main;
        self.cursor = 0;
    }

    /// Handle one frame of input while the menu is up. Returns the action the main
    /// loop should take. The master-volume slider writes straight to `sounds`.
    pub fn handle_input(&mut self, sounds: &mut SoundBank) -> PauseAction {
        match self.view {
            View::Main => self.input_main(),
            View::Options => self.input_options(sounds),
        }
    }

    fn input_main(&mut self) -> PauseAction {
        if is_key_pressed(KeyCode::Down) {
            self.cursor = (self.cursor + 1) % MAIN_ITEMS.len();
        }
        if is_key_pressed(KeyCode::Up) {
            self.cursor = (self.cursor + MAIN_ITEMS.len() - 1) % MAIN_ITEMS.len();
        }
        // Esc resumes — the same gesture that opened the menu closes it.
        if is_key_pressed(KeyCode::Escape) {
            return PauseAction::Resume;
        }
        if is_key_pressed(KeyCode::Enter) {
            match self.cursor {
                0 => return PauseAction::Resume,
                1 => {
                    self.view = View::Options;
                    self.cursor = 0;
                }
                _ => return PauseAction::Quit,
            }
        }
        PauseAction::None
    }

    fn input_options(&mut self, sounds: &mut SoundBank) -> PauseAction {
        if is_key_pressed(KeyCode::Down) {
            self.cursor = (self.cursor + 1) % OPTIONS_ROWS;
        }
        if is_key_pressed(KeyCode::Up) {
            self.cursor = (self.cursor + OPTIONS_ROWS - 1) % OPTIONS_ROWS;
        }
        // The slider (row 0): Left/Right nudge the master gain a notch.
        if self.cursor == 0 {
            if is_key_pressed(KeyCode::Right) {
                sounds.set_master(snap(sounds.master() + MASTER_STEP));
            }
            if is_key_pressed(KeyCode::Left) {
                sounds.set_master(snap(sounds.master() - MASTER_STEP));
            }
        }
        // Back to the main view, whether by Esc or Enter on the Back row.
        let back = is_key_pressed(KeyCode::Escape)
            || (is_key_pressed(KeyCode::Enter) && self.cursor == 1);
        if back {
            self.view = View::Main;
            self.cursor = 1; // land back on Options
        }
        PauseAction::None
    }

    /// Draw the menu over the (frozen) scene.
    pub fn render(&self, sounds: &SoundBank, w: f32, h: f32) {
        // Dim the world behind the board so the parchment reads clearly.
        draw_rectangle(0.0, 0.0, w, h, Color::new(0.0, 0.0, 0.0, 0.55));

        let pw = 380.0_f32.min(w * 0.7);
        let ph = 300.0_f32.min(h * 0.7);
        let x0 = (w - pw) * 0.5;
        let y0 = (h - ph) * 0.5;
        draw_rectangle(x0, y0, pw, ph, parchment());
        draw_rectangle_lines(x0, y0, pw, ph, 3.0, parchment_edge());

        let pad = 28.0;
        let cx = x0 + pad;
        match self.view {
            View::Main => self.render_main(cx, x0, y0, pw),
            View::Options => self.render_options(sounds, cx, x0, y0, pw, ph, pad),
        }
    }

    fn render_main(&self, cx: f32, x0: f32, y0: f32, pw: f32) {
        draw_text("Paused", cx, y0 + 50.0, 36.0, ink());
        draw_text(
            "The voyage lies hove to.",
            cx,
            y0 + 78.0,
            18.0,
            dim_ink(),
        );

        let mut ry = y0 + 132.0;
        let row_h = 44.0;
        for (i, label) in MAIN_ITEMS.iter().enumerate() {
            if i == self.cursor {
                draw_rectangle(x0 + 12.0, ry - 26.0, pw - 24.0, row_h - 6.0, row_highlight());
            }
            draw_text(label, cx, ry, 28.0, ink());
            ry += row_h;
        }

        draw_text(
            "↑/↓ move · Enter select · Esc resume",
            cx,
            y0 + 132.0 + row_h * MAIN_ITEMS.len() as f32 + 8.0,
            16.0,
            dim_ink(),
        );
    }

    fn render_options(
        &self,
        sounds: &SoundBank,
        cx: f32,
        x0: f32,
        y0: f32,
        pw: f32,
        ph: f32,
        pad: f32,
    ) {
        draw_text("Options", cx, y0 + 50.0, 36.0, ink());

        // --- Master volume slider (row 0) ---
        let row_y = y0 + 120.0;
        if self.cursor == 0 {
            draw_rectangle(x0 + 12.0, row_y - 26.0, pw - 24.0, 70.0, row_highlight());
        }
        let master = sounds.master();
        draw_text("Master Volume", cx, row_y, 24.0, ink());
        draw_text(
            &format!("{}%", (master * 100.0).round() as i32),
            x0 + pw - pad - 56.0,
            row_y,
            24.0,
            ink(),
        );

        // The track and its filled portion, with a knob at the current level.
        let track_x = cx;
        let track_y = row_y + 22.0;
        let track_w = pw - 2.0 * pad;
        let track_h = 8.0;
        draw_rectangle(track_x, track_y, track_w, track_h, parchment_edge());
        draw_rectangle(track_x, track_y, track_w * master, track_h, ink());
        let knob_x = track_x + track_w * master;
        draw_circle(knob_x, track_y + track_h * 0.5, 8.0, ink());
        if self.cursor == 0 {
            draw_text("◄ / ►", track_x, track_y + 34.0, 16.0, dim_ink());
        }

        // --- Back (row 1) ---
        let back_y = y0 + ph - 64.0;
        if self.cursor == 1 {
            draw_rectangle(x0 + 12.0, back_y - 26.0, pw - 24.0, 38.0, row_highlight());
        }
        draw_text("Back", cx, back_y, 28.0, ink());

        draw_text(
            "↑/↓ move · ◄/► adjust · Esc back",
            cx,
            y0 + ph - 20.0,
            16.0,
            dim_ink(),
        );
    }
}

/// Snap a master-gain value to the nearest 10% notch and clamp to [0, 1], so the
/// slider lands cleanly on 0/10/…/100% rather than drifting off float error.
fn snap(v: f32) -> f32 {
    ((v / MASTER_STEP).round() * MASTER_STEP).clamp(0.0, 1.0)
}

// Parchment palette, matching the port board and captain's log.
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
