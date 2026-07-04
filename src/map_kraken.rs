//! The kraken flourish for the captain's-log world map: an antique-woodcut sea-beast
//! inked into the chart's roomiest empty quarter (see [`crate::minimap::render_world`]).

use macroquad::prelude::*;

use crate::minimap::hash01;

/// Ink a small kraken in the manner of an antique woodcut: a peaked mantle whose
/// outline swells on the shaded flank, engraved hatching along that flank, fierce
/// browed eyes over a hooked beak, and tentacles that taper into tight spiral curls
/// (the front pair dotted with suckers). Centred at (`cx`,`cy`) and sized to `size`,
/// for the empty-quarter flourish on the world map. `s` is the map's glyph scale
/// (for stroke width); `col` the chart ink.
pub fn draw_kraken(cx: f32, cy: f32, size: f32, s: f32, col: Color) {
    use std::f32::consts::{FRAC_1_SQRT_2, PI, TAU};
    let th = (1.3 * s).max(1.0);
    let hr = size * 0.42; // mantle radius
    // The carver's light falls from the upper right, so strokes thicken toward the
    // opposite flank; this points into the shade (screen down-left).
    let (shx, shy) = (-FRAC_1_SQRT_2, FRAC_1_SQRT_2);

    // The mantle: a wobbled dome, taller than wide with a peaked crown, tucked in at
    // the bottom where the arms emerge. Laid down segment by segment so the stroke
    // can swell on the shaded side the way a carved line breathes.
    const N: usize = 26;
    let mut px = [0.0f32; N];
    let mut py = [0.0f32; N];
    for k in 0..N {
        let ang = k as f32 / N as f32 * TAU;
        let crown = ang.cos().max(0.0).powi(3) * 0.22;
        let tuck = (-ang.cos()).max(0.0) * 0.10;
        let wob = 0.035 * (2.0 * hash01(k as u32 * 41 + 7) - 1.0);
        let rr = hr * (1.0 + crown - tuck + wob);
        px[k] = cx + ang.sin() * rr;
        py[k] = cy - ang.cos() * rr * 1.15;
    }
    for k in 0..N {
        let n = (k + 1) % N;
        let mid = (k as f32 + 0.5) / N as f32 * TAU;
        // The outward normal's shade-facing share drives the stroke weight.
        let shade = (mid.sin() * shx - mid.cos() * shy).max(0.0);
        draw_line(px[k], py[k], px[n], py[n], th * (0.75 + 0.95 * shade), col);
    }

    // Engraved shading on the lower-left flank: arcs running parallel to the rim,
    // the way an engraver lays latitude lines under a sphere's shadowed cheek.
    for (h, &rf) in [0.94f32, 0.84, 0.73].iter().enumerate() {
        let r = hr * rf;
        const HSEG: usize = 8;
        let a0 = PI * (1.06 + 0.03 * h as f32);
        let a1 = PI * (1.52 - 0.05 * h as f32);
        let mut prev: Option<(f32, f32)> = None;
        for j in 0..=HSEG {
            let a = a0 + (a1 - a0) * j as f32 / HSEG as f32;
            let pt = (cx + a.sin() * r, cy - a.cos() * r * 1.15);
            if let Some(pr) = prev {
                draw_line(pr.0, pr.1, pt.0, pt.1, (th * 0.55).max(0.8), col);
            }
            prev = Some(pt);
        }
    }

    // Great round eyes on the upper mantle, each glaring from under a heavy lid
    // slash that drops steeply toward the midline and chops the ring half-closed.
    let er = (hr * 0.17).max(1.2);
    for side in [-1.0f32, 1.0] {
        let ex = cx + side * hr * 0.38;
        let ey = cy - hr * 0.28;
        draw_circle_lines(ex, ey, er, (th * 0.8).max(1.0), col);
        draw_circle(ex - side * er * 0.1, ey + er * 0.15, (er * 0.55).max(1.0), col);
        draw_line(
            ex + side * er * 1.2,
            ey - er * 1.1,
            ex - side * er * 1.05,
            ey + er * 0.05,
            (th * 1.3).max(1.0),
            col,
        );
    }

    // The arms: a fan of tentacles rooted under the mantle, each a chain of tapering
    // strokes whose bearing turns ever tighter toward the tip, so the ends wind into
    // the open spiral curls of an engraved sea-beast.
    const N_TENT: usize = 8;
    for i in 0..N_TENT {
        let frac = i as f32 / (N_TENT - 1) as f32;
        let fan = PI * (0.02 + 0.96 * frac); // 0 = east, sweeping down and round to west
        // Every arm curls outward and up, the menacing splay of the old engravings.
        let curl = if fan < PI / 2.0 { -1.0 } else { 1.0 };
        let mut p = (cx + fan.cos() * hr * 0.70, cy + hr * 0.86 + fan.sin() * hr * 0.22);
        let reach = size * (0.72 + 0.36 * fan.sin()) * (0.90 + 0.16 * hash01(i as u32 * 29 + 11));
        const SEG: usize = 20;
        let step0 = reach / SEG as f32 * 1.4;
        // Per-arm wind strength, so some tips wrap a full turn and others stay looser.
        // The near-vertical front pair gets extra so it writhes instead of dangling.
        let mut wind = 0.95 * (0.85 + 0.45 * hash01(i as u32 * 53 + 17));
        if i == 3 || i == 4 {
            wind *= 1.3;
        }
        let mut dir = fan;
        for j in 1..=SEG {
            let t = j as f32 / SEG as f32;
            // A slight early lean against the curl gives each arm an S-shaped sway
            // before the quadratic term wraps the tip round on itself.
            dir += curl * (wind * t * t - 0.06)
                + 0.04 * (2.0 * hash01(i as u32 * 97 + j as u32) - 1.0);
            let step = step0 * (1.0 - 0.60 * t);
            let q = (p.0 + dir.cos() * step, p.1 + dir.sin() * step);
            let w = (th * (0.2 + 1.6 * (1.0 - t))).max(0.7);
            draw_line(p.0, p.1, q.0, q.1, w, col);
            // Sucker dots along the concave edge of the front pair only, so the
            // detail stays on the beast's face-side arms instead of everywhere.
            if (i == 3 || i == 4) && j % 2 == 0 && t < 0.6 {
                let pa = dir + curl * PI / 2.0;
                draw_circle(
                    (p.0 + q.0) / 2.0 + pa.cos() * w,
                    (p.1 + q.1) / 2.0 + pa.sin() * w,
                    (th * 0.5).max(0.8),
                    col,
                );
            }
            p = q;
        }
    }

    // Ripple dashes among the arms, the stylised sea an old chart's beast rears from.
    for (r, &(ox, oy, half)) in [(-0.62f32, 1.02f32, 0.18f32), (0.64, 1.06, 0.16), (0.0, 1.20, 0.22)]
        .iter()
        .enumerate()
        .map(|(r, v)| (r as u32, v))
    {
        let (rx, ry) = (cx + ox * size, cy + oy * size);
        let hw = half * size;
        const RSEG: usize = 6;
        let mut prev = (rx - hw, ry);
        for j in 1..=RSEG {
            let t = j as f32 / RSEG as f32;
            let x = rx - hw + 2.0 * hw * t;
            let y = ry - (t * PI).sin() * size * 0.08 * (1.0 + 0.4 * hash01(r * 7 + 1));
            draw_line(prev.0, prev.1, x, y, (th * 0.5).max(0.8), col);
            prev = (x, y);
        }
    }
}
