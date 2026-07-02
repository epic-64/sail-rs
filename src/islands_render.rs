//! Low-poly island rendering. Replaces the original SVG billboards (which the
//! game's author disliked, and which let the waves clip through) with cohesive
//! flat-shaded geometry that matches the faceted wave mesh.
//!
//! Each island is a floor disc lying on the sea (a foreshortened ellipse, ported
//! from `IslandFloorRenderer`) plus a faceted landmass body. The body is no
//! longer a single circular cone: it is a polar **heightfield** ([`IsleTerrain`])
//! whose coastline lumps in and out (lobes, inlets, peninsulas — a more complex
//! shape than one circle) and whose surface rises into one or several **hills**
//! (a sum of Gaussian peaks), flat-shaded against the sun in world space.
//! Mechanics are unchanged: islands are placed and sized by `WorldGen`, sit on
//! the waterline at their own distance (so they parallax as you sail around), and
//! ride the swell by the same heave the sea uses. The grounding circle is still
//! `isle.radius`; the lumpy coast stays inside it, so collision is unaffected.
//!
//! Correct wave occlusion is handled by the caller ([`OceanRenderer::render`]),
//! which draws each island *between* the wave bands by distance — so a near crest
//! rolls in front of a far island's base while its summit stands clear. Within an
//! island the mound triangles are depth-sorted, and scenery on the far slopes is
//! culled where a hill stands between it and the eye.

use macroquad::prelude::*;

use crate::geometry::{clamp, wrap_angle, Vec2};
use crate::isle_features::{FeatureKind, IsleFeature};
use crate::isle_terrain::IsleTerrain;
use crate::projection::{BASE_EYE, EYE_HEIGHT, MAX_VIEW, SHORE_LIFT};
use crate::sailing::Kinematics;
use crate::world::{Island, IsleKind};

use std::f32::consts::{FRAC_PI_2, TAU};

const FLOOR_SEG: usize = 80; // floor ellipse: smooth
const MOUND_SEG: usize = 80; // landmass body: angular facets around the coast
const SIDE_CULL: f32 = 1.6; // how far off-axis an isle may sit before it's skipped
const AMBIENT: f32 = 0.45; // floor of the directional shading

const SHADOW: [f32; 3] = [8.0, 40.0, 30.0];

// Per-island scenery is dense (scores of billboards each), and the costly part is
// the per-feature occlusion raymarch. Distant isles are small on screen, so we fade
// their features out and stop drawing them entirely past the end: between these two
// ranges (m) feature alpha lerps 1 → 0, and beyond `FEATURE_FADE_END` the whole
// feature pass is skipped. The island body itself keeps rendering to `MAX_VIEW`.
const FEATURE_FADE_START: f32 = 2000.0;
const FEATURE_FADE_END: f32 = 3500.0;

/// Past ±90° off the heading a world point sits *behind* the camera, where the
/// cylindrical [`project`] map folds across the ±π bearing seam: a triangle with
/// one corner just left of dead-astern and another just right of it smears clear
/// across the view as a stray "slice of land in the open sea". The live FOV is
/// only ~±68°, so refusing to draw any triangle that touches a behind-camera
/// vertex removes nothing on-screen — it just stops the off-screen rear of an
/// isle you've sailed past (left abeam or astern) from wrapping into the middle
/// of the ocean. Every surviving corner is within ±90°, so no two of them can
/// straddle the ±π seam.
#[inline]
fn behind_camera(wp: Vec2, kin: &Kinematics) -> bool {
    wrap_angle(kin.pos.bearing_to(wp) - kin.heading_rad).abs() > FRAC_PI_2
}

/// Camera/view parameters shared with the wave renderer for one frame.
pub struct IslandView {
    pub w: f32,
    pub horizon: f32,
    pub px_per_rad: f32,
    pub px_per_rad_h: f32,
    pub half_fov_h_view: f32,
    pub eye_rise: f32,
    /// World-space unit vector pointing toward the active light (x, y on chart;
    /// z up) — the sun by day, the moon by night.
    pub sun: (f32, f32, f32),
    /// The directional ("key") light colour, day/night brightness already folded
    /// in: warm-white at noon, blood-orange at dusk, dim cool-blue under the moon.
    /// Lights the sun-facing portion of every facet ([`IslandView::lit`]).
    pub key: (f32, f32, f32),
    /// The ambient sky-fill colour with brightness: the hue the dome washes over
    /// the shadowed faces (cool blue by day, purple-orange at dusk). Sets the floor
    /// of every facet's shading, so the land takes the hour's colour rather than
    /// only darkening into a grey silhouette.
    pub ambient: (f32, f32, f32),
    /// The low sun's warm hue (normalised so its brightest channel is 1) and how
    /// hard to pull the lit land toward it this frame (0 by day and under the moon,
    /// rising at dawn/dusk). See [`IslandView::warm_shift`] and `WARM_SHIFT`.
    pub warm: (f32, f32, f32),
    pub warm_amt: f32,
    /// How brightly the houses' windows burn (0 by day, 1 once the sun is well
    /// down): the dusk ramp from [`crate::port_lights::dusk_glow`]. Lights only the
    /// settlement on a port island.
    pub lamp: f32,
    /// Animation clock, for the gentle window-light twinkle.
    pub t: f32,
}

// --- Terrain (renderer-side query) -------------------------------------------
// The island's coastline + height model lives in [`crate::isle_terrain`], shared
// with collision and feature placement. Here we add only the one query that is
// renderer-specific because it leans on the camera projection.

