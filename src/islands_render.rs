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
use crate::projection::{BASE_EYE, EYE_HEIGHT, MAX_VIEW, SHORE_LIFT};
use crate::rng::Rng;
use crate::sailing::Kinematics;
use crate::world::{Island, IsleKind};

use std::f32::consts::{FRAC_PI_2, TAU};

const FLOOR_SEG: usize = 48; // floor ellipse: smooth
const MOUND_SEG: usize = 48; // landmass body: angular facets around the coast
const SIDE_CULL: f32 = 1.6; // how far off-axis an isle may sit before it's skipped
const AMBIENT: f32 = 0.45; // floor of the directional shading

const GOLDEN: i64 = 0x9e3779b97f4a7c15u64 as i64;
const SHADOW: [f32; 3] = [8.0, 40.0, 30.0];

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
    /// Overall brightness from the day/night clock (1 = full noon, ~0.3 deep
    /// night), multiplied into every facet so the isle darkens to a moonlit
    /// silhouette after dusk.
    pub light: f32,
}

// --- Terrain heightfield -----------------------------------------------------

/// One Gaussian hill: a bump centred at `off` (m from the island centre) rising
/// `height` m, with `sigma` the metric width of its skirt.
#[derive(Clone, Copy)]
struct Peak {
    off: Vec2,
    height: f32,
    sigma: f32,
}

/// One octave of directional value-noise: a travelling sine `amp·sin(dir·p·freq +
/// phase)` over the chart. Summed across octaves of rising frequency / falling
/// amplitude, these overlapping height-waves give the surface ridges, saddles and
/// dells instead of one smooth dome.
#[derive(Clone, Copy)]
struct Octave {
    freq: f32,
    dir: (f32, f32),
    phase: f32,
    amp: f32,
}

/// A deterministic per-island terrain. The outline is a lumpy coastline (mean
/// radius modulated by several low-frequency lobes — bays and headlands). The
/// height surface is the sum of a few Gaussian hills (the major massifs) plus
/// several octaves of overlapping noise-waves ([`Octave`]) that fold the slopes
/// into ridges and hollows, all faded to sea level at the shore. Built fresh each
/// frame from the island's id + position, so a given chart always grows the same
/// land without threading the world seed through the renderer.
struct IsleTerrain {
    center: Vec2,
    radius: f32,
    base: f32,
    lobes: [(f32, f32, f32); 4], // (frequency, amplitude, phase)
    peaks: Vec<Peak>,
    octaves: Vec<Octave>,
    /// Metres of relief the noise-waves add (±) on top of the Gaussian hills.
    relief: f32,
    /// Surface below this elevation (m) reads as beach/rock rim, above as foliage.
    beach: f32,
    /// Radial mesh resolution (ring count from centre to coast).
    rings: usize,
}

