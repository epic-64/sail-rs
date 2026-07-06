//! Drawing floating salvage as true low-poly 3-D miniatures: a planked crate,
//! an oak cask and a brass-bound strongbox bobbing on the swell. Follows the
//! same treatment the islands' scenery ([`crate::feature_models`]) and the
//! rival ([`crate::rival_render`]) got: world-space triangles projected through
//! the shared cylindrical map, Lambert-shaded per face against the scene's
//! active light, painter-sorted within the piece (macroquad's 2D pass has no
//! depth buffer). Replaces the flat-shaded billboards (themselves stand-ins for
//! the original's `crate.svg` / `barrel.svg` / `chest.svg` sprites), so a piece
//! turns honestly as the ship sails around it instead of always facing camera.
//!
//! Each piece rides the local wave so it heaves with the same sea the player
//! does, tilts into the swell slope sampled along its own axes so it rocks on
//! the water rather than standing bolt upright, and turns idly adrift. Drawn
//! inside the wave march (see [`crate::ocean_renderer`]) so nearer crests and
//! islands occlude it like any other world object.

use macroquad::prelude::*;

use crate::flotsam::FlotsamKind;
use crate::geometry::{clamp, wrap_angle, Vec2};
use crate::ocean;
use crate::ocean_renderer::WAVE_GAIN;
use crate::projection::BASE_EYE;
use crate::sailing::Kinematics;
use crate::scene::SceneView;

use std::f32::consts::TAU;

const TOP_M: f32 = 3.0; // reference height (m) the size floor guards, as the billboards used
const MAG: f32 = 1.6; // drawn a touch larger than life so it stays spottable
const MIN_PX: f32 = 5.0; // floor so a distant speck of salvage stays visible
const FOV_MARGIN: f32 = 1.12; // matches the wave mesh's column fan
// Salvage you can realistically reach sits within a few hundred metres of the bow,
// so a piece beyond that should read as out of reach: it fades from full opacity at
// FADE_NEAR to nothing by FADE_FAR (and is not drawn past it), dissolving into the
// haze instead of hanging on the horizon as a crisp, never-closing speck.
const FADE_NEAR: f32 = 800.0;
const FADE_FAR: f32 = 1400.0;
/// Floor under the per-face Lambert term, the sky-fill washing shadowed faces
/// (the same value the islands and the rival use).
const AMBIENT: f32 = 0.45;

// Albedos, carried over from the billboard palette. Form now comes from the
// Lambert shading, so each material takes one base colour.
const CRATE_FACE: [f32; 3] = [150.0, 108.0, 64.0];
const CRATE_LID: [f32; 3] = [176.0, 132.0, 84.0];
const BATTEN: [f32; 3] = [86.0, 60.0, 34.0];
const STAVE: [f32; 3] = [168.0, 120.0, 66.0];
const HEAD: [f32; 3] = [186.0, 140.0, 82.0];
const HOOP: [f32; 3] = [70.0, 58.0, 48.0];
const BODY: [f32; 3] = [92.0, 60.0, 36.0];
const LID: [f32; 3] = [110.0, 74.0, 46.0];
const BRASS: [f32; 3] = [206.0, 166.0, 74.0];

/// A point in the piece's local frame: x/y on the chart (metres, spun by its
/// yaw), z up from the waterline.
type P3 = (f32, f32, f32);

/// A projected vertex: screen spot, bearing/range (clipping and the painter
/// sort) and world position (for the face normals), as the rival carries them.
#[derive(Clone, Copy)]
struct Vtx {
    sx: f32,
    sy: f32,
    phi: f32,
    d: f32,
    wx: f32,
    wy: f32,
    wz: f32,
}

/// glam's screen-space vector, under an explicit name: the world maths here
/// uses our [`crate::geometry::Vec2`], which shadows it.
type ScreenV = macroquad::math::Vec2;

