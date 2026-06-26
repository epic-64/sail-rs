//! The reflection road for a port's harbour lights: after dusk the lit town on a
//! port island casts a single shimmering road of light down the water toward the
//! viewer, the way the moon's glitter road does. (The lamps themselves are tiny
//! per-house lights drawn on the island in [`crate::islands_render`]; this module
//! is only their pooled reflection on the sea.)
//!
//! The sun/moon "reflection" is not a drawn object: it is a per-facet Blinn-Phong
//! specular highlight baked into the wave mesh from one *global* directional light
//! (`ocean_renderer`), so nearer crests occlude it for free. A local light can't
//! ride that path, so the road is built instead as world-anchored glints that slot
//! into the **same wave-band march** the islands and salvage do: each sparkle sits
//! at its own sea distance, so the nearer wave bands paint over it just as they do a
//! floating crate, and the road sinks behind the swell rather than shining through
//! it. Purely decorative: it touches no world state and no determinism.

use macroquad::prelude::*;
use std::f32::consts::TAU;

use crate::celestial::Sky;
use crate::geometry::{clamp, wrap_angle, Vec2};
use crate::ocean;
use crate::ocean_renderer::WAVE_GAIN;
use crate::projection::{BASE_EYE, MAX_VIEW};
use crate::sailing::Kinematics;
use crate::scene::SceneView;
use crate::world::Island;

/// Warm-white colour of the pooled town reflection (a blend of the mixed lamps).
const ROAD_COL: (f32, f32, f32) = (255.0, 198.0, 142.0);
/// Glints strewn down one island's reflection road.
const GLINTS: usize = 44;
/// How far toward the viewer the glitter road reaches, as a fraction of the source's
/// distance (the rest of the way to the eye stays dark, as a real road peters out).
const ROAD_SPAN: f32 = 0.82;

/// One sparkle on a reflection road, sitting on the sea surface and slotted into the
/// wave-band march by distance. `phase`/`rate` drive its twinkle; `intensity` already
/// folds in its fade down the road.
pub struct Glint {
    pub pos: Vec2,
    intensity: f32,
    phase: f32,
    rate: f32,
}

/// How strongly the harbour lights burn at clock `sun_alt` (the sine of the sun's
/// altitude): dark by day, ramping on through dusk to full once the sun is well
/// down. 0 while the sun is up, 1 by the time it has sunk a little below the horizon.
/// Shared with [`crate::islands_render`], which lights the houses by the same clock.
pub fn dusk_glow(sun_alt: f32) -> f32 {
    clamp((0.06 - sun_alt) / 0.18, 0.0, 1.0)
}

/// Stable hash of a scalar to [0, 1) (the classic sin-fract trick), so each glint's
/// place and twinkle are fixed per island without an RNG or the banned `Math::random`.
#[inline]
fn hash01(n: f32) -> f32 {
    let s = (n * 12.9898).sin() * 43758.5453;
    s - s.floor()
}

/// Build the reflection-road glints for every visible port after dusk (one road per
/// island), sorted *farthest first* so the wave-band march can slot each in at its
/// own depth. `kin` is the view kinematics (carries the look-astern heading); `t`
/// drives the flicker.
pub fn build(
    islands: &[Island],
    kin: &Kinematics,
    sky: &Sky,
    t: f32,
    half_fov_h_view: f32,
) -> Vec<Glint> {
    let glow = dusk_glow(sky.sun_alt);
    let mut out: Vec<Glint> = Vec::new();
    if glow <= 0.01 {
        return out;
    }
    for isle in islands {
        if !isle.is_port {
            continue;
        }
        let d = kin.pos.distance_to(isle.pos);
        if d > MAX_VIEW || d < isle.radius {
            continue;
        }
        let rel = wrap_angle(kin.pos.bearing_to(isle.pos) - kin.heading_rad);
        if rel.abs() > half_fov_h_view * 1.15 {
            continue;
        }
        // Hold the road bright across the field, easing it out only at the far edge so
        // a port on the horizon still shows a glimmer rather than popping off.
        let fade = clamp((MAX_VIEW - d) / 1500.0, 0.0, 1.0);
        // A gentle flicker so the whole town's pool wavers on the water.
        let flick = 0.85 + 0.15 * (t * 2.3 + isle.id as f32 * 1.7).sin();
        let a = glow * fade * flick;
        if a <= 0.01 {
            continue;
        }

        // Unit vector from the island toward the ship; the road pools off the shore
        // point one radius out along it and runs back toward the viewer.
        let to_ship = kin.pos - isle.pos;
        let dl = to_ship.length().max(1e-3);
        let dirn = Vec2::new(to_ship.x / dl, to_ship.y / dl);
        let toward_isle = Vec2::new(-dirn.x, -dirn.y);
        let beam = Vec2::new(dirn.y, -dirn.x); // perpendicular, for the road's spread
        let d_shore = (d - isle.radius).max(1.0);

        let seedb = isle.id as f32;
        for i in 0..GLINTS {
            let fi = i as f32;
            let r1 = hash01(seedb * 1.7 + fi * 3.13);
            let r2 = hash01(seedb * 2.3 + fi * 7.91);
            let r3 = hash01(seedb * 0.7 + fi * 1.27);
            let f = (fi + r1) / GLINTS as f32; // 0 at the source, 1 toward the viewer
            let dg = d_shore * (1.0 - f * ROAD_SPAN); // distance from the ship
            let spread = d_shore * 0.02 * (0.3 + 1.6 * f); // road fans toward the eye
            let lat = (r2 * 2.0 - 1.0) * spread;
            let pos = kin.pos + toward_isle * dg + beam * lat;
            out.push(Glint {
                pos,
                intensity: a * (1.0 - 0.55 * f) * (0.7 + 0.5 * r3),
                phase: r1 * TAU,
                rate: 2.0 + r3 * 4.5,
            });
        }
    }
    out.sort_by(|a, b| {
        kin.pos
            .distance_to(b.pos)
            .partial_cmp(&kin.pos.distance_to(a.pos))
            .unwrap()
    });
    out
}

