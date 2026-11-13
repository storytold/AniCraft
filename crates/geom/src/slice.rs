//! 9-slice scaling (ADR 0055): a box cut by four guides into corners, edges and a centre, scaled
//! so the corners keep their size, the edges stretch one way and the centre both ways.
//!
//! The map is separable: each axis is three linear pieces ([`Axis`]). Within one of the nine
//! cells it is affine, so a Bézier piece that stays in one cell maps exactly by mapping its
//! control points. [`slice_path`] cuts every segment where it crosses a guide (de Casteljau at the
//! exact crossing parameters) and maps each piece with its cell's affine; at a guide both cells
//! agree, so the result stays connected.

use crate::{Affine, Path, PathCmd, Vec2};

/// One axis of a 9-slice map: the source stops `[lo, a, b, hi]` (the box's edges and the two
/// guides) and where they land.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Axis {
    from: [f32; 4],
    to: [f32; 4],
}

impl Axis {
    /// The axis of a box `lo..hi` with guides at `a` and `b`, scaled by `k` (at least 0) about the
    /// origin: the box lands where plain scaling puts it (`k·lo..k·hi`), the parts outside the
    /// guides keep their size and the centre takes the rest. When the rest would be negative the
    /// two outer parts share what there is, in proportion, and the centre closes.
    ///
    /// Guides outside the box are clamped into it (and `b` to at least `a`). `None` for an empty
    /// box, a negative scale or anything not finite.
    pub fn new(lo: f32, a: f32, b: f32, hi: f32, k: f32) -> Option<Axis> {
        if ![lo, a, b, hi, k].iter().all(|v| v.is_finite()) || hi <= lo || k < 0.0 {
            return None;
        }
        let a = a.clamp(lo, hi);
        let b = b.clamp(a, hi);
        let (near, far) = (a - lo, hi - b);
        let (tlo, thi) = (k * lo, k * hi);
        let room = thi - tlo;
        let edges = near + far;
        let (near, far) = if edges > room && edges > 0.0 { (near * room / edges, far * room / edges) } else { (near, far) };
        let to = [tlo, tlo + near, thi - far, thi];
        to.iter().all(|v| v.is_finite()).then_some(Axis { from: [lo, a, b, hi], to })
    }

    /// The identity map on `lo..hi` (what an unscaled axis is).
    pub fn identity(lo: f32, hi: f32) -> Axis {
        Axis { from: [lo, lo, hi, hi], to: [lo, lo, hi, hi] }
    }

    /// The two guides, in source space.
    pub fn guides(&self) -> [f32; 2] {
        [self.from[1], self.from[2]]
    }

    /// Which piece `v` is in: 0 before the first guide, 1 between them, 2 after the second.
    pub fn piece(&self, v: f32) -> usize {
        if v < self.from[1] {
            0
        } else if v <= self.from[2] {
            1
        } else {
            2
        }
    }

    /// Piece `i` as `v ↦ scale · v + offset`. The outer pieces carry on past the box's edges
    /// with their own slope; a piece of no width has the slope of plain scaling (nothing lies in
    /// it but its edge, which maps right either way).
    pub fn linear(&self, i: usize) -> (f32, f32) {
        let i = i.min(2);
        let (f0, f1) = (self.from[i], self.from[i + 1]);
        let (t0, t1) = (self.to[i], self.to[i + 1]);
        let whole = (self.to[3] - self.to[0]) / (self.from[3] - self.from[0]).max(f32::MIN_POSITIVE);
        let scale = if f1 - f0 > 1e-6 { (t1 - t0) / (f1 - f0) } else { whole };
        // Anchored where the piece starts (the outer ones at their guide, so they meet the centre).
        let (anchor, at) = if i == 0 { (f1, t1) } else { (f0, t0) };
        (scale, at - scale * anchor)
    }

    /// Where `v` lands.
    pub fn map(&self, v: f32) -> f32 {
        let (s, o) = self.linear(self.piece(v));
        s * v + o
    }
}