impl IsleTerrain {
    /// Is a feature standing at `wp` (foot elevation `foot_z`) hidden behind the
    /// island's own terrain from the camera at `kin.pos`? Marches a few samples
    /// along the eye->feature ray (only the part nearer than the feature) and asks
    /// whether the terrain there projects *above* the feature's foot on screen.
    /// Works for flat and hilly islands alike: a hummock can hide scenery just
    /// behind it.
    fn occluded(&self, wp: Vec2, foot_z: f32, kin: &Kinematics, v: &IslandView) -> bool {
        let foot_y = project(wp, foot_z, false, kin, v).1;
        let to_cam = kin.pos - wp;
        const N: usize = 5;
        for s in 1..=N {
            let frac = s as f32 / (N as f32 + 1.0);
            let sp = wp + to_cam * frac;
            let z = self.elevation_at(sp);
            if z <= foot_z + 1.0 {
                continue;
            }
            let ty = project(sp, z, false, kin, v).1;
            // Smaller screen-y is higher up: terrain crests above the foot occludes.
            if ty < foot_y - 1.5 {
                return true;
            }
        }
        false
    }
}

/// Sand-rim and foliage-interior colours per archetype (matches the original).
fn palette(isle: &Island) -> ([f32; 3], [f32; 3]) {
    match isle.terrain {
        IsleKind::Volcanic => ([78.0, 72.0, 68.0], [58.0, 52.0, 48.0]),
        IsleKind::Rocky => ([150.0, 146.0, 138.0], [96.0, 92.0, 84.0]),
        IsleKind::Jungle => ([232.0, 217.0, 168.0], [31.0, 104.0, 55.0]),
        IsleKind::Green => ([232.0, 217.0, 168.0], [47.0, 143.0, 78.0]),
    }
}

// --- Day/night island lighting -----------------------------------------------
// The land is lit by two coloured lights pulled from the same time-of-day palette
// the sea uses: a *key* (the sun by day, the moon by night) whose warmth swings
// from white noon through blood-orange dusk to cool moonlight, and an *ambient*
// sky fill that washes the shadowed faces with the colour of the dome overhead.
// Each reference hue is normalised to pure chroma then eased back toward neutral by
// these fractions, so the isles take the hour's colour without going as lurid as
// the water and sky they stand against. Raise them for a more saturated land,
// lower for a more muted one; 0 returns the old grey-only day/night dimming.
const KEY_TINT: f32 = 0.9;
const AMBIENT_TINT: f32 = 0.8;
// Sunrise/sunset reddening on top of the tinted key light. Tinting the light only
// *multiplies* the surface, so a green hillside stays green at dusk (it reflects little
// red however red the sun is) and the isles never really catch fire. This is a
// luminance-preserving hue *recolour*: it swings each facet's hue right onto the low
// sun's warm colour at its own brightness, so the whole island reads sunset-red the way
// the sea palette flips fully red, keeping only light/dark for form. Scaled by how warm
// the hour's light actually is, so it does nothing at noon or under the moon. `WARM_MAX`
// is the full-tilt strength at peak dusk (1 = the facet's hue is entirely the sun's);
// raise `WARM_GAIN` to reach full tilt sooner, drop `WARM_MAX` for a softer sunset.
// `WARM_KNEE` is the warmth below which the light counts as white and reddens nothing
// (the noon sun is a warm-*white*, so a plain gain would tint the land all day) — only
// the low sun, redder than the knee, swings the land toward its colour.
const WARM_KNEE: f32 = 0.22;
const WARM_GAIN: f32 = 1.45;
const WARM_MAX: f32 = 1.0;
// The low sun's own hue is a blood-*orange* (its green channel is high), so recolouring
// straight onto it leaves the land orange/yellow rather than red. This deepens the
// recolour target toward pure red by draining that share of its green and blue, so a
// house burns red at dusk, not amber. 0 = the sun's own orange, 1 = pure red.
const WARM_RED: f32 = 0.6;
// The share of the recolour a fully shadowed facet still takes: the whole landmass goes
// red, not just the sun-facing side (as the entire sea reddens, not only its lit crests),
// with the sunlit side taking all of it. 1 = flat uniform hue; lower keeps the shadowed
// faces a touch truer to their daylight colour for a little more relief.
const WARM_FLOOR: f32 = 0.7;

/// Normalise an RGB triple to pure chroma (mean channel = 1), then ease it back
/// toward neutral grey: `sat` scales how strongly the resulting hue reads (0 = grey,
/// 1 = full chroma).
fn tint(c: [f32; 3], sat: f32) -> (f32, f32, f32) {
    let mean = ((c[0] + c[1] + c[2]) / 3.0).max(1e-3);
    let n = |x: f32| 1.0 + (x / mean - 1.0) * sat;
    (n(c[0]), n(c[1]), n(c[2]))
}

/// The isles' key and ambient light colours for one frame. `brightness` is the
/// overall day/night level (1 at noon, ~0.35 under the moon); `sun` is the key
/// light's reference hue (the sea palette's warmth channel) and `sky` the ambient
/// fill's (the sky dome). Both returned colours already fold in `brightness`, and
/// reduce to neutral grey × `brightness` when their hues are colourless, so a flat
/// white light leaves the old behaviour untouched.
pub fn island_light(
    brightness: f32,
    sun: [f32; 3],
    sky: [f32; 3],
) -> ((f32, f32, f32), (f32, f32, f32)) {
    let scale = |t: (f32, f32, f32)| (t.0 * brightness, t.1 * brightness, t.2 * brightness);
    (scale(tint(sun, KEY_TINT)), scale(tint(sky, AMBIENT_TINT)))
}

