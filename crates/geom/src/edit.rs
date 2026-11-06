//! Point editing on Bézier paths: addressing anchors and handles, splitting and removing
//! segments, smooth/corner conversion and nearest-segment search.
//!
//! Addressing: command `i` of a path owns its *end point* (the anchor, slot 0) and its control
//! points: slot 1 is the first control point (`QuadTo`'s or `CubicTo`'s `c1`, the out-handle of
//! the previous anchor) and slot 2 is `CubicTo`'s `c2` (the in-handle of anchor `i`).

use crate::{Affine, Path, PathCmd, Vec2};

/// Slot numbers.
pub const ANCHOR: u8 = 0;
pub const C1: u8 = 1;
pub const C2: u8 = 2;

/// Points closer than this (stage units) are the same anchor.
const LINK_EPS: f32 = 1e-3;

impl Path {
    /// The point at `(cmd, slot)`.
    pub fn point(&self, cmd: usize, slot: u8) -> Option<Vec2> {
        match (self.cmds.get(cmd)?, slot) {
            (PathCmd::MoveTo(p) | PathCmd::LineTo(p) | PathCmd::QuadTo(_, p) | PathCmd::CubicTo(_, _, p), ANCHOR) => Some(*p),
            (PathCmd::QuadTo(c, _) | PathCmd::CubicTo(c, _, _), C1) => Some(*c),
            (PathCmd::CubicTo(_, c, _), C2) => Some(*c),
            _ => None,
        }
    }

    /// Sets the point at `(cmd, slot)`; false when it does not exist or `p` is not finite.
    pub fn set_point(&mut self, cmd: usize, slot: u8, p: Vec2) -> bool {
        if !p.is_finite() {
            return false;
        }
        let Some(c) = self.cmds.get_mut(cmd) else { return false };
        let target = match (c, slot) {
            (PathCmd::MoveTo(a) | PathCmd::LineTo(a) | PathCmd::QuadTo(_, a) | PathCmd::CubicTo(_, _, a), ANCHOR) => a,
            (PathCmd::QuadTo(c1, _) | PathCmd::CubicTo(c1, _, _), C1) => c1,
            (PathCmd::CubicTo(_, c2, _), C2) => c2,
            _ => return false,
        };
        *target = p;
        true
    }

    /// Commands that own an anchor, with the anchor position.
    pub fn anchors(&self) -> Vec<(usize, Vec2)> {
        (0..self.cmds.len()).filter_map(|i| self.point(i, ANCHOR).map(|p| (i, p))).collect()
    }

    /// The point a segment command starts from (previous anchor, or the subpath start after a
    /// `Close`). `None` for `MoveTo`/`Close` or out of range.
    pub fn segment_start(&self, cmd: usize) -> Option<Vec2> {
        if !matches!(self.cmds.get(cmd)?, PathCmd::LineTo(_) | PathCmd::QuadTo(..) | PathCmd::CubicTo(..)) {
            return None;
        }
        let (mut cur, mut start) = (Vec2::ZERO, Vec2::ZERO);
        for c in self.cmds.get(..cmd)? {
            match *c {
                PathCmd::MoveTo(p) => {
                    cur = p;
                    start = p;
                }
                PathCmd::LineTo(p) | PathCmd::QuadTo(_, p) | PathCmd::CubicTo(_, _, p) => cur = p,
                PathCmd::Close => cur = start,
            }
        }
        Some(cur)
    }

    /// Point on segment `cmd` at parameter `t ∈ [0, 1]`.
    pub fn segment_point(&self, cmd: usize, t: f32) -> Option<Vec2> {
        let s = self.segment_start(cmd)?;
        let mt = 1.0 - t;
        Some(match *self.cmds.get(cmd)? {
            PathCmd::LineTo(p) => s.lerp(p, t),
            PathCmd::QuadTo(c, p) => s * (mt * mt) + c * (2.0 * mt * t) + p * (t * t),
            PathCmd::CubicTo(c1, c2, p) => s * (mt * mt * mt) + c1 * (3.0 * mt * mt * t) + c2 * (3.0 * mt * t * t) + p * (t * t * t),
            _ => return None,
        })
    }

