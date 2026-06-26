//! The reflection road for a port's harbour lights: after dusk the lit town on a
//! port island casts a shimmering road of light down the water toward the viewer,
//! the way the moon's glitter road does. (The lamps themselves are tiny per-house
//! lights drawn on the island in [`crate::islands_render`]; this module is only
//! their pooled reflection on the sea.)
//!
//! Rather than draw the road as separate sparkles, we feed the town to the wave
//! mesh as an extra **local light** and let [`crate::ocean_renderer`] shade the
//! water quads with it, exactly as it does the sun: a warm diffuse term lights the
//! swell faces turned toward the town and a Blinn-Phong specular breaks into a
//! glitter road toward the eye. The lamp sits low over the shore, so (like a sun
//! near the horizon) its road runs long and grazing across the sea. Because the
//! road is baked into the same far-to-near band march, nearer crests paint over it
//! for free and it sinks behind the swell rather than shining through it. Purely
//! decorative: it touches no world state and no determinism.

use crate::celestial::Sky;
use crate::geometry::{clamp, wrap_angle, Vec2};
use crate::isle_features::{FeatureKind, IsleFeature};
use crate::projection::MAX_VIEW;
use crate::sailing::Kinematics;
use crate::world::Island;

/// Warm-white colour of the pooled town reflection (a blend of the mixed lamps).
pub const ROAD_COL: (f32, f32, f32) = (255.0, 198.0, 142.0);

/// How high over the sea the town's pooled light sits (metres). Kept low so the
/// road grazes long across the water like a sun near the horizon rather than
/// pooling in a tight disc under the shore.
const LAMP_HEIGHT: f32 = 8.0;
/// Reference range (squared) for the inverse-falloff: the road is brightest by the
/// town and eases off toward the viewer, half-strength about this far from the lamp.
const LAMP_REF2: f32 = 360.0 * 360.0;
/// Gain on the warm diffuse pool (the swell faces turned toward the town).
const LAMP_DIFFUSE: f32 = 1.15;
/// Gain on the Blinn-Phong glitter road toward the eye.
const LAMP_SPEC: f32 = 1.4;
/// Specular tightness of the glitter road (lower than the sun's needle so the town
/// reads as a shimmering pool, not a pin-point).
const LAMP_SHININESS: f32 = 48.0;
/// Half-vector gate below which the specular is zero (skips the cheap-but-pointless
/// `powf` on facets nowhere near the road).
const SPEC_GATE: f32 = 0.84;

/// How strongly the harbour lights burn at clock `sun_alt` (the sine of the sun's
/// altitude): dark by day, ramping on through dusk to full once the sun is well
/// down. 0 while the sun is up, 1 by the time it has sunk a little below the horizon.
/// Shared with [`crate::islands_render`], which lights the houses by the same clock.
pub fn dusk_glow(sun_alt: f32) -> f32 {
    clamp((0.06 - sun_alt) / 0.18, 0.0, 1.0)
}

/// One port's town light in world space: where its pooled glow sits and how hard it
/// burns this frame (the dusk ramp, eased out for distant ports, with a gentle
/// town-wide flicker). [`camera_frame`] turns these into the per-frame [`CamLight`]s
/// the wave shader consumes.
pub struct PortLight {
    pub pos: Vec2,
    pub burn: f32,
}

/// A town light resolved into the sea camera's `(right, forward, up)` frame, ready
/// to shade against each wave facet's normal in the band march.
pub struct CamLight {
    cx: f32,
    cy: f32,
    cz: f32,
    burn: f32,
}