/// The warm sunset colour and recolour strength for one frame, from the key light's
/// reference hue (the sea palette's warmth channel). Returns the hue normalised to
/// unit luminance (so recolouring onto it neither brightens nor darkens the land), and
/// a `warm_amt` in [0, `WARM_MAX`] that rises with how far the light leans red over
/// blue: 0 for a white noon sun or the cool moon, full at the blood-orange dusk sun.
/// Feeds [`IslandView::warm_shift`].
pub fn warm_light(sun: [f32; 3]) -> ((f32, f32, f32), f32) {
    let warmth = ((sun[0] - sun[2]) / 255.0).clamp(0.0, 1.0);
    // Deepen the sun's orange toward pure red for the recolour target (drain green/blue),
    // then normalise to unit luminance so the swing recolours without brightening.
    let r = sun[0];
    let g = sun[1] * (1.0 - WARM_RED);
    let b = sun[2] * (1.0 - WARM_RED);
    let luma = (r * 0.30 + g * 0.59 + b * 0.11).max(1.0);
    let t = ((warmth - WARM_KNEE) / (1.0 - WARM_KNEE)).clamp(0.0, 1.0);
    let amt = (t * WARM_GAIN).min(WARM_MAX);
    ((r / luma, g / luma, b / luma), amt)
}

impl IslandView {
    /// The light multiplier on a facet whose Lambert term is `diff` (0 in shadow,
    /// 1 fully sunlit): the ambient sky fill plus the key light scaled by the
    /// facet's exposure. With neutral (grey) lights this is exactly the old
    /// `brightness × (AMBIENT + (1 - AMBIENT) · diff)` shading.
    #[inline]
    fn lit(&self, diff: f32) -> (f32, f32, f32) {
        let key = (1.0 - AMBIENT) * diff;
        (
            self.ambient.0 * AMBIENT + self.key.0 * key,
            self.ambient.1 * AMBIENT + self.key.1 * key,
            self.ambient.2 * AMBIENT + self.key.2 * key,
        )
    }

    /// Recolour an already-lit colour onto the low sun's warm hue at its own
    /// brightness (see `WARM_GAIN`/`WARM_FLOOR`): a luminance-preserving hue swing.
    /// The sunlit side (`diff` = 1) takes the full recolour, shadowed faces still take
    /// `WARM_FLOOR` of it so the whole island goes red, not just its lit facets. A
    /// no-op away from dawn/dusk (`warm_amt` = 0).
    #[inline]
    fn warm_shift(&self, c: (f32, f32, f32), diff: f32) -> (f32, f32, f32) {
        let w = self.warm_amt * (WARM_FLOOR + (1.0 - WARM_FLOOR) * diff);
        if w <= 0.0 {
            return c;
        }
        let l = c.0 * 0.30 + c.1 * 0.59 + c.2 * 0.11;
        (
            c.0 + (self.warm.0 * l - c.0) * w,
            c.1 + (self.warm.1 * l - c.1) * w,
            c.2 + (self.warm.2 * l - c.2) * w,
        )
    }

    /// A surface colour lit for this frame: the base albedo through the key/ambient
    /// multiply ([`lit`]/[`flat`]), then warmed toward the sunset hue
    /// ([`warm_shift`]). `diff` is the facet's Lambert term (1 for flat scenery).
    #[inline]
    fn shade(&self, base: [f32; 3], diff: f32, alpha: f32) -> Color {
        let m = self.lit(diff);
        let c = (base[0] / 255.0 * m.0, base[1] / 255.0 * m.1, base[2] / 255.0 * m.2);
        let c = self.warm_shift(c, diff);
        Color::new(c.0, c.1, c.2, alpha)
    }
}

/// Project a world point at elevation `z` (m). When `waterline`, use the low
/// waterline eye (so the shore matches the floor disc and the sea); otherwise the
/// real eye height (so summits sit where the billboards used to).
#[inline]
fn project(wp: Vec2, z: f32, waterline: bool, kin: &Kinematics, v: &IslandView) -> (f32, f32) {
    let d = kin.pos.distance_to(wp).max(1.0);
    let rp = wrap_angle(kin.pos.bearing_to(wp) - kin.heading_rad);
    let sx = v.w * 0.5 + rp * v.px_per_rad_h;
    let sy = if waterline {
        v.horizon + (((BASE_EYE + v.eye_rise) / d).atan() - (SHORE_LIFT / d).atan()) * v.px_per_rad
    } else {
        v.horizon - ((z - EYE_HEIGHT - v.eye_rise) / d).atan() * v.px_per_rad
    };
    (sx, sy)
}

/// Fill a closed screen polygon (triangle fan from vertex 0), skipping any fan
/// triangle that touches a behind-camera vertex (`front[k] == false`) so the
/// rear of the ring never wraps across the view (see [`behind_camera`]).
fn fill_poly(xs: &[f32], ys: &[f32], front: &[bool], color: Color) {
    for i in 1..xs.len() - 1 {
        if !(front[0] && front[i] && front[i + 1]) {
            continue;
        }
        draw_triangle(
            vec2(xs[0], ys[0]),
            vec2(xs[i], ys[i]),
            vec2(xs[i + 1], ys[i + 1]),
            color,
        );
    }
}

/// The raw Lambert diffuse term (0 in shadow, 1 face-on to the light) of a triangle,
/// from its world-space (x, y, z) corners against the sun. The ambient floor and the
/// light's colour are applied later by [`IslandView::lit`].
fn diffuse(a: (f32, f32, f32), b: (f32, f32, f32), c: (f32, f32, f32), v: &IslandView) -> f32 {
    let (ux, uy, uz) = (b.0 - a.0, b.1 - a.1, b.2 - a.2);
    let (vx, vy, vz) = (c.0 - a.0, c.1 - a.1, c.2 - a.2);
    let (mut nx, mut ny, mut nz) = (uy * vz - uz * vy, uz * vx - ux * vz, ux * vy - uy * vx);
    if nz < 0.0 {
        nx = -nx;
        ny = -ny;
        nz = -nz;
    }
    let len = (nx * nx + ny * ny + nz * nz).sqrt().max(1e-6);
    ((nx * v.sun.0 + ny * v.sun.1 + nz * v.sun.2) / len).max(0.0)
}

