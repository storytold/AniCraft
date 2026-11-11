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

// ----- points shared by several shapes (ADR 0045) ------------------------------------------------

/// One segment of a subpath: a cubic, and whether it is a straight line (kept a line when the path
/// is written back).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Seg {
    pub c: Cubic,
    pub line: bool,
}

impl Seg {
    pub fn line(a: Vec2, b: Vec2) -> Seg {
        Seg { c: [a, a.lerp(b, 1.0 / 3.0), a.lerp(b, 2.0 / 3.0), b], line: true }
    }
    /// The same segment run the other way.
    pub fn reversed(&self) -> Seg {
        Seg { c: [self.c[3], self.c[2], self.c[1], self.c[0]], line: self.line }
    }
}

/// A subpath as its segments, each starting where the one before ends. A closed one ends where
/// it starts.
#[derive(Clone, Debug, PartialEq)]
pub struct Sub {
    pub segs: Vec<Seg>,
    pub closed: bool,
}

/// `path` as subpaths (a `Close` that draws an edge back to the start gets that edge as a line;
/// a lone move is dropped).
pub fn subpaths(path: &Path) -> Vec<Sub> {
    let mut p = path.clone();
    p.close_explicitly();
    let mut out: Vec<Sub> = Vec::new();
    let mut cur: Option<Sub> = None;
    for k in 0..p.cmds.len().min(MAX_CMDS) {
        match p.cmds.get(k) {
            Some(PathCmd::MoveTo(_)) => {
                out.extend(cur.take().filter(|s| !s.segs.is_empty()));
                cur = Some(Sub { segs: Vec::new(), closed: false });
            }
            Some(PathCmd::Close) => {
                if let Some(mut s) = cur.take().filter(|s| !s.segs.is_empty()) {
                    s.closed = true;
                    out.push(s);
                }
            }
            Some(cmd) => {
                if let Some(c) = segment_cubic(&p, k) {
                    let seg = Seg { c, line: matches!(cmd, PathCmd::LineTo(_)) };
                    cur.get_or_insert_with(|| Sub { segs: Vec::new(), closed: false }).segs.push(seg);
                }
            }
            None => break,
        }
    }
    out.extend(cur.filter(|s| !s.segs.is_empty()));
    out
}

/// The path of `subs` (lines as lines, everything else as cubics).
pub fn from_subpaths(subs: &[Sub]) -> Path {
    let mut p = Path::new();
    for s in subs {
        let Some(first) = s.segs.first() else { continue };
        p.move_to(first.c[0]);
        for g in &s.segs {
            if g.line {
                p.line_to(g.c[3]);
            } else {
                p.cubic_to(g.c[1], g.c[2], g.c[3]);
            }
        }
        if s.closed {
            p.close();
        }
    }
    p
}

/// The segments of `subs` that start or end at `at` (within `tol`), each as the edge leaving
/// `at` (reversed when it ends there).
pub fn edges_at(subs: &[Sub], at: Vec2, tol: f32) -> Vec<Seg> {
    let mut out = Vec::new();
    for s in subs {
        for g in &s.segs {
            if g.c[0].distance(at) <= tol {
                out.push(*g);
            }
            if g.c[3].distance(at) <= tol {
                out.push(g.reversed());
            }
        }
    }
    out
}

/// Whether two edges leaving one anchor are the same edge: their far ends and points along them
/// within `tol`.
pub fn same_edge(a: &Seg, b: &Seg, tol: f32) -> bool {
    a.c[3].distance(b.c[3]) <= tol && [0.25f32, 0.5, 0.75].iter().all(|&t| cubic_point(&a.c, t).distance(cubic_point(&b.c, t)) <= tol * 2.0)
}

/// The edges among `edges` that are distinct ([`same_edge`]), in order.
pub fn distinct_edges(edges: impl IntoIterator<Item = Seg>, tol: f32) -> Vec<Seg> {
    let mut out: Vec<Seg> = Vec::new();
    for e in edges {
        if !out.iter().any(|x| same_edge(x, &e, tol)) {
            out.push(e);
        }
    }
    out
}

