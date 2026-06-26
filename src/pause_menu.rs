//! The pause menu — a parchment overlay that freezes the voyage.
//!
//! Pressing **Esc** in open water (no log, no port board up) heaves the whole game
//! to: the world stops advancing and this menu opens over the frozen scene. It has
//! two views:
//!
//!   - **Main** — *Resume* (back to the helm), *Options*, *Quit*.
//!   - **Options** — a master-volume slider (in steps of 10%) that scales the whole
//!     mix, a **Scenery** density slider (the performance setting: steps the island
//!     foliage from Very Low to Very High), a bloom toggle, an MSAA 4× toggle, a
//!     fullscreen toggle, a **World Seed** text field (type digits, Enter to sail a
//!     fresh chart on that seed), plus *Back*.
//!     The bloom and MSAA rows are graphics settings that take effect immediately;
//!     both rely on render-to-texture / a WebGL2 resolve that the web build can't
//!     grant, so on the web they show "Not supported" and can't be toggled.
//!
//! Keyboard-driven like the rest of the game: Up/Down move the cursor, Left/Right
//! work the slider, Enter selects, Esc backs out (Options → Main, Main → Resume).
//! On the seed field, digits edit the value, Backspace deletes, and Enter applies
//! it. A rejected seed entry (a non-digit, an over-long value, or an empty field)
//! gets the same audio buzzer + red jiggle the port board uses for an illegal trade.

use std::cell::RefCell;

use macroquad::prelude::*;

use crate::sound::SoundBank;
use crate::touch::TouchState;
// The parchment palette and the UI scale are shared design tokens; see `crate::ui`.
// Only the menu's own row highlight / alarm inks live at the foot of this file.
use crate::ui::{dim_ink, ink, parchment, parchment_edge, px};

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
        let n = crate::touch_ui::nav_cluster(screen_width(), screen_height());
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

/// A region `render` recorded as tappable, hit-tested next frame in `handle_input`
/// (immediate-mode retained hitboxes, as the port board does). Geometry lives where
/// it's drawn — no second copy of the layout.
#[derive(Clone, Copy)]
enum Tap {
    /// Focus this row and press it (Enter-equivalent) — so a click on a menu item /
    /// toggle / Back acts in one go. A no-op "press" (the volume label, the seed row)
    /// simply leaves the cursor there.
    Select(usize),
    /// A slider track (master volume or scenery density): the `usize` is the owning
    /// row, the `Rect` the thin track; set that row's value from where the click
    /// landed along it.
    Slider(usize, Rect),
}