/// One ready-to-draw, depth-keyed mound triangle. Screen points are kept as plain
/// (x, y) tuples (the name `Vec2` here is our world-chart vector, not glam's).
struct Tri {
    key: f32, // distance from camera to world centroid (sort key, far → near)
    p: [(f32, f32); 3],
    color: Color,
}

/// Draw one island (floor disc + faceted heightfield + features), or nothing if
/// out of view.
pub fn paint_island(isle: &Island, features: &[IsleFeature], kin: &Kinematics, v: &IslandView) {
    let dist = kin.pos.distance_to(isle.pos);
    let rel = wrap_angle(kin.pos.bearing_to(isle.pos) - kin.heading_rad);
    // Skip isles faded out or off to the side. The off-axis limit is widened by the
    // isle's angular half-width (`asin(radius/d)`) so a large isle sailed alongside
    // (its near shore filling the view while its centre sits abeam) isn't culled the
    // moment its centre clears the FOV.
    let ang_r = (isle.radius / dist).min(1.0).asin();
    if dist > MAX_VIEW || rel.abs() > v.half_fov_h_view * SIDE_CULL + ang_r {
        return;
    }
    let (sand, foliage) = palette(isle);
    let terrain = IsleTerrain::for_island(isle);
    // The one distance we must skip is the camera actually *on* the land, where the
    // coast ring wraps around the eye and the projection degenerates. We now ground
    // against the lumpy shore, so the hull can sail deep into a bay (well inside the
    // plain `radius`) and the isle must keep drawing there; only being inside the
    // coastline itself is degenerate. (Collision keeps us off it, so this rarely fires.)
    if terrain.on_land(kin.pos, 0.0) {
        return;
    }
    let alpha = clamp((1.0 - dist / MAX_VIEW) * 1.5, 0.0, 1.0);

    // --- Floor disc: shadow + sand rim, following the lumpy coast -------------
    let mut xs = [0.0f32; FLOOR_SEG];
    let mut ys = [0.0f32; FLOOR_SEG];
    let mut front = [true; FLOOR_SEG];
    let mut floor_ring = |scale: f32, color: Color| {
        for i in 0..FLOOR_SEG {
            let a = i as f32 / FLOOR_SEG as f32 * TAU;
            let r = terrain.coast_radius(a) * scale;
            let wp = Vec2::new(isle.pos.x + a.cos() * r, isle.pos.y + a.sin() * r);
            let (sx, sy) = project(wp, 0.0, true, kin, v);
            xs[i] = sx;
            ys[i] = sy;
            front[i] = !behind_camera(wp, kin);
        }
        fill_poly(&xs, &ys, &front, color);
    };
    floor_ring(1.10, v.shade(SHADOW, 1.0, alpha * 0.45));
    floor_ring(1.0, v.shade(sand, 1.0, alpha));

    // --- Faceted heightfield body --------------------------------------------
    // Build a polar grid (rings × segments) of world (x, y, z) + screen points,
    // then triangulate the strips and depth-sort so near facets overlay far ones
    // without a depth buffer — robust even with several hills.
    let levels = terrain.rings + 1; // level 0 = centre, level `rings` = coast
    let mut wpz = vec![[(0.0f32, 0.0f32, 0.0f32); MOUND_SEG]; levels];
    let mut scr = vec![[(0.0f32, 0.0f32); MOUND_SEG]; levels];
    // Which grid vertices sit in front of the camera: triangles touching a
    // behind-camera corner are skipped below so the rear of a passed isle can't
    // wrap across the view (see [`behind_camera`]).
    let mut front = vec![[true; MOUND_SEG]; levels];
    for lvl in 0..levels {
        let t = lvl as f32 / terrain.rings as f32;
        for i in 0..MOUND_SEG {
            let a = i as f32 / MOUND_SEG as f32 * TAU;
            let r = terrain.coast_radius(a) * t;
            let wp = Vec2::new(isle.pos.x + a.cos() * r, isle.pos.y + a.sin() * r);
            // Pin the outermost ring to sea level so the shore meets the floor disc.
            let z = if lvl == levels - 1 {
                0.0
            } else {
                terrain.elevation_at(wp)
            };
            wpz[lvl][i] = (wp.x, wp.y, z);
            scr[lvl][i] = project(wp, z, z < 0.5, kin, v);
            front[lvl][i] = !behind_camera(wp, kin);
        }
    }

    // Sand-vs-foliage is decided per concentric ring, not per triangle. Picking
    // the colour from each triangle's own centroid makes the boundary zig-zag
    // along facet edges (the hard-edged green/beige sawtooth), because the height
    // field wiggles above and below `beach` within a single ring of triangles.
    // Averaging each ring's elevation around the whole island instead puts the
    // boundary on a clean ring loop (which still follows the lumpy coast), so a
    // whole strip is one colour and no facets straddle the shoreline.
    let mut ring_z = vec![0.0f32; levels];
    for lvl in 0..levels {
        let mut s = 0.0;
        for i in 0..MOUND_SEG {
            s += wpz[lvl][i].2;
        }
        ring_z[lvl] = s / MOUND_SEG as f32;
    }

    let mut tris: Vec<Tri> = Vec::with_capacity(terrain.rings * MOUND_SEG * 2);
    let mut push = |base: [f32; 3],
                    a: (f32, f32, f32), b: (f32, f32, f32), c: (f32, f32, f32),
                    pa: (f32, f32), pb: (f32, f32), pc: (f32, f32)| {
        let diff = diffuse(a, b, c, v);
        let cx = (a.0 + b.0 + c.0) / 3.0;
        let cy = (a.1 + b.1 + c.1) / 3.0;
        tris.push(Tri {
            key: kin.pos.distance_to(Vec2::new(cx, cy)),
            p: [pa, pb, pc],
            color: v.shade(base, diff, alpha),
        });
    };
    for lvl in 0..levels - 1 {
        // One colour for the whole strip, from the mean height across its two rings.
        let mid_z = (ring_z[lvl] + ring_z[lvl + 1]) * 0.5;
        let base = if mid_z < terrain.beach { sand } else { foliage };
        for i in 0..MOUND_SEG {
            let j = (i + 1) % MOUND_SEG;
            let a0 = wpz[lvl][i];
            let b0 = wpz[lvl][j];
            let a1 = wpz[lvl + 1][i];
            let b1 = wpz[lvl + 1][j];
            // Drop any facet with a corner behind the camera — see `front` above.
            if front[lvl][i] && front[lvl][j] && front[lvl + 1][j] {
                push(base, a0, b0, b1, scr[lvl][i], scr[lvl][j], scr[lvl + 1][j]);
            }
            if front[lvl][i] && front[lvl + 1][j] && front[lvl + 1][i] {
                push(base, a0, b1, a1, scr[lvl][i], scr[lvl + 1][j], scr[lvl + 1][i]);
            }
        }
    }
    tris.sort_by(|x, y| y.key.partial_cmp(&x.key).unwrap());
    for tri in &tris {
        draw_triangle(
            vec2(tri.p[0].0, tri.p[0].1),
            vec2(tri.p[1].0, tri.p[1].1),
            vec2(tri.p[2].0, tri.p[2].1),
            tri.color,
        );
    }

    // --- Features ------------------------------------------------------------
    // Stand each on the terrain surface and draw back-to-front so nearer scenery
    // overlays farther; cull any hidden behind the island's own hills. Past the
    // fade range skip the whole pass (no sort, no per-feature raymarch) so distant
    // isles cost only their body; within it dim the scenery toward transparent.
    if dist >= FEATURE_FADE_END {
        return;
    }
    let feat_fade =
        clamp((FEATURE_FADE_END - dist) / (FEATURE_FADE_END - FEATURE_FADE_START), 0.0, 1.0);
    let feat_alpha = alpha * feat_fade;
    let mut order: Vec<usize> = (0..features.len()).collect();
    order.sort_by(|&a, &b| {
        let da = kin.pos.distance_to(isle.pos + features[a].offset);
        let db = kin.pos.distance_to(isle.pos + features[b].offset);
        db.partial_cmp(&da).unwrap()
    });
    for &fi in &order {
        let f = &features[fi];
        let wp = isle.pos + f.offset;
        // Skip scenery behind the camera: its billboard would wrap across the seam
        // (see [`behind_camera`]); it is out of the forward view in any case.
        if behind_camera(wp, kin) {
            continue;
        }
        let base = terrain.elevation_at(wp);
        if terrain.occluded(wp, base, kin, v) {
            continue;
        }
        let (fx, fy) = project(wp, base, base < 0.5, kin, v);
        let (_, ty) = project(wp, base + f.height, false, kin, v);
        let h_px = fy - ty;
        if h_px < 1.0 {
            continue;
        }
        let w_px = (h_px * feature_aspect(f.kind) * f.size).max(2.0);
        draw_feature(f.kind, fx, fy, w_px, h_px, feat_alpha, v);
        // After dusk the settlement's windows light up: a tiny warm or cold lamp in
        // each house, the watchtower carrying a brighter beacon. Drawn over the
        // building it belongs to, so it rides the island's depth slot and a nearer
        // wave band paints over it like the rest of the isle.
        if isle.is_port && v.lamp > 0.01 {
            draw_window_light(f.kind, fi, fx, fy, h_px, feat_alpha, v.lamp, v.t);
        }
    }
}

