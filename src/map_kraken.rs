//! The kraken flourish for the captain's-log world map: an antique-woodcut sea-beast
//! inked into the chart's roomiest empty quarter (see [`crate::minimap::render_world`]).

use macroquad::prelude::*;

use crate::minimap::hash01;

/// Ink a kraken in the manner of an antique woodcut, the beast itself unseen: only a
/// fan of tentacles rearing out of the sea. Each arm is a closed tapering outline
/// (drawn like the whale's body: wobbled, and swelling on the shaded flank) that
/// winds into a curled tip; the front pair carries sucker dots, the great centre arm
/// engraved contour shading. The sea is ripple dashes along the waterline with
/// little splash ticks where each arm breaks it. Centred at (`cx`,`cy`) and sized to
/// `size`; `s` is the map's glyph scale (for stroke width); `col` the chart ink.
pub fn draw_kraken(cx: f32, cy: f32, size: f32, s: f32, col: Color) {
    use std::f32::consts::{FRAC_1_SQRT_2, PI};
    let th = (1.3 * s).max(1.0);
    // The same light as falls on the whale (upper right), so the pair reads as one
    // engraver's hand: strokes thicken toward the shade (screen down-left).
    let (shx, shy) = (-FRAC_1_SQRT_2, FRAC_1_SQRT_2);
    let u = |x: f32, y: f32| (cx + x * size, cy + y * size);

    // The waterline the arms rear out of, in unit space (y down).
    const WY: f32 = 0.42;

    // Each arm: root x on the waterline, base lean off vertical, reach (arc length),
    // curl direction, base half-width, and how hard the tip winds round (radians of
    // total turn). The line-up reads as one beast: tall in the middle, hooks at the
    // flanks, every tip curling toward or away from the centre in alternation.
    const ARMS: [(f32, f32, f32, f32, f32, f32); 5] = [
        (-0.86, -0.52, 0.62, 1.0, 0.058, 3.8),
        (-0.48, -0.30, 1.10, 1.0, 0.090, 4.4),
        (0.03, 0.08, 1.60, -1.0, 0.125, 4.2),
        (0.56, 0.34, 0.92, -1.0, 0.080, 4.8),
        (0.94, 0.42, 0.52, -1.0, 0.052, 3.0),
    ];

    const SEG: usize = 22;
    for (i, &(rx, lean, reach, curl, w0, wind)) in ARMS.iter().enumerate() {
        // The spine: marched up from just under the waterline, its bearing bent a
        // touch against the curl at first (an S-shaped sway) and then ever harder
        // with it, so the tip winds round on itself the way the arms of an engraved
        // sea-beast do. A per-step hash wobble keeps the march from looking drafted.
        let mut sx = [0.0f32; SEG + 1];
        let mut sy = [0.0f32; SEG + 1];
        let mut sd = [0.0f32; SEG + 1];
        let mut dir = -PI / 2.0 + lean;
        sx[0] = rx;
        sy[0] = WY + 0.06;
        sd[0] = dir;
        for j in 1..=SEG {
            let t = j as f32 / SEG as f32;
            dir += curl * (3.0 * wind * t * t - 1.6 * reach.min(1.0) * (1.0 - t)) / SEG as f32
                + 0.035 * (2.0 * hash01(i as u32 * 97 + j as u32) - 1.0);
            let step = reach * (1.25 - 0.5 * t) / SEG as f32;
            sx[j] = sx[j - 1] + dir.cos() * step;
            sy[j] = sy[j - 1] + dir.sin() * step;
            sd[j] = dir;
        }
        let w = |t: f32| (w0 * (1.0 - t)).max(0.008);

        // The outline: both offset edges of the tapering arm plus a spur past the
        // last sample to bring the tip to a point, closed across the (hidden)
        // underwater base. Traversal runs up the port edge and down the starboard
        // one, so the outward normal convention below matches the whale's.
        const M: usize = 2 * SEG + 3;
        let mut ox = [0.0f32; M];
        let mut oy = [0.0f32; M];
        for j in 0..=SEG {
            let t = j as f32 / SEG as f32;
            let (nx, ny) = ((sd[j] + PI / 2.0).cos(), (sd[j] + PI / 2.0).sin());
            let wj = w(t);
            ox[j] = sx[j] - nx * wj;
            oy[j] = sy[j] - ny * wj;
            ox[M - 1 - j] = sx[j] + nx * wj;
            oy[M - 1 - j] = sy[j] + ny * wj;
        }
        ox[SEG + 1] = sx[SEG] + sd[SEG].cos() * 0.03;
        oy[SEG + 1] = sy[SEG] + sd[SEG].sin() * 0.03;
        // Wobble each point along its local normal, then ink segment by segment
        // with the stroke weight driven by how much the normal faces the shade.
        for k in 0..M {
            let (pv, nx_) = ((k + M - 1) % M, (k + 1) % M);
            let (dx, dy) = (ox[nx_] - ox[pv], oy[nx_] - oy[pv]);
            let len = (dx * dx + dy * dy).sqrt().max(1e-6);
            let wob = 0.005 * (2.0 * hash01(i as u32 * 131 + k as u32 * 37 + 5) - 1.0);
            ox[k] += dy / len * wob;
            oy[k] += -dx / len * wob;
        }
        for k in 0..M {
            let n = (k + 1) % M;
            // The closing run across the base lies under the sea; the ripple dashes
            // own the waterline, so nothing is inked below it.
            if (oy[k] + oy[n]) * 0.5 > WY + 0.02 {
                continue;
            }
            let (dx, dy) = (ox[n] - ox[k], oy[n] - oy[k]);
            let len = (dx * dx + dy * dy).sqrt().max(1e-6);
            // Outward normal for this traversal order is (dy, -dx).
            let shade = ((dy * shx - dx * shy) / len).max(0.0);
            let a = u(ox[k], oy[k]);
            let b = u(ox[n], oy[n]);
            draw_line(a.0, a.1, b.0, b.1, th * (0.7 + 0.9 * shade), col);
        }

        // Sucker dots down the concave edge of the two face-side arms only, so the
        // detail stays in one place instead of stippling every arm.
        if i == 2 || i == 3 {
            for j in (0..=SEG).step_by(2) {
                let t = j as f32 / SEG as f32;
                if !(0.20..0.85).contains(&t) {
                    continue;
                }
                let na = sd[j] + curl * PI / 2.0;
                let p = u(sx[j] + na.cos() * w(t) * 0.32, sy[j] + na.sin() * w(t) * 0.32);
                draw_circle(p.0, p.1, (th * (0.3 + 0.5 * (1.0 - t))).max(0.8), col);
            }
        }

        // Engraved hatching on the great centre arm: short diagonal ticks laid
        // across the shaded flank of the lower half, the way an engraver darkens
        // one side of a column.
        if i == 2 {
            for j in (2..=SEG / 2).step_by(2) {
                let t = j as f32 / SEG as f32;
                let (nx, ny) = ((sd[j] + PI / 2.0).cos(), (sd[j] + PI / 2.0).sin());
                let sgn = if nx * shx + ny * shy >= 0.0 { 1.0 } else { -1.0 };
                let (tx, ty) = (sd[j].cos(), sd[j].sin());
                let wj = w(t);
                let a = u(sx[j] + nx * sgn * wj * 0.68, sy[j] + ny * sgn * wj * 0.68);
                let b = u(
                    sx[j] + nx * sgn * wj * 0.15 + tx * wj * 0.6,
                    sy[j] + ny * sgn * wj * 0.15 + ty * wj * 0.6,
                );
                draw_line(a.0, a.1, b.0, b.1, (th * 0.45).max(0.8), col);
            }
        }

        // Splash ticks where the arm breaks the surface: a pair of short strokes
        // flung up and outward on either side of the root.
        if reach > 0.7 {
            for side in [-1.0f32, 1.0] {
                let bx = rx + side * (w0 + 0.030);
                let a = u(bx, WY + 0.005);
                let b = u(bx + side * 0.05, WY - 0.055);
                draw_line(a.0, a.1, b.0, b.1, (th * 0.5).max(0.8), col);
            }
        }
    }

    // Ripple dashes along the waterline in the gaps between the arms (and a pair
    // out past the flanking hooks), the stylised sea of an old chart.
    for (r, &(rx, half)) in
        [(-1.02f32, 0.09f32), (-0.68, 0.10), (-0.23, 0.14), (0.28, 0.13), (0.71, 0.09), (1.09, 0.08)]
            .iter()
            .enumerate()
            .map(|(r, v)| (r as u32, v))
    {
        let hw = half * size;
        let ry = WY + 0.015 + 0.03 * (hash01(r * 13 + 3) - 0.5);
        const RSEG: usize = 6;
        let mut prev = u(rx - half, ry);
        for j in 1..=RSEG {
            let t = j as f32 / RSEG as f32;
            let x = cx + rx * size - hw + 2.0 * hw * t;
            let y = cy + ry * size - (t * PI).sin() * size * 0.022 * (1.0 + 0.4 * hash01(r * 7 + 1));
            draw_line(prev.0, prev.1, x, y, (th * 0.5).max(0.8), col);
            prev = (x, y);
        }
    }
}
