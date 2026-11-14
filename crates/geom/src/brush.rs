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
//!
//! **Tilt** (ADR 0093) turns the tip with the pen's azimuth and widens it as the pen leans.
//! When the tip turns along a stroke there is no one tip space, so the field is sampled in stage
//! space (in tip radii) instead, and each short piece of the stroke measures its distance in its
//! own tip space. **Texture** opens a paper grain of holes in the paint, and **scatter** throws
//! the dabs either side of the line: both are fixed patterns, so the same stroke always gives
//! the same outline, and both are still vector outlines.

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

/// One pointer sample: where, the pen pressure (0..1; a mouse gives 1) and the pen's tilt
/// where the system reports it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub at: Vec2,
    pub pressure: f32,
    pub tilt: Option<Tilt>,
}

impl Sample {
    /// A sample with no tilt.
    pub fn new(at: Vec2, pressure: f32) -> Sample {
        Sample { at, pressure, tilt: None }
    }
}

/// A pen's tilt, as the W3C Pointer Events spec gives it (in degrees): `altitude` from 90
/// (upright) to 0 (lying on the tablet), and `azimuth`, the direction the pen leans toward on
/// the stage, clockwise from the +x axis.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tilt {
    pub altitude: f32,
    pub azimuth: f32,
}

impl Tilt {
    /// How far the pen leans: 0 upright, 1 flat (`1 − altitude / 90°`).
    pub fn lean(self) -> f32 {
        let a = if self.altitude.is_finite() { self.altitude.clamp(0.0, 90.0) } else { 90.0 };
        1.0 - a / 90.0
    }
    /// The lean as a vector toward the azimuth (length [`Tilt::lean`]), which blends smoothly
    /// between samples and through upright.
    fn vector(self) -> Vec2 {
        let az = if self.azimuth.is_finite() { self.azimuth.rem_euclid(360.0).to_radians() } else { 0.0 };
        Vec2::new(az.cos(), az.sin()) * self.lean()
    }
}

/// What a pen's tilt shapes (Animate's tilt option).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TiltResponse {
    /// The tip turns with the pen: its angle is the brush's angle plus the azimuth.
    pub angle: bool,
    /// The tip widens as the pen leans: by `1 + TILT_WIDTH × lean`.
    pub width: bool,
}

/// How much wider a flat pen paints than an upright one, as a part of the size.
pub const TILT_WIDTH: f32 = 1.0;
/// Below this lean the azimuth is too noisy to turn the tip by.
const MIN_LEAN: f32 = 0.02;

/// The tip's angle (degrees) and size factor when the pen leans by `lean` (a vector toward the
/// azimuth, length 0..1), for a brush at `angle` (ADR 0093). Analytic: the angle is `angle +
/// azimuth`, and the size factor `1 + TILT_WIDTH × |lean|`.
pub fn tilted(angle: f32, lean: Vec2, response: TiltResponse) -> (f32, f32) {
    let l = if lean.is_finite() { lean.length().min(1.0) } else { 0.0 };
    let turned = if response.angle && l >= MIN_LEAN { angle + lean.y.atan2(lean.x).to_degrees() } else { angle };
    (turned, if response.width { 1.0 + TILT_WIDTH * l } else { 1.0 })
}

/// The tip's size factor at `pressure`: from `MIN_PRESSURE_SIZE` at none to 1 at full.
pub fn size_factor(pressure: f32) -> f32 {
    let p = if pressure.is_finite() { pressure.clamp(0.0, 1.0) } else { 1.0 };
    MIN_PRESSURE_SIZE + (1.0 - MIN_PRESSURE_SIZE) * p
}

/// How a stroke is shaped beyond its tip (ADR 0068): tapered ends, separate dabs and a ragged
/// edge. The default shapes nothing (ADR 0033's plain sweep).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Shaping {
    /// The part of the stroke's length (`0..=0.5`) over which it swells from a point at its
    /// start, and thins to a point at its end.
    pub taper_start: f32,
    pub taper_end: f32,
    /// `0`: one continuous sweep. Otherwise the tip is stamped as separate dabs this far apart,
    /// as a multiple of its size (`0.05..=10`).
    pub spacing: f32,
    /// How far the edge is worn away, as a part of the tip's size (`0..=0.5`): a chalk-like
    /// ragged edge from a fixed noise pattern, so it is the same every time.
    pub jitter: f32,
    /// A paper grain of holes through the paint (`0..=1`: none to most of it), fixed on the
    /// stage and scaled to the tip's size, as dry media on rough paper (ADR 0093).
    pub texture: f32,
    /// Dabs thrown either side of the line, up to this many sizes away (`0..=2`), and varied in
    /// size: spatter and stipple. A continuous stroke scatters as dabs a quarter size apart.
    pub scatter: f32,
    /// What the pen's tilt shapes.
    pub tilt: TiltResponse,
}

/// Most dabs one stroke stamps (the spacing widens past it).
pub const MAX_DABS: usize = 20_000;
/// Thinnest a taper makes the tip, as a part of its size.
const TAPER_TIP: f32 = 0.05;
/// Dab spacing of a scattered stroke that has none of its own, as a part of the size.
const SCATTER_SPACING: f32 = 0.25;

