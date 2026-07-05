//! Low-poly 3D scenery models: the trees, huts, rocks and landmarks that dress
//! an island. Replaces the old screen-space feature billboards. Each model is
//! built from a handful of world-space triangles (tapered tubes, boxes, cones
//! and thin fins) in a local frame standing on the terrain, Lambert-shaded
//! against the sun exactly like the landmass facets, and pushed into the same
//! depth-sorted triangle list ([`crate::islands_render::Tri`]) as the island
//! body. That shared sort is the whole point: a ridge occludes the cottage
//! behind it facet by facet (no binary pop-in), and a trunk can never clip
//! through the slope it stands on.
//!
//! The local model frame: x/y lie on the ground (1.0 = the feature's width in
//! metres), z points up (1.0 = the feature's height in metres), and the whole
//! frame is spun by a per-feature yaw hashed from the feature index, so a
//! village isn't a grid of identically-facing houses. Nothing here draws from
//! the world RNG: determinism of the generation streams is untouched.

use macroquad::color::Color;

use crate::geometry::Vec2;
use crate::isle_features::{FeatureKind, IsleFeature};
use crate::islands_render::{behind_camera, project, IslandView, Tri};
use crate::projection::EYE_HEIGHT;
use crate::sailing::Kinematics;

use std::f32::consts::{FRAC_PI_2, TAU};

/// A point in the local model frame (or, inside [`Sculptor::push`], in world
/// metres): x/y on the chart, z up.
type P3 = (f32, f32, f32);

fn add(a: P3, b: P3) -> P3 {
    (a.0 + b.0, a.1 + b.1, a.2 + b.2)
}
fn sub(a: P3, b: P3) -> P3 {
    (a.0 - b.0, a.1 - b.1, a.2 - b.2)
}
fn mul(a: P3, s: f32) -> P3 {
    (a.0 * s, a.1 * s, a.2 * s)
}
fn cross(a: P3, b: P3) -> P3 {
    (a.1 * b.2 - a.2 * b.1, a.2 * b.0 - a.0 * b.2, a.0 * b.1 - a.1 * b.0)
}
fn dot(a: P3, b: P3) -> f32 {
    a.0 * b.0 + a.1 * b.1 + a.2 * b.2
}
fn norm(a: P3) -> P3 {
    mul(a, 1.0 / dot(a, a).sqrt().max(1e-6))
}

/// A stable per-feature hash to [0, 1): the same sin-fract trick the window
/// lights use, so a model's yaw and jitter never change frame to frame and no
/// RNG stream is consumed.
#[inline]
fn hash01(n: f32) -> f32 {
    let s = (n * 12.9898).sin() * 43758.5453;
    s - s.floor()
}

/// Triangle emitter for one feature: owns the local frame (position, yaw,
/// width/height scale) and the shading/projection context, and pushes finished
/// screen-space [`Tri`]s onto the island's shared depth-sorted list.
struct Sculptor<'a, 'b> {
    kin: &'a Kinematics,
    v: &'a IslandView,
    tris: &'b mut Vec<Tri>,
    alpha: f32,
    key: f32,
    pos: Vec2,
    foot: f32,
    cos_yaw: f32,
    sin_yaw: f32,
    w: f32,
    h: f32,
    eye_z: f32,
    /// Whether triangles need the per-corner behind-camera seam check. Only a
    /// large feature close aboard can straddle the seam while its centre is in
    /// front; for everything else the caller's centre check suffices.
    check_seam: bool,
    /// Per-feature deterministic salt for [`Sculptor::jit`].
    salt: f32,
}

impl Sculptor<'_, '_> {
    /// Local frame to world metres: rotate x/y by the yaw, scale by width, lift
    /// z by the height onto the terrain foot.
    #[inline]
    fn world(&self, p: P3) -> P3 {
        (
            self.pos.x + (p.0 * self.cos_yaw - p.1 * self.sin_yaw) * self.w,
            self.pos.y + (p.0 * self.sin_yaw + p.1 * self.cos_yaw) * self.w,
            self.foot + p.2 * self.h,
        )
    }

    /// A deterministic per-feature jitter in [0, 1), keyed by `k`.
    #[inline]
    fn jit(&self, k: f32) -> f32 {
        hash01(self.salt + k)
    }

