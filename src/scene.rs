//! Per-frame camera/scene context shared by the world renderers.
//!
//! Bundles the projection and animation state a renderer needs to place a point
//! on the painted sea ([`SceneView`]) or on the sky dome ([`SkyView`]), so the
//! billboard, wave and sky renderers each take one context value instead of a
//! long tail of positional floats. Both are `Copy`; a renderer destructures the
//! fields it needs at entry, so its body reads exactly as before.

use crate::geometry::Vec2;
use crate::sailing::Kinematics;

/// The sea-anchored camera for one frame: the eye (`kin`), the wave field to
/// sample (`t`, `sea`, `heave`), the scene `light`, and the cylindrical
/// projection onto the screen. Built once in [`crate::ocean_renderer`] and handed
/// to the billboard and flow renderers, each of which reads the fields it needs.
#[derive(Clone, Copy)]
pub struct SceneView<'a> {
    pub kin: &'a Kinematics,
    pub t: f32,
    pub sea: f32,
    pub heave: f32,
    pub light: f32,
    pub horizon: f32,
    pub px_per_rad: f32,
    pub px_per_rad_h: f32,
    pub half_fov_h_view: f32,
    pub fwd: Vec2,
    pub right: Vec2,
    pub w: f32,
    pub h: f32,
}

/// The sky-dome projection for one frame: the bow `heading`, half field of view
/// and viewport that map an (azimuth, altitude) above the horizon onto the
/// screen. Shared by the sky gradient ([`crate::main`]) and the celestial bodies.
#[derive(Clone, Copy)]
pub struct SkyView {
    pub heading: f32,
    pub half_fov_h: f32,
    pub w: f32,
    pub horizon: f32,
}
