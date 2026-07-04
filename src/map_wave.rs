//! The breaking-wave flourish for the captain's-log world map: the third empty-quarter
//! ornament alongside the kraken and whale (see [`crate::minimap::render_world`]).

use macroquad::prelude::*;

/// Ink a small hand-drawn breaking wave (a swell curling into a foam-tipped lip over a
/// gently rippled surface) centred at (`cx`,`cy`) and sized to `size`, the third of the
/// world map's empty-quarter flourishes. `s` is the glyph scale; `col` the ink.
pub fn draw_wave(cx: f32, cy: f32, size: f32, s: f32, col: Color) {
    let th = (1.2 * s).max(1.0);
    // A quadratic bezier, laid down as short segments (macroquad draws only straight
    // lines) so the swell and lip read as smooth hand-drawn curves.
    let bez = |p0: (f32, f32), p1: (f32, f32), p2: (f32, f32)| {
        const ST: i32 = 14;
        let mut prev = p0;
        for i in 1..=ST {
            let t = i as f32 / ST as f32;
            let u = 1.0 - t;
            let x = u * u * p0.0 + 2.0 * u * t * p1.0 + t * t * p2.0;
            let y = u * u * p0.1 + 2.0 * u * t * p1.1 + t * t * p2.1;
            draw_line(prev.0, prev.1, x, y, th, col);
            prev = (x, y);
        }
    };

    // Back swell rising from the trough to the crest, the curling lip, and the inner curl.
    let crest = (cx + 0.15 * size, cy - 0.6 * size);
    let lip = (cx + 0.7 * size, cy - 0.05 * size);
    bez((cx - size, cy + 0.45 * size), (cx - 0.5 * size, cy - 0.55 * size), crest);
    bez(crest, (cx + 0.75 * size, cy - 0.75 * size), lip);
    bez(lip, (cx + 0.55 * size, cy + 0.2 * size), (cx + 0.2 * size, cy + 0.05 * size));
    bez(crest, (cx + 0.45 * size, cy - 0.4 * size), (cx + 0.35 * size, cy - 0.05 * size));

    // Foam droplets flung off the lip.
    for &(fx, fy, fr) in &[(0.4f32, -0.7f32, 0.06f32), (0.6, -0.55, 0.05), (0.25, -0.78, 0.045)] {
        draw_circle(cx + fx * size, cy + fy * size, (size * fr).max(1.0), col);
    }

    // The water surface: a gentle ripple beneath the swell.
    let base_y = cy + 0.5 * size;
    const STEPS: i32 = 24;
    let mut prev = (cx - size, base_y);
    for i in 1..=STEPS {
        let t = i as f32 / STEPS as f32;
        let x = cx - size + t * 2.0 * size;
        let y = base_y + (t * std::f32::consts::PI * 3.0).sin() * size * 0.07;
        draw_line(prev.0, prev.1, x, y, th * 0.9, col);
        prev = (x, y);
    }
}