    /// Emit one triangle. With an `inside` reference point the face is part of
    /// a closed surface: its normal is oriented outward (away from `inside`),
    /// back faces are culled, and the Lambert term is one-sided. Without it the
    /// face is a thin double-sided fin (a frond, a flag) lit from either side.
    /// `emissive` skips the sun shading entirely (flame, lava).
    fn push(&mut self, base: [f32; 3], a: P3, b: P3, c: P3, inside: Option<P3>, emissive: bool) {
        let aw = self.world(a);
        let bw = self.world(b);
        let cw = self.world(c);
        if self.check_seam {
            for q in [aw, bw, cw] {
                if behind_camera(Vec2::new(q.0, q.1), self.kin) {
                    return;
                }
            }
        }
        let centroid = mul(add(add(aw, bw), cw), 1.0 / 3.0);
        let mut n = cross(sub(bw, aw), sub(cw, aw));
        let one_sided = if let Some(ins) = inside {
            let insw = self.world(ins);
            if dot(n, sub(centroid, insw)) < 0.0 {
                n = mul(n, -1.0);
            }
            let eye = (self.kin.pos.x, self.kin.pos.y, self.eye_z);
            if dot(n, sub(eye, centroid)) <= 0.0 {
                return; // back face of a closed shape: never visible
            }
            true
        } else {
            false
        };
        let color = if emissive {
            Color::new(base[0] / 255.0, base[1] / 255.0, base[2] / 255.0, self.alpha)
        } else {
            let nn = norm(n);
            let raw = nn.0 * self.v.sun.0 + nn.1 * self.v.sun.1 + nn.2 * self.v.sun.2;
            let diff = if one_sided { raw.max(0.0) } else { raw.abs() };
            self.v.shade(base, diff, self.alpha)
        };
        let p = [aw, bw, cw]
            .map(|q| project(Vec2::new(q.0, q.1), q.2, q.2 < 0.5, self.kin, self.v));
        self.tris.push(Tri { key: self.key, p, color });
    }

    /// A double-sided fin triangle (fronds, blades, cloth).
    fn fin(&mut self, base: [f32; 3], a: P3, b: P3, c: P3) {
        self.push(base, a, b, c, None, false);
    }

    fn fin_quad(&mut self, base: [f32; 3], a: P3, b: P3, c: P3, d: P3) {
        self.fin(base, a, b, c);
        self.fin(base, a, c, d);
    }

    /// One face of a closed surface, oriented outward from `inside` and culled
    /// when facing away.
    fn wall(&mut self, base: [f32; 3], a: P3, b: P3, c: P3, inside: P3) {
        self.push(base, a, b, c, Some(inside), false);
    }

    fn wall_quad(&mut self, base: [f32; 3], a: P3, b: P3, c: P3, d: P3, inside: P3) {
        self.wall(base, a, b, c, inside);
        self.wall(base, a, c, d, inside);
    }

    /// An unshaded, full-bright triangle (fire and lava read as light sources).
    fn glow(&mut self, base: [f32; 3], a: P3, b: P3, c: P3) {
        self.push(base, a, b, c, None, true);
    }

    /// A tapered tube between two circles set perpendicular to the axis c0 to
    /// c1: the workhorse primitive. Vertical it is a frustum (trunks, towers),
    /// tilted it is a branch or a fallen log; `r1 = 0` closes it to a cone.
    /// `cap` roofs the far end with a fan.
    #[allow(clippy::too_many_arguments)]
    fn tube(&mut self, base: [f32; 3], c0: P3, r0: f32, c1: P3, r1: f32, sides: usize, cap: bool) {
        let axis = sub(c1, c0);
        let an = norm(axis);
        let reference = if an.2.abs() > 0.9 { (1.0, 0.0, 0.0) } else { (0.0, 0.0, 1.0) };
        let u = norm(cross(reference, an));
        let w2 = cross(an, u);
        let inside = mul(add(c0, c1), 0.5);
        let ring = |c: P3, r: f32, i: usize| -> P3 {
            let a = i as f32 / sides as f32 * TAU;
            add(c, add(mul(u, a.cos() * r), mul(w2, a.sin() * r)))
        };
        for i in 0..sides {
            let j = (i + 1) % sides;
            match (r0 > 1e-4, r1 > 1e-4) {
                (true, true) => self.wall_quad(
                    base,
                    ring(c0, r0, i),
                    ring(c0, r0, j),
                    ring(c1, r1, j),
                    ring(c1, r1, i),
                    inside,
                ),
                (true, false) => self.wall(base, ring(c0, r0, i), ring(c0, r0, j), c1, inside),
                (false, true) => self.wall(base, c0, ring(c1, r1, j), ring(c1, r1, i), inside),
                (false, false) => {}
            }
        }
        if cap && r1 > 1e-4 {
            for i in 0..sides {
                let j = (i + 1) % sides;
                self.wall(base, c1, ring(c1, r1, i), ring(c1, r1, j), inside);
            }
        }
    }

