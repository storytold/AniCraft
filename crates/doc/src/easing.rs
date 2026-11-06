//! Easing curves for tweens.

use serde::{Deserialize, Serialize};

/// Maps linear tween progress `t ∈ [0, 1]` to eased progress.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum Easing {
    #[default]
    Linear,
    QuadIn,
    QuadOut,
    QuadInOut,
    CubicIn,
    CubicOut,
    CubicInOut,
    /// The traditional single-slider ease: −100 (ease in) … 0 (linear) … +100 (ease out).
    Classic(f32),
    /// CSS-style cubic Bézier through (0,0), (x1,y1), (x2,y2), (1,1).
    CubicBezier(f32, f32, f32, f32),
}

impl Easing {
    /// Presets shown in the UI, in menu order.
    pub const PRESETS: [(&'static str, Easing); 8] = [
        ("Linear", Easing::Linear),
        ("Ease in (quad)", Easing::QuadIn),
        ("Ease out (quad)", Easing::QuadOut),
        ("Ease in-out (quad)", Easing::QuadInOut),
        ("Ease in (cubic)", Easing::CubicIn),
        ("Ease out (cubic)", Easing::CubicOut),
        ("Ease in-out (cubic)", Easing::CubicInOut),
        ("Smooth (bezier)", Easing::CubicBezier(0.25, 0.1, 0.25, 1.0)),
    ];

    pub fn label(&self) -> String {
        match self {
            Easing::Classic(s) => format!("Classic {s:+.0}"),
            Easing::CubicBezier(a, b, c, d) => {
                Easing::PRESETS.iter().find(|(_, e)| e == self).map_or_else(|| format!("Bezier({a:.2}, {b:.2}, {c:.2}, {d:.2})"), |(n, _)| (*n).to_string())
            }
            other => Easing::PRESETS.iter().find(|(_, e)| e == other).map_or("Custom", |(n, _)| n).to_string(),
        }
    }

    pub fn apply(&self, t: f32) -> f32 {
        let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
        match *self {
            Easing::Linear => t,
            Easing::QuadIn => t * t,
            Easing::QuadOut => t * (2.0 - t),
            Easing::QuadInOut => {
                if t < 0.5 {
                    2.0 * t * t
                } else {
                    -1.0 + (4.0 - 2.0 * t) * t
                }
            }
            Easing::CubicIn => t * t * t,
            Easing::CubicOut => {
                let u = t - 1.0;
                u * u * u + 1.0
            }
            Easing::CubicInOut => {
                if t < 0.5 {
                    4.0 * t * t * t
                } else {
                    let u = 2.0 * t - 2.0;
                    0.5 * u * u * u + 1.0
                }
            }
            Easing::Classic(strength) => {
                // Quadratic Bézier with the middle control point moved by `strength`.
                let s = if strength.is_finite() { strength.clamp(-100.0, 100.0) / 100.0 } else { 0.0 };
                let c = 0.5 + 0.5 * s; // y of control point at x = 0.5
                2.0 * (1.0 - t) * t * c + t * t
            }
            Easing::CubicBezier(x1, y1, x2, y2) => cubic_bezier(t, x1, y1, x2, y2),
        }
    }
}

fn cubic_bezier(x: f32, x1: f32, y1: f32, x2: f32, y2: f32) -> f32 {
    if ![x1, y1, x2, y2].iter().all(|v| v.is_finite()) {
        return x;
    }
    // x control points must stay in [0, 1] so x(t) is monotonic.
    let (x1, x2) = (x1.clamp(0.0, 1.0), x2.clamp(0.0, 1.0));
    let bez = |t: f32, p1: f32, p2: f32| {
        let mt = 1.0 - t;
        3.0 * mt * mt * t * p1 + 3.0 * mt * t * t * p2 + t * t * t
    };
    let dbez = |t: f32, p1: f32, p2: f32| {
        let mt = 1.0 - t;
        3.0 * mt * mt * p1 + 6.0 * mt * t * (p2 - p1) + 3.0 * t * t * (1.0 - p2)
    };
    // Newton, then bisection as a fallback.
    let mut t = x;
    for _ in 0..8 {
        let err = bez(t, x1, x2) - x;
        if err.abs() < 1e-6 {
            return bez(t, y1, y2);
        }
        let d = dbez(t, x1, x2);
        if d.abs() < 1e-6 {
            break;
        }
        t = (t - err / d).clamp(0.0, 1.0);
    }
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    t = x;
    for _ in 0..40 {
        let v = bez(t, x1, x2);
        if (v - x).abs() < 1e-6 {
            break;
        }
        if v < x {
            lo = t;
        } else {
            hi = t;
        }
        t = 0.5 * (lo + hi);
    }
    bez(t, y1, y2)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Easing; 10] = [
        Easing::Linear,
        Easing::QuadIn,
        Easing::QuadOut,
        Easing::QuadInOut,
        Easing::CubicIn,
        Easing::CubicOut,
        Easing::CubicInOut,
        Easing::Classic(-60.0),
        Easing::Classic(100.0),
        Easing::CubicBezier(0.25, 0.1, 0.25, 1.0),
    ];

    #[test]
    fn endpoints_fixed() {
        for e in ALL {
            assert!(e.apply(0.0).abs() < 1e-4, "{e:?}");
            assert!((e.apply(1.0) - 1.0).abs() < 1e-4, "{e:?}");
        }
    }

    #[test]
    fn monotonic() {
        for e in ALL {
            let mut prev = -1e-6;
            for i in 0..=100 {
                let v = e.apply(i as f32 / 100.0);
                assert!(v >= prev - 1e-4, "{e:?} at {i}");
                prev = v;
            }
        }
    }

    #[test]
    fn hostile_input_is_clamped() {
        assert_eq!(Easing::Linear.apply(f32::NAN), 0.0);
        assert_eq!(Easing::QuadIn.apply(5.0), 1.0);
        assert!(Easing::CubicBezier(f32::NAN, 0.0, 1.0, 1.0).apply(0.5).is_finite());
        assert!(Easing::Classic(f32::INFINITY).apply(0.5).is_finite());
    }

    #[test]
    fn ease_in_is_slow_at_start() {
        assert!(Easing::QuadIn.apply(0.25) < 0.25);
        assert!(Easing::QuadOut.apply(0.25) > 0.25);
        assert!(Easing::Classic(-100.0).apply(0.25) < 0.25);
        assert!(Easing::Classic(100.0).apply(0.25) > 0.25);
    }
}
