//! Fitting cubic Béziers to freehand strokes (Pencil "Smooth" mode).
//!
//! 1. Consecutive duplicate points are dropped (a held mouse reports the same position).
//! 2. Corners are where the stroke direction, measured over `corner_window` of arc length on
//!    each side, turns by more than 60°; only the sharpest point of each turn counts.
//! 3. Each run between corners is fitted by least squares with fixed end tangents and Newton
//!    reparameterisation, splitting at the worst point until within `tolerance` (P. J. Schneider,
//!    "An Algorithm for Automatically Fitting Digitized Curves", Graphics Gems, 1990).
//!
//! Ported from PhotoCraft's mask tracer (`vector::trace`, same project family, MIT OR
//! Apache-2.0) and adapted for strokes: open paths, a caller-chosen corner window (so it can
//! follow the zoom), end tangents measured over that window instead of from one noisy
//! neighbour, and no indexing that input can push out of range. Maths runs in `f64`.

use crate::{Path, PathCmd, Vec2};

type P = (f64, f64);

fn sub(a: P, b: P) -> P {
    (a.0 - b.0, a.1 - b.1)
}
fn add(a: P, b: P) -> P {
    (a.0 + b.0, a.1 + b.1)
}
fn mul(a: P, k: f64) -> P {
    (a.0 * k, a.1 * k)
}
fn dot(a: P, b: P) -> f64 {
    a.0 * b.0 + a.1 * b.1
}
fn len(a: P) -> f64 {
    a.0.hypot(a.1)
}
fn norm(a: P) -> P {
    let l = len(a);
    if l > 1e-12 { (a.0 / l, a.1 / l) } else { (0.0, 0.0) }
}

/// A smooth path through a freehand stroke: cubic segments within `tolerance` of the points,
/// with corners kept sharp. Fewer than two distinct points give a path with at most a `MoveTo`.
pub fn fit_curve(points: &[Vec2], tolerance: f32, corner_window: f32, closed: bool) -> Path {
    let tol = if tolerance.is_finite() && tolerance > 0.0 { f64::from(tolerance) } else { 1.0 };
    let window = if corner_window.is_finite() && corner_window > 0.0 { f64::from(corner_window) } else { 3.0 };
    let mut lp: Vec<P> = Vec::with_capacity(points.len());
    for p in points.iter().filter(|p| p.is_finite()) {
        let q = (f64::from(p.x), f64::from(p.y));
        if lp.last().is_none_or(|&l| len(sub(q, l)) > 1e-4) {
            lp.push(q);
        }
    }
    if closed && lp.len() > 2 && lp.first().zip(lp.last()).is_some_and(|(a, b)| len(sub(*a, *b)) <= 1e-4) {
        lp.pop();
    }
    let mut path = Path::new();
    let v = |p: P| Vec2::new(p.0 as f32, p.1 as f32);
    let Some(&first) = lp.first() else { return path };
    path.move_to(v(first));
    if lp.len() < 3 {
        for &p in lp.iter().skip(1) {
            path.line_to(v(p));
        }
        return path;
    }
    let segs = if closed { fit_closed(&lp, tol, window) } else { fit_open(&lp, tol, window) };
    // A closed loop with corners starts at its first corner, not necessarily at the first point.
    if let (Some(s), Some(PathCmd::MoveTo(m))) = (segs.first(), path.cmds.first_mut()) {
        *m = v(s[0]);
    }
    for s in segs {
        path.cubic_to(v(s[1]), v(s[2]), v(s[3]));
    }
    if closed {
        path.close();
    }
    path
}

/// The point `window` of arc length away from `i`, forwards or backwards. Open strokes stop at
/// their ends; closed ones wrap.
fn walk(lp: &[P], i: usize, window: f64, fwd: bool, closed: bool) -> P {
    let n = lp.len();
    let mut acc = 0.0;
    let mut cur = i.min(n - 1);
    for _ in 0..n {
        let nx = match (fwd, closed) {
            (true, true) => (cur + 1) % n,
            (false, true) => (cur + n - 1) % n,
            (true, false) if cur + 1 < n => cur + 1,
            (false, false) if cur > 0 => cur - 1,
            _ => break,
        };
        let d = len(sub(lp[nx], lp[cur]));
        if acc + d >= window {
            let t = (window - acc) / d.max(1e-12);
            return add(lp[cur], mul(sub(lp[nx], lp[cur]), t));
        }
        acc += d;
        cur = nx;
    }
    lp[cur]
}

