//! Brush strokes as filled outlines (ADR 0033).
//!
//! A brush paints the area its tip sweeps along the drag. The outline is built in four steps:
//!
//! 1. **Centre line.** The samples are smoothed by fitting cubic curves (`crate::fit_curve`)
//!    and flattened back to points; each point takes the pressure found at the same fraction
//!    of the stroke's length.
//! 2. **Tip space.** Everything is mapped by the inverse of the tip's transform (its size,
//!    roundness and angle), so the tip becomes the unit circle (round) or unit square (square)
//!    scaled by the pressure.
//! 3. **Distance field.** Between two points the tip sweeps exactly the convex hull of the tip
//!    at both ends (position and size both move linearly, and a convex combination of two
//!    points of a convex tip is in the tip). The signed distance to that hull is
//!    `min over t of |x - c(t)| - r(t)` (the L2 norm for round tips, L∞ for square ones), a
//!    convex function of `t` with a closed-form minimum. The minimum over all segments is the
//!    exact distance to the whole sweep outside it, and 1-Lipschitz everywhere. It is sampled
//!    on a grid, each segment only over its own bounds.
//! 4. **Outline.** The zero level is traced by marching squares (`crate::trace`), with linear
//!    interpolation along cell edges (accurate well below a cell, since the field is a true
//!    distance), mapped back to the stage and fitted with cubic curves again, keeping corners.
//!
//! The result is closed loops (outer boundaries and holes) filled with the even-odd rule. The
//! grid has at most `MAX_CELLS` cells, so cost and memory are bounded for any input.

use crate::{Affine, Path, PathCmd, Vec2, fit_curve, trace};

/// Largest distance grid one stroke uses (cells; 16 MB of `f32`).
pub const MAX_CELLS: usize = 4_000_000;
/// Most samples one stroke may have.
pub const MAX_SAMPLES: usize = 100_000;
/// The tip's size at zero pressure, as a fraction of its full size.
pub const MIN_PRESSURE_SIZE: f32 = 0.1;

/// The tip's outline.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TipShape {
    #[default]
    Round,
    Square,
}

/// A brush tip: its shape, its size (diameter or side, in stage units), its angle (degrees,
/// clockwise on the stage) and roundness (the narrow side as a fraction of the wide one).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tip {
    pub shape: TipShape,
    pub size: f32,
    pub angle: f32,
    pub roundness: f32,
}

impl Tip {
    pub fn round(size: f32) -> Tip {
        Tip { shape: TipShape::Round, size, angle: 0.0, roundness: 1.0 }
    }

    /// Maps the unit tip (circle or square of radius 1 about the origin) onto the stage.
    pub fn to_stage(&self) -> Affine {
        let half = if self.size.is_finite() { self.size.max(0.0) * 0.5 } else { 0.0 };
        let round = if self.roundness.is_finite() { self.roundness.clamp(0.05, 1.0) } else { 1.0 };
        let angle = if self.angle.is_finite() { self.angle.rem_euclid(180.0) } else { 0.0 };
        let r = if angle == 0.0 { Affine::IDENTITY } else { Affine::rotate(angle.to_radians()) };
        r.mul(&Affine::scale(half, half * round))
    }
}

/// One pointer sample: where, and the pen pressure (0..1; a mouse gives 1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub at: Vec2,
    pub pressure: f32,
}

/// The tip's size factor at `pressure`: from `MIN_PRESSURE_SIZE` at none to 1 at full.
pub fn size_factor(pressure: f32) -> f32 {
    let p = if pressure.is_finite() { pressure.clamp(0.0, 1.0) } else { 1.0 };
    MIN_PRESSURE_SIZE + (1.0 - MIN_PRESSURE_SIZE) * p
}

