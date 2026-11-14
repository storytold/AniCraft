//! Retiming a motion tween (ADR 0083): moving property keys in time, stretching the tween with
//! its span, and joining two tweens into one. Pure data edits; `evaluate` is untouched.
//!
//! Frames are offsets into the span, as everywhere in a [`MotionTween`].

use super::{MotionTween, Pin, Property, PropertyKey, Props, Track, own_value};
use crate::{Easing, Element};

/// A property key picked for retiming: its property and its frame (offset into the span).
pub type KeyRef = (Property, u32);

/// Values closer than this are the same (the join compares objects' values).
const SAME: f32 = 1e-4;

impl MotionTween {
    /// The keys `keys` names that exist, with the X and Y keys of a pinned frame taken together
    /// (they are the place of the path point pinned there, ADR 0049): sorted, no repeats.
    pub fn retime_selection(&self, keys: &[KeyRef]) -> Vec<KeyRef> {
        let pins = self.live_pins();
        let mut out: Vec<KeyRef> = Vec::with_capacity(keys.len());
        let add = |out: &mut Vec<KeyRef>, k: KeyRef| {
            if self.track(k.0).is_some_and(|t| t.key_at(k.1).is_some()) && !out.contains(&k) {
                out.push(k);
            }
        };
        for &(p, f) in keys.iter().take(super::MAX_KEYS * 4) {
            add(&mut out, (p, f));
            if matches!(p, Property::X | Property::Y) && pins.iter().any(|q| q.frame == f) {
                add(&mut out, (Property::X, f));
                add(&mut out, (Property::Y, f));
            }
        }
        out.sort_by_key(|(p, f)| (p.index(), *f));
        out
    }

    /// How far `by` frames the keys can really go: none before the span's first frame or after
    /// `last` (a key already past it stays where it is), and none onto or past a key of its
    /// track that isn't moving. The result has `by`'s sign (or is 0) and is never larger.
    pub fn retime_limit(&self, keys: &[KeyRef], by: i64, last: u32) -> i64 {
        let sel = self.retime_selection(keys);
        let (mut lo, mut hi) = (i64::MIN, i64::MAX);
        for t in &self.tracks {
            let picked = |f: u32| sel.contains(&(t.property, f));
            for (i, k) in t.keys.iter().enumerate() {
                if !picked(k.frame) {
                    continue;
                }
                let f = i64::from(k.frame);
                // The neighbours that stay put bound it; moving ones travel with it.
                let prev = i.checked_sub(1).and_then(|j| t.keys.get(j)).filter(|q| !picked(q.frame)).map_or(-1, |q| i64::from(q.frame));
                let next = t.keys.get(i + 1).filter(|q| !picked(q.frame)).map_or(i64::MAX, |q| i64::from(q.frame));
                let top = i64::from(last).max(f);
                lo = lo.max(prev + 1 - f);
                hi = hi.min(next.saturating_sub(1).min(top) - f);
            }
        }
        if lo > hi {
            return 0;
        }
        by.clamp(lo.min(0), hi.max(0))
    }

    /// Moves the keys `keys` names by `by` frames, as far as [`MotionTween::retime_limit`]
    /// lets them: values and eases travel with their keys, and a pinned path point's pin with
    /// its X and Y keys. Returns how far they went and where they are now.
    pub fn move_keys(&mut self, keys: &[KeyRef], by: i64, last: u32) -> (i64, Vec<KeyRef>) {
        let sel = self.retime_selection(keys);
        let by = self.retime_limit(&sel, by, last);
        let shift = |f: u32| u32::try_from(i64::from(f) + by).unwrap_or(f);
        if by == 0 {
            return (0, sel);
        }
        let pinned: Vec<u32> = self.live_pins().iter().map(|q| q.frame).filter(|f| sel.contains(&(Property::X, *f)) && sel.contains(&(Property::Y, *f))).collect();
        for t in &mut self.tracks {
            let p = t.property;
            for k in &mut t.keys {
                if sel.contains(&(p, k.frame)) {
                    k.frame = shift(k.frame);
                }
            }
        }
        for q in &mut self.pins {
            if pinned.contains(&q.frame) {
                q.frame = shift(q.frame);
            }
        }
        // The limits keep every track in order; a pin whose keys moved apart lets go.
        self.pins = self.live_pins();
        (by, sel.into_iter().map(|(p, f)| (p, shift(f))).collect())
    }

