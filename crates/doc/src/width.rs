//! Variable-width strokes (ADR 0018): a stroke's width profile.
//!
//! A profile is a list of width points along each subpath of the stroke: a position `t`
//! (0 at the subpath's start, 1 at its end, by arc length in the shape's own space) and the
//! half-width on each side as a factor of the stroke's nominal half-width (1 = the width set
//! in Properties, so changing the stroke width scales the profile). "Left" is on the left
//! of the direction of travel as seen on the (y-down) stage, in the shape's local space: the
//! side [`left_normal`] points to.
//!
//! Between width points the widths follow a monotone cubic (Fritsch–Carlson), so they are
//! smooth, pass through every point and never overshoot them: a taper never bulges past its
//! ends and never dips below zero. Before the first and after the last point the width holds.

use anicraft_geom::walk::PathWalk;
use anicraft_geom::{Path, Vec2};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Most width points one profile keeps.
pub const MAX_WIDTH_POINTS: usize = 64;
/// Widest a width point may make the stroke, as a factor of its nominal width.
pub const MAX_WIDTH_FACTOR: f32 = 16.0;
/// Width points closer than this (in `t`) are one point.
const MERGE: f32 = 1e-3;

/// A point on a stroke where its width is set.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WidthPoint {
    /// Position along the subpath, `0..=1` by arc length.
    pub t: f32,
    /// Half-width on the left side, as a factor of the nominal half-width.
    pub left: f32,
    /// Half-width on the right side, as a factor of the nominal half-width.
    pub right: f32,
}

impl WidthPoint {
    pub const fn new(t: f32, left: f32, right: f32) -> WidthPoint {
        WidthPoint { t, left, right }
    }
    fn sanitized(self) -> Option<WidthPoint> {
        let f = |v: f32| if v.is_finite() { v.clamp(0.0, MAX_WIDTH_FACTOR) } else { 1.0 };
        self.t.is_finite().then(|| WidthPoint { t: self.t.clamp(0.0, 1.0), left: f(self.left), right: f(self.right) })
    }
}

/// A stroke's width points: sorted by position, at least one, at most [`MAX_WIDTH_POINTS`],
/// factors in `0..=MAX_WIDTH_FACTOR`, never all 1 (a uniform stroke has no profile).
/// `Arc`-shared, so copying a stroke never copies its points.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(from = "Vec<WidthPoint>", into = "Vec<WidthPoint>")]
pub struct WidthProfile {
    points: Arc<[WidthPoint]>,
}

impl From<Vec<WidthPoint>> for WidthProfile {
    /// Sanitizes untrusted points (a file, a command). May be uniform: [`Stroke`] drops those.
    fn from(v: Vec<WidthPoint>) -> WidthProfile {
        let mut pts: Vec<WidthPoint> = v.into_iter().take(MAX_WIDTH_POINTS * 4).filter_map(WidthPoint::sanitized).collect();
        pts.sort_by(|a, b| a.t.total_cmp(&b.t));
        // Points at (nearly) the same place: the last one given wins.
        let mut out: Vec<WidthPoint> = Vec::with_capacity(pts.len());
        for p in pts {
            match out.last_mut() {
                Some(last) if p.t - last.t < MERGE => *last = WidthPoint { t: last.t, ..p },
                _ => out.push(p),
            }
        }
        out.truncate(MAX_WIDTH_POINTS);
        WidthProfile { points: out.into() }
    }
}

impl From<WidthProfile> for Vec<WidthPoint> {
    fn from(p: WidthProfile) -> Vec<WidthPoint> {
        p.points.to_vec()
    }
}

impl WidthProfile {
    /// A profile from width points (sanitized); `None` when they make a uniform stroke.
    pub fn new(points: Vec<WidthPoint>) -> Option<WidthProfile> {
        let p = WidthProfile::from(points);
        (!p.is_uniform()).then_some(p)
    }

    pub fn points(&self) -> &[WidthPoint] {
        &self.points
    }

    /// No points, or every side of every point at the nominal width.
    pub fn is_uniform(&self) -> bool {
        self.points.iter().all(|p| (p.left - 1.0).abs() < 1e-4 && (p.right - 1.0).abs() < 1e-4)
    }

