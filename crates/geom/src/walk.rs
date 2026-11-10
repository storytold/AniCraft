//! Walking along a path by distance: what a motion guide needs. A flattened polyline with its
//! cumulative arc lengths answers "the nearest point to p" and "the point s along the path".
//! [`CurveWalk`] does the same on the Bézier curve itself, with exact points and tangents, for
//! motion paths (ADR 0030): an object oriented to the path turns smoothly instead of in steps.

use crate::{Path, PathCmd, Vec2};

/// Most segments a [`CurveWalk`] measures (the rest of a longer path is ignored).
pub const MAX_WALK_SEGMENTS: usize = 10_000;

/// A cubic segment's point at `t`.
pub fn cubic_point(c: &[Vec2; 4], t: f32) -> Vec2 {
    let u = 1.0 - t;
    c[0] * (u * u * u) + c[1] * (3.0 * u * u * t) + c[2] * (3.0 * u * t * t) + c[3] * (t * t * t)
}

/// A cubic segment's derivative at `t`.
fn cubic_deriv(c: &[Vec2; 4], t: f32) -> Vec2 {
    let u = 1.0 - t;
    (c[1] - c[0]) * (3.0 * u * u) + (c[2] - c[1]) * (6.0 * u * t) + (c[3] - c[2]) * (3.0 * t * t)
}

/// One measured point of a [`CurveWalk`].
#[derive(Clone, Copy, Debug, PartialEq)]
struct Sample {
    /// Distance from the start.
    s: f32,
    seg: u32,
    t: f32,
    p: Vec2,
}

/// The first subpath of a Bézier path measured by arc length. Every segment is held as a
/// cubic (lines and quadratics are raised exactly), sampled densely; between samples the curve
/// parameter is interpolated, so points and tangents come from the curve itself.
#[derive(Clone, Debug, PartialEq)]
pub struct CurveWalk {
    /// Each segment's control points, with the path command it came from.
    segs: Vec<(usize, [Vec2; 4])>,
    samples: Vec<Sample>,
}

impl CurveWalk {
    /// `None` for a path without a segment of some length, or with non-finite points. A `Close`
    /// that draws an edge back to the start walks that edge too.
    pub fn new(path: &Path) -> Option<CurveWalk> {
        let mut segs: Vec<(usize, [Vec2; 4])> = Vec::new();
        let (mut cur, mut start, mut begun) = (Vec2::ZERO, Vec2::ZERO, false);
        for (i, c) in path.cmds.iter().enumerate() {
            if segs.len() >= MAX_WALK_SEGMENTS {
                break;
            }
            let seg = match *c {
                PathCmd::MoveTo(p) if !begun || segs.is_empty() => {
                    (cur, start, begun) = (p, p, true);
                    continue;
                }
                PathCmd::MoveTo(_) => break,
                PathCmd::LineTo(p) => [cur, cur.lerp(p, 1.0 / 3.0), cur.lerp(p, 2.0 / 3.0), p],
                PathCmd::QuadTo(q, p) => [cur, cur + (q - cur) * (2.0 / 3.0), p + (q - p) * (2.0 / 3.0), p],
                PathCmd::CubicTo(c1, c2, p) => [cur, c1, c2, p],
                PathCmd::Close => {
                    if cur.distance(start) > 1e-6 {
                        segs.push((i, [cur, cur.lerp(start, 1.0 / 3.0), cur.lerp(start, 2.0 / 3.0), start]));
                    }
                    break;
                }
            };
            if !seg.iter().all(|p| p.is_finite()) {
                return None;
            }
            cur = seg[3];
            segs.push((i, seg));
        }
        let first = segs.first()?.1[0];
        let mut samples = vec![Sample { s: 0.0, seg: 0, t: 0.0, p: first }];
        let mut d = 0.0f32;
        for (k, (_, c)) in segs.iter().enumerate() {
            // About one sample per two units of the control polygon: dense enough that the
            // chord and the arc agree to well under a thousandth.
            let poly = c[0].distance(c[1]) + c[1].distance(c[2]) + c[2].distance(c[3]);
            let n = (poly / 2.0).ceil().clamp(8.0, 256.0) as u32;
            let mut prev = c[0];
            for j in 1..=n {
                let t = j as f32 / n as f32;
                let p = cubic_point(c, t);
                d += prev.distance(p);
                prev = p;
                samples.push(Sample { s: d, seg: k as u32, t, p });
            }
        }
        (d.is_finite() && d > 1e-3).then_some(CurveWalk { segs, samples })
    }

