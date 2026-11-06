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
    let [b, c, d] = *s;
    let (ab, bc, cd) = (a.lerp(b, 0.5), b.lerp(c, 0.5), c.lerp(d, 0.5));
    let (abc, bcd) = (ab.lerp(bc, 0.5), bc.lerp(cd, 0.5));
    let m = abc.lerp(bcd, 0.5);
    ([ab, abc, m], [bcd, cd, d])
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

/// Two paths with the same structure (all cubics), ready to be blended with [`Path::lerp`].
/// `None` when either path is empty.
pub fn compatible(a: &Path, b: &Path) -> Option<(Path, Path)> {
    let mut sa = subpaths(a);
    let mut sb = subpaths(b);
    if sa.is_empty() || sb.is_empty() {
        return None;
    }
    // Pair by size, largest first, keeping the original drawing order within each path.
    let order = |v: &[Sub]| {
        let mut idx: Vec<usize> = (0..v.len()).collect();
        idx.sort_by(|&i, &j| v.get(j).map_or(0.0, |s| area(s).abs()).total_cmp(&v.get(i).map_or(0.0, |s| area(s).abs())));
        idx
    };
    let (oa, ob) = (order(&sa), order(&sb));
    let n = sa.len().max(sb.len());
    let mut pa: Vec<Sub> = Vec::with_capacity(n);
    let mut pb: Vec<Sub> = Vec::with_capacity(n);
    for r in 0..n {
        let x = oa.get(r).and_then(|&i| sa.get(i)).cloned();
        let y = ob.get(r).and_then(|&i| sb.get(i)).cloned();
        let (x, y) = match (x, y) {
            (Some(x), Some(y)) => (x, y),
            // Grow from (or shrink to) a point at the centre of the partner.
            (Some(x), None) => (x.clone(), point_like(centre(&x), &x)),
            (None, Some(y)) => (point_like(centre(&y), &y), y),
            (None, None) => continue,
        };
        pa.push(x);
        pb.push(y);
    }
    sa.clear();
    sb.clear();
    for (mut x, mut y) in pa.into_iter().zip(pb) {
        // An open and a closed subpath blend as two open ones.
        if x.closed != y.closed {
            x.closed = false;
            y.closed = false;
        }
        if x.closed && area(&x) * area(&y) < 0.0 {
            y = reverse(&y);
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
    let (x, y) = compatible(a, b)?;
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