/// The smoothed centre line: points with the tip's size factor at each. `smoothing` is the
/// curve fit's tolerance (stage units); `flatness` how closely the points follow the fitted
/// curves. One distinct sample gives one point (a dab).
pub fn centreline(samples: &[Sample], smoothing: f32, flatness: f32) -> Vec<(Vec2, f32)> {
    let mut pts: Vec<Vec2> = Vec::with_capacity(samples.len().min(MAX_SAMPLES));
    let mut press: Vec<f32> = Vec::with_capacity(pts.capacity());
    for s in samples.iter().take(MAX_SAMPLES).filter(|s| s.at.is_finite()) {
        if pts.last().is_some_and(|l| l.distance(s.at) <= 1e-4) {
            // A held pointer: keep the latest pressure.
            if let Some(p) = press.last_mut() {
                *p = size_factor(s.pressure);
            }
            continue;
        }
        pts.push(s.at);
        press.push(size_factor(s.pressure));
    }
    match (pts.as_slice(), press.as_slice()) {
        ([], _) => return Vec::new(),
        ([p], [f, ..]) => return vec![(*p, *f)],
        _ => {}
    }
    let tol = if smoothing.is_finite() && smoothing > 0.01 { smoothing } else { 0.01 };
    let flat = if flatness.is_finite() && flatness > 1e-3 { flatness } else { 0.05 };
    let fitted = fit_curve(&pts, tol, tol * 2.0, false);
    let line: Vec<Vec2> = fitted.flatten(&Affine::IDENTITY, flat).into_iter().next().map(|p| p.points).unwrap_or_else(|| pts.clone());
    // Pressure by fraction of length: the fit keeps the stroke's ends and its corners, so the
    // same fraction is about the same place.
    let along = |v: &[Vec2]| -> Vec<f32> {
        let mut acc = Vec::with_capacity(v.len());
        let mut d = 0.0;
        let mut prev = v.first().copied().unwrap_or(Vec2::ZERO);
        for p in v {
            d += prev.distance(*p);
            prev = *p;
            acc.push(d);
        }
        let total = d.max(1e-9);
        acc.iter_mut().for_each(|a| *a /= total);
        acc
    };
    let (src, dst) = (along(&pts), along(&line));
    let mut j = 0;
    line.iter()
        .zip(dst)
        .map(|(p, u)| {
            while j + 2 < src.len() && src.get(j + 1).is_some_and(|s| *s < u) {
                j += 1;
            }
            let (s0, s1) = (src.get(j).copied().unwrap_or(0.0), src.get(j + 1).copied().unwrap_or(1.0));
            let (f0, f1) = (press.get(j).copied().unwrap_or(1.0), press.get(j + 1).copied().or(press.get(j).copied()).unwrap_or(1.0));
            let t = if s1 - s0 > 1e-9 { ((u - s0) / (s1 - s0)).clamp(0.0, 1.0) } else { 0.0 };
            (*p, f0 + (f1 - f0) * t)
        })
        .collect()
}

