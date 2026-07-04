//! The whale flourish for the captain's-log world map: the companion sea-beast to the
//! kraken, inked into the chart's next-roomiest gap (see [`crate::minimap::render_world`]).

use macroquad::prelude::*;

/// Ink a small hand-drawn whale (a humpbacked body facing left, with eye, flipper,
/// tail flukes, and a spout) centred at (`cx`,`cy`) and sized to `size`, the companion
/// flourish to the kraken on the world map. `s` is the glyph scale; `col` the ink.
pub fn draw_whale(cx: f32, cy: f32, size: f32, s: f32, col: Color) {
    let th = (1.2 * s).max(1.0);
    let l = size * 1.5; // body length
    let h = size * 0.78; // body height

    // The body: a tapered, humped oval outline (head left, narrowing to the tail right).
    const N: usize = 16;
    let mut bx = [0.0f32; N];
    let mut by = [0.0f32; N];
    for k in 0..N {
        let ang = k as f32 / N as f32 * std::f32::consts::TAU;
        let ex = ang.cos();
        let ey = ang.sin();
        let taper = 1.0 - 0.6 * ex.max(0.0); // narrow toward the tail
        let hump = if ey < 0.0 { -0.12 * h * (ex * 0.5 + 0.5) } else { 0.0 };
        bx[k] = cx + ex * l * 0.5;
        by[k] = cy + ey * h * 0.5 * taper + hump;
    }
    for k in 0..N {
        let n = (k + 1) % N;
        draw_line(bx[k], by[k], bx[n], by[n], th, col);
    }

    // Tail flukes at the right tip.
    let tx = cx + l * 0.5;
    let f = size * 0.5;
    draw_line(tx, cy, tx + f, cy - f * 0.8, th, col);
    draw_line(tx + f, cy - f * 0.8, tx + f * 0.35, cy, th, col);
    draw_line(tx, cy, tx + f, cy + f * 0.8, th, col);
    draw_line(tx + f, cy + f * 0.8, tx + f * 0.35, cy, th, col);

    // Eye near the head, a small flipper under the belly, and a spout above the head.
    draw_circle(cx - l * 0.34, cy - h * 0.08, (size * 0.05).max(1.0), col);
    draw_line(cx - l * 0.1, cy + h * 0.32, cx + l * 0.05, cy + h * 0.55, th, col);
    draw_line(cx + l * 0.05, cy + h * 0.55, cx + l * 0.12, cy + h * 0.3, th, col);
    let (spx, spy) = (cx - l * 0.3, cy - h * 0.5);
    for &dxs in &[-0.18f32, 0.0, 0.18] {
        draw_line(spx, spy, spx + dxs * size, spy - size * 0.55, th * 0.9, col);
    }
}
