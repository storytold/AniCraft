//! AniCraft geometry (layer L0): points, affine transforms, rectangles and Bézier paths.
//!
//! Everything here is pure math with no allocation beyond the path buffers, deterministic
//! across platforms (plain `f32`, no fused ops that depend on the target).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod affine;
pub mod brush;
pub mod clip;
pub mod edit;
pub mod envelope;
pub mod morph;
pub mod planar;
pub mod pressure;
pub mod raw;
pub mod slice;
mod fit;
mod path;
pub mod svg;
pub mod trace;
pub mod walk;

pub use affine::{Affine, Decomposed};
pub use fit::fit_curve;
pub use path::{FillRule, POLYSTAR_MAX_SIDES, Path, PathCmd, Polyline, polystar_corners, segment_distance, star_inner_radius};

use serde::{Deserialize, Serialize};
use std::ops::{Add, Mul, Neg, Sub};

/// A 2D point or vector.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Vec2 {
    pub const ZERO: Vec2 = Vec2 { x: 0.0, y: 0.0 };

    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
    pub fn dot(self, o: Vec2) -> f32 {
        self.x * o.x + self.y * o.y
    }
    pub fn cross(self, o: Vec2) -> f32 {
        self.x * o.y - self.y * o.x
    }
    pub fn length(self) -> f32 {
        self.dot(self).sqrt()
    }
    pub fn normalized(self) -> Vec2 {
        let l = self.length();
        if l > 1e-12 { self * (1.0 / l) } else { Vec2::ZERO }
    }
    /// Perpendicular (rotated +90° in y-down screen space).
    pub fn perp(self) -> Vec2 {
        Vec2::new(-self.y, self.x)
    }
    pub fn lerp(self, o: Vec2, t: f32) -> Vec2 {
        self + (o - self) * t
    }
    pub fn distance(self, o: Vec2) -> f32 {
        (o - self).length()
    }
    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite()
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
    fn mul(self, s: f32) -> Vec2 {
        Vec2::new(self.x * s, self.y * s)
    }
}
impl Neg for Vec2 {
    type Output = Vec2;
    fn neg(self) -> Vec2 {
        Vec2::new(-self.x, -self.y)
    }
}

/// Axis-aligned rectangle (min inclusive, max exclusive in raster use).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub min: Vec2,
    pub max: Vec2,
}

impl Rect {
    /// The empty rectangle: union identity.
    pub const EMPTY: Rect = Rect {
        min: Vec2 { x: f32::INFINITY, y: f32::INFINITY },
        max: Vec2 { x: f32::NEG_INFINITY, y: f32::NEG_INFINITY },
    };

    pub fn new(x0: f32, y0: f32, x1: f32, y1: f32) -> Self {
        Rect { min: Vec2::new(x0.min(x1), y0.min(y1)), max: Vec2::new(x0.max(x1), y0.max(y1)) }
    }
    pub fn from_points(a: Vec2, b: Vec2) -> Self {
        Rect::new(a.x, a.y, b.x, b.y)
    }
    pub fn is_empty(&self) -> bool {
        !(self.max.x > self.min.x && self.max.y > self.min.y)
    }
    pub fn width(&self) -> f32 {
        (self.max.x - self.min.x).max(0.0)
    }
    pub fn height(&self) -> f32 {
        (self.max.y - self.min.y).max(0.0)
    }
    pub fn center(&self) -> Vec2 {
        (self.min + self.max) * 0.5
    }
    pub fn include(&mut self, p: Vec2) {
        self.min.x = self.min.x.min(p.x);
        self.min.y = self.min.y.min(p.y);
        self.max.x = self.max.x.max(p.x);
        self.max.y = self.max.y.max(p.y);
    }
    pub fn union(&self, o: &Rect) -> Rect {
        Rect {
            min: Vec2::new(self.min.x.min(o.min.x), self.min.y.min(o.min.y)),
            max: Vec2::new(self.max.x.max(o.max.x), self.max.y.max(o.max.y)),
        }
    }
    pub fn intersect(&self, o: &Rect) -> Rect {
        Rect {
            min: Vec2::new(self.min.x.max(o.min.x), self.min.y.max(o.min.y)),
            max: Vec2::new(self.max.x.min(o.max.x), self.max.y.min(o.max.y)),
        }
    }
    pub fn inflate(&self, d: f32) -> Rect {
        Rect { min: self.min - Vec2::new(d, d), max: self.max + Vec2::new(d, d) }
    }
    pub fn contains(&self, p: Vec2) -> bool {
        p.x >= self.min.x && p.x <= self.max.x && p.y >= self.min.y && p.y <= self.max.y
    }
    /// Bounds of this rectangle after a transform.
    pub fn transformed(&self, m: &Affine) -> Rect {
        // Only a rectangle with no points stays as it is: a flat one (a horizontal or vertical
        // line's bounds) still moves with the transform.
        if !(self.min.x <= self.max.x && self.min.y <= self.max.y) {
            return *self;
        }
        let mut r = Rect::EMPTY;
        for p in [self.min, Vec2::new(self.max.x, self.min.y), self.max, Vec2::new(self.min.x, self.max.y)] {
            r.include(m.apply(p));
        }
        r
    }
}

/// Linear interpolation of scalars.
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_rects_transform_and_empty_ones_stay_empty() {
        let flat = Rect::new(-10.0, 0.0, 10.0, 0.0);
        let r = flat.transformed(&Affine::translate(100.0, 50.0));
        assert_eq!((r.min, r.max), (Vec2::new(90.0, 50.0), Vec2::new(110.0, 50.0)));
        assert!(r.inflate(2.0).contains(Vec2::new(100.0, 51.0)));
        let e = Rect::EMPTY.transformed(&Affine::translate(1.0, 1.0));
        assert!(e.is_empty() && e.min.x.is_infinite());
    }

    #[test]
    fn rect_union_and_intersect() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(5.0, 5.0, 20.0, 20.0);
        assert_eq!(a.union(&b), Rect::new(0.0, 0.0, 20.0, 20.0));
        assert_eq!(a.intersect(&b), Rect::new(5.0, 5.0, 10.0, 10.0));
        assert!(Rect::EMPTY.is_empty());
        assert_eq!(Rect::EMPTY.union(&a), a);
    }

    #[test]
    fn vec_ops() {
        let v = Vec2::new(3.0, 4.0);
        assert_eq!(v.length(), 5.0);
        assert_eq!(v.perp().dot(v), 0.0);
        assert_eq!(Vec2::ZERO.normalized(), Vec2::ZERO);
    }
}
