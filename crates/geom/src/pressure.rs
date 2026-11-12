//! Pressure curves (ADR 0068): how pen pressure maps to the brush's size (and, optionally, the
//! paint's opacity).
//!
//! A curve is a few points `(input, output)` in `0..=1`, sorted by input, with one at input 0
//! and one at input 1. Between them it follows a monotone cubic (Fritsch–Carlson): smooth,
//! through every point, never overshooting a neighbour, so the output stays in `0..=1` and a
//! rising curve never dips. It is `Copy` (a fixed array), so tool options holding it stay cheap.

use serde::{Deserialize, Serialize};

/// Most points one curve keeps.
pub const MAX_CURVE_POINTS: usize = 8;
/// Points closer than this in input are one point (the last given wins).
const MERGE: f32 = 0.02;

/// A pressure curve. Build one with [`PressureCurve::new`] (sanitizing) or a preset.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(from = "Vec<[f32; 2]>", into = "Vec<[f32; 2]>")]
pub struct PressureCurve {
    n: u8,
    pts: [[f32; 2]; MAX_CURVE_POINTS],
}

/// The built-in curves: `(key, label)`.
pub const CURVE_PRESETS: [(&str, &str); 3] = [("linear", "Linear"), ("soft", "Soft"), ("firm", "Firm")];

impl Default for PressureCurve {
    fn default() -> Self {
        PressureCurve::LINEAR
    }
}

const fn fixed(a: [[f32; 2]; 3], n: u8) -> PressureCurve {
    let mut pts = [[0.0; 2]; MAX_CURVE_POINTS];
    pts[0] = a[0];
    pts[1] = a[1];
    pts[2] = a[2];
    PressureCurve { n, pts }
}

impl PressureCurve {
    /// Output equals input.
    pub const LINEAR: PressureCurve = fixed([[0.0, 0.0], [1.0, 1.0], [0.0, 0.0]], 2);
    /// A light touch already paints wide (for a stiff pen or a light hand).
    pub const SOFT: PressureCurve = fixed([[0.0, 0.0], [0.3, 0.6], [1.0, 1.0]], 3);
    /// It takes a firm press to paint wide (for a soft pen or a heavy hand).
    pub const FIRM: PressureCurve = fixed([[0.0, 0.0], [0.6, 0.3], [1.0, 1.0]], 3);

    /// A curve from untrusted points: non-finite ones dropped, clamped to `0..=1`, sorted,
    /// merged when closer than 0.02 in input, at most [`MAX_CURVE_POINTS`], and given ends at
    /// input 0 and 1 (holding the nearest point's output). No points: linear.
    pub fn new(points: &[[f32; 2]]) -> PressureCurve {
        let mut v: Vec<[f32; 2]> = points.iter().take(MAX_CURVE_POINTS * 8).filter(|p| p[0].is_finite() && p[1].is_finite()).map(|p| [p[0].clamp(0.0, 1.0), p[1].clamp(0.0, 1.0)]).collect();
        if v.is_empty() {
            return PressureCurve::LINEAR;
        }
        v.sort_by(|a, b| a[0].total_cmp(&b[0]));
        let mut out: Vec<[f32; 2]> = Vec::with_capacity(v.len() + 2);
        for p in v {
            match out.last_mut() {
                Some(last) if p[0] - last[0] < MERGE => last[1] = p[1],
                _ => out.push(p),
            }
        }
        // Ends at 0 and 1: snap a point that is nearly there, else add one.
        if let Some(first) = out.first_mut() {
            if first[0] < MERGE {
                first[0] = 0.0;
            } else {
                let y = first[1];
                out.insert(0, [0.0, y]);
            }
        }
        if let Some(last) = out.last_mut() {
            if last[0] > 1.0 - MERGE {
                last[0] = 1.0;
            } else {
                let y = last[1];
                out.push([1.0, y]);
            }
        }
        // Keep the ends and the first interior points that fit.
        if out.len() > MAX_CURVE_POINTS
            && let Some(end) = out.last().copied()
        {
            out.truncate(MAX_CURVE_POINTS - 1);
            out.push(end);
        }
        let mut pts = [[0.0; 2]; MAX_CURVE_POINTS];
        for (slot, p) in pts.iter_mut().zip(&out) {
            *slot = *p;
        }
        PressureCurve { n: out.len().min(MAX_CURVE_POINTS) as u8, pts }
    }

