//! Property-level animation channels (ADR 0102): one animatable value, its keys, and the
//! segments between them.
//!
//! AniCraft's keyframe spans animate *artwork* — a whole keyframe tweens into the next one.
//! Moho animates *properties*: every layer transform, the camera and (later) bones carry
//! their own little timeline, a `{when[], val[]}` channel of keys. [`Channel<T>`] is that
//! carrier, so imported documents keep their layer and camera animation without flattening
//! it onto elements. The two models coexist by design: spans drive elements and shapes,
//! channels drive layers, the camera and later skeletal data — the same split Moho and
//! Animate's motion editor draw.
//!
//! Interpolation follows Moho's public scripting constants (plan/moho › interp-modes):
//! linear, auto-smooth (approximated — Moho's exact formula is not public), hold, and a
//! cubic Bézier built from per-key tangent handles. Evaluation is pure:
//! [`Channel::sample`] maps a frame to a value, no state.

use crate::Easing;
use anicraft_geom::{lerp, Vec2};
use serde::{Deserialize, Serialize};

/// Most keys one channel keeps (files and commands alike).
pub const MAX_CHANNEL_KEYS: usize = 100_000;

/// Last frame a channel key may sit on.
pub const MAX_CHANNEL_FRAME: u32 = 100_000;

/// A tangent handle where a Bézier segment leaves or enters a key: how far along the
/// segment's time it sits (`percent`, a fraction of the span) and its slope (`angle`, in
/// radians; 0 = horizontal, i.e. no ease).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Handle {
    pub percent: f32,
    pub angle: f32,
}

impl Handle {
    /// Repairs a handle read from a file: finite, the percent inside the segment, the angle
    /// bounded (so its slope can't explode).
    pub fn sanitized(self) -> Self {
        let percent = if self.percent.is_finite() { self.percent.clamp(0.0, 1.0) } else { 1.0 / 3.0 };
        // ±80° keeps tan at ±5.67 — plenty of overshoot, no infinities.
        let angle = if self.angle.is_finite() { self.angle.clamp(-1.396, 1.396) } else { 0.0 };
        Handle { percent, angle }
    }
}

/// How the value travels from a key to the next one.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "h", rename_all = "snake_case")]
pub enum Interp {
    #[default]
    Linear,
    /// Auto-smooth, Moho's default (`INTERP_SMOOTH`): neighbouring keys steer the tangent.
    /// Moho's exact formula is not public; this is the usual Catmull-Rom spline through the
    /// keys, calibrated against Moho renders in the parity gates.
    Smooth,
    /// Hold this value until the next key (Moho's `INTERP_STEP`; string and bool channels
    /// always hold).
    Hold,
    /// A cubic Bézier: the segment's outgoing handles, one per component of the value.
    Bezier(Vec<Handle>),
}

fn is_linear(i: &Interp) -> bool {
    *i == Interp::Linear
}

/// One key of a channel.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Key<T> {
    /// Frame (0-based).
    pub frame: u32,
    pub val: T,
    /// How the value travels from this key to the next one.
    #[serde(default, skip_serializing_if = "is_linear")]
    pub out: Interp,
    /// The incoming handles the *previous* segment arrives by, one per component (used when
    /// that segment is a Bézier; Moho stores both of a key's handles in its interp setting).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub in_: Vec<Handle>,
    /// Frames to hold this value before the segment to the next key starts (Moho's `hold`).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub hold: u32,
    /// Animate on every `interval`-th frame (Moho's `interval`: 1 = every frame, 2 = on 2's).
    #[serde(default = "one")]
    pub interval: u32,
}

fn is_zero(v: &u32) -> bool {
    *v == 0
}
fn one() -> u32 {
    1
}

impl<T> Key<T> {
    pub fn new(frame: u32, val: T) -> Self {
        Key { frame: frame.min(MAX_CHANNEL_FRAME), val, out: Interp::Linear, in_: Vec::new(), hold: 0, interval: 1 }
    }
}

/// A value a channel can interpolate: its components sample independently (a Bézier or
/// smooth segment can ease x while y runs linearly). `COMPONENTS == 0` marks a value that
/// only ever steps (bools), so segments hold until the next key whatever their `out` says.
pub trait ChannelValue: Clone + PartialEq {
    const COMPONENTS: usize;
    /// One component of the value (never out of range: callers loop over `COMPONENTS`).
    fn component(&self, c: usize) -> f32;
    /// Builds the value from interpolated components.
    fn from_components(a: &Self, b: &Self, f: impl FnMut(usize) -> f32) -> Self;
}