    /// Splits segment `cmd` at `t` (de Casteljau), adding an anchor without changing the shape.
    pub fn split_segment(&mut self, cmd: usize, t: f32) -> bool {
        let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { return false };
        let Some(s) = self.segment_start(cmd) else { return false };
        let Some(&c) = self.cmds.get(cmd) else { return false };
        let (first, second) = match c {
            PathCmd::LineTo(p) => (PathCmd::LineTo(s.lerp(p, t)), PathCmd::LineTo(p)),
            PathCmd::QuadTo(c1, p) => {
                let (a, b) = (s.lerp(c1, t), c1.lerp(p, t));
                let m = a.lerp(b, t);
                (PathCmd::QuadTo(a, m), PathCmd::QuadTo(b, p))
            }
            PathCmd::CubicTo(c1, c2, p) => {
                let (a, b, c3) = (s.lerp(c1, t), c1.lerp(c2, t), c2.lerp(p, t));
                let (d, e) = (a.lerp(b, t), b.lerp(c3, t));
                let m = d.lerp(e, t);
                (PathCmd::CubicTo(a, d, m), PathCmd::CubicTo(e, c3, p))
            }
            _ => return false,
        };
        if let Some(slot) = self.cmds.get_mut(cmd) {
            *slot = first;
        }
        self.cmds.insert(cmd + 1, second);
        true
    }

    fn is_segment(&self, cmd: usize) -> bool {
        matches!(self.cmds.get(cmd), Some(PathCmd::LineTo(_) | PathCmd::QuadTo(..) | PathCmd::CubicTo(..)))
    }

    /// The `MoveTo` that starts the subpath containing `cmd`.
    fn subpath_start(&self, cmd: usize) -> Option<usize> {
        self.cmds.get(..=cmd)?.iter().rposition(|x| matches!(x, PathCmd::MoveTo(_)))
    }

    /// Writes every `Close` that would draw an implicit edge as an explicit line back to the
    /// subpath start, so each edge of the outline is a command that can be split and curved.
    /// The shape does not change.
    pub fn close_explicitly(&mut self) {
        let mut out = Vec::with_capacity(self.cmds.len() + 4);
        let (mut cur, mut start) = (Vec2::ZERO, Vec2::ZERO);
        for &c in &self.cmds {
            match c {
                PathCmd::MoveTo(p) => {
                    cur = p;
                    start = p;
                }
                PathCmd::LineTo(p) | PathCmd::QuadTo(_, p) | PathCmd::CubicTo(_, _, p) => cur = p,
                PathCmd::Close => {
                    if cur.distance(start) > LINK_EPS {
                        out.push(PathCmd::LineTo(start));
                    }
                    cur = start;
                }
            }
            out.push(c);
        }
        self.cmds = out;
    }

    /// In a closed subpath whose last segment returns to the start, the start `MoveTo` and that
    /// segment's end are one anchor stored twice. Returns the other copy.
    pub fn linked_anchor(&self, cmd: usize) -> Option<usize> {
        match self.cmds.get(cmd)? {
            PathCmd::MoveTo(p) => {
                let end = (cmd + 1..self.cmds.len()).find(|&j| !self.is_segment(j))?;
                let last = end.checked_sub(1).filter(|&l| l > cmd)?;
                (matches!(self.cmds.get(end), Some(PathCmd::Close)) && self.point(last, ANCHOR)?.distance(*p) <= LINK_EPS).then_some(last)
            }
            _ if self.is_segment(cmd) && matches!(self.cmds.get(cmd + 1), Some(PathCmd::Close)) => {
                let m = self.subpath_start(cmd)?;
                (self.linked_anchor(m) == Some(cmd)).then_some(m)
            }
            _ => None,
        }
    }

    /// The command that names an anchor: the `MoveTo` for a closed subpath's start, else `cmd`.
    /// Overlays list only canonical anchors, so the start is not drawn twice.
    pub fn canonical_anchor(&self, cmd: usize) -> usize {
        match self.cmds.get(cmd) {
            Some(PathCmd::MoveTo(_)) | None => cmd,
            _ => self.linked_anchor(cmd).unwrap_or(cmd),
        }
    }

    /// Anchors as the user sees them: one per position, start of a closed subpath once.
    pub fn editable_anchors(&self) -> Vec<(usize, Vec2)> {
        self.anchors().into_iter().filter(|(i, _)| self.canonical_anchor(*i) == *i).collect()
    }

    /// The segments that arrive at and leave anchor `cmd`: `(in, out)`. The in-segment's `C2`
    /// and the out-segment's `C1` are the anchor's handles.
    pub fn anchor_segments(&self, cmd: usize) -> (Option<usize>, Option<usize>) {
        let cmd = self.canonical_anchor(cmd);
        let out = |i: usize| (self.is_segment(i + 1)).then_some(i + 1);
        match self.cmds.get(cmd) {
            Some(PathCmd::MoveTo(_)) => (self.linked_anchor(cmd), out(cmd)),
            Some(_) if self.is_segment(cmd) => (Some(cmd), out(cmd)),
            _ => (None, None),
        }
    }