impl Shaping {
    /// The values clamped to their ranges (not a number reads as none).
    pub fn sanitized(self) -> Shaping {
        let c = |v: f32, hi: f32| if v.is_finite() { v.clamp(0.0, hi) } else { 0.0 };
        let spacing = c(self.spacing, 10.0);
        Shaping {
            taper_start: c(self.taper_start, 0.5),
            taper_end: c(self.taper_end, 0.5),
            spacing: if spacing > 0.0 { spacing.max(0.05) } else { 0.0 },
            jitter: c(self.jitter, 0.5),
            texture: c(self.texture, 1.0),
            scatter: c(self.scatter, 2.0),
            tilt: self.tilt,
        }
    }

    /// The dab spacing a stroke stamps at (scatter needs dabs).
    fn dab_spacing(&self) -> f32 {
        if self.spacing <= 0.0 && self.scatter > 0.0 { SCATTER_SPACING } else { self.spacing }
    }

    /// The tapers' factor at fraction `u` of the stroke's length.
    pub fn taper(&self, u: f32) -> f32 {
        let ease = |x: f32| {
            let x = x.clamp(0.0, 1.0);
            x * (2.0 - x)
        };
        let mut f: f32 = 1.0;
        if self.taper_start > 1e-4 {
            f = f.min(ease(u / self.taper_start));
        }
        if self.taper_end > 1e-4 {
            f = f.min(ease((1.0 - u) / self.taper_end));
        }
        TAPER_TIP + (1.0 - TAPER_TIP) * f
    }
}

/// The smoothed centre line: points with the tip's size factor at each. `smoothing` is the
/// curve fit's tolerance (stage units); `flatness` how closely the points follow the fitted
/// curves. One distinct sample gives one point (a dab).
pub fn centreline(samples: &[Sample], smoothing: f32, flatness: f32) -> Vec<(Vec2, f32)> {
    centreline_shaped(samples, smoothing, flatness, &Shaping::default())
}

/// The centre line with the shaping's tapers applied to the size factors (and the tilt's
/// widening, when the shaping says tilt shapes the width).
pub fn centreline_shaped(samples: &[Sample], smoothing: f32, flatness: f32, shaping: &Shaping) -> Vec<(Vec2, f32)> {
    centre_points(samples, smoothing, flatness, shaping).into_iter().map(|p| (p.at, p.f)).collect()
}

/// A point of the centre line: where, the tip's size factor, and the pen's lean there (a vector
/// toward the azimuth, zero without tilt).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Pt {
    at: Vec2,
    f: f32,
    lean: Vec2,
}

impl Pt {
    fn lerp(self, o: Pt, t: f32) -> Pt {
        Pt { at: self.at.lerp(o.at, t), f: self.f + (o.f - self.f) * t, lean: self.lean.lerp(o.lean, t) }
    }
}

fn centre_points(samples: &[Sample], smoothing: f32, flatness: f32, shaping: &Shaping) -> Vec<Pt> {
    let shaping = shaping.sanitized();
    let mut pts: Vec<Vec2> = Vec::with_capacity(samples.len().min(MAX_SAMPLES));
    let mut press: Vec<f32> = Vec::with_capacity(pts.capacity());
    let mut leans: Vec<Vec2> = Vec::with_capacity(pts.capacity());
    let lean_of = |s: &Sample| s.tilt.map_or(Vec2::ZERO, Tilt::vector);
    for s in samples.iter().take(MAX_SAMPLES).filter(|s| s.at.is_finite()) {
        if pts.last().is_some_and(|l| l.distance(s.at) <= 1e-4) {
            // A held pointer: keep the latest pressure and tilt.
            if let (Some(p), Some(l)) = (press.last_mut(), leans.last_mut()) {
                *p = size_factor(s.pressure);
                *l = lean_of(s);
            }
            continue;
        }
        pts.push(s.at);
        press.push(size_factor(s.pressure));
        leans.push(lean_of(s));
    }
    // The tilt's widening multiplies the size factor (ADR 0093).
    let widen = |f: f32, lean: Vec2| f * tilted(0.0, lean, TiltResponse { angle: false, width: shaping.tilt.width }).1;
    match (pts.as_slice(), press.as_slice(), leans.as_slice()) {
        ([], _, _) => return Vec::new(),
        ([p], [f, ..], [l, ..]) => return vec![Pt { at: *p, f: widen(*f, *l), lean: *l }],
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
    // A taper needs points along the line to follow it (a straight drag flattens to two).
    let line = if shaping.taper_start > 0.0 || shaping.taper_end > 0.0 { subdivide(&line, 96) } else { line };
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
            let (l0, l1) = (leans.get(j).copied().unwrap_or(Vec2::ZERO), leans.get(j + 1).copied().or(leans.get(j).copied()).unwrap_or(Vec2::ZERO));
            let t = if s1 - s0 > 1e-9 { ((u - s0) / (s1 - s0)).clamp(0.0, 1.0) } else { 0.0 };
            let lean = l0.lerp(l1, t);
            Pt { at: *p, f: widen((f0 + (f1 - f0) * t) * shaping.taper(u), lean), lean }
        })
        .collect()
}