impl ChannelValue for f32 {
    const COMPONENTS: usize = 1;
    fn component(&self, _c: usize) -> f32 {
        *self
    }
    fn from_components(a: &Self, b: &Self, mut f: impl FnMut(usize) -> f32) -> Self {
        lerp(*a, *b, f(0))
    }
}

impl ChannelValue for Vec2 {
    const COMPONENTS: usize = 2;
    fn component(&self, c: usize) -> f32 {
        match c {
            0 => self.x,
            _ => self.y,
        }
    }
    fn from_components(a: &Self, b: &Self, mut f: impl FnMut(usize) -> f32) -> Self {
        Vec2::new(lerp(a.x, b.x, f(0)), lerp(a.y, b.y, f(1)))
    }
}

impl ChannelValue for bool {
    const COMPONENTS: usize = 0;
    fn component(&self, _c: usize) -> f32 {
        0.0
    }
    fn from_components(a: &Self, _b: &Self, _f: impl FnMut(usize) -> f32) -> Self {
        *a
    }
}

/// One animatable value: its keys, sorted by frame, one per frame.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Channel<T> {
    #[serde(default)]
    pub keys: Vec<Key<T>>,
}

impl<T> Channel<T> {
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
    /// The channel as it looks when nothing animates it: one key holding `val`.
    pub fn constant(frame: u32, val: T) -> Channel<T> {
        Channel { keys: vec![Key::new(frame, val)] }
    }
    /// Index of the key at exactly `frame`.
    pub fn key_at(&self, frame: u32) -> Option<usize> {
        self.keys.iter().position(|k| k.frame == frame)
    }
    /// Writes `val` at `frame`: replaces that frame's key, else inserts one in order. The
    /// channel keeps at most [`MAX_CHANNEL_KEYS`] keys.
    pub fn set_key(&mut self, frame: u32, val: T)
    where
        T: Clone,
    {
        let frame = frame.min(MAX_CHANNEL_FRAME);
        match self.keys.binary_search_by_key(&frame, |k| k.frame) {
            Ok(i) => {
                if let Some(k) = self.keys.get_mut(i) {
                    k.val = val;
                }
            }
            Err(i) => {
                if self.keys.len() < MAX_CHANNEL_KEYS {
                    self.keys.insert(i, Key::new(frame, val));
                }
            }
        }
    }
    /// Removes the key at `frame`; `true` when there was one.
    pub fn remove_key(&mut self, frame: u32) -> bool {
        if let Ok(i) = self.keys.binary_search_by_key(&frame, |k| k.frame) {
            self.keys.remove(i);
            true
        } else {
            false
        }
    }
}

impl<T: ChannelValue> Channel<T> {
    /// The value at `frame`: the first key's value before it, the last key's after it, the
    /// segment between two keys otherwise. Fractional frames sample inside the segment.
    pub fn sample(&self, frame: f32) -> Option<T> {
        let first = self.keys.first()?;
        let last = self.keys.last()?;
        let frame = if frame.is_finite() { frame } else { first.frame as f32 };
        if frame <= first.frame as f32 || self.keys.len() == 1 {
            return Some(first.val.clone());
        }
        if frame >= last.frame as f32 {
            return Some(last.val.clone());
        }
        // The segment [a, b) holding `frame`: binary search on the key frames.
        let i = self.keys.partition_point(|k| (k.frame as f32) <= frame).checked_sub(1)?;
        let a = self.keys.get(i)?;
        let b = self.keys.get(i + 1)?;
        // A component that steps (bools, and Hold): the value stays until the next key.
        if T::COMPONENTS == 0 || a.out == Interp::Hold {
            return Some(a.val.clone());
        }
        // The key's `hold` delays the segment; the rest of it interpolates.
        let seg_start = a.frame.saturating_add(a.hold) as f32;
        if frame < seg_start {
            return Some(a.val.clone());
        }
        let seg_len = (b.frame as f32 - seg_start).max(1.0);
        let mut u = (frame - seg_start) / seg_len;
        if a.interval > 1 {
            // Animating on 2's (or 3's…): the value only advances on the interval grid.
            u = ((u * seg_len / a.interval as f32).floor() * a.interval as f32) / seg_len;
        }
        let u = u.clamp(0.0, 1.0);
        Some(T::from_components(&a.val, &b.val, |c| eased(self, i, c, u)))
    }
    /// [`Channel::sample`] with a default for the empty channel.
    pub fn sample_or(&self, frame: f32, default: T) -> T {
        self.sample(frame).unwrap_or(default)
    }
    /// Whether anything animates (more than one key).
    pub fn is_animated(&self) -> bool {
        self.keys.len() > 1
    }
    /// Whether the channel is present but does nothing: one key (or none) holding one value.
    pub fn is_static(&self) -> bool {
        self.keys.len() <= 1
    }
    /// Restores invariants after a load: sorted, one key per frame, finite values, bounded
    /// size, sane handles and hold/interval fields.
    pub fn sanitize(&mut self) {
        if self.keys.len() > MAX_CHANNEL_KEYS {
            self.keys.truncate(MAX_CHANNEL_KEYS);
        }
        if !self.keys.is_sorted_by_key(|k| k.frame) {
            self.keys.sort_by_key(|k| k.frame);
        }
        self.keys.retain_mut(|k| {
            k.frame = k.frame.min(MAX_CHANNEL_FRAME);
            if k.val.component(0).is_finite() && (0..T::COMPONENTS).all(|c| k.val.component(c).is_finite()) {
                if let Interp::Bezier(handles) = &mut k.out {
                    handles.truncate(T::COMPONENTS.max(1));
                    for h in handles.iter_mut() {
                        *h = h.sanitized();
                    }
                }
                for h in k.in_.iter_mut() {
                    *h = h.sanitized();
                }
                k.hold = k.hold.min(MAX_CHANNEL_FRAME);
                k.interval = k.interval.clamp(1, 100);
                true
            } else {
                false
            }
        });
        self.keys.dedup_by_key(|k| k.frame);
    }
}

