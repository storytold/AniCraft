//! Bézier paths, adaptive flattening (Wang's formula) and winding-number hit testing.

use crate::{Affine, Rect, Vec2};
use serde::{Deserialize, Serialize};

/// Fill rule for closed regions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FillRule {
    #[default]
    NonZero,
    EvenOdd,
}

/// One path command. Coordinates are in the element's local space.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum PathCmd {
    MoveTo(Vec2),
    LineTo(Vec2),
    QuadTo(Vec2, Vec2),
    CubicTo(Vec2, Vec2, Vec2),
    Close,
}

/// A sequence of subpaths.
///
/// Serialized as compact SVG path data (`"M0 0L10 0Q5 5 0 10Z"`, absolute commands only), which
/// is about four times smaller than a tagged JSON list and readable by people and tools. The
/// legacy `{"cmds": [...]}` form is still accepted when reading.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Path {
    pub cmds: Vec<PathCmd>,
}

/// Largest path accepted from a file.
pub const MAX_PATH_CMDS: usize = 4_000_000;

impl Path {
    /// SVG path data with shortest round-trip number formatting.
    pub fn to_svg(&self) -> String {
        use std::fmt::Write;
        let mut s = String::with_capacity(self.cmds.len() * 16);
        let pt = |s: &mut String, p: Vec2| {
            let _ = write!(s, "{} {}", p.x, p.y);
        };
        for c in &self.cmds {
            match *c {
                PathCmd::MoveTo(p) => {
                    s.push('M');
                    pt(&mut s, p);
                }
                PathCmd::LineTo(p) => {
                    s.push('L');
                    pt(&mut s, p);
                }
                PathCmd::QuadTo(a, p) => {
                    s.push('Q');
                    pt(&mut s, a);
                    s.push(' ');
                    pt(&mut s, p);
                }
                PathCmd::CubicTo(a, b, p) => {
                    s.push('C');
                    pt(&mut s, a);
                    s.push(' ');
                    pt(&mut s, b);
                    s.push(' ');
                    pt(&mut s, p);
                }
                PathCmd::Close => s.push('Z'),
            }
        }
        s
    }

    /// Parses the absolute-command subset written by [`Path::to_svg`] (M, L, Q, C, Z; numbers
    /// separated by spaces or commas). Never panics; rejects non-finite numbers.
    pub fn from_svg(src: &str) -> Result<Path, String> {
        let mut cmds = Vec::new();
        let mut it = src.char_indices().peekable();
        let mut nums: Vec<f32> = Vec::with_capacity(6);
        let mut cur: Option<char> = None;
        let flush = |cur: Option<char>, nums: &mut Vec<f32>, cmds: &mut Vec<PathCmd>| -> Result<(), String> {
            let Some(c) = cur else {
                return if nums.is_empty() { Ok(()) } else { Err("number before the first command".into()) };
            };
            let need = match c {
                'M' | 'L' => 2,
                'Q' => 4,
                'C' => 6,
                _ => 0,
            };
            if c == 'Z' {
                if !nums.is_empty() {
                    return Err("Z takes no numbers".into());
                }
                cmds.push(PathCmd::Close);
                return Ok(());
            }
            if nums.is_empty() || !nums.len().is_multiple_of(need) {
                return Err(format!("{c} needs a multiple of {need} numbers, got {}", nums.len()));
            }
            for g in nums.chunks(need) {
                let v = |i: usize| Vec2::new(g[i], g[i + 1]);
                cmds.push(match c {
                    'M' => PathCmd::MoveTo(v(0)),
                    'L' => PathCmd::LineTo(v(0)),
                    'Q' => PathCmd::QuadTo(v(0), v(2)),
                    _ => PathCmd::CubicTo(v(0), v(2), v(4)),
                });
            }
            nums.clear();
            if cmds.len() > MAX_PATH_CMDS {
                return Err(format!("path has more than {MAX_PATH_CMDS} commands"));
            }
            Ok(())
        };
        while let Some(&(i, ch)) = it.peek() {
            match ch {
                'M' | 'L' | 'Q' | 'C' | 'Z' | 'z' => {
                    flush(cur, &mut nums, &mut cmds)?;
                    cur = Some(ch.to_ascii_uppercase());
                    it.next();
                }
                ' ' | ',' | '\n' | '\t' | '\r' => {
                    it.next();
                }
                '-' | '+' | '.' | '0'..='9' => {
                    let start = i;
                    let mut end = i + ch.len_utf8();
                    it.next();
                    while let Some(&(j, c2)) = it.peek() {
                        let exp_sign = (c2 == '-' || c2 == '+') && matches!(src.get(..j).and_then(|s| s.chars().last()), Some('e' | 'E'));
                        if c2.is_ascii_digit() || c2 == '.' || c2 == 'e' || c2 == 'E' || exp_sign {
                            end = j + c2.len_utf8();
                            it.next();
                        } else {
                            break;
                        }
                    }
                    let tok = src.get(start..end).unwrap_or("");
                    let v: f32 = tok.parse().map_err(|_| format!("bad number '{tok}'"))?;
                    if !v.is_finite() {
                        return Err(format!("number '{tok}' is not finite"));
                    }
                    nums.push(v);
                    if nums.len() > 6 {
                        // repeated implicit commands: emit as we go to bound memory
                        let c = cur.ok_or("number before the first command")?;
                        let need = match c {
                            'M' | 'L' => 2,
                            'Q' => 4,
                            'C' => 6,
                            _ => return Err("Z takes no numbers".into()),
                        };
                        if nums.len().is_multiple_of(need) {
                            flush(cur, &mut nums, &mut cmds)?;
                        }
                    }
                }
                other => return Err(format!("unexpected character '{other}' in path data")),
            }
        }
        flush(cur, &mut nums, &mut cmds)?;
        Ok(Path { cmds })
    }
}