    pub fn length(&self) -> f32 {
        self.samples.last().map_or(0.0, |s| s.s)
    }

    /// The segment (index into the walk's segments) and curve parameter at distance `s`
    /// (clamped to the path).
    fn find(&self, s: f32) -> (usize, f32) {
        let s = if s.is_finite() { s.clamp(0.0, self.length()) } else { 0.0 };
        let j = self.samples.partition_point(|x| x.s < s).clamp(1, self.samples.len().saturating_sub(1).max(1));
        let (Some(a), Some(b)) = (self.samples.get(j - 1), self.samples.get(j)) else { return (0, 0.0) };
        // A sample ending one segment starts the next at t = 0.
        let ta = if a.seg == b.seg { a.t } else { 0.0 };
        let f = if b.s > a.s { (s - a.s) / (b.s - a.s) } else { 1.0 };
        (b.seg as usize, ta + (b.t - ta) * f)
    }

    /// The path command and curve parameter at distance `s`: where to split or bend the path.
    pub fn locate(&self, s: f32) -> (usize, f32) {
        let (k, t) = self.find(s);
        (self.segs.get(k).map_or(0, |x| x.0), t)
    }

    /// The point `s` along the path.
    pub fn at(&self, s: f32) -> Vec2 {
        let (k, t) = self.find(s);
        self.segs.get(k).map_or(Vec2::ZERO, |(_, c)| cubic_point(c, t))
    }

    /// The direction of travel at `s` (unit length). Where a handle sits on its anchor the
    /// derivative vanishes; the direction is then taken from the curve on either side.
    pub fn tangent(&self, s: f32) -> Vec2 {
        let (k, t) = self.find(s);
        let Some((_, c)) = self.segs.get(k) else { return Vec2::new(1.0, 0.0) };
        let d = cubic_deriv(c, t);
        if d.length() > 1e-4 {
            return d.normalized();
        }
        let d = cubic_point(c, (t + 1e-3).min(1.0)) - cubic_point(c, (t - 1e-3).max(0.0));
        if d.length() > 1e-6 {
            return d.normalized();
        }
        let d = c[3] - c[0];
        if d.length() > 1e-6 { d.normalized() } else { Vec2::new(1.0, 0.0) }
    }

    /// The distance along the path of the point nearest to `p`.
    pub fn project(&self, p: Vec2) -> f32 {
        let mut best = (f32::INFINITY, 0.0f32);
        for w in self.samples.windows(2) {
            let [a, b] = w else { continue };
            let ab = b.p - a.p;
            let len2 = ab.dot(ab);
            let t = if len2 > 0.0 { ((p - a.p).dot(ab) / len2).clamp(0.0, 1.0) } else { 0.0 };
            let d = (a.p + ab * t).distance(p);
            if d < best.0 {
                best = (d, a.s + (b.s - a.s) * t);
            }
        }
        best.1
    }