/// The main-menu rows, in cursor order.
const MAIN_ITEMS: [&str; 3] = ["Resume", "Options", "Quit"];
/// The options rows, in cursor order:
/// 0 = master volume, 1 = scenery density, 2 = bloom, 3 = MSAA 4×, 4 = fullscreen,
/// 5 = world seed, 6 = back.
const OPTIONS_ROWS: usize = 7;
const ROW_MASTER: usize = 0;
const ROW_SCENERY: usize = 1;
const ROW_BLOOM: usize = 2;
const ROW_MSAA: usize = 3;
const ROW_FULLSCREEN: usize = 4;
const ROW_SEED: usize = 5;
const ROW_BACK: usize = 6;
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
    /// Scenery-density level (0..`isle_features::DENSITY_LEVELS`), the performance
    /// slider. Read by `main`, which rebuilds the island features when it changes and
    /// persists it as a global preference. Defaults to the tuned middle.
    feat_density: usize,
    /// The world-seed field's edit buffer (digits only). Seeds the current chart;
    /// applied with Enter on the seed row to start a fresh voyage.
    seed_text: String,
    /// When a rejected seed entry last flashed (`get_time` seconds), driving the
    /// red jiggle; `None` once it has decayed or never fired.
    seed_flash: Option<f64>,
    /// Tappable regions from the last `render`, consumed by touch in `handle_input`.
    taps: RefCell<Vec<(Rect, Tap)>>,
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
            // The tuned middle (`Medium`); `main` overrides it from the saved
            // preference at startup if one exists.
            feat_density: crate::isle_features::DENSITY_LEVELS / 2,
            // Matches the world the loop boots on (`run_game(1, …)` in main).
            seed_text: String::from("1"),
            seed_flash: None,
            taps: RefCell::new(Vec::new()),
        }
    }

    /// Record a tappable region for this frame (consumed by `handle_input`).
    fn tap(&self, rect: Rect, t: Tap) {
        self.taps.borrow_mut().push((rect, t));
    }

    /// Whether post-process bloom is enabled (always false on the web).
    pub fn bloom(&self) -> bool {
        self.bloom
    }

    /// Whether 4× MSAA on the scene is enabled (always false on the web).
    pub fn msaa(&self) -> bool {
        self.msaa
    }

    /// The current scenery-density level (the performance slider).
    pub fn feat_density(&self) -> usize {
        self.feat_density
    }

    /// Set the scenery-density level (clamped to the valid range), e.g. to apply a
    /// saved preference at startup.
    pub fn set_feat_density(&mut self, level: usize) {
        self.feat_density = level.min(crate::isle_features::DENSITY_LEVELS - 1);
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
        let mut nav = Nav::read(touch);
        // Direct taps on rows the last `render` recorded: a click on a menu item /
        // toggle focuses it and presses it (so it acts like the d-pad's ✓), and a
        // click on the volume track sets the gain. Both feed the same handlers below.
        let hits: Vec<(Rect, Tap)> = self.taps.borrow().clone();
        for (rect, t) in hits {
            match t {
                Tap::Slider(row, track) => {
                    // Hit on the wide band (`rect`), fraction from the thin track.
                    if let Some(p) = touch.tap_pos_in(rect) {
                        let f = ((p.x - track.x) / track.w).clamp(0.0, 1.0);
                        match row {
                            ROW_SCENERY => self.feat_density = density_from_frac(f),
                            _ => sounds.set_master(snap(f)),
                        }
                        self.cursor = row;
                    }
                }
                Tap::Select(row) => {
                    if touch.tapped_in(rect) {
                        self.cursor = row;
                        nav.confirm = true;
                    }
                }
            }
        }
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
        // The scenery-density slider: Left/Right step one level along the ladder.
        if self.cursor == ROW_SCENERY {
            if nav.right && self.feat_density + 1 < crate::isle_features::DENSITY_LEVELS {
                self.feat_density += 1;
            }
            if nav.left && self.feat_density > 0 {
                self.feat_density -= 1;
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
        // Fresh hit regions for this layout; the render below repopulates them as it
        // draws, and `handle_input` taps them next frame.
        self.taps.borrow_mut().clear();

        // Dim the world behind the board so the parchment reads clearly.
        draw_rectangle(0.0, 0.0, w, h, Color::new(0.0, 0.0, 0.0, 0.55));

        let pw = px(420.0).min(w * 0.82);
        // Tall enough for the Options view's seven rows (two of them sliders, plus the
        // seed field's hint line); the Main view just has more breathing room.
        let ph = px(568.0).min(h * 0.9);
        let x0 = (w - pw) * 0.5;
        let y0 = (h - ph) * 0.5;
        draw_rectangle(x0, y0, pw, ph, parchment());
        draw_rectangle_lines(x0, y0, pw, ph, px(3.0), parchment_edge());

        let pad = px(28.0);
        let cx = x0 + pad;
        match self.view {
            View::Main => self.render_main(cx, x0, y0, pw),
            View::Options => self.render_options(sounds, cx, x0, y0, pw, ph, pad),
        }
    }

    fn render_main(&self, cx: f32, x0: f32, y0: f32, pw: f32) {
        crate::font::heading(|| draw_text("Paused", cx, y0 + px(50.0), px(36.0), ink()));
        draw_text(
            "The voyage lies hove to.",
            cx,
            y0 + px(78.0),
            px(15.0),
            dim_ink(),
        );

        let mut ry = y0 + px(132.0);
        let row_h = px(44.0);
        for (i, label) in MAIN_ITEMS.iter().enumerate() {
            let rect = Rect::new(x0 + px(12.0), ry - px(26.0), pw - px(24.0), row_h - px(6.0));
            self.tap(rect, Tap::Select(i));
            if i == self.cursor {
                draw_rectangle(rect.x, rect.y, rect.w, rect.h, row_highlight());
            }
            draw_text(label, cx, ry, px(22.0), ink());
            ry += row_h;
        }

        draw_text(
            "↑/↓ move · Enter select · Esc resume",
            cx,
            y0 + px(132.0) + row_h * MAIN_ITEMS.len() as f32 + px(8.0),
            px(14.0),
            dim_ink(),
        );
    }

    /// Draw the world-seed row: label, the edit buffer right-aligned (with a caret
    /// and an edit hint when focused), jiggling red toward the alarm ink when an
    /// entry was just rejected.
    fn render_seed_row(&self, cx: f32, x0: f32, y: f32, pw: f32, pad: f32) {
        let focused = self.cursor == ROW_SEED;
        self.tap(Rect::new(x0 + px(12.0), y - px(26.0), pw - px(24.0), px(58.0)), Tap::Select(ROW_SEED));
        if focused {
            // A taller highlight than the other rows to take in the edit hint below.
            draw_rectangle(x0 + px(12.0), y - px(26.0), pw - px(24.0), px(58.0), row_highlight());
        }
        draw_text("World Seed", cx, y, px(19.0), ink());

        // The value, with a blinking-free caret while focused so the field reads as
        // editable; jiggles red on a rejected entry.
        let value = if focused {
            format!("{}_", self.seed_text)
        } else {
            self.seed_text.clone()
        };
        let (dx, red) = self.seed_flash_state();
        let vw = measure_text(&value, None, px(19.0) as u16, 1.0).width;
        draw_text(&value, x0 + pw - pad - vw + dx, y, px(19.0), flash_ink(red));

        if focused {
            draw_text("type digits · Enter sail a new chart", cx, y + px(22.0), px(13.0), dim_ink());
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
        crate::font::heading(|| draw_text("Options", cx, y0 + px(50.0), px(36.0), ink()));

        // --- Master volume slider (row 0) ---
        let row_y = y0 + px(110.0);
        let master = sounds.master();
        let master_pct = format!("{}%", (master * 100.0).round() as i32);
        self.slider_row(ROW_MASTER, "Master Volume", &master_pct, master, cx, x0, row_y, pw, pad);

        // --- Scenery density slider (row 1) ---
        // The performance setting: steps the foliage from Very Low to Very High. The
        // fraction places the knob on its notch along the five-level ladder.
        let scenery_y = row_y + px(80.0);
        let lvl = self.feat_density;
        let frac = lvl as f32 / (crate::isle_features::DENSITY_LEVELS - 1).max(1) as f32;
        self.slider_row(
            ROW_SCENERY,
            "Scenery",
            crate::isle_features::density_label(lvl),
            frac,
            cx,
            x0,
            scenery_y,
            pw,
            pad,
        );

        // --- Bloom toggle (row 2) ---
        // On the web these graphics rows are inert: show why instead of On/Off.
        let bloom_y = scenery_y + px(80.0);
        if GRAPHICS_SUPPORTED {
            self.toggle_row(ROW_BLOOM, "Bloom", on_off(self.bloom), cx, x0, bloom_y, pw, pad);
        } else {
            self.disabled_row(ROW_BLOOM, "Bloom", cx, x0, bloom_y, pw, pad);
        }

        // --- MSAA 4× toggle (row 3) ---
        let msaa_y = bloom_y + px(40.0);
        if GRAPHICS_SUPPORTED {
            self.toggle_row(ROW_MSAA, "MSAA 4×", on_off(self.msaa), cx, x0, msaa_y, pw, pad);
        } else {
            self.disabled_row(ROW_MSAA, "MSAA 4×", cx, x0, msaa_y, pw, pad);
        }

        // --- Fullscreen toggle (row 4) ---
        let fs_y = msaa_y + px(40.0);
        self.toggle_row(ROW_FULLSCREEN, "Fullscreen", on_off(self.fullscreen), cx, x0, fs_y, pw, pad);

        // --- World seed field (row 5) ---
        // Sits just above Back; its edit hint needs the extra room below.
        let seed_y = fs_y + px(56.0);
        self.render_seed_row(cx, x0, seed_y, pw, pad);

        // --- Back (row 6) ---
        let back_y = y0 + ph - px(56.0);
        self.tap(Rect::new(x0 + px(12.0), back_y - px(26.0), pw - px(24.0), px(38.0)), Tap::Select(ROW_BACK));
        if self.cursor == ROW_BACK {
            draw_rectangle(x0 + px(12.0), back_y - px(26.0), pw - px(24.0), px(38.0), row_highlight());
        }
        draw_text("Back", cx, back_y, px(22.0), ink());

        draw_text(
            "↑/↓ move · ◄/► or Enter adjust · Esc back",
            cx,
            y0 + ph - px(20.0),
            px(14.0),
            dim_ink(),
        );
    }

    /// Draw a slider row (label + right-aligned value, then a track with a knob at
    /// `frac` of the way along) at `row_y`, recording the row's focus hitbox and the
    /// track's drag band. Shared by the master-volume and scenery-density sliders.
    #[allow(clippy::too_many_arguments)]
    fn slider_row(
        &self,
        row: usize,
        label: &str,
        value: &str,
        frac: f32,
        cx: f32,
        x0: f32,
        row_y: f32,
        pw: f32,
        pad: f32,
    ) {
        self.tap(
            Rect::new(x0 + px(12.0), row_y - px(26.0), pw - px(24.0), px(70.0)),
            Tap::Select(row),
        );
        if self.cursor == row {
            draw_rectangle(x0 + px(12.0), row_y - px(26.0), pw - px(24.0), px(70.0), row_highlight());
        }
        draw_text(label, cx, row_y, px(19.0), ink());
        let vw = measure_text(value, None, px(19.0) as u16, 1.0).width;
        draw_text(value, x0 + pw - pad - vw, row_y, px(19.0), ink());

        // The track and its filled portion, with a knob at the current level. A click
        // anywhere along a band around the thin track sets the value; the fraction is
        // read from the track's own x/width (carried in the rect).
        let track_x = cx;
        let track_y = row_y + px(22.0);
        let track_w = pw - 2.0 * pad;
        let track_h = px(8.0);
        let frac = frac.clamp(0.0, 1.0);
        self.tap(
            Rect::new(track_x, track_y - px(14.0), track_w, track_h + px(28.0)),
            Tap::Slider(row, Rect::new(track_x, track_y, track_w, track_h)),
        );
        draw_rectangle(track_x, track_y, track_w, track_h, parchment_edge());
        draw_rectangle(track_x, track_y, track_w * frac, track_h, ink());
        draw_circle(track_x + track_w * frac, track_y + track_h * 0.5, px(8.0), ink());
        if self.cursor == row {
            draw_text("◄ / ►", track_x, track_y + px(34.0), px(14.0), dim_ink());
        }
    }

    /// Draw a label/value toggle row at `y`, highlighting it if the cursor is on it
    /// and right-aligning `value` to the panel's inner edge.
    #[allow(clippy::too_many_arguments)]
    fn toggle_row(&self, row: usize, label: &str, value: &str, cx: f32, x0: f32, y: f32, pw: f32, pad: f32) {
        self.tap(Rect::new(x0 + px(12.0), y - px(26.0), pw - px(24.0), px(38.0)), Tap::Select(row));
        if self.cursor == row {
            draw_rectangle(x0 + px(12.0), y - px(26.0), pw - px(24.0), px(38.0), row_highlight());
        }
        draw_text(label, cx, y, px(19.0), ink());
        let vw = measure_text(value, None, px(19.0) as u16, 1.0).width;
        draw_text(value, x0 + pw - pad - vw, y, px(19.0), ink());
    }

    /// Like `toggle_row`, but for a row that can't be changed in this build (web): the
    /// value reads "Not supported" in dim ink so it's clearly inert.
    #[allow(clippy::too_many_arguments)] // row layout geometry is inherent
    fn disabled_row(&self, row: usize, label: &str, cx: f32, x0: f32, y: f32, pw: f32, pad: f32) {
        self.tap(Rect::new(x0 + px(12.0), y - px(26.0), pw - px(24.0), px(38.0)), Tap::Select(row));
        if self.cursor == row {
            draw_rectangle(x0 + px(12.0), y - px(26.0), pw - px(24.0), px(38.0), row_highlight());
        }
        draw_text(label, cx, y, px(19.0), dim_ink());
        let value = "Not supported";
        let vw = measure_text(value, None, px(15.0) as u16, 1.0).width;
        draw_text(value, x0 + pw - pad - vw, y, px(15.0), dim_ink());
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

/// Map a slider fraction [0, 1] to the nearest scenery-density level, so a click or
/// drag lands cleanly on one of the five notches.
fn density_from_frac(f: f32) -> usize {
    let last = crate::isle_features::DENSITY_LEVELS - 1;
    (f.clamp(0.0, 1.0) * last as f32).round() as usize
}

// The menu's own inks atop the shared parchment palette (imported from `ui`).
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
