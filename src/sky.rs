//! The sky backdrop: a time-of-day gradient dome with a directional twilight
//! split that tracks the sun's bearing. Split out of `main` so the sailing loop
//! reads as orchestration rather than gradient maths.

use macroquad::prelude::*;

use crate::geometry::{clamp, wrap_angle};
use crate::{lerp3, palette, rgb, scene, smoothstep};

/// Paint the sky as a vertical three-stop gradient (top → mid → horizon), eased
/// toward the storm overcast by `storm`. `sky` is the clock's fair-weather gradient.
///
/// The sky is treated as a skybox locked to the sun's bearing: a warm "lit" dome
/// toward the sun and a cool, dark dome away from it, the split sharpening as the
/// sun nears the horizon. So at sunrise the eastern sky around the sun glows while
/// the west, the sides and the zenith stay night-dark; at sunset the red sits on
/// the sun's side and the opposite sky has already gone dark. The directional split
/// is gated to the low-sun `twilight` window (active whether the sun is just above
/// *or* just below the horizon) and weighted toward the horizon, so high-noon and
/// deep-night skies stay uniform. Because the bearing is taken relative to the
/// heading, the bright side stays pinned to the sun as the helm swings the view.
/// Built as one mesh, since macroquad has no built-in gradient.
pub fn draw_sky(
    view: &scene::SkyView,
    sky: [(f32, f32, f32); 3],
    storm: f32,
    m: f32,
    sun_az: f32,
    sun_alt: f32,
) {
    let scene::SkyView {
        heading,
        half_fov_h: half_fov,
        w,
        horizon,
    } = *view;
    // How fully night has fallen: the overcast darkens with it (an unlit night sky has
    // no sun behind the cloud), so a storm at night reads dark rather than daytime-grey.
    let base_night = palette::night_factor(sun_alt); // 0 by day, 1 once set
    let storm_sky = palette::storm_sky(base_night);
    // The two gradients blended between horizontally: the clock's "lit" sky and the
    // cool night sky stood in for the un-sunlit side. Both eased toward the overcast.
    let storm_blend = |g: [(f32, f32, f32); 3]| {
        [
            lerp3(g[0], storm_sky[0], storm),
            lerp3(g[1], storm_sky[1], storm),
            lerp3(g[2], storm_sky[2], storm),
        ]
    };
    let lit = storm_blend(sky);
    let dark = storm_blend(palette::fair_sky(palette::Daytime::Night));

    // Vertical three-stop sample (top → mid → horizon) at `t` in [0, 1].
    let grad = |g: &[(f32, f32, f32); 3], t: f32| {
        if t < 0.5 {
            lerp3(g[0], g[1], t * 2.0)
        } else {
            lerp3(g[1], g[2], (t - 0.5) * 2.0)
        }
    };

    // `base_night` (computed above) eases the whole sky to the night gradient as the
    // sun sinks past the horizon, so no warm tint lingers overhead once it's down.
    // The strength of the *directional* split: a bell centred on the sun sitting on
    // the horizon, so it rises as the sun approaches the sea line (from above on the
    // way down, from below on the way up) and fades both at high noon and at the dead
    // of night. This is what makes the warm side appear while the sun is still up —
    // the old code keyed the split off `base_night`, which is zero until the sun has
    // already set, so a sunrise lit every bearing equally.
    const TWILIGHT_WIDTH: f32 = 0.34;
    let twilight = (-(sun_alt / TWILIGHT_WIDTH).powi(2)).exp();
    // The sun's bearing across the view (relative to the heading).
    let rel_sun = wrap_angle(sun_az - heading);
    // Angular half-width of the warm glow around the sun's bearing.
    const GLOW_WIDTH: f32 = 0.85;

    // Backstop fill (covered by the mesh) so a hard camera tilt never bares the
    // cleared background past the gradient's edges.
    let back = lerp3(grad(&lit, 0.0), grad(&dark, 0.0), base_night);
    draw_rectangle(-m, -m, w + 2.0 * m, horizon + m, rgb(back));

    if twilight <= 0.001 {
        // No twilight split (high day, or deep night): a plain vertical gradient —
        // eased uniformly toward night by `base_night` — is enough.
        let strips = 96;
        let strip_h = horizon / strips as f32;
        for i in 0..strips {
            let t = i as f32 / (strips - 1) as f32;
            let y = i as f32 * strip_h;
            let c = lerp3(grad(&lit, t), grad(&dark, t), base_night);
            draw_rectangle(-m, y, w + 2.0 * m, strip_h + 1.0, rgb(c));
        }
        return;
    }

    // Directional gradient as a grid mesh: rows give the vertical gradient, columns
    // the sideways lit→dark blend by angle from the sun. Kept small enough that the
    // index count stays under macroquad's per-drawcall limit (max_indices = 5000;
    // 24×32×6 = 4608); the per-vertex colours interpolate smoothly across each quad.
    let cols = 24usize;
    let rows = 32usize;
    let x0 = -m;
    let x1 = w + m;
    let y0 = -m;
    let y1 = horizon;
    let mut vertices: Vec<Vertex> = Vec::with_capacity((cols + 1) * (rows + 1));
    for r in 0..=rows {
        let fy = r as f32 / rows as f32;
        let y = y0 + (y1 - y0) * fy;
        // Vertical gradient parameter: clamp the over-scan above y=0 to the top stop.
        let t = clamp(y / horizon, 0.0, 1.0);
        let lit_c = grad(&lit, t);
        let dark_c = grad(&dark, t);
        // The split is confined to the lower sky (the zenith stays uniform, tracking
        // `base_night` only) and fades out toward the very top.
        let horizon_band = smoothstep(0.30, 0.95, t);
        for c in 0..=cols {
            let fx = c as f32 / cols as f32;
            let x = x0 + (x1 - x0) * fx;
            // This column's bearing relative to the heading, and its angle from the
            // sun; the warm glow falls off as a soft bell around the sun's bearing.
            let rel_col = (x - w * 0.5) / (w * 0.5) * half_fov;
            let sep = wrap_angle(rel_col - rel_sun);
            let glow = (-(sep / GLOW_WIDTH).powi(2)).exp();
            // Two directional pushes, both scaled by the twilight strength and limited
            // to the lower sky:
            //   warm_keep — near the sun, hold the warm "lit" sky even after the sun
            //               has dipped (spares the afterglow from `base_night`);
            //   dark_push — away from the sun, pull toward the cool night sky even
            //               while the sun is still up, so the anti-solar sky and the
            //               sides darken at sunrise/sunset instead of brightening.
            let warm_keep = glow * horizon_band * twilight;
            let dark_push = (1.0 - glow) * horizon_band * twilight;
            let night_amt =
                clamp(base_night * (1.0 - warm_keep) + dark_push * (1.0 - base_night), 0.0, 1.0);
            let col = lerp3(lit_c, dark_c, night_amt);
            vertices.push(Vertex::new(x, y, 0.0, fx, fy, rgb(col)));
        }
    }
    let stride = (cols + 1) as u16;
    let mut indices: Vec<u16> = Vec::with_capacity(cols * rows * 6);
    for r in 0..rows as u16 {
        for c in 0..cols as u16 {
            let i0 = r * stride + c;
            let i1 = i0 + 1;
            let i2 = i0 + stride;
            let i3 = i2 + 1;
            indices.extend_from_slice(&[i0, i1, i2, i1, i3, i2]);
        }
    }
    draw_mesh(&Mesh {
        vertices,
        indices,
        texture: None,
    });
}
