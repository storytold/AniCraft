//! Motion tweens (ADR 0016): one object's properties, each on its own curve.
//!
//! A classic tween interpolates a whole keyframe into the next one. A motion tween instead
//! belongs to one keyframe span and one object in it (its *target*). Each property (x, y,
//! scale, rotation, skew, alpha, tint) may have a [`Track`] of property keyframes anywhere in
//! the span; between two keys the value follows the segment's [`Easing`], which may be a custom
//! cubic Bézier. A property without a track keeps the target's own value (its *base*: how it
//! looks on the span's first frame). Evaluation is pure: [`MotionTween::apply`] maps the base
//! element and a frame offset to the element as it looks then.
//!
//! A tween may also carry a *motion path* (ADR 0030): a cubic Bézier path in the layer's space
//! that the registration point travels along by arc length. The X/Y curves then decide only
//! *when* it gets where: the share of the way along the path at a frame is the share of the
//! distance the X/Y curves have covered by then. With *orient to path* the object also turns
//! as the path turns. Without a path the X/Y curves place the object, as before.

use crate::{Element, ElementId, Easing};
use anicraft_geom::walk::CurveWalk;
use anicraft_geom::{Affine, Decomposed, Path, PathCmd, Vec2};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Most keys one track keeps (files and commands alike).
pub const MAX_KEYS: usize = 10_000;

/// Most segments a motion path keeps (files and commands alike).
pub const MAX_PATH_SEGMENTS: usize = 1_000;

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
    /// The motion path (ADR 0030), in the layer's space: a move and cubics (see
    /// [`motion_path`]). Its ends are the start and end positions; the X/Y curves give the
    /// timing along it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<Path>,
    /// Orient to path: the object turns as the path turns (relative to its turn at the start).
    #[serde(default, skip_serializing_if = "is_false")]
    pub orient: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// A path in the form a motion path keeps: its first subpath as one move and cubics (lines and
/// quadratics raised exactly, a closing edge written out), at most [`MAX_PATH_SEGMENTS`],
/// inside the X/Y range. `None` when nothing of some length is left.
pub fn motion_path(p: &Path) -> Option<Path> {
    let r = Property::X.range();
    let ok = |v: Vec2| v.is_finite() && r.contains(&v.x) && r.contains(&v.y);
    let mut cmds: Vec<PathCmd> = Vec::new();
    let (mut cur, mut start) = (Vec2::ZERO, Vec2::ZERO);
    for c in &p.cmds {
        if cmds.len() > MAX_PATH_SEGMENTS {
            break;
        }
        let seg = match *c {
            // A move before any segment (re)starts the path; one after them ends it.
            PathCmd::MoveTo(q) if cmds.len() <= 1 => {
                if !ok(q) {
                    return None;
                }
                (cur, start) = (q, q);
                cmds = vec![PathCmd::MoveTo(q)];
                continue;
            }
            PathCmd::MoveTo(_) => break,
            PathCmd::LineTo(q) => PathCmd::CubicTo(cur.lerp(q, 1.0 / 3.0), cur.lerp(q, 2.0 / 3.0), q),
            PathCmd::QuadTo(c1, q) => PathCmd::CubicTo(cur + (c1 - cur) * (2.0 / 3.0), q + (c1 - q) * (2.0 / 3.0), q),
            PathCmd::CubicTo(..) => *c,
            PathCmd::Close => {
                if cur.distance(start) > 1e-6 && !cmds.is_empty() {
                    cmds.push(PathCmd::CubicTo(cur.lerp(start, 1.0 / 3.0), cur.lerp(start, 2.0 / 3.0), start));
                }
                break;
            }
        };
        let PathCmd::CubicTo(a, b, q) = seg else { continue };
        if !(ok(a) && ok(b) && ok(q)) {
            return None;
        }
        if cmds.is_empty() {
            cmds.push(PathCmd::MoveTo(cur));
        }
        cmds.push(seg);
        cur = q;
    }
    let out = Path { cmds };
    CurveWalk::new(&out).map(|_| out)
}

