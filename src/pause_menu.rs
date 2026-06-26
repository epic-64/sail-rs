//! The pause menu — a parchment overlay that freezes the voyage.
//!
//! Pressing **Esc** in open water (no log, no port board up) heaves the whole game
//! to: the world stops advancing and this menu opens over the frozen scene. It has
//! two views:
//!
//!   - **Main** — *Resume* (back to the helm), *Options*, *Quit*.
//!   - **Options** — a master-volume slider (in steps of 10%) that scales the whole
//!     mix, a bloom toggle, an MSAA 4× toggle, a fullscreen toggle, a **World Seed**
//!     text field (type digits, Enter to sail a fresh chart on that seed), plus *Back*.
//!     The bloom and MSAA rows are graphics settings that take effect immediately;
//!     both rely on render-to-texture / a WebGL2 resolve that the web build can't
//!     grant, so on the web they show "Not supported" and can't be toggled.
//!
//! Keyboard-driven like the rest of the game: Up/Down move the cursor, Left/Right
//! work the slider, Enter selects, Esc backs out (Options → Main, Main → Resume).
//! On the seed field, digits edit the value, Backspace deletes, and Enter applies
//! it. A rejected seed entry (a non-digit, an over-long value, or an empty field)
//! gets the same audio buzzer + red jiggle the port board uses for an illegal trade.

use macroquad::prelude::*;

use crate::sound::SoundBank;
use crate::touch::TouchState;

/// The menu's directional verbs for this frame, OR-ing the keyboard with taps on
/// the on-screen nav cluster (see `touch_ui`). The seed field still needs the
/// keyboard (digit entry) — a soft keyboard is a later job.
struct Nav {
    up: bool,
    down: bool,
    left: bool,
    right: bool,
    confirm: bool,
    back: bool,
}

impl Nav {
    fn read(touch: &TouchState) -> Nav {
        let n = crate::touch_ui::nav_cluster(screen_width(), screen_height(), false);
        Nav {
            up: is_key_pressed(KeyCode::Up) || touch.tapped_in(n.up),
            down: is_key_pressed(KeyCode::Down) || touch.tapped_in(n.down),
            left: is_key_pressed(KeyCode::Left) || touch.tapped_in(n.left),
            right: is_key_pressed(KeyCode::Right) || touch.tapped_in(n.right),
            confirm: is_key_pressed(KeyCode::Enter) || touch.tapped_in(n.confirm),
            back: is_key_pressed(KeyCode::Escape) || touch.tapped_in(n.back),
        }
    }
}

/// Which page of the menu is showing.
#[derive(Clone, Copy, PartialEq)]
enum View {
    Main,
    Options,
}

/// The main-menu rows, in cursor order.
const MAIN_ITEMS: [&str; 3] = ["Resume", "Options", "Quit"];
/// The options rows, in cursor order:
/// 0 = master volume, 1 = bloom, 2 = MSAA 4×, 3 = fullscreen, 4 = world seed, 5 = back.
const OPTIONS_ROWS: usize = 6;
const ROW_MASTER: usize = 0;
const ROW_BLOOM: usize = 1;
const ROW_MSAA: usize = 2;
const ROW_FULLSCREEN: usize = 3;
const ROW_SEED: usize = 4;
const ROW_BACK: usize = 5;
const MASTER_STEP: f32 = 0.1; // the slider moves in 10% notches

/// World-seed field rules: digits only, 1..=`SEED_MAX_LEN` of them. The cap keeps
/// the value comfortably inside `i64` (18 nines < `i64::MAX`), so any accepted
/// string parses; the lower bound (non-empty) is enforced at apply time.
const SEED_MAX_LEN: usize = 18;

