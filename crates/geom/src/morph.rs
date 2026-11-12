//! Shape morphing: turns two arbitrary paths into a compatible pair (same subpaths, same number
//! of cubic segments, aligned start points and windings), so a shape tween is a plain lerp.
//!
//! Method (standard, from the public literature on polygon and Bézier blending):
//! 1. every segment becomes a cubic (lines and quadratics are degree-elevated exactly);
//! 2. subpaths are paired largest to largest by area; a missing partner is grown from, or
//!    shrunk to, the centre of the subpath it pairs with;
//! 3. the shorter subpath of each pair is subdivided, longest segment first (de Casteljau at
//!    the middle), until both have the same number of segments;
//! 4. closed subpaths are aligned: opposite windings are reversed, and the start of the second
//!    is rotated to the anchor that minimizes the summed squared distance between anchors.
//!
//! Shape hints (ADR 0023) steer steps 2 to 4: a hint is a point on the first outline and the
//! point on the second that it must become. Subpaths joined by a hint are paired with each
//! other; each outline is cut at its hinted points, and the pieces between consecutive hints
//! correspond in order (each pair subdivided to the same count), so the hinted points meet
//! exactly. Hints that cross the order of the others are dropped (the longest run in order
//! is kept), as are hints whose subpaths are already paired another way.

use crate::{Path, PathCmd, Vec2};

/// A cubic segment: control points and end point (the start is the previous end).
type Seg = [Vec2; 3];

#[derive(Clone, Debug, PartialEq)]
struct Sub {
    start: Vec2,
    segs: Vec<Seg>,
    closed: bool,
}

/// Most segments a subpath is subdivided to (bounds the work on huge paths).
const MAX_SEGS: usize = 4096;
/// Above this many anchors the start-point search samples a subset of rotations.
const ALIGN_EXACT: usize = 256;

fn subpaths(p: &Path) -> Vec<Sub> {
    let mut out: Vec<Sub> = Vec::new();
    let mut cur: Option<Sub> = None;
    let mut last = Vec2::new(0.0, 0.0);
    let third = |a: Vec2, b: Vec2| [a.lerp(b, 1.0 / 3.0), a.lerp(b, 2.0 / 3.0), b];
    for c in &p.cmds {
        match *c {
            PathCmd::MoveTo(q) => {
                if let Some(s) = cur.take().filter(|s| !s.segs.is_empty()) {
                    out.push(s);
                }
                cur = Some(Sub { start: q, segs: Vec::new(), closed: false });
                last = q;
            }
            PathCmd::LineTo(q) => {
                cur.get_or_insert(Sub { start: last, segs: Vec::new(), closed: false }).segs.push(third(last, q));
                last = q;
            }
            PathCmd::QuadTo(c1, q) => {
                // Exact degree elevation: c = p + 2/3 (c1 - p), d = q + 2/3 (c1 - q).
                let seg = [last + (c1 - last) * (2.0 / 3.0), q + (c1 - q) * (2.0 / 3.0), q];
                cur.get_or_insert(Sub { start: last, segs: Vec::new(), closed: false }).segs.push(seg);
                last = q;
            }
            PathCmd::CubicTo(c1, c2, q) => {
                cur.get_or_insert(Sub { start: last, segs: Vec::new(), closed: false }).segs.push([c1, c2, q]);
                last = q;
            }
            PathCmd::Close => {
                if let Some(mut s) = cur.take() {
                    // An explicit closing edge, so every anchor is a segment end.
                    if s.start.distance(last) > 1e-6 {
                        s.segs.push(third(last, s.start));
                    }
                    s.closed = true;
                    last = s.start;
                    if !s.segs.is_empty() {
                        out.push(s);
                    }
                }
            }
        }
    }
    if let Some(s) = cur.filter(|s| !s.segs.is_empty()) {
        out.push(s);
    }
    out
}

fn anchors(s: &Sub) -> impl Iterator<Item = Vec2> + '_ {
    std::iter::once(s.start).chain(s.segs.iter().map(|g| g[2]))
}

/// Signed area of the anchor polygon (enough to compare windings and sizes).
fn area(s: &Sub) -> f32 {
    let pts: Vec<Vec2> = anchors(s).collect();
    let mut a = 0.0;
    for w in pts.windows(2) {
        a += w[0].cross(w[1]);
    }
    if let (Some(f), Some(l)) = (pts.first(), pts.last()) {
        a += l.cross(*f);
    }
    a * 0.5
}

fn centre(s: &Sub) -> Vec2 {
    let (mut c, mut n) = (Vec2::new(0.0, 0.0), 0.0);
    for p in anchors(s) {
        c = c + p;
        n += 1.0;
    }
    if n > 0.0 { c * (1.0 / n) } else { s.start }
}

/// A subpath of `n` zero-length segments at `p`, shaped like `like` (closed or open).
fn point_like(p: Vec2, like: &Sub) -> Sub {
    Sub { start: p, segs: vec![[p, p, p]; like.segs.len().max(1)], closed: like.closed }
}

fn split(a: Vec2, s: &Seg) -> (Seg, Seg) {
    split_at(a, s, 0.5)
}

/// De Casteljau split of the cubic from `a` along `s` at parameter `t`.
fn split_at(a: Vec2, s: &Seg, t: f32) -> (Seg, Seg) {
    let [b, c, d] = *s;
    let (ab, bc, cd) = (a.lerp(b, t), b.lerp(c, t), c.lerp(d, t));
    let (abc, bcd) = (ab.lerp(bc, t), bc.lerp(cd, t));
    let m = abc.lerp(bcd, t);
    ([ab, abc, m], [bcd, cd, d])
}

fn point_at(a: Vec2, s: &Seg, t: f32) -> Vec2 {
    let [b, c, d] = *s;
    let u = 1.0 - t;
    a * (u * u * u) + b * (3.0 * u * u * t) + c * (3.0 * u * t * t) + d * (t * t * t)
}