/// The filled outline of a brush stroke: closed loops (even-odd), or an empty path when there
/// is nothing to paint (no finite samples, a zero-size tip, or a stroke too large to resolve).
/// `smoothing` is the centre line's curve-fit tolerance and `tolerance` the outline's (both in
/// stage units).
pub fn outline(samples: &[Sample], tip: &Tip, smoothing: f32, tolerance: f32) -> Path {
    let tol = if tolerance.is_finite() && tolerance > 1e-3 { tolerance } else { 0.25 };
    let line = centreline(samples, smoothing, tol * 0.25);
    let m = tip.to_stage();
    let Some(inv) = m.inverse().filter(|i| i.is_finite()) else { return Path::new() };
    if line.is_empty() || !(tip.size.is_finite() && tip.size > 0.0) {
        return Path::new();
    }
    // Tip space, in f64.
    let q: Vec<(P, f64)> = line.iter().map(|(p, f)| (v64(inv.apply(*p)), f64::from(*f))).collect();
    let (mut lo, mut hi) = ((f64::INFINITY, f64::INFINITY), (f64::NEG_INFINITY, f64::NEG_INFINITY));
    let mut fmin: f64 = 1.0;
    for (p, f) in &q {
        lo = (lo.0.min(p.0 - f), lo.1.min(p.1 - f));
        hi = (hi.0.max(p.0 + f), hi.1.max(p.1 + f));
        fmin = fmin.min(*f);
    }
    if !(lo.0.is_finite() && lo.1.is_finite() && hi.0.is_finite() && hi.1.is_finite()) {
        return Path::new();
    }
    // Cells small enough to resolve the thinnest part and the tip's corners, unless the grid
    // budget says otherwise.
    let fine = match tip.shape {
        TipShape::Round => 1.0 / 6.0,
        TipShape::Square => 1.0 / 8.0,
    };
    let mut c: f64 = (fmin / 2.5).min(fine);
    for _ in 0..4 {
        let cells = ((hi.0 - lo.0) / c + 2.0 * MARGIN + 2.0) * ((hi.1 - lo.1) / c + 2.0 * MARGIN + 2.0);
        if cells <= MAX_CELLS as f64 {
            break;
        }
        c *= (cells / MAX_CELLS as f64).sqrt() * 1.01;
    }
    let origin = (lo.0 - MARGIN * c, lo.1 - MARGIN * c);
    let w = ((hi.0 - lo.0) / c + 2.0 * MARGIN).ceil() + 1.0;
    let h = ((hi.1 - lo.1) / c + 2.0 * MARGIN).ceil() + 1.0;
    if !(w * h).is_finite() || w * h > MAX_CELLS as f64 * 1.05 {
        return Path::new();
    }
    let (w, h) = (w as usize, h as usize);
    let mut field = vec![OUTSIDE as f32; w * h];
    // Nothing thinner than a cell, so no part slips between the samples.
    let pts: Vec<(P, f64)> = thin(&q, c * 0.5).into_iter().map(|(p, f)| (p, f.max(c))).collect();
    let mut stamp = |a: P, ra: f64, b: P, rb: f64| {
        let reach = ra.max(rb) + (MARGIN - 1.0) * c;
        let (x0, x1) = (((a.0.min(b.0) - reach - origin.0) / c).floor().max(0.0), ((a.0.max(b.0) + reach - origin.0) / c).ceil());
        let (y0, y1) = (((a.1.min(b.1) - reach - origin.1) / c).floor().max(0.0), ((a.1.max(b.1) + reach - origin.1) / c).ceil());
        let (x1, y1) = (x1.min(w as f64 - 1.0), y1.min(h as f64 - 1.0));
        if x1 < x0 || y1 < y0 {
            return;
        }
        for y in y0 as usize..=y1 as usize {
            let py = origin.1 + (y as f64 + 0.5) * c;
            for x in x0 as usize..=x1 as usize {
                let px = origin.0 + (x as f64 + 0.5) * c;
                let d = match tip.shape {
                    TipShape::Round => round_dist((px, py), a, ra, b, rb),
                    TipShape::Square => square_dist((px, py), a, ra, b, rb),
                };
                let v = (0.5 - d / c).clamp(OUTSIDE, 1.0 + MARGIN) as f32;
                if let Some(cell) = field.get_mut(y * w + x)
                    && v > *cell
                {
                    *cell = v;
                }
            }
        }
    };
    match pts.as_slice() {
        [(p, f)] => stamp(*p, *f, *p, *f),
        _ => {
            for pair in pts.windows(2) {
                let &[(a, ra), (b, rb)] = pair else { continue };
                // Long segments in short pieces, so each one's bounds stay small (the pieces'
                // sweeps are the segment's).
                let n = (dist(a, b) / PIECE.max(16.0 * c)).ceil().clamp(1.0, 1e6) as usize;
                for i in 0..n {
                    let (t0, t1) = (i as f64 / n as f64, (i + 1) as f64 / n as f64);
                    stamp(lerp(a, b, t0), ra + (rb - ra) * t0, lerp(a, b, t1), ra + (rb - ra) * t1);
                }
            }
        }
    }
    let to_stage = |p: Vec2| m.apply(Vec2::new((origin.0 + f64::from(p.x) * c) as f32, (origin.1 + f64::from(p.y) * c) as f32));
    // A corner is a turn of 60° over the window either side. Round tips: two cells (their
    // tightest curve is 2.5 cells, too gentle to count). Square tips: four, so a corner the
    // tracing cut off still turns enough to stay sharp.
    let cell_stage = (c as f32) * tip.size * 0.5;
    let cells = match tip.shape {
        TipShape::Round => 2.0,
        TipShape::Square => 4.0,
    };
    let window = (cells * cell_stage).max(tol * 2.0);
    let mut out = Path::new();
    for lp in trace::contours(&field, w, h) {
        let stage: Vec<Vec2> = lp.into_iter().map(to_stage).collect();
        let part = fit_curve(&stage, tol, window, true);
        if part.cmds.iter().any(|c| matches!(c, PathCmd::CubicTo(..) | PathCmd::LineTo(..))) {
            out.cmds.extend(part.cmds);
        }
    }
    out
}