impl IsleTerrain {
    fn for_island(isle: &Island) -> IsleTerrain {
        // Vary the shape by both island id and its (seed-dependent) position, so
        // different worlds grow different coastlines for the same slot.
        let bits = (isle.id as i64).wrapping_mul(GOLDEN)
            ^ (isle.pos.x.to_bits() as i64)
            ^ ((isle.pos.y.to_bits() as i64) << 21);
        let mut rng = Rng::from_seed(bits);
        let r = isle.radius;
        let h = isle.height;
        let tau = TAU as f64;

        // Coastline lobes: low frequencies dominate so the outline reads as broad
        // bays and headlands rather than noise. Amplitudes sum below `base`'s
        // headroom (≈0.20) so the coast never pushes past `radius` (the grounding
        // circle); the mean pulls in to ~0.78·radius, giving inlets that bite in.
        let lobes = [
            (2.0, rng.between(0.06, 0.12) as f32, rng.between(0.0, tau) as f32),
            (3.0, rng.between(0.04, 0.09) as f32, rng.between(0.0, tau) as f32),
            (5.0, rng.between(0.02, 0.05) as f32, rng.between(0.0, tau) as f32),
            (7.0, rng.between(0.01, 0.035) as f32, rng.between(0.0, tau) as f32),
        ];

        // A small offset near the centre for a summit, so the peak isn't dead-centred.
        let peak = |rng: &mut Rng, rad_lo: f32, rad_hi: f32, height: f32, sig: f32| -> Peak {
            let a = rng.between(0.0, tau) as f32;
            let rad = rng.between(rad_lo as f64, rad_hi as f64) as f32 * r;
            Peak {
                off: Vec2::new(a.cos() * rad, a.sin() * rad),
                height,
                sigma: sig * r,
            }
        };

        // The major massifs (a handful of overlapping Gaussian hills) and how much
        // the noise-waves then fold the slopes. Volcanic keeps a recognisable cone;
        // rocky is the craggiest; green/jungle roll gently.
        let mut peaks = Vec::new();
        let relief = match isle.terrain {
            IsleKind::Volcanic => {
                peaks.push(peak(&mut rng, 0.0, 0.10, h, 0.30));
                if rng.next_f64() < 0.6 {
                    peaks.push(peak(&mut rng, 0.20, 0.42, h * 0.5, 0.22));
                }
                h * 0.22
            }
            IsleKind::Rocky => {
                peaks.push(peak(&mut rng, 0.0, 0.16, h, 0.30));
                let extra = rng.int_between(2, 4);
                for _ in 0..extra {
                    let hh = rng.between(0.5, 0.85) as f32 * h;
                    peaks.push(peak(&mut rng, 0.18, 0.48, hh, 0.22));
                }
                h * 0.42
            }
            IsleKind::Green | IsleKind::Jungle => {
                peaks.push(peak(&mut rng, 0.0, 0.18, h, 0.36));
                let extra = rng.int_between(1, 3);
                for _ in 0..extra {
                    let hh = rng.between(0.5, 0.85) as f32 * h;
                    peaks.push(peak(&mut rng, 0.18, 0.46, hh, 0.30));
                }
                h * 0.32
            }
        };

        // Overlapping height-waves: four octaves of directional value-noise, each
        // half the wavelength and ~half the amplitude of the last. The longest is a
        // touch over the island span (one or two broad swells across it); the
        // shortest stays above the mesh's sampling limit so it doesn't alias.
        let mut octaves = Vec::new();
        let mut wavelength = r * rng.between(1.1, 1.5) as f32;
        let mut amp = 1.0f32;
        for _ in 0..4 {
            let ang = rng.between(0.0, tau) as f32;
            octaves.push(Octave {
                freq: TAU / wavelength,
                dir: (ang.cos(), ang.sin()),
                phase: rng.between(0.0, tau) as f32,
                amp,
            });
            wavelength *= 0.5;
            amp *= 0.5;
        }

        let tall = matches!(isle.terrain, IsleKind::Rocky | IsleKind::Volcanic);
        IsleTerrain {
            center: isle.pos,
            radius: r,
            base: 0.78,
            lobes,
            peaks,
            octaves,
            relief,
            beach: (h * 0.06).max(1.4),
            rings: if tall { 11 } else { 8 },
        }
    }

    /// Shore radius (m) in compass-free local angle `a` (atan2(y, x)).
    #[inline]
    fn coast_radius(&self, a: f32) -> f32 {
        let mut s = self.base;
        for &(f, amp, ph) in &self.lobes {
            s += amp * (f * a + ph).sin();
        }
        self.radius * s.max(0.3)
    }

    /// Summed octave noise at a local point, normalised to roughly [-1, 1].
    #[inline]
    fn noise(&self, local: Vec2) -> f32 {
        let mut s = 0.0;
        let mut norm = 0.0;
        for o in &self.octaves {
            let t = (local.x * o.dir.0 + local.y * o.dir.1) * o.freq + o.phase;
            s += o.amp * t.sin();
            norm += o.amp;
        }
        s / norm.max(1e-6)
    }

    /// Surface elevation (m above sea) at a world point, 0 outside the coast.
    #[inline]
    fn elevation_at(&self, p: Vec2) -> f32 {
        let local = p - self.center;
        let dist = local.length();
        let a = local.y.atan2(local.x);
        let rc = self.coast_radius(a);
        if dist >= rc {
            return 0.0;
        }
        // Major massifs.
        let mut field = 0.0;
        for pk in &self.peaks {
            let dx = local.x - pk.off.x;
            let dy = local.y - pk.off.y;
            let d2 = dx * dx + dy * dy;
            field += pk.height * (-d2 / (2.0 * pk.sigma * pk.sigma)).exp();
        }
        // Overlapping height-waves fold the slopes into ridges and hollows across
        // the interior, fading out only over the outer band so the coastline stays
        // at sea level rather than the waves punching land out into the water.
        let mut w = ((rc - dist) / (rc * 0.32)).clamp(0.0, 1.0);
        w = w * w * (3.0 - 2.0 * w);
        field += self.noise(local) * self.relief * w;
        let field = field.max(0.0);
        // Smooth fade to sea level over the outer fifth so the shore lies flat.
        let mut edge = ((rc - dist) / (rc * 0.22)).clamp(0.0, 1.0);
        edge = edge * edge * (3.0 - 2.0 * edge);
        field * edge
    }