/// The one edge in place of the edges `a` and `b` that leave an anchor: from `a`'s far end to
/// `b`'s. Two lines make a line; anything else the cubic that fits both best, keeping the
/// directions they leave their far ends in ([`Path::merge_at`]'s fit).
pub fn join(a: &Seg, b: &Seg) -> Option<Seg> {
    if a.line && b.line {
        return Some(Seg::line(a.c[3], b.c[3]));
    }
    let c = crate::edit::merge_cubics(a.reversed().c, b.c)?;
    Some(Seg { c, line: false })
}

/// The edges `a` and `b` that leave one anchor, made smooth there: their handles at the anchor on
/// one line, along the direction from `a`'s far end to `b`'s, each a third of its edge's chord
/// long ([`Path::make_smooth`]). `None` when the far ends coincide.
pub fn smooth_pair(a: &Seg, b: &Seg) -> Option<(Seg, Seg)> {
    let at = a.c[0];
    let dir = (b.c[3] - a.c[3]).normalized();
    if dir == Vec2::ZERO {
        return None;
    }
    let ha = at - dir * (at.distance(a.c[3]) / 3.0);
    let hb = at + dir * (at.distance(b.c[3]) / 3.0);
    let (na, nb) = (Seg { c: [at, ha, a.c[2], a.c[3]], line: false }, Seg { c: [at, hb, b.c[2], b.c[3]], line: false });
    (na.c.iter().chain(&nb.c).all(|p| p.is_finite())).then_some((na, nb))
}

/// `e` (leaving an anchor) with its handle at the anchor drawn in onto it: a corner there. `None`
/// when it has none to draw in (a line, or a handle already on the anchor).
pub fn corner(e: &Seg) -> Option<Seg> {
    (!e.line && e.c[1].distance(e.c[0]) > 1e-4).then_some(Seg { c: [e.c[0], e.c[0], e.c[2], e.c[3]], line: false })
}

/// Handles at one anchor lying on one line, on opposite sides, within this angle's sine (3°): a
/// smooth anchor.
const SMOOTH_SIN: f32 = 0.052;

/// Whether the handles `h` and `k` of the anchor `a` make it smooth: both off the anchor, on one
/// line through it, on opposite sides.
pub fn smooth_at(a: Vec2, h: Vec2, k: Vec2) -> bool {
    let (u, v) = (h - a, k - a);
    let (lu, lv) = (u.length(), v.length());
    lu > 1e-4 && lv > 1e-4 && u.dot(v) < 0.0 && (u.cross(v) / (lu * lv)).abs() <= SMOOTH_SIN
}

/// `k` turned about the anchor `a` so it lies opposite `h`, keeping its length (the other handle
/// of a smooth anchor while `h` is dragged). `k` itself when `h` is on the anchor.
pub fn turned_opposite(a: Vec2, h: Vec2, k: Vec2) -> Vec2 {
    let dir = (h - a).normalized();
    if dir == Vec2::ZERO {
        return k;
    }
    a - dir * a.distance(k)
}

