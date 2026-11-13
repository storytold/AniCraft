//! Exact clipping of filled paths by a convex polygon.
//!
//! Sutherland and Hodgman's re-entrant polygon clipping (CACM 17(1), 1974), extended to Bézier
//! segments: each subpath is clipped against the polygon one edge's half-plane at a time. A
//! segment is split where it crosses the edge's line (its signed distance from the line is a
//! polynomial in `t` of the segment's degree, whose roots are bracketed between its critical
//! points and bisected), the pieces inside are kept as exact sub-curves (de Casteljau), and the
//! pieces outside become the straight run along the line between where they leave and come
//! back. For every point strictly inside the polygon the result winds as often as the original,
//! so the filled area is exactly the intersection, with either fill rule. Meant for fills: the
//! runs along the polygon are new edges, so a stroke of the result would differ.

use crate::{Path, PathCmd, Vec2};

/// The corners of `path` when it is one convex polygon of straight edges (three or more
/// corners that are not all on a line), in the path's order; `None` otherwise.
pub fn convex_polygon(path: &Path) -> Option<Vec<Vec2>> {
    let mut pts: Vec<Vec2> = Vec::new();
    let mut closed = false;
    for (i, c) in path.cmds.iter().enumerate() {
        match *c {
            PathCmd::MoveTo(p) if i == 0 => pts.push(p),
            PathCmd::LineTo(p) if !closed && i > 0 => pts.push(p),
            PathCmd::Close if i > 0 => closed = true,
            _ => return None,
        }
    }
    if !pts.iter().all(|p| p.is_finite()) {
        return None;
    }
    pts.dedup_by(|a, b| a.distance(*b) <= 1e-6);
    while pts.len() > 1 && pts.first().zip(pts.last()).is_some_and(|(a, b)| a.distance(*b) <= 1e-6) {
        pts.pop();
    }
    // Drop corners on a straight run, then check that every turn goes the same way and the
    // turns add up to one full turn (a pentagram turns one way too, twice around).
    let scale = pts.iter().fold(0.0f32, |m, p| m.max(p.x.abs()).max(p.y.abs())).max(1.0);
    let mut i = 0;
    while pts.len() >= 3 && i < pts.len() {
        let n = pts.len();
        let (a, b, c) = (pts[(i + n - 1) % n], pts[i], pts[(i + 1) % n]);
        if (b - a).cross(c - b).abs() <= 1e-7 * scale * scale {
            pts.remove(i);
        } else {
            i += 1;
        }
    }
    let n = pts.len();
    if n < 3 {
        return None;
    }
    let mut sign = 0.0f32;
    let mut turning = 0.0f32;
    for i in 0..n {
        let (a, b, c) = (pts[i], pts[(i + 1) % n], pts[(i + 2) % n]);
        let (u, v) = (b - a, c - b);
        let cr = u.cross(v);
        if sign == 0.0 {
            sign = cr.signum();
        } else if cr.signum() != sign {
            return None;
        }
        turning += cr.atan2(u.dot(v));
    }
    ((turning.abs() - std::f32::consts::TAU).abs() < 1e-2).then_some(pts)
}

#[derive(Clone, Copy, Debug)]
enum Seg {
    Line(Vec2),
    Quad(Vec2, Vec2),
    Cubic(Vec2, Vec2, Vec2),
}

impl Seg {
    fn end(&self) -> Vec2 {
        match *self {
            Seg::Line(p) | Seg::Quad(_, p) | Seg::Cubic(_, _, p) => p,
        }
    }
}

struct Contour {
    start: Vec2,
    segs: Vec<Seg>,
}

/// The subpaths of `path`, each closed (as a fill closes it).
fn contours(path: &Path) -> Vec<Contour> {
    let mut out: Vec<Contour> = Vec::new();
    let mut cur: Option<Contour> = None;
    let mut last = Vec2::ZERO;
    let finish = |c: Option<Contour>, out: &mut Vec<Contour>| {
        if let Some(mut c) = c
            && !c.segs.is_empty()
        {
            if c.segs.last().is_some_and(|s| s.end() != c.start) {
                c.segs.push(Seg::Line(c.start));
            }
            out.push(c);
        }
    };
    for cmd in &path.cmds {
        let seg = match *cmd {
            PathCmd::MoveTo(p) => {
                finish(cur.take(), &mut out);
                cur = Some(Contour { start: p, segs: Vec::new() });
                last = p;
                continue;
            }
            PathCmd::Close => {
                if let Some(c) = &cur {
                    last = c.start;
                }
                finish(cur.take(), &mut out);
                continue;
            }
            PathCmd::LineTo(p) => Seg::Line(p),
            PathCmd::QuadTo(a, p) => Seg::Quad(a, p),
            PathCmd::CubicTo(a, b, p) => Seg::Cubic(a, b, p),
        };
        let c = cur.get_or_insert(Contour { start: last, segs: Vec::new() });
        c.segs.push(seg);
        last = seg.end();
    }
    finish(cur, &mut out);
    out
}