/// A whole 9-slice map: the horizontal and the vertical axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slice9 {
    pub x: Axis,
    pub y: Axis,
}

impl Slice9 {
    pub fn map(&self, p: Vec2) -> Vec2 {
        Vec2::new(self.x.map(p.x), self.y.map(p.y))
    }

    /// The affine map of the cell `(ix, iy)`.
    pub fn cell(&self, ix: usize, iy: usize) -> Affine {
        let (sx, ox) = self.x.linear(ix);
        let (sy, oy) = self.y.linear(iy);
        Affine { m: [sx, 0.0, 0.0, sy, ox, oy] }
    }

    /// The cell a point is in.
    fn cell_of(&self, p: Vec2) -> Affine {
        self.cell(self.x.piece(p.x), self.y.piece(p.y))
    }
}

/// Largest number of commands [`slice_path`] writes (a path that would grow past it is mapped
/// point by point instead, which is exact for straight edges and close for curves).
pub const MAX_SLICED_CMDS: usize = 4_000_000;

/// `path` placed by `pre` (its own space to the box's) and then sliced by `map`: every segment cut
/// where it crosses a guide, each piece mapped by its cell's affine. Straight edges stay straight
/// within a cell, curves stay curves, fill areas stay closed (an implied closing line is drawn
/// out, since it may cross a guide).
pub fn slice_path(path: &Path, pre: &Affine, map: &Slice9) -> Path {
    if path.cmds.len().saturating_mul(4) > MAX_SLICED_CMDS {
        return path.transformed(pre).transformed_by(|p| map.map(p));
    }
    let gx = map.x.guides();
    let gy = map.y.guides();
    let mut out = Vec::with_capacity(path.cmds.len() + 8);
    let mut cur = Vec2::ZERO;
    let mut start = Vec2::ZERO;
    let mut ts: Vec<f32> = Vec::with_capacity(16);
    for c in &path.cmds {
        match *c {
            PathCmd::MoveTo(p) => {
                let p = pre.apply(p);
                out.push(PathCmd::MoveTo(map.map(p)));
                cur = p;
                start = p;
            }
            PathCmd::LineTo(p) => {
                let p = pre.apply(p);
                pieces(&[cur, p], &gx, &gy, map, &mut ts, &mut out);
                cur = p;
            }
            PathCmd::QuadTo(a, p) => {
                let (a, p) = (pre.apply(a), pre.apply(p));
                pieces(&[cur, a, p], &gx, &gy, map, &mut ts, &mut out);
                cur = p;
            }
            PathCmd::CubicTo(a, b, p) => {
                let (a, b, p) = (pre.apply(a), pre.apply(b), pre.apply(p));
                pieces(&[cur, a, b, p], &gx, &gy, map, &mut ts, &mut out);
                cur = p;
            }
            PathCmd::Close => {
                if cur != start {
                    pieces(&[cur, start], &gx, &gy, map, &mut ts, &mut out);
                }
                out.push(PathCmd::Close);
                cur = start;
            }
        }
    }
    Path { cmds: out }
}

/// Writes segment `ctrl` (2, 3 or 4 control points: a line, a quadratic or a cubic) as its pieces
/// between guide crossings, each mapped by its cell.
fn pieces(ctrl: &[Vec2], gx: &[f32; 2], gy: &[f32; 2], map: &Slice9, ts: &mut Vec<f32>, out: &mut Vec<PathCmd>) {
    ts.clear();
    let xs: Vec<f32> = ctrl.iter().map(|p| p.x).collect();
    let ys: Vec<f32> = ctrl.iter().map(|p| p.y).collect();
    for g in gx {
        crossings(&xs, *g, ts);
    }
    for g in gy {
        crossings(&ys, *g, ts);
    }
    ts.sort_by(f32::total_cmp);
    ts.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
    let mut t0 = 0.0;
    for &t1 in ts.iter().chain(std::iter::once(&1.0)) {
        if t1 - t0 < 1e-6 {
            continue;
        }
        let part = sub(ctrl, t0, t1);
        let mid = eval(ctrl, (t0 + t1) * 0.5);
        let m = map.cell_of(mid);
        push(&part, &m, out);
        t0 = t1;
    }
}