/// The anchor segment `i` starts at.
fn seg_start(s: &Sub, i: usize) -> Vec2 {
    if i == 0 { s.start } else { s.segs.get(i - 1).map_or(s.start, |g| g[2]) }
}

/// A place on a subpath: segment `seg` at parameter `t` (0..=1).
#[derive(Clone, Copy, Debug, PartialEq)]
struct At {
    seg: usize,
    t: f32,
}

impl At {
    fn param(self) -> f32 {
        self.seg as f32 + self.t
    }
}

/// Coarse samples per segment before the nearest-point search refines.
const NEAREST_SAMPLES: usize = 16;

/// The place on `s` nearest `p`, the point there and its squared distance to `p`.
fn nearest_on(s: &Sub, p: Vec2) -> Option<(At, Vec2, f32)> {
    let mut best: Option<(At, Vec2, f32)> = None;
    let d2 = |q: Vec2| {
        let d = q - p;
        d.dot(d)
    };
    for (i, g) in s.segs.iter().enumerate() {
        let a = seg_start(s, i);
        // A cubic lies inside its control points' box: skip segments that can't be nearer.
        if let Some((_, _, bd)) = best {
            let (lo, hi) = [g[0], g[1], g[2]].iter().fold((a, a), |(lo, hi), q| (Vec2::new(lo.x.min(q.x), lo.y.min(q.y)), Vec2::new(hi.x.max(q.x), hi.y.max(q.y))));
            let (dx, dy) = ((lo.x - p.x).max(p.x - hi.x).max(0.0), (lo.y - p.y).max(p.y - hi.y).max(0.0));
            if dx * dx + dy * dy > bd {
                continue;
            }
        }
        let mut bt = 0.0f32;
        let mut bdist = f32::INFINITY;
        for k in 0..=NEAREST_SAMPLES {
            let t = k as f32 / NEAREST_SAMPLES as f32;
            let d = d2(point_at(a, g, t));
            if d < bdist {
                (bt, bdist) = (t, d);
            }
        }
        // Ternary search around the best sample (the distance is unimodal that close); Newton
        // below takes it the rest of the way.
        let h = 1.0 / NEAREST_SAMPLES as f32;
        let (mut lo, mut hi) = ((bt - h).max(0.0), (bt + h).min(1.0));
        for _ in 0..10 {
            let (m1, m2) = (lo + (hi - lo) / 3.0, hi - (hi - lo) / 3.0);
            if d2(point_at(a, g, m1)) < d2(point_at(a, g, m2)) {
                hi = m2;
            } else {
                lo = m1;
            }
        }
        // Squared distance is flat at its minimum (f32 can't see past ~1e-4 there): finish with
        // Newton steps on its derivative, (B - p)·B', which crosses zero sharply.
        let (blo, bhi) = ((bt - h).max(0.0), (bt + h).min(1.0));
        let mut t = (lo + hi) * 0.5;
        for _ in 0..4 {
            let [b, c, d] = *g;
            let u = 1.0 - t;
            let d1 = (b - a) * (3.0 * u * u) + (c - b) * (6.0 * u * t) + (d - c) * (3.0 * t * t);
            let d2v = (c - b * 2.0 + a) * (6.0 * u) + (d - c * 2.0 + b) * (6.0 * t);
            let e = point_at(a, g, t) - p;
            let (f1, f2) = (e.dot(d1), d1.dot(d1) + e.dot(d2v));
            if f2.abs() < 1e-12 || !f2.is_finite() {
                break;
            }
            t = (t - f1 / f2).clamp(blo, bhi);
        }
        let q = point_at(a, g, t);
        let (t, q, d) = if d2(q) <= bdist { (t, q, d2(q)) } else { (bt, point_at(a, g, bt), bdist) };
        if best.is_none_or(|(_, _, b)| d < b) {
            best = Some((At { seg: i, t }, q, d));
        }
    }
    best.filter(|(_, q, d)| q.is_finite() && d.is_finite())
}

/// The subpath of `subs` nearest `p`, and the place on it.
fn locate(subs: &[Sub], p: Vec2) -> Option<(usize, At, Vec2, f32)> {
    let mut best: Option<(usize, At, Vec2, f32)> = None;
    for (i, s) in subs.iter().enumerate() {
        if let Some((at, q, d)) = nearest_on(s, p)
            && best.is_none_or(|b| d < b.3)
        {
            best = Some((i, at, q, d));
        }
    }
    best
}

/// The point on `path`'s outline nearest `p`, and its distance from `p` (`None` for an empty
/// path). What shape hints snap to.
pub fn nearest_point(path: &Path, p: Vec2) -> Option<(Vec2, f32)> {
    if !p.is_finite() {
        return None;
    }
    let (_, _, q, d) = locate(&subpaths(path), p)?;
    Some((q, d.sqrt()))
}

/// Parameters this close count as the same place (or as an anchor).
const PARAM_EPS: f32 = 1e-4;