    /// Is a feature standing at `wp` (foot elevation `foot_z`) hidden behind the
    /// island's own terrain from the camera at `kin.pos`? Marches a few samples
    /// along the eye→feature ray (only the part nearer than the feature) and asks
    /// whether the terrain there projects *above* the feature's foot on screen.
    /// This replaces the old "tall isle, far hemisphere" heuristic and works for
    /// flat and hilly islands alike — a hummock can hide scenery just behind it.
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

#[inline]
fn col(base: [f32; 3], shade: f32, alpha: f32) -> Color {
    Color::new(
        base[0] / 255.0 * shade,
        base[1] / 255.0 * shade,
        base[2] / 255.0 * shade,
        alpha,
    )
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

/// Flat-shade a triangle from its world-space (x, y, z) corners against the sun.
fn shade(a: (f32, f32, f32), b: (f32, f32, f32), c: (f32, f32, f32), v: &IslandView) -> f32 {
    let (ux, uy, uz) = (b.0 - a.0, b.1 - a.1, b.2 - a.2);
    let (vx, vy, vz) = (c.0 - a.0, c.1 - a.1, c.2 - a.2);
    let (mut nx, mut ny, mut nz) = (uy * vz - uz * vy, uz * vx - ux * vz, ux * vy - uy * vx);
    if nz < 0.0 {
        nx = -nx;
        ny = -ny;
        nz = -nz;
    }
    let len = (nx * nx + ny * ny + nz * nz).sqrt().max(1e-6);
    let diff = ((nx * v.sun.0 + ny * v.sun.1 + nz * v.sun.2) / len).max(0.0);
    AMBIENT + (1.0 - AMBIENT) * diff
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
    // Skip isles faded out, off to the side, or ones we've sailed inside of (the
    // ring projection degenerates there). Grounding keeps the hull just outside the
    // shore radius, so culling only strictly-inside keeps big isles visible up close.
    // The off-axis limit is widened by the isle's angular half-width (`asin(radius/d)`)
    // so a large isle sailed alongside — its near shore filling the view while its
    // centre sits abeam — isn't culled the moment its centre clears the FOV.
    let ang_r = (isle.radius / dist).min(1.0).asin();
    if dist > MAX_VIEW || dist < isle.radius || rel.abs() > v.half_fov_h_view * SIDE_CULL + ang_r {
        return;
    }
    let alpha = clamp((1.0 - dist / MAX_VIEW) * 1.5, 0.0, 1.0);
    let (sand, foliage) = palette(isle);
    let terrain = IsleTerrain::for_island(isle);

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
    floor_ring(1.10, col(SHADOW, v.light, alpha * 0.45));
    floor_ring(1.0, col(sand, v.light, alpha));

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
        let sh = shade(a, b, c, v);
        let cx = (a.0 + b.0 + c.0) / 3.0;
        let cy = (a.1 + b.1 + c.1) / 3.0;
        tris.push(Tri {
            key: kin.pos.distance_to(Vec2::new(cx, cy)),
            p: [pa, pb, pc],
            color: col(base, sh * v.light, alpha),
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
    // overlays farther; cull any hidden behind the island's own hills.
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
        draw_feature(f.kind, fx, fy, w_px, h_px, alpha, v.light);
    }
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

fn draw_feature(kind: FeatureKind, cx: f32, foot: f32, w: f32, h: f32, alpha: f32, light: f32) {
    // Local→screen.
    let p = |lx: f32, ly: f32| vec2(cx + lx * w, foot - ly * h);
    let quad = |x0: f32, y0: f32, x1: f32, y1: f32, c: Color| {
        draw_triangle(p(x0, y0), p(x1, y0), p(x1, y1), c);
        draw_triangle(p(x0, y0), p(x1, y1), p(x0, y1), c);
    };
    let tri = |a: (f32, f32), b: (f32, f32), cc: (f32, f32), c: Color| {
        draw_triangle(p(a.0, a.1), p(b.0, b.1), p(cc.0, cc.1), c);
    };
    // Shadow the module `rgba` with one dimmed by the day/night light, so every
    // feature colour below darkens with the rest of the isle after dusk.
    let rgba = |c: [f32; 3], a: f32| {
        Color::new(c[0] / 255.0 * light, c[1] / 255.0 * light, c[2] / 255.0 * light, a)
    };

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