    /// The widest either side gets, as a factor of the nominal half-width (the monotone
    /// interpolation never exceeds its points).
    pub fn max_factor(&self) -> f32 {
        self.points.iter().fold(0.0f32, |m, p| m.max(p.left).max(p.right))
    }

    /// Half-width factors `(left, right)` at `t` along a subpath.
    pub fn at(&self, t: f32) -> (f32, f32) {
        let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
        (monotone(&self.points, t, |p| p.left), monotone(&self.points, t, |p| p.right))
    }

    /// The point index at `t` (within the merge distance), if any.
    pub fn index_at(&self, t: f32) -> Option<usize> {
        self.points.iter().position(|p| (p.t - t).abs() < MERGE)
    }

    /// With a width point at `t` (replacing one already there); `None` when uniform. Its
    /// index in the result is [`WidthProfile::index_at`]`(t)`.
    pub fn with_point(&self, p: WidthPoint) -> Option<WidthProfile> {
        let mut v: Vec<WidthPoint> = self.points.iter().copied().filter(|q| (q.t - p.t).abs() >= MERGE).collect();
        if v.len() >= MAX_WIDTH_POINTS {
            return Some(self.clone());
        }
        v.push(p);
        WidthProfile::new(v)
    }

    /// With point `index` replaced by `p`, kept between its neighbours so the order (and so the
    /// indices) stay the same; `None` when uniform, unchanged when there is no such point.
    pub fn with_replaced(&self, index: usize, p: WidthPoint) -> Option<WidthProfile> {
        let mut v = self.points.to_vec();
        let lo = index.checked_sub(1).and_then(|i| v.get(i)).map_or(0.0, |q| q.t + MERGE * 1.5);
        let hi = v.get(index + 1).map_or(1.0, |q| q.t - MERGE * 1.5);
        let Some(slot) = v.get_mut(index) else { return Some(self.clone()) };
        let t = if lo <= hi { p.t.clamp(lo, hi) } else { slot.t };
        *slot = WidthPoint { t, ..p };
        WidthProfile::new(v)
    }

    /// Without point `index`; `None` when what is left is uniform.
    pub fn without(&self, index: usize) -> Option<WidthProfile> {
        let v: Vec<WidthPoint> = self.points.iter().enumerate().filter(|(i, _)| *i != index).map(|(_, p)| *p).collect();
        WidthProfile::new(v)
    }

    /// The profile `t` of the way from `a` to `b` (`None` is uniform). Profiles with the same
    /// number of points blend point by point; others blend their widths at every point of
    /// either, so any two profiles tween.
    pub fn lerp(a: Option<&WidthProfile>, b: Option<&WidthProfile>, t: f32) -> Option<WidthProfile> {
        let l = anicraft_geom::lerp;
        match (a, b) {
            (None, None) => None,
            (Some(x), Some(y)) if x.points.len() == y.points.len() => {
                WidthProfile::new(x.points.iter().zip(y.points.iter()).map(|(p, q)| WidthPoint::new(l(p.t, q.t, t), l(p.left, q.left, t), l(p.right, q.right, t))).collect())
            }
            _ => {
                let at = |p: Option<&WidthProfile>, s: f32| p.map_or((1.0, 1.0), |p| p.at(s));
                let mut ts: Vec<f32> = a.into_iter().chain(b).flat_map(|p| p.points.iter().map(|q| q.t)).collect();
                ts.sort_by(f32::total_cmp);
                ts.dedup_by(|x, y| (*x - *y).abs() < MERGE);
                WidthProfile::new(
                    ts.into_iter()
                        .map(|s| {
                            let ((al, ar), (bl, br)) = (at(a, s), at(b, s));
                            WidthPoint::new(s, l(al, bl, t), l(ar, br, t))
                        })
                        .collect(),
                )
            }
        }
    }

    /// The built-in profile this one is, if any.
    pub fn preset(&self) -> Option<ProfilePreset> {
        ProfilePreset::ALL.into_iter().find(|p| p.profile().as_ref() == Some(self))
    }
}

