//! Plain 2D world-chart vectors and a couple of scalar helpers, ported from the
//! Scala `shared.Vec2` / `shared.Ship` so the rest of the port reads the same.
//!
//! This `Vec2` is deliberately *ours* (not glam's `Vec2` from the macroquad
//! prelude): it carries the game's bearing/heading conventions (0 = north,
//! clockwise) that the projection and ocean maths depend on. Modules that glob
//! `macroquad::prelude::*` also `use crate::geometry::Vec2;` — the explicit
//! import shadows glam's, which is what we want.

use std::ops::{Add, Mul, Sub};
use std::f32::consts::{PI, TAU};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Vec2 {
    pub const ZERO: Vec2 = Vec2 { x: 0.0, y: 0.0 };

    pub fn new(x: f32, y: f32) -> Self {
        Vec2 { x, y }
    }

    pub fn dot(self, o: Vec2) -> f32 {
        self.x * o.x + self.y * o.y
    }

    pub fn length(self) -> f32 {
        self.x.hypot(self.y)
    }

    pub fn distance_to(self, o: Vec2) -> f32 {
        (self.x - o.x).hypot(self.y - o.y)
    }

    /// Compass-style bearing (radians) from this point toward another.
    pub fn bearing_to(self, o: Vec2) -> f32 {
        (o.x - self.x).atan2(o.y - self.y)
    }

    /// Unit vector for a heading where 0 = north (+y), increasing clockwise.
    pub fn from_heading(rad: f32) -> Vec2 {
        Vec2 {
            x: rad.sin(),
            y: rad.cos(),
        }
    }
}

impl Add for Vec2 {
    type Output = Vec2;
    fn add(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x + o.x, self.y + o.y)
    }
}

impl Sub for Vec2 {
    type Output = Vec2;
    fn sub(self, o: Vec2) -> Vec2 {
        Vec2::new(self.x - o.x, self.y - o.y)
    }
}

impl Mul<f32> for Vec2 {
    type Output = Vec2;
    fn mul(self, k: f32) -> Vec2 {
        Vec2::new(self.x * k, self.y * k)
    }
}

/// `shared.Ship.clamp`.
pub fn clamp(v: f32, lo: f32, hi: f32) -> f32 {
    v.max(lo).min(hi)
}

/// `shared.Ship.wrapAngle` — fold an angle into (-π, π].
pub fn wrap_angle(a: f32) -> f32 {
    let m = a % TAU;
    if m > PI {
        m - TAU
    } else if m < -PI {
        m + TAU
    } else {
        m
    }
}