/// A stable hash to [0, 1) (the sin-fract trick the scenery uses), keyed off
/// the piece's anchor so its yaw never changes frame to frame and no RNG
/// stream is consumed.
#[inline]
fn hash01(n: f32) -> f32 {
    let s = (n * 12.9898).sin() * 43758.5453;
    s - s.floor()
}

/// Triangle emitter for one piece: owns its frame on the water (anchor, yaw,
/// swell tilt, scale) and the shading/projection context, and collects shaded
/// screen triangles for the painter sort.
struct Bob<'a> {
    kin: &'a Kinematics,
    pos: Vec2,
    ax: Vec2, // local +x on the chart (spun by the piece's yaw)
    ay: Vec2, // local +y
    sa: f32,  // tilt about local y (lifts the +x end up the swell slope)
    ca: f32,
    sb: f32, // tilt about local x (lifts the +y end)
    cb: f32,
    s: f32, // model metres to world metres (magnification + distance floor)
    foot_disp: f32,
    alpha: f32,
    light: f32,
    sun: (f32, f32, f32),
    w: f32,
    horizon: f32,
    px_per_rad: f32,
    px_per_rad_h: f32,
    phi_clip: f32,
    prims: Vec<(f32, [ScreenV; 3], Color)>,
}

impl Bob<'_> {
    /// A local point through the piece's swell tilt and yaw into the world,
    /// then through the same cylindrical map the waves use (the `cos(phi)`
    /// factor matches how the billboards sat on the painted sea).
    fn vert(&self, p: P3) -> Vtx {
        let x1 = p.0 * self.ca - p.2 * self.sa;
        let zt = p.0 * self.sa + p.2 * self.ca;
        let y1 = p.1 * self.cb - zt * self.sb;
        let z2 = p.1 * self.sb + zt * self.cb;
        let wp = self.pos + self.ax * (x1 * self.s) + self.ay * (y1 * self.s);
        let elev = z2 * self.s;
        let dv = self.kin.pos.distance_to(wp).max(1.0);
        let phi = wrap_angle(self.kin.pos.bearing_to(wp) - self.kin.heading_rad);
        Vtx {
            sx: self.w * 0.5 + phi * self.px_per_rad_h,
            sy: self.horizon
                + ((BASE_EYE - self.foot_disp - elev) * phi.cos() / dv).atan() * self.px_per_rad,
            phi,
            d: dv,
            wx: wp.x,
            wy: wp.y,
            wz: elev,
        }
    }

    /// One face of a closed surface: its normal is oriented outward (away from
    /// `inside`), back faces are culled (so the far side never shows through
    /// the distance fade), and it is lit one-sided against the scene's light
    /// with the ambient floor. Faces smeared by the projection (hard against
    /// the camera or swung far outside the drawn fan) are dropped.
    fn wall(&mut self, base: [f32; 3], a: P3, b: P3, c: P3, inside: P3) {
        let va = self.vert(a);
        let vb = self.vert(b);
        let vc = self.vert(c);
        if va.d.min(vb.d).min(vc.d) < 1.0
            || va.phi.abs().max(vb.phi.abs()).max(vc.phi.abs()) > self.phi_clip
        {
            return;
        }
        let e1 = (vb.wx - va.wx, vb.wy - va.wy, vb.wz - va.wz);
        let e2 = (vc.wx - va.wx, vc.wy - va.wy, vc.wz - va.wz);
        let mut n = (
            e1.1 * e2.2 - e1.2 * e2.1,
            e1.2 * e2.0 - e1.0 * e2.2,
            e1.0 * e2.1 - e1.1 * e2.0,
        );
        let cen = (
            (va.wx + vb.wx + vc.wx) / 3.0,
            (va.wy + vb.wy + vc.wy) / 3.0,
            (va.wz + vb.wz + vc.wz) / 3.0,
        );
        let vi = self.vert(inside);
        if n.0 * (cen.0 - vi.wx) + n.1 * (cen.1 - vi.wy) + n.2 * (cen.2 - vi.wz) < 0.0 {
            n = (-n.0, -n.1, -n.2);
        }
        let to_eye = (
            self.kin.pos.x - cen.0,
            self.kin.pos.y - cen.1,
            BASE_EYE - cen.2,
        );
        if n.0 * to_eye.0 + n.1 * to_eye.1 + n.2 * to_eye.2 <= 0.0 {
            return; // back face of a closed shape: never visible
        }
        let nl = (n.0 * n.0 + n.1 * n.1 + n.2 * n.2).sqrt().max(1e-6);
        let diff = ((n.0 * self.sun.0 + n.1 * self.sun.1 + n.2 * self.sun.2) / nl).max(0.0);
        let m = self.light * (AMBIENT + (1.0 - AMBIENT) * diff);
        let col = Color::new(
            base[0] / 255.0 * m,
            base[1] / 255.0 * m,
            base[2] / 255.0 * m,
            self.alpha,
        );
        let depth = (va.d + vb.d + vc.d) / 3.0;
        self.prims.push((
            depth,
            [vec2(va.sx, va.sy), vec2(vb.sx, vb.sy), vec2(vc.sx, vc.sy)],
            col,
        ));
    }

    fn wall_quad(&mut self, base: [f32; 3], a: P3, b: P3, c: P3, d: P3, inside: P3) {
        self.wall(base, a, b, c, inside);
        self.wall(base, a, c, d, inside);
    }
}

