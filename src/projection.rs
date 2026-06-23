//! First-person camera constants, ported from `shared.Projection`.
//!
//! The sea is a *cylindrical* projection of a flat ground plane (column =
//! bearing, row = depression angle). Only the constants the scene + wave system
//! need are ported here; island projection comes later.

/// Default horizontal half field of view (~94° total) at the design aspect ratio.
pub const HALF_FOV_H: f32 = 0.82;

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

/// Metres; nothing renders past this.
pub const MAX_VIEW: f32 = 5000.0;