/// Cuts `s` at each place in `at`, so each becomes an anchor; returns their anchor indices
/// (anchor 0 is `start`, anchor k ends segment k-1; on a closed subpath the end is anchor 0).
fn cut(s: &mut Sub, at: &[At]) -> Vec<usize> {
    let n = s.segs.len();
    let mut order: Vec<usize> = (0..at.len()).collect();
    order.sort_by(|&i, &j| at.get(i).map_or(0.0, |a| a.param()).total_cmp(&at.get(j).map_or(0.0, |a| a.param())));
    let mut anchors = vec![0usize; at.len()];
    let mut segs: Vec<Seg> = Vec::with_capacity(n + at.len());
    let mut next = order.iter().peekable();
    for (i, g) in s.segs.iter().enumerate() {
        let mut a = seg_start(s, i);
        let mut cur = *g;
        let mut done = 0.0f32; // the part of the original segment already pushed
        while let Some(&&h) = next.peek() {
            let Some(place) = at.get(h).filter(|p| p.seg.min(n.saturating_sub(1)) == i) else { break };
            next.next();
            let t = place.t.clamp(0.0, 1.0);
            let local = if done >= 1.0 - PARAM_EPS { 1.0 } else { ((t - done) / (1.0 - done)).clamp(0.0, 1.0) };
            let slot = anchors.get_mut(h);
            if local <= PARAM_EPS {
                if let Some(slot) = slot {
                    *slot = segs.len();
                }
            } else if local >= 1.0 - PARAM_EPS {
                // The end of this segment: the anchor after it.
                if let Some(slot) = slot {
                    *slot = segs.len() + 1;
                }
            } else {
                let (l, r) = split_at(a, &cur, local);
                segs.push(l);
                if let Some(slot) = slot {
                    *slot = segs.len();
                }
                a = l[2];
                cur = r;
                done = t;
            }
        }
        segs.push(cur);
    }
    let m = segs.len();
    s.segs = segs;
    for a in &mut anchors {
        if s.closed && *a >= m {
            *a = 0;
        }
        *a = (*a).min(m);
    }
    anchors
}

/// The segments of `s` from anchor `from` to anchor `to`, as an open subpath.
fn piece(s: &Sub, from: usize, to: usize) -> Sub {
    Sub { start: seg_start(s, from), segs: s.segs.get(from..to).map(<[Seg]>::to_vec).unwrap_or_default(), closed: false }
}

/// The longest run of `v` that strictly increases (indices into `v`, in order).
fn increasing_run(v: &[f32]) -> Vec<usize> {
    let n = v.len();
    let mut len = vec![1usize; n];
    let mut prev: Vec<Option<usize>> = vec![None; n];
    for (i, &b) in v.iter().enumerate() {
        for (j, &a) in v.iter().enumerate().take(i) {
            let (Some(&lj), Some(&li)) = (len.get(j), len.get(i)) else { continue };
            if a + PARAM_EPS < b && lj + 1 > li {
                if let Some(slot) = len.get_mut(i) {
                    *slot = lj + 1;
                }
                if let Some(slot) = prev.get_mut(i) {
                    *slot = Some(j);
                }
            }
        }
    }
    let mut end = (0..n).max_by_key(|&i| (len.get(i).copied().unwrap_or(0), std::cmp::Reverse(i)));
    let mut out = Vec::new();
    while let Some(i) = end {
        out.push(i);
        end = prev.get(i).copied().flatten();
    }
    out.reverse();
    out
}

/// Approximate length of a cubic: the mean of its chord and control polygon (exact for lines,
/// close for the gentle segments outlines are made of).
fn seg_len(a: Vec2, g: &Seg) -> f32 {
    let poly = a.distance(g[0]) + g[0].distance(g[1]) + g[1].distance(g[2]);
    (poly + a.distance(g[2])) * 0.5
}

/// How far along `s` each anchor is, as fractions of its length (0 first, 1 last). `None`
/// for a piece of no length.
fn fractions(s: &Sub) -> Option<Vec<f32>> {
    let mut acc = vec![0.0f32];
    let mut total = 0.0f32;
    for (i, g) in s.segs.iter().enumerate() {
        total += seg_len(seg_start(s, i), g);
        acc.push(total);
    }
    if !(total > 1e-6 && total.is_finite()) {
        return None;
    }
    Some(acc.into_iter().map(|c| c / total).collect())
}

/// Adds anchors to `s` (whose anchors sit at fractions `own`) at the fractions in `other`
/// it doesn't have yet.
fn cut_at_fractions(s: &mut Sub, own: &[f32], other: &[f32]) {
    const SAME: f32 = 1e-4;
    let mut at = Vec::new();
    for &f in other {
        if own.iter().any(|&g| (g - f).abs() <= SAME) {
            continue;
        }
        // The segment it falls in, and how far into it (by length, taken as linear in t).
        let Some(i) = own.windows(2).position(|w| f > w[0] && f < w[1]) else { continue };
        let (Some(&lo), Some(&hi)) = (own.get(i), own.get(i + 1)) else { continue };
        at.push(At { seg: i, t: ((f - lo) / (hi - lo).max(1e-9)).clamp(0.0, 1.0) });
    }
    if !at.is_empty() {
        cut(s, &at);
    }
}