    /// Every stored point that moves when anchor `cmd` moves: both copies of the anchor and its
    /// two handles.
    pub fn anchor_group(&self, cmd: usize) -> Vec<(usize, u8)> {
        let cmd = self.canonical_anchor(cmd);
        let mut v = vec![(cmd, ANCHOR)];
        if let Some(k) = self.linked_anchor(cmd) {
            v.push((k, ANCHOR));
        }
        let (i, o) = self.anchor_segments(cmd);
        if let Some(i) = i.filter(|&i| matches!(self.cmds.get(i), Some(PathCmd::CubicTo(..)))) {
            v.push((i, C2));
        }
        if let Some(o) = o.filter(|&o| matches!(self.cmds.get(o), Some(PathCmd::CubicTo(..)))) {
            v.push((o, C1));
        }
        v
    }

    /// Removes anchor `cmd`. A segment anchor is dropped (the next segment then starts from the
    /// previous anchor); a subpath start passes to the next anchor. Refuses when the subpath
    /// would be left with fewer than two anchors.
    pub fn remove_anchor(&mut self, cmd: usize) -> bool {
        let cmd = self.canonical_anchor(cmd);
        let Some(&c) = self.cmds.get(cmd) else { return false };
        let Some(m) = self.subpath_start(cmd) else { return false };
        let segs = (m + 1..self.cmds.len()).take_while(|&j| self.is_segment(j)).count();
        let linked = self.linked_anchor(m);
        let distinct = 1 + segs - usize::from(linked.is_some());
        if distinct <= 2 {
            return false;
        }
        match c {
            PathCmd::MoveTo(_) => {
                let Some(next_end) = self.point(cmd + 1, ANCHOR) else { return false };
                if let Some(k) = linked {
                    self.set_point(k, ANCHOR, next_end);
                }
                if let Some(slot) = self.cmds.get_mut(cmd) {
                    *slot = PathCmd::MoveTo(next_end);
                }
                self.cmds.remove(cmd + 1);
                true
            }
            PathCmd::LineTo(_) | PathCmd::QuadTo(..) | PathCmd::CubicTo(..) => {
                self.cmds.remove(cmd);
                true
            }
            PathCmd::Close => false,
        }
    }

    /// Makes anchor `cmd` a corner: its handles collapse onto it (segments stay curves).
    pub fn make_corner(&mut self, cmd: usize) -> bool {
        let Some(a) = self.point(cmd, ANCHOR) else { return false };
        let (i, o) = self.anchor_segments(cmd);
        let mut changed = false;
        if let Some(i) = i.filter(|&i| matches!(self.cmds.get(i), Some(PathCmd::CubicTo(..)))) {
            changed |= self.set_point(i, C2, a);
        }
        if let Some(o) = o.filter(|&o| matches!(self.cmds.get(o), Some(PathCmd::CubicTo(..)))) {
            changed |= self.set_point(o, C1, a);
        }
        changed
    }

    /// Makes anchor `cmd` smooth: neighbouring segments become cubics whose handles at this
    /// anchor are collinear, along the direction from the previous to the next anchor.
    pub fn make_smooth(&mut self, cmd: usize) -> bool {
        let Some(a) = self.point(cmd, ANCHOR) else { return false };
        let (i, o) = self.anchor_segments(cmd);
        let prev = i.and_then(|i| self.segment_start(i));
        let next = o.and_then(|o| self.point(o, ANCHOR));
        let dir = match (prev, next) {
            (Some(p), Some(n)) => (n - p).normalized(),
            (Some(p), None) => (a - p).normalized(),
            (None, Some(n)) => (n - a).normalized(),
            (None, None) => return false,
        };
        if dir == Vec2::ZERO {
            return false;
        }
        if let (Some(i), Some(p)) = (i, prev) {
            self.to_cubic(i);
            self.set_point(i, C2, a - dir * (a.distance(p) / 3.0));
        }
        if let (Some(o), Some(n)) = (o, next) {
            self.to_cubic(o);
            self.set_point(o, C1, a + dir * (a.distance(n) / 3.0));
        }
        true
    }

    /// Converts segment `cmd` to an equivalent cubic (no-op for cubics and non-segments).
    pub fn to_cubic(&mut self, cmd: usize) {
        let Some(s) = self.segment_start(cmd) else { return };
        let Some(c) = self.cmds.get_mut(cmd) else { return };
        *c = match *c {
            PathCmd::LineTo(p) => PathCmd::CubicTo(s.lerp(p, 1.0 / 3.0), s.lerp(p, 2.0 / 3.0), p),
            PathCmd::QuadTo(q, p) => PathCmd::CubicTo(s + (q - s) * (2.0 / 3.0), p + (q - p) * (2.0 / 3.0), p),
            other => other,
        };
    }