/// Draw one piece of salvage on the water, or nothing if it is out of view.
/// Called inside the world-camera pass (interleaved with the wave bands) so it
/// rides the camera ride and sits on the painted sea. The scene's `light` and
/// `sun` shade it into the night with everything else.
pub fn draw(pos: Vec2, kind: FlotsamKind, view: &SceneView) {
    let SceneView {
        kin,
        t,
        sea,
        heave,
        light,
        horizon,
        px_per_rad,
        half_fov_h_view,
        w,
        sun,
        ..
    } = *view;
    let d = kin.pos.distance_to(pos);
    if !(1.0..=FADE_FAR).contains(&d) {
        return;
    }
    let phi = wrap_angle(kin.pos.bearing_to(pos) - kin.heading_rad);
    if phi.abs() > half_fov_h_view * FOV_MARGIN {
        return;
    }
    let px_per_rad_h = (w * 0.5) / half_fov_h_view;

    // Foot on the local wave surface (gained like the sea); the piece's own
    // metres are not gained, matching how the islands stand on the water.
    let wave = ocean::height(pos, t, sea);
    let foot_disp = (wave - heave) * WAVE_GAIN;

    // The magnification: larger than life, grown further when the reference
    // height would drop under the visibility floor, so far salvage scales up
    // as one shape instead of degenerating to a smear.
    let cphi = phi.cos();
    let foot_y = horizon + ((BASE_EYE - foot_disp) * cphi / d).atan() * px_per_rad;
    let top_y = horizon + ((BASE_EYE - foot_disp - TOP_M) * cphi / d).atan() * px_per_rad;
    let raw_h = (foot_y - top_y).max(0.1);
    let s = MAG * (MIN_PX / raw_h).max(1.0);
    // Fade out with distance so far, unreachable salvage dissolves into the haze
    // rather than reading as a crisp speck that never grows.
    let alpha = clamp((FADE_FAR - d) / (FADE_FAR - FADE_NEAR), 0.0, 1.0);

    // A stable per-piece yaw hashed from its anchor (a piece never moves), plus
    // a slow drift spin so it turns idly on the water instead of holding one
    // frozen aspect.
    let h0 = hash01(pos.x * 0.173 + pos.y * 0.391);
    let yaw = h0 * TAU + t * (h0 - 0.5) * 0.25;
    let (ys, yc) = yaw.sin_cos();
    let ax = Vec2::new(yc, ys);
    let ay = Vec2::new(-ys, yc);

    // Tilt into the local swell slope sampled along its own axes (by the same
    // gain the mesh uses) so it rocks on the waves rather than standing bolt
    // upright.
    let span = 3.0;
    let slope = |axis: Vec2| -> f32 {
        let hi = ocean::height(pos + axis * span, t, sea);
        let lo = ocean::height(pos - axis * span, t, sea);
        clamp(((hi - lo) * WAVE_GAIN / (2.0 * span)).atan() * 0.8, -0.5, 0.5)
    };
    let (sa, ca) = slope(ax).sin_cos();
    let (sb, cb) = slope(ay).sin_cos();

    let mut bob = Bob {
        kin,
        pos,
        ax,
        ay,
        sa,
        ca,
        sb,
        cb,
        s,
        foot_disp,
        alpha,
        light,
        sun,
        w,
        horizon,
        px_per_rad,
        px_per_rad_h,
        phi_clip: half_fov_h_view * 1.35,
        prims: Vec::with_capacity(96),
    };
    match kind {
        FlotsamKind::Crate => build_crate(&mut bob),
        FlotsamKind::Barrel => build_barrel(&mut bob),
        FlotsamKind::Chest => build_chest(&mut bob),
    }

    // Painter order within the piece: farthest faces first, so the near side
    // covers the far (ties keep emission order: the sort is stable, so a
    // batten drawn after its wall stays on top of it).
    bob.prims.sort_by(|x, y| y.0.total_cmp(&x.0));
    for (_, p, col) in bob.prims {
        draw_triangle(p[0], p[1], p[2], col);
    }
}