/// A pair of subpaths made compatible so that each hint's two places meet: both are cut at
/// the hinted places and the pieces between consecutive hints correspond in order. `None`
/// when no hint survives (the caller aligns the pair the usual way).
fn hinted_pair(x: &Sub, y: &Sub, hints: &[(Vec2, Vec2)]) -> Option<(Sub, Sub)> {
    let mut placed: Vec<(At, At)> = hints.iter().filter_map(|&(p, q)| Some((nearest_on(x, p)?.0, nearest_on(y, q)?.0))).collect();
    if placed.is_empty() {
        return None;
    }
    placed.sort_by(|a, b| a.0.param().total_cmp(&b.0.param()));
    placed.dedup_by(|b, a| (b.0.param() - a.0.param()).abs() <= PARAM_EPS);
    let closed = x.closed && y.closed;
    let ny = y.segs.len() as f32;
    // On a loop the order counts from the first hint, wrapping round.
    let y0 = placed.first().map_or(0.0, |h| h.1.param());
    let key: Vec<f32> = placed.iter().map(|h| if closed { (h.1.param() - y0).rem_euclid(ny.max(1.0)) } else { h.1.param() }).collect();
    let keep: Vec<(At, At)> = if closed {
        // The first hint anchors the loop; the rest keep the longest run in order after it.
        let rest: Vec<usize> = (1..key.len()).filter(|&i| key.get(i).is_some_and(|&k| k > PARAM_EPS && k < ny - PARAM_EPS)).collect();
        let run = increasing_run(&rest.iter().filter_map(|&i| key.get(i).copied()).collect::<Vec<_>>());
        std::iter::once(0).chain(run.into_iter().filter_map(|r| rest.get(r).copied())).filter_map(|i| placed.get(i).copied()).collect()
    } else {
        increasing_run(&key).into_iter().filter_map(|i| placed.get(i).copied()).collect()
    };
    let (mut x, mut y) = (x.clone(), y.clone());
    let mut ax = cut(&mut x, &keep.iter().map(|h| h.0).collect::<Vec<_>>());
    let mut ay = cut(&mut y, &keep.iter().map(|h| h.1).collect::<Vec<_>>());
    if closed {
        // Both loops start at the first hint.
        let (rx, ry) = (ax.first().copied().unwrap_or(0), ay.first().copied().unwrap_or(0));
        let (nx, ny) = (x.segs.len().max(1), y.segs.len().max(1));
        x = rotate(&x, rx);
        y = rotate(&y, ry);
        ax.iter_mut().for_each(|a| *a = (*a + nx - rx % nx) % nx);
        ay.iter_mut().for_each(|a| *a = (*a + ny - ry % ny) % ny);
    }
    // Piece boundaries: hints in order, from the start to the end.
    let bounds = |a: &[usize], n: usize| -> Vec<usize> {
        let mut b: Vec<usize> = if closed { a.to_vec() } else { std::iter::once(0).chain(a.iter().copied()).collect() };
        b.push(n);
        b
    };
    let (bx, by) = (bounds(&ax, x.segs.len()), bounds(&ay, y.segs.len()));
    if bx.windows(2).any(|w| w[0] > w[1]) || by.windows(2).any(|w| w[0] > w[1]) {
        return None; // rounding put two hints out of order: morph the usual way
    }
    let (mut ox, mut oy) = (Sub { start: x.start, segs: Vec::new(), closed: x.closed }, Sub { start: y.start, segs: Vec::new(), closed: y.closed });
    for (wx, wy) in bx.windows(2).zip(by.windows(2)) {
        let (mut px, mut py) = (piece(&x, wx[0], wx[1]), piece(&y, wy[0], wy[1]));
        let n = px.segs.len().max(py.segs.len());
        if n == 0 {
            continue;
        }
        // A hint on an end point leaves an empty piece: it stays a point while the other moves.
        if px.segs.is_empty() {
            px = point_like(px.start, &py);
        }
        if py.segs.is_empty() {
            py = point_like(py.start, &px);
        }
        // Points correspond by how far along the piece they are: each piece gets the other's
        // anchors at the same fractions of its length.
        if ox.segs.len() + px.segs.len() + py.segs.len() > MAX_SEGS {
            return None;
        }
        let (fx, fy) = (fractions(&px), fractions(&py));
        if let (Some(fx), Some(fy)) = (&fx, &fy) {
            cut_at_fractions(&mut px, fx, fy);
            cut_at_fractions(&mut py, fy, fx);
        }
        let n = px.segs.len().max(py.segs.len());
        subdivide(&mut px, n);
        subdivide(&mut py, n);
        if px.segs.len() != py.segs.len() {
            return None;
        }
        ox.segs.extend(px.segs);
        oy.segs.extend(py.segs);
    }
    if ox.segs.is_empty() {
        return None;
    }
    Some((ox, oy))
}

/// Subdivides the longest segments (by chord) until there are `n`.
fn subdivide(s: &mut Sub, n: usize) {
    let n = n.min(MAX_SEGS);
    while s.segs.len() < n {
        let mut best = (0usize, -1.0f32);
        let mut prev = s.start;
        for (i, g) in s.segs.iter().enumerate() {
            let len = prev.distance(g[0]) + g[0].distance(g[1]) + g[1].distance(g[2]);
            if len > best.1 {
                best = (i, len);
            }
            prev = g[2];
        }
        let i = best.0;
        let start = if i == 0 { s.start } else { s.segs.get(i - 1).map_or(s.start, |g| g[2]) };
        let Some(seg) = s.segs.get(i).copied() else { return };
        let (l, r) = split(start, &seg);
        if let Some(slot) = s.segs.get_mut(i) {
            *slot = l;
        }
        s.segs.insert(i + 1, r);
    }
}

fn reverse(s: &Sub) -> Sub {
    // Anchors in reverse; each segment's control points swap.
    let pts: Vec<Vec2> = anchors(s).collect();
    let mut segs = Vec::with_capacity(s.segs.len());
    for (i, g) in s.segs.iter().enumerate().rev() {
        let from = pts.get(i).copied().unwrap_or(s.start);
        segs.push([g[1], g[0], from]);
    }
    Sub { start: pts.last().copied().unwrap_or(s.start), segs, closed: s.closed }
}

/// The closed subpath started at anchor `k` (anchor 0 is `start`, anchor i ends segment i-1).
fn rotate(s: &Sub, k: usize) -> Sub {
    let n = s.segs.len();
    if n == 0 || k.is_multiple_of(n) {
        return s.clone();
    }
    let k = k % n;
    let start = s.segs.get(k - 1).map_or(s.start, |g| g[2]);
    let mut segs = Vec::with_capacity(n);
    segs.extend_from_slice(s.segs.get(k..).unwrap_or(&[]));
    segs.extend_from_slice(s.segs.get(..k).unwrap_or(&[]));
    Sub { start, segs, closed: true }
}

/// Rotates closed `b` so its anchors best match `a`'s (both have the same segment count).
fn align(a: &Sub, b: &Sub) -> Sub {
    let n = b.segs.len();
    let pa: Vec<Vec2> = a.segs.iter().map(|g| g[2]).collect();
    let pb: Vec<Vec2> = b.segs.iter().map(|g| g[2]).collect();
    if n == 0 || pa.len() != n {
        return b.clone();
    }
    let step = n.div_ceil(ALIGN_EXACT).max(1);
    let mut best = (0usize, f32::INFINITY);
    for k in (0..n).step_by(step) {
        let cost: f32 = (0..n).step_by(step).map(|i| {
            let (Some(p), Some(q)) = (pa.get(i), pb.get((i + k) % n)) else { return 0.0 };
            let d = *p - *q;
            d.dot(d)
        }).sum();
        if cost < best.1 {
            best = (k, cost);
        }
    }
    // Segment j of the result is old segment (j + k), so it ends at old anchor pb[j + k].
    rotate(b, best.0)
}