impl Serialize for Path {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_svg())
    }
}

impl<'de> Deserialize<'de> for Path {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Path, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Svg(String),
            Legacy { cmds: Vec<PathCmd> },
        }
        match Repr::deserialize(d)? {
            Repr::Svg(s) => Path::from_svg(&s).map_err(serde::de::Error::custom),
            Repr::Legacy { cmds } => {
                if cmds.len() > MAX_PATH_CMDS {
                    return Err(serde::de::Error::custom("path is too long"));
                }
                Ok(Path { cmds })
            }
        }
    }
}

/// A flattened subpath.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Polyline {
    pub points: Vec<Vec2>,
    pub closed: bool,
}

/// Hard cap on segments per curve, so a hostile curve cannot allocate unboundedly.
const MAX_SEGMENTS_PER_CURVE: usize = 1024;
/// Bézier circle constant.
const KAPPA: f32 = 0.552_284_8;

/// Most corners a PolyStar has on each side of its outline (sides or points).
pub const POLYSTAR_MAX_SIDES: u32 = 256;

/// The inner radius of a star of `points` points for a point size from 0 to 1 (ADR 0080): at 1
/// the inner corners lie on the edges of the polygon through the tips (the star is that polygon),
/// at 0 they meet in the centre (needle points), and in between the depth is proportional. Out
/// of range or not a number, the point size is clamped (or 0.5).
pub fn star_inner_radius(outer: f32, points: u32, point_size: f32) -> f32 {
    let n = points.clamp(3, POLYSTAR_MAX_SIDES) as f32;
    let size = if point_size.is_finite() { point_size.clamp(0.0, 1.0) } else { 0.5 };
    outer * size * (std::f32::consts::PI / n).cos()
}

/// The corners of [`Path::polystar`], in order (clockwise on screen, y down), the first straight
/// up: `sides` of them for a polygon, `2 × sides` (tip, inner, tip, …) for a star. `sides` is
/// clamped to 3..=[`POLYSTAR_MAX_SIDES`].
pub fn polystar_corners(cx: f32, cy: f32, radius: f32, sides: u32, star: bool, point_size: f32) -> Vec<Vec2> {
    let n = sides.clamp(3, POLYSTAR_MAX_SIDES) as usize;
    let inner = star_inner_radius(radius, sides, point_size);
    let steps = if star { n * 2 } else { n };
    (0..steps)
        .map(|i| {
            let r = if star && i % 2 == 1 { inner } else { radius };
            // f64 so the corners of a big polygon stay where they belong.
            let a = -std::f64::consts::FRAC_PI_2 + i as f64 * std::f64::consts::TAU / steps as f64;
            Vec2::new(cx + r * a.cos() as f32, cy + r * a.sin() as f32)
        })
        .collect()
}

