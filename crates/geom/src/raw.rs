//! Editing raw shapes that share edges and points (ADR 0041). A layer's merged drawing is one
//! element per fill region and per stroke run (ADR 0031), so a corner or an edge they share is
//! stored once in each of them. Bending an edge or moving a point with the Selection tool has to
//! change every copy the same way, so the drawing stays joined. These are the pure parts: finding
//! the copies of an edge in a path and replacing them, making sure a path has an anchor where
//! another shape ends, and the nearest point on a curve (for snapping).

use crate::walk::cubic_point;
use crate::{Affine, Path, PathCmd, Vec2};

/// A cubic Bézier segment: start, two control points, end.
pub type Cubic = [Vec2; 4];

/// Most commands one pass looks at (keeps an edit's cost bounded).
const MAX_CMDS: usize = 100_000;

/// Segment `cmd` of `path` as a cubic from where it starts (lines and quadratics raised exactly).
/// `None` for anything that is not a segment.
pub fn segment_cubic(path: &Path, cmd: usize) -> Option<Cubic> {
    let s = path.segment_start(cmd)?;
    Some(match *path.cmds.get(cmd)? {
        PathCmd::LineTo(p) => [s, s.lerp(p, 1.0 / 3.0), s.lerp(p, 2.0 / 3.0), p],
        PathCmd::QuadTo(c, p) => [s, s.lerp(c, 2.0 / 3.0), p.lerp(c, 2.0 / 3.0), p],
        PathCmd::CubicTo(a, b, p) => [s, a, b, p],
        _ => return None,
    })
}

/// `c` mapped by `m` (an affine map takes control points to control points exactly).
pub fn transform(c: &Cubic, m: &Affine) -> Cubic {
    c.map(|p| m.apply(p))
}

/// `c` with its point at `t` moved by `d` and its ends kept: the smallest moves of the two
/// control points that carry that point exactly, as [`Path::bend_segment`] does.
pub fn bend(c: &Cubic, t: f32, d: Vec2) -> Option<Cubic> {
    let mut p = Path::new();
    p.move_to(c[0]).cubic_to(c[1], c[2], c[3]);
    if !p.bend_segment(1, t, d) {
        return None;
    }
    segment_cubic(&p, 1).filter(|b| b.iter().all(|q| q.is_finite()))
}

fn split(c: &Cubic, t: f32) -> (Cubic, Cubic) {
    let (a, b, d) = (c[0].lerp(c[1], t), c[1].lerp(c[2], t), c[2].lerp(c[3], t));
    let (e, f) = (a.lerp(b, t), b.lerp(d, t));
    let m = e.lerp(f, t);
    ([c[0], a, e, m], [m, f, d, c[3]])
}

/// The part of `c` from `t0` to `t1` (reversed when `t1 < t0`).
pub fn sub(c: &Cubic, t0: f32, t1: f32) -> Cubic {
    if t1 < t0 {
        let r = sub(c, t1, t0);
        return [r[3], r[2], r[1], r[0]];
    }
    let left = if t1 >= 1.0 { *c } else { split(c, t1.max(0.0)).0 };
    if t0 <= 0.0 || t1 <= 0.0 {
        return left;
    }
    split(&left, (t0 / t1).clamp(0.0, 1.0)).1
}

/// The parameter on `c` nearest `p`, and the distance.
pub fn nearest(c: &Cubic, p: Vec2) -> (f32, f32) {
    const N: usize = 32;
    let mut best = (0.0f32, f32::INFINITY);
    for i in 0..=N {
        let t = i as f32 / N as f32;
        let d = cubic_point(c, t).distance(p);
        if d < best.1 {
            best = (t, d);
        }
    }
    // Golden-section search around the best sample.
    let (mut lo, mut hi) = ((best.0 - 1.0 / N as f32).max(0.0), (best.0 + 1.0 / N as f32).min(1.0));
    const G: f32 = 0.618_034;
    for _ in 0..30 {
        let (m1, m2) = (hi - (hi - lo) * G, lo + (hi - lo) * G);
        if cubic_point(c, m1).distance(p) < cubic_point(c, m2).distance(p) {
            hi = m2;
        } else {
            lo = m1;
        }
    }
    let t = 0.5 * (lo + hi);
    let d = cubic_point(c, t).distance(p);
    if d < best.1 { (t, d) } else { best }
}