/// Flash timing for a rejected seed entry — a brief red jiggle that decays to
/// nothing, matching the port board's constraint flash (`port_view`).
const FLASH_DUR: f32 = 0.42; // seconds the jiggle lasts
const FLASH_AMP: f32 = 4.0; // peak horizontal wobble, px
const FLASH_FREQ: f32 = 7.0; // wobble oscillations per second

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
    /// The captain entered a new world seed: end this voyage and begin a fresh one
    /// on the given seed.
    NewWorld(i64),
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
    /// The world-seed field's edit buffer (digits only). Seeds the current chart;
    /// applied with Enter on the seed row to start a fresh voyage.
    seed_text: String,
    /// When a rejected seed entry last flashed (`get_time` seconds), driving the
    /// red jiggle; `None` once it has decayed or never fired.
    seed_flash: Option<f64>,
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
            // Matches the world the loop boots on (`run_game(1, …)` in main).
            seed_text: String::from("1"),
            seed_flash: None,
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
    pub fn handle_input(&mut self, sounds: &mut SoundBank, touch: &TouchState) -> PauseAction {
        let nav = Nav::read(touch);
        match self.view {
            View::Main => self.input_main(&nav),
            View::Options => self.input_options(sounds, &nav, touch),
        }
    }

    fn input_main(&mut self, nav: &Nav) -> PauseAction {
        if nav.down {
            self.cursor = (self.cursor + 1) % MAIN_ITEMS.len();
        }
        if nav.up {
            self.cursor = (self.cursor + MAIN_ITEMS.len() - 1) % MAIN_ITEMS.len();
        }
        // Esc resumes — the same gesture that opened the menu closes it.
        if nav.back {
            return PauseAction::Resume;
        }
        if nav.confirm {
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

    fn input_options(&mut self, sounds: &mut SoundBank, nav: &Nav, touch: &TouchState) -> PauseAction {
        if nav.down {
            self.cursor = (self.cursor + 1) % OPTIONS_ROWS;
        }
        if nav.up {
            self.cursor = (self.cursor + OPTIONS_ROWS - 1) % OPTIONS_ROWS;
        }
        // The seed field owns the keyboard while focused: digits edit it, Backspace
        // deletes, Enter applies, Esc backs out. Handle it on its own and return so
        // none of the toggle/slider logic below sees the keys. (Touch can move the
        // cursor here and back out, but typing a seed still needs a keyboard.)
        if self.cursor == ROW_SEED {
            return self.input_seed(sounds, nav, touch);
        }
        // A row's toggle is worked with Enter / confirm or Left/Right.
        let toggled = nav.confirm || nav.left || nav.right;
        // The slider (master volume): Left/Right nudge the master gain a notch.
        if self.cursor == ROW_MASTER {
            if nav.right {
                sounds.set_master(snap(sounds.master() + MASTER_STEP));
            }
            if nav.left {
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
        // Back to the main view, whether by Esc/back or Enter on the Back row.
        let back = nav.back || (nav.confirm && self.cursor == ROW_BACK);
        if back {
            self.view = View::Main;
            self.cursor = 1; // land back on Options
        }
        PauseAction::None
    }

    /// One frame of input while the world-seed field is focused. Digits append (up
    /// to the length cap), Backspace deletes, Enter applies, Esc backs out to Main.
    /// A character the rule won't take — a non-digit, or a digit past the cap —
    /// is rejected with the standard buzzer + red jiggle, never silently dropped.
    fn input_seed(&mut self, sounds: &mut SoundBank, nav: &Nav, _touch: &TouchState) -> PauseAction {
        if is_key_pressed(KeyCode::Backspace) {
            self.seed_text.pop();
        }
        if is_key_pressed(KeyCode::Enter) {
            return self.apply_seed(sounds);
        }
        // A touch tap on back (or Esc) leaves the field. Moving the cursor off the
        // seed row is handled by `input_options` before it dispatches here.
        if nav.back {
            self.view = View::Main;
            self.cursor = 1; // land back on Options
            return PauseAction::None;
        }
        // Drain the frame's typed characters. Control keys (Enter/Backspace/Esc,
        // handled above) arrive as control chars and are ignored here.
        while let Some(c) = get_char_pressed() {
            if c.is_ascii_digit() && self.seed_text.len() < SEED_MAX_LEN {
                self.seed_text.push(c);
            } else if !c.is_control() {
                self.reject(sounds);
            }
        }
        PauseAction::None
    }

    /// Validate and apply the seed buffer. A non-empty digit string (guaranteed to
    /// fit `i64` by the length cap) starts a fresh voyage with a confirming chime;
    /// an empty field is rejected like any illegal entry.
    fn apply_seed(&mut self, sounds: &mut SoundBank) -> PauseAction {
        match self.seed_text.parse::<i64>() {
            Ok(seed) => {
                sounds.transaction(); // the coin chime confirms the new chart
                self.open = false;
                PauseAction::NewWorld(seed)
            }
            Err(_) => {
                self.reject(sounds);
                PauseAction::None
            }
        }
    }

    /// Reject an illegal seed entry: the buzzer plus a fresh red jiggle on the field.
    fn reject(&mut self, sounds: &SoundBank) {
        sounds.invalid();
        self.seed_flash = Some(get_time());
    }

    /// The seed field's current `(dx, redness)` jiggle, or `(0, 0)` once it has
    /// decayed — same shape as the port board's constraint flash.
    fn seed_flash_state(&self) -> (f32, f32) {
        match self.seed_flash {
            Some(start) => {
                let age = (get_time() - start) as f32;
                if age >= FLASH_DUR {
                    return (0.0, 0.0);
                }
                let decay = 1.0 - age / FLASH_DUR;
                let dx = FLASH_AMP * (age * FLASH_FREQ * std::f32::consts::TAU).sin() * decay;
                (dx, decay)
            }
            None => (0.0, 0.0),
        }
    }

    /// Draw the menu over the (frozen) scene.
    pub fn render(&self, sounds: &SoundBank, w: f32, h: f32) {
        // Dim the world behind the board so the parchment reads clearly.
        draw_rectangle(0.0, 0.0, w, h, Color::new(0.0, 0.0, 0.0, 0.55));

        let pw = 420.0_f32.min(w * 0.82);
        // Tall enough for the Options view's six rows (the seed field's hint adds a
        // line); the Main view just has more breathing room.
        let ph = 488.0_f32.min(h * 0.9);
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
        crate::font::heading(|| draw_text("Paused", cx, y0 + 50.0, 36.0, ink()));
        draw_text(
            "The voyage lies hove to.",
            cx,
            y0 + 78.0,
            15.0,
            dim_ink(),
        );

        let mut ry = y0 + 132.0;
        let row_h = 44.0;
        for (i, label) in MAIN_ITEMS.iter().enumerate() {
            if i == self.cursor {
                draw_rectangle(x0 + 12.0, ry - 26.0, pw - 24.0, row_h - 6.0, row_highlight());
            }
            draw_text(label, cx, ry, 22.0, ink());
            ry += row_h;
        }

        draw_text(
            "↑/↓ move · Enter select · Esc resume",
            cx,
            y0 + 132.0 + row_h * MAIN_ITEMS.len() as f32 + 8.0,
            14.0,
            dim_ink(),
        );
    }

    /// Draw the world-seed row: label, the edit buffer right-aligned (with a caret
    /// and an edit hint when focused), jiggling red toward the alarm ink when an
    /// entry was just rejected.
    fn render_seed_row(&self, cx: f32, x0: f32, y: f32, pw: f32, pad: f32) {
        let focused = self.cursor == ROW_SEED;
        if focused {
            // A taller highlight than the other rows to take in the edit hint below.
            draw_rectangle(x0 + 12.0, y - 26.0, pw - 24.0, 58.0, row_highlight());
        }
        draw_text("World Seed", cx, y, 19.0, ink());

        // The value, with a blinking-free caret while focused so the field reads as
        // editable; jiggles red on a rejected entry.
        let value = if focused {
            format!("{}_", self.seed_text)
        } else {
            self.seed_text.clone()
        };
        let (dx, red) = self.seed_flash_state();
        let vw = measure_text(&value, None, 19, 1.0).width;
        draw_text(&value, x0 + pw - pad - vw + dx, y, 19.0, flash_ink(red));

        if focused {
            draw_text("type digits · Enter sail a new chart", cx, y + 22.0, 13.0, dim_ink());
        }
    }

    #[allow(clippy::too_many_arguments)] // layout geometry passed down from the caller
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
        crate::font::heading(|| draw_text("Options", cx, y0 + 50.0, 36.0, ink()));

        // --- Master volume slider (row 0) ---
        let row_y = y0 + 110.0;
        if self.cursor == ROW_MASTER {
            draw_rectangle(x0 + 12.0, row_y - 26.0, pw - 24.0, 70.0, row_highlight());
        }
        let master = sounds.master();
        draw_text("Master Volume", cx, row_y, 19.0, ink());
        draw_text(
            format!("{}%", (master * 100.0).round() as i32),
            x0 + pw - pad - 56.0,
            row_y,
            19.0,
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
            draw_text("◄ / ►", track_x, track_y + 34.0, 14.0, dim_ink());
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

        // --- World seed field (row 4) ---
        // Sits just above Back; its edit hint needs the extra room below.
        let seed_y = fs_y + 56.0;
        self.render_seed_row(cx, x0, seed_y, pw, pad);

        // --- Back (row 5) ---
        let back_y = y0 + ph - 56.0;
        if self.cursor == ROW_BACK {
            draw_rectangle(x0 + 12.0, back_y - 26.0, pw - 24.0, 38.0, row_highlight());
        }
        draw_text("Back", cx, back_y, 22.0, ink());

        draw_text(
            "↑/↓ move · ◄/► or Enter adjust · Esc back",
            cx,
            y0 + ph - 20.0,
            14.0,
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
        draw_text(label, cx, y, 19.0, ink());
        let vw = measure_text(value, None, 19, 1.0).width;
        draw_text(value, x0 + pw - pad - vw, y, 19.0, ink());
    }

    /// Like `toggle_row`, but for a row that can't be changed in this build (web): the
    /// value reads "Not supported" in dim ink so it's clearly inert.
    #[allow(clippy::too_many_arguments)] // row layout geometry is inherent
    fn disabled_row(&self, row: usize, label: &str, cx: f32, x0: f32, y: f32, pw: f32, pad: f32) {
        if self.cursor == row {
            draw_rectangle(x0 + 12.0, y - 26.0, pw - 24.0, 38.0, row_highlight());
        }
        draw_text(label, cx, y, 19.0, dim_ink());
        let value = "Not supported";
        let vw = measure_text(value, None, 15, 1.0).width;
        draw_text(value, x0 + pw - pad - vw, y, 15.0, dim_ink());
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
/// The alarm red a rejected seed entry flashes toward (matches `port_view`).
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
