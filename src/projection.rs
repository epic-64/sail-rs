//! First-person camera constants, ported from `shared.Projection`.
//!
//! The sea is a *cylindrical* projection of a flat ground plane (column =
//! bearing, row = depression angle). Only the constants the scene + wave system
//! need are ported here; island projection comes later.

/// Cap on the live half-FOV (~60° to the side, ~120° total). Past this the
/// cylindrical ground-plane projection degenerates (near water rears up to the
/// horizon at the sides), so a wider window stretches the capped span instead.
pub const MAX_HALF_FOV_H: f32 = 1.05;

/// Metres above the sea, used for summit elevation of distant features.
pub const EYE_HEIGHT: f32 = 2.2;

/// Virtual eye height for the *waterline* depression: the camera rides low,
/// almost at the waterline, so the sea barely tilts down and the swell can rise
/// up over the horizon.
pub const BASE_EYE: f32 = 7.0;

/// Metres the shoreline is lifted above the true sea so the shore reads as a
/// beach a touch proud of the water rather than awash at the mean waterline.
pub const SHORE_LIFT: f32 = 2.0;

/// Metres; nothing renders past this. Objects sink under the horizon (see
/// [`curve_dip`]) a little before reaching it, so the cull itself is never seen.
pub const MAX_VIEW: f32 = 6000.0;

/// Range (m) at which the fake planetary curvature begins to bite. Nearer than
/// this a hull or shore sits square on the water; beyond it the world starts to
/// sink below the swell. See [`curve_dip`].
pub const CURVE_START: f32 = 2500.0;

/// The extra depression (radians) the curve has piled on by [`MAX_VIEW`]: enough
/// that the tallest isle and a distant sail are wholly swallowed a little before
/// the cull, so distance removes them by sinking rather than the old fade-out.
/// The ramp is quadratic from [`CURVE_START`], so it is gentle at first and only
/// bites in the last couple of kilometres.
pub const CURVE_DIP_MAX: f32 = 0.056;

/// Extra downward depression (radians) added to every world point at range `dist`
/// to fake the planet's curve: distant islands and ships sink hull-first under the
/// horizon (the nearer, opaque water swallows them from the waterline up) instead
/// of fading to nothing. Zero within [`CURVE_START`], then growing with the square
/// of the distance past it up to [`CURVE_DIP_MAX`] at [`MAX_VIEW`].
#[inline]
pub fn curve_dip(dist: f32) -> f32 {
    let frac = ((dist - CURVE_START) / (MAX_VIEW - CURVE_START)).max(0.0);
    CURVE_DIP_MAX * frac * frac
}
