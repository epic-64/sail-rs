//! Tracks which non-pointer input device the player is currently using —
//! keyboard or gamepad — so on-screen keybind hints show the right glyphs.
//! Touch/mouse is its own thing, handled by [`crate::touch::TouchState::active`]
//! (which now also folds in gamepad edges to hide the mobile overlay, see
//! `touch.rs`); this tracker only decides *which* letter/word a hint uses once
//! that overlay is down.
//!
//! A thread_local rather than threading a parameter through every hint-drawing
//! function in `hud.rs`, `guide.rs`, `captains_log.rs`, `port_view.rs`,
//! `dig_view.rs` and `pause_menu.rs` — set once a frame from `main`'s loop,
//! read wherever a hint is drawn. Mirrors the pattern `ship_render.rs` uses for
//! `SHATTER`.

std::thread_local! {
    static GAMEPAD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Call once a frame, after `touch::TouchState::update` and `Pad::update`, with
/// whether a key was pressed this frame and whether the pad had a fresh edge
/// ([`crate::pad::Pad::any_pressed`]). Flips the remembered device on whichever
/// fired; leaves it alone when neither did, so a held direction (or the analog
/// steer sitting mid-turn) doesn't flip it back.
pub fn update(key_pressed: bool, pad_pressed: bool) {
    if pad_pressed {
        GAMEPAD.with(|c| c.set(true));
    } else if key_pressed {
        GAMEPAD.with(|c| c.set(false));
    }
}

/// True when the last keyboard/gamepad input came from the pad; on-screen
/// hints should show controller glyphs instead of key letters. `SAIL_PAD=1`
/// (native env var) forces gamepad hints on for desktop testing without a
/// controller, mirroring `SAIL_TOUCH` in `touch.rs`.
pub fn gamepad() -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    if std::env::var("SAIL_PAD").is_ok_and(|v| v == "1") {
        return true;
    }
    GAMEPAD.with(|c| c.get())
}

/// Pick the keyboard or gamepad phrasing of a hint, whichever fits the device
/// last used. A thin wrapper around [`gamepad`] so call sites read as prose.
pub fn hint<'a>(keyboard: &'a str, pad: &'a str) -> &'a str {
    if gamepad() {
        pad
    } else {
        keyboard
    }
}