impl Path {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn move_to(&mut self, p: Vec2) -> &mut Self {
        self.cmds.push(PathCmd::MoveTo(p));
        self
    }
    pub fn line_to(&mut self, p: Vec2) -> &mut Self {
        self.cmds.push(PathCmd::LineTo(p));
        self
    }
    pub fn quad_to(&mut self, c: Vec2, p: Vec2) -> &mut Self {
        self.cmds.push(PathCmd::QuadTo(c, p));
        self
    }
    pub fn cubic_to(&mut self, c1: Vec2, c2: Vec2, p: Vec2) -> &mut Self {
        self.cmds.push(PathCmd::CubicTo(c1, c2, p));
        self
    }
    pub fn close(&mut self) -> &mut Self {
        self.cmds.push(PathCmd::Close);
        self
    }

    /// Axis-aligned rectangle, clockwise on a y-down screen.
    pub fn rect(x: f32, y: f32, w: f32, h: f32) -> Path {
        let mut p = Path::new();
        p.move_to(Vec2::new(x, y))
            .line_to(Vec2::new(x + w, y))
            .line_to(Vec2::new(x + w, y + h))
            .line_to(Vec2::new(x, y + h))
            .close();
        p
    }

    /// Rectangle with rounded corners (radius clamped to half the shorter side).
    pub fn rounded_rect(x: f32, y: f32, w: f32, h: f32, r: f32) -> Path {
        let r = r.max(0.0).min(w.abs() * 0.5).min(h.abs() * 0.5);
        if r <= 0.0 {
            return Path::rect(x, y, w, h);
        }
        let k = r * (1.0 - KAPPA);
        let (x1, y1) = (x + w, y + h);
        let mut p = Path::new();
        p.move_to(Vec2::new(x + r, y))
            .line_to(Vec2::new(x1 - r, y))
            .cubic_to(Vec2::new(x1 - k, y), Vec2::new(x1, y + k), Vec2::new(x1, y + r))
            .line_to(Vec2::new(x1, y1 - r))
            .cubic_to(Vec2::new(x1, y1 - k), Vec2::new(x1 - k, y1), Vec2::new(x1 - r, y1))
            .line_to(Vec2::new(x + r, y1))
            .cubic_to(Vec2::new(x + k, y1), Vec2::new(x, y1 - k), Vec2::new(x, y1 - r))
            .line_to(Vec2::new(x, y + r))
            .cubic_to(Vec2::new(x, y + k), Vec2::new(x + k, y), Vec2::new(x + r, y))
            .close();
        p
    }

    /// Ellipse centred at `(cx, cy)` built from four cubic arcs.
    pub fn ellipse(cx: f32, cy: f32, rx: f32, ry: f32) -> Path {
        let (kx, ky) = (rx * KAPPA, ry * KAPPA);
        let mut p = Path::new();
        p.move_to(Vec2::new(cx + rx, cy))
            .cubic_to(Vec2::new(cx + rx, cy + ky), Vec2::new(cx + kx, cy + ry), Vec2::new(cx, cy + ry))
            .cubic_to(Vec2::new(cx - kx, cy + ry), Vec2::new(cx - rx, cy + ky), Vec2::new(cx - rx, cy))
            .cubic_to(Vec2::new(cx - rx, cy - ky), Vec2::new(cx - kx, cy - ry), Vec2::new(cx, cy - ry))
            .cubic_to(Vec2::new(cx + kx, cy - ry), Vec2::new(cx + rx, cy - ky), Vec2::new(cx + rx, cy))
            .close();
        p
    }

    /// The PolyStar tool's outline (ADR 0080): a regular polygon of `sides` corners, or a star of
    /// `sides` points whose inner corners are at [`star_inner_radius`] for `point_size`, centred
    /// at `(cx, cy)` with the first corner straight up.
    pub fn polystar(cx: f32, cy: f32, radius: f32, sides: u32, star: bool, point_size: f32) -> Path {
        let mut p = Path::new();
        for (i, v) in polystar_corners(cx, cy, radius, sides, star, point_size).into_iter().enumerate() {
            if i == 0 {
                p.move_to(v);
            } else {
                p.line_to(v);
            }
        }
        p.close();
        p
    }

