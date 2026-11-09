//! Motion tweens (ADR 0014): one object's properties, each on its own curve.
//!
//! A classic tween interpolates a whole keyframe into the next one. A motion tween instead
//! belongs to one keyframe span and one object in it (its *target*). Each property (x, y,
//! scale, rotation, skew, alpha, tint) may have a [`Track`] of property keyframes anywhere in
//! the span; between two keys the value follows the segment's [`Easing`], which may be a custom
//! cubic Bézier. A property without a track keeps the target's own value (its *base*: how it
//! looks on the span's first frame). Evaluation is pure: [`MotionTween::apply`] maps the base
//! element and a frame offset to the element as it looks then.

use crate::{Element, ElementId, Easing};
use anicraft_geom::{Affine, Decomposed};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Most keys one track keeps (files and commands alike).
pub const MAX_KEYS: usize = 10_000;

/// A property a motion tween animates. Values are in the units the Motion Editor shows:
/// pixels, percent and degrees.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Property {
    /// Registration point, stage pixels.
    X,
    Y,
    /// Percent (100 = natural size; negative flips).
    ScaleX,
    ScaleY,
    /// Degrees, clockwise on screen; not wrapped, so 0 → 720 turns twice.
    Rotation,
    /// Degrees of horizontal skew, within ±89.
    Skew,
    /// Colour effect: opacity and tint amount, percent.
    Alpha,
    Tint,
}

impl Property {
    pub const ALL: [Property; 8] = [Property::X, Property::Y, Property::ScaleX, Property::ScaleY, Property::Rotation, Property::Skew, Property::Alpha, Property::Tint];

    /// The name used in files and commands (`x`, `scale_x`, `alpha`…).
    pub fn key(self) -> &'static str {
        match self {
            Property::X => "x",
            Property::Y => "y",
            Property::ScaleX => "scale_x",
            Property::ScaleY => "scale_y",
            Property::Rotation => "rotation",
            Property::Skew => "skew",
            Property::Alpha => "alpha",
            Property::Tint => "tint",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Property::X => "X",
            Property::Y => "Y",
            Property::ScaleX => "Scale X",
            Property::ScaleY => "Scale Y",
            Property::Rotation => "Rotation",
            Property::Skew => "Skew",
            Property::Alpha => "Alpha",
            Property::Tint => "Tint",
        }
    }

    pub fn unit(self) -> &'static str {
        match self {
            Property::X | Property::Y => "px",
            Property::ScaleX | Property::ScaleY | Property::Alpha | Property::Tint => "%",
            Property::Rotation | Property::Skew => "°",
        }
    }

    /// The Motion Editor's groups: Position, Transform, Colour effect.
    pub fn group(self) -> &'static str {
        match self {
            Property::X | Property::Y => "Position",
            Property::ScaleX | Property::ScaleY | Property::Rotation | Property::Skew => "Transform",
            Property::Alpha | Property::Tint => "Colour effect",
        }
    }

    pub fn from_key(s: &str) -> Option<Property> {
        let s = s.trim().to_ascii_lowercase().replace([' ', '-'], "_");
        let s = match s.as_str() {
            "scalex" => "scale_x",
            "scaley" => "scale_y",
            "rotate" => "rotation",
            "opacity" => "alpha",
            other => other,
        };
        Property::ALL.into_iter().find(|p| p.key() == s)
    }

    /// Values a key may hold (commands reject others; files are clamped into it).
    pub fn range(self) -> std::ops::RangeInclusive<f32> {
        match self {
            Property::X | Property::Y => -1_000_000.0..=1_000_000.0,
            Property::ScaleX | Property::ScaleY => -1_000_000.0..=1_000_000.0,
            Property::Rotation => -100_000.0..=100_000.0,
            Property::Skew => -89.0..=89.0,
            Property::Alpha | Property::Tint => 0.0..=100.0,
        }
    }

    fn index(self) -> usize {
        Property::ALL.iter().position(|p| *p == self).unwrap_or(0)
    }

    fn is_transform(self) -> bool {
        !matches!(self, Property::Alpha | Property::Tint)
    }

    fn clamp(self, v: f32) -> f32 {
        let r = self.range();
        if v.is_finite() { v.clamp(*r.start(), *r.end()) } else { 0.0 }
    }
}