/// The first and last anchors of a path.
pub fn path_ends(p: &Path) -> Option<(Vec2, Vec2)> {
    let anchors = p.anchors();
    Some((anchors.first()?.1, anchors.last()?.1))
}

/// A motion path measured for one object: where the object is along it, frame by frame.
///
/// Timing: the position keys (frames holding an X or Y key) split the span into legs. On each
/// leg the X/Y curves run from one key's position to the next; how far along that chord they
/// are (signed, so an ease that overshoots or backs up does so along the path too) is how far
/// along the path's matching share the object is. The shares are the chords' lengths. A
/// straight path therefore reproduces the X/Y curves exactly.
#[derive(Clone, Debug)]
pub struct OnPath<'a> {
    tween: &'a MotionTween,
    base: Props,
    walk: CurveWalk,
    /// Position keys: frame and where the X/Y curves are then.
    keys: Vec<(f32, Vec2)>,
    /// `cum[i]`: chord length from the first position key to key `i`.
    cum: Vec<f32>,
    /// Direction of travel at the start, radians.
    start_angle: f32,
}

impl OnPath<'_> {
    pub fn walk(&self) -> &CurveWalk {
        &self.walk
    }

    /// The share of the path covered `frame` frames into the span: 0 up to the first position
    /// key (and without two), 1 from the last; an overshooting ease may leave 0..1 in between.
    /// Where the chords add up to nothing (a path back to its start) the object keeps an even
    /// pace between the first and last position keys.
    pub fn progress(&self, frame: f32) -> f32 {
        let (Some(first), Some(last)) = (self.keys.first(), self.keys.last()) else { return 0.0 };
        if !frame.is_finite() || frame <= first.0 || last.0 <= first.0 {
            return 0.0;
        }
        if frame >= last.0 {
            return 1.0;
        }
        let total = self.cum.last().copied().unwrap_or(0.0);
        if total <= 1e-4 {
            return (frame - first.0) / (last.0 - first.0);
        }
        let i = self.keys.partition_point(|k| k.0 <= frame).saturating_sub(1);
        let (Some(a), Some(b)) = (self.keys.get(i), self.keys.get(i + 1)) else { return 1.0 };
        let chord = b.1 - a.1;
        let len = chord.length();
        let along = if len > 1e-6 { (self.tween.trace_at(&self.base, frame) - a.1).dot(chord) / len } else { 0.0 };
        let p = (self.cum.get(i).copied().unwrap_or(0.0) + along) / total;
        if p.is_finite() { p } else { 0.0 }
    }

    /// Distance along the path at `frame` (beyond the ends while an ease overshoots them).
    pub fn distance(&self, frame: f32) -> f32 {
        self.progress(frame) * self.walk.length()
    }

    /// The registration point at `frame`. Overshooting an end carries on along the path's
    /// direction there.
    pub fn point(&self, frame: f32) -> Vec2 {
        let (s, l) = (self.distance(frame), self.walk.length());
        if s < 0.0 {
            self.walk.at(0.0) + self.walk.tangent(0.0) * s
        } else if s > l {
            self.walk.at(l) + self.walk.tangent(l) * (s - l)
        } else {
            self.walk.at(s)
        }
    }

    /// How far the path has turned at `frame` since its start, degrees clockwise on screen
    /// (within ±180).
    pub fn turn(&self, frame: f32) -> f32 {
        let d = self.walk.tangent(self.distance(frame));
        let a = (d.y.atan2(d.x) - self.start_angle).to_degrees();
        let a = (a + 180.0).rem_euclid(360.0) - 180.0;
        if a.is_finite() { a } else { 0.0 }
    }
}

impl MotionTween {
    pub fn new(target: ElementId) -> MotionTween {
        MotionTween { target, tracks: Vec::new(), path: None, orient: false }
    }

    /// The first and last frames holding an X or Y key.
    pub fn position_range(&self) -> Option<(u32, u32)> {
        let keys = || [Property::X, Property::Y].into_iter().filter_map(|p| self.track(p)).flat_map(|t| t.keys.iter().map(|k| k.frame));
        Some((keys().min()?, keys().max()?))
    }