fn bounds_meet(a: &Cubic, b: &Cubic, pad: f32) -> bool {
    let lo = |c: &Cubic| c.iter().fold(Vec2::new(f32::INFINITY, f32::INFINITY), |m, p| Vec2::new(m.x.min(p.x), m.y.min(p.y)));
    let hi = |c: &Cubic| c.iter().fold(Vec2::new(f32::NEG_INFINITY, f32::NEG_INFINITY), |m, p| Vec2::new(m.x.max(p.x), m.y.max(p.y)));
    let (al, ah, bl, bh) = (lo(a), hi(a), lo(b), hi(b));
    al.x <= bh.x + pad && bl.x <= ah.x + pad && al.y <= bh.y + pad && bl.y <= ah.y + pad
}

/// Where `c` runs along `of`: the parameters on `of` of its two ends, when both ends and three
/// points between lie within `tol` of it and it is not a single point there.
fn along(c: &Cubic, of: &Cubic, tol: f32) -> Option<(f32, f32)> {
    let (u0, d0) = nearest(of, c[0]);
    let (u1, d1) = nearest(of, c[3]);
    if d0 > tol || d1 > tol || (u1 - u0).abs() < 1e-4 {
        return None;
    }
    [0.25, 0.5, 0.75].iter().all(|&t| nearest(of, cubic_point(c, t)).1 <= tol).then_some((u0, u1))
}

/// `piece` with its ends moved onto `a` and `b`, the handles moving with them (the anchors of a
/// path stay exactly where they were, so it stays joined to its neighbours).
fn pinned(piece: &Cubic, a: Vec2, b: Vec2) -> PathCmd {
    PathCmd::CubicTo(piece[1] + (a - piece[0]), piece[2] + (b - piece[3]), b)
}

/// Replaces every stretch of `path` that runs along `old` with the same stretch of `new` (the
/// edge `old` bent into `new`; their ends are the same). A segment lying along part of `old`
/// takes that part of `new`; a segment that `old` lies along part of is split at `old`'s ends
/// first. Returns how many segments changed.
pub fn follow(path: &mut Path, old: &Cubic, new: &Cubic, tol: f32) -> usize {
    if !(tol.is_finite() && tol > 0.0) || !old.iter().chain(new).all(|p| p.is_finite()) {
        return 0;
    }
    let mut changed = 0;
    let mut k = 0;
    while k < path.cmds.len().min(MAX_CMDS) {
        let Some(s) = segment_cubic(path, k) else {
            k += 1;
            continue;
        };
        if !bounds_meet(&s, old, tol) {
            k += 1;
            continue;
        }
        if let Some((u0, u1)) = along(&s, old, tol) {
            // All of this segment is a stretch of the edge.
            if let Some(c) = path.cmds.get_mut(k) {
                *c = pinned(&sub(new, u0, u1), s[0], s[3]);
                changed += 1;
            }
            k += 1;
            continue;
        }
        let Some((s0, s1)) = along(old, &s, tol) else {
            k += 1;
            continue;
        };
        // The edge is a stretch of this (longer) segment: cut it out and replace it.
        let (lo, hi) = (s0.min(s1), s0.max(s1));
        const END: f32 = 1e-4;
        if hi < 1.0 - END && !path.split_segment(k, hi) {
            k += 1;
            continue;
        }
        let mid = if lo > END && path.split_segment(k, lo / hi.max(END)) { k + 1 } else { k };
        if let (Some(m), Some(start)) = (segment_cubic(path, mid), path.segment_start(mid)) {
            let piece = if s0 <= s1 { *new } else { [new[3], new[2], new[1], new[0]] };
            if let Some(c) = path.cmds.get_mut(mid) {
                *c = pinned(&piece, start, m[3]);
                changed += 1;
            }
        }
        k = mid + 1;
    }
    changed
}