/// Every property's value, indexed like [`Property::ALL`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Props(pub [f32; 8]);

impl Props {
    /// An element's own values.
    pub fn of(el: &Element) -> Props {
        let d = el.transform.decompose();
        Props([d.tx, d.ty, d.sx * 100.0, d.sy * 100.0, d.rotation.to_degrees(), d.skew.atan().to_degrees(), el.color.alpha * 100.0, el.color.tint_amount * 100.0])
    }
    pub fn get(&self, p: Property) -> f32 {
        self.0.get(p.index()).copied().unwrap_or(0.0)
    }
    pub fn set(&mut self, p: Property, v: f32) {
        if let Some(x) = self.0.get_mut(p.index()) {
            *x = v;
        }
    }
    /// The transform these values describe.
    pub fn transform(&self) -> Affine {
        // Scale never reaches zero (that would collapse the artwork and lose its rotation).
        let scale = |pct: f32| {
            let s = Property::ScaleX.clamp(pct) / 100.0;
            if s.abs() < 1e-4 { 1e-4f32.copysign(s) } else { s }
        };
        Decomposed {
            tx: Property::X.clamp(self.get(Property::X)),
            ty: Property::Y.clamp(self.get(Property::Y)),
            sx: scale(self.get(Property::ScaleX)),
            sy: scale(self.get(Property::ScaleY)),
            rotation: Property::Rotation.clamp(self.get(Property::Rotation)).to_radians(),
            skew: Property::Skew.clamp(self.get(Property::Skew)).to_radians().tan(),
        }
        .compose()
    }
}

/// A property keyframe.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PropertyKey {
    /// Frames from the span's first frame.
    pub frame: u32,
    pub value: f32,
    /// How the value travels from this key to the next one.
    #[serde(default, skip_serializing_if = "is_linear")]
    pub ease: Easing,
}

fn is_linear(e: &Easing) -> bool {
    *e == Easing::Linear
}

/// One property's keys, sorted by frame, one per frame.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Track {
    pub property: Property,
    #[serde(default)]
    pub keys: Vec<PropertyKey>,
}

impl Track {
    /// The value at `frame` (fractional frames sample the curve between frames): the first
    /// key's value before it, the last key's after it, the segment's ease in between.
    pub fn value_at(&self, frame: f32) -> Option<f32> {
        let first = self.keys.first()?;
        let last = self.keys.last()?;
        if !frame.is_finite() || frame <= first.frame as f32 {
            return Some(first.value);
        }
        if frame >= last.frame as f32 {
            return Some(last.value);
        }
        // The segment [a, b) holding `frame`: binary search on the key frames.
        let i = self.keys.partition_point(|k| k.frame as f32 <= frame);
        let (a, b) = (self.keys.get(i.checked_sub(1)?)?, self.keys.get(i)?);
        let span = (b.frame - a.frame).max(1) as f32;
        let t = a.ease.apply((frame - a.frame as f32) / span);
        Some(a.value + (b.value - a.value) * t)
    }

    /// Index of the key at exactly `frame`.
    pub fn key_at(&self, frame: u32) -> Option<usize> {
        self.keys.binary_search_by_key(&frame, |k| k.frame).ok()
    }

    /// Index of the key starting the segment that holds `frame` (`None` at or after the last
    /// key, where no segment follows).
    pub fn segment_at(&self, frame: u32) -> Option<usize> {
        let i = self.keys.partition_point(|k| k.frame <= frame).checked_sub(1)?;
        (i + 1 < self.keys.len()).then_some(i)
    }