// --- the three kinds, each a small closed solid awash at the waterline --------

/// A planked wooden crate: a near-cube riding low in the water, a pale lid
/// catching the sky, and an X of two battens nailed proud of every wall.
fn build_crate(b: &mut Bob) {
    const H: f32 = 1.0; // half-footprint (m)
    const Z0: f32 = -0.45; // keel, awash below the waterline
    const Z1: f32 = 1.65; // lid
    let inside = (0.0, 0.0, (Z0 + Z1) * 0.5);
    let corners = [(-H, -H), (H, -H), (H, H), (-H, H)];
    for i in 0..4 {
        let (x0, y0) = corners[i];
        let (x1, y1) = corners[(i + 1) % 4];
        b.wall_quad(CRATE_FACE, (x0, y0, Z0), (x1, y1, Z0), (x1, y1, Z1), (x0, y0, Z1), inside);
    }
    b.wall_quad(CRATE_LID, (-H, -H, Z1), (H, -H, Z1), (H, H, Z1), (-H, H, Z1), inside);

    // Cross-battens proud of each wall. `face(t, z)` maps the wall's tangent
    // coordinate and height onto the wall plane pushed out by `P`.
    const P: f32 = H + 0.05;
    let faces: [&dyn Fn(f32, f32) -> P3; 4] = [
        &|t, z| (t, -P, z),
        &|t, z| (P, t, z),
        &|t, z| (-t, P, z),
        &|t, z| (-P, -t, z),
    ];
    let (a, zb, zt, k) = (0.80, Z0 + 0.22, Z1 - 0.18, 0.26);
    for f in faces {
        b.wall_quad(BATTEN, f(-a, zb + k), f(-a + k, zb), f(a, zt - k), f(a - k, zt), inside);
        b.wall_quad(BATTEN, f(a, zb + k), f(a - k, zb), f(-a, zt - k), f(-a + k, zt), inside);
    }
}

