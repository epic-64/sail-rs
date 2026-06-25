//! Text faces for the UI.
//!
//! macroquad's bundled default font is `ProggyClean` — a small bitmap-style pixel
//! font that turns soft and blurry once it's antialiased up to HUD/logbook sizes,
//! and (the bigger problem) carries no arrows, dashes or symbols, so glyphs like
//! `→ ← ↑ ↓ ° · ≈ ±` render as blanks. We replace it with two faces:
//!
//!   - **sans** (DejaVu Sans) — the body face for *everything*: HUD, world overlays,
//!     and all the running text on the captain's log and the port boards. Crisp at
//!     any size with full symbol coverage.
//!   - **display** (IM Fell English SC) — a 1600s English printing-press small-caps
//!     face used **only for headings** (spread titles, port names, section headers),
//!     for an old sea-chart / ship's-log flavour. Never used for body text — it's
//!     characterful but not a reading face, and lacks the symbol glyphs the body
//!     needs anyway.
//!
//! Rather than thread a `Font` through every `draw_text` call site, we lean on
//! macroquad's global [`set_default_font`]: every `draw_text` / `measure_text(.., None,
//! ..)` falls back to that default. Sans is the standing default (reset at the top of
//! every frame); a heading is drawn by bracketing its draw/measure calls in
//! [`heading`], which flips the default to the display face and restores sans after.
//! The renderer is single-threaded and draws in sequence, so the global swap is safe.
//!
//! Both `.ttf` files are embedded with `include_bytes!`, so the binary is
//! self-contained (no runtime asset path — important for the wasm build).

use std::cell::RefCell;

use macroquad::prelude::*;
use macroquad::text::set_default_font;

struct Faces {
    sans: Font,
    display: Font,
}

thread_local! {
    static FACES: RefCell<Option<Faces>> = const { RefCell::new(None) };
}

/// Load the faces and make sans the active default. Must be called once after the
/// macroquad context exists (it uploads glyph atlases to the GPU), i.e. inside
/// `async fn main`, before the render loop.
pub fn init() {
    let sans = load_ttf_font_from_bytes(include_bytes!("../assets/fonts/DejaVuSans.ttf"))
        .expect("DejaVuSans.ttf failed to load");
    let display = load_ttf_font_from_bytes(include_bytes!("../assets/fonts/IMFellEnglishSC.ttf"))
        .expect("IMFellEnglishSC.ttf failed to load");
    FACES.with(|f| *f.borrow_mut() = Some(Faces { sans, display }));
    use_sans();
}

/// Make the body sans the active font. The standing default — reset at the top of
/// every frame and after each heading.
pub fn use_sans() {
    FACES.with(|f| {
        if let Some(faces) = f.borrow().as_ref() {
            set_default_font(faces.sans.clone());
        }
    });
}

/// Draw a heading in the display face, then restore the body sans. Wrap a heading's
/// `measure_text`/`draw_text` calls (which use the default-font fallback) in this:
///
/// ```ignore
/// font::heading(|| {
///     let d = measure_text(title, None, fs, 1.0);
///     draw_text(title, cx - d.width / 2.0, y, fs as f32, ink);
/// });
/// ```
pub fn heading<R>(draw: impl FnOnce() -> R) -> R {
    FACES.with(|f| {
        if let Some(faces) = f.borrow().as_ref() {
            set_default_font(faces.display.clone());
        }
    });
    let r = draw();
    use_sans();
    r
}
