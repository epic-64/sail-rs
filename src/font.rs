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
//! The `draw_text` / `measure_text` every module actually calls are *this module's*
//! shadows of the macroquad pair (imported explicitly, so they win over the prelude
//! glob): same signatures, but the size is quantized first, which is what keeps the
//! glyph cache bounded (see "Glyph-cache hygiene" below).
//!
//! Both `.ttf` files are embedded with `include_bytes!`, so the binary is
//! self-contained (no runtime asset path — important for the wasm build).

use std::cell::RefCell;

use macroquad::prelude::*;
use macroquad::text::set_default_font;

use crate::ui;

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
    let faces = Faces { sans, display };
    warm(&faces);
    FACES.with(|f| *f.borrow_mut() = Some(faces));
    use_sans();
}

// --- Glyph-cache hygiene --------------------------------------------------------
//
// macroquad rasterizes glyphs lazily into a per-font atlas keyed by
// (character, font size) and never evicts; any frame that adds a glyph
// re-uploads the whole atlas texture. Left unmanaged that means two failure
// modes: a size that varies continuously (a 3D-projected label tracking the
// camera) leaks a fresh glyph set nearly every frame, and even a fixed size
// stalls the game the first time a menu draws it.
//
// The game's answer is to make the set of sizes that can ever reach the cache
// *finite*: all text draws through this module's [`draw_text`] /
// [`measure_text`], which quantize any requested size through [`bucket`],
// and [`init`] pre-rasterizes the everyday rungs so nothing hitches mid-game.
// Every text-drawing module imports the pair explicitly, shadowing the
// macroquad prelude the same way `geometry::Vec2` shadows glam's. Only text
// that needs raw `TextParams` (rotation) bypasses the shadow, and such a site
// must run its size through [`bucket`] itself (see the berth tags in
// `ship_render`).

/// Sizes up to here rasterize at their exact integer pixel size, so ordinary
/// text is drawn precisely as crisply as without any quantization. Above it,
/// sizes snap to the [`COARSE`] rungs.
const EXACT_MAX: u16 = 32;

/// The rungs above [`EXACT_MAX`], spaced ~1.1x apart so the residual
/// `font_scale` stays in (0.9, 1.0]: glyphs only ever scale *down*, by a
/// margin that is invisible at these sizes.
const COARSE: [u16; 19] = [
    36, 40, 44, 48, 53, 58, 64, 70, 77, 85, 94, 103, 113, 124, 136, 150, 165, 182, 200,
];

/// Quantize a desired on-screen pixel size to the `(font_size, font_scale)`
/// pair that reaches it: exact below [`EXACT_MAX`], the next rung up (scaled
/// down to fit) above it, the top rung scaled up past the ladder's end.
pub fn bucket(desired_px: f32) -> (u16, f32) {
    if desired_px <= EXACT_MAX as f32 {
        return (desired_px.floor().max(1.0) as u16, 1.0);
    }
    let size = COARSE
        .iter()
        .copied()
        .find(|&b| desired_px <= b as f32)
        .unwrap_or(COARSE[COARSE.len() - 1]);
    (size, desired_px / size as f32)
}

/// Drop-in shadow of macroquad's `draw_text` (same signature and return), the
/// size quantized through [`bucket`] before it can touch the glyph cache.
/// Follows the active default face, so it works inside [`heading`] closures.
pub fn draw_text(
    text: impl AsRef<str>,
    x: f32,
    y: f32,
    font_size: f32,
    color: Color,
) -> TextDimensions {
    let (size, scale) = bucket(font_size);
    macroquad::text::draw_text_ex(
        text,
        x,
        y,
        TextParams {
            font_size: size,
            font_scale: scale,
            color,
            ..Default::default()
        },
    )
}

/// Drop-in shadow of macroquad's `measure_text`, quantized exactly as
/// [`draw_text`] draws, so measured and drawn dimensions agree.
pub fn measure_text(
    text: &str,
    font: Option<&Font>,
    font_size: u16,
    font_scale: f32,
) -> TextDimensions {
    let (size, scale) = bucket(font_size as f32 * font_scale);
    macroquad::text::measure_text(text, font, size, scale)
}

/// Symbols the game draws beyond printable ASCII (arrows, marks, vulgar
/// fractions). Best-effort: a glyph missing here still renders, it just
/// rasterizes on first use.
const SYMBOL_GLYPHS: &str = "←↑→↓►◄✓✕•·…°±−–×≈≤≥¼½¾²";

/// Pre-rasterize what [`bucket`] can emit at everyday text sizes: every exact
/// integer rung, plus the coarse rungs up to just past this screen's title
/// tier. Larger rungs (rare event banners) rasterize lazily, at most once per
/// rung ever, against a still-small atlas. Both faces get the full set, so a
/// board or menu opening mid-game finds every glyph already cached. The cache
/// looks glyphs up under the dpi-scaled size (`ceil(font_size * dpi_scale)`),
/// so the warm keys the same way.
fn warm(faces: &Faces) {
    let chars: Vec<char> = (' '..='~').chain(SYMBOL_GLYPHS.chars()).collect();
    let dpi = macroquad::miniquad::window::dpi_scale();
    let top = ui::px(26.0).ceil();
    let sizes =
        (1..=EXACT_MAX).chain(COARSE.iter().copied().take_while(|&b| (b as f32) <= top));
    for size in sizes {
        let key = (size as f32 * dpi).ceil() as u16;
        faces.sans.populate_font_cache(&chars, key);
        faces.display.populate_font_cache(&chars, key);
    }
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

#[cfg(test)]
mod tests {
    use super::{bucket, COARSE, EXACT_MAX};

    /// Small text must come out exactly as asked (integer size, no
    /// resampling): this is what keeps body text pixel-identical to
    /// unquantized drawing.
    #[test]
    fn small_sizes_are_exact() {
        for want in 1..=EXACT_MAX {
            assert_eq!(bucket(want as f32), (want, 1.0));
        }
    }

    /// Above the exact band the chosen rung sits at or above the target, the
    /// residual scale lands the exact size by only ever shrinking, and the
    /// rungs are spaced tightly enough that the shrink stays mild.
    #[test]
    fn coarse_sizes_never_upscale() {
        let top = COARSE[COARSE.len() - 1];
        for want in (EXACT_MAX + 1)..=top {
            let (size, scale) = bucket(want as f32);
            assert!(size >= want, "rung below target at {want}");
            assert!(scale <= 1.0 + 1e-6, "size {want} upscales");
            assert!((size as f32 * scale - want as f32).abs() < 1e-3);
            assert!(scale > 0.9, "rung spacing too coarse at {want}");
        }
    }

    /// Past the top rung the size stays pinned (the cache stays bounded) and
    /// the scale takes up the rest.
    #[test]
    fn bounded_above() {
        let (size, scale) = bucket(500.0);
        assert_eq!(size, COARSE[COARSE.len() - 1]);
        assert!((size as f32 * scale - 500.0).abs() < 1e-3);
    }
}