fn to_path(subs: &[Sub]) -> Path {
    let mut p = Path::default();
    for s in subs {
        p.cmds.push(PathCmd::MoveTo(s.start));
        for g in &s.segs {
            p.cmds.push(PathCmd::CubicTo(g[0], g[1], g[2]));
        }
        if s.closed {
            p.cmds.push(PathCmd::Close);
        }
    }
    p
}

/// Most open subpaths [`chain`] joins (the search is quadratic); past it they stay as they are.
const MAX_CHAIN: usize = 1024;

/// Joins the open subpaths of `paths` that meet end to end (within `tol`, turning one round
/// where needed) into longer ones, closing a chain that comes back to its start. Merge
/// Drawing cuts a shape's line at every corner; chained, the pieces are its outline again, so
/// a shape tween morphs them as it morphs the fill. Closed subpaths pass through as they are.
pub fn chain(paths: &[Path], tol: f32) -> Path {
    let (mut out, mut open): (Vec<Sub>, Vec<Sub>) = paths.iter().flat_map(subpaths).partition(|s| s.closed);
    if open.len() > MAX_CHAIN {
        out.append(&mut open);
        return to_path(&out);
    }
    let end = |s: &Sub| s.segs.last().map_or(s.start, |g| g[2]);
    let near = |a: Vec2, b: Vec2| a.distance(b) <= tol;
    open.reverse(); // taken from the back, so in drawing order
    while let Some(mut cur) = open.pop() {
        // A chain that has come back to its start is done (other pieces may meet it there).
        while !near(end(&cur), cur.start) {
            let e = end(&cur);
            if let Some(i) = open.iter().rposition(|s| near(s.start, e) || near(end(s), e)) {
                let s = open.remove(i);
                cur.segs.extend(if near(s.start, e) { s.segs } else { reverse(&s).segs });
            } else if let Some(i) = open.iter().rposition(|s| near(end(s), cur.start) || near(s.start, cur.start)) {
                let s = open.remove(i);
                let mut s = if near(end(&s), cur.start) { s } else { reverse(&s) };
                s.segs.append(&mut cur.segs);
                cur = s;
            } else {
                break;
            }
        }
        if near(end(&cur), cur.start) {
            if let Some(g) = cur.segs.last_mut() {
                g[2] = cur.start;
            }
            cur.closed = true;
        }
        out.push(cur);
    }
    to_path(&out)
}

/// Most shape hints a morph honours (Animate's a to z).
pub const MAX_HINTS: usize = 26;

/// Two paths with the same structure (all cubics), ready to be blended with [`Path::lerp`].
/// `None` when either path is empty.
pub fn compatible(a: &Path, b: &Path) -> Option<(Path, Path)> {
    compatible_hinted(a, b, &[])
}

/// [`compatible`] steered by shape hints: each `(p, q)` is a point on `a`'s outline and the
/// point on `b`'s it becomes (in the paths' own coordinates; see the module docs). No hints
/// gives exactly [`compatible`].
pub fn compatible_hinted(a: &Path, b: &Path, hints: &[(Vec2, Vec2)]) -> Option<(Path, Path)> {
    let mut sa = subpaths(a);
    let mut sb = subpaths(b);
    if sa.is_empty() || sb.is_empty() {
        return None;
    }
    // Which subpaths each hint joins (and where it sits on each).
    let joins: Vec<(usize, usize, Vec2, Vec2)> = hints
        .iter()
        .take(MAX_HINTS)
        .filter(|(p, q)| p.is_finite() && q.is_finite())
        .filter_map(|&(p, q)| {
            let (i, _, pp, _) = locate(&sa, p)?;
            let (j, _, qq, _) = locate(&sb, q)?;
            Some((i, j, pp, qq))
        })
        .collect();
    // Subpaths a hint joins pair with each other (the first hint on a subpath decides).
    let mut forced: Vec<(usize, usize)> = Vec::new();
    for &(i, j, _, _) in &joins {
        if !forced.iter().any(|&(x, y)| x == i || y == j) {
            forced.push((i, j));
        }
    }
    // The rest pair by size, largest first, keeping the original drawing order within each path.
    let order = |v: &[Sub], taken: &dyn Fn(usize) -> bool| {
        let mut idx: Vec<usize> = (0..v.len()).filter(|&i| !taken(i)).collect();
        idx.sort_by(|&i, &j| v.get(j).map_or(0.0, |s| area(s).abs()).total_cmp(&v.get(i).map_or(0.0, |s| area(s).abs())));
        idx
    };
    let oa = order(&sa, &|i| forced.iter().any(|f| f.0 == i));
    let ob = order(&sb, &|j| forced.iter().any(|f| f.1 == j));
    let n = oa.len().max(ob.len());
    let rows: Vec<(Option<usize>, Option<usize>)> = forced.iter().map(|&(i, j)| (Some(i), Some(j))).chain((0..n).map(|r| (oa.get(r).copied(), ob.get(r).copied()))).collect();
    let mut pa: Vec<Sub> = Vec::with_capacity(rows.len());
    let mut pb: Vec<Sub> = Vec::with_capacity(rows.len());
    let mut pair_hints: Vec<Vec<(Vec2, Vec2)>> = Vec::with_capacity(rows.len());
    for (ia, ib) in rows {
        let x = ia.and_then(|i| sa.get(i)).cloned();
        let y = ib.and_then(|i| sb.get(i)).cloned();
        let (x, y) = match (x, y) {
            (Some(x), Some(y)) => (x, y),
            // Grow from (or shrink to) a point at the centre of the partner.
            (Some(x), None) => (x.clone(), point_like(centre(&x), &x)),
            (None, Some(y)) => (point_like(centre(&y), &y), y),
            (None, None) => continue,
        };
        let here = joins.iter().filter(|h| Some(h.0) == ia && Some(h.1) == ib).map(|h| (h.2, h.3)).collect();
        pa.push(x);
        pb.push(y);
        pair_hints.push(here);
    }
    sa.clear();
    sb.clear();
    for ((mut x, mut y), hints) in pa.into_iter().zip(pb).zip(pair_hints) {
        // An open and a closed subpath blend as two open ones.
        if x.closed != y.closed {
            x.closed = false;
            y.closed = false;
        }
        if x.closed && area(&x) * area(&y) < 0.0 {
            y = reverse(&y);
        }
        if !hints.is_empty()
            && let Some((hx, hy)) = hinted_pair(&x, &y, &hints)
        {
            sa.push(hx);
            sb.push(hy);
            continue;
        }
        let n = x.segs.len().max(y.segs.len());
        if n > MAX_SEGS {
            return None; // too detailed to morph; the tween holds the first shape
        }
        subdivide(&mut x, n);
        subdivide(&mut y, n);
        if x.segs.len() != y.segs.len() {
            return None; // past MAX_SEGS
        }
        if x.closed {
            y = align(&x, &y);
        }
        sa.push(x);
        sb.push(y);
    }
    Some((to_path(&sa), to_path(&sb)))
}

