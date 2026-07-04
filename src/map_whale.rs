//! The whale flourish for the captain's-log world map: the companion sea-beast to the
//! kraken, inked into the chart's next-roomiest gap (see [`crate::minimap::render_world`]).

use macroquad::prelude::*;

use crate::minimap::hash01;

/// Ink a small blue whale in the manner of an antique woodcut: a long, low-slung body
/// facing left, drawn as a wobbled outline that swells on the shaded flank, with the
/// long sweep of the jaw, engraved throat grooves, a slender flipper, the little
/// falcate dorsal fin set far astern, broad notched flukes, and the tall single
/// column of the blow. Centred at (`cx`,`cy`) and sized to `size`; `s` is the map's
/// glyph scale (for stroke width); `col` the chart ink.
pub fn draw_whale(cx: f32, cy: f32, size: f32, s: f32, col: Color) {
    use std::f32::consts::{FRAC_1_SQRT_2, PI};
    let th = (1.3 * s).max(1.0);
    // The same light as falls on the kraken (upper right), so the pair reads as one
    // engraver's hand: strokes thicken toward the shade (screen down-left).
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

    // The body: a closed Catmull-Rom outline through hand-placed points (unit space,
    // whale facing left, y down), sampled into short segments so the stroke can
    // wobble like a carved line and swell on the shaded side.
    const PTS: [(f32, f32); 16] = [
        (-1.15, 0.00),  // snout tip
        (-1.02, -0.16), // rostrum
        (-0.72, -0.28), // head top (the blow rises from here)
        (-0.30, -0.35), // fore-back
        (0.10, -0.36),  // mid-back peak
        (0.46, -0.30),  // rear back (dorsal fin sits here)
        (0.70, -0.20),  // saddle before the tail lifts
        (0.84, -0.24),  // tail stock curling up
        (0.93, -0.33),  // fluke root, raised above the back line
        (0.97, -0.22),  // stock, aft edge
        (0.82, -0.06),
        (0.55, 0.06),
        (0.20, 0.18),  // rear belly
        (-0.30, 0.28), // mid belly
        (-0.80, 0.26), // chin / throat pouch
        (-1.12, 0.10), // jaw tip
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
        let wob = 0.012 * (2.0 * hash01(k as u32 * 37 + 5) - 1.0);
        ox[k] += dy / len * wob;
        oy[k] += -dx / len * wob;
    }
    for k in 0..M {
        let n = (k + 1) % M;
        let (dx, dy) = (ox[n] - ox[k], oy[n] - oy[k]);
        let len = (dx * dx + dy * dy).sqrt().max(1e-6);
        // Outward normal for this traversal order is (dy, -dx).
        let shade = ((dy * shx - dx * shy) / len).max(0.0);
        let a = u(ox[k], oy[k]);
        let b = u(ox[n], oy[n]);
        draw_line(a.0, a.1, b.0, b.1, th * (0.7 + 0.9 * shade), col);
    }

    // The flukes: broad blades fanned from the raised fluke root, notched between;
    // the lower blade sits in the shade so it carries the heavier stroke.
    bez((0.87, -0.30), (0.90, -0.52), (1.00, -0.68), th * 0.9);
    bez((1.00, -0.68), (1.04, -0.50), (1.12, -0.40), th * 0.8);
    bez((0.98, -0.24), (1.20, -0.20), (1.36, -0.32), th * 1.2);
    bez((1.36, -0.32), (1.22, -0.38), (1.12, -0.40), th * 1.0);

    // The dorsal fin, small and falcate, set far astern the way a blue whale's is.
    bez((0.33, -0.315), (0.39, -0.44), (0.46, -0.42), th * 0.9);
    bez((0.46, -0.42), (0.44, -0.35), (0.51, -0.28), th * 0.9);

    // The jaw: one long gape line from the snout sweeping aft along the lower body,
    // with a small inked eye just above its corner.
    bez((-1.10, 0.065), (-0.80, 0.19), (-0.48, 0.14), th * 0.9);
    draw_circle(cx - 0.42 * size, cy + 0.08 * size, (0.020 * size).max(1.0), col);

    // Throat grooves: the long ventral pleats, engraved as curves parallel to the
    // gape, fanning back across the pouch and shortening as they near the belly.
    bez((-0.98, 0.145), (-0.66, 0.215), (-0.34, 0.205), (th * 0.5).max(0.8));
    bez((-0.92, 0.19), (-0.64, 0.25), (-0.38, 0.235), (th * 0.5).max(0.8));
    bez((-0.84, 0.235), (-0.62, 0.275), (-0.44, 0.26), (th * 0.5).max(0.8));

    // The flipper: a short broad blade trailing down and aft, set abaft the pouch so
    // it doesn't tangle with the pleats.
    bez((-0.30, 0.215), (-0.18, 0.27), (-0.04, 0.38), th * 0.9);
    bez((-0.16, 0.245), (-0.10, 0.31), (-0.04, 0.38), th * 0.8);

    // Engraved shading on the after flank: arcs running parallel to the belly line,
    // the way an engraver lays latitude lines under a hull's shadowed quarter.
    for j in 0..3 {
        let d = j as f32 * 0.055;
        bez(
            (0.12 + 0.05 * j as f32, 0.14 - d),
            (0.38, 0.09 - d),
            (0.58 - 0.03 * j as f32, -0.06 - 0.9 * d),
            (th * 0.55).max(0.8),
        );
    }

    // The blow: the tall column of a blue whale's spout, jets that rise near-vertical
    // then flare outward at the head, crowned with flung droplets.
    let (bx, by) = (-0.72, -0.28);
    for &(dx, hgt) in &[(-0.16f32, 0.55f32), (0.0, 0.72), (0.14, 0.58)] {
        bez((bx, by), (bx - dx * 0.15, by - hgt * 0.5), (bx + dx, by - hgt), (th * 0.85).max(1.0));
    }
    for &(dx, dy, r) in
        &[(-0.24f32, -0.62f32, 0.020f32), (-0.04, -0.80, 0.024), (0.10, -0.76, 0.018), (0.22, -0.63, 0.020)]
    {
        draw_circle(cx + (bx + dx) * size, cy + (by + dy) * size, (r * size).max(1.0), col);
    }

    // Ripple dashes at the waterline fore and aft, the stylised sea of an old chart.
    for (r, &(rx, ry, half)) in [(-0.95f32, 0.33f32, 0.18f32), (0.62, 0.30, 0.15)]
        .iter()
        .enumerate()
        .map(|(r, v)| (r as u32, v))
    {
        let hw = half * size;
        const RSEG: usize = 6;
        let mut prev = u(rx - half, ry);
        for j in 1..=RSEG {
            let t = j as f32 / RSEG as f32;
            let x = cx + rx * size - hw + 2.0 * hw * t;
            let y = cy + ry * size - (t * PI).sin() * size * 0.05 * (1.0 + 0.4 * hash01(r * 7 + 1));
            draw_line(prev.0, prev.1, x, y, (th * 0.5).max(0.8), col);
            prev = (x, y);
        }
    }
}