    /// A built-in curve by key (`linear`, `soft`, `firm`).
    pub fn preset(key: &str) -> Option<PressureCurve> {
        match key {
            "linear" => Some(PressureCurve::LINEAR),
            "soft" => Some(PressureCurve::SOFT),
            "firm" => Some(PressureCurve::FIRM),
            _ => None,
        }
    }

    /// The built-in curve this is, if any.
    pub fn preset_key(&self) -> Option<&'static str> {
        CURVE_PRESETS.iter().map(|(k, _)| *k).find(|k| PressureCurve::preset(k).is_some_and(|c| c == *self))
    }

    pub fn points(&self) -> &[[f32; 2]] {
        self.pts.get(..usize::from(self.n)).unwrap_or(&[])
    }

    /// The same curve with point `i` moved to `to` (the end points move only up and down) or,
    /// with `i` past the end, a point added at `to`.
    pub fn with_point(&self, i: usize, to: [f32; 2]) -> PressureCurve {
        let mut v = self.points().to_vec();
        let last = v.len().saturating_sub(1);
        match v.get_mut(i) {
            Some(p) if i == 0 => p[1] = to[1],
            Some(p) if i == last => p[1] = to[1],
            // An interior point stays between its neighbours.
            Some(p) => *p = to,
            None => v.push(to),
        }
        PressureCurve::new(&v)
    }

    /// Without interior point `i` (the ends stay).
    pub fn without_point(&self, i: usize) -> PressureCurve {
        let v: Vec<[f32; 2]> = self.points().iter().enumerate().filter(|(j, _)| *j != i || *j == 0 || *j + 1 == usize::from(self.n)).map(|(_, p)| *p).collect();
        PressureCurve::new(&v)
    }

    /// The output at `pressure` (clamped to `0..=1`; not a number reads as full pressure).
    pub fn map(&self, pressure: f32) -> f32 {
        let x = if pressure.is_finite() { pressure.clamp(0.0, 1.0) } else { 1.0 };
        let p = self.points();
        let n = p.len();
        let (Some(first), Some(last)) = (p.first(), p.last()) else { return x };
        if n == 1 || x <= first[0] {
            return first[1];
        }
        if x >= last[0] {
            return last[1];
        }
        // Secant slopes, then Fritsch–Carlson tangents.
        let sec = |i: usize| -> f32 {
            match (p.get(i), p.get(i + 1)) {
                (Some(a), Some(b)) if b[0] - a[0] > 1e-9 => (b[1] - a[1]) / (b[0] - a[0]),
                _ => 0.0,
            }
        };
        let tangent = |i: usize| -> f32 {
            if i == 0 {
                return sec(0);
            }
            if i + 1 >= n {
                return sec(n.saturating_sub(2));
            }
            let (d0, d1) = (sec(i - 1), sec(i));
            if d0 * d1 <= 0.0 {
                return 0.0;
            }
            // The weighted harmonic mean of the secants (Fritsch–Butland): monotone as it is.
            let h = |j: usize| match (p.get(j), p.get(j + 1)) {
                (Some(a), Some(b)) => (b[0] - a[0]).max(1e-6),
                _ => 1e-6,
            };
            let (h0, h1) = (h(i - 1), h(i));
            3.0 * (h0 + h1) / ((2.0 * h1 + h0) / d0 + (h1 + 2.0 * h0) / d1)
        };
        let k = p.windows(2).position(|w| matches!(w, [a, b] if x >= a[0] && x <= b[0])).unwrap_or(0);
        let (Some(a), Some(b)) = (p.get(k), p.get(k + 1)) else { return x };
        let h = b[0] - a[0];
        if h <= 1e-9 {
            return b[1];
        }
        let d = sec(k);
        let (mut m0, mut m1) = (tangent(k), tangent(k + 1));
        // Keep the piece monotone where the data is (Fritsch–Carlson's limit).
        if d == 0.0 {
            m0 = 0.0;
            m1 = 0.0;
        } else {
            let (al, be) = (m0 / d, m1 / d);
            let s = al * al + be * be;
            if s > 9.0 {
                let t = 3.0 / s.sqrt();
                m0 = t * al * d;
                m1 = t * be * d;
            }
            if al < 0.0 {
                m0 = 0.0;
            }
            if be < 0.0 {
                m1 = 0.0;
            }
        }
        let t = (x - a[0]) / h;
        let (t2, t3) = (t * t, t * t * t);
        let y = (2.0 * t3 - 3.0 * t2 + 1.0) * a[1] + (t3 - 2.0 * t2 + t) * h * m0 + (-2.0 * t3 + 3.0 * t2) * b[1] + (t3 - t2) * h * m1;
        y.clamp(0.0, 1.0)
    }
}