/// Deletes the anchor at `at` (within `tol`) from `subs`, where the drawing has the distinct
/// edges `edges` leaving it.
///
/// * Two edges: every copy of either becomes `joined` (the edge from the first's far end to the
///   second's, see [`join`]). A run through the anchor takes it once in place of both; a line
///   ending there now runs along the whole new edge, so the drawing stays connected.
/// * One edge (the free end of lines): the segment there goes.
///
/// Returns whether anything changed. Subpaths may be left empty or flat: see [`tidy`].
pub fn delete_anchor(subs: &mut [Sub], at: Vec2, edges: &[Seg], joined: Option<&Seg>, tol: f32) -> bool {
    let which = |e: &Seg| edges.iter().position(|x| same_edge(x, e, tol));
    let toward = |from: usize| -> Option<Seg> {
        // The new edge leaving the far end of edge `from` (towards the other edge's far end).
        let j = joined?;
        Some(if from == 0 { *j } else { j.reversed() })
    };
    let mut changed = false;
    for s in subs.iter_mut() {
        // Each pass takes one vertex at `at`; bounded by the segments there are.
        for _ in 0..=s.segs.len() {
            let Some(step) = vertex_at(s, at, tol) else { break };
            let done = match step {
                Vertex::Through(v) => {
                    // in: segs[v - 1] arrives, out: segs[v] leaves (v ≥ 1 after rotation).
                    let (Some(inn), Some(out)) = (s.segs.get(v - 1).copied(), s.segs.get(v).copied()) else { break };
                    match (which(&inn.reversed()), which(&out), edges.len()) {
                        (Some(a), Some(b), 2) if a != b => match toward(a) {
                            Some(n) => {
                                s.segs.splice(v - 1..=v, [n]);
                                true
                            }
                            None => false,
                        },
                        _ => false,
                    }
                }
                Vertex::Start => match (s.segs.first().and_then(which), edges.len()) {
                    // The new first segment ends where the old one did.
                    (Some(a), 2) => match (toward(1 - a), s.segs.first_mut()) {
                        (Some(n), Some(first)) => {
                            *first = n;
                            true
                        }
                        _ => false,
                    },
                    (Some(_), 1) => {
                        s.segs.remove(0);
                        true
                    }
                    _ => false,
                },
                Vertex::End => {
                    match (s.segs.last().and_then(|g| which(&g.reversed())), edges.len()) {
                        (Some(a), 2) => match (toward(a), s.segs.last_mut()) {
                            (Some(n), Some(last)) => {
                                *last = n;
                                true
                            }
                            _ => false,
                        },
                        (Some(_), 1) => {
                            s.segs.pop();
                            true
                        }
                        _ => false,
                    }
                }
            };
            if !done {
                break;
            }
            changed = true;
        }
    }
    changed
}

/// Where a subpath has a vertex at a point.
enum Vertex {
    /// Between segments `v - 1` and `v` (a closed subpath is turned so `v ≥ 1`).
    Through(usize),
    /// An open subpath's start or end.
    Start,
    End,
}

fn vertex_at(s: &mut Sub, at: Vec2, tol: f32) -> Option<Vertex> {
    let n = s.segs.len();
    if n == 0 {
        return None;
    }
    if s.closed {
        let v = s.segs.iter().position(|g| g.c[0].distance(at) <= tol)?;
        // Turn the loop so the vertex is not at the seam.
        let turn = if v == 0 { n - 1 } else { 0 };
        s.segs.rotate_left(turn);
        return Some(Vertex::Through(if v == 0 { 1 } else { v }));
    }
    if s.segs.first().is_some_and(|g| g.c[0].distance(at) <= tol) {
        return Some(Vertex::Start);
    }
    if s.segs.last().is_some_and(|g| g.c[3].distance(at) <= tol) {
        return Some(Vertex::End);
    }
    (1..n).find(|&v| s.segs.get(v).is_some_and(|g| g.c[0].distance(at) <= tol)).map(Vertex::Through)
}