/// The polyline with no piece longer than a `n`th of its length (at most `n` points added).
fn subdivide(line: &[Vec2], n: usize) -> Vec<Vec2> {
    let total: f32 = line.windows(2).map(|w| if let [a, b] = w { a.distance(*b) } else { 0.0 }).sum();
    let piece = total / n.max(1) as f32;
    if !(piece.is_finite() && piece > 1e-6) {
        return line.to_vec();
    }
    let mut out = Vec::with_capacity(line.len() + n + 1);
    out.extend(line.first().copied());
    for w in line.windows(2) {
        let &[a, b] = w else { continue };
        let k = (a.distance(b) / piece).ceil().clamp(1.0, n as f32 + 1.0) as usize;
        out.extend((1..=k).map(|i| a.lerp(b, i as f32 / k as f32)));
    }
    out
}

/// The Paint Brush's line (ADR 0068): the smoothed centre line as an open path, and the tip's
/// size factor along it as `(fraction of length, factor)` at `max` (2..) evenly spaced places.
/// An empty path when the samples don't make a line (one point, or none finite).
pub fn centre_stroke(samples: &[Sample], shaping: &Shaping, smoothing: f32, flatness: f32, max: usize) -> (Path, Vec<(f32, f32)>) {
    let line = centreline_shaped(samples, smoothing, flatness, shaping);
    if line.len() < 2 {
        return (Path::new(), Vec::new());
    }
    let pts: Vec<Vec2> = line.iter().map(|(p, _)| *p).collect();
    let tol = if smoothing.is_finite() && smoothing > 0.01 { smoothing.min(flatness.max(0.05) * 4.0) } else { 0.05 };
    let path = fit_curve(&pts, tol, tol * 2.0, false);
    let mut along = Vec::with_capacity(line.len());
    let mut d = 0.0;
    for w in line.windows(2) {
        if let [a, b] = w {
            along.push(d);
            d += a.0.distance(b.0);
        }
    }
    along.push(d);
    let total = d.max(1e-9);
    let n = max.clamp(2, 1000);
    let mut j = 0;
    let profile = (0..n)
        .map(|i| {
            let u = i as f32 / (n - 1) as f32;
            let at = u * total;
            while j + 2 < along.len() && along.get(j + 1).is_some_and(|a| *a < at) {
                j += 1;
            }
            let (a0, a1) = (along.get(j).copied().unwrap_or(0.0), along.get(j + 1).copied().unwrap_or(total));
            let (f0, f1) = (line.get(j).map_or(1.0, |l| l.1), line.get(j + 1).map_or(1.0, |l| l.1));
            let t = if a1 - a0 > 1e-9 { ((at - a0) / (a1 - a0)).clamp(0.0, 1.0) } else { 0.0 };
            (u, f0 + (f1 - f0) * t)
        })
        .collect();
    (path, profile)
}

/// Dabs every `step` stage units along a centre line (the first point always), with the size
/// factor where each falls. At most [`MAX_DABS`]: a longer line spaces them wider.
pub fn dabs(line: &[(Vec2, f32)], step: f32) -> Vec<(Vec2, f32)> {
    let pts: Vec<Pt> = line.iter().map(|&(at, f)| Pt { at, f, lean: Vec2::ZERO }).collect();
    dab_points(&pts, step).into_iter().map(|(p, _)| (p.at, p.f)).collect()
}

/// [`dabs`] over centre-line points, each with the line's direction there.
fn dab_points(line: &[Pt], step: f32) -> Vec<(Pt, Vec2)> {
    let total: f32 = line.windows(2).map(|w| if let [a, b] = w { a.at.distance(b.at) } else { 0.0 }).sum();
    let dir0 = line.windows(2).find_map(|w| if let [a, b] = w { Some((b.at - a.at).normalized()).filter(|d| d.is_finite() && d.length() > 0.5) } else { None }).unwrap_or(Vec2::new(1.0, 0.0));
    let mut out: Vec<(Pt, Vec2)> = line.first().copied().map(|p| (p, dir0)).into_iter().collect();
    if !(step.is_finite() && step > 1e-3) {
        return out;
    }
    let step = step.max(total / MAX_DABS as f32);
    let mut next = step;
    let mut walked = 0.0;
    for w in line.windows(2) {
        let &[a, b] = w else { continue };
        let l = a.at.distance(b.at);
        let dir = if l > 1e-9 { (b.at - a.at) * (1.0 / l) } else { dir0 };
        while next <= walked + l && out.len() < MAX_DABS {
            let t = if l > 1e-9 { (next - walked) / l } else { 0.0 };
            out.push((a.lerp(b, t), dir));
            next += step;
        }
        walked += l;
    }
    out
}

/// The filled outline of a brush stroke: closed loops (even-odd), or an empty path when there
/// is nothing to paint (no finite samples, a zero-size tip, or a stroke too large to resolve).
/// `smoothing` is the centre line's curve-fit tolerance and `tolerance` the outline's (both in
/// stage units).
pub fn outline(samples: &[Sample], tip: &Tip, smoothing: f32, tolerance: f32) -> Path {
    outline_shaped(samples, tip, &Shaping::default(), smoothing, tolerance)
}