/// Indices of corner points (never an open stroke's ends).
fn corners(lp: &[P], window: f64, closed: bool) -> Vec<usize> {
    let n = lp.len();
    let cos_thresh = 60f64.to_radians().cos();
    let interior = |i: usize| closed || (i > 0 && i + 1 < n);
    let turn: Vec<f64> = (0..n)
        .map(|i| {
            if !interior(i) {
                return 1.0;
            }
            let a = norm(sub(lp[i], walk(lp, i, window, false, closed)));
            let b = norm(sub(walk(lp, i, window, true, closed), lp[i]));
            dot(a, b)
        })
        .collect();
    let step = |j: usize, fwd: bool| -> Option<usize> {
        match (fwd, closed) {
            (true, true) => Some((j + 1) % n),
            (false, true) => Some((j + n - 1) % n),
            (true, false) => (j + 1 < n).then_some(j + 1),
            (false, false) => j.checked_sub(1),
        }
    };
    let mut out = Vec::new();
    for i in 0..n {
        if turn[i] >= cos_thresh {
            continue;
        }
        // The sharpest point within ±window (ties go to the first index).
        let mut is_min = true;
        for fwd in [true, false] {
            let (mut acc, mut j) = (0.0, i);
            while is_min && acc < window {
                let Some(nx) = step(j, fwd) else { break };
                acc += len(sub(lp[nx], lp[j]));
                j = nx;
                if j == i {
                    break;
                }
                if if fwd { turn[j] < turn[i] } else { turn[j] <= turn[i] } {
                    is_min = false;
                }
            }
        }
        if is_min {
            out.push(i);
        }
    }
    out
}

fn fit_open(lp: &[P], tol: f64, window: f64) -> Vec<[P; 4]> {
    let n = lp.len();
    let mut cuts = vec![0];
    cuts.extend(corners(lp, window, false));
    cuts.push(n - 1);
    let mut segs = Vec::new();
    for w in cuts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let run = &lp[a..=b];
        let t1 = norm(sub(walk(run, 0, window, true, false), run[0]));
        let t2 = norm(sub(walk(run, run.len() - 1, window, false, false), run[run.len() - 1]));
        fit_cubic(run, t1, t2, tol * tol, &mut segs, 0);
    }
    segs
}

fn fit_closed(lp: &[P], tol: f64, window: f64) -> Vec<[P; 4]> {
    let n = lp.len();
    let cs = corners(lp, window, true);
    let mut segs = Vec::new();
    if cs.is_empty() {
        // A smooth loop: start at 0 with one tangent across the seam.
        let mut pts = lp.to_vec();
        pts.push(lp[0]);
        let t = norm(sub(walk(lp, 0, window, true, true), walk(lp, 0, window, false, true)));
        fit_cubic(&pts, t, mul(t, -1.0), tol * tol, &mut segs, 0);
        return segs;
    }
    for (k, &c) in cs.iter().enumerate() {
        let end = cs[(k + 1) % cs.len()];
        let mut pts = vec![lp[c]];
        let mut j = c;
        loop {
            j = (j + 1) % n;
            pts.push(lp[j]);
            if j == end {
                break;
            }
        }
        let m = pts.len();
        let t1 = norm(sub(walk(&pts, 0, window, true, false), pts[0]));
        let t2 = norm(sub(walk(&pts, m - 1, window, false, false), pts[m - 1]));
        fit_cubic(&pts, t1, t2, tol * tol, &mut segs, 0);
    }
    segs
}

fn bezier(b: &[P; 4], t: f64) -> P {
    let mt = 1.0 - t;
    let (a, bb, c, d) = (mt * mt * mt, 3.0 * mt * mt * t, 3.0 * mt * t * t, t * t * t);
    (a * b[0].0 + bb * b[1].0 + c * b[2].0 + d * b[3].0, a * b[0].1 + bb * b[1].1 + c * b[2].1 + d * b[3].1)
}

fn bezier_d1(b: &[P; 4], t: f64) -> P {
    let mt = 1.0 - t;
    let q0 = mul(sub(b[1], b[0]), 3.0);
    let q1 = mul(sub(b[2], b[1]), 3.0);
    let q2 = mul(sub(b[3], b[2]), 3.0);
    add(add(mul(q0, mt * mt), mul(q1, 2.0 * mt * t)), mul(q2, t * t))
}

fn bezier_d2(b: &[P; 4], t: f64) -> P {
    let r0 = mul(add(sub(b[2], mul(b[1], 2.0)), b[0]), 6.0);
    let r1 = mul(add(sub(b[3], mul(b[2], 2.0)), b[1]), 6.0);
    add(mul(r0, 1.0 - t), mul(r1, t))
}