    /// An axis-aligned box in the local frame: four walls and a lid (the ground
    /// hides the floor).
    #[allow(clippy::too_many_arguments)]
    fn boxy(&mut self, base: [f32; 3], x0: f32, y0: f32, z0: f32, x1: f32, y1: f32, z1: f32) {
        let inside = ((x0 + x1) * 0.5, (y0 + y1) * 0.5, (z0 + z1) * 0.5);
        self.wall_quad(base, (x0, y0, z0), (x1, y0, z0), (x1, y0, z1), (x0, y0, z1), inside);
        self.wall_quad(base, (x1, y0, z0), (x1, y1, z0), (x1, y1, z1), (x1, y0, z1), inside);
        self.wall_quad(base, (x1, y1, z0), (x0, y1, z0), (x0, y1, z1), (x1, y1, z1), inside);
        self.wall_quad(base, (x0, y1, z0), (x0, y0, z0), (x0, y0, z1), (x0, y1, z1), inside);
        self.wall_quad(base, (x0, y0, z1), (x1, y0, z1), (x1, y1, z1), (x0, y1, z1), inside);
    }

    /// A rectangular-based pyramid (hut roofs): four faces to an apex.
    #[allow(clippy::too_many_arguments)]
    fn pyramid(&mut self, base: [f32; 3], x0: f32, y0: f32, x1: f32, y1: f32, z0: f32, apex: P3) {
        let inside = ((x0 + x1) * 0.5, (y0 + y1) * 0.5, z0 + (apex.2 - z0) * 0.25);
        let c = [(x0, y0, z0), (x1, y0, z0), (x1, y1, z0), (x0, y1, z0)];
        for i in 0..4 {
            self.wall(base, c[i], c[(i + 1) % 4], apex, inside);
        }
    }
}

/// The footprint width of a model as a ratio of its height (times the
/// feature's own `size` multiplier): the same proportions the old billboards
/// used, so the scatter tuning still reads right.
fn width_ratio(kind: FeatureKind) -> f32 {
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
        FeatureKind::DeadTree => 0.7,
        FeatureKind::FlowerPatch => 1.8,
        FeatureKind::Reeds => 1.2,
        FeatureKind::Cactus => 0.7,
        FeatureKind::FallenLog => 2.4,
        FeatureKind::Cairn => 1.1,
        FeatureKind::StoneArch => 1.3,
        FeatureKind::LavaVent => 1.6,
        FeatureKind::Campfire => 1.4,
        FeatureKind::Windmill => 1.0,
        FeatureKind::Lighthouse => 0.55,
    }
}

/// How far from its centre (m) a model actually bears on the ground, as opposed
/// to overhanging it. The renderer samples the terrain across this patch and
/// stands the model on the *lowest* point, so a flat base laid across a slope
/// sinks to meet the downhill ground instead of hanging over it. Trunked
/// plants touch only at the trunk (the canopy may overhang a drop freely);
/// slender towers at their drum; spreading clumps and anything built bear on
/// most of their footprint.
pub fn contact_radius(f: &IsleFeature) -> f32 {
    use FeatureKind::*;
    let ratio = match f.kind {
        Tree | Palm | Pine | DeadTree | Flag | Fern => 0.10,
        Cactus => 0.18,
        Tower | Windmill | Lighthouse => 0.32,
        Reeds => 0.42,
        _ => 0.5,
    };
    f.height * width_ratio(f.kind) * f.size * ratio
}