    /// Where the X/Y curves alone put the registration point at `frame`.
    fn trace_at(&self, base: &Props, frame: f32) -> Vec2 {
        let v = |p: Property| self.track(p).and_then(|t| t.value_at(frame)).unwrap_or(base.get(p));
        Vec2::new(v(Property::X), v(Property::Y))
    }

    /// The motion path measured for an object whose own values are `base`; `None` without a
    /// path.
    pub fn on_path(&self, base: &Props) -> Option<OnPath<'_>> {
        let walk = CurveWalk::new(self.path.as_ref()?)?;
        let mut frames: Vec<u32> = [Property::X, Property::Y].into_iter().filter_map(|p| self.track(p)).flat_map(|t| t.keys.iter().map(|k| k.frame)).collect();
        frames.sort_unstable();
        frames.dedup();
        let keys: Vec<(f32, Vec2)> = frames.into_iter().map(|f| (f as f32, self.trace_at(base, f as f32))).collect();
        let mut cum = Vec::with_capacity(keys.len());
        let mut d = 0.0f32;
        for (i, k) in keys.iter().enumerate() {
            d += i.checked_sub(1).and_then(|p| keys.get(p)).map_or(0.0, |p| p.1.distance(k.1));
            cum.push(d);
        }
        let t = walk.tangent(0.0);
        Some(OnPath { tween: self, base: *base, start_angle: t.y.atan2(t.x), walk, keys, cum })
    }

    /// The registration point on each of the span's first `frames` frames (at most a day of
    /// frames): what the stage draws, one dot per frame.
    pub fn positions(&self, base: &Props, frames: u32) -> Vec<Vec2> {
        let frames = frames.min(100_000);
        match self.on_path(base) {
            Some(op) => (0..frames).map(|f| op.point(f as f32)).collect(),
            None => (0..frames).map(|f| self.trace_at(base, f as f32)).collect(),
        }
    }

    /// A motion path through the X/Y curves' positions at their keys, as straight segments:
    /// the path a tween without one shows, made editable. `None` when the object doesn't move.
    pub fn path_from_tracks(&self, base: &Props) -> Option<Path> {
        let mut frames: Vec<u32> = [Property::X, Property::Y].into_iter().filter_map(|p| self.track(p)).flat_map(|t| t.keys.iter().map(|k| k.frame)).collect();
        frames.sort_unstable();
        frames.dedup();
        let mut pts: Vec<Vec2> = Vec::new();
        for f in frames.into_iter().take(MAX_PATH_SEGMENTS + 1) {
            let q = self.trace_at(base, f as f32);
            if pts.last().is_none_or(|l| l.distance(q) > 1e-3) {
                pts.push(q);
            }
        }
        let (first, rest) = pts.split_first()?;
        let mut p = Path::new();
        p.move_to(*first);
        for q in rest {
            p.line_to(*q);
        }
        motion_path(&p)
    }

    /// Gives the tween motion path `path` (made a [`motion_path`]; false when nothing usable
    /// is left). Its ends become the X/Y keys at the first and last position keys, or at the
    /// span's first frame and frame `last` when there are none, so the curves start and end
    /// where the path does.
    pub fn set_path(&mut self, path: &Path, base: &Props, last: u32) -> bool {
        let Some(p) = motion_path(path) else { return false };
        let Some((a, b)) = path_ends(&p) else { return false };
        let (f0, f1) = match self.position_range() {
            Some((f0, f1)) if f1 > f0 => (f0, f1),
            _ if last > 0 => (0, last),
            _ => return false,
        };
        for (prop, v0, v1) in [(Property::X, a.x, b.x), (Property::Y, a.y, b.y)] {
            // Ends already in place keep their keys exactly (bending the path keeps its timing).
            let holds = |m: &MotionTween, f: u32, v: f32| m.track(prop).and_then(|t| t.key_at(f).and_then(|i| t.keys.get(i))).is_some_and(|k| k.value == v);
            if !holds(self, f0, v0) {
                self.set_key(prop, f0, v0, base.get(prop));
            }
            if !holds(self, f1, v1) {
                self.set_key(prop, f1, v1, base.get(prop));
            }
        }
        self.path = Some(p);
        true
    }

    /// Moves the whole motion by `d`: the path and every X/Y key, so the object travels the same
    /// way, `d` over, on the same frames (the timing compares positions only with each other).
    /// An axis without keys follows the object's own position, which the caller moves. False,
    /// with nothing changed, when a key or the path would leave the X/Y range.
    pub fn translate(&mut self, d: Vec2) -> bool {
        if !d.is_finite() {
            return false;
        }
        let path = match &self.path {
            Some(p) => match motion_path(&p.transformed(&Affine::translate(d.x, d.y))) {
                Some(moved) => Some(moved),
                None => return false,
            },
            None => None,
        };
        let shift = |p: Property| if p == Property::X { d.x } else { d.y };
        let fits = [Property::X, Property::Y].into_iter().all(|p| self.track(p).is_none_or(|t| t.keys.iter().all(|k| p.range().contains(&(k.value + shift(p))))));
        if !fits {
            return false;
        }
        for p in [Property::X, Property::Y] {
            if let Some(t) = self.track_mut(p) {
                for k in &mut t.keys {
                    k.value += shift(p);
                }
            }
        }
        if path.is_some() {
            self.path = path;
        }
        true
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
        if self.tracks.is_empty() && self.path.is_none() {
            return el;
        }
        let base_props = Props::of(base);
        let mut props = self.props_at(&base_props, frame);
        let on_path = self.on_path(&base_props);
        if let Some(op) = &on_path {
            let p = op.point(frame);
            props.set(Property::X, p.x);
            props.set(Property::Y, p.y);
            if self.orient {
                props.set(Property::Rotation, props.get(Property::Rotation) + op.turn(frame));
            }
        }
        if on_path.is_some() || self.tracks.iter().any(|t| t.property.is_transform()) {
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
    /// two keeps the motion: each track gets a key at the cut with the value it had there, and
    /// the motion path is cut where the object is). `base` is the object's own values; the
    /// second half's object must be this one as it looks at `frame`.
    pub fn split_at(&self, base: &Props, frame: u32) -> MotionTween {
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
        let mut tail = MotionTween { target: self.target, tracks, path: None, orient: self.orient };
        let Some(op) = self.on_path(base) else { return tail };
        let f = frame as f32;
        match op.walk().tail(op.distance(f)) {
            // The cut path starts where the object is; the X/Y keys go on timing it.
            Some(p) => tail.path = Some(p),
            // Past the path's end: the object holds where the path ended.
            None => {
                let p = op.point(f);
                tail.tracks.retain(|t| !matches!(t.property, Property::X | Property::Y));
                tail.set_key(Property::X, 0, p.x, p.x);
                tail.set_key(Property::Y, 0, p.y, p.y);
            }
        }
        // What the path has turned the object so far, the second half starts from.
        if self.orient
            && let Some(t) = tail.track_mut(Property::Rotation)
        {
            let turn = op.turn(f);
            for k in &mut t.keys {
                k.value = Property::Rotation.clamp(k.value + turn);
            }
        }
        tail
    }

    /// Whether loading must repair it (unsorted or duplicate keys, values out of range,
    /// duplicate or empty tracks).
    pub fn is_sane(&self) -> bool {
        self.path.as_ref().is_none_or(|p| motion_path(p).as_ref() == Some(p))
            && self.tracks.iter().all(|t| !t.keys.is_empty() && t.is_sane()) && self.tracks.windows(2).all(|w| matches!(w, [a, b] if a.property.index() < b.property.index()))
    }

    /// Repairs a loaded tween.
    pub fn sanitize(&mut self) {
        self.path = self.path.as_ref().and_then(motion_path);
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
        let tail = m.split_at(&Props::of(&base), 4);
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
            ..MotionTween::new(ElementId(1))
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

    // ----- motion paths (ADR 0030) ----------------------------------------------------------

    fn line(a: (f32, f32), b: (f32, f32)) -> Path {
        let mut p = Path::new();
        p.move_to(Vec2::new(a.0, a.1)).line_to(Vec2::new(b.0, b.1));
        p
    }

    /// The cubic quarter circle of radius `r` about `(0, r)`: from the origin heading +x round
    /// to `(r, r)` heading +y.
    fn quarter(r: f32) -> Path {
        let k = 0.552_284_8 * r;
        let mut p = Path::new();
        p.move_to(Vec2::new(0.0, 0.0)).cubic_to(Vec2::new(k, 0.0), Vec2::new(r, r - k), Vec2::new(r, r));
        p
    }

    fn at(m: &MotionTween, base: &Element, f: f32) -> Vec2 {
        m.apply(base, f).transform.translation()
    }

    /// X 0 → 100 and Y 0 → 100 over frames 0..20 with `ease` on both.
    fn diagonal(ease: Easing) -> MotionTween {
        let mut m = MotionTween::new(ElementId(1));
        m.tracks = vec![track(Property::X, &[(0, 0.0, ease), (20, 100.0, Easing::Linear)]), track(Property::Y, &[(0, 0.0, ease), (20, 100.0, Easing::Linear)])];
        m
    }

    #[test]
    fn a_straight_path_reproduces_linear_and_eased_xy() {
        let base = el(1, 0.0, 0.0);
        for ease in [Easing::Linear, Easing::QuadIn, Easing::CubicBezier(0.2, 0.9, 0.4, 1.3)] {
            let plain = diagonal(ease);
            let mut m = plain.clone();
            assert!(m.set_path(&line((0.0, 0.0), (100.0, 100.0)), &Props::of(&base), 20));
            assert_eq!(m.tracks, plain.tracks, "ends already in place: the keys stay as they were");
            for i in 0..=80 {
                let f = i as f32 / 4.0;
                let (a, b) = (at(&plain, &base, f), at(&m, &base, f));
                assert!(a.distance(b) < 2e-3, "{ease:?} frame {f}: {a:?} vs {b:?}");
            }
        }
    }

    #[test]
    fn a_quarter_circle_keeps_the_object_on_the_arc() {
        let r = 100.0;
        let base = el(1, 0.0, 0.0);
        let mut m = diagonal(Easing::Linear);
        assert!(m.set_path(&quarter(r), &Props::of(&base), 20));
        let mut last = -1.0f32;
        for f in 0..=20 {
            let p = at(&m, &base, f as f32);
            assert!((p.distance(Vec2::new(0.0, r)) - r).abs() < 0.05, "frame {f}: {p:?}");
            // Linear X/Y on a straight diagonal: an even pace along the arc.
            let angle = (p.x / r).clamp(-1.0, 1.0).asin().to_degrees();
            assert!((angle - 90.0 * f as f32 / 20.0).abs() < 0.2, "frame {f}: {angle}°");
            assert!(angle >= last);
            last = angle;
        }
        // Before the first position key and after the last, it holds at the ends.
        assert!(at(&m, &base, 30.0).distance(Vec2::new(r, r)) < 1e-3);
    }

    #[test]
    fn orient_to_path_turns_with_the_tangent() {
        let r = 100.0;
        for start in [0.0f32, 30.0] {
            let base = Element { transform: Affine::rotate(start.to_radians()), ..el(1, 0.0, 0.0) };
            let mut m = diagonal(Easing::Linear);
            assert!(m.set_path(&quarter(r), &Props::of(&base), 20));
            m.orient = true;
            let walk = CurveWalk::new(m.path.as_ref().unwrap()).unwrap();
            for f in 0..=20 {
                let rot = m.apply(&base, f as f32).transform.decompose().rotation.to_degrees();
                let p = at(&m, &base, f as f32);
                let tan = walk.tangent(walk.project(p));
                let want = start + tan.y.atan2(tan.x).to_degrees();
                assert!((rot - want).abs() < 0.05, "start {start}° frame {f}: {rot} vs {want}");
                assert!((p.distance(Vec2::new(0.0, r)) - r).abs() < 0.05);
            }
            // Without orient the rotation stays.
            m.orient = false;
            assert!((m.apply(&base, 10.0).transform.decompose().rotation.to_degrees() - start).abs() < 1e-3);
        }
    }

    #[test]
    fn a_rotation_curve_adds_to_the_turn() {
        let base = el(1, 0.0, 0.0);
        let mut m = diagonal(Easing::Linear);
        assert!(m.set_path(&quarter(100.0), &Props::of(&base), 20));
        m.orient = true;
        m.set_key(Property::Rotation, 20, 10.0, 0.0);
        let rot = m.apply(&base, 20.0).transform.decompose().rotation.to_degrees();
        assert!((rot - 100.0).abs() < 0.1, "a quarter turn from the path and 10° keyed: {rot}");
    }

    #[test]
    fn splitting_a_path_tween_keeps_the_motion_and_the_turn() {
        let base = el(1, 0.0, 0.0);
        let mut m = diagonal(Easing::QuadOut);
        assert!(m.set_path(&quarter(100.0), &Props::of(&base), 20));
        m.orient = true;
        m.set_key(Property::Rotation, 20, 45.0, 0.0);
        for cut in [7u32, 20, 25] {
            let tail = m.split_at(&Props::of(&base), cut);
            let tail_base = m.apply(&base, cut as f32);
            for f in cut..=30 {
                let (a, b) = (m.apply(&base, f as f32), tail.apply(&tail_base, (f - cut) as f32));
                let (pa, pb) = (a.transform.translation(), b.transform.translation());
                assert!(pa.distance(pb) < 0.05, "cut {cut} frame {f}: {pa:?} vs {pb:?}");
                let (ra, rb) = (a.transform.decompose().rotation.to_degrees(), b.transform.decompose().rotation.to_degrees());
                assert!((ra - rb).abs() < 0.1, "cut {cut} frame {f}: {ra} vs {rb}");
            }
            assert_eq!(tail.path.is_some(), cut < 20, "past the end the tail holds still");
        }
    }

    #[test]
    fn moving_the_whole_path_keeps_the_motion_and_its_timing() {
        let base = el(1, 0.0, 0.0);
        let mut m = diagonal(Easing::CubicBezier(0.2, 0.9, 0.4, 1.3));
        assert!(m.set_path(&quarter(100.0), &Props::of(&base), 20));
        m.orient = true;
        m.set_key(Property::Y, 8, 30.0, 0.0);
        let d = Vec2::new(-37.5, 212.25);
        let mut moved = m.clone();
        assert!(moved.translate(d));
        let moved_base = el(1, d.x, d.y);
        for i in 0..=100 {
            let f = i as f32 / 4.0;
            let (a, b) = (m.apply(&base, f), moved.apply(&moved_base, f));
            assert!((a.transform.translation() + d).distance(b.transform.translation()) < 1e-3, "frame {f}");
            assert!((a.transform.decompose().rotation - b.transform.decompose().rotation).abs() < 1e-4, "frame {f}: the turn is the same");
        }
        assert_eq!(moved.key_frames(), m.key_frames(), "no key moves in time");
        // Out of range: refused, nothing changed.
        let before = moved.clone();
        assert!(!moved.translate(Vec2::new(2e6, 0.0)) && !moved.translate(Vec2::new(f32::NAN, 0.0)));
        assert_eq!(moved, before);
    }

    #[test]
    fn a_path_from_the_tracks_moves_the_same_way() {
        // Right 100 over 10 frames, then down 100 over the next 10.
        let base = el(1, 0.0, 0.0);
        let mut plain = MotionTween::new(ElementId(1));
        plain.tracks = vec![track(Property::X, &[(0, 0.0, Easing::Linear), (10, 100.0, Easing::Linear)]), track(Property::Y, &[(0, 0.0, Easing::Linear), (10, 0.0, Easing::Linear), (20, 100.0, Easing::Linear)])];
        let path = plain.path_from_tracks(&Props::of(&base)).unwrap();
        assert_eq!(path.anchors().len(), 3);
        let mut m = plain.clone();
        assert!(m.set_path(&path, &Props::of(&base), 20));
        for f in 0..=20 {
            assert!(at(&plain, &base, f as f32).distance(at(&m, &base, f as f32)) < 1e-2, "frame {f}");
        }
        assert_eq!(m.positions(&Props::of(&base), 21).len(), 21);
        assert!(MotionTween::new(ElementId(1)).path_from_tracks(&Props::of(&base)).is_none(), "nothing moves");
    }

    #[test]
    fn a_path_without_position_keys_spans_the_tween() {
        let base = el(1, 5.0, 5.0);
        let mut m = MotionTween::new(ElementId(1));
        assert!(!m.set_path(&line((0.0, 0.0), (50.0, 0.0)), &Props::of(&base), 0), "a single frame has no room to move");
        assert!(m.set_path(&line((0.0, 0.0), (50.0, 0.0)), &Props::of(&base), 10));
        assert_eq!(m.position_range(), Some((0, 10)));
        assert!(at(&m, &base, 5.0).distance(Vec2::new(25.0, 0.0)) < 1e-3);
        // A loop (back to the start) is travelled at an even pace.
        let mut l = MotionTween::new(ElementId(1));
        let mut ring = Path::new();
        ring.move_to(Vec2::new(0.0, 0.0)).line_to(Vec2::new(40.0, 0.0)).line_to(Vec2::new(40.0, 40.0)).line_to(Vec2::new(0.0, 40.0)).close();
        assert!(l.set_path(&ring, &Props::of(&base), 16));
        assert!(at(&l, &base, 4.0).distance(Vec2::new(40.0, 0.0)) < 1e-2 && at(&l, &base, 8.0).distance(Vec2::new(40.0, 40.0)) < 1e-2);
    }

    #[test]
    fn motion_paths_are_normalised_and_hostile_ones_dropped() {
        let mut p = Path::new();
        p.move_to(Vec2::new(9.0, 9.0)).move_to(Vec2::new(0.0, 0.0)).quad_to(Vec2::new(10.0, 10.0), Vec2::new(20.0, 0.0)).line_to(Vec2::new(30.0, 0.0)).move_to(Vec2::new(99.0, 99.0)).line_to(Vec2::new(0.0, 0.0));
        let n = motion_path(&p).unwrap();
        assert_eq!(n.cmds.len(), 3, "the first subpath, as a move and cubics");
        assert!(n.cmds.iter().skip(1).all(|c| matches!(c, PathCmd::CubicTo(..))));
        assert_eq!(motion_path(&n), Some(n.clone()), "normal form is stable");
        assert!(motion_path(&line((0.0, 0.0), (0.0, 0.0))).is_none());
        assert!(motion_path(&line((0.0, 0.0), (f32::NAN, 0.0))).is_none());
        assert!(motion_path(&line((0.0, 0.0), (5e6, 0.0))).is_none());
        let mut long = Path::new();
        long.move_to(Vec2::ZERO);
        for i in 1..=5000 {
            long.line_to(Vec2::new(i as f32, 0.0));
        }
        assert_eq!(motion_path(&long).map(|p| p.cmds.len()), Some(MAX_PATH_SEGMENTS + 1));
        // Loading repairs a tween holding a bad path, and a good one survives a round trip.
        let mut m = diagonal(Easing::Linear);
        m.path = Some(p);
        m.orient = true;
        assert!(!m.is_sane());
        m.sanitize();
        assert!(m.is_sane() && m.path.is_some());
        let json = serde_json::to_string(&m).unwrap();
        let back: MotionTween = serde_json::from_str(&json).unwrap();
        assert_eq!(back, m);
        let old: MotionTween = serde_json::from_str(r#"{"target": 1}"#).unwrap();
        assert!(old.path.is_none() && !old.orient, "files from before motion paths open unchanged");
        m.path = Some(line((0.0, 0.0), (f32::INFINITY, 1.0)));
        m.sanitize();
        assert!(m.path.is_none());
    }
}