/// Fritsch–Carlson monotone cubic through `pts` (sorted by `t`) at `t`, channel `v`.
fn monotone(pts: &[WidthPoint], t: f32, v: impl Fn(&WidthPoint) -> f32) -> f32 {
    let (Some(first), Some(last)) = (pts.first(), pts.last()) else { return 1.0 };
    if t <= first.t {
        return v(first);
    }
    if t >= last.t {
        return v(last);
    }
    let k = pts.partition_point(|p| p.t <= t).saturating_sub(1);
    let (Some(p0), Some(p1)) = (pts.get(k), pts.get(k + 1)) else { return v(last) };
    let h = p1.t - p0.t;
    if h <= 0.0 {
        return v(p1);
    }
    let slope = |i: usize| -> Option<f32> {
        let (a, b) = (pts.get(i)?, pts.get(i + 1)?);
        let d = b.t - a.t;
        (d > 0.0).then(|| (v(b) - v(a)) / d)
    };
    // Tangent at point i: zero at a local extremum or where a side is flat, else the harmonic
    // mean of the neighbouring slopes (which keeps the curve monotone between points).
    let tangent = |i: usize| -> f32 {
        let before = i.checked_sub(1).and_then(slope);
        let after = slope(i);
        match (before, after) {
            (Some(a), Some(b)) if a * b > 0.0 => 2.0 * a * b / (a + b),
            (Some(_), Some(_)) => 0.0,
            (Some(a), None) | (None, Some(a)) => a,
            (None, None) => 0.0,
        }
    };
    let (m0, m1) = (tangent(k), tangent(k + 1));
    let s = (t - p0.t) / h;
    let (s2, s3) = (s * s, s * s * s);
    let out = (2.0 * s3 - 3.0 * s2 + 1.0) * v(p0) + (s3 - 2.0 * s2 + s) * h * m0 + (-2.0 * s3 + 3.0 * s2) * v(p1) + (s3 - s2) * h * m1;
    // Rounding only: the monotone curve stays between its two points.
    out.clamp(v(p0).min(v(p1)), v(p0).max(v(p1)))
}

/// The built-in width profiles offered in Properties and the drawing tools' options.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ProfilePreset {
    #[default]
    Uniform,
    TaperBoth,
    TaperStart,
    TaperEnd,
    Bulge,
    Pinch,
}

impl ProfilePreset {
    pub const ALL: [ProfilePreset; 6] = [ProfilePreset::Uniform, ProfilePreset::TaperBoth, ProfilePreset::TaperStart, ProfilePreset::TaperEnd, ProfilePreset::Bulge, ProfilePreset::Pinch];

    /// The name used in commands (`taper_both`…).
    pub fn key(self) -> &'static str {
        match self {
            ProfilePreset::Uniform => "uniform",
            ProfilePreset::TaperBoth => "taper_both",
            ProfilePreset::TaperStart => "taper_start",
            ProfilePreset::TaperEnd => "taper_end",
            ProfilePreset::Bulge => "bulge",
            ProfilePreset::Pinch => "pinch",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ProfilePreset::Uniform => "Uniform",
            ProfilePreset::TaperBoth => "Taper both ends",
            ProfilePreset::TaperStart => "Taper start",
            ProfilePreset::TaperEnd => "Taper end",
            ProfilePreset::Bulge => "Bulge",
            ProfilePreset::Pinch => "Pinch",
        }
    }

    pub fn from_key(s: &str) -> Option<ProfilePreset> {
        let s = s.trim().to_ascii_lowercase().replace([' ', '-'], "_");
        ProfilePreset::ALL.into_iter().find(|p| p.key() == s)
    }

    /// The profile (`None` for Uniform).
    pub fn profile(self) -> Option<WidthProfile> {
        let p = |t, w| WidthPoint::new(t, w, w);
        WidthProfile::new(match self {
            ProfilePreset::Uniform => return None,
            ProfilePreset::TaperBoth => vec![p(0.0, 0.0), p(0.5, 1.0), p(1.0, 0.0)],
            ProfilePreset::TaperStart => vec![p(0.0, 0.0), p(0.4, 1.0), p(1.0, 1.0)],
            ProfilePreset::TaperEnd => vec![p(0.0, 1.0), p(0.6, 1.0), p(1.0, 0.0)],
            ProfilePreset::Bulge => vec![p(0.0, 0.4), p(0.5, 1.6), p(1.0, 0.4)],
            ProfilePreset::Pinch => vec![p(0.0, 1.0), p(0.5, 0.25), p(1.0, 1.0)],
        })
    }
}