// Albedos, carried over from the billboard palette. Form now comes from the
// Lambert shading, so most shapes take one base colour; the few explicit dark
// variants left mark a genuinely different material (moss on a log, a roof
// against its wall), not a painted-on shadow.
const TRUNK: [f32; 3] = [92.0, 64.0, 40.0];
const CANOPY: [f32; 3] = [52.0, 132.0, 64.0];
const FROND: [f32; 3] = [60.0, 140.0, 72.0];
const FROND_DK: [f32; 3] = [40.0, 104.0, 56.0];
const PINE: [f32; 3] = [40.0, 102.0, 70.0];
const PINE_DK: [f32; 3] = [28.0, 76.0, 52.0];
const FERN: [f32; 3] = [72.0, 150.0, 80.0];
const FERN_DK: [f32; 3] = [50.0, 116.0, 62.0];
const BUSH: [f32; 3] = [66.0, 138.0, 72.0];
const ROCK: [f32; 3] = [126.0, 122.0, 114.0];
const ROCK_DK: [f32; 3] = [94.0, 90.0, 84.0];
const STONE: [f32; 3] = [156.0, 150.0, 140.0];
const STONE_DK: [f32; 3] = [116.0, 110.0, 102.0];
const WALL: [f32; 3] = [198.0, 172.0, 132.0];
const ROOF: [f32; 3] = [150.0, 72.0, 52.0];
const ROOF_DK: [f32; 3] = [120.0, 56.0, 40.0];
const WOOD: [f32; 3] = [126.0, 90.0, 56.0];
const WOOD_DK: [f32; 3] = [96.0, 66.0, 40.0];
const FLAGC: [f32; 3] = [205.0, 64.0, 58.0];
const POLE: [f32; 3] = [82.0, 72.0, 60.0];
const WRECK: [f32; 3] = [74.0, 56.0, 42.0];
const SNAG: [f32; 3] = [122.0, 108.0, 90.0];
const SNAG_DK: [f32; 3] = [92.0, 80.0, 66.0];
const MEADOW: [f32; 3] = [78.0, 148.0, 78.0];
const BLOOM_A: [f32; 3] = [236.0, 108.0, 150.0]; // pink
const BLOOM_B: [f32; 3] = [246.0, 214.0, 96.0]; // yellow
const BLOOM_C: [f32; 3] = [232.0, 236.0, 244.0]; // white
const REED: [f32; 3] = [150.0, 158.0, 92.0];
const REED_DK: [f32; 3] = [118.0, 128.0, 70.0];
const CACTUS: [f32; 3] = [86.0, 132.0, 78.0];
const CACTUS_DK: [f32; 3] = [62.0, 102.0, 60.0];
const MOSS: [f32; 3] = [78.0, 128.0, 72.0];
const BASALT: [f32; 3] = [72.0, 66.0, 66.0];
const EMBER: [f32; 3] = [255.0, 92.0, 44.0];
const FLAME: [f32; 3] = [242.0, 132.0, 46.0];
const FLAME_HOT: [f32; 3] = [252.0, 214.0, 96.0];

