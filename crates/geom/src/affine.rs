//! 2D affine transforms and their decomposition for tweening.

use crate::Vec2;
use serde::{Deserialize, Serialize};

/// Affine transform `[a b c d e f]` mapping `(x, y)` to `(a x + c y + e, b x + d y + f)`
/// (the column convention used by SVG, PDF and most animation formats).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Affine {
    pub m: [f32; 6],
}

impl Default for Affine {
    fn default() -> Self {
        Affine::IDENTITY
    }
}

impl Affine {
    pub const IDENTITY: Affine = Affine { m: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0] };

    pub fn translate(tx: f32, ty: f32) -> Affine {
        Affine { m: [1.0, 0.0, 0.0, 1.0, tx, ty] }
    }
    pub fn scale(sx: f32, sy: f32) -> Affine {
        Affine { m: [sx, 0.0, 0.0, sy, 0.0, 0.0] }
    }
    /// Rotation by `radians` (clockwise on a y-down screen).
    pub fn rotate(radians: f32) -> Affine {
        let (s, c) = radians.sin_cos();
        Affine { m: [c, s, -s, c, 0.0, 0.0] }
    }

    /// `self ∘ other`: apply `other` first, then `self`.
    pub fn then_apply_to(&self, other: &Affine) -> Affine {
        let [a, b, c, d, e, f] = self.m;
        let [a2, b2, c2, d2, e2, f2] = other.m;
        Affine {
            m: [
                a * a2 + c * b2,
                b * a2 + d * b2,
                a * c2 + c * d2,
                b * c2 + d * d2,
                a * e2 + c * f2 + e,
                b * e2 + d * f2 + f,
            ],
        }
    }

    /// Composition in reading order: `a.mul(b)` applies `b` first, then `a`.
    pub fn mul(&self, other: &Affine) -> Affine {
        self.then_apply_to(other)
    }

    pub fn apply(&self, p: Vec2) -> Vec2 {
        let [a, b, c, d, e, f] = self.m;
        Vec2::new(a * p.x + c * p.y + e, b * p.x + d * p.y + f)
    }

    /// Applies only the linear part (for vectors).
    pub fn apply_vector(&self, v: Vec2) -> Vec2 {
        let [a, b, c, d, _, _] = self.m;
        Vec2::new(a * v.x + c * v.y, b * v.x + d * v.y)
    }

    pub fn determinant(&self) -> f32 {
        self.m[0] * self.m[3] - self.m[1] * self.m[2]
    }

    /// Inverse, or `None` for a singular (or non-finite) matrix.
    pub fn inverse(&self) -> Option<Affine> {
        let det = self.determinant();
        if !det.is_finite() || det.abs() < 1e-12 {
            return None;
        }
        let [a, b, c, d, e, f] = self.m;
        let inv = 1.0 / det;
        let ia = d * inv;
        let ib = -b * inv;
        let ic = -c * inv;
        let id = a * inv;
        Some(Affine { m: [ia, ib, ic, id, -(ia * e + ic * f), -(ib * e + id * f)] })
    }

    pub fn translation(&self) -> Vec2 {
        Vec2::new(self.m[4], self.m[5])
    }

    pub fn is_finite(&self) -> bool {
        self.m.iter().all(|v| v.is_finite())
    }

    /// Approximate uniform scale factor (sqrt of |det|), used for stroke widths and tolerances.
    pub fn mean_scale(&self) -> f32 {
        self.determinant().abs().sqrt()
    }

    /// Decompose into translation, rotation, scale and skew (QR-style).
    pub fn decompose(&self) -> Decomposed {
        let [a, b, c, d, e, f] = self.m;
        let sx = (a * a + b * b).sqrt();
        let det = a * d - b * c;
        let (rotation, sy, skew) = if sx > 1e-12 {
            let sy = det / sx;
            // col1 · col2 = sx · sy · skew
            let skew = if det.abs() > 1e-12 { (a * c + b * d) / det } else { 0.0 };
            (b.atan2(a), sy, skew)
        } else {
            // Degenerate first column: take the rotation from the second column, which
            // `compose` maps to (-sin r, cos r) · sy.
            ((-c).atan2(d), (c * c + d * d).sqrt(), 0.0)
        };
        Decomposed { tx: e, ty: f, sx, sy, rotation, skew }
    }
}