impl From<Vec<[f32; 2]>> for PressureCurve {
    fn from(v: Vec<[f32; 2]>) -> PressureCurve {
        PressureCurve::new(&v)
    }
}

impl From<PressureCurve> for Vec<[f32; 2]> {
    fn from(c: PressureCurve) -> Vec<[f32; 2]> {
        c.points().to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_map_as_named() {
        for x in [0.0, 0.1, 0.25, 0.5, 0.8, 1.0] {
            assert!((PressureCurve::LINEAR.map(x) - x).abs() < 1e-5, "linear at {x}");
        }
        // Soft paints wider than linear at a light touch, firm narrower; all meet at the ends.
        for x in [0.1, 0.3, 0.5, 0.7, 0.9] {
            assert!(PressureCurve::SOFT.map(x) > x + 0.02 && PressureCurve::FIRM.map(x) < x - 0.02, "{x}");
        }
        for c in [PressureCurve::SOFT, PressureCurve::FIRM] {
            assert_eq!((c.map(0.0), c.map(1.0)), (0.0, 1.0));
            assert!((c.map(0.3) - c.points()[1][1]).abs() < 1e-5 || (c.map(0.6) - c.points()[1][1]).abs() < 1e-5, "passes through its point");
        }
        assert_eq!(PressureCurve::preset("soft").and_then(|c| c.preset_key()), Some("soft"));
        assert_eq!(PressureCurve::new(&[[0.0, 0.2], [1.0, 0.9]]).preset_key(), None);
    }

    #[test]
    fn rising_curves_never_dip_or_overshoot() {
        let c = PressureCurve::new(&[[0.0, 0.0], [0.1, 0.9], [0.15, 0.92], [1.0, 1.0]]);
        let mut prev = -1.0;
        for i in 0..=1000 {
            let y = c.map(i as f32 / 1000.0);
            assert!(y >= prev - 1e-6 && (0.0..=1.0).contains(&y), "at {i}: {y} after {prev}");
            prev = y;
        }
    }

    #[test]
    fn untrusted_points_are_sanitized() {
        let c = PressureCurve::new(&[[f32::NAN, 0.0], [0.5, 2.0], [0.51, 0.4], [-3.0, 0.1]]);
        // (-3, 0.1) clamps to (0, 0.1); 0.5 and 0.51 merge (the last wins); an end at 1 holds.
        assert_eq!(c.points(), &[[0.0, 0.1], [0.5, 0.4], [1.0, 0.4]]);
        assert_eq!(PressureCurve::new(&[]), PressureCurve::LINEAR);
        let many: Vec<[f32; 2]> = (0..100).map(|i| [i as f32 / 99.0, 0.5]).collect();
        let c = PressureCurve::new(&many);
        assert!(c.points().len() <= MAX_CURVE_POINTS && c.points().last() == Some(&[1.0, 0.5]));
        assert_eq!(c.map(f32::NAN), 0.5);
        // Moving the end points only moves them up and down; adding and removing.
        let c = PressureCurve::LINEAR.with_point(0, [0.7, 0.2]);
        assert_eq!(c.points(), &[[0.0, 0.2], [1.0, 1.0]]);
        let c = c.with_point(9, [0.5, 0.9]);
        assert_eq!(c.points().len(), 3);
        assert_eq!(c.without_point(1).points().len(), 2);
        assert_eq!(c.without_point(0).points().len(), 3, "ends stay");
        // A file's curve round-trips.
        let json = serde_json::to_string(&PressureCurve::SOFT).unwrap();
        assert_eq!(serde_json::from_str::<PressureCurve>(&json).unwrap(), PressureCurve::SOFT);
    }
}
