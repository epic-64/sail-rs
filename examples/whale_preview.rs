//! Throwaway dev preview: renders the map beasts big on a parchment ground and dumps
//! a PNG, so the artwork can be iterated on without navigating the game to the log.

use macroquad::prelude::*;

// Shim for the `crate::minimap::hash01` the beast modules use (same function).
mod minimap {
    pub(crate) fn hash01(n: u32) -> f32 {
        let mut x = n.wrapping_mul(0x9e3779b1);
        x ^= x >> 16;
        x = x.wrapping_mul(0x7feb352d);
        x ^= x >> 15;
        (x & 0xffff) as f32 / 65535.0
    }
}
#[path = "../src/map_kraken.rs"]
mod map_kraken;
#[path = "../src/map_whale.rs"]
mod map_whale;

fn conf() -> Conf {
    Conf { window_title: "beast preview".into(), window_width: 1280, window_height: 720, ..Default::default() }
}

#[macroquad::main(conf)]
async fn main() {
    let out = std::env::var("PREVIEW_OUT").unwrap_or_else(|_| "whale_preview.png".into());
    let mut frames = 0;
    loop {
        clear_background(Color::new(0.86, 0.79, 0.64, 1.0));
        let ink = Color::new(0.23, 0.17, 0.11, 1.0);
        let (w, h) = (screen_width(), screen_height());
        map_whale::draw_whale(w * 0.46, h * 0.42, 230.0, 5.8, ink);
        // Small, at roughly in-game map scale.
        map_whale::draw_whale(w * 0.75, h * 0.85, 36.0, 1.0, ink);
        map_kraken::draw_kraken(w * 0.90, h * 0.80, 60.0, 1.6, ink);
        frames += 1;
        if frames >= 4 {
            get_screen_data().export_png(&out);
            break;
        }
        next_frame().await;
    }
}