fn push(part: &[Vec2], m: &Affine, out: &mut Vec<PathCmd>) {
    match *part {
        [_, p] => out.push(PathCmd::LineTo(m.apply(p))),
        [_, a, p] => out.push(PathCmd::QuadTo(m.apply(a), m.apply(p))),
        [_, a, b, p] => out.push(PathCmd::CubicTo(m.apply(a), m.apply(b), m.apply(p))),
        _ => {}
    }
}

/// The point at `t` (de Casteljau).
fn eval(ctrl: &[Vec2], t: f32) -> Vec2 {
    let mut p = [Vec2::ZERO; 4];
    let n = ctrl.len().min(4);
    p[..n].copy_from_slice(&ctrl[..n]);
    for k in (1..n).rev() {
        for i in 0..k {
            p[i] = p[i].lerp(p[i + 1], t);
        }
    }
    p[0]
}

/// The piece of the curve between `t0` and `t1`, as control points of the same degree.
fn sub(ctrl: &[Vec2], t0: f32, t1: f32) -> Vec<Vec2> {
    // Split at t1, keep the first part; then split that at t0 / t1, keep the second.
    let first = split(ctrl, t1).0;
    if t0 <= 0.0 {
        return first;
    }
    let u = if t1 > 1e-12 { t0 / t1 } else { 0.0 };
    split(&first, u).1
}

/// De Casteljau split at `t`: the two halves.
fn split(ctrl: &[Vec2], t: f32) -> (Vec<Vec2>, Vec<Vec2>) {
    // Each level is one point shorter: the first and last points of every level are the halves'.
    let n = ctrl.len().min(4);
    let mut work = [Vec2::ZERO; 4];
    work[..n].copy_from_slice(&ctrl[..n]);
    let mut left = Vec::with_capacity(n);
    let mut right = Vec::with_capacity(n);
    for k in (0..n).rev() {
        left.push(work[0]);
        right.push(work[k]);
        for i in 0..k {
            work[i] = work[i].lerp(work[i + 1], t);
        }
    }
    right.reverse();
    (left, right)
}

