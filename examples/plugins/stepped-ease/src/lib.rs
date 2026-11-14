//! Stepped Eases: an ease plug-in. Tweens that move in steps, as stop-motion and limited
//! animation do (on twos, on fours), with a smooth variant that eases each step. Each ease is
//! a pure function of time; AniCraft samples it once when the plug-in loads, and a tween that
//! uses one keeps the curve, so the movie plays the same without the plug-in.

use anicraft_plugin_sdk as ac;

const MANIFEST: &str = r#"{
  "id": "org.anicraft.stepped-ease",
  "name": "Stepped Eases",
  "version": "1.0.0",
  "kind": "ease",
  "author": "AniCraft contributors",
  "description": "Tweens that move in steps, like stop motion, plain or with each step eased.",
  "eases": [
    {"id": "steps2", "name": "Stepped 2"},
    {"id": "steps4", "name": "Stepped 4"},
    {"id": "steps8", "name": "Stepped 8"},
    {"id": "smooth-steps4", "name": "Smooth Steps 4"}
  ]
}"#;

ac::ease!(MANIFEST, ease);

/// Ease `index` of the manifest at time `t`.
fn ease(index: usize, t: f64) -> f64 {
    let t = if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
    match index {
        0 => steps(t, 2.0),
        1 => steps(t, 4.0),
        2 => steps(t, 8.0),
        _ => smooth_steps(t, 4.0),
    }
}

/// `n` equal steps, holding each: 0 until 1/n, then 1/n, …; exactly 1 at the end.
fn steps(t: f64, n: f64) -> f64 {
    if t >= 1.0 { 1.0 } else { (t * n).floor() / n }
}

/// `n` steps, each eased in and out (smoothstep), so the motion settles between beats.
fn smooth_steps(t: f64, n: f64) -> f64 {
    if t >= 1.0 {
        return 1.0;
    }
    let x = t * n;
    let (k, f) = (x.floor(), x - x.floor());
    (k + f * f * (3.0 - 2.0 * f)) / n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_hold_then_jump_and_end_at_one() {
        assert_eq!([0.0, 0.2, 0.25, 0.49, 0.5, 0.99, 1.0].map(|t| ease(1, t)), [0.0, 0.0, 0.25, 0.25, 0.5, 0.75, 1.0]);
        assert_eq!([0.0, 0.49, 0.5, 1.0].map(|t| ease(0, t)), [0.0, 0.0, 0.5, 1.0]);
        assert_eq!(ease(2, 0.126), 0.125);
    }

    #[test]
    fn smooth_steps_pass_through_each_step_and_never_go_back() {
        assert_eq!((ease(3, 0.0), ease(3, 0.25), ease(3, 1.0)), (0.0, 0.25, 1.0));
        assert!((ease(3, 0.125) - 0.125).abs() < 1e-12, "half way through the first step");
        let mut last = 0.0;
        for i in 0..=1000 {
            let v = ease(3, i as f64 / 1000.0);
            assert!(v >= last && v.is_finite());
            last = v;
        }
    }

    #[test]
    fn every_ease_is_finite_and_ends_on_its_keyframes() {
        for i in 0..4 {
            assert_eq!((ease(i, 0.0), ease(i, 1.0)), (0.0, 1.0));
            assert_eq!(ease(i, f64::NAN), 0.0);
        }
    }
}