    /// Star (or polygon when `inner == outer`) with `points` tips, first tip pointing up.
    pub fn star(cx: f32, cy: f32, outer: f32, inner: f32, points: u32) -> Path {
        let n = points.clamp(3, 256) as usize;
        let mut p = Path::new();
        for i in 0..n * 2 {
            let r = if i % 2 == 0 { outer } else { inner };
            let a = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / n as f32;
            let v = Vec2::new(cx + r * a.cos(), cy + r * a.sin());
            if i == 0 {
                p.move_to(v);
            } else {
                p.line_to(v);
            }
        }
        p.close();
        p
    }

    /// Open two-point line.
    pub fn line(a: Vec2, b: Vec2) -> Path {
        let mut p = Path::new();
        p.move_to(a).line_to(b);
        p
    }

    pub fn is_empty(&self) -> bool {
        self.cmds.is_empty()
    }

    /// Control-point bounds (a superset of the exact curve bounds).
    pub fn bounds(&self) -> Rect {
        let mut r = Rect::EMPTY;
        for c in &self.cmds {
            match *c {
                PathCmd::MoveTo(p) | PathCmd::LineTo(p) => r.include(p),
                PathCmd::QuadTo(a, p) => {
                    r.include(a);
                    r.include(p);
                }
                PathCmd::CubicTo(a, b, p) => {
                    r.include(a);
                    r.include(b);
                    r.include(p);
                }
                PathCmd::Close => {}
            }
        }
        r
    }

    /// Path with every point transformed.
    pub fn transformed(&self, m: &Affine) -> Path {
        let t = |p: Vec2| m.apply(p);
        Path {
            cmds: self
                .cmds
                .iter()
                .map(|c| match *c {
                    PathCmd::MoveTo(p) => PathCmd::MoveTo(t(p)),
                    PathCmd::LineTo(p) => PathCmd::LineTo(t(p)),
                    PathCmd::QuadTo(a, p) => PathCmd::QuadTo(t(a), t(p)),
                    PathCmd::CubicTo(a, b, p) => PathCmd::CubicTo(t(a), t(b), t(p)),
                    PathCmd::Close => PathCmd::Close,
                })
                .collect(),
        }
    }

    /// Point-wise interpolation for shape tweens. `None` when the command structures differ.
    pub fn lerp(&self, o: &Path, t: f32) -> Option<Path> {
        if self.cmds.len() != o.cmds.len() {
            return None;
        }
        let l = |a: Vec2, b: Vec2| a.lerp(b, t);
        let mut cmds = Vec::with_capacity(self.cmds.len());
        for (a, b) in self.cmds.iter().zip(&o.cmds) {
            cmds.push(match (*a, *b) {
                (PathCmd::MoveTo(p), PathCmd::MoveTo(q)) => PathCmd::MoveTo(l(p, q)),
                (PathCmd::LineTo(p), PathCmd::LineTo(q)) => PathCmd::LineTo(l(p, q)),
                (PathCmd::QuadTo(p1, p), PathCmd::QuadTo(q1, q)) => PathCmd::QuadTo(l(p1, q1), l(p, q)),
                (PathCmd::CubicTo(p1, p2, p), PathCmd::CubicTo(q1, q2, q)) => PathCmd::CubicTo(l(p1, q1), l(p2, q2), l(p, q)),
                (PathCmd::Close, PathCmd::Close) => PathCmd::Close,
                _ => return None,
            });
        }
        Some(Path { cmds })
    }