fn chord_params(pts: &[P]) -> Vec<f64> {
    let mut u = vec![0.0; pts.len()];
    for i in 1..pts.len() {
        u[i] = u[i - 1] + len(sub(pts[i], pts[i - 1]));
    }
    let total = u[pts.len() - 1];
    if total > 0.0 {
        for v in &mut u {
            *v /= total;
        }
    }
    u
}

/// Least-squares control points with fixed end tangents (`pts.len() >= 2`).
fn generate(pts: &[P], u: &[f64], t1: P, t2: P) -> [P; 4] {
    let (p0, p3) = (pts[0], pts[pts.len() - 1]);
    let mut c = [[0.0f64; 2]; 2];
    let mut x = [0.0f64; 2];
    for (i, &t) in u.iter().enumerate() {
        let mt = 1.0 - t;
        let b1 = 3.0 * mt * mt * t;
        let b2 = 3.0 * mt * t * t;
        let a1 = mul(t1, b1);
        let a2 = mul(t2, b2);
        c[0][0] += dot(a1, a1);
        c[0][1] += dot(a1, a2);
        c[1][1] += dot(a2, a2);
        let tmp = sub(pts[i], add(mul(p0, mt * mt * mt + b1), mul(p3, b2 + t * t * t)));
        x[0] += dot(a1, tmp);
        x[1] += dot(a2, tmp);
    }
    c[1][0] = c[0][1];
    let det = c[0][0] * c[1][1] - c[1][0] * c[0][1];
    let det0 = c[0][0] * x[1] - c[1][0] * x[0];
    let det1 = x[0] * c[1][1] - x[1] * c[0][1];
    let (mut al, mut ar) = if det.abs() > 1e-12 { (det1 / det, det0 / det) } else { (0.0, 0.0) };
    let seg = len(sub(p3, p0));
    // A handle longer than the points' own length is a bad solve (a nearly closed stretch can
    // send it far off): fall back, and the error check splits the stretch instead (ADR 0093).
    let arc: f64 = pts.windows(2).map(|w| len(sub(w[1], w[0]))).sum();
    // Schneider's fallback when the solve is degenerate or flips a handle.
    if al < 1e-6 * seg || ar < 1e-6 * seg || al > arc || ar > arc {
        al = seg / 3.0;
        ar = seg / 3.0;
    }
    [p0, add(p0, mul(t1, al)), add(p3, mul(t2, ar)), p3]
}

/// Largest squared distance between the points and the curve, and where (`pts.len() >= 3`).
fn max_error(pts: &[P], b: &[P; 4], u: &[f64]) -> (f64, usize) {
    let mut best = (0.0, pts.len() / 2);
    for i in 1..pts.len() - 1 {
        let d = sub(bezier(b, u[i]), pts[i]);
        let e = dot(d, d);
        if e >= best.0 {
            best = (e, i);
        }
    }
    best
}

/// One Newton step towards each point's nearest parameter on the curve.
fn reparam(pts: &[P], u: &[f64], b: &[P; 4]) -> Vec<f64> {
    pts.iter()
        .zip(u)
        .map(|(p, &t)| {
            let q = bezier(b, t);
            let d1 = bezier_d1(b, t);
            let d2 = bezier_d2(b, t);
            let num = dot(sub(q, *p), d1);
            let den = dot(d1, d1) + dot(sub(q, *p), d2);
            if den.abs() < 1e-12 { t } else { (t - num / den).clamp(0.0, 1.0) }
        })
        .collect()
}