/// Gather the town lights of every visible port after dusk (one per island). Returns
/// nothing by day, so the wave shader pays no harbour cost until the lamps come on.
/// `kin` is the view kinematics (carries the look-astern heading); `t` drives the
/// town-wide flicker.
pub fn build(
    islands: &[Island],
    features: &[Vec<IsleFeature>],
    kin: &Kinematics,
    sky: &Sky,
    t: f32,
    half_fov_h_view: f32,
) -> Vec<PortLight> {
    let glow = dusk_glow(sky.sun_alt);
    let mut out: Vec<PortLight> = Vec::new();
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
        if rel.abs() > half_fov_h_view * 1.3 {
            continue;
        }
        // Hold the road bright across the field, easing it out only at the far edge so
        // a port on the horizon still shows a glimmer rather than popping off.
        let fade = clamp((MAX_VIEW - d) / 1500.0, 0.0, 1.0);
        // A gentle flicker so the whole town's pool wavers on the water.
        let flick = 0.85 + 0.15 * (t * 2.3 + isle.id as f32 * 1.7).sin();
        let burn = glow * fade * flick;
        if burn <= 0.01 {
            continue;
        }
        // Anchor the road on the port's watchtower beacon (its lighthouse) rather than
        // the island centre, so the glitter trails from the light itself. Falls back to
        // the centre if a port somehow has no tower.
        let src = features
            .get(isle.id as usize)
            .and_then(|fs| fs.iter().find(|f| f.kind == FeatureKind::Tower))
            .map(|f| isle.pos + f.offset)
            .unwrap_or(isle.pos);
        out.push(PortLight { pos: src, burn });
    }
    out
}

/// Resolve the world-space town lights into the sea camera's `(right, forward, up)`
/// frame, so the band march can shade each wave facet against them without re-running
/// the projection per facet. `fwd`/`right` are the camera basis the waves use.
pub fn camera_frame(lights: &[PortLight], kin: &Kinematics, fwd: Vec2, right: Vec2) -> Vec<CamLight> {
    lights
        .iter()
        .map(|pl| {
            let rel = pl.pos - kin.pos;
            CamLight {
                cx: rel.dot(right),
                cy: rel.dot(fwd),
                cz: LAMP_HEIGHT,
                burn: pl.burn,
            }
        })
        .collect()
}

impl CamLight {
    /// The town light's contribution to one wave facet, in the same camera frame the
    /// wave shading uses: a warm diffuse pool plus a Blinn-Phong glitter road toward
    /// the eye, attenuated by range from the lamp. `(sx, fy, mz)` is the facet centre
    /// (right, forward, up), `n*` its surface normal, `v*` the unit view ray from the
    /// facet back to the eye. Returns a 0..~1 road weight to blend toward [`ROAD_COL`].
    #[inline]
    #[allow(clippy::too_many_arguments)]
    pub fn on_facet(
        &self,
        sx: f32,
        fy: f32,
        mz: f32,
        nx: f32,
        ny: f32,
        nz: f32,
        vx: f32,
        vy: f32,
        vz: f32,
    ) -> f32 {
        // Facet -> lamp vector and its range.
        let lvx = self.cx - sx;
        let lvy = self.cy - fy;
        let lvz = self.cz - mz;
        let d2 = lvx * lvx + lvy * lvy + lvz * lvz;
        if d2 < 1.0 {
            return 0.0;
        }
        let inv = 1.0 / d2.sqrt();
        let lx = lvx * inv;
        let ly = lvy * inv;
        let lz = lvz * inv;
        // Warm diffuse: faces tilted toward the town light up, the lee falls dark.
        let ndl = nx * lx + ny * ly + nz * lz;
        let diff = ndl.max(0.0);
        // Blinn-Phong specular: the half-vector between the lamp and the eye sparkles
        // along the road that runs from the shore back toward the viewer. Gated to the
        // lit side (`ndl > 0`): a facet on the near face of an intervening swell turns
        // its back to a lamp that sits *beyond* the crest, so it cannot mirror the town
        // toward the eye and must not catch the road, even where its slope would.
        let spec = if ndl > 0.0 {
            let hx = lx + vx;
            let hy = ly + vy;
            let hz = lz + vz;
            let hl = (hx * hx + hy * hy + hz * hz).sqrt();
            if hl > 1e-4 {
                let ndh = clamp((nx * hx + ny * hy + nz * hz) / hl, 0.0, 1.0);
                if ndh > SPEC_GATE {
                    ndh.powf(LAMP_SHININESS)
                } else {
                    0.0
                }
            } else {
                0.0
            }
        } else {
            0.0
        };
        let atten = LAMP_REF2 / (LAMP_REF2 + d2);
        (LAMP_DIFFUSE * diff + LAMP_SPEC * spec) * atten * self.burn
    }
}