    /// Flattens under `m` into polylines whose deviation from the true curve is at most
    /// `tolerance` (in output units). Non-finite points are dropped.
    pub fn flatten(&self, m: &Affine, tolerance: f32) -> Vec<Polyline> {
        let tol = if tolerance.is_finite() && tolerance > 1e-4 { tolerance } else { 0.1 };
        let mut out: Vec<Polyline> = Vec::new();
        let mut cur = Polyline::default();
        let mut start = Vec2::ZERO;
        let mut last = Vec2::ZERO;
        let flush = |cur: &mut Polyline, out: &mut Vec<Polyline>| {
            if cur.points.len() >= 2 {
                out.push(std::mem::take(cur));
            } else {
                cur.points.clear();
                cur.closed = false;
            }
        };
        for c in &self.cmds {
            match *c {
                PathCmd::MoveTo(p) => {
                    flush(&mut cur, &mut out);
                    let p = m.apply(p);
                    start = p;
                    last = p;
                    push_finite(&mut cur.points, p);
                }
                PathCmd::LineTo(p) => {
                    let p = m.apply(p);
                    if cur.points.is_empty() {
                        push_finite(&mut cur.points, last);
                    }
                    push_finite(&mut cur.points, p);
                    last = p;
                }
                PathCmd::QuadTo(c1, p) => {
                    let (c1, p) = (m.apply(c1), m.apply(p));
                    if cur.points.is_empty() {
                        push_finite(&mut cur.points, last);
                    }
                    let dd = (last - c1 * 2.0 + p).length();
                    let n = segments(0.25 * dd, tol);
                    for i in 1..=n {
                        let t = i as f32 / n as f32;
                        let mt = 1.0 - t;
                        let q = last * (mt * mt) + c1 * (2.0 * mt * t) + p * (t * t);
                        push_finite(&mut cur.points, q);
                    }
                    last = p;
                }
                PathCmd::CubicTo(c1, c2, p) => {
                    let (c1, c2, p) = (m.apply(c1), m.apply(c2), m.apply(p));
                    if cur.points.is_empty() {
                        push_finite(&mut cur.points, last);
                    }
                    let dd = (last - c1 * 2.0 + c2).length().max((c1 - c2 * 2.0 + p).length());
                    let n = segments(0.75 * dd, tol);
                    for i in 1..=n {
                        let t = i as f32 / n as f32;
                        let mt = 1.0 - t;
                        let q = last * (mt * mt * mt) + c1 * (3.0 * mt * mt * t) + c2 * (3.0 * mt * t * t) + p * (t * t * t);
                        push_finite(&mut cur.points, q);
                    }
                    last = p;
                }
                PathCmd::Close => {
                    cur.closed = true;
                    flush(&mut cur, &mut out);
                    last = start;
                }
            }
        }
        flush(&mut cur, &mut out);
        out
    }

    /// Whether `p` (in the same space as `m`'s output) is inside the filled path.
    pub fn contains(&self, p: Vec2, m: &Affine, rule: FillRule) -> bool {
        let mut winding = 0i32;
        for pl in self.flatten(m, 0.25) {
            let pts = &pl.points;
            let n = pts.len();
            for i in 0..n {
                let a = pts[i];
                let b = pts[(i + 1) % n];
                if a.y <= p.y {
                    if b.y > p.y && (b - a).cross(p - a) > 0.0 {
                        winding += 1;
                    }
                } else if b.y <= p.y && (b - a).cross(p - a) < 0.0 {
                    winding -= 1;
                }
            }
        }
        match rule {
            FillRule::NonZero => winding != 0,
            FillRule::EvenOdd => winding % 2 != 0,
        }
    }

    /// Distance from `p` to the nearest flattened edge (for stroke hit testing).
    pub fn distance_to_edge(&self, p: Vec2, m: &Affine) -> f32 {
        let mut best = f32::INFINITY;
        for pl in self.flatten(m, 0.25) {
            let n = pl.points.len();
            let segs = if pl.closed { n } else { n.saturating_sub(1) };
            for i in 0..segs {
                let a = pl.points[i];
                let b = pl.points[(i + 1) % n];
                best = best.min(segment_distance(p, a, b));
            }
        }
        best
    }
}

fn push_finite(v: &mut Vec<Vec2>, p: Vec2) {
    if p.is_finite() {
        v.push(p);
    }
}

fn segments(scaled_dd: f32, tol: f32) -> usize {
    let n = (scaled_dd / tol).sqrt().ceil();
    if n.is_finite() && n >= 1.0 { (n as usize).min(MAX_SEGMENTS_PER_CURVE) } else { 1 }
}