    /// The path from distance `s` to the end, as a move and cubics (`None` when less than a
    /// thousandth of a unit is left).
    pub fn tail(&self, s: f32) -> Option<Path> {
        if self.length() - s.max(0.0) <= 1e-3 {
            return None;
        }
        let (k, t) = self.find(s);
        let (_, c) = self.segs.get(k)?;
        // de Casteljau: the second half of segment k from t.
        let (a, b, c3) = (c[0].lerp(c[1], t), c[1].lerp(c[2], t), c[2].lerp(c[3], t));
        let (d, e) = (a.lerp(b, t), b.lerp(c3, t));
        let m = d.lerp(e, t);
        let mut cmds = vec![PathCmd::MoveTo(m), PathCmd::CubicTo(e, c3, c[3])];
        cmds.extend(self.segs.iter().skip(k + 1).map(|(_, c)| PathCmd::CubicTo(c[1], c[2], c[3])));
        Some(Path { cmds })
    }
}

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

    /// The usual cubic approximation of a quarter circle of radius `r` about `(0, r)`, from
    /// `(0, 0)` heading +x round to `(r, r)` heading +y.
    fn quarter(r: f32) -> Path {
        let k = 0.552_284_8 * r;
        let mut p = Path::new();
        p.move_to(Vec2::new(0.0, 0.0)).cubic_to(Vec2::new(k, 0.0), Vec2::new(r, r - k), Vec2::new(r, r));
        p
    }

    #[test]
    fn a_straight_cubic_walks_evenly() {
        let mut p = Path::new();
        p.move_to(Vec2::new(10.0, 20.0)).line_to(Vec2::new(110.0, 20.0)).line_to(Vec2::new(110.0, 70.0));
        let w = CurveWalk::new(&p).unwrap();
        assert!((w.length() - 150.0).abs() < 1e-3);
        for s in [0.0f32, 12.5, 50.0, 99.0] {
            assert!(w.at(s).distance(Vec2::new(10.0 + s, 20.0)) < 1e-3, "{s}: {:?}", w.at(s));
            assert!(w.tangent(s).distance(Vec2::new(1.0, 0.0)) < 1e-5);
        }
        assert!(w.at(125.0).distance(Vec2::new(110.0, 45.0)) < 1e-3);
        assert!(w.tangent(125.0).distance(Vec2::new(0.0, 1.0)) < 1e-5);
        assert_eq!(w.locate(125.0).0, 2, "the second line is command 2");
        assert!((w.project(Vec2::new(130.0, 60.0)) - 140.0).abs() < 1e-2);
        let tail = w.tail(50.0).unwrap();
        let tw = CurveWalk::new(&tail).unwrap();
        assert!((tw.length() - 100.0).abs() < 1e-2 && tw.at(0.0).distance(Vec2::new(60.0, 20.0)) < 1e-3);
        assert!(w.tail(150.0).is_none(), "nothing left at the end");
    }

    #[test]
    fn a_quarter_circle_keeps_to_the_arc_and_turns_with_it() {
        let r = 100.0;
        let w = CurveWalk::new(&quarter(r)).unwrap();
        let arc = std::f32::consts::FRAC_PI_2 * r;
        assert!((w.length() - arc).abs() / arc < 1e-3, "{} vs {arc}", w.length());
        for i in 0..=20 {
            let s = w.length() * i as f32 / 20.0;
            let p = w.at(s);
            // On the circle (the cubic strays by 0.03% of r at most) …
            assert!((p.distance(Vec2::new(0.0, r)) - r).abs() < 0.03, "{i}: {p:?}");
            // … and heading along it: the tangent is perpendicular to the radius.
            let radial = (p - Vec2::new(0.0, r)).normalized();
            assert!(w.tangent(s).dot(radial).abs() < 2e-3, "{i}");
        }
        assert!(w.tangent(0.0).distance(Vec2::new(1.0, 0.0)) < 1e-5, "{:?}", w.tangent(0.0));
        assert!(w.tangent(arc).distance(Vec2::new(0.0, 1.0)) < 1e-3, "{:?}", w.tangent(arc));
    }

    #[test]
    fn degenerate_paths_have_no_walk() {
        let mut p = Path::new();
        p.move_to(Vec2::new(1.0, 1.0));
        assert!(CurveWalk::new(&p).is_none());
        p.line_to(Vec2::new(1.0, 1.0));
        assert!(CurveWalk::new(&p).is_none());
        let mut q = Path::new();
        q.move_to(Vec2::new(0.0, 0.0)).line_to(Vec2::new(f32::NAN, 1.0));
        assert!(CurveWalk::new(&q).is_none());
        // Only the first subpath is walked; a closing edge is walked too.
        let mut c = Path::new();
        c.move_to(Vec2::new(0.0, 0.0)).line_to(Vec2::new(10.0, 0.0)).line_to(Vec2::new(10.0, 10.0)).close().move_to(Vec2::new(50.0, 50.0)).line_to(Vec2::new(60.0, 50.0));
        let w = CurveWalk::new(&c).unwrap();
        assert!((w.length() - (20.0 + 200f32.sqrt())).abs() < 1e-2);
        assert!(w.at(f32::NAN).distance(Vec2::ZERO) < 1e-6);
    }
}