/// A stable hash of a feature index to [0, 1), so a house's window stays the same
/// colour/brightness frame to frame without an RNG (the classic sin-fract trick).
#[inline]
fn lamp_hash(n: f32) -> f32 {
    let s = (n * 12.9898).sin() * 43758.5453;
    s - s.floor()
}

/// A single lit window on a house (or a beacon atop the watchtower): a small glowing
/// dot with a soft halo, in a mostly-warm scatter of lamp colours with the odd cold
/// one. Non-dwelling features get no light. `idx` keys the deterministic colour,
/// whether it is lit at all, and its twinkle.
fn draw_window_light(
    kind: FeatureKind,
    idx: usize,
    fx: f32,
    fy: f32,
    h_px: f32,
    alpha: f32,
    lamp: f32,
    t: f32,
) {
    // Where up the building the light sits, the share of houses that are lit, and a
    // size multiplier. The watchtower is always lit and brighter (a harbour beacon).
    let (frac_up, gate, big) = match kind {
        FeatureKind::Hut => (0.42, 0.78, 1.0),
        FeatureKind::Cottage => (0.40, 0.82, 1.05),
        FeatureKind::Tower => (0.90, 1.0, 1.7),
        _ => return,
    };
    if h_px < 3.0 {
        return;
    }
    let n = idx as f32;
    let h1 = lamp_hash(n + 0.5);
    if h1 > gate {
        return; // a darkened house
    }
    let h2 = lamp_hash(n * 2.7 + 1.3);
    let h3 = lamp_hash(n * 5.1 + 2.9);
    // Mostly warm lamplight, with a scatter of cool blue-white windows.
    let col = if h2 < 0.55 {
        [255.0, 168.0, 86.0] // warm lamp
    } else if h2 < 0.80 {
        [255.0, 206.0, 128.0] // amber
    } else if h2 < 0.93 {
        [188.0, 214.0, 255.0] // cool blue
    } else {
        [236.0, 242.0, 255.0] // pale white
    };
    let tw = 0.82 + 0.18 * (t * (1.5 + h3 * 2.0) + h1 * std::f32::consts::TAU).sin();
    let a = clamp(lamp * alpha * tw, 0.0, 1.0);
    if a <= 0.01 {
        return;
    }
    let ly = fy - h_px * frac_up;
    let sz = (h_px * 0.12 * big).clamp(0.8, h_px * 0.5);
    let core = Color::new(col[0] / 255.0, col[1] / 255.0, col[2] / 255.0, a);
    let halo = Color::new(col[0] / 255.0, col[1] / 255.0, col[2] / 255.0, a * 0.30);
    draw_circle(fx, ly, sz * 2.2, halo);
    draw_circle(fx, ly, sz, core);
}