/// The shape `t` of the way from `a` to `b` (0 = `a`, 1 = `b`), for any two paths.
///
/// Always goes through [`compatible`]: two paths with the same commands can still start at
/// different corners or wind the other way, and blending them point by point would twist.
pub fn morph(a: &Path, b: &Path, t: f32) -> Option<Path> {
    morph_hinted(a, b, &[], t)
}

/// [`morph`] steered by shape hints (see [`compatible_hinted`]).
pub fn morph_hinted(a: &Path, b: &Path, hints: &[(Vec2, Vec2)], t: f32) -> Option<Path> {
    let (x, y) = compatible_hinted(a, b, hints)?;
    if t <= 0.0 {
        return Some(x);
    }
    if t >= 1.0 {
        return Some(y);
    }
    x.lerp(&y, t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Affine, FillRule};

    fn close(a: Vec2, b: Vec2) -> bool {
        a.distance(b) < 1e-3
    }

    #[test]
    fn ends_reproduce_the_original_shapes() {
        let a = Path::rect(0.0, 0.0, 100.0, 100.0);
        let b = Path::ellipse(200.0, 50.0, 40.0, 40.0);
        let (x, y) = compatible(&a, &b).unwrap();
        assert_eq!(x.cmds.len(), y.cmds.len());
        // Same geometry as the originals (only re-parametrized): the bounds and inside tests agree.
        for (orig, comp) in [(&a, &x), (&b, &y)] {
            let (ob, cb) = (orig.bounds(), comp.bounds());
            assert!(close(ob.min, cb.min) && close(ob.max, cb.max), "{ob:?} {cb:?}");
            for p in [Vec2::new(50.0, 50.0), Vec2::new(200.0, 50.0), Vec2::new(150.0, 150.0), Vec2::new(230.0, 50.0)] {
                assert_eq!(orig.contains(p, &Affine::IDENTITY, FillRule::NonZero), comp.contains(p, &Affine::IDENTITY, FillRule::NonZero), "{p:?}");
            }
        }
        assert_eq!(morph(&a, &b, 0.0).unwrap(), x);
        assert_eq!(morph(&a, &b, 1.0).unwrap(), y);
    }

    #[test]
    fn halfway_is_in_between_and_does_not_twist() {
        // A square and the same square drawn from another corner, the other way round.
        let a = Path::rect(0.0, 0.0, 10.0, 10.0);
        let mut b = Path::default();
        b.move_to(Vec2::new(10.0, 10.0)).line_to(Vec2::new(10.0, 0.0)).line_to(Vec2::new(0.0, 0.0)).line_to(Vec2::new(0.0, 10.0)).close();
        let mid = morph(&a, &b, 0.5).unwrap();
        // Aligned and same winding: the halfway shape is the square itself, not a collapsed bow tie.
        let bb = mid.bounds();
        assert!(close(bb.min, Vec2::new(0.0, 0.0)) && close(bb.max, Vec2::new(10.0, 10.0)), "{bb:?}");
        assert!(mid.contains(Vec2::new(5.0, 5.0), &Affine::IDENTITY, FillRule::NonZero));
        // A move: halfway between two distant circles is a circle in the middle.
        let m = morph(&Path::ellipse(0.0, 0.0, 10.0, 10.0), &Path::ellipse(100.0, 0.0, 10.0, 10.0), 0.5).unwrap();
        assert!(close(m.bounds().center(), Vec2::new(50.0, 0.0)));
    }

    #[test]
    fn different_subpath_counts_grow_and_shrink() {
        // One square to two squares: the second grows from a point.
        let a = Path::rect(0.0, 0.0, 10.0, 10.0);
        let mut b = Path::rect(0.0, 0.0, 10.0, 10.0);
        b.cmds.extend(Path::rect(50.0, 0.0, 4.0, 4.0).cmds);
        let (x, y) = compatible(&a, &b).unwrap();
        assert_eq!(x.cmds.iter().filter(|c| matches!(c, PathCmd::MoveTo(_))).count(), 2);
        assert_eq!(x.cmds.len(), y.cmds.len());
        let end = morph(&a, &b, 1.0).unwrap().bounds();
        assert!(close(end.min, b.bounds().min) && close(end.max, b.bounds().max), "{end:?}");
        // Open paths, a quadratic and empty input.
        let mut line = Path::default();
        line.move_to(Vec2::new(0.0, 0.0)).quad_to(Vec2::new(5.0, 10.0), Vec2::new(10.0, 0.0));
        assert!(morph(&line, &a, 0.5).is_some());
        assert!(compatible(&Path::default(), &a).is_none());
    }

    /// Unsigned area of a path's single flattened outline.
    fn flat_area(p: &Path) -> f32 {
        let pl = p.flatten(&Affine::IDENTITY, 0.05).into_iter().next().unwrap();
        let pts = pl.points;
        let mut a = 0.0;
        for (i, q) in pts.iter().enumerate() {
            a += q.cross(pts[(i + 1) % pts.len()]);
        }
        (a * 0.5).abs()
    }

    fn anchors_of(p: &Path) -> Vec<Vec2> {
        p.cmds
            .iter()
            .filter_map(|c| match *c {
                PathCmd::MoveTo(q) | PathCmd::LineTo(q) | PathCmd::QuadTo(_, q) | PathCmd::CubicTo(_, _, q) => Some(q),
                PathCmd::Close => None,
            })
            .collect()
    }

    #[test]
    fn a_hinted_corner_lands_where_asked() {
        // A square into a circle: the top-left corner must become the circle's bottom point.
        let a = Path::rect(0.0, 0.0, 100.0, 100.0);
        let b = Path::ellipse(250.0, 50.0, 50.0, 50.0);
        let (corner, bottom) = (Vec2::new(0.0, 0.0), Vec2::new(250.0, 100.0));
        // Without the hint the corner goes somewhere else (the nearest-anchors alignment).
        let (x0, y0) = compatible(&a, &b).unwrap();
        let k0 = anchors_of(&x0).iter().position(|p| close(*p, corner)).unwrap();
        assert!(!close(anchors_of(&y0)[k0], bottom));
        let (x, y) = compatible_hinted(&a, &b, &[(corner, bottom)]).unwrap();
        assert_eq!(x.cmds.len(), y.cmds.len());
        let (ax, ay) = (anchors_of(&x), anchors_of(&y));
        let k = ax.iter().position(|p| close(*p, corner)).unwrap();
        assert!(close(ay[k], bottom), "{:?}", ay[k]);
        // Halfway, the corner's point is halfway along its way.
        let mid = morph_hinted(&a, &b, &[(corner, bottom)], 0.5).unwrap();
        assert!(close(anchors_of(&mid)[k], corner.lerp(bottom, 0.5)));
        // Natural hints (corners to the matching points of the circle, some a hair off its
        // anchors so one side has a tiny piece): points correspond by length along each piece,
        // so the in-between shape stays full instead of folding (count-based splitting gives
        // 8,100 to 8,600 here; a perfect blend about 8,950).
        let r = 50.0f32;
        let on = |deg: f32| Vec2::new(250.0 + r * deg.to_radians().cos(), 50.0 + r * deg.to_radians().sin());
        let cases: [&[(Vec2, Vec2)]; 4] = [
            &[(Vec2::new(100.0, 0.0), on(-45.0))],
            &[(Vec2::new(100.0, 0.0), on(-44.0))],
            &[(Vec2::new(100.0, 0.0), on(-45.0)), (Vec2::new(0.0, 100.0), on(135.8))],
            &[(Vec2::new(100.0, 30.0), on(-20.0)), (Vec2::new(0.0, 0.0), on(-135.0))],
        ];
        for hints in cases {
            let area = flat_area(&morph_hinted(&a, &b, hints, 0.5).unwrap());
            assert!(area > 8_800.0, "{hints:?} {area}");
        }
        // The ends are still the original shapes.
        for (orig, comp) in [(&a, &x), (&b, &y)] {
            let (ob, cb) = (orig.bounds(), comp.bounds());
            assert!(close(ob.min, cb.min) && close(ob.max, cb.max), "{ob:?} {cb:?}");
        }
    }

    #[test]
    fn several_hints_meet_in_order_and_crossing_ones_drop() {
        let a = Path::rect(0.0, 0.0, 100.0, 100.0);
        let b = Path::ellipse(250.0, 50.0, 50.0, 50.0);
        // Two hints between corners: a point on an edge (not an anchor) to one off the axes.
        let edge = Vec2::new(100.0, 30.0);
        let off = Vec2::new(250.0 + 50.0 * 0.6, 50.0 - 50.0 * 0.8);
        let hints = [(Vec2::new(0.0, 0.0), Vec2::new(250.0, 100.0)), (edge, off)];
        let (x, y) = compatible_hinted(&a, &b, &hints).unwrap();
        let (ax, ay) = (anchors_of(&x), anchors_of(&y));
        for (p, q) in hints {
            let k = ax.iter().position(|v| v.distance(p) < 1e-2).unwrap();
            assert!(ay[k].distance(q) < 1e-2, "{p:?} → {:?}, wanted {q:?}", ay[k]);
        }
        // Three hints where the third crosses the second: one of the crossing pair is dropped,
        // and the morph still pairs every segment.
        let crossing = [
            (Vec2::new(0.0, 0.0), Vec2::new(200.0, 50.0)),
            (Vec2::new(100.0, 0.0), Vec2::new(300.0, 50.0)),
            (Vec2::new(100.0, 100.0), Vec2::new(250.0, 100.0)),
            (Vec2::new(0.0, 100.0), Vec2::new(250.0, 0.0)),
        ];
        let (x, y) = compatible_hinted(&a, &b, &crossing).unwrap();
        assert_eq!(x.cmds.len(), y.cmds.len());
        let (ax, ay) = (anchors_of(&x), anchors_of(&y));
        let met = crossing.iter().filter(|(p, q)| ax.iter().position(|v| close(*v, *p)).is_some_and(|k| close(ay[k], *q))).count();
        assert!((2..=3).contains(&met), "{met}");
        // No hints is exactly the unhinted morph.
        assert_eq!(compatible_hinted(&a, &b, &[]), compatible(&a, &b));
    }

    #[test]
    fn a_hint_decides_which_subpaths_pair() {
        // A big square left and a small one right, into the same two swapped over: by size the
        // big one stays big; a hint from the big square to the small one makes it shrink.
        let mut a = Path::rect(0.0, 0.0, 40.0, 40.0);
        a.cmds.extend(Path::rect(100.0, 0.0, 10.0, 10.0).cmds);
        let mut b = Path::rect(0.0, 0.0, 10.0, 10.0);
        b.cmds.extend(Path::rect(100.0, 0.0, 40.0, 40.0).cmds);
        let mid = morph(&a, &b, 0.5).unwrap();
        // By size the big one travels right: halfway it is at x 50..90.
        assert!(!mid.contains(Vec2::new(20.0, 20.0), &Affine::IDENTITY, FillRule::NonZero));
        assert!(mid.contains(Vec2::new(70.0, 20.0), &Affine::IDENTITY, FillRule::NonZero));
        let hinted = morph_hinted(&a, &b, &[(Vec2::new(0.0, 0.0), Vec2::new(0.0, 0.0))], 0.5).unwrap();
        // Each square now stays on its side and changes size in place.
        let (x, y) = compatible_hinted(&a, &b, &[(Vec2::new(0.0, 0.0), Vec2::new(0.0, 0.0))]).unwrap();
        assert!(close(anchors_of(&x)[0], Vec2::new(0.0, 0.0)) && close(anchors_of(&y)[0], Vec2::new(0.0, 0.0)));
        assert!(hinted.contains(Vec2::new(20.0, 20.0), &Affine::IDENTITY, FillRule::NonZero));
        // Open paths take hints too, and a hint on an end point is fine.
        let mut l1 = Path::default();
        l1.move_to(Vec2::new(0.0, 0.0)).line_to(Vec2::new(100.0, 0.0));
        let mut l2 = Path::default();
        l2.move_to(Vec2::new(0.0, 50.0)).line_to(Vec2::new(50.0, 80.0)).line_to(Vec2::new(100.0, 50.0));
        let (x, y) = compatible_hinted(&l1, &l2, &[(Vec2::new(25.0, 0.0), Vec2::new(50.0, 80.0)), (Vec2::new(0.0, 0.0), Vec2::new(0.0, 50.0))]).unwrap();
        let k = anchors_of(&x).iter().position(|v| close(*v, Vec2::new(25.0, 0.0))).unwrap();
        assert!(close(anchors_of(&y)[k], Vec2::new(50.0, 80.0)));
    }

    #[test]
    fn chain_joins_pieces_that_meet_into_outlines() {
        // A triangle's sides, out of order, one drawn backwards and one a hair off its corner,
        // plus a stray line and a closed square: the sides become one closed outline.
        let (p, q, r) = (Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0), Vec2::new(5.0, 8.0));
        let sides = [Path::line(q, r), Path::line(Vec2::new(0.05, 0.0), q), Path::line(p, r)];
        let stray = Path::line(Vec2::new(50.0, 50.0), Vec2::new(60.0, 50.0));
        let square = Path::rect(100.0, 0.0, 5.0, 5.0);
        let mut all = sides.to_vec();
        all.extend([stray.clone(), square.clone()]);
        let joined = chain(&all, 0.2);
        let subs = subpaths(&joined);
        assert_eq!(subs.len(), 3, "{subs:?}");
        let tri = subs.iter().find(|s| s.segs.len() == 3).unwrap();
        assert!(tri.closed);
        let corners: Vec<Vec2> = anchors(tri).collect();
        for c in [q, r] {
            assert!(corners.iter().any(|a| close(*a, c)), "{c:?} in {corners:?}");
        }
        assert!(corners.iter().any(|a| a.distance(p) < 0.1));
        assert!(subs.iter().any(|s| !s.closed && s.segs.len() == 1), "the stray line stays open");
        assert!(subs.iter().any(|s| s.closed && s.segs.len() == 4), "the square passes through");
        // The joined triangle morphs like the filled one.
        assert!(morph(&joined, &Path::ellipse(0.0, 0.0, 5.0, 5.0), 0.5).is_some());
        assert!(chain(&[], 0.2).cmds.is_empty());
    }

    #[test]
    fn nearest_point_snaps_to_the_outline() {
        let sq = Path::rect(0.0, 0.0, 100.0, 100.0);
        let (q, d) = nearest_point(&sq, Vec2::new(50.0, 40.0)).unwrap();
        assert!(close(q, Vec2::new(50.0, 0.0)) && (d - 40.0).abs() < 1e-3, "{q:?} {d}");
        let c = Path::ellipse(0.0, 0.0, 10.0, 10.0);
        let (q, d) = nearest_point(&c, Vec2::new(30.0, 0.0)).unwrap();
        assert!(q.distance(Vec2::new(10.0, 0.0)) < 1e-2 && (d - 20.0).abs() < 1e-2);
        assert!(nearest_point(&Path::default(), Vec2::new(0.0, 0.0)).is_none());
        assert!(nearest_point(&sq, Vec2::new(f32::NAN, 0.0)).is_none());
        // Junk hints never panic.
        let junk = [(Vec2::new(f32::INFINITY, 0.0), Vec2::new(0.0, 0.0)), (Vec2::new(1e30, -1e30), Vec2::new(-1e30, 1e30))];
        assert!(compatible_hinted(&sq, &c, &junk).is_some());
    }

    #[test]
    fn huge_paths_stay_bounded() {
        let mut big = Path::default();
        big.move_to(Vec2::new(0.0, 0.0));
        for i in 1..6000 {
            big.line_to(Vec2::new(i as f32, (i % 7) as f32));
        }
        big.close();
        let small = Path::rect(0.0, 0.0, 1.0, 1.0);
        let t0 = std::time::Instant::now();
        let r = morph(&small, &big, 0.5);
        assert!(r.is_none() || r.is_some_and(|p| p.cmds.iter().all(|c| !matches!(c, PathCmd::LineTo(_)))));
        assert!(t0.elapsed().as_secs_f32() < 2.0, "{:?}", t0.elapsed());
    }
}