    /// Stretches the tween from a span of `from` frames to one of `to`, as dragging the span's
    /// end does in Animate: every key (and pinned frame) keeps its place in proportion, the
    /// first frame on the first and the last on the last. Keys that would land on one frame are
    /// spread onto the frames beside it; only when a track has more keys than the span has
    /// frames do keys become one (on the first frame the first key stays, elsewhere the later
    /// key's value and ease win). A span of one frame has nothing to stretch.
    pub fn stretch(&mut self, from: u32, to: u32) {
        let (a, b) = (u64::from(from.saturating_sub(1)), u64::from(to.saturating_sub(1)));
        if a == 0 || a == b {
            return;
        }
        let map = |f: u32| i64::try_from((u64::from(f) * b + a / 2) / a).unwrap_or(i64::MAX);
        let (old_last, new_last) = (a as u32, b as i64);
        // Where each key of the X track goes: the pins follow their frames' keys.
        let mut x_moves: Vec<(u32, u32)> = Vec::new();
        for t in &mut self.tracks {
            let mut at: Vec<i64> = t.keys.iter().map(|k| map(k.frame)).collect();
            // Apart going forward, then back inside the span from its last frame.
            let mut floor = i64::MIN;
            for c in &mut at {
                *c = (*c).max(floor);
                floor = c.saturating_add(1);
            }
            let inside = t.keys.iter().filter(|k| k.frame <= old_last).count();
            let mut cap = new_last;
            for i in (0..inside).rev() {
                if let Some(c) = at.get_mut(i) {
                    *c = (*c).min(cap).max(0);
                    cap = *c - 1;
                }
            }
            let mut keys: Vec<PropertyKey> = Vec::with_capacity(t.keys.len());
            for (k, f) in t.keys.iter().zip(at) {
                let frame = u32::try_from(f).unwrap_or(u32::MAX);
                if t.property == Property::X {
                    x_moves.push((k.frame, frame));
                }
                let k = PropertyKey { frame, ..k.clone() };
                match keys.last_mut() {
                    Some(prev) if prev.frame >= k.frame && prev.frame == 0 => {}
                    Some(prev) if prev.frame >= k.frame => *prev = PropertyKey { frame: prev.frame, ..k },
                    _ => keys.push(k),
                }
            }
            t.keys = keys;
        }
        let mut pins: Vec<Pin> = Vec::with_capacity(self.pins.len());
        for q in &self.pins {
            let frame = x_moves.iter().find(|(f, _)| *f == q.frame).map_or_else(|| u32::try_from(map(q.frame)).unwrap_or(u32::MAX), |m| m.1);
            if pins.last().is_none_or(|p| p.frame < frame) {
                pins.push(Pin { frame, ..*q });
            }
        }
        self.pins = pins;
        self.pins = self.live_pins();
    }

