//! The breaking-wave flourish for the captain's-log world map: the third empty-quarter
//! ornament alongside the kraken and whale (see [`crate::minimap::render_world`]).

use macroquad::prelude::*;

use crate::minimap::hash01;

/// Ink a breaking wave in the manner of an antique woodcut, kin to the kraken and the
/// whale: a swell rearing out of the sea, its lip winding over into a curl drawn as a
/// tapering wobbled outline that swells on the shaded flank, the curl's outer edge
/// broken into scalloped foam, spray flung ahead of the crest, and engraved contour
/// shading laid down the back slope and the hollow of the face. Centred at (`cx`,`cy`)
/// and sized to `size`; `s` is the map's glyph scale (for stroke width); `col` the ink.
pub fn draw_wave(cx: f32, cy: f32, size: f32, s: f32, col: Color) {
    use std::f32::consts::{FRAC_1_SQRT_2, PI};
    let th = (1.3 * s).max(1.0);
    // The same light as falls on the beasts (upper right), so the three flourishes
    // read as one engraver's hand: strokes thicken toward the shade (screen down-left).
    let (shx, shy) = (-FRAC_1_SQRT_2, FRAC_1_SQRT_2);
    let u = |x: f32, y: f32| (cx + x * size, cy + y * size);

    // A quadratic bezier laid down as short segments, for every curved detail stroke.
    let bez = |p0: (f32, f32), p1: (f32, f32), p2: (f32, f32), w: f32| {
        const ST: usize = 10;
        let mut prev = u(p0.0, p0.1);
        for i in 1..=ST {
            let t = i as f32 / ST as f32;
            let v = 1.0 - t;
            let q = u(
                v * v * p0.0 + 2.0 * v * t * p1.0 + t * t * p2.0,
                v * v * p0.1 + 2.0 * v * t * p1.1 + t * t * p2.1,
            );
            draw_line(prev.0, prev.1, q.0, q.1, w, col);
            prev = q;
        }
    };

    // The waterline the swell rears out of, in unit space (y down).
    const WY: f32 = 0.42;

    // The swell: a closed Catmull-Rom outline through hand-placed points (unit space,
    // the sea running left to right, y down), sampled into short segments so the
    // stroke can wobble like a carved line and swell on the shaded side. The run
    // below the waterline is never inked (the ripple dashes own the sea, as with the
    // kraken's arms).
    const PTS: [(f32, f32); 13] = [
        (-1.05, 0.44), // left foot, under the waterline
        (-0.78, 0.30), // the back slope, long and shallow
        (-0.44, 0.10),
        (-0.10, -0.22),
        (0.20, -0.44), // shoulder steepening toward the crest
        (0.40, -0.54), // crest (the lip springs from here)
        (0.56, -0.50),
        (0.52, -0.26), // the face hollows under the overhanging lip
        (0.58, 0.00),
        (0.74, 0.24),
        (0.88, 0.44), // fore foot, under the waterline
        (0.30, 0.50), // underwater return, never inked
        (-0.40, 0.50),
    ];
    const SUB: usize = 6;
    const M: usize = PTS.len() * SUB;
    let mut ox = [0.0f32; M];
    let mut oy = [0.0f32; M];
    for i in 0..PTS.len() {
        let p0 = PTS[(i + PTS.len() - 1) % PTS.len()];
        let p1 = PTS[i];
        let p2 = PTS[(i + 1) % PTS.len()];
        let p3 = PTS[(i + 2) % PTS.len()];
        for j in 0..SUB {
            let t = j as f32 / SUB as f32;
            let (t2, t3) = (t * t, t * t * t);
            let cr = |a: f32, b: f32, c: f32, d: f32| {
                0.5 * (2.0 * b
                    + (c - a) * t
                    + (2.0 * a - 5.0 * b + 4.0 * c - d) * t2
                    + (3.0 * (b - c) + d - a) * t3)
            };
            ox[i * SUB + j] = cr(p0.0, p1.0, p2.0, p3.0);
            oy[i * SUB + j] = cr(p0.1, p1.1, p2.1, p3.1);
        }
    }
    // Wobble each sample along its outward normal, then ink the loop segment by
    // segment with the stroke weight driven by how much the normal faces the shade.
    for k in 0..M {
        let (pv, nx) = ((k + M - 1) % M, (k + 1) % M);
        let (dx, dy) = (ox[nx] - ox[pv], oy[nx] - oy[pv]);
        let len = (dx * dx + dy * dy).sqrt().max(1e-6);
        let wob = 0.012 * (2.0 * hash01(k as u32 * 41 + 9) - 1.0);
        ox[k] += dy / len * wob;
        oy[k] += -dx / len * wob;
    }
    for k in 0..M {
        let n = (k + 1) % M;
        if (oy[k] + oy[n]) * 0.5 > WY + 0.01 {
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

    // The lip: a tapering spine marched out of the crest and wound clockwise into
    // the curl, built exactly like a kraken arm (both offset edges closed over a
    // pointed tip, wobbled, shade-weighted). The convex outer edge is not inked
    // smooth; the foam scallops below take its place.
    const SEG: usize = 18;
    const WIND: f32 = 4.5;
    const REACH: f32 = 1.05;
    let mut sx = [0.0f32; SEG + 1];
    let mut sy = [0.0f32; SEG + 1];
    let mut sd = [0.0f32; SEG + 1];
    let mut dir = -0.60f32;
    sx[0] = 0.36;
    sy[0] = -0.53;
    sd[0] = dir;
    for j in 1..=SEG {
        let t = j as f32 / SEG as f32;
        dir += WIND * (0.5 + 1.5 * t * t) / SEG as f32
            + 0.030 * (2.0 * hash01(j as u32 * 89 + 7) - 1.0);
        let step = REACH * (1.3 - 0.6 * t) / SEG as f32;
        sx[j] = sx[j - 1] + dir.cos() * step;
        sy[j] = sy[j - 1] + dir.sin() * step;
        sd[j] = dir;
    }
    let w = |t: f32| (0.13 * (1.0 - t)).max(0.008);

    const LM: usize = 2 * SEG + 3;
    let mut lx = [0.0f32; LM];
    let mut ly = [0.0f32; LM];
    for j in 0..=SEG {
        let t = j as f32 / SEG as f32;
        let (nx, ny) = ((sd[j] + PI / 2.0).cos(), (sd[j] + PI / 2.0).sin());
        let wj = w(t);
        lx[j] = sx[j] - nx * wj;
        ly[j] = sy[j] - ny * wj;
        lx[LM - 1 - j] = sx[j] + nx * wj;
        ly[LM - 1 - j] = sy[j] + ny * wj;
    }
    lx[SEG + 1] = sx[SEG] + sd[SEG].cos() * 0.03;
    ly[SEG + 1] = sy[SEG] + sd[SEG].sin() * 0.03;
    for k in 0..LM {
        let (pv, nx_) = ((k + LM - 1) % LM, (k + 1) % LM);
        let (dx, dy) = (lx[nx_] - lx[pv], ly[nx_] - ly[pv]);
        let len = (dx * dx + dy * dy).sqrt().max(1e-6);
        let wob = 0.006 * (2.0 * hash01(k as u32 * 53 + 3) - 1.0);
        lx[k] += dy / len * wob;
        ly[k] += -dx / len * wob;
    }
    for k in 0..LM - 1 {
        // The outer edge carries the foam scallops instead of a smooth stroke, and
        // the base closing run is buried in the crest; ink the tip and the hollow
        // underside only.
        if k < SEG {
            continue;
        }
        let n = k + 1;
        let (dx, dy) = (lx[n] - lx[k], ly[n] - ly[k]);
        let len = (dx * dx + dy * dy).sqrt().max(1e-6);
        let shade = ((dy * shx - dx * shy) / len).max(0.0);
        let a = u(lx[k], ly[k]);
        let b = u(lx[n], ly[n]);
        draw_line(a.0, a.1, b.0, b.1, th * (0.7 + 0.9 * shade), col);
    }

    // The foam: the curl's outer edge broken into scallops, each a little arc bowed
    // outward between neighbouring edge samples, shrinking as the lip tapers.
    for j in (0..SEG - 1).step_by(2) {
        let t = (j + 1) as f32 / SEG as f32;
        let (nx, ny) = ((sd[j + 1] + PI / 2.0).cos(), (sd[j + 1] + PI / 2.0).sin());
        let bump = 0.075 * (1.0 - t) + 0.014;
        let mid = (
            (lx[j] + lx[j + 2]) * 0.5 - nx * bump,
            (ly[j] + ly[j + 2]) * 0.5 - ny * bump,
        );
        bez((lx[j], ly[j]), mid, (lx[j + 2], ly[j + 2]), th * 0.8);
    }

    // Spray: droplets flung up and ahead of the curl, thicker where they leave the
    // foam and thinning as they scatter downwind.
    for &(px, py, r) in &[
        (0.82f32, -0.79f32, 0.018f32),
        (0.97, -0.74, 0.022),
        (1.08, -0.61, 0.024),
        (1.15, -0.45, 0.020),
        (1.18, -0.28, 0.016),
    ] {
        draw_circle(cx + px * size, cy + py * size, (r * size).max(1.0), col);
    }

    // Engraved contour shading down the back slope: strokes laid parallel to the
    // surface on the shaded lower flank, the way an engraver darkens a hillside.
    bez((-0.90, 0.36), (-0.56, 0.14), (-0.24, -0.10), (th * 0.55).max(0.8));
    bez((-0.71, 0.38), (-0.40, 0.19), (-0.06, -0.05), (th * 0.55).max(0.8));
    bez((-0.50, 0.40), (-0.22, 0.26), (0.11, 0.05), (th * 0.5).max(0.8));
    bez((-0.27, 0.41), (-0.02, 0.32), (0.26, 0.16), (th * 0.5).max(0.8));

    // And down the hollow of the face, hugging the concave sweep under the curl.
    bez((0.42, -0.42), (0.42, -0.14), (0.52, 0.12), (th * 0.5).max(0.8));
    bez((0.49, -0.35), (0.49, -0.08), (0.61, 0.18), (th * 0.5).max(0.8));
    bez((0.55, -0.24), (0.57, -0.02), (0.70, 0.22), (th * 0.5).max(0.8));

    // Splash ticks where the face runs back into the sea, kin to the kraken's roots.
    for side in [-1.0f32, 1.0] {
        let a = u(0.90 + side * 0.045, WY + 0.005);
        let b = u(0.90 + side * 0.095, WY - 0.055);
        draw_line(a.0, a.1, b.0, b.1, (th * 0.5).max(0.8), col);
    }

    // Ripple dashes along the waterline either side of the swell, the stylised sea
    // of an old chart.
    for (r, &(rx, half)) in [(-1.22f32, 0.14f32), (1.14, 0.13)].iter().enumerate().map(|(r, v)| (r as u32, v)) {
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