/// An oak cask floating on end: a lathed body (narrow chimes, fat belly)
/// banded by two dark iron hoops, the pale end-grain of the head on top.
fn build_barrel(b: &mut Bob) {
    // Rims from keel chime to head: (z, radius, colour of the band above this
    // rim; the last entry's colour is unused).
    const RIMS: [(f32, f32, [f32; 3]); 7] = [
        (-0.50, 0.58, STAVE),
        (0.10, 0.74, HOOP),
        (0.34, 0.79, STAVE),
        (0.72, 0.83, STAVE), // the belly
        (1.10, 0.79, HOOP),
        (1.34, 0.74, STAVE),
        (2.05, 0.56, STAVE),
    ];
    const SIDES: usize = 7;
    let rim = |i: usize, r: f32, z: f32| -> P3 {
        let a = i as f32 / SIDES as f32 * TAU;
        (a.cos() * r, a.sin() * r, z)
    };
    for k in 0..RIMS.len() - 1 {
        let (z0, r0, col) = RIMS[k];
        let (z1, r1, _) = RIMS[k + 1];
        let inside = (0.0, 0.0, (z0 + z1) * 0.5);
        for i in 0..SIDES {
            let j = (i + 1) % SIDES;
            b.wall_quad(col, rim(i, r0, z0), rim(j, r0, z0), rim(j, r1, z1), rim(i, r1, z1), inside);
        }
    }
    let (zt, rt, _) = RIMS[RIMS.len() - 1];
    let inside = (0.0, 0.0, zt - 0.4);
    for i in 0..SIDES {
        let j = (i + 1) % SIDES;
        b.wall(HEAD, (0.0, 0.0, zt), rim(i, rt, zt), rim(j, rt, zt), inside);
    }
}

/// A half-sunk strongbox: a dark hardwood chest under a domed lid that
/// overhangs its body, a brass band over the front and lid with a proud
/// lock-plate: the rare, valuable salvage.
fn build_chest(b: &mut Bob) {
    const HX: f32 = 1.20; // half-width (the long side)
    const HY: f32 = 0.75; // half-depth
    const Z0: f32 = -0.35; // keel, awash
    const ZT: f32 = 0.95; // the lid seam
    const BR: f32 = 0.15; // half-width of the brass band
    let inside = (0.0, 0.0, 0.45);

    // Body walls; the front is split into strips so the brass band is its own
    // faces (no coplanar overdraw to flicker in the painter sort).
    for (x0, x1, col) in [(-HX, -BR, BODY), (-BR, BR, BRASS), (BR, HX, BODY)] {
        b.wall_quad(col, (x0, -HY, Z0), (x1, -HY, Z0), (x1, -HY, ZT), (x0, -HY, ZT), inside);
    }
    b.wall_quad(BODY, (-HX, HY, Z0), (HX, HY, Z0), (HX, HY, ZT), (-HX, HY, ZT), inside);
    b.wall_quad(BODY, (-HX, -HY, Z0), (-HX, HY, Z0), (-HX, HY, ZT), (-HX, -HY, ZT), inside);
    b.wall_quad(BODY, (HX, -HY, Z0), (HX, HY, Z0), (HX, HY, ZT), (HX, -HY, ZT), inside);

    // The domed lid: an arc profile (front to back) run along the width,
    // overhanging the body a touch, with fan-closed ends. The brass band
    // carries on over the dome.
    const HL: f32 = HX + 0.06; // lid half-width (the overhang)
    const ARC: [(f32, f32); 5] =
        [(-0.82, ZT), (-0.55, 1.30), (0.0, 1.48), (0.55, 1.30), (0.82, ZT)];
    let lid_in = (0.0, 0.0, ZT);
    for k in 0..ARC.len() - 1 {
        let (y0, z0) = ARC[k];
        let (y1, z1) = ARC[k + 1];
        for (x0, x1, col) in [(-HL, -BR, LID), (-BR, BR, BRASS), (BR, HL, LID)] {
            b.wall_quad(col, (x0, y0, z0), (x1, y0, z0), (x1, y1, z1), (x0, y1, z1), lid_in);
        }
    }
    for x in [-HL, HL] {
        for k in 1..ARC.len() - 1 {
            let (y0, z0) = ARC[k];
            let (y1, z1) = ARC[k + 1];
            b.wall(LID, (x, ARC[0].0, ARC[0].1), (x, y0, z0), (x, y1, z1), lid_in);
        }
    }

    // The lock-plate, proud of the brass band at the lid seam.
    b.wall_quad(
        BRASS,
        (-0.24, -HY - 0.06, 0.55),
        (0.24, -HY - 0.06, 0.55),
        (0.24, -HY - 0.06, 0.90),
        (-0.24, -HY - 0.06, 0.90),
        inside,
    );
}