/// Adds every `t` in (0, 1) where the 1-D Bézier `c` (2 to 4 coefficients) equals `v`: the curve
/// is cut where its derivative is zero into monotone runs, and each run that crosses `v` is
/// bisected. Exact to `f32`, at most three crossings per guide.
fn crossings(c: &[f32], v: f32, ts: &mut Vec<f32>) {
    let f = |t: f32| bez1(c, t) - v;
    let mut stops = [0.0f32; 4];
    let mut n = 0;
    stops[n] = 0.0;
    n += 1;
    if let [x0, x1, x2, x3] = *c {
        // Derivative (a quadratic) in power form: A t² + B t + C.
        let (q0, q1, q2) = (3.0 * (x1 - x0), 3.0 * (x2 - x1), 3.0 * (x3 - x2));
        let (a, b, cc) = (q0 - 2.0 * q1 + q2, 2.0 * (q1 - q0), q0);
        let mut roots = [f32::NAN; 2];
        if a.abs() > 1e-12 {
            let disc = b * b - 4.0 * a * cc;
            if disc >= 0.0 {
                let s = disc.sqrt();
                roots = [(-b - s) / (2.0 * a), (-b + s) / (2.0 * a)];
            }
        } else if b.abs() > 1e-12 {
            roots[0] = -cc / b;
        }
        roots.sort_by(f32::total_cmp);
        for r in roots {
            if r > 0.0 && r < 1.0 && n < 3 {
                stops[n] = r;
                n += 1;
            }
        }
    } else if let [x0, x1, x2] = *c {
        let den = x0 - 2.0 * x1 + x2;
        if den.abs() > 1e-12 {
            let r = (x0 - x1) / den;
            if r > 0.0 && r < 1.0 {
                stops[n] = r;
                n += 1;
            }
        }
    }
    stops[n] = 1.0;
    n += 1;
    for w in stops[..n].windows(2) {
        let (mut lo, mut hi) = (w[0], w[1]);
        let (flo, fhi) = (f(lo), f(hi));
        if !(flo.is_finite() && fhi.is_finite()) || flo == 0.0 || fhi == 0.0 || (flo > 0.0) == (fhi > 0.0) {
            continue;
        }
        let rising = fhi > flo;
        for _ in 0..40 {
            let mid = 0.5 * (lo + hi);
            if (f(mid) > 0.0) == rising {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        let t = 0.5 * (lo + hi);
        if t > 1e-6 && t < 1.0 - 1e-6 {
            ts.push(t);
        }
    }
}

fn bez1(c: &[f32], t: f32) -> f32 {
    let u = 1.0 - t;
    match *c {
        [a, b] => u * a + t * b,
        [a, b, d] => u * u * a + 2.0 * u * t * b + t * t * d,
        [a, b, d, e] => u * u * u * a + 3.0 * u * u * t * b + 3.0 * u * t * t * d + t * t * t * e,
        _ => 0.0,
    }
}

impl Path {
    /// Every point mapped by `f` (any map, not only an affine one).
    pub fn transformed_by(&self, f: impl Fn(Vec2) -> Vec2) -> Path {
        Path {
            cmds: self
                .cmds
                .iter()
                .map(|c| match *c {
                    PathCmd::MoveTo(p) => PathCmd::MoveTo(f(p)),
                    PathCmd::LineTo(p) => PathCmd::LineTo(f(p)),
                    PathCmd::QuadTo(a, p) => PathCmd::QuadTo(f(a), f(p)),
                    PathCmd::CubicTo(a, b, p) => PathCmd::CubicTo(f(a), f(b), f(p)),
                    PathCmd::Close => PathCmd::Close,
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn an_axis_keeps_its_ends_and_gives_the_centre_the_rest() {
        // A 100-wide box, 10 and 10 outside the guides, scaled 3x: 300 wide, 280 in the centre.
        let ax = Axis::new(0.0, 10.0, 90.0, 100.0, 3.0).unwrap();
        assert!(close(ax.map(0.0), 0.0));
        assert!(close(ax.map(10.0), 10.0));
        assert!(close(ax.map(90.0), 290.0));
        assert!(close(ax.map(100.0), 300.0));
        assert!(close(ax.map(50.0), 150.0), "the centre scales by 280 / 80");
        // About the origin, as plain scaling: a box from -50 lands on -150..150.
        let ax = Axis::new(-50.0, -40.0, 40.0, 50.0, 3.0).unwrap();
        assert!(close(ax.map(-50.0), -150.0) && close(ax.map(-40.0), -140.0) && close(ax.map(50.0), 150.0));
    }

    #[test]
    fn no_spare_space_shares_what_there_is_and_closes_the_centre() {
        // Corners 10 + 30 = 40 of a 100 box, scaled to 20: they shrink to 5 and 15.
        let ax = Axis::new(0.0, 10.0, 70.0, 100.0, 0.2).unwrap();
        assert!(close(ax.map(10.0), 5.0) && close(ax.map(70.0), 5.0) && close(ax.map(100.0), 20.0));
        // Exactly no spare space: the centre is a line.
        let ax = Axis::new(0.0, 10.0, 70.0, 100.0, 0.4).unwrap();
        assert!(close(ax.map(10.0), 10.0) && close(ax.map(70.0), 10.0) && close(ax.map(100.0), 40.0));
        // Zero scale: everything at the origin.
        let ax = Axis::new(0.0, 10.0, 70.0, 100.0, 0.0).unwrap();
        assert!(close(ax.map(0.0), 0.0) && close(ax.map(100.0), 0.0));
        // Guides outside or crossed are clamped; bad numbers give nothing.
        let ax = Axis::new(0.0, -20.0, 140.0, 100.0, 2.0).unwrap();
        assert!(close(ax.map(50.0), 100.0));
        assert!(Axis::new(0.0, 10.0, 90.0, 0.0, 2.0).is_none());
        assert!(Axis::new(0.0, 10.0, 90.0, 100.0, -1.0).is_none());
        assert!(Axis::new(0.0, f32::NAN, 90.0, 100.0, 1.0).is_none());
    }

    #[test]
    fn a_line_across_the_guides_is_cut_there() {
        let map = Slice9 { x: Axis::new(0.0, 10.0, 90.0, 100.0, 3.0).unwrap(), y: Axis::new(0.0, 10.0, 30.0, 40.0, 1.0).unwrap() };
        let p = slice_path(&Path::line(Vec2::new(0.0, 20.0), Vec2::new(100.0, 20.0)), &Affine::IDENTITY, &map);
        let pts: Vec<Vec2> = p
            .cmds
            .iter()
            .filter_map(|c| match *c {
                PathCmd::MoveTo(p) | PathCmd::LineTo(p) => Some(p),
                _ => None,
            })
            .collect();
        assert_eq!(pts.len(), 4);
        for (got, want) in pts.iter().zip([0.0, 10.0, 290.0, 300.0]) {
            assert!(close(got.x, want) && close(got.y, 20.0), "{got:?} vs {want}");
        }
    }

    #[test]
    fn a_closing_line_is_drawn_out_so_it_bends_at_the_guides() {
        // A triangle whose closing edge runs diagonally across the left guide.
        let p = Path { cmds: vec![PathCmd::MoveTo(Vec2::new(0.0, 0.0)), PathCmd::LineTo(Vec2::new(100.0, 0.0)), PathCmd::LineTo(Vec2::new(100.0, 40.0)), PathCmd::Close] };
        let map = Slice9 { x: Axis::new(0.0, 10.0, 90.0, 100.0, 3.0).unwrap(), y: Axis::identity(0.0, 40.0) };
        let s = slice_path(&p, &Affine::IDENTITY, &map);
        assert_eq!(s.cmds.last(), Some(&PathCmd::Close));
        // Every vertex of the result is the map of a point on the source outline.
        assert!(s.cmds.len() > p.cmds.len() + 2);
    }

    #[test]
    fn curves_are_cut_at_exact_crossings_and_stay_on_the_mapped_curve() {
        let c = [Vec2::new(0.0, 0.0), Vec2::new(30.0, 60.0), Vec2::new(70.0, -20.0), Vec2::new(100.0, 40.0)];
        let p = Path { cmds: vec![PathCmd::MoveTo(c[0]), PathCmd::CubicTo(c[1], c[2], c[3])] };
        let map = Slice9 { x: Axis::new(0.0, 25.0, 75.0, 100.0, 2.5).unwrap(), y: Axis::new(-20.0, 0.0, 20.0, 60.0, 1.5).unwrap() };
        let s = slice_path(&p, &Affine::IDENTITY, &map);
        // Sample the source curve; each sample's image must lie on the sliced path (within
        // flattening tolerance): the piecewise-affine image of a Bézier is the pieces' Béziers.
        let flat: Vec<Vec2> = s.flatten(&Affine::IDENTITY, 0.01).into_iter().flat_map(|l| l.points).collect();
        for i in 0..=50 {
            let q = map.map(eval(&c, i as f32 / 50.0));
            let d = flat.windows(2).map(|w| crate::segment_distance(q, w[0], w[1])).fold(f32::INFINITY, f32::min);
            assert!(d < 0.1, "sample {i}: {d}");
        }
    }

    #[test]
    fn the_pre_transform_places_the_path_first() {
        let map = Slice9 { x: Axis::new(0.0, 10.0, 90.0, 100.0, 2.0).unwrap(), y: Axis::identity(0.0, 10.0) };
        let s = slice_path(&Path::line(Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0)), &Affine::translate(90.0, 0.0), &map);
        assert_eq!(s.cmds.first(), Some(&PathCmd::MoveTo(Vec2::new(190.0, 0.0))));
        assert!(matches!(s.cmds.last(), Some(PathCmd::LineTo(p)) if close(p.x, 200.0)));
    }
}