/// Build one feature's model and push its triangles onto the island's shared
/// depth-sorted list. `foot` is the ground elevation the model stands on (the
/// caller samples the drawn facet mesh under its contact patch, see
/// [`contact_radius`]), `key` the depth-sort key the caller chose for the
/// whole model, `alpha` the distance fade.
#[allow(clippy::too_many_arguments)]
pub fn emit(
    f: &IsleFeature,
    fi: usize,
    wp: Vec2,
    foot: f32,
    key: f32,
    alpha: f32,
    kin: &Kinematics,
    v: &IslandView,
    tris: &mut Vec<Tri>,
) {
    let salt = fi as f32 * 13.73 + 0.37;
    let yaw = hash01(salt) * TAU;
    let w = f.height * width_ratio(f.kind) * f.size;
    let dist = kin.pos.distance_to(wp);
    let mut s = Sculptor {
        kin,
        v,
        tris,
        alpha,
        key,
        pos: wp,
        foot,
        cos_yaw: yaw.cos(),
        sin_yaw: yaw.sin(),
        w,
        h: f.height,
        eye_z: EYE_HEIGHT + v.eye_rise,
        check_seam: dist < (w + f.height) * 8.0,
        salt,
    };

    match f.kind {
        FeatureKind::Tree => {
            s.tube(TRUNK, (0.0, 0.0, 0.0), 0.09, (0.0, 0.0, 0.45), 0.06, 5, false);
            s.tube(CANOPY, (0.0, 0.0, 0.30), 0.48, (0.0, 0.0, 0.72), 0.16, 6, false);
            s.tube(CANOPY, (0.0, 0.0, 0.55), 0.36, (0.0, 0.0, 1.0), 0.0, 6, false);
        }
        FeatureKind::Palm => {
            s.tube(TRUNK, (0.0, 0.0, 0.0), 0.06, (0.10, 0.0, 0.62), 0.04, 5, false);
            let crown = (0.10, 0.0, 0.60);
            for k in 0..5 {
                let a = k as f32 / 5.0 * TAU + 0.4;
                let (dx, dy) = (a.cos(), a.sin());
                let mid = (crown.0 + dx * 0.24, crown.1 + dy * 0.24, crown.2 + 0.18);
                let tip = (crown.0 + dx * 0.5, crown.1 + dy * 0.5, crown.2 + 0.05);
                let p = (-dy * 0.05, dx * 0.05, 0.0);
                let col = if k % 2 == 0 { FROND } else { FROND_DK };
                s.fin(col, crown, add(mid, p), sub(mid, p));
                s.fin(col, add(mid, p), sub(mid, p), tip);
            }
        }
        FeatureKind::Pine => {
            s.tube(TRUNK, (0.0, 0.0, 0.0), 0.07, (0.0, 0.0, 0.34), 0.05, 5, false);
            s.tube(PINE_DK, (0.0, 0.0, 0.22), 0.5, (0.0, 0.0, 0.56), 0.0, 6, false);
            s.tube(PINE, (0.0, 0.0, 0.44), 0.40, (0.0, 0.0, 0.78), 0.0, 6, false);
            s.tube(PINE, (0.0, 0.0, 0.64), 0.28, (0.0, 0.0, 1.0), 0.0, 6, false);
        }
        FeatureKind::Fern => {
            for k in 0..6 {
                let a = k as f32 / 6.0 * TAU + s.jit(1.1) * 1.5;
                let (dx, dy) = (a.cos(), a.sin());
                let root = (dx * 0.05, dy * 0.05, 0.0);
                let tip = (dx * 0.42, dy * 0.42, 0.8 + s.jit(k as f32 + 2.2) * 0.25);
                let p = (-dy * 0.06, dx * 0.06, 0.0);
                let col = if k % 2 == 0 { FERN } else { FERN_DK };
                s.fin(col, add(root, p), sub(root, p), tip);
            }
        }
        FeatureKind::Bush => {
            s.tube(BUSH, (0.0, 0.0, 0.0), 0.5, (0.0, 0.0, 0.55), 0.34, 6, false);
            s.tube(BUSH, (0.0, 0.0, 0.55), 0.34, (0.0, 0.0, 0.95), 0.0, 6, false);
        }
        FeatureKind::Rock => {
            // An irregular faceted boulder: a jittered 5-sided frustum under a
            // ridged top fan, so no two rocks read as the same prop.
            let n = 5;
            let inside = (0.0, 0.0, 0.3);
            let apex = ((s.jit(9.1) - 0.5) * 0.25, (s.jit(9.7) - 0.5) * 0.25, 0.9);
            let pt = |sc: &Sculptor, i: usize, rk: f32, spread: f32, z: f32| -> P3 {
                let a = i as f32 / n as f32 * TAU;
                let r = 0.32 + spread * sc.jit(i as f32 * 1.7 + rk);
                (a.cos() * r, a.sin() * r, z)
            };
            for i in 0..n {
                let j = (i + 1) % n;
                let b0 = pt(&s, i, 0.3, 0.2, 0.0);
                let b1 = pt(&s, j, 0.3, 0.2, 0.0);
                let t0 = mul(pt(&s, i, 4.3, 0.12, 0.0), 0.6);
                let t1 = mul(pt(&s, j, 4.3, 0.12, 0.0), 0.6);
                let (t0, t1) = ((t0.0, t0.1, 0.55), (t1.0, t1.1, 0.55));
                s.wall_quad(ROCK, b0, b1, t1, t0, inside);
                s.wall(ROCK, t0, t1, apex, inside);
            }
        }
        FeatureKind::Ruin => {
            s.boxy(STONE_DK, -0.5, -0.28, 0.0, 0.5, 0.28, 0.14);
            s.tube(STONE, (-0.3, 0.06, 0.12), 0.09, (-0.3, 0.06, 0.78), 0.08, 5, true);
            s.tube(STONE, (0.02, -0.08, 0.12), 0.09, (0.02, -0.08, 1.0), 0.08, 5, true);
            s.tube(STONE_DK, (0.32, 0.1, 0.12), 0.09, (0.32, 0.1, 0.55), 0.08, 5, true);
        }
        FeatureKind::Hut => {
            s.boxy(WALL, -0.4, -0.32, 0.0, 0.4, 0.32, 0.6);
            s.pyramid(ROOF, -0.5, -0.42, 0.5, 0.42, 0.55, (0.0, 0.0, 1.0));
        }
        FeatureKind::Cottage => {
            s.boxy(WALL, -0.5, -0.3, 0.0, 0.5, 0.3, 0.55);
            let inside = (0.0, 0.0, 0.55);
            // A gabled roof: two planes to a ridge, closed by end triangles.
            s.wall_quad(
                ROOF,
                (-0.55, -0.38, 0.5),
                (0.55, -0.38, 0.5),
                (0.48, 0.0, 0.92),
                (-0.48, 0.0, 0.92),
                inside,
            );
            s.wall_quad(
                ROOF,
                (0.55, 0.38, 0.5),
                (-0.55, 0.38, 0.5),
                (-0.48, 0.0, 0.92),
                (0.48, 0.0, 0.92),
                inside,
            );
            s.wall(WALL, (-0.5, -0.3, 0.55), (-0.5, 0.3, 0.55), (-0.48, 0.0, 0.92), inside);
            s.wall(WALL, (0.5, -0.3, 0.55), (0.5, 0.3, 0.55), (0.48, 0.0, 0.92), inside);
            s.boxy(STONE_DK, 0.22, -0.07, 0.62, 0.36, 0.07, 1.0);
        }
        FeatureKind::Tower => {
            s.tube(STONE, (0.0, 0.0, 0.0), 0.30, (0.0, 0.0, 0.84), 0.26, 6, false);
            // The parapet: a slightly wider lidded drum reads as the wall-walk.
            s.tube(STONE, (0.0, 0.0, 0.84), 0.30, (0.0, 0.0, 1.0), 0.30, 6, true);
        }
        FeatureKind::Dock => {
            s.boxy(WOOD, -0.5, -0.16, 0.42, 0.5, 0.16, 0.55);
            for (px, py) in
                [(-0.42, -0.11), (-0.42, 0.11), (0.0, -0.11), (0.0, 0.11), (0.42, -0.11), (0.42, 0.11)]
            {
                s.tube(WOOD_DK, (px, py, 0.0), 0.035, (px, py, 0.46), 0.035, 4, false);
            }
        }
        FeatureKind::Flag => {
            s.tube(POLE, (0.0, 0.0, 0.0), 0.045, (0.0, 0.0, 1.0), 0.03, 4, false);
            // The cloth ripples: two hinged panels waving on the frame clock.
            let ph = v.t * 2.6 + s.jit(3.3) * TAU;
            let y1 = ph.sin() * 0.07;
            let y2 = (ph + 1.8).sin() * 0.12;
            s.fin_quad(
                FLAGC,
                (0.03, 0.0, 0.98),
                (0.28, y1, 0.94),
                (0.28, y1, 0.72),
                (0.03, 0.0, 0.68),
            );
            s.fin_quad(
                FLAGC,
                (0.28, y1, 0.94),
                (0.5, y2, 0.90),
                (0.5, y2, 0.76),
                (0.28, y1, 0.72),
            );
        }
        FeatureKind::Shipwreck => {
            // A canted hull carcass, bow buried in the sand, and a snapped mast.
            s.tube(WRECK, (-0.5, 0.0, 0.0), 0.03, (0.05, 0.0, 0.16), 0.17, 5, false);
            s.tube(WRECK, (0.05, 0.0, 0.16), 0.17, (0.5, 0.02, 0.30), 0.10, 5, true);
            s.tube(WOOD_DK, (0.0, 0.0, 0.18), 0.035, (0.16, 0.10, 0.85), 0.015, 4, false);
        }
        FeatureKind::DeadTree => {
            s.tube(SNAG, (0.0, 0.0, 0.0), 0.06, (0.06, 0.0, 0.75), 0.035, 5, false);
            s.tube(SNAG, (0.03, 0.0, 0.5), 0.03, (0.3, 0.08, 0.88), 0.0, 4, false);
            s.tube(SNAG_DK, (0.0, 0.0, 0.62), 0.028, (-0.26, -0.06, 0.92), 0.0, 4, false);
            s.tube(SNAG_DK, (0.05, 0.0, 0.72), 0.025, (0.14, -0.1, 1.0), 0.0, 4, false);
        }
        FeatureKind::FlowerPatch => {
            // A low green mound dotted with bloom cones set on its slope.
            s.tube(MEADOW, (0.0, 0.0, 0.0), 0.5, (0.0, 0.0, 0.4), 0.0, 6, false);
            for (k, col) in [BLOOM_A, BLOOM_B, BLOOM_C, BLOOM_A].into_iter().enumerate() {
                let a = s.jit(k as f32 * 2.9 + 0.7) * TAU;
                let r = 0.10 + s.jit(k as f32 * 1.3 + 4.1) * 0.22;
                let zc = 0.4 * (1.0 - r / 0.5);
                let c = (a.cos() * r, a.sin() * r, zc);
                s.tube(col, c, 0.07, (c.0, c.1, zc + 0.24), 0.0, 4, false);
            }
        }
        FeatureKind::Reeds => {
            for k in 0..5 {
                let a = k as f32 / 5.0 * TAU + s.jit(0.9) * 2.0;
                let r = 0.10 + s.jit(k as f32 + 5.3) * 0.28;
                let (x, y) = (a.cos() * r, a.sin() * r);
                let lean = (s.jit(k as f32 + 7.7) - 0.5) * 0.2;
                let top = (x + lean, y + lean * 0.6, 0.8 + s.jit(k as f32 + 8.1) * 0.25);
                let col = if k % 2 == 0 { REED } else { REED_DK };
                s.tube(col, (x, y, 0.0), 0.035, top, 0.0, 3, false);
            }
        }
        FeatureKind::Cactus => {
            s.tube(CACTUS, (0.0, 0.0, 0.0), 0.15, (0.0, 0.0, 0.92), 0.11, 5, false);
            s.tube(CACTUS, (0.0, 0.0, 0.92), 0.11, (0.0, 0.0, 1.0), 0.0, 5, false);
            s.tube(CACTUS_DK, (-0.12, 0.0, 0.42), 0.07, (-0.3, 0.0, 0.46), 0.07, 4, false);
            s.tube(CACTUS_DK, (-0.3, 0.0, 0.46), 0.07, (-0.3, 0.0, 0.74), 0.0, 4, false);
            s.tube(CACTUS_DK, (0.12, 0.0, 0.52), 0.06, (0.28, 0.0, 0.56), 0.06, 4, false);
            s.tube(CACTUS_DK, (0.28, 0.0, 0.56), 0.06, (0.28, 0.0, 0.8), 0.0, 4, false);
        }
        FeatureKind::FallenLog => {
            s.tube(WOOD, (-0.5, 0.0, 0.16), 0.17, (0.44, 0.04, 0.13), 0.14, 5, true);
            // The moss cushion along the upper side.
            s.fin_quad(
                MOSS,
                (-0.45, -0.08, 0.30),
                (0.38, -0.05, 0.26),
                (0.38, 0.09, 0.25),
                (-0.45, 0.10, 0.29),
            );
        }
        FeatureKind::Cairn => {
            s.tube(ROCK, (0.0, 0.0, 0.0), 0.42, (0.0, 0.0, 0.3), 0.34, 5, true);
            s.tube(ROCK, (0.03, 0.0, 0.3), 0.30, (0.03, 0.0, 0.58), 0.22, 5, true);
            s.tube(ROCK_DK, (-0.02, 0.0, 0.58), 0.18, (-0.02, 0.0, 0.84), 0.12, 5, true);
            s.tube(ROCK, (0.0, 0.0, 0.84), 0.10, (0.0, 0.0, 1.0), 0.0, 4, false);
        }
        FeatureKind::StoneArch => {
            s.boxy(STONE, -0.5, -0.11, 0.0, -0.28, 0.11, 0.82);
            s.boxy(STONE, 0.28, -0.11, 0.0, 0.5, 0.11, 0.82);
            s.boxy(STONE_DK, -0.56, -0.13, 0.8, 0.56, 0.13, 1.0);
        }
        FeatureKind::LavaVent => {
            // A basalt cinder cone split by a glowing fissure, its crater a
            // full-bright ember disc (the flickering halo is the lamp pass).
            s.tube(BASALT, (0.0, 0.0, 0.0), 0.5, (0.0, 0.0, 0.55), 0.16, 6, false);
            for i in 0..6 {
                let a0 = i as f32 / 6.0 * TAU;
                let a1 = (i as f32 + 1.0) / 6.0 * TAU;
                let r = 0.13;
                s.glow(
                    EMBER,
                    (0.0, 0.0, 0.56),
                    (a0.cos() * r, a0.sin() * r, 0.53),
                    (a1.cos() * r, a1.sin() * r, 0.53),
                );
            }
            s.glow(EMBER, (-0.05, -0.46, 0.05), (0.05, -0.46, 0.05), (0.0, -0.17, 0.5));
        }
        FeatureKind::Campfire => {
            for k in 0..5 {
                let a = k as f32 / 5.0 * TAU + 0.2;
                let (x, y) = (a.cos() * 0.34, a.sin() * 0.34);
                s.tube(ROCK_DK, (x, y, 0.0), 0.09, (x, y, 0.14), 0.06, 4, false);
            }
            s.tube(WOOD_DK, (-0.3, -0.2, 0.02), 0.04, (0.3, 0.2, 0.2), 0.03, 4, false);
            s.tube(WOOD, (0.3, -0.2, 0.02), 0.04, (-0.3, 0.2, 0.2), 0.03, 4, false);
            // Two crossed flame fins with a hot core, full-bright.
            s.glow(FLAME, (-0.16, 0.0, 0.12), (0.16, 0.0, 0.12), (0.0, 0.0, 0.9));
            s.glow(FLAME, (0.0, -0.16, 0.12), (0.0, 0.16, 0.12), (0.02, 0.0, 0.88));
            s.glow(FLAME_HOT, (-0.08, 0.0, 0.12), (0.08, 0.0, 0.12), (0.0, 0.0, 0.55));
        }
        FeatureKind::Windmill => {
            s.tube(WALL, (0.0, 0.0, 0.0), 0.24, (0.0, 0.0, 0.7), 0.15, 6, false);
            s.tube(ROOF, (0.0, 0.0, 0.7), 0.17, (0.0, 0.0, 0.88), 0.0, 6, false);
            // The sail cross turns slowly on the frame clock, mounted proud of
            // one face so the blades never sink into the tower.
            let hub = (0.0, -0.18, 0.74);
            s.tube(WOOD_DK, (0.0, -0.10, 0.74), 0.05, hub, 0.0, 4, false);
            let spin = v.t * 0.6 + s.jit(6.1) * TAU;
            for k in 0..4 {
                let a = spin + k as f32 * FRAC_PI_2;
                let (dx, dz) = (a.cos(), a.sin());
                let inner = (hub.0 + dx * 0.07, hub.1, hub.2 + dz * 0.07);
                let outer = (hub.0 + dx * 0.34, hub.1, hub.2 + dz * 0.34);
                let p = (-dz * 0.05, 0.0, dx * 0.05);
                s.fin_quad(WOOD, add(inner, p), add(outer, p), sub(outer, p), sub(inner, p));
            }
        }
        FeatureKind::Lighthouse => {
            // The banded tower: red rings on a tapering white-washed drum.
            let r_at = |z: f32| 0.30 - z * (0.10 / 0.72);
            let bands: [(f32, f32, [f32; 3]); 5] = [
                (0.0, 0.20, WALL),
                (0.20, 0.34, ROOF),
                (0.34, 0.50, WALL),
                (0.50, 0.62, ROOF),
                (0.62, 0.72, WALL),
            ];
            for (z0, z1, col) in bands {
                s.tube(col, (0.0, 0.0, z0), r_at(z0), (0.0, 0.0, z1), r_at(z1), 6, false);
            }
            s.tube(STONE_DK, (0.0, 0.0, 0.72), 0.16, (0.0, 0.0, 0.9), 0.15, 6, false);
            s.tube(ROOF_DK, (0.0, 0.0, 0.9), 0.19, (0.0, 0.0, 1.0), 0.0, 6, false);
        }
    }
}