/// The anchors of `path` at `p` (within `tol`), as the commands that name them. When none is
/// there but a segment passes within `tol` (another shape ends on this one's edge), the segment
/// is split there first, so the point can move with the shapes that end on it.
pub fn anchors_at(path: &mut Path, p: Vec2, tol: f32) -> Vec<usize> {
    let near = |path: &Path| path.editable_anchors().into_iter().filter(|(_, q)| q.distance(p) <= tol).map(|(i, _)| i).collect::<Vec<_>>();
    let found = near(path);
    if !found.is_empty() || path.cmds.len() >= MAX_CMDS {
        return found;
    }
    match path.nearest_segment(p, &Affine::IDENTITY) {
        Some((cmd, t, d)) if d <= tol && t > 0.0 && t < 1.0 && path.split_segment(cmd, t) => vec![cmd],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(a: (f32, f32), b: (f32, f32)) -> Cubic {
        let (a, b) = (Vec2::new(a.0, a.1), Vec2::new(b.0, b.1));
        [a, a.lerp(b, 1.0 / 3.0), a.lerp(b, 2.0 / 3.0), b]
    }

    #[test]
    fn a_bent_edge_passes_through_the_dragged_point_and_keeps_its_ends() {
        let c = line((0.0, 0.0), (90.0, 0.0));
        let Some(b) = bend(&c, 0.5, Vec2::new(0.0, 30.0)) else { panic!("no bend") };
        assert_eq!((b[0], b[3]), (c[0], c[3]));
        assert!(cubic_point(&b, 0.5).distance(Vec2::new(45.0, 30.0)) < 1e-3, "{:?}", cubic_point(&b, 0.5));
        // Off centre too: the grabbed point goes exactly where the pointer took it.
        let Some(b) = bend(&c, 0.2, Vec2::new(5.0, -12.0)) else { panic!("no bend") };
        assert!(cubic_point(&b, 0.2).distance(Vec2::new(23.0, -12.0)) < 1e-3);
    }

    #[test]
    fn every_copy_of_an_edge_follows_a_bend() {
        // A fill's outline (a rectangle) shares its top edge with a stroke drawn the other way.
        let mut fill = Path::rect(0.0, 0.0, 90.0, 60.0);
        fill.close_explicitly();
        let old = line((90.0, 0.0), (0.0, 0.0));
        let Some(new) = bend(&old, 0.5, Vec2::new(0.0, -20.0)) else { panic!("no bend") };
        assert_eq!(follow(&mut fill, &old, &new, 0.2), 1);
        // The top now bulges up through (45, −20); the other sides are untouched.
        let top = fill.cmds.iter().position(|c| matches!(c, PathCmd::CubicTo(..))).and_then(|k| segment_cubic(&fill, k));
        let Some(top) = top else { panic!("no curve: {:?}", fill.cmds) };
        assert!(cubic_point(&top, 0.5).distance(Vec2::new(45.0, -20.0)) < 1e-3);
        assert_eq!(fill.cmds.iter().filter(|c| matches!(c, PathCmd::LineTo(_))).count(), 3);
        // A longer segment that the edge is part of is cut at the edge's ends.
        let mut long = Path::line(Vec2::new(-30.0, 0.0), Vec2::new(120.0, 0.0));
        assert_eq!(follow(&mut long, &old, &new, 0.2), 1);
        assert_eq!(long.cmds.len(), 4, "{:?}", long.cmds);
        let a = long.anchors().into_iter().map(|(_, p)| p).collect::<Vec<_>>();
        assert!(a.iter().any(|p| p.distance(Vec2::ZERO) < 1e-3) && a.iter().any(|p| p.distance(Vec2::new(90.0, 0.0)) < 1e-3), "{a:?}");
        let Some(mid) = segment_cubic(&long, 2) else { panic!() };
        assert!(cubic_point(&mid, 0.5).distance(Vec2::new(45.0, -20.0)) < 1e-2, "{:?}", cubic_point(&mid, 0.5));
        // Something merely nearby is left alone.
        let mut other = Path::line(Vec2::new(0.0, 5.0), Vec2::new(90.0, 5.0));
        assert_eq!(follow(&mut other, &old, &new, 0.2), 0);
    }

    #[test]
    fn a_shape_gets_an_anchor_where_another_ends_on_it() {
        let mut p = Path::line(Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0));
        assert_eq!(anchors_at(&mut p, Vec2::new(100.05, 0.0), 0.2), vec![1], "an end that is there");
        assert_eq!(anchors_at(&mut p, Vec2::new(40.0, 0.1), 0.2), vec![1], "split there");
        assert_eq!(p.cmds.len(), 3);
        assert!(anchors_at(&mut p, Vec2::new(40.0, 5.0), 0.2).is_empty());
    }

    #[test]
    fn nearest_finds_the_closest_point_on_a_curve() {
        let c = [Vec2::new(0.0, 0.0), Vec2::new(0.0, 50.0), Vec2::new(100.0, 50.0), Vec2::new(100.0, 0.0)];
        let (t, d) = nearest(&c, Vec2::new(50.0, 60.0));
        assert!((t - 0.5).abs() < 1e-3 && (d - 22.5).abs() < 1e-2, "{t} {d}");
    }
}