/// Width-to-height ratio of each feature's billboard.
fn feature_aspect(kind: FeatureKind) -> f32 {
    match kind {
        FeatureKind::Tree => 0.85,
        FeatureKind::Palm => 0.8,
        FeatureKind::Pine => 0.55,
        FeatureKind::Fern => 1.5,
        FeatureKind::Bush => 1.7,
        FeatureKind::Rock => 1.4,
        FeatureKind::Ruin => 1.2,
        FeatureKind::Hut => 1.35,
        FeatureKind::Cottage => 1.5,
        FeatureKind::Tower => 0.5,
        FeatureKind::Dock => 2.6,
        FeatureKind::Flag => 0.6,
        FeatureKind::Shipwreck => 1.9,
    }
}

// --- Feature billboards (flat-shaded vector shapes) --------------------------
// Drawn in a local space where x ∈ [-0.5, 0.5] and y ∈ [0, 1] (0 = foot at the
// ground, 1 = top), mapped to screen at (cx + lx·w, foot − ly·h). Two-tone where
// it helps imply form, matching the faceted low-poly look.

fn draw_feature(kind: FeatureKind, cx: f32, foot: f32, w: f32, h: f32, alpha: f32, v: &IslandView) {
    // Local→screen.
    let p = |lx: f32, ly: f32| vec2(cx + lx * w, foot - ly * h);
    let quad = |x0: f32, y0: f32, x1: f32, y1: f32, c: Color| {
        draw_triangle(p(x0, y0), p(x1, y0), p(x1, y1), c);
        draw_triangle(p(x0, y0), p(x1, y1), p(x0, y1), c);
    };
    let tri = |a: (f32, f32), b: (f32, f32), cc: (f32, f32), c: Color| {
        draw_triangle(p(a.0, a.1), p(b.0, b.1), p(cc.0, cc.1), c);
    };
    // Shadow the module `rgba` with one shaded by the day/night light (flat, like the
    // floor disc), so every feature colour below takes the hour's warmth, reddens at
    // dusk with the rest of the isle, and darkens after dark.
    let rgba = |c: [f32; 3], a: f32| v.shade(c, 1.0, a);

    const TRUNK: [f32; 3] = [92.0, 64.0, 40.0];
    const CANOPY: [f32; 3] = [52.0, 132.0, 64.0];
    const CANOPY_DK: [f32; 3] = [34.0, 96.0, 48.0];
    const FROND: [f32; 3] = [60.0, 140.0, 72.0];
    const FROND_DK: [f32; 3] = [40.0, 104.0, 56.0];
    const PINE: [f32; 3] = [40.0, 102.0, 70.0];
    const PINE_DK: [f32; 3] = [28.0, 76.0, 52.0];
    const FERN: [f32; 3] = [72.0, 150.0, 80.0];
    const FERN_DK: [f32; 3] = [50.0, 116.0, 62.0];
    const BUSH: [f32; 3] = [66.0, 138.0, 72.0];
    const BUSH_DK: [f32; 3] = [46.0, 106.0, 56.0];
    const ROCK: [f32; 3] = [126.0, 122.0, 114.0];
    const ROCK_DK: [f32; 3] = [94.0, 90.0, 84.0];
    const STONE: [f32; 3] = [156.0, 150.0, 140.0];
    const STONE_DK: [f32; 3] = [116.0, 110.0, 102.0];
    const WALL: [f32; 3] = [198.0, 172.0, 132.0];
    const WALL_DK: [f32; 3] = [160.0, 136.0, 100.0];
    const ROOF: [f32; 3] = [150.0, 72.0, 52.0];
    const ROOF_DK: [f32; 3] = [120.0, 56.0, 40.0];
    const WOOD: [f32; 3] = [126.0, 90.0, 56.0];
    const WOOD_DK: [f32; 3] = [96.0, 66.0, 40.0];
    const FLAGC: [f32; 3] = [205.0, 64.0, 58.0];
    const POLE: [f32; 3] = [82.0, 72.0, 60.0];
    const WRECK: [f32; 3] = [74.0, 56.0, 42.0];
    const WRECK_DK: [f32; 3] = [52.0, 40.0, 30.0];

    match kind {
        FeatureKind::Tree => {
            quad(-0.08, 0.0, 0.08, 0.42, rgba(TRUNK, alpha));
            tri((-0.45, 0.3), (0.0, 0.3), (0.0, 1.0), rgba(CANOPY, alpha));
            tri((0.0, 0.3), (0.45, 0.3), (0.0, 1.0), rgba(CANOPY_DK, alpha));
        }
        FeatureKind::Palm => {
            quad(-0.05, 0.0, 0.06, 0.6, rgba(TRUNK, alpha));
            // Fronds fanning from the crown.
            tri((0.0, 0.55), (-0.5, 0.78), (-0.28, 0.92), rgba(FROND, alpha));
            tri((0.0, 0.55), (-0.18, 0.95), (0.04, 1.02), rgba(FROND, alpha));
            tri((0.0, 0.55), (0.5, 0.8), (0.28, 0.92), rgba(FROND_DK, alpha));
            tri((0.0, 0.55), (0.2, 0.98), (0.0, 1.0), rgba(FROND_DK, alpha));
        }
        FeatureKind::Pine => {
            // A tall conifer: a slim trunk under three stacked skirts.
            quad(-0.06, 0.0, 0.06, 0.3, rgba(TRUNK, alpha));
            tri((-0.5, 0.24), (0.5, 0.24), (0.0, 0.58), rgba(PINE_DK, alpha));
            tri((-0.4, 0.46), (0.4, 0.46), (0.0, 0.78), rgba(PINE, alpha));
            tri((-0.28, 0.66), (0.28, 0.66), (0.0, 1.0), rgba(PINE, alpha));
        }
        FeatureKind::Fern => {
            // A low spray of fronds fanning from the ground.
            tri((-0.5, 0.0), (-0.12, 0.0), (-0.34, 1.0), rgba(FERN, alpha));
            tri((-0.2, 0.0), (0.2, 0.0), (0.0, 1.05), rgba(FERN_DK, alpha));
            tri((0.12, 0.0), (0.5, 0.0), (0.34, 1.0), rgba(FERN, alpha));
        }
        FeatureKind::Bush => {
            tri((-0.5, 0.0), (0.5, 0.0), (-0.12, 0.95), rgba(BUSH, alpha));
            tri((0.5, 0.0), (0.12, 0.95), (-0.12, 0.95), rgba(BUSH_DK, alpha));
            tri((-0.5, 0.0), (-0.12, 0.95), (0.12, 0.95), rgba(BUSH, alpha));
        }
        FeatureKind::Rock => {
            // Irregular faceted boulder.
            tri((-0.5, 0.0), (-0.28, 0.7), (0.06, 0.95), rgba(ROCK, alpha));
            tri((-0.5, 0.0), (0.06, 0.95), (0.5, 0.45), rgba(ROCK, alpha));
            tri((0.06, 0.95), (0.5, 0.45), (0.5, 0.0), rgba(ROCK_DK, alpha));
            tri((-0.5, 0.0), (0.5, 0.45), (0.5, 0.0), rgba(ROCK_DK, alpha));
        }
        FeatureKind::Ruin => {
            // A few broken columns on a low base.
            quad(-0.5, 0.0, 0.5, 0.14, rgba(STONE_DK, alpha));
            quad(-0.4, 0.1, -0.22, 0.78, rgba(STONE, alpha));
            quad(-0.08, 0.1, 0.1, 1.0, rgba(STONE, alpha));
            quad(0.24, 0.1, 0.42, 0.55, rgba(STONE_DK, alpha));
        }
        FeatureKind::Hut => {
            quad(-0.4, 0.0, 0.4, 0.6, rgba(WALL, alpha));
            quad(0.0, 0.0, 0.4, 0.6, rgba(WALL_DK, alpha));
            tri((-0.5, 0.55), (0.5, 0.55), (-0.05, 1.0), rgba(ROOF, alpha));
            tri((0.5, 0.55), (-0.05, 1.0), (0.05, 1.0), rgba(ROOF_DK, alpha));
        }
        FeatureKind::Cottage => {
            // A larger dwelling: a long body, a gable end, and a chimney.
            quad(-0.5, 0.0, 0.5, 0.55, rgba(WALL, alpha));
            quad(0.08, 0.0, 0.5, 0.55, rgba(WALL_DK, alpha));
            tri((-0.5, 0.5), (0.5, 0.5), (-0.02, 0.92), rgba(ROOF, alpha));
            tri((0.5, 0.5), (-0.02, 0.92), (0.04, 0.92), rgba(ROOF_DK, alpha));
            quad(0.28, 0.72, 0.4, 1.0, rgba(STONE_DK, alpha));
        }
        FeatureKind::Tower => {
            quad(-0.26, 0.0, 0.26, 0.82, rgba(STONE, alpha));
            quad(0.04, 0.0, 0.26, 0.82, rgba(STONE_DK, alpha));
            // Crenellations.
            quad(-0.3, 0.82, -0.12, 1.0, rgba(STONE, alpha));
            quad(-0.06, 0.82, 0.12, 1.0, rgba(STONE_DK, alpha));
            quad(0.18, 0.82, 0.3, 1.0, rgba(STONE_DK, alpha));
        }
        FeatureKind::Dock => {
            quad(-0.5, 0.3, 0.5, 0.62, rgba(WOOD, alpha));
            // Pilings.
            quad(-0.42, 0.0, -0.3, 0.4, rgba(WOOD_DK, alpha));
            quad(0.3, 0.0, 0.42, 0.4, rgba(WOOD_DK, alpha));
        }
        FeatureKind::Flag => {
            quad(-0.04, 0.0, 0.04, 1.0, rgba(POLE, alpha));
            tri((0.04, 0.66), (0.5, 0.82), (0.04, 1.0), rgba(FLAGC, alpha));
        }
        FeatureKind::Shipwreck => {
            // A broken hull canted on the beach with a snapped mast.
            tri((-0.5, 0.1), (0.45, 0.0), (0.5, 0.5), rgba(WRECK, alpha));
            tri((-0.5, 0.1), (0.5, 0.5), (-0.34, 0.55), rgba(WRECK_DK, alpha));
            quad(0.02, 0.45, 0.14, 1.0, rgba(WOOD_DK, alpha));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A view lit by `key`/`ambient`, the camera fields left at harmless values
    /// (only the light colours matter for the shading maths under test).
    fn view(key: (f32, f32, f32), ambient: (f32, f32, f32)) -> IslandView {
        IslandView {
            w: 0.0,
            horizon: 0.0,
            px_per_rad: 0.0,
            px_per_rad_h: 0.0,
            half_fov_h_view: 0.0,
            eye_rise: 0.0,
            sun: (0.0, 0.0, 1.0),
            key,
            ambient,
            warm: (1.0, 1.0, 1.0),
            warm_amt: 0.0,
            lamp: 0.0,
            t: 0.0,
        }
    }

    /// A colourless (grey) light must reproduce the original scalar shading exactly:
    /// `brightness × (AMBIENT + (1 - AMBIENT) · diff)`, on every channel. This is the
    /// invariant that keeps daytime looking as it always did.
    #[test]
    fn neutral_light_matches_the_old_scalar_shading() {
        let brightness = 0.8;
        let (key, ambient) = island_light(brightness, [120.0, 120.0, 120.0], [40.0, 40.0, 40.0]);
        let v = view(key, ambient);
        for &diff in &[0.0, 0.3, 1.0] {
            let want = brightness * (AMBIENT + (1.0 - AMBIENT) * diff);
            let (r, g, b) = v.lit(diff);
            for got in [r, g, b] {
                assert!((got - want).abs() < 1e-5, "diff {diff}: {got} vs {want}");
            }
        }
    }

    /// A warm key light (orange sun) reddens the sunlit faces: the lit multiplier's
    /// red channel outruns its blue. The shadowed floor (`diff` = 0) leans on the
    /// ambient sky fill instead, so a cool sky there keeps blue ahead of red.
    #[test]
    fn warm_sun_reddens_lit_faces_cool_sky_fills_shadow() {
        let (key, ambient) = island_light(1.0, [255.0, 112.0, 60.0], [40.0, 80.0, 160.0]);
        let v = view(key, ambient);
        let (r1, _, b1) = v.lit(1.0); // full sun
        assert!(r1 > b1, "sunlit face should be warm: r {r1} <= b {b1}");
        let (r0, _, b0) = v.lit(0.0); // shadow, ambient only
        assert!(b0 > r0, "shadowed face should be cool: b {b0} <= r {r0}");
    }

    /// Brightness scales both lights linearly, so night is a dimmer version of the
    /// same hue rather than a different colour.
    #[test]
    fn brightness_scales_the_lights() {
        let (ka, aa) = island_light(1.0, [255.0, 112.0, 60.0], [40.0, 80.0, 160.0]);
        let (kb, ab) = island_light(0.5, [255.0, 112.0, 60.0], [40.0, 80.0, 160.0]);
        assert!((kb.0 - ka.0 * 0.5).abs() < 1e-5);
        assert!((ab.2 - aa.2 * 0.5).abs() < 1e-5);
    }

    /// `warm_light` only fires for a genuinely warm (red-over-blue) light: the dusk
    /// sun pulls hard, a white noon sun barely at all, and the cool moon not at all.
    #[test]
    fn warm_light_tracks_the_hour() {
        let (_, dusk) = warm_light([255.0, 112.0, 60.0]);
        let (_, noon) = warm_light([255.0, 246.0, 222.0]);
        let (_, moon) = warm_light([138.0, 170.0, 212.0]);
        assert!(dusk >= WARM_MAX - 1e-6, "dusk should pull full tilt: {dusk}");
        assert!(noon == 0.0, "the warm-white noon sun must not redden the land: {noon}");
        assert!(moon == 0.0, "the cool moon must not warm the land: {moon}");
    }

    /// The warm-shift reddens a face a plain multiply cannot: a green hillside at dusk
    /// comes out fully red (red well ahead of green) at the same brightness (a hue
    /// recolour, not a brighten). Even a shadowed facet reddens (the whole island goes
    /// red, per `WARM_FLOOR`); only an absence of warmth leaves it untouched.
    #[test]
    fn warm_shift_reddens_green_without_brightening() {
        let (warm, warm_amt) = warm_light([255.0, 112.0, 60.0]);
        let mut v = view((1.0, 1.0, 1.0), (1.0, 1.0, 1.0));
        v.warm = warm;
        v.warm_amt = warm_amt;
        let green = (0.28, 0.45, 0.20); // a foliage facet already through the dusk multiply
        let luma = |c: (f32, f32, f32)| c.0 * 0.30 + c.1 * 0.59 + c.2 * 0.11;
        let lit = v.warm_shift(green, 1.0);
        assert!(lit.0 > lit.1 * 1.5, "dusk green should turn fully red: r {} vs g {}", lit.0, lit.1);
        assert!((luma(lit) - luma(green)).abs() < 1e-5, "warm-shift must preserve brightness");
        // Shadowed faces still redden (WARM_FLOOR), just less than the sunlit side.
        let shade = v.warm_shift(green, 0.0);
        assert!(shade.0 > green.0 && shade.0 < lit.0, "shadow should redden, but less: {shade:?}");
        // Only an absence of warmth (noon/night) leaves the colour untouched.
        v.warm_amt = 0.0;
        assert_eq!(v.warm_shift(green, 1.0), green);
    }
}