/// A Bernstein polynomial's value at `t` (de Casteljau on its coefficients).
fn bernstein(c: &[f32], t: f32) -> f32 {
    let mut v: [f32; 4] = [0.0; 4];
    let n = c.len().min(4);
    v[..n].copy_from_slice(&c[..n]);
    for k in (1..n).rev() {
        for i in 0..k {
            v[i] = v[i] + (v[i + 1] - v[i]) * t;
        }
    }
    v[0]
}

/// Where a Bernstein polynomial of degree 1 to 3 changes sign in (0, 1), sorted. Its critical
/// points split [0, 1] into monotone stretches, and each stretch whose ends differ in sign holds
/// one root, found by bisection.
fn sign_changes(c: &[f32]) -> Vec<f32> {
    let mut breaks: Vec<f32> = Vec::with_capacity(4);
    match *c {
        [_, _, _, _] => {
            // The derivative's Bernstein coefficients, as a power-basis quadratic.
            let (e0, e1, e2) = (c[1] - c[0], c[2] - c[1], c[3] - c[2]);
            let (a, b, k) = (e0 - 2.0 * e1 + e2, 2.0 * (e1 - e0), e0);
            if a.abs() > 1e-12 {
                let disc = b * b - 4.0 * a * k;
                if disc >= 0.0 {
                    let q = -0.5 * (b + b.signum() * disc.sqrt());
                    breaks.push(q / a);
                    if q != 0.0 {
                        breaks.push(k / q);
                    }
                }
            } else if b.abs() > 1e-12 {
                breaks.push(-k / b);
            }
        }
        [_, _, _] => {
            let (e0, e1) = (c[1] - c[0], c[2] - c[1]);
            if (e0 - e1).abs() > 1e-12 {
                breaks.push(e0 / (e0 - e1));
            }
        }
        _ => {}
    }
    breaks.retain(|t| t.is_finite() && *t > 0.0 && *t < 1.0);
    breaks.push(0.0);
    breaks.push(1.0);
    breaks.sort_by(f32::total_cmp);
    let mut roots = Vec::new();
    for w in breaks.windows(2) {
        let (mut lo, mut hi) = (w[0], w[1]);
        let (flo, fhi) = (bernstein(c, lo), bernstein(c, hi));
        if flo == 0.0 && lo > 0.0 {
            roots.push(lo);
        }
        if flo * fhi >= 0.0 {
            continue;
        }
        for _ in 0..48 {
            let mid = 0.5 * (lo + hi);
            if (bernstein(c, mid) < 0.0) == (flo < 0.0) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        roots.push(0.5 * (lo + hi));
    }
    roots.dedup_by(|a, b| (*a - *b).abs() < 1e-7);
    roots
}

fn split_quad(p: [Vec2; 3], t: f32) -> ([Vec2; 3], [Vec2; 3]) {
    let a = p[0].lerp(p[1], t);
    let b = p[1].lerp(p[2], t);
    let m = a.lerp(b, t);
    ([p[0], a, m], [m, b, p[2]])
}

fn split_cubic(p: [Vec2; 4], t: f32) -> ([Vec2; 4], [Vec2; 4]) {
    let (a, b, c) = (p[0].lerp(p[1], t), p[1].lerp(p[2], t), p[2].lerp(p[3], t));
    let (d, e) = (a.lerp(b, t), b.lerp(c, t));
    let m = d.lerp(e, t);
    ([p[0], a, d, m], [m, e, c, p[3]])
}

/// The piece of a segment (starting at `p0`) between `t0` and `t1`, without its start point.
fn piece(p0: Vec2, seg: Seg, t0: f32, t1: f32) -> Seg {
    // Split at t1, then the first part at t0 / t1.
    let u = if t1 > 0.0 { (t0 / t1).clamp(0.0, 1.0) } else { 0.0 };
    match seg {
        Seg::Line(p) => Seg::Line(p0.lerp(p, t1)),
        Seg::Quad(a, p) => {
            let (left, _) = split_quad([p0, a, p], t1);
            let (_, r) = split_quad(left, u);
            Seg::Quad(r[1], r[2])
        }
        Seg::Cubic(a, b, p) => {
            let (left, _) = split_cubic([p0, a, b, p], t1);
            let (_, r) = split_cubic(left, u);
            Seg::Cubic(r[1], r[2], r[3])
        }
    }
}

fn point_at(p0: Vec2, seg: Seg, t: f32) -> Vec2 {
    match seg {
        Seg::Line(p) => p0.lerp(p, t),
        Seg::Quad(a, p) => split_quad([p0, a, p], t).0[2],
        Seg::Cubic(a, b, p) => split_cubic([p0, a, b, p], t).0[3],
    }
}

/// `c` clipped to the half-plane where `dist ≥ 0`, with points outside projected onto its line.
fn clip_half(c: &Contour, dist: &dyn Fn(Vec2) -> f32, proj: &dyn Fn(Vec2) -> Vec2) -> Contour {
    let mut out = Contour { start: if dist(c.start) >= 0.0 { c.start } else { proj(c.start) }, segs: Vec::with_capacity(c.segs.len() + 4) };
    let mut p0 = c.start;
    for &seg in &c.segs {
        let ds: Vec<f32> = match seg {
            Seg::Line(p) => vec![dist(p0), dist(p)],
            Seg::Quad(a, p) => vec![dist(p0), dist(a), dist(p)],
            Seg::Cubic(a, b, p) => vec![dist(p0), dist(a), dist(b), dist(p)],
        };
        if ds.iter().all(|d| *d >= 0.0) {
            out.segs.push(seg);
        } else if ds.iter().all(|d| *d <= 0.0) {
            out.segs.push(Seg::Line(proj(seg.end())));
        } else {
            let mut ts = vec![0.0f32];
            ts.extend(sign_changes(&ds));
            ts.push(1.0);
            for w in ts.windows(2) {
                let (t0, t1) = (w[0], w[1]);
                if t1 - t0 <= 1e-9 {
                    continue;
                }
                if bernstein(&ds, 0.5 * (t0 + t1)) >= 0.0 {
                    out.segs.push(piece(p0, seg, t0, t1));
                } else {
                    out.segs.push(Seg::Line(proj(point_at(p0, seg, t1))));
                }
            }
        }
        p0 = seg.end();
    }
    out
}

/// The filled area of `path` inside the convex `polygon` (any orientation), exactly: curves
/// stay curves. Every subpath of the result is closed. An empty or degenerate polygon gives an
/// empty path.
pub fn clip_convex(path: &Path, polygon: &[Vec2]) -> Path {
    let n = polygon.len();
    let area2: f32 = (0..n).map(|i| polygon[i].cross(polygon[(i + 1) % n])).sum();
    if n < 3 || !area2.is_finite() || area2.abs() <= 1e-12 {
        return Path::new();
    }
    let s = area2.signum();
    let mut cs = contours(path);
    for i in 0..n {
        let (a, b) = (polygon[i], polygon[(i + 1) % n]);
        let e = b - a;
        let len = e.length();
        if len <= 1e-9 {
            continue;
        }
        let dir = e * (1.0 / len);
        // Signed distance, positive inside; projection onto the edge's line.
        let dist = move |p: Vec2| s * dir.cross(p - a);
        let proj = move |p: Vec2| a + dir * dir.dot(p - a);
        cs = cs.iter().map(|c| clip_half(c, &dist, &proj)).collect();
    }
    let mut out = Path::new();
    for c in cs {
        // A contour left lying along the polygon's edges encloses nothing.
        if c.segs.iter().all(|s| matches!(s, Seg::Line(_))) {
            let mut pts = vec![c.start];
            pts.extend(c.segs.iter().map(Seg::end));
            let m = pts.len();
            let area: f32 = (0..m).map(|i| pts[i].cross(pts[(i + 1) % m])).sum::<f32>() * 0.5;
            let r = pts.iter().fold(crate::Rect::EMPTY, |mut r, p| {
                r.include(*p);
                r
            });
            if area.abs() <= 1e-6 * (r.width() * r.height()).max(1e-12) {
                continue;
            }
        }
        out.cmds.push(PathCmd::MoveTo(c.start));
        for seg in c.segs {
            out.cmds.push(match seg {
                Seg::Line(p) => PathCmd::LineTo(p),
                Seg::Quad(a, p) => PathCmd::QuadTo(a, p),
                Seg::Cubic(a, b, p) => PathCmd::CubicTo(a, b, p),
            });
        }
        out.cmds.push(PathCmd::Close);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Affine, FillRule};

    fn square(x: f32, y: f32, w: f32) -> Vec<Vec2> {
        vec![Vec2::new(x, y), Vec2::new(x + w, y), Vec2::new(x + w, y + w), Vec2::new(x, y + w)]
    }

    /// Area by sampling a fine grid of points (winding rule `rule`).
    fn area(p: &Path, rule: FillRule) -> f32 {
        let b = p.bounds();
        let step = 0.25;
        let mut n = 0u32;
        let mut y = b.min.y + step * 0.5;
        while y < b.max.y {
            let mut x = b.min.x + step * 0.5;
            while x < b.max.x {
                if p.contains(Vec2::new(x, y), &Affine::IDENTITY, rule) {
                    n += 1;
                }
                x += step;
            }
            y += step;
        }
        n as f32 * step * step
    }

    #[test]
    fn convex_polygons_are_recognised() {
        assert_eq!(convex_polygon(&Path::rect(0.0, 0.0, 10.0, 5.0)).map(|p| p.len()), Some(4));
        let mut tri = Path::new();
        tri.move_to(Vec2::new(0.0, 0.0)).line_to(Vec2::new(5.0, 0.0)).line_to(Vec2::new(10.0, 0.0)).line_to(Vec2::new(0.0, 10.0));
        assert_eq!(convex_polygon(&tri).map(|p| p.len()), Some(3), "an open polygon closes; a straight-through corner goes");
        let mut bow = Path::new();
        bow.move_to(Vec2::new(0.0, 0.0)).line_to(Vec2::new(10.0, 10.0)).line_to(Vec2::new(10.0, 0.0)).line_to(Vec2::new(0.0, 10.0)).close();
        assert!(convex_polygon(&bow).is_none(), "self-crossing");
        assert!(convex_polygon(&Path::star(0.0, 0.0, 10.0, 4.0, 5)).is_none(), "concave");
        assert!(convex_polygon(&Path::ellipse(0.0, 0.0, 5.0, 5.0)).is_none(), "curved");
        let mut two = Path::rect(0.0, 0.0, 1.0, 1.0);
        two.cmds.extend(Path::rect(5.0, 5.0, 1.0, 1.0).cmds);
        assert!(convex_polygon(&two).is_none(), "two subpaths");
        assert!(convex_polygon(&Path::line(Vec2::ZERO, Vec2::new(1.0, 1.0))).is_none());
    }

    #[test]
    fn rectangles_clip_to_their_overlap() {
        let r = clip_convex(&Path::rect(0.0, 0.0, 10.0, 10.0), &square(5.0, 5.0, 10.0));
        assert_eq!(r.bounds(), crate::Rect::new(5.0, 5.0, 10.0, 10.0));
        assert!((area(&r, FillRule::NonZero) - 25.0).abs() < 0.01);
        assert!(clip_convex(&Path::rect(0.0, 0.0, 1.0, 1.0), &square(5.0, 5.0, 1.0)).is_empty(), "no overlap, no contour");
        // A shape around the whole polygon becomes the polygon.
        let all = clip_convex(&Path::rect(-50.0, -50.0, 100.0, 100.0), &square(0.0, 0.0, 4.0));
        assert!((area(&all, FillRule::NonZero) - 16.0).abs() < 0.01, "{all:?}");
        assert!(clip_convex(&Path::rect(0.0, 0.0, 1.0, 1.0), &[Vec2::ZERO, Vec2::new(1.0, 1.0)]).is_empty());
    }

    #[test]
    fn curves_stay_curves_and_areas_are_exact() {
        // A circle of radius 10 cut by the half-plane x ≥ 0 (a big square): half the disc.
        let circle = Path::ellipse(0.0, 0.0, 10.0, 10.0);
        let half = clip_convex(&circle, &square(0.0, -20.0, 40.0));
        assert!(half.cmds.iter().any(|c| matches!(c, PathCmd::CubicTo(..))), "the arc is still a curve");
        let full = area(&circle, FillRule::NonZero);
        assert!((area(&half, FillRule::NonZero) - full / 2.0).abs() < 1.0, "{} vs {}", area(&half, FillRule::NonZero), full / 2.0);
        // Every point of the result's curves lies inside the polygon.
        let b = half.bounds();
        assert!(b.min.x >= -1e-4 && b.max.x <= 10.0 + 1e-3, "{b:?}");
        // A quarter: the circle inside a square at its centre.
        let q = clip_convex(&circle, &square(0.0, 0.0, 30.0));
        assert!((area(&q, FillRule::NonZero) - full / 4.0).abs() < 1.0);
        // A wavy cubic crossing the line three times.
        let mut wave = Path::new();
        wave.move_to(Vec2::new(0.0, 0.0)).cubic_to(Vec2::new(3.0, 20.0), Vec2::new(6.0, -20.0), Vec2::new(10.0, 5.0)).line_to(Vec2::new(10.0, 10.0)).line_to(Vec2::new(0.0, 10.0)).close();
        let cut = clip_convex(&wave, &square(-5.0, 2.0, 20.0));
        let expect = {
            // Sample the original, keeping points with y ≥ 2 inside the square.
            let mut n = 0u32;
            let mut y = -20.0f32 + 0.125;
            while y < 20.0 {
                let mut x = -5.0f32 + 0.125;
                while x < 15.0 {
                    if y >= 2.0 && x <= 15.0 && wave.contains(Vec2::new(x, y), &Affine::IDENTITY, FillRule::NonZero) {
                        n += 1;
                    }
                    x += 0.25;
                }
                y += 0.25;
            }
            n as f32 * 0.0625
        };
        assert!((area(&cut, FillRule::NonZero) - expect).abs() < 1.5, "{} vs {expect}", area(&cut, FillRule::NonZero));
    }

    #[test]
    fn holes_and_fill_rules_survive() {
        // A ring (outer square, inner square the other way round) cut in half keeps its hole.
        let mut ring = Path::rect(0.0, 0.0, 20.0, 20.0);
        let mut inner = Path::new();
        inner.move_to(Vec2::new(5.0, 5.0)).line_to(Vec2::new(5.0, 15.0)).line_to(Vec2::new(15.0, 15.0)).line_to(Vec2::new(15.0, 5.0)).close();
        ring.cmds.extend(inner.cmds);
        let cut = clip_convex(&ring, &square(10.0, -10.0, 40.0));
        assert!((area(&cut, FillRule::NonZero) - (200.0 - 50.0)).abs() < 0.1);
        assert!(!cut.contains(Vec2::new(12.0, 10.0), &Affine::IDENTITY, FillRule::NonZero), "the hole stays a hole");
        // Even-odd: the same hole drawn the same way round.
        let mut eo = Path::rect(0.0, 0.0, 20.0, 20.0);
        eo.cmds.extend(Path::rect(5.0, 5.0, 10.0, 10.0).cmds);
        let cut = clip_convex(&eo, &square(10.0, -10.0, 40.0));
        assert!((area(&cut, FillRule::EvenOdd) - 150.0).abs() < 0.1);
        // A rotated (diamond) polygon, and a path that never closes itself.
        let diamond = [Vec2::new(10.0, 0.0), Vec2::new(20.0, 10.0), Vec2::new(10.0, 20.0), Vec2::new(0.0, 10.0)];
        let mut open = Path::new();
        open.move_to(Vec2::new(0.0, 0.0)).line_to(Vec2::new(20.0, 0.0)).line_to(Vec2::new(20.0, 20.0)).line_to(Vec2::new(0.0, 20.0));
        let cut = clip_convex(&open, &diamond);
        let pts: Vec<Vec2> = cut.cmds.iter().filter_map(|c| if let PathCmd::MoveTo(p) | PathCmd::LineTo(p) = c { Some(*p) } else { None }).collect();
        let shoelace: f32 = (0..pts.len()).map(|i| pts[i].cross(pts[(i + 1) % pts.len()])).sum::<f32>() * 0.5;
        assert!((shoelace.abs() - 200.0).abs() < 1e-3, "{shoelace}: {cut:?}");
    }
}