/// Fits `pts` (at least two) and appends the segments to `out`. Each split shortens both
/// halves, so recursion ends; `depth` caps it anyway.
fn fit_cubic(pts: &[P], t1: P, t2: P, err2: f64, out: &mut Vec<[P; 4]>, depth: u32) {
    let n = pts.len();
    if n < 2 {
        return;
    }
    if n == 2 || depth > 40 {
        let d = len(sub(pts[n - 1], pts[0])) / 3.0;
        out.push([pts[0], add(pts[0], mul(t1, d)), add(pts[n - 1], mul(t2, d)), pts[n - 1]]);
        return;
    }
    let mut u = chord_params(pts);
    let mut b = generate(pts, &u, t1, t2);
    let (mut e, mut split) = max_error(pts, &b, &u);
    if e < err2 {
        out.push(b);
        return;
    }
    if e < err2 * 16.0 {
        for _ in 0..6 {
            u = reparam(pts, &u, &b);
            b = generate(pts, &u, t1, t2);
            (e, split) = max_error(pts, &b, &u);
            if e < err2 {
                out.push(b);
                return;
            }
        }
    }
    let split = split.clamp(1, n - 2);
    let tc = norm(sub(pts[split - 1], pts[split + 1]));
    let tc = if len(tc) < 0.5 { norm(sub(pts[split - 1], pts[split])) } else { tc };
    fit_cubic(&pts[..=split], t1, tc, err2, out, depth + 1);
    fit_cubic(&pts[split..], mul(tc, -1.0), t2, err2, out, depth + 1);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Affine;

    fn max_dist(path: &Path, pts: &[Vec2]) -> f32 {
        let lines = path.flatten(&Affine::IDENTITY, 0.05);
        pts.iter()
            .map(|p| {
                lines
                    .iter()
                    .flat_map(|l| l.points.windows(2).map(|w| crate::path::segment_distance(*p, w[0], w[1])))
                    .fold(f32::INFINITY, f32::min)
            })
            .fold(0.0, f32::max)
    }

    #[test]
    fn smooth_wave_fits_within_tolerance_with_few_segments() {
        let wave: Vec<Vec2> = (0..=300).map(|i| Vec2::new(i as f32, (i as f32 * 0.04).sin() * 40.0)).collect();
        let path = fit_curve(&wave, 1.0, 3.0, false);
        let segs = path.cmds.iter().filter(|c| matches!(c, PathCmd::CubicTo(..))).count();
        assert!((1..=12).contains(&segs), "{segs} segments");
        assert!(max_dist(&path, &wave) <= 1.05, "{}", max_dist(&path, &wave));
        assert_eq!(path.point(0, crate::edit::ANCHOR), Some(wave[0]), "starts where the stroke starts");
    }

    #[test]
    fn corners_stay_sharp() {
        // An "L": down then right, sampled every pixel.
        let mut pts: Vec<Vec2> = (0..=50).map(|i| Vec2::new(0.0, i as f32)).collect();
        pts.extend((1..=50).map(|i| Vec2::new(i as f32, 50.0)));
        let path = fit_curve(&pts, 0.5, 3.0, false);
        assert!(path.anchors().iter().any(|(_, a)| a.distance(Vec2::new(0.0, 50.0)) < 1e-3), "an anchor at the corner");
        assert!(max_dist(&path, &pts) <= 0.55);
    }

    #[test]
    fn closed_loop_with_corners_starts_on_a_corner() {
        // A square traced from the middle of a side: the loop must start at a corner, or its
        // first edge would cut straight across from the first point.
        let mut pts: Vec<Vec2> = (25..50).map(|i| Vec2::new(i as f32, 0.0)).collect();
        pts.extend((0..50).map(|i| Vec2::new(50.0, i as f32)));
        pts.extend((0..50).map(|i| Vec2::new(50.0 - i as f32, 50.0)));
        pts.extend((0..50).map(|i| Vec2::new(0.0, 50.0 - i as f32)));
        pts.extend((0..25).map(|i| Vec2::new(i as f32, 0.0)));
        let path = fit_curve(&pts, 0.5, 3.0, true);
        assert!(max_dist(&path, &pts) <= 0.55);
        let start = path.point(0, crate::edit::ANCHOR);
        assert!(start.is_some_and(|s| [(0.0, 0.0), (50.0, 0.0), (50.0, 50.0), (0.0, 50.0)].iter().any(|c| s.distance(Vec2::new(c.0, c.1)) < 1e-3)), "{start:?}");
        // Every point of the curve is on the square's outline (no shortcut across it).
        for pl in path.flatten(&Affine::IDENTITY, 0.05) {
            assert!(pl.points.iter().all(|p| p.x.min(p.y).min(50.0 - p.x).min(50.0 - p.y).abs() < 0.6), "a segment cuts across");
        }
    }

    #[test]
    fn closed_loop_and_degenerate_input() {
        let circle: Vec<Vec2> = (0..120).map(|i| {
            let a = i as f32 / 120.0 * std::f32::consts::TAU;
            Vec2::new(50.0 + 30.0 * a.cos(), 50.0 + 30.0 * a.sin())
        }).collect();
        let path = fit_curve(&circle, 0.5, 3.0, true);
        assert!(matches!(path.cmds.last(), Some(PathCmd::Close)));
        assert!(max_dist(&path, &circle) <= 0.55);
        // Repeated, non-finite and tiny inputs never panic.
        let same = vec![Vec2::new(1.0, 1.0); 20];
        assert_eq!(fit_curve(&same, 1.0, 3.0, false).cmds.len(), 1);
        assert!(fit_curve(&[], 1.0, 3.0, true).is_empty());
        let odd = [Vec2::new(0.0, 0.0), Vec2::new(f32::NAN, 1.0), Vec2::new(5.0, 0.0), Vec2::new(5.0, 0.0)];
        assert_eq!(fit_curve(&odd, f32::NAN, -1.0, false).cmds.len(), 2);
    }
}