/// The outline with tapers, dabs, a ragged edge, texture, scatter or tilt (ADR 0068, 0093).
pub fn outline_shaped(samples: &[Sample], tip: &Tip, shaping: &Shaping, smoothing: f32, tolerance: f32) -> Path {
    let shaping = shaping.sanitized();
    let tol = if tolerance.is_finite() && tolerance > 1e-3 { tolerance } else { 0.25 };
    let mut line = centre_points(samples, smoothing, tol * 0.25, &shaping);
    let spacing = shaping.dab_spacing();
    let dabbed = spacing > 0.0 && line.len() > 1;
    if dabbed {
        line = scattered(dab_points(&line, spacing * tip.size), shaping.scatter * tip.size);
    }
    if line.is_empty() || !(tip.size.is_finite() && tip.size > 0.0) {
        return Path::new();
    }
    // The tip's angle at each point: the brush's, turned by the pen's azimuth (ADR 0093).
    let angles: Vec<f32> = line.iter().map(|p| tilted(tip.angle, p.lean, TiltResponse { angle: shaping.tilt.angle, width: false }).0).collect();
    let turns = angles.iter().any(|a| (a - tip.angle).abs() > 1e-3);
    let round = if tip.roundness.is_finite() { f64::from(tip.roundness.clamp(0.05, 1.0)) } else { 1.0 };
    let half = if tip.size.is_finite() { tip.size.max(0.0) * 0.5 } else { 0.0 };
    // The grid's space: the tip's own, or (when the tip turns) the stage in tip radii, where
    // each piece measures in its own tip space.
    let m = if turns { Affine::scale(half, half) } else { tip.to_stage() };
    let Some(inv) = m.inverse().filter(|i| i.is_finite()) else { return Path::new() };
    // Grid space, in f64, with each point's tip angle (radians).
    let q: Vec<(P, f64, f64)> = line.iter().zip(&angles).map(|(p, a)| (v64(inv.apply(p.at)), f64::from(p.f), f64::from(*a).to_radians())).collect();
    let (mut lo, mut hi) = ((f64::INFINITY, f64::INFINITY), (f64::NEG_INFINITY, f64::NEG_INFINITY));
    let mut fmin: f64 = 1.0;
    for (p, f, _) in &q {
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
    // A ragged edge needs cells finer than its grain, and the paper grain finer still.
    let fine: f64 = if shaping.jitter > 0.0 { f64::min(fine, 1.0 / 14.0) } else { fine };
    let fine: f64 = if shaping.texture > 0.0 { f64::min(fine, 1.0 / 18.0) } else { fine };
    // In stage space the tip's narrow side is `round` radii across.
    let narrow = if turns { round } else { 1.0 };
    let mut c: f64 = (fmin / 2.5).min(fine) * narrow;
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
    // How deep the ragged edge wears in, in cells (the tip's radius is 1, its size 2); the
    // field inside reaches that much deeper, so the wear never opens holes in the middle.
    let wear = f64::from(shaping.jitter) * 2.0 / c;
    let top = 1.0 + MARGIN + wear;
    // Nothing thinner than a cell, so no part slips between the samples.
    let pts: Vec<(P, f64, f64)> = thin(&q, c * 0.5).into_iter().map(|(p, f, a)| (p, f.max(c / narrow), a)).collect();
    // A piece's frame when the tip turns: grid space to its tip space is a turn by −angle and
    // a stretch of the narrow side. A distance there times `round` is at most the distance in
    // the grid, with the same zero level.
    let mut stamp = |a: P, ra: f64, b: P, rb: f64, turn: Option<(f64, f64)>| {
        let reach = ra.max(rb) + (MARGIN - 1.0) * c;
        let (x0, x1) = (((a.0.min(b.0) - reach - origin.0) / c).floor().max(0.0), ((a.0.max(b.0) + reach - origin.0) / c).ceil());
        let (y0, y1) = (((a.1.min(b.1) - reach - origin.1) / c).floor().max(0.0), ((a.1.max(b.1) + reach - origin.1) / c).ceil());
        let (x1, y1) = (x1.min(w as f64 - 1.0), y1.min(h as f64 - 1.0));
        if x1 < x0 || y1 < y0 {
            return;
        }
        let to_tip = |p: P| match turn {
            Some((cs, sn)) => (p.0 * cs + p.1 * sn, (-p.0 * sn + p.1 * cs) / round),
            None => p,
        };
        let scale = if turn.is_some() { round } else { 1.0 };
        let (ta, tb) = (to_tip(a), to_tip(b));
        for y in y0 as usize..=y1 as usize {
            let py = origin.1 + (y as f64 + 0.5) * c;
            for x in x0 as usize..=x1 as usize {
                let px = origin.0 + (x as f64 + 0.5) * c;
                let tx = to_tip((px, py));
                let d = match tip.shape {
                    TipShape::Round => round_dist(tx, ta, ra, tb, rb),
                    TipShape::Square => square_dist(tx, ta, ra, tb, rb),
                } * scale;
                let v = (0.5 - d / c).clamp(OUTSIDE, top) as f32;
                if let Some(cell) = field.get_mut(y * w + x)
                    && v > *cell
                {
                    *cell = v;
                }
            }
        }
    };
    let turn_at = |a: f64| turns.then(|| (a.cos(), a.sin()));
    match pts.as_slice() {
        [(p, f, a)] => stamp(*p, *f, *p, *f, turn_at(*a)),
        _ if dabbed => pts.iter().for_each(|(p, f, a)| stamp(*p, *f, *p, *f, turn_at(*a))),
        _ => {
            for pair in pts.windows(2) {
                let &[(a, ra, aa), (b, rb, ab)] = pair else { continue };
                // Long segments in short pieces, so each one's bounds stay small (the pieces'
                // sweeps are the segment's); a turning tip turns at most 3° a piece.
                let n = (dist(a, b) / PIECE.max(16.0 * c)).ceil().clamp(1.0, 1e6);
                let da = half_turn(ab - aa);
                let n = if turns { n.max((da.abs() / 3f64.to_radians()).ceil()).min(1e6) } else { n } as usize;
                for i in 0..n {
                    let (t0, t1) = (i as f64 / n as f64, (i + 1) as f64 / n as f64);
                    let mid = aa + da * (t0 + t1) * 0.5;
                    stamp(lerp(a, b, t0), ra + (rb - ra) * t0, lerp(a, b, t1), ra + (rb - ra) * t1, turn_at(mid));
                }
            }
        }
    }
    if wear > 0.0 {
        // Wear the edge by a fixed noise pattern in tip space (so the grain stays put however
        // the stroke runs): the field is a distance, so lowering it by `wear × n` moves the
        // edge in by that much.
        for (i, v) in field.iter_mut().enumerate() {
            if *v <= OUTSIDE as f32 {
                continue;
            }
            let (x, y) = ((i % w) as f64, (i / w) as f64);
            *v -= (wear * grain((origin.0 + (x + 0.5) * c, origin.1 + (y + 0.5) * c))) as f32;
        }
    }
    if shaping.texture > 0.0 {
        // The paper grain: where a fixed noise on the stage (in tip radii, so it scales with
        // the brush) falls below the threshold, the paint has a hole. The noise's slope is about
        // one per grain, so `(n − τ) × grain` is roughly a distance, which is all the tracing's
        // interpolation needs.
        let tau = 0.2 + 0.4 * f64::from(shaping.texture);
        let unit = 1.0 / f64::from(half).max(1e-9);
        let grain_cells = PAPER / c;
        for (i, v) in field.iter_mut().enumerate() {
            if *v <= 0.0 {
                continue;
            }
            let (x, y) = ((i % w) as f64, (i / w) as f64);
            let st = m.apply(Vec2::new((origin.0 + (x + 0.5) * c) as f32, (origin.1 + (y + 0.5) * c) as f32));
            let n = paper((f64::from(st.x) * unit, f64::from(st.y) * unit));
            let hole = ((n - tau) * grain_cells) as f32;
            if hole < *v {
                *v = hole;
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
    for lp in trace::contours(&field, w, h).into_iter().take(MAX_LOOPS) {
        let stage: Vec<Vec2> = lp.into_iter().map(to_stage).collect();
        let part = fit_curve(&stage, tol, window, true);
        if part.cmds.iter().any(|c| matches!(c, PathCmd::CubicTo(..) | PathCmd::LineTo(..))) {
            out.cmds.extend(part.cmds);
        }
    }
    out
}

/// Most loops one outline keeps (a fine paper grain on a long stroke opens many holes).
pub const MAX_LOOPS: usize = 20_000;
/// The paper grain's size, in tip radii.
const PAPER: f64 = 0.35;

/// An angle difference (radians) as the shortest turn of a tip that looks the same turned
/// half way round: in `-π/2..=π/2`.
fn half_turn(d: f64) -> f64 {
    let pi = std::f64::consts::PI;
    let d = d.rem_euclid(pi);
    if d > pi * 0.5 { d - pi } else { d }
}

/// Dabs thrown up to `reach` either side of the line, and varied from 60 to 100 % in size, by
/// a fixed hash of each dab's number (so the same stroke scatters the same way).
fn scattered(dabs: Vec<(Pt, Vec2)>, reach: f32) -> Vec<Pt> {
    if !(reach.is_finite() && reach > 0.0) {
        return dabs.into_iter().map(|(p, _)| p).collect();
    }
    dabs.into_iter()
        .enumerate()
        .map(|(i, (p, dir))| {
            let (u, v) = (unit_hash(i as u64, 1), unit_hash(i as u64, 2));
            let side = Vec2::new(-dir.y, dir.x) * ((u as f32 * 2.0 - 1.0) * reach);
            Pt { at: p.at + side, f: p.f * (0.6 + 0.4 * v as f32), lean: p.lean }
        })
        .collect()
}

/// A number in `0..1` from `(i, k)`, the same on every platform (SplitMix64's finaliser).
fn unit_hash(i: u64, k: u64) -> f64 {
    let mut z = i.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ k.wrapping_mul(0xD1B5_4A32_D192_ED03);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 11) as f64 / (1u64 << 53) as f64
}

/// The paper grain: value noise in `0..1`, two octaves ([`PAPER`] and 0.4 of it), on lattice
/// rows of its own (apart from the ragged edge's).
fn paper(p: P) -> f64 {
    grain_octaves(p, PAPER, PAPER * 0.4, 7)
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

/// Smooth value noise in `0..1` from a fixed integer hash: two octaves, 0.5 and 0.2 tip units
/// across. The same point always gives the same value, on every platform.
fn grain(p: P) -> f64 {
    grain_octaves(p, 0.5, 0.2, 0)
}

/// Two octaves of value noise (`big` and `small` across, the big one weighted twice) on lattice
/// rows from `layer`: each octave reads rows of its own (`layer` shifts them far apart), so the
/// octaves are independent.
fn grain_octaves(p: P, big: f64, small: f64, layer: i64) -> f64 {
    fn hash(x: i64, y: i64) -> f64 {
        // SplitMix64's finaliser over the lattice point.
        let mut z = (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 11) as f64 / (1u64 << 53) as f64
    }
    fn octave(p: P, size: f64, layer: i64) -> f64 {
        let (u, v) = (p.0 / size, p.1 / size);
        let (x0, y0) = (u.floor(), v.floor());
        let s = |t: f64| t * t * (3.0 - 2.0 * t);
        let (sx, sy) = (s(u - x0), s(v - y0));
        let (xi, yi) = (x0.clamp(-1e15, 1e15) as i64, (y0.clamp(-1e15, 1e15) as i64).wrapping_add(layer << 52));
        let top = hash(xi, yi) + (hash(xi + 1, yi) - hash(xi, yi)) * sx;
        let bottom = hash(xi, yi + 1) + (hash(xi + 1, yi + 1) - hash(xi, yi + 1)) * sx;
        top + (bottom - top) * sy
    }
    (octave(p, big, layer) * 2.0 + octave(p, small, layer + 1)) / 3.0
}

/// Drops points closer than `gap` to the last kept one (the last point always stays).
fn thin(q: &[(P, f64, f64)], gap: f64) -> Vec<(P, f64, f64)> {
    let mut out: Vec<(P, f64, f64)> = Vec::with_capacity(q.len());
    for (i, &(p, f, a)) in q.iter().enumerate() {
        let last = i + 1 == q.len();
        match out.last_mut() {
            Some((l, lf, la)) if dist(*l, p) < gap => {
                if last {
                    (*l, *lf, *la) = (p, f, a);
                }
            }
            _ => out.push((p, f, a)),
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
        (0..=n).map(|i| i as f32 / n as f32).map(|t| Sample::new(a.lerp(b, t), pressure(t))).collect()
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
        let p = outline(&[Sample::new(Vec2::new(10.0, 10.0), 1.0)], &Tip::round(8.0), 1.0, 0.05);
        let want = std::f32::consts::PI * 16.0;
        assert!((area(&p) - want).abs() / want < 0.01, "{}", area(&p));
        // A square tip at 45°: a diamond of the same area as the square.
        let tip = Tip { shape: TipShape::Square, size: 10.0, angle: 45.0, roundness: 1.0 };
        let d = outline(&[Sample::new(Vec2::ZERO, 1.0)], &tip, 1.0, 0.05);
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
        let s: Vec<Sample> = (0..=200).map(|i| Sample::new(Vec2::new(i as f32 * 2.0, ((i * 7919) % 13) as f32 * 0.35), 1.0)).collect();
        let rough = outline(&s, &Tip::round(12.0), 0.3, 0.3);
        let smooth = outline(&s, &Tip::round(12.0), 6.0, 0.3);
        let (a, b) = (rough.anchors().len(), smooth.anchors().len());
        assert!(b * 2 < a, "smoothing should cut anchors: {a} rough, {b} smooth");
    }

    #[test]
    fn crossing_strokes_leave_holes_and_ellipses_follow_the_angle() {
        // A loop that closes on itself leaves a hole in the middle.
        let ring: Vec<Sample> = (0..=72).map(|i| i as f32 / 72.0 * std::f32::consts::TAU).map(|a| Sample::new(Vec2::new(100.0 * a.cos(), 100.0 * a.sin()), 1.0)).collect();
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

    fn loops(p: &Path) -> usize {
        p.cmds.iter().filter(|c| matches!(c, PathCmd::MoveTo(_))).count()
    }

    #[test]
    fn tapers_come_to_points_and_keep_the_middle() {
        let s = line(Vec2::ZERO, Vec2::new(300.0, 0.0), 60, |_| 1.0);
        let sh = Shaping { taper_start: 0.3, taper_end: 0.3, ..Shaping::default() };
        let p = outline_shaped(&s, &Tip::round(20.0), &sh, 1.0, 0.1);
        let inside = |x: f32, y: f32| p.contains(Vec2::new(x, y), &Affine::IDENTITY, FillRule::EvenOdd);
        assert!(inside(150.0, 9.0), "full width in the middle: {:?} {}", p.bounds(), loops(&p));
        assert!(inside(5.0, 0.0) && !inside(5.0, 3.0), "pointed at the start");
        assert!(!inside(295.0, 3.0), "pointed at the end");
        // Area: less than the plain stroke, more than its middle part alone.
        let plain = area(&outline(&s, &Tip::round(20.0), 1.0, 0.1));
        let a = area(&p);
        assert!(a < plain * 0.85 && a > 0.4 * 300.0 * 20.0, "{a} vs {plain}");
        assert!((sh.taper(0.5) - 1.0).abs() < 1e-6 && sh.taper(0.0) < 0.06 && sh.taper(1.0) < 0.06);
    }

    #[test]
    fn spacing_stamps_separate_dabs() {
        let s = line(Vec2::ZERO, Vec2::new(200.0, 0.0), 40, |_| 1.0);
        // Dabs two sizes apart: 200 / 20 + 1 = 11 separate dots of the tip's area.
        let sh = Shaping { spacing: 2.0, ..Shaping::default() };
        let p = outline_shaped(&s, &Tip::round(10.0), &sh, 0.5, 0.1);
        assert_eq!(loops(&p), 11);
        let want = 11.0 * std::f32::consts::PI * 25.0;
        assert!((area(&p) - want).abs() / want < 0.02, "{} vs {want}", area(&p));
        // Closer than the tip's size, they join into one stroke.
        let joined = outline_shaped(&s, &Tip::round(10.0), &Shaping { spacing: 0.5, ..Shaping::default() }, 0.5, 0.1);
        assert_eq!(loops(&joined), 1);
        // A dab budget: a huge stroke at the smallest spacing still stamps a bounded number.
        let long = dabs(&[(Vec2::ZERO, 1.0), (Vec2::new(1e6, 0.0), 1.0)], 0.01);
        assert!(long.len() <= MAX_DABS);
    }

    #[test]
    fn a_ragged_edge_wears_in_and_is_the_same_every_time() {
        let s = line(Vec2::ZERO, Vec2::new(300.0, 40.0), 60, |t| 0.6 + 0.4 * t);
        let sh = Shaping { jitter: 0.2, ..Shaping::default() };
        let tip = Tip::round(24.0);
        let p = outline_shaped(&s, &tip, &sh, 1.0, 0.2);
        let plain = outline(&s, &tip, 1.0, 0.2);
        let (a, b) = (area(&p), area(&plain));
        // It wears in by up to a fifth of the size on each side: within the plain stroke,
        // never less than what is left after the deepest wear.
        assert!(a < b * 0.97 && a > b * 0.55, "{a} vs {b}");
        assert!(p.bounds().width() <= plain.bounds().width() + 0.5);
        assert!(p.anchors().len() > plain.anchors().len() * 2, "ragged: {} vs {}", p.anchors().len(), plain.anchors().len());
        assert_eq!(p, outline_shaped(&s, &tip, &sh, 1.0, 0.2), "deterministic");
    }

    #[test]
    fn the_paint_brush_line_has_a_profile() {
        let s = line(Vec2::ZERO, Vec2::new(200.0, 0.0), 40, |t| t);
        let (path, prof) = centre_stroke(&s, &Shaping::default(), 1.0, 0.1, 16);
        assert!(!path.is_empty() && prof.len() == 16);
        assert_eq!(prof.first().map(|p| p.0), Some(0.0));
        assert_eq!(prof.last().map(|p| p.0), Some(1.0));
        // Pressure from 0 to 1: factors from the floor to full.
        assert!(prof.first().is_some_and(|p| (p.1 - MIN_PRESSURE_SIZE).abs() < 0.03));
        assert!(prof.last().is_some_and(|p| p.1 > 0.97));
        assert!(prof.windows(2).all(|w| w[1].1 >= w[0].1 - 1e-4), "rising");
        let b = path.bounds();
        assert!((b.width() - 200.0).abs() < 1.0 && b.height() < 1.0);
        assert!(centre_stroke(&s[..1], &Shaping::default(), 1.0, 0.1, 16).0.is_empty(), "a dab is no line");
    }

    /// The bounds of the curve itself (`Path::bounds` includes control points).
    trait Tight {
        fn tight(&self) -> crate::Rect;
    }
    impl Tight for Path {
        fn tight(&self) -> crate::Rect {
            let mut r = crate::Rect::EMPTY;
            for pl in self.flatten(&Affine::IDENTITY, 0.05) {
                pl.points.into_iter().for_each(|x| r.include(x));
            }
            r
        }
    }

    fn tilted_line(n: usize, len: f32, tilt: impl Fn(f32) -> Tilt) -> Vec<Sample> {
        (0..=n).map(|i| i as f32 / n as f32).map(|t| Sample { at: Vec2::new(len * t, 0.0), pressure: 1.0, tilt: Some(tilt(t)) }).collect()
    }

    #[test]
    fn tilt_maps_to_angle_and_width_analytically() {
        let both = TiltResponse { angle: true, width: true };
        let lean = |alt: f32, az: f32| Tilt { altitude: alt, azimuth: az }.vector();
        // Upright: the brush as set.
        assert_eq!(tilted(20.0, lean(90.0, 75.0), both), (20.0, 1.0));
        // Lying flat toward 30°: turned by 30°, twice as wide.
        let (a, f) = tilted(20.0, lean(0.0, 30.0), both);
        assert!((a - 50.0).abs() < 1e-3 && (f - 2.0).abs() < 1e-5, "{a} {f}");
        // Half way down (45°): half the lean, 1.5 × as wide; azimuth 270 is −90.
        let (a, f) = tilted(0.0, lean(45.0, 270.0), both);
        assert!((a + 90.0).abs() < 1e-3 && (f - 1.5).abs() < 1e-5, "{a} {f}");
        // Each part only when asked for.
        assert_eq!(tilted(10.0, lean(0.0, 30.0), TiltResponse::default()), (10.0, 1.0));
        assert!((tilted(10.0, lean(0.0, 30.0), TiltResponse { angle: false, width: true }).1 - 2.0).abs() < 1e-5);
        // Hostile tilts read as upright.
        assert_eq!(Tilt { altitude: f32::NAN, azimuth: f32::INFINITY }.lean(), 0.0);
        assert_eq!(tilted(5.0, Vec2::new(f32::NAN, 1.0), both), (5.0, 1.0));
    }

    #[test]
    fn a_tilted_flat_tip_turns_with_the_pen() {
        let flat = Tip { shape: TipShape::Round, size: 20.0, angle: 0.0, roundness: 0.2 };
        let turn = Shaping { tilt: TiltResponse { angle: true, width: false }, ..Shaping::default() };
        // Leaning toward 90° all along: the nib stands across the drag, as a 90° brush does.
        let across = outline_shaped(&tilted_line(10, 100.0, |_| Tilt { altitude: 30.0, azimuth: 90.0 }), &flat, &turn, 1.0, 0.1);
        let set = outline(&line(Vec2::ZERO, Vec2::new(100.0, 0.0), 10, |_| 1.0), &Tip { angle: 90.0, ..flat }, 1.0, 0.1);
        assert!((across.tight().height() - 20.0).abs() < 0.6, "{:?}", across.tight());
        assert!((area(&across) - area(&set)).abs() / area(&set) < 0.03, "{} vs {}", area(&across), area(&set));
        // Without the tilt option the pen's lean does nothing.
        let plain = outline_shaped(&tilted_line(10, 100.0, |_| Tilt { altitude: 30.0, azimuth: 90.0 }), &flat, &Shaping::default(), 1.0, 0.1);
        assert!((plain.tight().height() - 4.0).abs() < 0.6, "{:?}", plain.tight());
        // Turning from along the drag (thin) to across it (wide) as it goes.
        let s = tilted_line(60, 300.0, |t| Tilt { altitude: 20.0, azimuth: 90.0 * t });
        let p = outline_shaped(&s, &flat, &turn, 1.0, 0.1);
        let inside = |x: f32, y: f32| p.contains(Vec2::new(x, y), &Affine::IDENTITY, FillRule::EvenOdd);
        assert!(inside(15.0, 0.0) && !inside(15.0, 4.0), "thin at the start");
        assert!(inside(290.0, 9.0), "wide at the end");
        assert_eq!(loops(&p), 1);
        // Widening: a round 10 tip flat on the paper paints 20 wide.
        let wide = Shaping { tilt: TiltResponse { angle: false, width: true }, ..Shaping::default() };
        let w = outline_shaped(&tilted_line(10, 100.0, |_| Tilt { altitude: 0.0, azimuth: 0.0 }), &Tip::round(10.0), &wide, 1.0, 0.1);
        assert!((w.tight().height() - 20.0).abs() < 0.5, "{:?}", w.tight());
        assert_eq!(w, outline_shaped(&tilted_line(10, 100.0, |_| Tilt { altitude: 0.0, azimuth: 0.0 }), &Tip::round(10.0), &wide, 1.0, 0.1), "deterministic");
    }

    #[test]
    fn texture_opens_a_fixed_paper_grain_and_scatter_throws_dabs() {
        let s = line(Vec2::ZERO, Vec2::new(300.0, 30.0), 60, |_| 1.0);
        let tip = Tip::round(30.0);
        let plain = outline(&s, &tip, 1.0, 0.2);
        let sh = Shaping { texture: 0.5, ..Shaping::default() };
        let p = outline_shaped(&s, &tip, &sh, 1.0, 0.2);
        let (a, b) = (area(&p), area(&plain));
        assert!(a < b * 0.9 && a > b * 0.25, "grain takes some of the paint: {a} vs {b}");
        assert!(loops(&p) > 10, "holes through the paint: {}", loops(&p));
        assert!(p.tight().width() <= plain.tight().width() + 0.5 && p.tight().height() <= plain.tight().height() + 0.5);
        assert_eq!(p, outline_shaped(&s, &tip, &sh, 1.0, 0.2), "deterministic");
        // More texture, less paint.
        let more = area(&outline_shaped(&s, &tip, &Shaping { texture: 1.0, ..Shaping::default() }, 1.0, 0.2));
        assert!(more < a, "{more} vs {a}");
        // Scatter: dabs either side of the line, past the plain stroke's edge.
        let flat = line(Vec2::ZERO, Vec2::new(300.0, 0.0), 60, |_| 1.0);
        let sc = Shaping { scatter: 1.5, spacing: 0.8, ..Shaping::default() };
        let q = outline_shaped(&flat, &Tip::round(10.0), &sc, 1.0, 0.2);
        assert!(q.tight().height() > 20.0, "{:?}", q.tight());
        assert!(q.tight().height() < 10.0 + 2.0 * 15.0 + 1.0, "within the scatter's reach: {:?}", q.tight());
        assert!(loops(&q) > 5);
        assert_eq!(q, outline_shaped(&flat, &Tip::round(10.0), &sc, 1.0, 0.2), "deterministic");
        // Scatter on a continuous brush stamps dabs a quarter size apart.
        let cont = outline_shaped(&flat, &Tip::round(10.0), &Shaping { scatter: 1.0, ..Shaping::default() }, 1.0, 0.2);
        assert!(cont.tight().height() > 12.0);
    }

    #[test]
    fn hostile_input_is_empty_or_bounded() {
        let wild = Shaping { taper_start: f32::NAN, taper_end: 1e9, spacing: -3.0, jitter: f32::INFINITY, texture: f32::NAN, scatter: 1e9, tilt: TiltResponse::default() };
        assert_eq!(wild.sanitized(), Shaping { taper_start: 0.0, taper_end: 0.5, spacing: 0.0, jitter: 0.0, texture: 0.0, scatter: 2.0, tilt: TiltResponse::default() });
        let _ = outline_shaped(&line(Vec2::ZERO, Vec2::new(10.0, 0.0), 3, |_| 1.0), &Tip::round(5.0), &Shaping { spacing: 1e-9, jitter: 0.5, ..wild }, 1.0, 1.0);
        assert!(outline(&[], &Tip::round(10.0), 1.0, 1.0).is_empty());
        let nan = [Sample::new(Vec2::new(f32::NAN, 0.0), f32::NAN)];
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