/// Draw one glint, projected through the same cylindrical map the waves use so it sits
/// on the local wave surface (and the swell carries it). A tiny horizontal flare
/// (centre bright, four tips fading to transparent) that twinkles sharply on and off
/// the way the sun's specular highlights flash on the wave facets. Called from inside
/// the wave-band march (see [`crate::ocean_renderer`]) so nearer bands paint over it.
pub fn draw(g: &Glint, view: &SceneView) {
    let SceneView {
        kin, t, sea, heave, horizon, px_per_rad, px_per_rad_h, half_fov_h_view, w, h, ..
    } = *view;
    let d = kin.pos.distance_to(g.pos);
    if !(1.0..=MAX_VIEW).contains(&d) {
        return;
    }
    let phi = wrap_angle(kin.pos.bearing_to(g.pos) - kin.heading_rad);
    if phi.abs() > half_fov_h_view * 1.15 {
        return;
    }
    // Sharp, mostly-off twinkle: two beat sines cubed, like the star streaks.
    let beat = (t * g.rate + g.phase).sin() * (t * g.rate * 0.6 + g.phase * 2.1).sin();
    let spark = (0.5 + 0.5 * beat).powi(3);
    let a = clamp(g.intensity * spark * 1.4, 0.0, 1.0);
    if a <= 0.01 {
        return;
    }
    let sx = w * 0.5 + phi * px_per_rad_h;
    let wave = ocean::height(g.pos, t, sea);
    let foot_disp = (wave - heave) * WAVE_GAIN;
    let cphi = phi.cos();
    let y = horizon + ((BASE_EYE - foot_disp) * cphi / d).atan() * px_per_rad;

    let sz = clamp(h * 2.6 / d, 1.2, h * 0.011) * (0.6 + 0.9 * spark);
    let hl = sz * 1.7; // horizontal flare half-length
    let ht = sz * 0.6; // vertical half-thickness
    // The hotter the flash, the whiter the flare, so it reads as a glint not a smear.
    let white = 0.5 * spark;
    let (cr, cg, cb) = ROAD_COL;
    let hot = Color::new(
        (cr + (255.0 - cr) * white) / 255.0,
        (cg + (255.0 - cg) * white) / 255.0,
        (cb + (255.0 - cb) * white) / 255.0,
        a,
    );
    let edge = Color::new(cr / 255.0, cg / 255.0, cb / 255.0, 0.0);
    let vertices = vec![
        Vertex::new(sx, y, 0.0, 0.0, 0.0, hot),       // 0 centre
        Vertex::new(sx - hl, y, 0.0, 0.0, 0.0, edge), // 1 left
        Vertex::new(sx, y - ht, 0.0, 0.0, 0.0, edge), // 2 top
        Vertex::new(sx + hl, y, 0.0, 0.0, 0.0, edge), // 3 right
        Vertex::new(sx, y + ht, 0.0, 0.0, 0.0, edge), // 4 bottom
    ];
    let indices = vec![0, 1, 2, 0, 2, 3, 0, 3, 4, 0, 4, 1];
    draw_mesh(&Mesh {
        vertices,
        indices,
        texture: None,
    });
}