/// Drops the subpaths an edit left with nothing to show: no segments, no length, or (when
/// `filled`) a closed outline with no area.
pub fn tidy(subs: &mut Vec<Sub>, filled: bool) {
    subs.retain(|s| {
        let pts: Vec<Vec2> = s.segs.iter().flat_map(|g| (0..8).map(move |i| cubic_point(&g.c, i as f32 / 8.0))).chain(s.segs.last().map(|g| g.c[3])).collect();
        let length: f32 = pts.windows(2).map(|w| w.first().zip(w.get(1)).map_or(0.0, |(a, b)| a.distance(*b))).sum();
        if s.segs.is_empty() || length <= 1e-3 {
            return false;
        }
        if filled && s.closed {
            let area: f32 = pts.windows(2).map(|w| w.first().zip(w.get(1)).map_or(0.0, |(a, b)| a.cross(*b))).sum::<f32>() * 0.5;
            return area.abs() > 1e-3;
        }
        true
    });
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

    fn v(x: f32, y: f32) -> Vec2 {
        Vec2::new(x, y)
    }

    #[test]
    fn subpaths_round_trip() {
        let mut p = Path::rect(0.0, 0.0, 10.0, 10.0);
        p.move_to(v(20.0, 0.0)).cubic_to(v(20.0, 5.0), v(25.0, 10.0), v(30.0, 10.0));
        let subs = subpaths(&p);
        assert_eq!(subs.len(), 2);
        assert!(subs[0].closed && subs[0].segs.len() == 4 && subs[0].segs.iter().all(|g| g.line), "the closing edge is written out: {:?}", subs[0]);
        assert!(!subs[1].closed && !subs[1].segs[0].line);
        let back = subpaths(&from_subpaths(&subs));
        assert_eq!(back, subs);
    }

    /// A raw rectangle as merge drawing stores it: the fill and four sides, each its own path.
    fn outlined_rect() -> Vec<Vec<Sub>> {
        let (a, b, c, d) = (v(0.0, 0.0), v(100.0, 0.0), v(100.0, 80.0), v(0.0, 80.0));
        // The fill starts at the corner deleted below, to cover the seam.
        let fill = vec![Sub { segs: vec![Seg::line(a, b), Seg::line(b, c), Seg::line(c, d), Seg::line(d, a)], closed: true }];
        let side = |p, q| vec![Sub { segs: vec![Seg::line(p, q)], closed: false }];
        vec![fill, side(a, b), side(b, c), side(c, d), side(d, a)]
    }

    fn delete(shapes: &mut [Vec<Sub>], at: Vec2) -> usize {
        let edges = distinct_edges(shapes.iter().flat_map(|s| edges_at(s, at, 0.2)), 0.2);
        let joined = if edges.len() == 2 { join(&edges[0], &edges[1]) } else { None };
        shapes.iter_mut().map(|s| delete_anchor(s, at, &edges, joined.as_ref(), 0.2)).filter(|c| *c).count()
    }

    #[test]
    fn deleting_a_corner_keeps_the_drawing_connected() {
        let mut shapes = outlined_rect();
        let edges = distinct_edges(shapes.iter().flat_map(|s| edges_at(s, v(0.0, 0.0), 0.2)), 0.2);
        assert_eq!(edges.len(), 2, "two edges meet at a corner, each in the fill and a side");
        assert_eq!(delete(&mut shapes, v(0.0, 0.0)), 3, "the fill and both sides there");
        // The fill is a triangle now, cut along the diagonal from (0, 80) to (100, 0).
        let fill = &shapes[0][0];
        assert!(fill.closed && fill.segs.len() == 3 && fill.segs.iter().all(|g| g.line), "{fill:?}");
        assert!(fill.segs.iter().all(|g| g.c[0].distance(v(0.0, 0.0)) > 1.0));
        // Both sides that ended there now run along the diagonal: every anchor is shared.
        let diag = Seg::line(v(0.0, 80.0), v(100.0, 0.0));
        for s in [&shapes[1], &shapes[4]] {
            let g = s[0].segs[0];
            assert!(same_edge(&g, &diag, 0.01) || same_edge(&g.reversed(), &diag, 0.01), "{g:?}");
        }
        let anchors: Vec<Vec2> = shapes.iter().flatten().flat_map(|s| s.segs.iter().flat_map(|g| [g.c[0], g.c[3]])).collect();
        for p in &anchors {
            assert!(anchors.iter().filter(|q| q.distance(*p) < 1e-3).count() >= 2, "{p:?} is on one shape only");
        }
    }

    #[test]
    fn deleting_a_point_that_splitting_added_gives_the_curve_back() {
        let arc = Seg { c: [v(0.0, 0.0), v(0.0, -60.0), v(100.0, -60.0), v(100.0, 0.0)], line: false };
        let (a, b) = (sub(&arc.c, 0.0, 0.35), sub(&arc.c, 0.35, 1.0));
        let at = a[3];
        // A fill under the arc and the arc's own stroke, both with the point.
        let fill = vec![Sub { segs: vec![Seg { c: a, line: false }, Seg { c: b, line: false }, Seg::line(v(100.0, 0.0), v(0.0, 0.0))], closed: true }];
        let stroke = vec![Sub { segs: vec![Seg { c: b, line: false }.reversed(), Seg { c: a, line: false }.reversed()], closed: false }];
        let mut shapes = vec![fill, stroke];
        assert_eq!(delete(&mut shapes, at), 2);
        let (f, s) = (shapes[0][0].segs[0], shapes[1][0].segs[0]);
        assert_eq!((shapes[0][0].segs.len(), shapes[1][0].segs.len()), (2, 1));
        for t in [0.1f32, 0.3, 0.5, 0.7, 0.9] {
            assert!(nearest(&f.c, cubic_point(&arc.c, t)).1 < 0.05, "fill off the old curve at {t}");
        }
        assert!(same_edge(&f, &s.reversed(), 1e-3), "the fill and the stroke keep one edge");
    }

    #[test]
    fn deleting_a_free_end_shortens_the_line_and_its_last_point_removes_it() {
        let mut shapes = vec![vec![Sub { segs: vec![Seg::line(v(0.0, 0.0), v(50.0, 0.0)), Seg::line(v(50.0, 0.0), v(50.0, 50.0))], closed: false }]];
        assert_eq!(delete(&mut shapes, v(50.0, 50.0)), 1);
        assert_eq!(shapes[0][0].segs.len(), 1);
        assert_eq!(delete(&mut shapes, v(50.0, 0.0)), 1);
        tidy(&mut shapes[0], false);
        assert!(shapes[0].is_empty());
        // A triangle's fill losing a corner has no area left.
        let tri = |a, b, c| vec![Sub { segs: vec![Seg::line(a, b), Seg::line(b, c), Seg::line(c, a)], closed: true }];
        let mut t = vec![tri(v(0.0, 0.0), v(10.0, 0.0), v(5.0, 9.0))];
        assert_eq!(delete(&mut t, v(5.0, 9.0)), 1);
        tidy(&mut t[0], true);
        assert!(t[0].is_empty(), "{:?}", t[0]);
    }

    #[test]
    fn smooth_and_corner_handles() {
        let (a, b) = (Seg::line(v(0.0, 0.0), v(-30.0, 10.0)), Seg::line(v(0.0, 0.0), v(40.0, 10.0)));
        let Some((sa, sb)) = smooth_pair(&a, &b) else { panic!("no pair") };
        assert!(smooth_at(v(0.0, 0.0), sa.c[1], sb.c[1]), "{sa:?} {sb:?}");
        assert!((sa.c[1].distance(v(0.0, 0.0)) - a.c[3].length() / 3.0).abs() < 1e-3);
        assert_eq!((sa.c[3], sb.c[3]), (a.c[3], b.c[3]), "far ends stay");
        let Some(c) = corner(&sa) else { panic!("a curve has a handle") };
        assert_eq!(c.c[1], c.c[0]);
        assert!(corner(&a).is_none() && corner(&c).is_none(), "nothing to draw in");
        // Dragging one handle of a smooth anchor turns the other with it, keeping its length.
        let k = turned_opposite(v(0.0, 0.0), v(0.0, 10.0), v(20.0, 0.0));
        assert!(k.distance(v(0.0, -20.0)) < 1e-4, "{k:?}");
        assert!(!smooth_at(v(0.0, 0.0), v(10.0, 0.0), v(0.0, 10.0)), "a corner");
        assert!(!smooth_at(v(0.0, 0.0), v(10.0, 0.0), v(20.0, 0.1)), "same side");
    }

    #[test]
    fn nearest_finds_the_closest_point_on_a_curve() {
        let c = [Vec2::new(0.0, 0.0), Vec2::new(0.0, 50.0), Vec2::new(100.0, 50.0), Vec2::new(100.0, 0.0)];
        let (t, d) = nearest(&c, Vec2::new(50.0, 60.0));
        assert!((t - 0.5).abs() < 1e-3 && (d - 22.5).abs() < 1e-2, "{t} {d}");
    }
}