/// Distance from point `p` to segment `ab`.
pub fn segment_distance(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let len2 = ab.dot(ab);
    let t = if len2 > 1e-12 { ((p - a).dot(ab) / len2).clamp(0.0, 1.0) } else { 0.0 };
    p.distance(a + ab * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shoelace area of a polygon.
    fn poly_area(pts: &[Vec2]) -> f64 {
        let n = pts.len();
        (0..n).map(|i| {
            let (a, b) = (pts[i], pts[(i + 1) % n]);
            f64::from(a.x) * f64::from(b.y) - f64::from(b.x) * f64::from(a.y)
        }).sum::<f64>() * 0.5
    }

    /// ADR 0080: a regular polygon's corners are on its circle, its first straight up, and its
    /// area is n/2 · r² · sin(2π/n).
    #[test]
    fn polystar_polygon_is_regular() {
        for n in [3u32, 4, 5, 6, 8, 12, 32] {
            let r = 50.0;
            let pts = polystar_corners(10.0, -20.0, r, n, false, 0.3);
            assert_eq!(pts.len(), n as usize);
            assert!((pts[0].x - 10.0).abs() < 1e-4 && (pts[0].y - (-20.0 - r)).abs() < 1e-4, "first corner straight up: {:?}", pts[0]);
            for p in &pts {
                assert!(((*p - Vec2::new(10.0, -20.0)).length() - r).abs() < 1e-3);
            }
            let want = f64::from(n) / 2.0 * f64::from(r * r) * (std::f64::consts::TAU / f64::from(n)).sin();
            let got = poly_area(&pts);
            assert!((got.abs() - want).abs() < want * 1e-5, "n {n}: {got} vs {want}");
            // Clockwise on screen (y down): positive shoelace area.
            assert!(got > 0.0);
            // The path is the same polygon.
            let path = Path::polystar(10.0, -20.0, r, n, false, 0.3);
            assert_eq!(path.cmds.len(), n as usize + 1);
        }
    }

    /// ADR 0080: a star's inner corners are at r · size · cos(π/n): at 1 they lie on the
    /// polygon's edges (the star covers exactly the polygon), at 0 in the centre, and its area
    /// is n · r · r_in · sin(π/n).
    #[test]
    fn polystar_star_point_size() {
        let r = 80.0f32;
        for n in [3u32, 5, 7, 32] {
            let half = std::f64::consts::PI / f64::from(n);
            for size in [0.0f32, 0.25, 0.5, 1.0] {
                let pts = polystar_corners(0.0, 0.0, r, n, true, size);
                assert_eq!(pts.len(), 2 * n as usize);
                let inner = f64::from(r) * f64::from(size) * half.cos();
                assert!((f64::from(star_inner_radius(r, n, size)) - inner).abs() < 1e-3);
                for (i, p) in pts.iter().enumerate() {
                    let want = if i % 2 == 0 { f64::from(r) } else { inner };
                    assert!((f64::from(p.length()) - want).abs() < 1e-3, "n {n} size {size} corner {i}");
                }
                let want = f64::from(n) * f64::from(r) * inner * half.sin();
                assert!((poly_area(&pts) - want).abs() <= want * 1e-4 + 1e-3, "n {n} size {size}");
            }
            // Point size 1: the area of the polygon through the tips.
            let full = poly_area(&polystar_corners(0.0, 0.0, r, n, true, 1.0));
            let poly = poly_area(&polystar_corners(0.0, 0.0, r, n, false, 1.0));
            assert!((full - poly).abs() < poly * 1e-5);
        }
        // Out of range and not a number: clamped, or the default.
        assert_eq!(star_inner_radius(10.0, 5, 3.0), star_inner_radius(10.0, 5, 1.0));
        assert_eq!(star_inner_radius(10.0, 5, -1.0), 0.0);
        assert_eq!(star_inner_radius(10.0, 5, f32::NAN), star_inner_radius(10.0, 5, 0.5));
        assert_eq!(polystar_corners(0.0, 0.0, 1.0, 0, false, 0.5).len(), 3);
        assert_eq!(polystar_corners(0.0, 0.0, 1.0, u32::MAX, false, 0.5).len(), POLYSTAR_MAX_SIDES as usize);
    }

    #[test]
    fn flatten_rect_is_closed_polygon() {
        let pls = Path::rect(0.0, 0.0, 10.0, 5.0).flatten(&Affine::IDENTITY, 0.1);
        assert_eq!(pls.len(), 1);
        assert!(pls[0].closed);
        assert_eq!(pls[0].points.len(), 4);
    }

    #[test]
    fn ellipse_flatten_within_tolerance() {
        let r = 100.0;
        let pls = Path::ellipse(0.0, 0.0, r, r).flatten(&Affine::IDENTITY, 0.1);
        for p in &pls[0].points {
            assert!((p.length() - r).abs() < 0.2, "{}", p.length());
        }
        // tighter tolerance yields more points
        let fine = Path::ellipse(0.0, 0.0, r, r).flatten(&Affine::IDENTITY, 0.01);
        assert!(fine[0].points.len() > pls[0].points.len());
    }

    #[test]
    fn contains_respects_fill_rule() {
        let mut p = Path::rect(0.0, 0.0, 10.0, 10.0);
        p.cmds.extend(Path::rect(2.0, 2.0, 6.0, 6.0).cmds); // same winding → hole only with even-odd
        let c = Vec2::new(5.0, 5.0);
        assert!(p.contains(c, &Affine::IDENTITY, FillRule::NonZero));
        assert!(!p.contains(c, &Affine::IDENTITY, FillRule::EvenOdd));
        assert!(!p.contains(Vec2::new(11.0, 5.0), &Affine::IDENTITY, FillRule::NonZero));
    }

    #[test]
    fn hostile_curves_are_bounded() {
        let mut p = Path::new();
        p.move_to(Vec2::ZERO).cubic_to(Vec2::new(1e30, 0.0), Vec2::new(f32::NAN, 1.0), Vec2::new(1.0, 1.0));
        let pls = p.flatten(&Affine::IDENTITY, 0.1);
        assert!(pls.iter().all(|pl| pl.points.len() <= MAX_SEGMENTS_PER_CURVE + 1));
    }

    #[test]
    fn svg_round_trip_is_exact() {
        let mut p = Path::rounded_rect(-1.5, 2.25, 100.0, 33.333, 7.0);
        p.cmds.extend(Path::star(1e-7, -3e5, 12.0, 5.0, 7).cmds);
        p.quad_to(Vec2::new(0.1, 0.2), Vec2::new(1.0 / 3.0, -0.0));
        let s = p.to_svg();
        assert_eq!(Path::from_svg(&s).unwrap(), p);
        assert!(s.starts_with('M'));
    }

    #[test]
    fn svg_parser_rejects_garbage_without_panicking() {
        for bad in ["L", "M 1", "Z 3", "12 M", "M1 2 x", "M nan 1", "M 1e99 1", "M1 2L", "C1 2 3 4 5"] {
            assert!(Path::from_svg(bad).is_err(), "{bad}");
        }
        assert_eq!(Path::from_svg("").unwrap(), Path::new());
        assert_eq!(Path::from_svg("M1,2 L3,4 5,6 z").unwrap().cmds.len(), 4);
        assert_eq!(Path::from_svg("M1e-3 2E+2").unwrap().cmds[0], PathCmd::MoveTo(Vec2::new(1e-3, 200.0)));
    }

    #[test]
    fn legacy_json_paths_still_load() {
        let legacy = r#"{"cmds":[{"MoveTo":{"x":1.0,"y":2.0}},{"LineTo":{"x":3.0,"y":4.0}},"Close"]}"#;
        let p: Path = serde_json::from_str(legacy).unwrap();
        assert_eq!(p.cmds.len(), 3);
        let s = serde_json::to_string(&p).unwrap();
        assert_eq!(s, "\"M1 2L3 4Z\"");
    }

    #[test]
    fn lerp_requires_matching_structure() {
        let a = Path::rect(0.0, 0.0, 10.0, 10.0);
        let b = Path::rect(10.0, 10.0, 20.0, 20.0);
        let m = a.lerp(&b, 0.5).unwrap();
        assert_eq!(m.bounds(), Rect::new(5.0, 5.0, 20.0, 20.0));
        assert!(a.lerp(&Path::ellipse(0.0, 0.0, 1.0, 1.0), 0.5).is_none());
    }
}