    /// The segment nearest to `p` (in the space `m` maps to): `(cmd, t, distance)`.
    pub fn nearest_segment(&self, p: Vec2, m: &Affine) -> Option<(usize, f32, f32)> {
        let mut best: Option<(usize, f32, f32)> = None;
        for cmd in 0..self.cmds.len().min(100_000) {
            if self.segment_start(cmd).is_none() {
                continue;
            }
            let steps = 32;
            for i in 0..=steps {
                let t = i as f32 / steps as f32;
                let Some(q) = self.segment_point(cmd, t) else { continue };
                let d = m.apply(q).distance(p);
                if best.is_none_or(|b| d < b.2) {
                    best = Some((cmd, t, d));
                }
            }
        }
        // refine t around the best sample
        let (cmd, mut t, mut d) = best?;
        let mut step = 1.0 / 64.0;
        for _ in 0..12 {
            for cand in [t - step, t + step] {
                let c = cand.clamp(0.0, 1.0);
                if let Some(q) = self.segment_point(cmd, c) {
                    let dc = m.apply(q).distance(p);
                    if dc < d {
                        d = dc;
                        t = c;
                    }
                }
            }
            step *= 0.5;
        }
        Some((cmd, t, d))
    }
}

/// Ramer–Douglas–Peucker simplification (iterative; bounded memory).
pub fn simplify(points: &[Vec2], tolerance: f32) -> Vec<Vec2> {
    let n = points.len();
    if n < 3 {
        return points.to_vec();
    }
    let tol = if tolerance.is_finite() && tolerance > 0.0 { tolerance } else { 0.5 };
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[n - 1] = true;
    let mut stack = vec![(0usize, n - 1)];
    while let Some((a, b)) = stack.pop() {
        if b <= a + 1 {
            continue;
        }
        let (pa, pb) = (points[a], points[b]);
        let mut far = (a, 0.0f32);
        for (i, p) in points.iter().enumerate().take(b).skip(a + 1) {
            let d = crate::path::segment_distance(*p, pa, pb);
            if d > far.1 {
                far = (i, d);
            }
        }
        if far.1 > tol {
            keep[far.0] = true;
            stack.push((a, far.0));
            stack.push((far.0, b));
        }
    }
    points.iter().zip(keep).filter_map(|(p, k)| k.then_some(*p)).collect()
}

