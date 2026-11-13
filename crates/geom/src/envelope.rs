//! Envelope (ADR 0080): bending outlines by a bicubic Bézier patch, as Animate's Modify ›
//! Transform › Envelope does.
//!
//! The patch has 4 × 4 control points over a rest rectangle. A point at `(u, v)` of the
//! rectangle (0 to 1 across and down) goes to `Σ Bᵢ(u) Bⱼ(v) Pᵢⱼ`, the Bernstein polynomials of
//! degree 3. At rest the control points are evenly spaced (a third of the way apart), and the
//! map is exactly the identity (Bernstein polynomials reproduce linear functions).
//!
//! The image of a cubic under the patch is a curve of degree 9, so it is approximated: each
//! segment is cut in halves until the cubic through its mapped control points is within the
//! tolerance of the true image at its quarter points. A straight segment that stays straight (any
//! affine part of the patch) stays one line.

use crate::{Path, PathCmd, Rect, Vec2};

/// Most times a segment is halved.
const MAX_DEPTH: u32 = 10;
/// Most commands a bent path may have; beyond it [`warp`] declines.
pub const MAX_COMMANDS: usize = 200_000;

/// The patch's control points, row by row from the top-left (`points[4 * row + column]`).
pub type Mesh = [Vec2; 16];

/// The mesh that leaves everything where it is: control points evenly over `rest`.
pub fn rest_mesh(rest: &Rect) -> Mesh {
    let mut m = [Vec2::ZERO; 16];
    for (k, p) in m.iter_mut().enumerate() {
        let (i, j) = ((k % 4) as f32, (k / 4) as f32);
        *p = Vec2::new(rest.min.x + rest.width() * i / 3.0, rest.min.y + rest.height() * j / 3.0);
    }
    m
}

/// The indices of the boundary handles, clockwise from the top-left corner: corners and the two
/// tangent points of each side (the inner four follow them, [`coons_interior`]).
pub const BOUNDARY: [usize; 12] = [0, 1, 2, 3, 7, 11, 15, 14, 13, 12, 8, 4];
/// The corners, clockwise from the top-left.
pub const CORNERS: [usize; 4] = [0, 3, 15, 12];

/// Sets the four inner control points from the boundary, as a bicubic Coons patch does: the
/// sides bend the inside smoothly, and a rest boundary gives the rest mesh.
pub fn coons_interior(m: &mut Mesh) {
    let p = |r: usize, c: usize| m[4 * r + c];
    let blend = |a: Vec2, b: Vec2, c: Vec2, d: Vec2, e: Vec2, f: Vec2, g: Vec2, h: Vec2| (a * -4.0 + (b + c) * 6.0 - (d + e) * 2.0 + (f + g) * 3.0 - h) * (1.0 / 9.0);
    let p11 = blend(p(0, 0), p(0, 1), p(1, 0), p(0, 3), p(3, 0), p(3, 1), p(1, 3), p(3, 3));
    let p12 = blend(p(0, 3), p(0, 2), p(1, 3), p(0, 0), p(3, 3), p(3, 2), p(1, 0), p(3, 0));
    let p21 = blend(p(3, 0), p(3, 1), p(2, 0), p(3, 3), p(0, 0), p(0, 1), p(2, 3), p(0, 3));
    let p22 = blend(p(3, 3), p(3, 2), p(2, 3), p(3, 0), p(0, 3), p(0, 2), p(2, 0), p(0, 0));
    m[5] = p11;
    m[6] = p12;
    m[9] = p21;
    m[10] = p22;
}

fn bernstein(t: f64) -> [f64; 4] {
    let u = 1.0 - t;
    [u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t]
}

/// Where the patch puts `p` (a point of the rest rectangle's plane; outside it, the patch's
/// polynomial carries on).
pub fn map(mesh: &Mesh, rest: &Rect, p: Vec2) -> Vec2 {
    let (w, h) = (f64::from(rest.width()), f64::from(rest.height()));
    if !(w > 1e-9 && h > 1e-9) {
        return p;
    }
    let u = (f64::from(p.x) - f64::from(rest.min.x)) / w;
    let v = (f64::from(p.y) - f64::from(rest.min.y)) / h;
    let (bu, bv) = (bernstein(u), bernstein(v));
    let (mut x, mut y) = (0.0f64, 0.0f64);
    for (k, q) in mesh.iter().enumerate() {
        let wgt = bu[k % 4] * bv[k / 4];
        x += wgt * f64::from(q.x);
        y += wgt * f64::from(q.y);
    }
    Vec2::new(x as f32, y as f32)
}

