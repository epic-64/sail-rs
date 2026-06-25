//! The pause menu — a parchment overlay that freezes the voyage.
//!
//! Pressing **Esc** in open water (no log, no port board up) heaves the whole game
//! to: the world stops advancing and this menu opens over the frozen scene. It has
//! two views:
//!
//!   - **Main** — *Resume* (back to the helm), *Options*, *Quit*.
//!   - **Options** — a master-volume slider (in steps of 10%) that scales the whole
//!     mix, a bloom toggle, an MSAA 4× toggle, a fullscreen toggle, plus *Back*. The
//!     bloom and MSAA rows are graphics settings that take effect immediately; both
//!     rely on render-to-texture / a WebGL2 resolve that the web build can't grant,
//!     so on the web they show "Not supported" and can't be toggled.
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
/// The options rows, in cursor order:
/// 0 = master volume, 1 = bloom, 2 = MSAA 4×, 3 = fullscreen, 4 = back.
const OPTIONS_ROWS: usize = 5;
const ROW_MASTER: usize = 0;
const ROW_BLOOM: usize = 1;
const ROW_MSAA: usize = 2;
const ROW_FULLSCREEN: usize = 3;
const ROW_BACK: usize = 4;
const MASTER_STEP: f32 = 0.1; // the slider moves in 10% notches

/// Whether the graphics features (bloom, MSAA) can run in this build. They need
/// render-to-texture / a WebGL2 resolve the web build deliberately avoids (see
/// `window_conf`/`bloom` in `main.rs`), so they're disabled on the web.
const GRAPHICS_SUPPORTED: bool = !cfg!(target_arch = "wasm32");

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
    /// Our own record of the window state — macroquad offers no getter, so we track
    /// it here and toggle `set_fullscreen` against it.
    fullscreen: bool,
    /// Post-process bloom over the scene. Read each frame by the render loop. Off on
    /// the web (the build has no render-to-texture there).
    bloom: bool,
    /// 4× MSAA on the offscreen scene. Read each frame by the render loop. Off on the
    /// web (the resolve needs WebGL2, which the build avoids).
    msaa: bool,
}

impl PauseMenu {
    pub fn new() -> PauseMenu {
        PauseMenu {
            open: false,
            view: View::Main,
            cursor: 0,
            fullscreen: true, // matches the window launching full-screen (see window_conf)
            // Both default on where supported (matching the previous always-on native
            // bloom + 4× MSAA), and forced off on the web.
            bloom: GRAPHICS_SUPPORTED,
            msaa: GRAPHICS_SUPPORTED,
        }
    }

    /// Whether post-process bloom is enabled (always false on the web).
    pub fn bloom(&self) -> bool {
        self.bloom
    }

