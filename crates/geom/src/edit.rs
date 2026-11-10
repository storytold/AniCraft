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

    /// Removes the anchor between segments `cmd` and `cmd + 1`, replacing both with the one
    /// cubic that comes closest to them: it keeps the outer anchors and the directions their
    /// handles leave in, and takes the handle lengths that fit the old curve best (least
    /// squares over samples of both segments, with Newton steps refining where each sample
    /// falls on the new curve; Schneider, "An Algorithm for Automatically Fitting Digitized
    /// Curves", Graphics Gems, 1990). Deleting a point that splitting added gives the old curve
    /// back. False (nothing changed) unless both commands are segments.
    pub fn merge_at(&mut self, cmd: usize) -> bool {
        if !(self.is_segment(cmd) && self.is_segment(cmd + 1)) {
            return false;
        }
        self.to_cubic(cmd);
        self.to_cubic(cmd + 1);
        let (Some(p0), Some(&PathCmd::CubicTo(a1, a2, m)), Some(&PathCmd::CubicTo(b1, b2, p3))) = (self.segment_start(cmd), self.cmds.get(cmd), self.cmds.get(cmd + 1)) else { return false };
        let Some(c) = merge_cubics([p0, a1, a2, m], [m, b1, b2, p3]) else { return false };
        if let Some(slot) = self.cmds.get_mut(cmd + 1) {
            *slot = PathCmd::CubicTo(c[1], c[2], c[3]);
        }
        self.cmds.remove(cmd);
        true
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

    /// Moves anchor `cmd` by `d` with everything attached to it (its other copy on a closed
    /// subpath and both handles), so the curve keeps its shape around the anchor.
    pub fn move_anchor(&mut self, cmd: usize, d: Vec2) -> bool {
        if !d.is_finite() {
            return false;
        }
        let mut moved = false;
        for (c, slot) in self.anchor_group(cmd) {
            if let Some(p) = self.point(c, slot) {
                moved |= self.set_point(c, slot, p + d);
            }
        }
        moved
    }

    /// Bends segment `cmd` so its point at `t` moves by `d` while its ends stay put: dragging a
    /// curve, as the Selection tool does. The segment becomes a cubic and its two control
    /// points take the smallest moves that carry that point exactly (the least-squares
    /// solution of `b₁·o₁ + b₂·o₂ = d` with the Bernstein weights `b₁ = 3(1−t)²t`,
    /// `b₂ = 3(1−t)t²`). `t` is kept off the very ends, where no handle move could reach.
    pub fn bend_segment(&mut self, cmd: usize, t: f32, d: Vec2) -> bool {
        if !t.is_finite() || !d.is_finite() || self.segment_start(cmd).is_none() {
            return false;
        }
        self.to_cubic(cmd);
        let t = t.clamp(0.02, 0.98);
        let (b1, b2) = (3.0 * (1.0 - t) * (1.0 - t) * t, 3.0 * (1.0 - t) * t * t);
        let norm = b1 * b1 + b2 * b2;
        let (Some(c1), Some(c2)) = (self.point(cmd, C1), self.point(cmd, C2)) else { return false };
        self.set_point(cmd, C1, c1 + d * (b1 / norm)) && self.set_point(cmd, C2, c2 + d * (b2 / norm))
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

/// The first direction of `cands` (vectors from an end anchor) of some length, unit length.
fn leaving(cands: [Vec2; 3]) -> Option<Vec2> {
    cands.into_iter().find(|v| v.length() > 1e-4).map(Vec2::normalized)
}

/// One cubic in place of `a` then `b` (see [`Path::merge_at`]). `None` for non-finite input.
fn merge_cubics(a: [Vec2; 4], b: [Vec2; 4]) -> Option<[Vec2; 4]> {
    use crate::walk::cubic_point;
    const N: usize = 24;
    let (p0, p3) = (a[0], b[3]);
    if !a.iter().chain(&b).all(|p| p.is_finite()) {
        return None;
    }
    // Direction of travel out of each end (a handle on its anchor: the next point along).
    let span = p0.distance(p3);
    let t1 = leaving([a[1] - p0, a[2] - p0, a[3] - p0]).unwrap_or_else(|| (p3 - p0).normalized());
    let t2 = leaving([b[2] - p3, b[1] - p3, b[0] - p3]).unwrap_or_else(|| (p0 - p3).normalized());
    let poly = |c: &[Vec2; 4]| c[0].distance(c[1]) + c[1].distance(c[2]) + c[2].distance(c[3]);
    // (The points are finite, so the lengths are too.)
    if poly(&a) + poly(&b) <= 1e-6 {
        // Nothing of any length: the straight cubic between the ends.
        return Some([p0, p0.lerp(p3, 1.0 / 3.0), p0.lerp(p3, 2.0 / 3.0), p3]);
    }
    // Where the middle anchor falls on the new curve, `tau`: had the two been cut from one
    // cubic at `tau`, the middle anchor's handles would be in the ratio tau : 1 − tau (de
    // Casteljau), so that ratio places it exactly; a corner falls back on the lengths.
    let (inn, out) = (a[3].distance(a[2]), b[1].distance(b[0]));
    let tau = if inn + out > 1e-6 { inn / (inn + out) } else { poly(&a) / (poly(&a) + poly(&b)) };
    let tau = if tau.is_finite() { tau.clamp(0.02, 0.98) } else { 0.5 };
    // Samples of the old curve, and where each starts on the new one.
    let pts: Vec<Vec2> = (0..=N).map(|i| cubic_point(&a, i as f32 / N as f32)).chain((1..=N).map(|i| cubic_point(&b, i as f32 / N as f32))).collect();
    let mut u: Vec<f32> = (0..=N).map(|i| tau * i as f32 / N as f32).chain((1..=N).map(|i| tau + (1.0 - tau) * i as f32 / N as f32)).collect();
    let mut best = [p0, p0 + t1 * (span / 3.0), p3 + t2 * (span / 3.0), p3];
    for _ in 0..16 {
        // Least squares for the handle lengths with the parameters fixed (Schneider's
        // generateBezier).
        let (mut c00, mut c01, mut c11, mut x0, mut x1) = (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64);
        for (q, &t) in pts.iter().zip(&u) {
            let s = 1.0 - t;
            let (b0, b1, b2, b3) = (s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t);
            let (v1, v2) = (t1 * b1, t2 * b2);
            let rest = *q - (p0 * (b0 + b1) + p3 * (b2 + b3));
            c00 += f64::from(v1.dot(v1));
            c01 += f64::from(v1.dot(v2));
            c11 += f64::from(v2.dot(v2));
            x0 += f64::from(v1.dot(rest));
            x1 += f64::from(v2.dot(rest));
        }
        let det = c00 * c11 - c01 * c01;
        let (mut l1, mut l2) = if det.abs() > 1e-12 { (((x0 * c11 - x1 * c01) / det) as f32, ((c00 * x1 - c01 * x0) / det) as f32) } else { (span / 3.0, span / 3.0) };
        // A handle pointing backwards (or too short to mean anything) is no fit: a third of
        // the way, as a straight segment would have.
        if !(l1.is_finite() && l2.is_finite()) || l1 < span * 1e-3 || l2 < span * 1e-3 {
            (l1, l2) = (span / 3.0, span / 3.0);
        }
        best = [p0, p0 + t1 * l1, p3 + t2 * l2, p3];
        // Newton steps: where on the new curve each sample is nearest.
        for (q, t) in pts.iter().zip(u.iter_mut()) {
            let s = 1.0 - *t;
            let c = &best;
            let p = cubic_point(c, *t);
            let d1 = (c[1] - c[0]) * (3.0 * s * s) + (c[2] - c[1]) * (6.0 * s * *t) + (c[3] - c[2]) * (3.0 * *t * *t);
            let d2 = (c[2] - c[1] * 2.0 + c[0]) * (6.0 * s) + (c[3] - c[2] * 2.0 + c[1]) * (6.0 * *t);
            let num = (p - *q).dot(d1);
            let den = d1.dot(d1) + (p - *q).dot(d2);
            if den.abs() > 1e-9 {
                let next = *t - num / den;
                if next.is_finite() {
                    *t = next.clamp(0.0, 1.0);
                }
            }
        }
    }
    best.iter().all(|p| p.is_finite()).then_some(best)
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
    fn bending_carries_the_grabbed_point_and_keeps_the_ends() {
        for t in [0.2f32, 0.5, 0.7] {
            let mut p = cubic_path();
            let before = p.segment_point(1, t).unwrap();
            let d = Vec2::new(3.0, -7.0);
            assert!(p.bend_segment(1, t, d));
            assert!(p.segment_point(1, t).unwrap().distance(before + d) < 1e-3, "t {t}");
            assert_eq!(p.point(0, ANCHOR), Some(Vec2::new(0.0, 0.0)));
            assert_eq!(p.point(1, ANCHOR), Some(Vec2::new(40.0, 0.0)));
        }
        // A line bends into a curve.
        let mut p = cubic_path();
        assert!(p.bend_segment(2, 0.5, Vec2::new(10.0, 0.0)));
        assert!(matches!(p.cmds[2], PathCmd::CubicTo(..)));
        assert!(p.segment_point(2, 0.5).unwrap().distance(Vec2::new(50.0, -10.0)) < 1e-3);
        assert!(!p.bend_segment(0, 0.5, Vec2::new(1.0, 0.0)), "a move is no segment");
        assert!(!p.bend_segment(1, f32::NAN, Vec2::new(1.0, 0.0)));
    }

    #[test]
    fn moving_an_anchor_takes_its_handles() {
        let mut p = cubic_path();
        p.to_cubic(2);
        let h = p.point(2, C1).unwrap();
        assert!(p.move_anchor(1, Vec2::new(5.0, 5.0)));
        assert_eq!(p.point(1, ANCHOR), Some(Vec2::new(45.0, 5.0)));
        assert_eq!(p.point(1, C2), Some(Vec2::new(35.0, 25.0)), "in-handle");
        assert_eq!(p.point(2, C1), Some(h + Vec2::new(5.0, 5.0)), "out-handle");
        assert_eq!(p.point(1, C1), Some(Vec2::new(10.0, 20.0)), "the previous anchor's handle stays");
        assert!(!p.move_anchor(1, Vec2::new(f32::NAN, 0.0)));
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

    /// Largest distance from points of `p`'s segment `cmd` to the curve `q` (sampled densely).
    fn deviation(p: &Path, q: &Path) -> f32 {
        let dense = q.flatten(&Affine::IDENTITY, 0.001);
        let qs = &dense[0].points;
        let near = |pt: Vec2| qs.windows(2).map(|w| crate::path::segment_distance(pt, w[0], w[1])).fold(f32::INFINITY, f32::min);
        p.flatten(&Affine::IDENTITY, 0.01)[0].points.iter().map(|pt| near(*pt)).fold(0.0, f32::max)
    }

    #[test]
    fn merging_undoes_a_split() {
        for t in [0.2f32, 0.5, 0.83] {
            let p = cubic_path();
            let mut q = p.clone();
            assert!(q.split_segment(1, t));
            assert!(q.merge_at(1));
            assert_eq!(q.cmds.len(), p.cmds.len());
            assert_eq!(q.point(1, ANCHOR), p.point(1, ANCHOR), "the outer anchors stay");
            assert_eq!(q.point(0, ANCHOR), p.point(0, ANCHOR));
            let (Some(a), Some(b)) = (q.point(1, C1), p.point(1, C1)) else { panic!("cubic") };
            assert!(a.distance(b) < 0.01 && q.point(1, C2).unwrap().distance(p.point(1, C2).unwrap()) < 0.01, "t {t}: {:?} vs {:?}", q.cmds[1], p.cmds[1]);
            assert!(deviation(&q, &p) < 0.01 && deviation(&p, &q) < 0.01, "t {t}");
        }
    }

    #[test]
    fn merging_two_unrelated_curves_keeps_the_ends_and_their_directions() {
        // An S: up and over, then down and under; the merged curve keeps the S's ends and the
        // directions it leaves and arrives in, and stays near it.
        let mut p = Path::new();
        p.move_to(Vec2::new(0.0, 0.0)).cubic_to(Vec2::new(0.0, -40.0), Vec2::new(50.0, -40.0), Vec2::new(50.0, 0.0)).cubic_to(Vec2::new(50.0, 40.0), Vec2::new(100.0, 40.0), Vec2::new(100.0, 0.0));
        let before = p.clone();
        assert!(p.merge_at(1));
        assert_eq!(p.anchors().iter().map(|a| a.1).collect::<Vec<_>>(), vec![Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0)]);
        let (c1, c2) = (p.point(1, C1).unwrap(), p.point(1, C2).unwrap());
        assert!(c1.x.abs() < 1e-4 && c1.y < 0.0, "leaves straight up: {c1:?}");
        assert!((c2.x - 100.0).abs() < 1e-4 && c2.y > 0.0, "arrives straight up: {c2:?}");
        assert!(deviation(&p, &before) < 12.0, "{}", deviation(&p, &before));
        // Lines merge into a line; corners (handles on their anchors) still find a direction.
        let mut l = Path::new();
        l.move_to(Vec2::new(0.0, 0.0)).line_to(Vec2::new(10.0, 0.0)).line_to(Vec2::new(20.0, 0.0));
        assert!(l.merge_at(1));
        assert_eq!(l.cmds.len(), 2);
        assert!((0..=10).all(|i| l.segment_point(1, i as f32 / 10.0).is_some_and(|q| q.y.abs() < 1e-5 && (0.0..=20.0).contains(&q.x))), "{:?}", l.cmds);
        let mut c = Path::new();
        c.move_to(Vec2::new(0.0, 0.0)).cubic_to(Vec2::new(0.0, 0.0), Vec2::new(10.0, 10.0), Vec2::new(10.0, 10.0)).cubic_to(Vec2::new(10.0, 10.0), Vec2::new(20.0, 0.0), Vec2::new(20.0, 0.0));
        assert!(c.merge_at(1) && c.cmds.iter().all(|x| match x {
            PathCmd::MoveTo(p) | PathCmd::LineTo(p) => p.is_finite(),
            PathCmd::CubicTo(a, b, p) => a.is_finite() && b.is_finite() && p.is_finite(),
            _ => true,
        }));
        // Only between two segments.
        assert!(!cubic_path().merge_at(0) && !cubic_path().merge_at(2) && !cubic_path().merge_at(9));
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