/// A straight-segment path through `points`.
pub fn polyline_path(points: &[Vec2], closed: bool) -> Path {
    let mut path = Path::new();
    for (i, &p) in points.iter().enumerate() {
        if i == 0 {
            path.move_to(p);
        } else {
            path.line_to(p);
        }
    }
    if closed && points.len() > 2 {
        path.close();
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cubic_path() -> Path {
        let mut p = Path::new();
        p.move_to(Vec2::new(0.0, 0.0)).cubic_to(Vec2::new(10.0, 20.0), Vec2::new(30.0, 20.0), Vec2::new(40.0, 0.0)).line_to(Vec2::new(40.0, -20.0));
        p
    }

    #[test]
    fn addressing() {
        let p = cubic_path();
        assert_eq!(p.anchors().len(), 3);
        assert_eq!(p.point(1, C2), Some(Vec2::new(30.0, 20.0)));
        assert_eq!(p.point(2, C1), None);
        assert_eq!(p.segment_start(2), Some(Vec2::new(40.0, 0.0)));
        let mut q = p.clone();
        assert!(q.set_point(1, C1, Vec2::new(5.0, 5.0)));
        assert!(!q.set_point(0, C1, Vec2::new(5.0, 5.0)));
        assert!(!q.set_point(1, ANCHOR, Vec2::new(f32::NAN, 0.0)));
    }

    #[test]
    fn split_keeps_the_curve() {
        let p = cubic_path();
        let mut q = p.clone();
        assert!(q.split_segment(1, 0.3));
        assert_eq!(q.anchors().len(), 4);
        // the new anchor lies on the original curve
        let on = p.segment_point(1, 0.3).unwrap();
        assert!(q.point(1, ANCHOR).unwrap().distance(on) < 1e-4);
        // and the flattened shapes agree
        let a = p.flatten(&Affine::IDENTITY, 0.01)[0].points.clone();
        let b = q.flatten(&Affine::IDENTITY, 0.01)[0].points.clone();
        let near = |pt: &Vec2, set: &[Vec2]| set.windows(2).map(|w| crate::path::segment_distance(*pt, w[0], w[1])).fold(f32::INFINITY, f32::min);
        assert!(b.iter().all(|pt| near(pt, &a) < 0.2));
        assert!(!q.split_segment(0, 0.5), "MoveTo is not a segment");
    }

    #[test]
    fn remove_keeps_at_least_two_anchors() {
        let mut p = cubic_path();
        assert!(p.remove_anchor(1));
        assert_eq!(p.anchors().len(), 2);
        assert!(!p.remove_anchor(1), "a subpath needs two anchors");
        let mut r = Path::rect(0.0, 0.0, 10.0, 10.0);
        assert!(r.remove_anchor(0), "MoveTo takes over the next anchor");
        assert_eq!(r.point(0, ANCHOR), Some(Vec2::new(10.0, 0.0)));
    }

    #[test]
    fn closed_subpaths_have_one_start_anchor() {
        let mut r = Path::rect(0.0, 0.0, 10.0, 10.0);
        let before = r.flatten(&Affine::IDENTITY, 0.1);
        r.close_explicitly();
        assert_eq!(r.flatten(&Affine::IDENTITY, 0.1)[0].points.len(), before[0].points.len() + 1, "only a duplicate end point added");
        assert_eq!(r.linked_anchor(0), Some(4));
        assert_eq!(r.linked_anchor(4), Some(0));
        assert_eq!(r.canonical_anchor(4), 0);
        assert_eq!(r.editable_anchors().len(), 4);
        assert_eq!(r.anchor_segments(0), (Some(4), Some(1)));
        r.close_explicitly();
        assert_eq!(r.cmds.len(), 6, "idempotent");
        // The start anchor is smooth across the seam and removable.
        assert!(r.make_smooth(0));
        assert!(matches!(r.cmds[4], PathCmd::CubicTo(..)) && matches!(r.cmds[1], PathCmd::CubicTo(..)));
        let mut q = r.clone();
        assert!(q.remove_anchor(4), "the duplicate names the start");
        assert_eq!(q.editable_anchors().len(), 3);
        assert_eq!(q.point(0, ANCHOR), Some(Vec2::new(10.0, 0.0)));
        assert_eq!(q.linked_anchor(0), Some(3), "still closed on the new start");
        assert!(q.remove_anchor(0));
        assert!(!q.remove_anchor(0), "two anchors stay");
        // Moving the start moves both copies and both handles.
        let g = r.anchor_group(0);
        assert!(g.contains(&(0, ANCHOR)) && g.contains(&(4, ANCHOR)) && g.contains(&(4, C2)) && g.contains(&(1, C1)));
    }

    #[test]
    fn smooth_and_corner() {
        let mut p = Path::new();
        p.move_to(Vec2::new(0.0, 0.0)).line_to(Vec2::new(10.0, 10.0)).line_to(Vec2::new(20.0, 0.0));
        assert!(p.make_smooth(1));
        let (a, i, o) = (p.point(1, ANCHOR).unwrap(), p.point(1, C2).unwrap(), p.point(2, C1).unwrap());
        assert!((i - a).cross(o - a).abs() < 1e-3, "handles collinear");
        assert!((i - a).dot(o - a) < 0.0, "on opposite sides");
        assert!(p.make_corner(1));
        assert_eq!(p.point(1, C2), Some(a));
    }

    #[test]
    fn nearest_segment_finds_the_curve() {
        let p = cubic_path();
        let target = p.segment_point(1, 0.62).unwrap();
        let (cmd, t, d) = p.nearest_segment(target + Vec2::new(0.0, 0.5), &Affine::IDENTITY).unwrap();
        assert_eq!(cmd, 1);
        assert!((t - 0.62).abs() < 0.05, "{t}");
        assert!(d < 1.0);
    }

    #[test]
    fn simplify_and_fit() {
        let pts: Vec<Vec2> = (0..=100).map(|i| Vec2::new(i as f32, (i as f32 * 0.1).sin() * 0.05)).collect();
        assert_eq!(simplify(&pts, 0.5).len(), 2, "a nearly straight stroke collapses to its ends");
        let wave: Vec<Vec2> = (0..=200).map(|i| Vec2::new(i as f32, (i as f32 * 0.05).sin() * 30.0)).collect();
        let s = simplify(&wave, 1.0);
        assert!(s.len() > 4 && s.len() < 60, "{}", s.len());
        let path = polyline_path(&s, false);
        assert_eq!(path.anchors().len(), s.len());
        assert!(path.anchors().iter().zip(&s).all(|((_, a), b)| a.distance(*b) < 1e-4), "passes through the points");
        assert!(simplify(&[], 1.0).is_empty());
    }
}