    /// Sets the key at `frame` (keeping its ease), or adds one that eases like the segment it
    /// splits.
    pub fn set(&mut self, frame: u32, value: f32) {
        let value = self.property.clamp(value);
        match self.keys.binary_search_by_key(&frame, |k| k.frame) {
            Ok(i) => {
                if let Some(k) = self.keys.get_mut(i) {
                    k.value = value;
                }
            }
            Err(i) => {
                let ease = i.checked_sub(1).and_then(|p| self.keys.get(p)).map_or(Easing::Linear, |k| k.ease);
                if self.keys.len() < MAX_KEYS {
                    self.keys.insert(i, PropertyKey { frame, value, ease });
                }
            }
        }
    }

    fn sanitize(&mut self) {
        self.keys.retain(|k| k.value.is_finite());
        for k in &mut self.keys {
            k.value = self.property.clamp(k.value);
        }
        if !self.keys.is_sorted_by_key(|k| k.frame) {
            self.keys.sort_by_key(|k| k.frame);
        }
        self.keys.dedup_by_key(|k| k.frame);
        self.keys.truncate(MAX_KEYS);
    }

    fn is_sane(&self) -> bool {
        self.keys.len() <= MAX_KEYS && self.keys.windows(2).all(|w| matches!(w, [a, b] if a.frame < b.frame)) && self.keys.iter().all(|k| self.property.range().contains(&k.value))
    }
}

/// A motion tween on a keyframe span (see the module docs).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MotionTween {
    /// The object it animates. When no element has this id but the keyframe holds exactly one
    /// element, that one is the target (so edits that replace the object keep its motion).
    pub target: ElementId,
    /// At most one per property, in [`Property::ALL`] order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tracks: Vec<Track>,
}

impl MotionTween {
    pub fn new(target: ElementId) -> MotionTween {
        MotionTween { target, tracks: Vec::new() }
    }

    pub fn track(&self, p: Property) -> Option<&Track> {
        self.tracks.iter().find(|t| t.property == p)
    }

    pub fn track_mut(&mut self, p: Property) -> Option<&mut Track> {
        self.tracks.iter_mut().find(|t| t.property == p)
    }

    /// Index of the target among a keyframe's elements.
    pub fn target_index(&self, elements: &[Arc<Element>]) -> Option<usize> {
        elements.iter().position(|e| e.id == self.target).or((elements.len() == 1).then_some(0))
    }

    /// Every property's value at `frame` frames into the span, for an object whose own values
    /// are `base`.
    pub fn props_at(&self, base: &Props, frame: f32) -> Props {
        let mut out = *base;
        for t in &self.tracks {
            if let Some(v) = t.value_at(frame) {
                out.set(t.property, v);
            }
        }
        out
    }

    /// The target as it looks `frame` frames into the span. Untracked properties keep the
    /// base's exact transform and colour effect.
    pub fn apply(&self, base: &Element, frame: f32) -> Element {
        let mut el = base.clone();
        if self.tracks.is_empty() {
            return el;
        }
        let props = self.props_at(&Props::of(base), frame);
        if self.tracks.iter().any(|t| t.property.is_transform()) {
            let m = props.transform();
            if m.is_finite() {
                el.transform = m;
            }
        }
        if let Some(t) = self.track(Property::Alpha).and_then(|t| t.value_at(frame)) {
            el.color.alpha = (t / 100.0).clamp(0.0, 1.0);
        }
        if let Some(t) = self.track(Property::Tint).and_then(|t| t.value_at(frame)) {
            el.color.tint_amount = (t / 100.0).clamp(0.0, 1.0);
        }
        el
    }