type P = (f64, f64);

/// Cells of free margin round the sweep (the field is sampled this far out).
const MARGIN: f64 = 3.0;
/// The field value of a cell no segment reaches (more than `MARGIN` cells outside).
const OUTSIDE: f64 = 0.5 - MARGIN;
/// Longest segment piece, in tip radii.
const PIECE: f64 = 4.0;

fn v64(p: Vec2) -> P {
    (f64::from(p.x), f64::from(p.y))
}
fn dist(a: P, b: P) -> f64 {
    (a.0 - b.0).hypot(a.1 - b.1)
}
fn lerp(a: P, b: P, t: f64) -> P {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

/// Drops points closer than `gap` to the last kept one (the last point always stays).
fn thin(q: &[(P, f64)], gap: f64) -> Vec<(P, f64)> {
    let mut out: Vec<(P, f64)> = Vec::with_capacity(q.len());
    for (i, &(p, f)) in q.iter().enumerate() {
        let last = i + 1 == q.len();
        match out.last_mut() {
            Some((l, lf)) if dist(*l, p) < gap => {
                if last {
                    (*l, *lf) = (p, f);
                }
            }
            _ => out.push((p, f)),
        }
    }
    out
}

/// Signed distance from `x` to the round sweep from disc (`a`, `ra`) to disc (`b`, `rb`):
/// the minimum over `t` of `|x - c(t)| - r(t)`, convex in `t`, so its stationary point (clamped
/// to the segment) is the minimum.
fn round_dist(x: P, a: P, ra: f64, b: P, rb: f64) -> f64 {
    let d = (b.0 - a.0, b.1 - a.1);
    let l = d.0.hypot(d.1);
    let dr = rb - ra;
    let ends = || (dist(x, a) - ra).min(dist(x, b) - rb);
    if l < 1e-12 || dr.abs() >= l {
        // One disc holds the other: the minimum is at an end.
        return ends();
    }
    let u = (x.0 - a.0, x.1 - a.1);
    let along = (u.0 * d.0 + u.1 * d.1) / l;
    let off = (u.0 * d.1 - u.1 * d.0).abs() / l;
    let k = dr / l;
    // h'(t) = 0 where (tL - along) / |..| = k.
    let z = k * off / (1.0 - k * k).sqrt();
    let t = ((along + z) / l).clamp(0.0, 1.0);
    let c = lerp(a, b, t);
    dist(x, c) - (ra + dr * t)
}

/// The same for a square tip (axis-aligned in tip space, so the L∞ norm): the maximum of four
/// lines in `t`, so the minimum is at an end or where two of them cross.
fn square_dist(x: P, a: P, ra: f64, b: P, rb: f64) -> f64 {
    let d = (b.0 - a.0, b.1 - a.1);
    let u = (x.0 - a.0, x.1 - a.1);
    let dr = rb - ra;
    // l_k(t) = slope * t + icept, for ±(t·d - u) on each axis, minus the radius.
    let lines = [(d.0 - dr, -u.0 - ra), (-d.0 - dr, u.0 - ra), (d.1 - dr, -u.1 - ra), (-d.1 - dr, u.1 - ra)];
    let h = |t: f64| lines.iter().map(|(s, i)| s * t + i).fold(f64::NEG_INFINITY, f64::max);
    let mut best = h(0.0).min(h(1.0));
    for (i, (s1, i1)) in lines.iter().enumerate() {
        for (s2, i2) in lines.iter().skip(i + 1) {
            if (s1 - s2).abs() > 1e-12 {
                let t = (i2 - i1) / (s1 - s2);
                if (0.0..=1.0).contains(&t) {
                    best = best.min(h(t));
                }
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FillRule;

    fn line(a: Vec2, b: Vec2, n: usize, pressure: impl Fn(f32) -> f32) -> Vec<Sample> {
        (0..=n).map(|i| i as f32 / n as f32).map(|t| Sample { at: a.lerp(b, t), pressure: pressure(t) }).collect()
    }

    /// Area of an even-odd path (flattened finely).
    fn area(p: &Path) -> f32 {
        p.flatten(&Affine::IDENTITY, 0.01)
            .iter()
            .map(|pl| {
                let n = pl.points.len();
                (0..n).map(|i| pl.points[i].cross(pl.points[(i + 1) % n])).sum::<f32>() * 0.5
            })
            .map(f32::abs)
            .sum()
    }

    #[test]
    fn straight_round_stroke_has_the_analytic_area() {
        // Length × size plus the two half-disc ends.
        let (len, size) = (200.0, 20.0);
        let s = line(Vec2::new(50.0, 80.0), Vec2::new(50.0 + len, 80.0), 40, |_| 1.0);
        let p = outline(&s, &Tip::round(size), 1.0, 0.1);
        let want = len * size + std::f32::consts::PI * (size / 2.0).powi(2);
        let got = area(&p);
        assert!((got - want).abs() / want < 0.01, "area {got}, want {want}");
        assert!(p.contains(Vec2::new(150.0, 80.0 + size * 0.45), &Affine::IDENTITY, FillRule::EvenOdd));
        assert!(!p.contains(Vec2::new(150.0, 80.0 + size * 0.55), &Affine::IDENTITY, FillRule::EvenOdd));
        // Few curves at the editor's tolerance (a third of a screen point): two long sides and
        // two caps.
        let p = outline(&s, &Tip::round(size), 1.0, 0.35);
        assert!(p.anchors().len() <= 16, "{} anchors", p.anchors().len());
    }

    #[test]
    fn a_dab_is_the_tip() {
        let p = outline(&[Sample { at: Vec2::new(10.0, 10.0), pressure: 1.0 }], &Tip::round(8.0), 1.0, 0.05);
        let want = std::f32::consts::PI * 16.0;
        assert!((area(&p) - want).abs() / want < 0.01, "{}", area(&p));
        // A square tip at 45°: a diamond of the same area as the square.
        let tip = Tip { shape: TipShape::Square, size: 10.0, angle: 45.0, roundness: 1.0 };
        let d = outline(&[Sample { at: Vec2::ZERO, pressure: 1.0 }], &tip, 1.0, 0.05);
        assert!((area(&d) - 100.0).abs() < 2.0, "{}", area(&d));
        assert!(d.contains(Vec2::new(0.0, 6.8), &Affine::IDENTITY, FillRule::EvenOdd), "a corner points down");
        assert!(!d.contains(Vec2::new(4.6, 4.6), &Affine::IDENTITY, FillRule::EvenOdd), "an unturned square's corner is outside");
    }

    #[test]
    fn pressure_varies_the_width() {
        let s = line(Vec2::ZERO, Vec2::new(300.0, 0.0), 60, |t| t);
        let p = outline(&s, &Tip::round(20.0), 1.0, 0.1);
        let inside = |x: f32, y: f32| p.contains(Vec2::new(x, y), &Affine::IDENTITY, FillRule::EvenOdd);
        // Near the start the tip is small (pressure ~0.1), at the end full size.
        assert!(inside(30.0, 1.0) && !inside(30.0, 4.0), "thin at the start");
        assert!(inside(290.0, 9.0), "full width at the end");
        // Halfway: factor 0.1 + 0.9 * 0.5 = 0.55, half-width 5.5.
        assert!(inside(150.0, 5.0) && !inside(150.0, 6.2));
    }

    #[test]
    fn smoothing_reduces_anchors() {
        // A shaky hand: a straight drag with jitter.
        let s: Vec<Sample> = (0..=200).map(|i| Sample { at: Vec2::new(i as f32 * 2.0, ((i * 7919) % 13) as f32 * 0.35), pressure: 1.0 }).collect();
        let rough = outline(&s, &Tip::round(12.0), 0.3, 0.3);
        let smooth = outline(&s, &Tip::round(12.0), 6.0, 0.3);
        let (a, b) = (rough.anchors().len(), smooth.anchors().len());
        assert!(b * 2 < a, "smoothing should cut anchors: {a} rough, {b} smooth");
    }

    #[test]
    fn crossing_strokes_leave_holes_and_ellipses_follow_the_angle() {
        // A loop that closes on itself leaves a hole in the middle.
        let ring: Vec<Sample> = (0..=72).map(|i| i as f32 / 72.0 * std::f32::consts::TAU).map(|a| Sample { at: Vec2::new(100.0 * a.cos(), 100.0 * a.sin()), pressure: 1.0 }).collect();
        let p = outline(&ring, &Tip::round(10.0), 0.5, 0.2);
        let loops = p.cmds.iter().filter(|c| matches!(c, PathCmd::MoveTo(_))).count();
        assert_eq!(loops, 2, "outer edge and hole");
        assert!(!p.contains(Vec2::ZERO, &Affine::IDENTITY, FillRule::EvenOdd));
        assert!(p.contains(Vec2::new(100.0, 0.0), &Affine::IDENTITY, FillRule::EvenOdd));
        // A flat tip across a horizontal drag paints wide; along it, thin.
        let flat = |angle: f32| Tip { shape: TipShape::Round, size: 20.0, angle, roundness: 0.2 };
        let s = line(Vec2::ZERO, Vec2::new(100.0, 0.0), 10, |_| 1.0);
        let across = outline(&s, &flat(90.0), 1.0, 0.1).bounds();
        let along = outline(&s, &flat(0.0), 1.0, 0.1).bounds();
        assert!((across.height() - 20.0).abs() < 0.5 && (along.height() - 4.0).abs() < 0.5, "{} {}", across.height(), along.height());
    }

    #[test]
    fn distance_functions_match_brute_force() {
        let cases = [((0.0, 0.0), 1.0, (10.0, 0.0), 3.0), ((0.0, 0.0), 2.0, (0.5, 0.5), 0.2), ((5.0, -3.0), 0.5, (-4.0, 6.0), 0.5)];
        for (a, ra, b, rb) in cases {
            for x in [(3.0, 4.0), (-2.0, 0.1), (12.0, -1.0), (0.2, 0.2), (5.0, 0.0)] {
                let brute = |norm: fn(P) -> f64| (0..=4000).map(|i| i as f64 / 4000.0).map(|t| norm((x.0 - lerp(a, b, t).0, x.1 - lerp(a, b, t).1)) - (ra + (rb - ra) * t)).fold(f64::INFINITY, f64::min);
                let r = round_dist(x, a, ra, b, rb);
                let s = square_dist(x, a, ra, b, rb);
                assert!((r - brute(|v| v.0.hypot(v.1))).abs() < 1e-3, "round {x:?}: {r}");
                assert!((s - brute(|v| v.0.abs().max(v.1.abs()))).abs() < 1e-3, "square {x:?}: {s}");
            }
        }
    }

    #[test]
    fn hostile_input_is_empty_or_bounded() {
        assert!(outline(&[], &Tip::round(10.0), 1.0, 1.0).is_empty());
        let nan = [Sample { at: Vec2::new(f32::NAN, 0.0), pressure: f32::NAN }];
        assert!(outline(&nan, &Tip::round(10.0), 1.0, 1.0).is_empty());
        assert!(outline(&line(Vec2::ZERO, Vec2::new(10.0, 0.0), 3, |_| 1.0), &Tip::round(0.0), 1.0, 1.0).is_empty());
        assert!(outline(&line(Vec2::ZERO, Vec2::new(10.0, 0.0), 3, |_| 1.0), &Tip::round(f32::INFINITY), 1.0, 1.0).is_empty());
        // Huge strokes coarsen the grid instead of growing it (and may come out empty).
        let far = line(Vec2::new(-1e7, -1e7), Vec2::new(1e7, 1e7), 4, |_| f32::NAN);
        let t = std::time::Instant::now();
        let _ = outline(&far, &Tip::round(3.0), f32::NAN, -1.0);
        let _ = outline(&far, &Tip { shape: TipShape::Square, size: 1e6, angle: f32::INFINITY, roundness: 0.0 }, 1.0, 1.0);
        assert!(t.elapsed().as_secs() < 20, "bounded work");
    }
}