fn cubic_at(c: &[Vec2; 4], t: f32) -> Vec2 {
    let u = 1.0 - t;
    c[0] * (u * u * u) + c[1] * (3.0 * u * u * t) + c[2] * (3.0 * u * t * t) + c[3] * (t * t * t)
}

fn split(c: &[Vec2; 4]) -> ([Vec2; 4], [Vec2; 4]) {
    let (ab, bc, cd) = (c[0].lerp(c[1], 0.5), c[1].lerp(c[2], 0.5), c[2].lerp(c[3], 0.5));
    let (abc, bcd) = (ab.lerp(bc, 0.5), bc.lerp(cd, 0.5));
    let m = abc.lerp(bcd, 0.5);
    ([c[0], ab, abc, m], [m, bcd, cd, c[3]])
}

/// Whether every mesh point is finite and the rest rectangle has an area.
pub fn valid(mesh: &Mesh, rest: &Rect) -> bool {
    mesh.iter().all(|p| p.x.is_finite() && p.y.is_finite()) && rest.width() > 1e-6 && rest.height() > 1e-6 && rest.min.x.is_finite() && rest.min.y.is_finite() && rest.max.x.is_finite() && rest.max.y.is_finite()
}

/// `path` bent by the patch (both in the same space), within `tol`. `None` when the bent path
/// would be too large ([`MAX_COMMANDS`]) or the mesh isn't valid.
pub fn warp(path: &Path, mesh: &Mesh, rest: &Rect, tol: f32) -> Option<Path> {
    if !valid(mesh, rest) {
        return None;
    }
    let tol = if tol.is_finite() { tol.max(1e-4) } else { 0.05 };
    let mut out = Path::new();
    let (mut start, mut cur) = (Vec2::ZERO, Vec2::ZERO);
    let line = |a: Vec2, b: Vec2| [a, a.lerp(b, 1.0 / 3.0), a.lerp(b, 2.0 / 3.0), b];
    for c in &path.cmds {
        match *c {
            PathCmd::MoveTo(p) => {
                out.move_to(map(mesh, rest, p));
                start = p;
                cur = p;
            }
            PathCmd::LineTo(p) => {
                segment(&mut out, &line(cur, p), true, mesh, rest, tol, 0);
                cur = p;
            }
            PathCmd::QuadTo(q, p) => {
                segment(&mut out, &[cur, cur.lerp(q, 2.0 / 3.0), p.lerp(q, 2.0 / 3.0), p], false, mesh, rest, tol, 0);
                cur = p;
            }
            PathCmd::CubicTo(a, b, p) => {
                segment(&mut out, &[cur, a, b, p], false, mesh, rest, tol, 0);
                cur = p;
            }
            PathCmd::Close => {
                // The closing edge bends too: when it no longer is straight, draw it, then
                // close on the start.
                if cur != start && !stays_straight(&line(cur, start), mesh, rest, tol) {
                    segment(&mut out, &line(cur, start), true, mesh, rest, tol, 0);
                }
                out.close();
                cur = start;
            }
        }
        if out.cmds.len() > MAX_COMMANDS {
            return None;
        }
    }
    Some(out)
}

/// Whether the line `c` (as a cubic) stays straight: its image's quarter points are on the
/// chord between its mapped ends, evenly.
fn stays_straight(c: &[Vec2; 4], mesh: &Mesh, rest: &Rect, tol: f32) -> bool {
    let (a, b) = (map(mesh, rest, c[0]), map(mesh, rest, c[3]));
    [0.25f32, 0.5, 0.75].iter().all(|&t| map(mesh, rest, cubic_at(c, t)).distance(a.lerp(b, t)) <= tol)
}