    /// Sets property `p`'s key at `frame`. A property's first key also gets a key on the span's
    /// first frame holding `base` (its value until now), so the rest of the span is unchanged.
    pub fn set_key(&mut self, p: Property, frame: u32, value: f32, base: f32) {
        if self.track(p).is_none() {
            let mut keys = Vec::new();
            if frame > 0 {
                keys.push(PropertyKey { frame: 0, value: p.clamp(base), ease: Easing::Linear });
            }
            let at = Property::ALL.iter().position(|x| *x == p).unwrap_or(0);
            let pos = self.tracks.partition_point(|t| Property::ALL.iter().position(|x| *x == t.property).unwrap_or(0) < at);
            self.tracks.insert(pos, Track { property: p, keys });
        }
        if let Some(t) = self.track_mut(p) {
            t.set(frame, value);
        }
    }

    /// Removes property `p`'s key at `frame`; a track left with no keys goes. Whether a key was
    /// there.
    pub fn remove_key(&mut self, p: Property, frame: u32) -> bool {
        let Some(t) = self.track_mut(p) else { return false };
        let Some(i) = t.key_at(frame) else { return false };
        t.keys.remove(i);
        self.tracks.retain(|t| !t.keys.is_empty());
        true
    }

    /// Frames (from the span's start) that hold a key of any property, sorted.
    pub fn key_frames(&self) -> Vec<u32> {
        let mut v: Vec<u32> = self.tracks.iter().flat_map(|t| t.keys.iter().map(|k| k.frame)).collect();
        v.sort_unstable();
        v.dedup();
        v
    }

    /// The part of this tween from `frame` on, re-based to start there (splitting a span in
    /// two keeps the motion: each track gets a key at the cut with the value it had there).
    pub fn split_at(&self, frame: u32) -> MotionTween {
        let tracks = self
            .tracks
            .iter()
            .filter_map(|t| {
                let v = t.value_at(frame as f32)?;
                let ease = t.segment_at(frame).and_then(|i| t.keys.get(i)).map_or(Easing::Linear, |k| k.ease);
                let mut keys = vec![PropertyKey { frame: 0, value: v, ease }];
                keys.extend(t.keys.iter().filter(|k| k.frame > frame).map(|k| PropertyKey { frame: k.frame - frame, ..*k }));
                Some(Track { property: t.property, keys })
            })
            .collect();
        MotionTween { target: self.target, tracks }
    }

    /// Whether loading must repair it (unsorted or duplicate keys, values out of range,
    /// duplicate or empty tracks).
    pub fn is_sane(&self) -> bool {
        self.tracks.iter().all(|t| !t.keys.is_empty() && t.is_sane()) && self.tracks.windows(2).all(|w| matches!(w, [a, b] if a.property.index() < b.property.index()))
    }

    /// Repairs a loaded tween.
    pub fn sanitize(&mut self) {
        for t in &mut self.tracks {
            t.sanitize();
        }
        self.tracks.retain(|t| !t.keys.is_empty());
        self.tracks.sort_by_key(|t| t.property.index());
        self.tracks.dedup_by_key(|t| t.property);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ColorEffect, ElementKind, Shape};
    use anicraft_geom::Path;

    fn el(id: u64, x: f32, y: f32) -> Element {
        Element {
            id: ElementId(id),
            name: String::new(),
            transform: Affine::translate(x, y),
            color: ColorEffect::default(),
            kind: ElementKind::Shape(Shape::filled(Path::rect(0.0, 0.0, 10.0, 10.0), crate::Paint::Solid(crate::Rgba::BLACK))),
        }
    }

    fn track(p: Property, keys: &[(u32, f32, Easing)]) -> Track {
        Track { property: p, keys: keys.iter().map(|&(frame, value, ease)| PropertyKey { frame, value, ease }).collect() }
    }

