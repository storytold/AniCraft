//! Walking along a path by distance: what a motion guide needs. A flattened polyline with its
//! cumulative arc lengths answers "the nearest point to p" and "the point s along the path".

use crate::Vec2;

/// A polyline measured by arc length.
#[derive(Clone, Debug, PartialEq)]
pub struct PathWalk {
    pts: Vec<Vec2>,
    /// `cum[i]` is the distance from the start to `pts[i]`.
    cum: Vec<f32>,
}

impl PathWalk {
    /// `None` for fewer than two distinct points or non-finite input.
    pub fn new(points: &[Vec2]) -> Option<PathWalk> {
        let mut pts: Vec<Vec2> = Vec::with_capacity(points.len());
        for &p in points {
            if !p.is_finite() {
                return None;
            }
            if pts.last().is_none_or(|q| q.distance(p) > 1e-6) {
                pts.push(p);
            }
        }
        if pts.len() < 2 {
            return None;
        }
        let mut cum = Vec::with_capacity(pts.len());
        let mut d = 0.0f32;
        cum.push(0.0);
        for w in pts.windows(2) {
            d += w[0].distance(w[1]);
            cum.push(d);
        }
        d.is_finite().then_some(PathWalk { pts, cum })
    }

    pub fn length(&self) -> f32 {
        self.cum.last().copied().unwrap_or(0.0)
    }

    /// The distance along the path of the point nearest to `p`.
    pub fn project(&self, p: Vec2) -> f32 {
        let mut best = (f32::INFINITY, 0.0f32);
        for (i, w) in self.pts.windows(2).enumerate() {
            let (a, b) = (w[0], w[1]);
            let ab = b - a;
            let len2 = ab.dot(ab);
            let t = if len2 > 0.0 { ((p - a).dot(ab) / len2).clamp(0.0, 1.0) } else { 0.0 };
            let q = a + ab * t;
            let d = q.distance(p);
            if d < best.0 {
                let start = self.cum.get(i).copied().unwrap_or(0.0);
                best = (d, start + len2.sqrt() * t);
            }
        }
        best.1
    }

    /// The segment containing distance `s` (clamped to the path) and how far into it.
    fn locate(&self, s: f32) -> (usize, f32) {
        let s = if s.is_finite() { s.clamp(0.0, self.length()) } else { 0.0 };
        let i = self.cum.partition_point(|&c| c <= s).saturating_sub(1).min(self.pts.len() - 2);
        let (c0, c1) = (self.cum.get(i).copied().unwrap_or(0.0), self.cum.get(i + 1).copied().unwrap_or(0.0));
        let t = if c1 > c0 { (s - c0) / (c1 - c0) } else { 0.0 };
        (i, t)
    }

    /// The point `s` along the path.
    pub fn at(&self, s: f32) -> Vec2 {
        let (i, t) = self.locate(s);
        match (self.pts.get(i), self.pts.get(i + 1)) {
            (Some(a), Some(b)) => a.lerp(*b, t),
            _ => self.pts.first().copied().unwrap_or(Vec2::new(0.0, 0.0)),
        }
    }

    /// The direction of travel at `s` (unit length).
    pub fn tangent(&self, s: f32) -> Vec2 {
        let (i, _) = self.locate(s);
        match (self.pts.get(i), self.pts.get(i + 1)) {
            (Some(a), Some(b)) => (*b - *a).normalized(),
            _ => Vec2::new(1.0, 0.0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measures_projects_and_walks() {
        // An L: right 10, then down 10.
        let w = PathWalk::new(&[Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0), Vec2::new(10.0, 10.0)]).unwrap();
        assert_eq!(w.length(), 20.0);
        assert_eq!(w.at(5.0), Vec2::new(5.0, 0.0));
        assert_eq!(w.at(15.0), Vec2::new(10.0, 5.0));
        assert_eq!(w.at(-3.0), Vec2::new(0.0, 0.0), "clamped to the ends");
        assert_eq!(w.at(99.0), Vec2::new(10.0, 10.0));
        assert!((w.project(Vec2::new(4.0, -3.0)) - 4.0).abs() < 1e-5);
        assert!((w.project(Vec2::new(14.0, 7.0)) - 17.0).abs() < 1e-5);
        assert_eq!(w.tangent(2.0), Vec2::new(1.0, 0.0));
        assert_eq!(w.tangent(12.0), Vec2::new(0.0, 1.0));
        assert!(PathWalk::new(&[Vec2::new(1.0, 1.0), Vec2::new(1.0, 1.0)]).is_none());
        assert!(PathWalk::new(&[Vec2::new(f32::NAN, 0.0), Vec2::new(1.0, 1.0)]).is_none());
        assert_eq!(w.at(f32::NAN), Vec2::new(0.0, 0.0));
    }
}