/// The eased progress of component `c` inside segment `i` at linear progress `u`.
fn eased<T: ChannelValue>(ch: &Channel<T>, i: usize, c: usize, u: f32) -> f32 {
    let (Some(a), Some(b)) = (ch.keys.get(i), ch.keys.get(i + 1)) else { return u };
    let dt = (b.frame.saturating_sub(a.frame)).max(1) as f32;
    let value_span = b.val.component(c) - a.val.component(c);
    if value_span.abs() < 1e-9 {
        return u; // A value that doesn't move: any easing is that same line.
    }
    // The Bézier control points, in the normalized segment (CSS cubic-bezier shape): the out
    // handle sits `percent·dt` past `a` with slope tan(angle), the in handle the same before
    // `b`; their y's become fractions of the value span.
    let bezier = |out: &Handle, into: &Handle| {
        let y1 = out.angle.sin() / out.angle.cos() * out.percent * dt / value_span;
        let y2 = 1.0 - (into.angle.sin() / into.angle.cos()) * into.percent * dt / value_span;
        Easing::CubicBezier(out.percent.clamp(0.0, 1.0), y1, (1.0 - into.percent).clamp(0.0, 1.0), y2)
    };
    match &a.out {
        Interp::Linear => u,
        Interp::Hold => 0.0,
        // Auto-smooth: the Catmull-Rom spline through the neighbouring keys, as a Bézier.
        Interp::Smooth => {
            // The slope through a key: both neighbours when it has them, one-sided at the
            // ends. Frames are distinct after sanitize, so plain signed differences.
            let slope = |x: &Key<T>, y: &Key<T>| {
                let dt = y.frame as f32 - x.frame as f32;
                if dt.abs() < 1.0 { 0.0 } else { (y.val.component(c) - x.val.component(c)) / dt }
            };
            let tangent = |l: Option<&Key<T>>, r: Option<&Key<T>>, own: &Key<T>| match (l, r) {
                (Some(l), Some(r)) => slope(l, r),
                (l, r) => {
                    let other = r.or(l);
                    other.map_or(0.0, |o| slope(own, o))
                }
            };
            let m_a = tangent(ch.keys.get(i.checked_sub(1).unwrap_or(i)), Some(b), a) * dt;
            let m_b = tangent(Some(a), ch.keys.get(i + 2), b) * dt;
            // The Hermite tangents become handles at a third of the segment; a handle's
            // angle is its slope over the segment's time.
            bezier(&Handle { percent: 1.0 / 3.0, angle: (m_a / dt).atan() }, &Handle { percent: 1.0 / 3.0, angle: (m_b / dt).atan() }).apply(u)
        }
        Interp::Bezier(out_handles) => {
            // A missing out handle leaves the segment linear; a missing in handle arrives
            // heading straight for the key (the diagonal at a third of the segment).
            let out = out_handles.get(c).or_else(|| out_handles.first());
            let Some(out) = out else { return u };
            let into = b.in_.get(c).or_else(|| b.in_.first()).copied().unwrap_or(Handle { percent: 1.0 / 3.0, angle: (value_span / dt).atan() });
            bezier(out, &into).apply(u)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-6;

    fn ch(keys: Vec<(u32, f32)>) -> Channel<f32> {
        Channel { keys: keys.into_iter().map(|(f, v)| Key::new(f, v)).collect() }
    }

    #[test]
    fn empty_and_bounds() {
        let c: Channel<f32> = Channel::default();
        assert_eq!(c.sample(3.0), None);
        assert_eq!(c.sample_or(3.0, 7.0), 7.0);
        let c = ch(vec![(5, 2.0)]);
        assert_eq!(c.sample(0.0), Some(2.0));
        assert_eq!(c.sample(5.0), Some(2.0));
        assert_eq!(c.sample(100.0), Some(2.0));
        assert!(c.is_static() && !c.is_animated());
    }

    #[test]
    fn linear_is_exact() {
        let c = ch(vec![(0, 1.0), (10, 3.0)]);
        // Linear segments are exact up to f32 rounding — well inside the ±1e-6 the parity
        // gates ask for.
        for f in 0..=10u32 {
            assert!((c.sample(f as f32).unwrap() - (1.0 + 0.2 * f as f32)).abs() < 1e-6, "frame {f}");
        }
        assert_eq!(c.sample(-4.0), Some(1.0));
        assert_eq!(c.sample(99.0), Some(3.0));
        assert!((c.sample(2.5).unwrap() - 1.5).abs() < EPS);
    }

    #[test]
    fn hold_steps_and_hold_frames() {
        let mut c = ch(vec![(0, 1.0), (10, 2.0)]);
        c.keys[0].out = Interp::Hold;
        assert_eq!(c.sample(9.0), Some(1.0));
        assert_eq!(c.sample(10.0), Some(2.0));
        // Hold for 5 frames, then run: halfway through the remaining span.
        let mut c = ch(vec![(0, 0.0), (10, 10.0)]);
        c.keys[0].hold = 5;
        assert_eq!(c.sample(4.0), Some(0.0));
        assert!((c.sample(10.0).unwrap() - 10.0).abs() < EPS);
        assert!((c.sample(7.5).unwrap() - 5.0).abs() < EPS);
    }

    #[test]
    fn interval_is_on_twos() {
        let mut c = ch(vec![(0, 0.0), (10, 10.0)]);
        c.keys[0].interval = 2;
        assert_eq!(c.sample(1.0), Some(0.0));
        assert!((c.sample(2.0).unwrap() - 2.0).abs() < EPS);
        assert!((c.sample(3.0).unwrap() - 2.0).abs() < EPS);
        assert!((c.sample(4.0).unwrap() - 4.0).abs() < EPS);
    }

    #[test]
    fn smooth_runs_through_the_keys() {
        let mut c = ch(vec![(0, 0.0), (10, 10.0), (20, 0.0)]);
        c.keys[0].out = Interp::Smooth;
        c.keys[1].out = Interp::Smooth;
        // The key frames are exact.
        for (f, v) in [(0.0, 0.0), (10.0, 10.0), (20.0, 0.0)] {
            assert!((c.sample(f).unwrap() - v).abs() < 1e-4, "at {f}");
        }
        // The key at 10 is a summit: the spline flattens into it and away from it, running
        // above the straight line on both sides.
        assert!(c.sample(8.0).unwrap() > 8.0);
        assert!(c.sample(12.0).unwrap() > 8.0);
        // The keys are symmetric about frame 10, and the spline follows.
        assert!((c.sample(5.0).unwrap() - c.sample(15.0).unwrap()).abs() < 1e-3);
    }

    #[test]
    fn bezier_matches_the_curve() {
        // Ease-in-out: slow start, fast middle, slow end.
        let mut c = ch(vec![(0, 0.0), (100, 1.0)]);
        c.keys[0].out = Interp::Bezier(vec![Handle { percent: 0.25, angle: 0.0 }]);
        c.keys[1].in_ = vec![Handle { percent: 0.25, angle: 0.0 }];
        // flat handles at 25%: the standard (0.25, 0) (0.75, 1) ease-in-out.
        assert!(c.sample(25.0).unwrap() < 0.25);
        assert!((c.sample(50.0).unwrap() - 0.5).abs() < 0.01);
        assert!(c.sample(75.0).unwrap() > 0.75);
        // Handles are point-symmetric about the segment's middle, overshoot included.
        let mut c = ch(vec![(0, 0.0), (10, 1.0)]);
        c.keys[0].out = Interp::Bezier(vec![Handle { percent: 1.0 / 3.0, angle: 0.9 }]);
        c.keys[1].in_ = vec![Handle { percent: 1.0 / 3.0, angle: 0.9 }];
        assert_eq!(c.sample(0.0), Some(0.0));
        assert_eq!(c.sample(10.0), Some(1.0));
        let (a, b) = (c.sample(2.5).unwrap(), c.sample(7.5).unwrap());
        assert!((a + b - 1.0).abs() < 1e-3, "symmetric: {a} + {b}");
        // A missing out handle leaves the segment linear.
        let mut c = ch(vec![(0, 0.0), (10, 1.0)]);
        c.keys[0].out = Interp::Bezier(vec![]);
        assert!((c.sample(2.5).unwrap() - 0.25).abs() < 1e-6);
    }

    #[test]
    fn vec2_components_sample_independently() {
        let mut c = Channel {
            keys: vec![
                Key::new(0, Vec2::new(0.0, 0.0)),
                Key::new(10, Vec2::new(10.0, 100.0)),
            ],
        };
        c.keys[0].out = Interp::Hold;
        assert_eq!(c.sample(5.0), Some(Vec2::new(0.0, 0.0)), "Hold steps the whole value");
        c.keys[0].out = Interp::Bezier(vec![Handle { percent: 1.0, angle: 0.0 }, Handle { percent: 0.0, angle: 0.0 }]);
        // x is held back (out handle at the far end), y runs ahead (out handle at the start).
        let v = c.sample(5.0).unwrap();
        assert!(v.x < 1.0 && (v.y - 50.0).abs() < 1e-3, "{v:?}");
    }

    #[test]
    fn bools_step() {
        let mut c = Channel { keys: vec![Key::new(0, false), Key::new(10, true)] };
        c.keys[0].out = Interp::Smooth;
        assert!(!c.sample(9.0).unwrap());
        assert!(c.sample(10.0).unwrap());
    }

    #[test]
    fn set_and_remove_keys() {
        let mut c = ch(vec![(0, 0.0), (10, 10.0)]);
        c.set_key(5, 4.0);
        assert_eq!(c.key_at(5), Some(1));
        assert_eq!(c.keys.len(), 3);
        c.set_key(5, 5.0);
        assert_eq!(c.sample(5.0), Some(5.0));
        assert!(c.remove_key(5) && !c.remove_key(5));
        assert_eq!(c.keys.len(), 2);
        c.set_key(u32::MAX, 3.0);
        assert_eq!(c.keys.last().map(|k| k.frame), Some(MAX_CHANNEL_FRAME));
    }

    #[test]
    fn sanitize_repairs_files() {
        let mut c = ch(vec![(10, 2.0), (0, f32::NAN), (0, 1.0), (20, 3.0)]);
        c.keys[2].out = Interp::Bezier(vec![Handle { percent: 5.0, angle: f32::INFINITY }]);
        c.keys[3].interval = 400;
        c.sanitize();
        // The NaN key is dropped, frames sort and dedupe, handles clamp.
        assert_eq!(c.keys.iter().map(|k| k.frame).collect::<Vec<_>>(), vec![0, 10, 20]);
        assert_eq!(c.keys[0].val, 1.0);
        assert_eq!(c.keys[0].out, Interp::Bezier(vec![Handle { percent: 1.0, angle: 0.0 }]));
        assert_eq!(c.keys[2].interval, 100);
        assert_eq!(c.sample(10.0), Some(2.0));
    }

    #[test]
    fn serde_round_trip() {
        let mut c = ch(vec![(0, 1.0), (10, 2.0)]);
        c.keys[0].out = Interp::Smooth;
        c.keys[1].in_ = vec![Handle { percent: 0.5, angle: 0.25 }];
        let json = serde_json::to_string(&c).unwrap();
        assert!(!json.contains("kind\":\"linear"), "the default omits: {json}");
        let back: Channel<f32> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, c);
        // A bare [] or missing keys field reads as an empty channel.
        let empty: Channel<f32> = serde_json::from_str("[]").unwrap();
        assert!(empty.is_empty());
    }
}