    #[test]
    fn a_track_holds_before_and_after_and_eases_between() {
        let t = track(Property::X, &[(4, 100.0, Easing::Linear), (8, 200.0, Easing::QuadIn), (12, 0.0, Easing::Linear)]);
        assert_eq!(t.value_at(0.0), Some(100.0));
        assert_eq!(t.value_at(4.0), Some(100.0));
        assert_eq!(t.value_at(6.0), Some(150.0), "linear midpoint");
        // QuadIn on 8..12: a quarter of the way there at half time.
        assert_eq!(t.value_at(10.0), Some(200.0 - 200.0 * 0.25));
        assert_eq!(t.value_at(12.0), Some(0.0));
        assert_eq!(t.value_at(99.0), Some(0.0));
        assert_eq!(t.value_at(f32::NAN), Some(100.0));
        assert_eq!(track(Property::X, &[]).value_at(3.0), None);
        assert_eq!(t.segment_at(3), None);
        assert_eq!(t.segment_at(4), Some(0));
        assert_eq!(t.segment_at(11), Some(1));
        assert_eq!(t.segment_at(12), None);
    }

    #[test]
    fn a_custom_bezier_ease_matches_its_analytic_curve() {
        // Control points (0, 0) (0, 1) (1, 0) (1, 1): x(s) = 3s² − 2s³, y(s) = 3s(1 − s)² + s³;
        // the value at frame 10·x(s) must be 100·y(s).
        let e = Easing::CubicBezier(0.0, 1.0, 1.0, 0.0);
        let t = track(Property::Y, &[(0, 0.0, e), (10, 100.0, Easing::Linear)]);
        for s in [0.1f32, 0.3, 0.5, 0.8] {
            let bez = |p1: f32, p2: f32| 3.0 * (1.0 - s) * (1.0 - s) * s * p1 + 3.0 * (1.0 - s) * s * s * p2 + s * s * s;
            let (x, y) = (bez(0.0, 1.0), bez(1.0, 0.0));
            let v = t.value_at(x * 10.0).unwrap_or(f32::NAN);
            assert!((v - y * 100.0).abs() < 0.05, "s {s}: {v} vs {}", y * 100.0);
        }
    }

    #[test]
    fn untracked_properties_keep_the_base_exactly() {
        let base = Element { transform: Affine::translate(5.0, 6.0).mul(&Affine::rotate(0.3)).mul(&Affine::scale(2.0, 0.5)), ..el(1, 0.0, 0.0) };
        let mut m = MotionTween::new(ElementId(1));
        assert_eq!(m.apply(&base, 3.0), base, "no tracks: the base itself");
        m.set_key(Property::Alpha, 10, 0.0, 100.0);
        let at5 = m.apply(&base, 5.0);
        assert_eq!(at5.transform, base.transform, "colour-only tween keeps the transform bit for bit");
        assert!((at5.color.alpha - 0.5).abs() < 1e-6);
    }

    #[test]
    fn the_first_key_of_a_property_pins_the_span_start_to_the_base() {
        let base = el(1, 50.0, 20.0);
        let mut m = MotionTween::new(ElementId(1));
        m.set_key(Property::X, 10, 150.0, 50.0);
        assert_eq!(m.track(Property::X).map(|t| t.keys.len()), Some(2));
        let x = |f: f32| m.apply(&base, f).transform.translation().x;
        assert!((x(0.0) - 50.0).abs() < 1e-3 && (x(5.0) - 100.0).abs() < 1e-3 && (x(10.0) - 150.0).abs() < 1e-3);
        // Y untouched, and a key added later in a segment eases like the segment it splits.
        assert!((m.apply(&base, 5.0).transform.translation().y - 20.0).abs() < 1e-3);
        if let Some(t) = m.track_mut(Property::X) {
            t.keys[0].ease = Easing::CubicOut;
        }
        m.set_key(Property::X, 4, 70.0, 50.0);
        assert_eq!(m.track(Property::X).map(|t| t.keys[1].ease), Some(Easing::CubicOut));
        assert!(m.remove_key(Property::X, 4) && !m.remove_key(Property::X, 4));
        assert!(m.remove_key(Property::X, 0) && m.remove_key(Property::X, 10));
        assert!(m.tracks.is_empty(), "an emptied track goes");
    }