/// Transform components that interpolate well over time.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Decomposed {
    pub tx: f32,
    pub ty: f32,
    pub sx: f32,
    pub sy: f32,
    /// Radians.
    pub rotation: f32,
    /// Shear factor (x += skew * y before scaling).
    pub skew: f32,
}

impl Default for Decomposed {
    fn default() -> Self {
        Decomposed { tx: 0.0, ty: 0.0, sx: 1.0, sy: 1.0, rotation: 0.0, skew: 0.0 }
    }
}

impl Decomposed {
    /// `T · R · Shear · S` — the inverse of [`Affine::decompose`].
    pub fn compose(&self) -> Affine {
        let t = Affine::translate(self.tx, self.ty);
        let r = Affine::rotate(self.rotation);
        let k = Affine { m: [1.0, 0.0, self.skew, 1.0, 0.0, 0.0] };
        let s = Affine::scale(self.sx, self.sy);
        t.mul(&r).mul(&k).mul(&s)
    }

    /// Component-wise interpolation; rotation takes the shortest arc.
    pub fn lerp(&self, o: &Decomposed, t: f32) -> Decomposed {
        let mut dr = o.rotation - self.rotation;
        let tau = std::f32::consts::TAU;
        while dr > std::f32::consts::PI {
            dr -= tau;
        }
        while dr < -std::f32::consts::PI {
            dr += tau;
        }
        Decomposed {
            tx: crate::lerp(self.tx, o.tx, t),
            ty: crate::lerp(self.ty, o.ty, t),
            sx: crate::lerp(self.sx, o.sx, t),
            sy: crate::lerp(self.sy, o.sy, t),
            rotation: self.rotation + dr * t,
            skew: crate::lerp(self.skew, o.skew, t),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: &Affine, b: &Affine) -> bool {
        a.m.iter().zip(b.m.iter()).all(|(x, y)| (x - y).abs() < 1e-4)
    }

    #[test]
    fn decompose_roundtrip_zero_sx() {
        for m in [[0.0, 0.0, -1.0, 0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0, 3.0, 4.0], [0.0, 0.0, 2.0, -3.0, 1.0, 1.0]] {
            let m = Affine { m };
            assert!(close(&m.decompose().compose(), &m), "{:?}", m.decompose().compose());
        }
    }

    #[test]
    fn inverse_roundtrip() {
        let m = Affine::translate(10.0, -4.0).mul(&Affine::rotate(0.7)).mul(&Affine::scale(2.0, 0.5));
        let inv = m.inverse().unwrap();
        assert!(close(&m.mul(&inv), &Affine::IDENTITY));
        assert!(Affine::scale(0.0, 1.0).inverse().is_none());
    }

    #[test]
    fn decompose_compose_identity() {
        for (rot, sx, sy, skew) in [(0.3, 2.0, 1.5, 0.0), (-2.5, 0.5, 3.0, 0.2), (1.0, 1.0, -1.0, 0.0)] {
            let d = Decomposed { tx: 5.0, ty: 7.0, sx, sy, rotation: rot, skew };
            let m = d.compose();
            let back = m.decompose().compose();
            assert!(close(&m, &back), "{m:?} vs {back:?}");
        }
    }

    #[test]
    fn rotation_lerp_takes_short_arc() {
        let a = Decomposed { rotation: 3.0, ..Default::default() };
        let b = Decomposed { rotation: -3.0, ..Default::default() };
        let mid = a.lerp(&b, 0.5);
        assert!((mid.rotation.abs() - std::f32::consts::PI).abs() < 0.01);
    }

    #[test]
    fn apply_order() {
        let m = Affine::translate(10.0, 0.0).mul(&Affine::scale(2.0, 2.0));
        assert_eq!(m.apply(Vec2::new(1.0, 1.0)), Vec2::new(12.0, 2.0));
    }
}