    /// Whether 4× MSAA on the scene is enabled (always false on the web).
    pub fn msaa(&self) -> bool {
        self.msaa
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
        // A row's toggle is worked with Enter or Left/Right.
        let toggled = is_key_pressed(KeyCode::Enter)
            || is_key_pressed(KeyCode::Left)
            || is_key_pressed(KeyCode::Right);
        // The slider (master volume): Left/Right nudge the master gain a notch.
        if self.cursor == ROW_MASTER {
            if is_key_pressed(KeyCode::Right) {
                sounds.set_master(snap(sounds.master() + MASTER_STEP));
            }
            if is_key_pressed(KeyCode::Left) {
                sounds.set_master(snap(sounds.master() - MASTER_STEP));
            }
        }
        // The bloom toggle — only togglable where graphics RTT is supported (not web).
        if self.cursor == ROW_BLOOM && toggled && GRAPHICS_SUPPORTED {
            self.bloom = !self.bloom;
        }
        // The MSAA 4× toggle — likewise native-only.
        if self.cursor == ROW_MSAA && toggled && GRAPHICS_SUPPORTED {
            self.msaa = !self.msaa;
        }
        // The fullscreen toggle.
        if self.cursor == ROW_FULLSCREEN && toggled {
            self.fullscreen = !self.fullscreen;
            set_fullscreen(self.fullscreen);
        }
        // Back to the main view, whether by Esc or Enter on the Back row.
        let back = is_key_pressed(KeyCode::Escape)
            || (is_key_pressed(KeyCode::Enter) && self.cursor == ROW_BACK);
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

        let pw = 420.0_f32.min(w * 0.82);
        // Tall enough for the Options view's five rows; the Main view just has more
        // breathing room.
        let ph = 440.0_f32.min(h * 0.85);
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
        let row_y = y0 + 104.0;
        if self.cursor == ROW_MASTER {
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
        if self.cursor == ROW_MASTER {
            draw_text("◄ / ►", track_x, track_y + 34.0, 16.0, dim_ink());
        }

        // --- Bloom toggle (row 1) ---
        // On the web these graphics rows are inert: show why instead of On/Off.
        let bloom_y = row_y + 80.0;
        if GRAPHICS_SUPPORTED {
            self.toggle_row(ROW_BLOOM, "Bloom", on_off(self.bloom), cx, x0, bloom_y, pw, pad);
        } else {
            self.disabled_row(ROW_BLOOM, "Bloom", cx, x0, bloom_y, pw, pad);
        }

        // --- MSAA 4× toggle (row 2) ---
        let msaa_y = bloom_y + 40.0;
        if GRAPHICS_SUPPORTED {
            self.toggle_row(ROW_MSAA, "MSAA 4×", on_off(self.msaa), cx, x0, msaa_y, pw, pad);
        } else {
            self.disabled_row(ROW_MSAA, "MSAA 4×", cx, x0, msaa_y, pw, pad);
        }

        // --- Fullscreen toggle (row 3) ---
        let fs_y = msaa_y + 40.0;
        self.toggle_row(ROW_FULLSCREEN, "Fullscreen", on_off(self.fullscreen), cx, x0, fs_y, pw, pad);

        // --- Back (row 4) ---
        let back_y = y0 + ph - 56.0;
        if self.cursor == ROW_BACK {
            draw_rectangle(x0 + 12.0, back_y - 26.0, pw - 24.0, 38.0, row_highlight());
        }
        draw_text("Back", cx, back_y, 28.0, ink());

        draw_text(
            "↑/↓ move · ◄/► or Enter adjust · Esc back",
            cx,
            y0 + ph - 20.0,
            16.0,
            dim_ink(),
        );
    }

    /// Draw a label/value toggle row at `y`, highlighting it if the cursor is on it
    /// and right-aligning `value` to the panel's inner edge.
    #[allow(clippy::too_many_arguments)]
    fn toggle_row(&self, row: usize, label: &str, value: &str, cx: f32, x0: f32, y: f32, pw: f32, pad: f32) {
        if self.cursor == row {
            draw_rectangle(x0 + 12.0, y - 26.0, pw - 24.0, 38.0, row_highlight());
        }
        draw_text(label, cx, y, 24.0, ink());
        let vw = measure_text(value, None, 24, 1.0).width;
        draw_text(value, x0 + pw - pad - vw, y, 24.0, ink());
    }

    /// Like `toggle_row`, but for a row that can't be changed in this build (web): the
    /// value reads "Not supported" in dim ink so it's clearly inert.
    fn disabled_row(&self, row: usize, label: &str, cx: f32, x0: f32, y: f32, pw: f32, pad: f32) {
        if self.cursor == row {
            draw_rectangle(x0 + 12.0, y - 26.0, pw - 24.0, 38.0, row_highlight());
        }
        draw_text(label, cx, y, 24.0, dim_ink());
        let value = "Not supported";
        let vw = measure_text(value, None, 18, 1.0).width;
        draw_text(value, x0 + pw - pad - vw, y, 18.0, dim_ink());
    }
}

/// "On"/"Off" for a toggle value.
fn on_off(v: bool) -> &'static str {
    if v {
        "On"
    } else {
        "Off"
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