    #[test]
    fn rotation_and_scale_compose_like_the_element_transform() {
        let base = el(1, 10.0, 10.0);
        let mut m = MotionTween::new(ElementId(1));
        m.set_key(Property::Rotation, 8, 360.0, 0.0);
        m.set_key(Property::ScaleX, 8, 300.0, 100.0);
        let d = m.apply(&base, 4.0).transform.decompose();
        assert!((d.rotation.to_degrees() - 180.0).abs() < 1e-2 || (d.rotation.to_degrees() + 180.0).abs() < 1e-2, "half of a whole turn");
        assert!((d.sx - 2.0).abs() < 1e-4 && (d.sy - 1.0).abs() < 1e-4);
        assert!((d.tx - 10.0).abs() < 1e-4);
        // Scale through zero never collapses the artwork.
        m.set_key(Property::ScaleY, 8, -100.0, 100.0);
        assert!(m.apply(&base, 4.0).transform.determinant().abs() > 0.0);
    }

    #[test]
    fn splitting_keeps_the_motion() {
        let base = el(1, 0.0, 0.0);
        let mut m = MotionTween::new(ElementId(1));
        m.set_key(Property::X, 10, 100.0, 0.0);
        m.set_key(Property::Y, 6, 60.0, 0.0);
        let tail = m.split_at(4);
        let tail_base = m.apply(&base, 4.0);
        for f in 4..=12u32 {
            let a = m.apply(&base, f as f32).transform.translation();
            let b = tail.apply(&tail_base, (f - 4) as f32).transform.translation();
            assert!((a.x - b.x).abs() < 1e-3 && (a.y - b.y).abs() < 1e-3, "frame {f}: {a:?} vs {b:?}");
        }
    }

    #[test]
    fn the_target_falls_back_to_a_sole_element() {
        let m = MotionTween::new(ElementId(9));
        assert_eq!(m.target_index(&[Arc::new(el(1, 0.0, 0.0))]), Some(0));
        assert_eq!(m.target_index(&[Arc::new(el(1, 0.0, 0.0)), Arc::new(el(9, 0.0, 0.0))]), Some(1));
        assert_eq!(m.target_index(&[Arc::new(el(1, 0.0, 0.0)), Arc::new(el(2, 0.0, 0.0))]), None);
    }

    #[test]
    fn sanitize_repairs_a_hostile_tween() {
        let mut m = MotionTween {
            target: ElementId(1),
            tracks: vec![
                track(Property::Alpha, &[(5, 500.0, Easing::Linear), (1, f32::NAN, Easing::Linear), (2, 10.0, Easing::Linear), (2, 20.0, Easing::Linear)]),
                track(Property::X, &[(0, 1.0, Easing::Linear)]),
                track(Property::X, &[(0, 2.0, Easing::Linear)]),
                track(Property::Y, &[]),
            ],
        };
        assert!(!m.is_sane());
        m.sanitize();
        assert!(m.is_sane());
        assert_eq!(m.tracks.iter().map(|t| t.property).collect::<Vec<_>>(), vec![Property::X, Property::Alpha]);
        assert_eq!(m.track(Property::Alpha).map(|t| t.keys.iter().map(|k| (k.frame, k.value)).collect::<Vec<_>>()), Some(vec![(2, 10.0), (5, 100.0)]));
    }

    #[test]
    fn property_keys_round_trip() {
        for p in Property::ALL {
            assert_eq!(Property::from_key(p.key()), Some(p));
            assert!(!p.label().is_empty() && !p.unit().is_empty() && !p.group().is_empty());
        }
        assert_eq!(Property::from_key("Scale X"), Some(Property::ScaleX));
        assert_eq!(Property::from_key("nope"), None);
    }
}