/// The unit normal on the left of direction `d` on a y-down screen (`-d.perp()`).
pub fn left_normal(d: Vec2) -> Vec2 {
    Vec2::new(d.y, -d.x)
}

/// A shape's subpaths as walks in its local space (closed ones end where they start), for
/// placing and finding width points. `tolerance` is in local units.
pub fn stroke_walks(path: &Path, tolerance: f32) -> Vec<PathWalk> {
    path.flatten(&anicraft_geom::Affine::IDENTITY, tolerance)
        .into_iter()
        .take(4096)
        .filter_map(|line| {
            let mut pts = line.points;
            if line.closed
                && let Some(&first) = pts.first()
            {
                pts.push(first);
            }
            PathWalk::new(&pts)
        })
        .collect()
}

/// Where a stroke passes nearest to `p` (local space): `(subpath, t, distance, side)`, `side`
/// being `1.0` on the left of the direction of travel and `-1.0` on the right.
pub fn nearest_on_stroke(walks: &[PathWalk], p: Vec2) -> Option<(usize, f32, f32, f32)> {
    let mut best: Option<(usize, f32, f32, f32)> = None;
    for (i, w) in walks.iter().enumerate() {
        let s = w.project(p);
        let q = w.at(s);
        let d = q.distance(p);
        if best.is_none_or(|b| d < b.2) {
            let side = if (p - q).dot(left_normal(w.tangent(s))) >= 0.0 { 1.0 } else { -1.0 };
            let len = w.length();
            best = Some((i, if len > 0.0 { s / len } else { 0.0 }, d, side));
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_are_sanitized_and_uniform_ones_dropped() {
        let p = WidthProfile::from(vec![WidthPoint::new(0.9, 2.0, 2.0), WidthPoint::new(f32::NAN, 0.0, 0.0), WidthPoint::new(-3.0, -1.0, 99.0), WidthPoint::new(0.9002, 3.0, 3.0)]);
        assert_eq!(p.points(), &[WidthPoint::new(0.0, 0.0, MAX_WIDTH_FACTOR), WidthPoint::new(0.9, 3.0, 3.0)]);
        assert!(WidthProfile::new(vec![WidthPoint::new(0.3, 1.0, 1.0)]).is_none(), "all nominal: uniform");
        assert!(WidthProfile::new(vec![]).is_none());
        let many: Vec<WidthPoint> = (0..1000).map(|i| WidthPoint::new(i as f32 / 1000.0, 2.0, 2.0)).collect();
        assert_eq!(WidthProfile::from(many).points().len(), MAX_WIDTH_POINTS);
    }

    #[test]
    fn widths_pass_through_points_smoothly_without_overshoot() {
        let p = ProfilePreset::TaperBoth.profile().unwrap();
        assert_eq!(p.at(0.0), (0.0, 0.0));
        assert_eq!(p.at(0.5), (1.0, 1.0));
        assert_eq!(p.at(1.0), (0.0, 0.0));
        // Symmetric, rising then falling, never above the peak.
        let (a, _) = p.at(0.25);
        let (b, _) = p.at(0.75);
        assert!((a - b).abs() < 1e-5 && a > 0.3 && a < 1.0, "{a} {b}");
        let mut last = -1.0;
        for i in 0..=50 {
            let (w, _) = p.at(i as f32 / 100.0);
            assert!(w >= last - 1e-6 && w <= 1.0);
            last = w;
        }
        // Flat at the peak: the tangent there is zero.
        assert!(p.at(0.49).0 > 0.999);
        // Holds outside the points; garbage positions are safe.
        let q = WidthProfile::new(vec![WidthPoint::new(0.25, 2.0, 0.5), WidthPoint::new(0.75, 0.5, 2.0)]).unwrap();
        assert_eq!(q.at(0.0), (2.0, 0.5));
        assert_eq!(q.at(1.0), (0.5, 2.0));
        assert_eq!(q.at(f32::NAN), (2.0, 0.5));
        assert_eq!(q.max_factor(), 2.0);
    }

    #[test]
    fn editing_points_keeps_order_and_falls_back_to_uniform() {
        let p = ProfilePreset::TaperBoth.profile().unwrap();
        let q = p.with_point(WidthPoint::new(0.25, 3.0, 3.0)).unwrap();
        assert_eq!(q.index_at(0.25), Some(1));
        assert_eq!(q.points().len(), 4);
        // Moving a point stays between its neighbours.
        let r = q.with_replaced(1, WidthPoint::new(0.9, 3.0, 1.0)).unwrap();
        assert!(r.points()[1].t < 0.5 && r.points()[1].t > 0.25);
        assert_eq!(q.with_replaced(9, WidthPoint::new(0.5, 1.0, 1.0)), Some(q.clone()), "no such point: unchanged");
        let one = WidthProfile::new(vec![WidthPoint::new(0.5, 2.0, 2.0)]).unwrap();
        assert!(one.without(0).is_none(), "nothing left: uniform");
        assert_eq!(p.preset(), Some(ProfilePreset::TaperBoth));
        assert_eq!(q.preset(), None);
        assert_eq!(ProfilePreset::from_key("Taper Both"), Some(ProfilePreset::TaperBoth));
        assert!(ProfilePreset::Uniform.profile().is_none());
    }

    #[test]
    fn profiles_tween_pointwise_or_by_resampling() {
        let a = ProfilePreset::TaperBoth.profile().unwrap();
        let b = ProfilePreset::Bulge.profile().unwrap();
        let mid = WidthProfile::lerp(Some(&a), Some(&b), 0.5).unwrap();
        assert_eq!(mid.points().len(), 3);
        assert!((mid.at(0.0).0 - 0.2).abs() < 1e-5 && (mid.at(0.5).0 - 1.3).abs() < 1e-5);
        // Different point counts, and to or from uniform: widths blend everywhere.
        let c = WidthProfile::new(vec![WidthPoint::new(0.2, 2.0, 2.0), WidthPoint::new(0.8, 0.5, 0.5)]).unwrap();
        let m2 = WidthProfile::lerp(Some(&a), Some(&c), 0.5).unwrap();
        assert_eq!(m2.points().len(), 5);
        for s in [0.0, 0.2, 0.5, 0.8, 1.0] {
            let want = (a.at(s).0 + c.at(s).0) * 0.5;
            assert!((m2.at(s).0 - want).abs() < 1e-4, "{s}");
        }
        let half = WidthProfile::lerp(Some(&a), None, 0.5).unwrap();
        assert!((half.at(0.0).0 - 0.5).abs() < 1e-5);
        assert!(WidthProfile::lerp(Some(&a), None, 1.0).is_none(), "all the way to uniform");
        assert!(WidthProfile::lerp(None, None, 0.3).is_none());
    }

    #[test]
    fn finds_the_nearest_place_on_a_stroke() {
        let mut path = Path::new();
        path.move_to(Vec2::new(0.0, 0.0)).line_to(Vec2::new(100.0, 0.0));
        let walks = stroke_walks(&path, 0.25);
        let (sub, t, d, side) = nearest_on_stroke(&walks, Vec2::new(25.0, -4.0)).unwrap();
        assert_eq!(sub, 0);
        assert!((t - 0.25).abs() < 1e-5 && (d - 4.0).abs() < 1e-5);
        assert_eq!(side, 1.0, "heading right on a y-down screen, up (-y) is on the left");
        assert_eq!(nearest_on_stroke(&walks, Vec2::new(25.0, 4.0)).unwrap().3, -1.0);
        // A closed square walks back to its start.
        let sq = stroke_walks(&Path::rect(0.0, 0.0, 10.0, 10.0), 0.25);
        assert_eq!(sq[0].length(), 40.0);
        assert!(nearest_on_stroke(&[], Vec2::ZERO).is_none());
    }
}