    /// This tween over `frames` frames followed by `next` (the next span's, whose object's own
    /// values are `next_base`), as one tween over both spans (Join Motion). When `next` is what
    /// splitting this one at `frames` gives, the join is exact: this tween comes back, its keys
    /// past the first span included. Otherwise each property keeps its values on every frame:
    /// this tween's keys before the cut, `next`'s after it, with a key holding the value up to
    /// the cut where the two differ. `None` when either has a motion path and they aren't one
    /// split tween (joining two paths isn't supported yet).
    pub fn join(&self, base: &Element, frames: u32, next: &MotionTween, next_base: &Element) -> Option<MotionTween> {
        let pa = Props::of(base);
        if frames > 0 && self.split_at(&pa, frames) == *next && self.apply(base, frames as f32) == *next_base {
            return Some(self.clone());
        }
        if self.path.is_some() || next.path.is_some() || frames == 0 {
            return None;
        }
        let last = frames - 1;
        let mut tracks = Vec::new();
        // Every property, and the filters' numbers both objects have (ADR 0092; a filter only
        // one of them has can't be joined, and its track goes).
        let mut props: Vec<Property> = Property::ALL.to_vec();
        for t in self.tracks.iter().chain(&next.tracks).filter(|t| t.property.is_filter()) {
            if !props.contains(&t.property) && own_value(base, t.property).is_some() && own_value(next_base, t.property).is_some() {
                props.push(t.property);
            }
        }
        props.sort_by_key(|p| p.index());
        for p in props {
            let (ta, tb) = (self.track(p), next.track(p));
            let (va, vb) = (own_value(base, p).unwrap_or(0.0), own_value(next_base, p).unwrap_or(0.0));
            if ta.is_none() && tb.is_none() && (va - vb).abs() <= SAME {
                continue;
            }
            // This span's part: its keys before the cut, and its value on its last frame when
            // a segment runs on past it.
            let mut keys: Vec<PropertyKey> = match ta {
                Some(t) => {
                    let mut v: Vec<PropertyKey> = t.keys.iter().filter(|k| k.frame < frames).cloned().collect();
                    let runs_on = t.keys.iter().any(|k| k.frame > last);
                    if (v.is_empty() || runs_on) && t.key_at(last).is_none() {
                        let at = if v.is_empty() { 0 } else { last };
                        let ease = t.segment_at(at).and_then(|i| t.keys.get(i)).map_or(Easing::Linear, |k| k.ease.clone());
                        v.push(PropertyKey { frame: at, value: t.value_at(at as f32).unwrap_or(va), ease });
                    }
                    v
                }
                None => vec![PropertyKey { frame: 0, value: va, ease: Easing::Linear }],
            };
            let end = keys.last().map_or(va, |k| k.value);
            // The next span's part, after the cut. Its rotation turns on from where this one
            // ended (the same look, whole turns added), so the curve doesn't spin back.
            let mut after: Vec<PropertyKey> = match tb {
                Some(t) => t.keys.iter().map(|k| PropertyKey { frame: k.frame.saturating_add(frames), ..k.clone() }).collect(),
                None => vec![PropertyKey { frame: frames, value: vb, ease: Easing::Linear }],
            };
            if p == Property::Rotation
                && let Some(first) = after.first()
            {
                let turns = ((end - first.value) / 360.0).round() * 360.0;
                for k in &mut after {
                    k.value = p.clamp(k.value + turns);
                }
            }
            let start = after.first().map_or(end, |k| k.value);
            if let Some(k) = keys.last_mut() {
                // The step at the cut: hold this span's value to its last frame, then jump.
                if (start - end).abs() > SAME {
                    if k.frame < last {
                        let hold = PropertyKey { frame: last, value: end, ease: Easing::Linear };
                        k.ease = Easing::Linear;
                        keys.push(hold);
                    } else {
                        k.ease = Easing::Linear;
                    }
                }
            }
            keys.extend(after);
            keys.truncate(super::MAX_KEYS);
            tracks.push(Track { property: p, keys });
        }
        Some(MotionTween { target: self.target, tracks, path: None, orient: self.orient, pins: Vec::new() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ColorEffect, ElementId, ElementKind, Shape};
    use anicraft_geom::{Affine, Path, Vec2};

    fn el(x: f32, y: f32) -> Element {
        Element {
            id: ElementId(1),
            name: String::new(),
            transform: Affine::translate(x, y),
            color: ColorEffect::default(),
            kind: ElementKind::Shape(Shape::filled(Path::rect(0.0, 0.0, 10.0, 10.0), crate::Paint::Solid(crate::Rgba::BLACK))),
        }
    }

    fn tween(tracks: &[(Property, &[(u32, f32)])]) -> MotionTween {
        let mut m = MotionTween::new(ElementId(1));
        for (p, keys) in tracks {
            m.tracks.push(Track { property: *p, keys: keys.iter().map(|&(frame, value)| PropertyKey { frame, value, ease: Easing::Linear }).collect() });
        }
        m
    }

    fn frames(m: &MotionTween, p: Property) -> Vec<u32> {
        m.track(p).map(|t| t.keys.iter().map(|k| k.frame).collect()).unwrap_or_default()
    }

    #[test]
    fn a_moved_key_keeps_its_value_and_ease() {
        let mut m = tween(&[(Property::X, &[(0, 0.0), (10, 100.0), (20, 50.0)])]);
        if let Some(k) = m.track_mut(Property::X).and_then(|t| t.keys.get_mut(1)) {
            k.ease = Easing::QuadIn;
        }
        let (by, now) = m.move_keys(&[(Property::X, 10)], 4, 20);
        assert_eq!((by, now), (4, vec![(Property::X, 14)]));
        let t = m.track(Property::X).expect("track");
        let k = t.keys.get(1).expect("key");
        assert_eq!((k.frame, k.value, &k.ease), (14, 100.0, &Easing::QuadIn));
        assert_eq!(t.value_at(14.0), Some(100.0));
    }

    #[test]
    fn no_key_crosses_or_lands_on_a_neighbour() {
        let mut m = tween(&[(Property::X, &[(0, 0.0), (10, 100.0), (20, 50.0)])]);
        // Right: up to the frame before the next key. Left: up to the frame after the first.
        assert_eq!(m.retime_limit(&[(Property::X, 10)], 50, 30), 9);
        assert_eq!(m.retime_limit(&[(Property::X, 10)], -50, 30), -9);
        // Never before the first frame, or after the last (the span's 30 frames).
        assert_eq!(m.retime_limit(&[(Property::X, 0)], -3, 30), 0);
        assert_eq!(m.retime_limit(&[(Property::X, 20)], 40, 25), 5);
        // Keys moving together don't bound each other.
        assert_eq!(m.retime_limit(&[(Property::X, 10), (Property::X, 20)], 40, 30), 10);
        m.move_keys(&[(Property::X, 10)], 1000, 30);
        assert_eq!(frames(&m, Property::X), vec![0, 19, 20]);
        // Fuzz: every move keeps every track strictly in order and inside the span.
        let mut seed = 7u32;
        let mut rnd = |n: u32| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (seed >> 8) % n
        };
        for _ in 0..500 {
            let p = if rnd(2) == 0 { Property::X } else { Property::Y };
            let pick: Vec<KeyRef> = (0..rnd(3) + 1).map(|_| (p, rnd(31))).collect();
            let by = i64::from(rnd(41)) - 20;
            let mut m2 = tween(&[(Property::X, &[(0, 0.0), (5, 1.0), (6, 2.0), (30, 3.0)]), (Property::Y, &[(3, 0.0), (9, 1.0)])]);
            m2.move_keys(&pick, by, 30);
            for t in &m2.tracks {
                assert!(t.keys.windows(2).all(|w| matches!(w, [a, b] if a.frame < b.frame)), "{pick:?} {by}: {:?}", t.keys);
                assert!(t.keys.iter().all(|k| k.frame <= 30));
            }
            assert!(m2.is_sane());
        }
    }

    #[test]
    fn keys_across_properties_move_together() {
        let mut m = tween(&[(Property::X, &[(0, 0.0), (10, 100.0)]), (Property::Rotation, &[(0, 0.0), (8, 90.0), (12, 180.0)])]);
        // The rotation key at 12 stops the X key at 10 going further than 3 frames.
        let (by, _) = m.move_keys(&[(Property::X, 10), (Property::Rotation, 8)], 6, 20);
        assert_eq!(by, 3);
        assert_eq!(frames(&m, Property::X), vec![0, 13]);
        assert_eq!(frames(&m, Property::Rotation), vec![0, 11, 12]);
    }

    #[test]
    fn a_pinned_frames_keys_and_pin_move_together() {
        let mut m = tween(&[(Property::X, &[(0, 0.0), (10, 50.0), (20, 100.0)]), (Property::Y, &[(0, 0.0), (10, 0.0), (20, 0.0)])]);
        let mut path = Path::new();
        path.move_to(Vec2::new(0.0, 0.0));
        path.cubic_to(Vec2::new(10.0, -20.0), Vec2::new(40.0, -20.0), Vec2::new(50.0, 0.0));
        path.cubic_to(Vec2::new(60.0, 20.0), Vec2::new(90.0, 20.0), Vec2::new(100.0, 0.0));
        m.path = Some(path);
        m.pins = vec![Pin { frame: 10, anchor: 1 }];
        assert_eq!(m.live_pins().len(), 1);
        // Picking only X still takes Y along: they are the point's place.
        let (by, now) = m.move_keys(&[(Property::X, 10)], 3, 20);
        assert_eq!(by, 3);
        assert_eq!(now, vec![(Property::X, 13), (Property::Y, 13)]);
        assert_eq!(m.pins, vec![Pin { frame: 13, anchor: 1 }]);
    }

    #[test]
    fn stretching_scales_keys_in_proportion() {
        // 21 frames (0..=20) to 41 (0..=40): every key doubles its offset.
        let mut m = tween(&[(Property::X, &[(0, 0.0), (5, 10.0), (10, 20.0), (20, 40.0)])]);
        let before = m.clone();
        m.stretch(21, 41);
        assert_eq!(frames(&m, Property::X), vec![0, 10, 20, 40]);
        // Values at the keys are kept, and between them the curve is the same, stretched.
        for f in 0..=20 {
            let (a, b) = (before.track(Property::X).and_then(|t| t.value_at(f as f32)), m.track(Property::X).and_then(|t| t.value_at(2.0 * f as f32)));
            assert_eq!(a, b, "frame {f}");
        }
        // And back: a round trip.
        m.stretch(41, 21);
        assert_eq!(m, before);
        // Analytic: 0..=30 into 0..=20 rounds f · 20/30 to the nearest frame.
        let mut m = tween(&[(Property::X, &[(0, 0.0), (7, 1.0), (15, 2.0), (30, 3.0)])]);
        m.stretch(31, 21);
        assert_eq!(frames(&m, Property::X), vec![0, 5, 10, 20]);
    }

    #[test]
    fn a_much_shorter_span_merges_keys_into_the_later_one() {
        // Spread first: 0, 1, 2, 10 of 0..=10 into 0..=4 land on 0, 0, 0, 4 and go to 0, 1, 2, 4.
        let mut m = tween(&[(Property::X, &[(0, 0.0), (1, 5.0), (2, 9.0), (10, 20.0)])]);
        let mut spread = m.clone();
        spread.stretch(11, 5);
        assert_eq!(frames(&spread, Property::X), vec![0, 1, 2, 4]);
        // Four keys in three frames: the first stays, the last two win the frames after it.
        m.stretch(11, 3);
        let t = m.track(Property::X).expect("track");
        assert_eq!(t.keys.iter().map(|k| (k.frame, k.value)).collect::<Vec<_>>(), vec![(0, 0.0), (1, 9.0), (2, 20.0)]);
        assert!(m.is_sane());
        // A one-frame span has nothing to stretch.
        let mut one = tween(&[(Property::X, &[(0, 0.0)])]);
        one.stretch(1, 30);
        assert_eq!(frames(&one, Property::X), vec![0]);
    }

    #[test]
    fn split_then_join_gives_the_tween_back() {
        let base = el(0.0, 0.0);
        let mut m = tween(&[(Property::X, &[(0, 0.0), (10, 100.0), (20, 0.0)]), (Property::Rotation, &[(0, 0.0), (20, 400.0)])]);
        if let Some(k) = m.track_mut(Property::X).and_then(|t| t.keys.first_mut()) {
            k.ease = Easing::CubicBezier(0.4, 0.0, 0.2, 1.0);
        }
        let props = Props::of(&base);
        let tail = m.split_at(&props, 6);
        let tail_base = m.apply(&base, 6.0);
        assert_eq!(m.join(&base, 6, &tail, &tail_base), Some(m.clone()));
    }

    #[test]
    fn joining_two_tweens_keeps_every_frame() {
        let a = el(0.0, 0.0);
        let ma = tween(&[(Property::X, &[(0, 0.0), (9, 90.0)])]);
        // The second span's object starts elsewhere and turns.
        let b = el(200.0, 10.0);
        let mb = tween(&[(Property::Rotation, &[(0, 0.0), (5, 45.0)])]);
        let j = ma.join(&a, 10, &mb, &b).expect("joins");
        for f in 0..10 {
            let want = ma.apply(&a, f as f32).transform;
            let got = j.apply(&a, f as f32).transform;
            assert!(want.m.iter().zip(got.m).all(|(w, g)| (w - g).abs() < 1e-3), "frame {f}: {want:?} {got:?}");
        }
        for f in 0..6 {
            let want = mb.apply(&b, f as f32).transform;
            let got = j.apply(&a, (10 + f) as f32).transform;
            assert!(want.m.iter().zip(got.m).all(|(w, g)| (w - g).abs() < 1e-3), "frame {}: {want:?} {got:?}", 10 + f);
        }
        assert!(j.is_sane());
        // Two motion paths don't join (yet).
        let mut pa = ma.clone();
        pa.path = Some(Path::rect(0.0, 0.0, 10.0, 10.0));
        assert_eq!(pa.join(&a, 10, &mb, &b), None);
    }
}