/// One segment (as a cubic, `straight` when it is a line), bent and appended.
fn segment(out: &mut Path, c: &[Vec2; 4], straight: bool, mesh: &Mesh, rest: &Rect, tol: f32, depth: u32) {
    let m = [map(mesh, rest, c[0]), map(mesh, rest, c[1]), map(mesh, rest, c[2]), map(mesh, rest, c[3])];
    let err = [0.25f32, 0.5, 0.75].iter().map(|&t| map(mesh, rest, cubic_at(c, t)).distance(cubic_at(&m, t))).fold(0.0f32, f32::max);
    if straight && stays_straight(c, mesh, rest, tol) {
        out.line_to(m[3]);
        return;
    }
    if err <= tol || depth >= MAX_DEPTH || out.cmds.len() > MAX_COMMANDS {
        out.cubic_to(m[1], m[2], m[3]);
        return;
    }
    let (a, b) = split(c);
    segment(out, &a, false, mesh, rest, tol, depth + 1);
    segment(out, &b, false, mesh, rest, tol, depth + 1);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Affine;

    fn rest() -> Rect {
        Rect::new(10.0, 20.0, 110.0, 70.0)
    }

    /// At rest the map is the identity, and a bent path is the path itself, command for command.
    #[test]
    fn identity_at_rest() {
        let r = rest();
        let m = rest_mesh(&r);
        for p in [Vec2::new(10.0, 20.0), Vec2::new(60.0, 45.0), Vec2::new(110.0, 70.0), Vec2::new(-30.0, 200.0)] {
            assert!(map(&m, &r, p).distance(p) < 1e-3, "{p:?}");
        }
        let mut path = Path::rounded_rect(10.0, 20.0, 100.0, 50.0, 12.0);
        path.cmds.extend(Path::star(60.0, 45.0, 20.0, 8.0, 5).cmds);
        let bent = warp(&path, &m, &r, 0.01).expect("bends");
        assert_eq!(bent.cmds.len(), path.cmds.len(), "nothing is subdivided at rest");
        for (a, b) in bent.cmds.iter().zip(&path.cmds) {
            assert_eq!(std::mem::discriminant(a), std::mem::discriminant(b), "lines stay lines");
        }
        let (fa, fb) = (bent.flatten(&Affine::IDENTITY, 0.01), path.flatten(&Affine::IDENTITY, 0.01));
        for (pa, pb) in fa.iter().zip(&fb) {
            for (x, y) in pa.points.iter().zip(&pb.points) {
                assert!(x.distance(*y) < 1e-3);
            }
        }
    }

    /// A rest boundary gives the rest interior; moving every point by the same affine map
    /// moves the path by it, lines staying lines.
    #[test]
    fn coons_and_affine_meshes() {
        let r = rest();
        let mut m = rest_mesh(&r);
        let want = m;
        m[5] = Vec2::ZERO;
        m[10] = Vec2::new(1e3, 1e3);
        coons_interior(&mut m);
        for (a, b) in m.iter().zip(&want) {
            assert!(a.distance(*b) < 1e-3);
        }
        let xf = Affine::translate(5.0, -3.0).mul(&Affine::rotate(0.4)).mul(&Affine::scale(1.5, 0.75));
        let moved: Mesh = want.map(|p| xf.apply(p));
        let path = Path::rect(20.0, 30.0, 40.0, 20.0);
        let bent = warp(&path, &moved, &r, 0.01).expect("bends");
        assert!(bent.cmds.iter().all(|c| !matches!(c, PathCmd::CubicTo(..))), "an affine envelope keeps lines straight");
        let pts = bent.flatten(&Affine::IDENTITY, 0.01);
        for (p, q) in pts[0].points.iter().zip(path.flatten(&xf, 0.01)[0].points.iter()) {
            assert!(p.distance(*q) < 1e-2, "{p:?} vs {q:?}");
        }
    }

    /// A bulged top side bends a straight top edge into a curve through the bulge, within the
    /// tolerance of the exact image everywhere.
    #[test]
    fn bending_is_within_tolerance() {
        let r = rest();
        let mut m = rest_mesh(&r);
        m[1].y -= 30.0;
        m[2].y -= 30.0;
        coons_interior(&mut m);
        let path = Path::rect(10.0, 20.0, 100.0, 50.0);
        let tol = 0.05;
        let bent = warp(&path, &m, &r, tol).expect("bends");
        assert!(bent.cmds.iter().any(|c| matches!(c, PathCmd::CubicTo(..))));
        // The top edge's middle rose by ¾ of the handles' lift (B₁ + B₂ at ½).
        let top = map(&m, &r, Vec2::new(60.0, 20.0));
        assert!((top.y - (20.0 - 22.5)).abs() < 1e-3, "{top:?}");
        // Every point of the exact image of the top edge is near the bent outline.
        let poly = &bent.flatten(&Affine::IDENTITY, 0.005)[0].points;
        for i in 0..=50 {
            let exact = map(&m, &r, Vec2::new(10.0 + 2.0 * i as f32, 20.0));
            let d = poly.windows(2).map(|w| crate::segment_distance(exact, w[0], w[1])).fold(f32::MAX, f32::min);
            assert!(d < tol * 2.0, "{i}: {d}");
        }
    }

    #[test]
    fn invalid_meshes_decline() {
        let r = rest();
        let mut m = rest_mesh(&r);
        m[3].x = f32::NAN;
        assert!(warp(&Path::rect(0.0, 0.0, 1.0, 1.0), &m, &r, 0.1).is_none());
        let flat = Rect::new(0.0, 0.0, 10.0, 0.0);
        assert!(warp(&Path::rect(0.0, 0.0, 1.0, 1.0), &rest_mesh(&flat), &flat, 0.1).is_none());
    }
}
