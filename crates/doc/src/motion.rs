//! Motion tweens (ADR 0016): one object's properties, each on its own curve.
//!
//! A classic tween interpolates a whole keyframe into the next one. A motion tween instead
//! belongs to one keyframe span and one object in it (its *target*). Each property (x, y,
//! scale, rotation, skew, and the colour effect's alpha, brightness, tint and advanced values) may have a [`Track`] of property keyframes anywhere in
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
//!
//! Anchors of the path may be *pinned* to frames (ADR 0049): the object passes a pinned anchor
//! exactly on its frame, and the X/Y keys there hold the anchor's place. The ends are always
//! pinned, to the first and last position keys. Pins cut the motion into legs; on each leg the
//! X/Y curves time the object along the stretch of path between its two pinned anchors, as they
//! time the whole path when only the ends are pinned. Anchors that aren't pinned shape the path
//! only.

use crate::{ColorEffect, ColorStyle, ColorTransform, Element, ElementId, Easing, Rgba};
use anicraft_geom::edit::ANCHOR;
use anicraft_geom::walk::CurveWalk;
use anicraft_geom::{Affine, Decomposed, Path, PathCmd, Vec2};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Most keys one track keeps (files and commands alike).
pub const MAX_KEYS: usize = 10_000;

/// Most segments a motion path keeps (files and commands alike).
pub const MAX_PATH_SEGMENTS: usize = 1_000;

/// A property a motion tween animates. Values are in the units the Motion Editor shows:
/// pixels, percent, degrees, and 0–255 for colour channels and offsets (as Animate shows them).
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
    /// Colour effect (ADR 0056), by style. Alpha: opacity, percent.
    Alpha,
    /// Brightness, −100..=100 %.
    Brightness,
    /// Tint amount, percent, and the tint colour's channels, 0–255.
    Tint,
    TintRed,
    TintGreen,
    TintBlue,
    /// Advanced: each channel's multiplier, −100..=100 %, and offset, −255..=255.
    RedMultiplier,
    GreenMultiplier,
    BlueMultiplier,
    AlphaMultiplier,
    RedOffset,
    GreenOffset,
    BlueOffset,
    AlphaOffset,
}

/// How many properties there are.
pub const PROPERTIES: usize = 20;

impl Property {
    pub const ALL: [Property; PROPERTIES] = [
        Property::X,
        Property::Y,
        Property::ScaleX,
        Property::ScaleY,
        Property::Rotation,
        Property::Skew,
        Property::Alpha,
        Property::Brightness,
        Property::Tint,
        Property::TintRed,
        Property::TintGreen,
        Property::TintBlue,
        Property::RedMultiplier,
        Property::GreenMultiplier,
        Property::BlueMultiplier,
        Property::AlphaMultiplier,
        Property::RedOffset,
        Property::GreenOffset,
        Property::BlueOffset,
        Property::AlphaOffset,
    ];

    /// The Advanced properties: multipliers then offsets, red first.
    pub const ADVANCED: [Property; 8] = [
        Property::RedMultiplier,
        Property::GreenMultiplier,
        Property::BlueMultiplier,
        Property::AlphaMultiplier,
        Property::RedOffset,
        Property::GreenOffset,
        Property::BlueOffset,
        Property::AlphaOffset,
    ];

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
            Property::Brightness => "brightness",
            Property::Tint => "tint",
            Property::TintRed => "tint_red",
            Property::TintGreen => "tint_green",
            Property::TintBlue => "tint_blue",
            Property::RedMultiplier => "red_multiplier",
            Property::GreenMultiplier => "green_multiplier",
            Property::BlueMultiplier => "blue_multiplier",
            Property::AlphaMultiplier => "alpha_multiplier",
            Property::RedOffset => "red_offset",
            Property::GreenOffset => "green_offset",
            Property::BlueOffset => "blue_offset",
            Property::AlphaOffset => "alpha_offset",
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
            Property::Brightness => "Brightness",
            Property::Tint => "Tint",
            Property::TintRed => "Tint red",
            Property::TintGreen => "Tint green",
            Property::TintBlue => "Tint blue",
            Property::RedMultiplier => "Red multiplier",
            Property::GreenMultiplier => "Green multiplier",
            Property::BlueMultiplier => "Blue multiplier",
            Property::AlphaMultiplier => "Alpha multiplier",
            Property::RedOffset => "Red offset",
            Property::GreenOffset => "Green offset",
            Property::BlueOffset => "Blue offset",
            Property::AlphaOffset => "Alpha offset",
        }
    }

    pub fn unit(self) -> &'static str {
        match self {
            Property::X | Property::Y => "px",
            Property::ScaleX | Property::ScaleY | Property::Alpha | Property::Brightness | Property::Tint => "%",
            Property::RedMultiplier | Property::GreenMultiplier | Property::BlueMultiplier | Property::AlphaMultiplier => "%",
            Property::Rotation | Property::Skew => "°",
            Property::TintRed | Property::TintGreen | Property::TintBlue | Property::RedOffset | Property::GreenOffset | Property::BlueOffset | Property::AlphaOffset => "",
        }
    }

    /// The Motion Editor's groups: Position, Transform, Colour effect.
    pub fn group(self) -> &'static str {
        match self.style() {
            None if matches!(self, Property::X | Property::Y) => "Position",
            None => "Transform",
            Some(_) => "Colour effect",
        }
    }

    /// The colour effect style a colour property belongs to (`None` for position and transform).
    pub fn style(self) -> Option<ColorStyle> {
        match self {
            Property::X | Property::Y | Property::ScaleX | Property::ScaleY | Property::Rotation | Property::Skew => None,
            Property::Alpha => Some(ColorStyle::Alpha),
            Property::Brightness => Some(ColorStyle::Brightness),
            Property::Tint | Property::TintRed | Property::TintGreen | Property::TintBlue => Some(ColorStyle::Tint),
            Property::RedMultiplier | Property::GreenMultiplier | Property::BlueMultiplier | Property::AlphaMultiplier | Property::RedOffset | Property::GreenOffset | Property::BlueOffset | Property::AlphaOffset => Some(ColorStyle::Advanced),
        }
    }

    pub fn from_key(s: &str) -> Option<Property> {
        let s = s.trim().to_ascii_lowercase().replace([' ', '-'], "_");
        let s = match s.as_str() {
            "scalex" => "scale_x",
            "scaley" => "scale_y",
            "rotate" => "rotation",
            "opacity" => "alpha",
            "tint_amount" => "tint",
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
            Property::Brightness | Property::RedMultiplier | Property::GreenMultiplier | Property::BlueMultiplier | Property::AlphaMultiplier => -100.0..=100.0,
            Property::TintRed | Property::TintGreen | Property::TintBlue => 0.0..=255.0,
            Property::RedOffset | Property::GreenOffset | Property::BlueOffset | Property::AlphaOffset => -255.0..=255.0,
        }
    }

    fn index(self) -> usize {
        Property::ALL.iter().position(|p| *p == self).unwrap_or(0)
    }

    fn is_transform(self) -> bool {
        self.style().is_none()
    }

    fn clamp(self, v: f32) -> f32 {
        let r = self.range();
        if v.is_finite() { v.clamp(*r.start(), *r.end()) } else { 0.0 }
    }
}

/// Every property's value, indexed like [`Property::ALL`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Props(pub [f32; PROPERTIES]);

impl Props {
    /// An element's own values. Colour properties come from its colour effect's style; the
    /// other styles' properties hold their rest values (alpha 100 %, brightness 0, tint 0 % of
    /// white, Advanced 100 % and 0).
    pub fn of(el: &Element) -> Props {
        let d = el.transform.decompose();
        let mut p = Props([0.0; PROPERTIES]);
        for (q, v) in [(Property::X, d.tx), (Property::Y, d.ty), (Property::ScaleX, d.sx * 100.0), (Property::ScaleY, d.sy * 100.0), (Property::Rotation, d.rotation.to_degrees()), (Property::Skew, d.skew.atan().to_degrees())] {
            p.set(q, v);
        }
        p.set_colour(&el.color);
        p
    }

    /// Sets the colour properties from `fx`, as [`Props::of`] does.
    fn set_colour(&mut self, fx: &ColorEffect) {
        let (mut alpha, mut bright, mut tint, mut rgb, mut adv) = (1.0, 0.0, 0.0, Rgba::WHITE, ColorTransform::IDENTITY);
        match *fx {
            ColorEffect::None => {}
            ColorEffect::Alpha(a) => alpha = a,
            ColorEffect::Brightness(b) => bright = b,
            ColorEffect::Tint { color, amount } => (tint, rgb) = (amount, color),
            ColorEffect::Advanced(t) => adv = t,
        }
        self.set(Property::Alpha, alpha * 100.0);
        self.set(Property::Brightness, bright * 100.0);
        self.set(Property::Tint, tint * 100.0);
        self.set(Property::TintRed, rgb.r * 255.0);
        self.set(Property::TintGreen, rgb.g * 255.0);
        self.set(Property::TintBlue, rgb.b * 255.0);
        let values = adv.mul.iter().map(|m| m * 100.0).chain(adv.add.iter().map(|a| a * 255.0));
        for (q, v) in Property::ADVANCED.into_iter().zip(values) {
            self.set(q, v);
        }
    }

    /// The colour effect of the styles in `styles` from these values (the other styles' values
    /// are left out): one style alone is that style; several compose into Advanced (ADR 0056).
    pub fn color_effect(&self, styles: &[ColorStyle]) -> ColorEffect {
        let v = |p: Property| p.clamp(self.get(p));
        let on = |s: ColorStyle| styles.contains(&s);
        let alpha = on(ColorStyle::Alpha).then(|| v(Property::Alpha) / 100.0);
        let bright = on(ColorStyle::Brightness).then(|| v(Property::Brightness) / 100.0);
        let tint = on(ColorStyle::Tint).then(|| (Rgba::new(v(Property::TintRed) / 255.0, v(Property::TintGreen) / 255.0, v(Property::TintBlue) / 255.0, 1.0), v(Property::Tint) / 100.0));
        let adv = on(ColorStyle::Advanced).then(|| {
            let [rm, gm, bm, am, ro, go, bo, ao] = Property::ADVANCED.map(v);
            ColorTransform { mul: [rm, gm, bm, am].map(|m| m / 100.0), add: [ro, go, bo, ao].map(|a| a / 255.0) }
        });
        ColorEffect::compose(alpha, bright, tint, adv).sanitized()
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

/// A motion path anchor pinned to a frame (ADR 0049): the object passes the anchor exactly
/// then. Only anchors between the ends are stored; the ends are pinned to the first and last
/// position keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pin {
    /// Frames from the span's first frame.
    pub frame: u32,
    /// The anchor: path command `anchor` (0 is the start).
    pub anchor: u32,
}

/// Why an anchor can't be pinned (or a pin added) where asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PinError {
    /// The tween has no motion path.
    NoPath,
    /// No such anchor on the path.
    NoAnchor,
    /// An end: always pinned to the first or last position key.
    End,
    /// The frame isn't strictly between the first and last position keys (these frames).
    Outside(u32, u32),
    /// Pins keep the path's order (a later frame on a later anchor); this one is in the way.
    Order(Pin),
    /// The object is off the path at that frame (an ease overshooting an end).
    OffPath,
    /// The path holds as many segments as it may.
    Full,
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
    /// Anchors between the ends pinned to frames (ADR 0049), by frame; both frames and anchors
    /// increase. Files from before pins have none: only the ends are pinned.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pins: Vec<Pin>,
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
    /// Where the legs start and end: the first position key, each pin and the last position key,
    /// as an index into `keys` and the distance along the path of the anchor pinned there.
    stations: Vec<(usize, f32)>,
    /// Direction of travel at the start, radians.
    start_angle: f32,
}

impl OnPath<'_> {
    pub fn walk(&self) -> &CurveWalk {
        &self.walk
    }

    /// The share of the path covered `frame` frames into the span (see [`OnPath::distance`]).
    pub fn progress(&self, frame: f32) -> f32 {
        let l = self.walk.length();
        if l > 0.0 { self.distance(frame) / l } else { 0.0 }
    }

    /// Distance along the path at `frame`: 0 up to the first position key (and without two), the
    /// whole length from the last; an overshooting ease may go beyond in between.
    ///
    /// The pins cut the span into legs, each from one pinned anchor to the next. On a leg, the
    /// position keys inside it split it again; on each of those pieces the X/Y curves run from
    /// one key's position to the next, and how far along that chord they are (signed) is how far
    /// along the piece's share of the leg's stretch of path the object is, the shares being the
    /// chords' lengths. Where a leg's chords add up to nothing (a loop) the object keeps an even
    /// pace across it. With only the ends pinned this is ADR 0030's timing, number for number.
    pub fn distance(&self, frame: f32) -> f32 {
        let l = self.walk.length();
        let (Some(first), Some(last)) = (self.keys.first(), self.keys.last()) else { return 0.0 };
        if !frame.is_finite() || frame <= first.0 || last.0 <= first.0 {
            return 0.0;
        }
        if frame >= last.0 {
            return l;
        }
        let i = self.keys.partition_point(|k| k.0 <= frame).saturating_sub(1);
        let leg = self.stations.partition_point(|st| st.0 <= i).saturating_sub(1);
        let (Some(&(ka, sa)), Some(&(kb, sb))) = (self.stations.get(leg), self.stations.get(leg + 1)) else { return l };
        let (Some(a), Some(b), Some(fa), Some(fb)) = (self.keys.get(i), self.keys.get(i + 1), self.keys.get(ka), self.keys.get(kb)) else { return l };
        let c0 = self.cum.get(ka).copied().unwrap_or(0.0);
        let total = self.cum.get(kb).copied().unwrap_or(0.0) - c0;
        let p = if total <= 1e-4 {
            (frame - fa.0) / (fb.0 - fa.0)
        } else {
            let chord = b.1 - a.1;
            let len = chord.length();
            let along = if len > 1e-6 { (self.tween.trace_at(&self.base, frame) - a.1).dot(chord) / len } else { 0.0 };
            (self.cum.get(i).copied().unwrap_or(0.0) - c0 + along) / total
        };
        let p = if p.is_finite() { p } else { 0.0 };
        sa + p * (sb - sa)
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
        MotionTween { target, tracks: Vec::new(), path: None, orient: false, pins: Vec::new() }
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
        let pins = self.live_pins();
        let mut frames: Vec<u32> = [Property::X, Property::Y].into_iter().filter_map(|p| self.track(p)).flat_map(|t| t.keys.iter().map(|k| k.frame)).collect();
        // A pin's frame holds X/Y keys (commands see to it); a leg ends there all the same.
        frames.extend(pins.iter().map(|p| p.frame));
        frames.sort_unstable();
        frames.dedup();
        let keys: Vec<(f32, Vec2)> = frames.iter().map(|&f| (f as f32, self.trace_at(base, f as f32))).collect();
        let mut cum = Vec::with_capacity(keys.len());
        let mut d = 0.0f32;
        for (i, k) in keys.iter().enumerate() {
            d += i.checked_sub(1).and_then(|p| keys.get(p)).map_or(0.0, |p| p.1.distance(k.1));
            cum.push(d);
        }
        let mut stations = vec![(0usize, 0.0f32)];
        for pin in &pins {
            if let (Ok(i), Some(s)) = (frames.binary_search(&pin.frame), walk.anchor_distance(pin.anchor as usize))
                && stations.last().is_some_and(|l| l.0 < i)
            {
                stations.push((i, s));
            }
        }
        if keys.len() > 1 {
            stations.push((keys.len() - 1, walk.length()));
        }
        let t = walk.tangent(0.0);
        Some(OnPath { tween: self, base: *base, start_angle: t.y.atan2(t.x), walk, keys, cum, stations })
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
        self.path_and_pins_from_tracks(base).map(|(p, _)| p)
    }

    /// [`MotionTween::path_from_tracks`] with the position keys between the ends pinned to the
    /// anchors they made (the first key at a place, where the object holds still a while).
    pub fn path_and_pins_from_tracks(&self, base: &Props) -> Option<(Path, Vec<Pin>)> {
        let mut frames: Vec<u32> = [Property::X, Property::Y].into_iter().filter_map(|p| self.track(p)).flat_map(|t| t.keys.iter().map(|k| k.frame)).collect();
        frames.sort_unstable();
        frames.dedup();
        let mut pts: Vec<Vec2> = Vec::new();
        let mut pins = Vec::new();
        for f in frames.into_iter().take(MAX_PATH_SEGMENTS + 1) {
            let q = self.trace_at(base, f as f32);
            if pts.last().is_none_or(|l| l.distance(q) > 1e-3) {
                pins.push(Pin { frame: f, anchor: pts.len() as u32 });
                pts.push(q);
            }
        }
        let (first, rest) = pts.split_first()?;
        let mut p = Path::new();
        p.move_to(*first);
        for q in rest {
            p.line_to(*q);
        }
        let path = motion_path(&p)?;
        let n = path.cmds.len();
        pins.retain(|p| p.anchor > 0 && (p.anchor as usize) + 1 < n);
        Some((path, pins))
    }

    /// The pins in force: between the ends in both frame and anchor, frames and anchors
    /// increasing. Others are ignored (and dropped when a file is loaded).
    pub fn live_pins(&self) -> Vec<Pin> {
        let (Some(path), Some((f0, f1))) = (&self.path, self.position_range()) else { return Vec::new() };
        let n = path.cmds.len();
        let mut out: Vec<Pin> = Vec::new();
        for p in &self.pins {
            let inside = p.frame > f0 && p.frame < f1 && p.anchor > 0 && (p.anchor as usize).saturating_add(1) < n;
            if inside && out.last().is_none_or(|q| q.frame < p.frame && q.anchor < p.anchor) {
                out.push(*p);
            }
        }
        out
    }

    /// Every anchor tied to a frame, in order: the start at the first position key, the pins,
    /// and the end at the last. A tween without a path of its own gives those of the straight
    /// path its X/Y curves trace. Empty while the object doesn't move.
    pub fn pinned_anchors(&self, base: &Props) -> Vec<Pin> {
        let Some((f0, f1)) = self.position_range().filter(|(a, b)| b > a) else { return Vec::new() };
        let (n, pins) = match &self.path {
            Some(p) => (p.cmds.len(), self.live_pins()),
            None => match self.path_and_pins_from_tracks(base) {
                Some((p, pins)) => (p.cmds.len(), pins),
                None => return Vec::new(),
            },
        };
        let mut v = Vec::with_capacity(pins.len() + 2);
        v.push(Pin { frame: f0, anchor: 0 });
        v.extend(pins);
        v.push(Pin { frame: f1, anchor: n.saturating_sub(1) as u32 });
        v
    }

    /// Gives the tween motion path `path` (made a [`motion_path`]; false when nothing usable
    /// is left). Its ends become the X/Y keys at the first and last position keys, or at the
    /// span's first frame and frame `last` when there are none, so the curves start and end
    /// where the path does; each pinned anchor's place becomes the X/Y keys on its frame.
    ///
    /// A tween without a path of its own shows the straight one through its position keys,
    /// which are pinned to its anchors: a path with as many anchors is that one edited, and
    /// keeps them pinned. Pins on anchors the new path doesn't have are let go, with their keys.
    pub fn set_path(&mut self, path: &Path, base: &Props, last: u32) -> bool {
        let Some(p) = motion_path(path) else { return false };
        let Some((a, b)) = path_ends(&p) else { return false };
        if self.path.is_none()
            && self.pins.is_empty()
            && let Some((implied, pins)) = self.path_and_pins_from_tracks(base)
            && implied.cmds.len() == p.cmds.len()
        {
            self.pins = pins;
        }
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
        let n = p.cmds.len();
        self.path = Some(p);
        let (keep, gone): (Vec<Pin>, Vec<Pin>) = self.pins.iter().partition(|q| q.anchor > 0 && (q.anchor as usize).saturating_add(1) < n);
        self.pins = keep;
        for q in gone {
            self.remove_pin_keys(q.frame);
        }
        self.sync_pins(base);
        true
    }

    /// Writes each pinned anchor's place as the X/Y keys on its frame (leaving keys alone that
    /// already hold it, so editing elsewhere keeps the timing exactly).
    fn sync_pins(&mut self, base: &Props) {
        let Some(path) = self.path.clone() else { return };
        for pin in self.live_pins() {
            let Some(q) = path.point(pin.anchor as usize, ANCHOR) else { continue };
            for (prop, v) in [(Property::X, q.x), (Property::Y, q.y)] {
                let holds = self.track(prop).and_then(|t| t.key_at(pin.frame).and_then(|i| t.keys.get(i))).is_some_and(|k| k.value == v);
                if !holds {
                    self.set_key(prop, pin.frame, v, base.get(prop));
                }
            }
        }
    }

    /// Removes the X/Y keys a pin held on `frame`, unless that frame is an end's.
    fn remove_pin_keys(&mut self, frame: u32) {
        if self.position_range().is_some_and(|(f0, f1)| f0 < frame && frame < f1) {
            self.remove_track_key(Property::X, frame);
            self.remove_track_key(Property::Y, frame);
        }
    }

    /// Pins anchor `anchor` to `frame`: the object passes it exactly then, and the X/Y keys on
    /// that frame hold its place. A pin already on that frame, or that anchor's pin on another
    /// frame, gives way.
    pub fn pin(&mut self, anchor: usize, frame: u32, base: &Props) -> Result<(), PinError> {
        let Some(path) = &self.path else { return Err(PinError::NoPath) };
        let n = path.cmds.len();
        if anchor >= n {
            return Err(PinError::NoAnchor);
        }
        if anchor == 0 || anchor + 1 == n {
            return Err(PinError::End);
        }
        let (f0, f1) = self.position_range().unwrap_or((0, 0));
        if !(f0 < frame && frame < f1) {
            return Err(PinError::Outside(f0, f1));
        }
        let Some(q) = path.point(anchor, ANCHOR) else { return Err(PinError::NoAnchor) };
        let a = anchor as u32;
        let live = self.live_pins();
        if let Some(p) = live.iter().find(|p| p.frame != frame && p.anchor != a && (p.frame < frame) != (p.anchor < a)) {
            return Err(PinError::Order(*p));
        }
        self.pins = live;
        if let Some(old) = self.pins.iter().find(|p| p.anchor == a && p.frame != frame).copied() {
            self.unpin(old.frame);
        }
        self.pins.retain(|p| p.frame != frame);
        let i = self.pins.partition_point(|p| p.frame < frame);
        self.pins.insert(i, Pin { frame, anchor: a });
        for (prop, v) in [(Property::X, q.x), (Property::Y, q.y)] {
            self.set_key(prop, frame, v, base.get(prop));
        }
        Ok(())
    }

    /// Pins the path where the object is on `frame`: on the anchor there, or on a new one (the
    /// path's shape unchanged). The anchor pinned.
    pub fn pin_here(&mut self, frame: u32, base: &Props) -> Result<usize, PinError> {
        if self.path.is_none() {
            return Err(PinError::NoPath);
        }
        let (f0, f1) = self.position_range().unwrap_or((0, 0));
        if !(f0 < frame && frame < f1) {
            return Err(PinError::Outside(f0, f1));
        }
        if let Some(p) = self.live_pins().into_iter().find(|p| p.frame == frame) {
            return Ok(p.anchor as usize);
        }
        let (s, l, (cmd, t)) = {
            let op = self.on_path(base).ok_or(PinError::NoPath)?;
            let s = op.distance(frame as f32);
            (s, op.walk().length(), op.walk().locate(s))
        };
        if !(0.0..=l).contains(&s) {
            return Err(PinError::OffPath);
        }
        let mut trial = self.clone();
        let anchor = if t <= 1e-3 {
            cmd.saturating_sub(1)
        } else if t >= 1.0 - 1e-3 {
            cmd
        } else {
            if self.path.as_ref().is_some_and(|p| p.cmds.len() > MAX_PATH_SEGMENTS) {
                return Err(PinError::Full);
            }
            if !trial.split_path(cmd, t) {
                return Err(PinError::OffPath);
            }
            cmd
        };
        trial.pin(anchor, frame, base)?;
        *self = trial;
        Ok(anchor)
    }

    /// Splits the path's segment `cmd` at curve parameter `t` (the curve unchanged; the new
    /// anchor is `cmd`). Pins after it stay on their anchors. False when there is no such
    /// segment.
    pub fn split_path(&mut self, cmd: usize, t: f32) -> bool {
        let Some(path) = self.path.as_mut() else { return false };
        if !path.split_segment(cmd, t) {
            return false;
        }
        for p in &mut self.pins {
            if p.anchor as usize >= cmd {
                p.anchor = p.anchor.saturating_add(1);
            }
        }
        true
    }

    /// Deletes anchor `i` between the ends, refitting the two segments around it as one
    /// ([`Path::merge_at`]). A pin on it goes, with its keys; later pins stay on their anchors.
    /// False when it can't be deleted (an end, or no such anchor).
    pub fn delete_anchor(&mut self, i: usize) -> bool {
        let Some(path) = self.path.as_mut() else { return false };
        if !path.merge_at(i) {
            return false;
        }
        if let Some(frame) = self.pins.iter().find(|p| p.anchor as usize == i).map(|p| p.frame) {
            self.unpin(frame);
        }
        for p in &mut self.pins {
            if p.anchor as usize > i {
                p.anchor -= 1;
            }
        }
        true
    }

    /// Lets the pin on `frame` go: its X/Y keys go with it and its anchor shapes the path only
    /// (ADR 0038's behaviour). The pin, if there was one.
    pub fn unpin(&mut self, frame: u32) -> Option<Pin> {
        let i = self.pins.iter().position(|p| p.frame == frame)?;
        let pin = self.pins.remove(i);
        self.remove_pin_keys(frame);
        Some(pin)
    }

    /// Moves anchor `anchor` (its handles with it) by `d`; an end or a pin takes its keys along.
    pub fn move_anchor(&mut self, anchor: usize, d: Vec2, base: &Props, last: u32) -> bool {
        let Some(mut path) = self.path.clone() else { return false };
        path.move_anchor(anchor, d) && self.set_path(&path, base, last)
    }

    /// Carries the path on from its end to `to`, reached on `frame` after the last position key:
    /// the old end stays pinned to its frame and the new straight stretch ends the motion. False
    /// (nothing changed) when the path is full or `frame` isn't after the last position key.
    pub fn extend_to(&mut self, frame: u32, to: Vec2, base: &Props) -> bool {
        let (Some(path), Some((f0, f1))) = (&self.path, self.position_range()) else { return false };
        if frame <= f1 || f1 <= f0 || path.cmds.len() > MAX_PATH_SEGMENTS {
            return false;
        }
        let n = path.cmds.len();
        let mut longer = path.clone();
        longer.line_to(to);
        let Some(longer) = motion_path(&longer).filter(|p| p.cmds.len() == n + 1) else { return false };
        let mut pins = self.live_pins();
        pins.push(Pin { frame: f1, anchor: (n - 1) as u32 });
        self.set_key(Property::X, frame, to.x, base.get(Property::X));
        self.set_key(Property::Y, frame, to.y, base.get(Property::Y));
        self.path = Some(longer);
        self.pins = pins;
        self.sync_pins(base);
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
        // Colour: the styles keyed here and the target's own, composed (ADR 0056).
        if self.tracks.iter().any(|t| !t.property.is_transform()) {
            el.color = props.color_effect(&self.colour_styles(&base.color));
        }
        el
    }

    /// The colour effect styles the target shows over the span. As in Animate, a motion tween
    /// has one colour effect: the style its colour properties are keyed in, replacing the
    /// target's own (which shows while none are keyed). Keys in several styles (older files
    /// keyed alpha and tint together) compose into Advanced.
    pub fn colour_styles(&self, own: &ColorEffect) -> Vec<ColorStyle> {
        let mut out: Vec<ColorStyle> = Vec::new();
        let keyed = self.tracks.iter().filter_map(|t| t.property.style());
        let own = (own.style() != ColorStyle::None && !self.tracks.iter().any(|t| t.property.style().is_some())).then_some(own.style());
        for s in keyed.chain(own) {
            if !out.contains(&s) {
                out.push(s);
            }
        }
        out
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
    /// there. Removing the X or Y key of a pinned frame lets the pin go (with both keys).
    pub fn remove_key(&mut self, p: Property, frame: u32) -> bool {
        if !matches!(p, Property::X | Property::Y) {
            return self.remove_track_key(p, frame);
        }
        // Only pins in force count (removing an end's keys can leave one outside the motion).
        self.pins = self.live_pins();
        if self.pins.iter().any(|q| q.frame == frame) {
            let had = self.track(p).is_some_and(|t| t.key_at(frame).is_some());
            self.unpin(frame);
            return had;
        }
        let had = self.remove_track_key(p, frame);
        // An end's key gone may leave pins outside the motion: they go, never to come back.
        self.pins = self.live_pins();
        had
    }

    fn remove_track_key(&mut self, p: Property, frame: u32) -> bool {
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
        let mut tail = MotionTween { target: self.target, tracks, path: None, orient: self.orient, pins: Vec::new() };
        let Some(op) = self.on_path(base) else { return tail };
        let f = frame as f32;
        match op.walk().tail(op.distance(f)) {
            // The cut path starts where the object is; the X/Y keys go on timing it, and the
            // pins after the cut stay on their anchors (the cut segment's end is anchor 1).
            Some(p) => {
                let (cmd, _) = op.walk().locate(op.distance(f));
                tail.pins = self.live_pins().into_iter().filter(|q| q.frame > frame && q.anchor as usize >= cmd).map(|q| Pin { frame: q.frame - frame, anchor: q.anchor + 1 - cmd as u32 }).collect();
                tail.path = Some(p);
            }
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
            && self.pins == self.live_pins()
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
        self.pins.truncate(MAX_PATH_SEGMENTS);
        self.pins.sort_by_key(|p| p.frame);
        self.pins.dedup_by_key(|p| p.frame);
        self.pins = self.live_pins();
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
        assert!((at5.color.alpha() - 0.5).abs() < 1e-6 && at5.color.style() == ColorStyle::Alpha);
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
            assert!(!p.label().is_empty() && (!p.unit().is_empty() || p.range() == (0.0..=255.0) || p.range() == (-255.0..=255.0)) && !p.group().is_empty());
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

    // ----- anchors pinned to frames (ADR 0049) ----------------------------------------------

    /// An S through three anchors: (0, 0), (100, 0) and (200, 0), over a hump and under a dip.
    fn s_path() -> Path {
        let mut p = Path::new();
        p.move_to(Vec2::new(0.0, 0.0)).cubic_to(Vec2::new(40.0, -60.0), Vec2::new(60.0, -60.0), Vec2::new(100.0, 0.0)).cubic_to(Vec2::new(140.0, 60.0), Vec2::new(160.0, 60.0), Vec2::new(200.0, 0.0));
        p
    }

    /// X 0 → 200 over frames 0..20 with `ease`, Y still: the S travelled left to right.
    fn along_s(ease: Easing) -> (MotionTween, Element) {
        let base = el(1, 0.0, 0.0);
        let mut m = MotionTween::new(ElementId(1));
        m.tracks = vec![track(Property::X, &[(0, 0.0, ease), (20, 200.0, Easing::Linear)]), track(Property::Y, &[(0, 0.0, Easing::Linear), (20, 0.0, Easing::Linear)])];
        assert!(m.set_path(&s_path(), &Props::of(&base), 20));
        (m, base)
    }

    fn anchor_at(m: &MotionTween, i: usize) -> Vec2 {
        m.path.as_ref().and_then(|p| p.point(i, ANCHOR)).unwrap()
    }

    #[test]
    fn the_object_passes_each_pinned_anchor_on_its_frame() {
        let (mut m, base) = along_s(Easing::QuadIn);
        let props = Props::of(&base);
        assert!(at(&m, &base, 6.0).distance(Vec2::new(100.0, 0.0)) > 10.0, "unpinned, the middle anchor is passed when the timing says");
        assert_eq!(m.pin(1, 6, &props), Ok(()));
        assert_eq!(m.pins, vec![Pin { frame: 6, anchor: 1 }]);
        assert!(at(&m, &base, 6.0).distance(Vec2::new(100.0, 0.0)) < 1e-3, "{:?}", at(&m, &base, 6.0));
        assert!(at(&m, &base, 0.0).distance(Vec2::ZERO) < 1e-3 && at(&m, &base, 20.0).distance(Vec2::new(200.0, 0.0)) < 1e-3, "the ends stay pinned");
        // Pinning where the object is adds an anchor there and moves nothing on that frame.
        let was = at(&m, &base, 14.0);
        assert_eq!(m.pin_here(14, &props), Ok(2));
        assert_eq!(m.path.as_ref().map(|p| p.cmds.len()), Some(4));
        assert_eq!(m.pins, vec![Pin { frame: 6, anchor: 1 }, Pin { frame: 14, anchor: 2 }]);
        assert!(at(&m, &base, 14.0).distance(was) < 0.01, "{:?} vs {was:?}", at(&m, &base, 14.0));
        assert_eq!(m.pin_here(14, &props), Ok(2), "already pinned there");
        // Moving a pinned anchor takes the object with it on that frame, and its keys follow.
        assert!(m.move_anchor(2, Vec2::new(0.0, 50.0), &props, 20));
        let a2 = anchor_at(&m, 2);
        assert!(at(&m, &base, 14.0).distance(a2) < 1e-3);
        assert_eq!(m.track(Property::Y).and_then(|t| t.key_at(14).map(|i| t.keys[i].value)), Some(a2.y));
        assert!(at(&m, &base, 6.0).distance(Vec2::new(100.0, 0.0)) < 1e-3, "the other pin holds");
        // What can't be pinned says why, and changes nothing.
        let before = m.clone();
        assert_eq!(m.pin(1, 16, &props), Err(PinError::Order(Pin { frame: 14, anchor: 2 })));
        assert_eq!(m.pin(0, 5, &props), Err(PinError::End));
        assert_eq!(m.pin(3, 5, &props), Err(PinError::End));
        assert_eq!(m.pin(9, 5, &props), Err(PinError::NoAnchor));
        assert_eq!(m.pin(1, 25, &props), Err(PinError::Outside(0, 20)));
        assert_eq!(m.pin_here(0, &props), Err(PinError::Outside(0, 20)));
        assert_eq!(m, before);
        // Re-pinning an anchor to another frame retimes it.
        assert_eq!(m.pin(1, 4, &props), Ok(()));
        assert_eq!(m.pins, vec![Pin { frame: 4, anchor: 1 }, Pin { frame: 14, anchor: 2 }]);
        assert!(at(&m, &base, 4.0).distance(Vec2::new(100.0, 0.0)) < 1e-3);
        assert!(m.track(Property::X).is_some_and(|t| t.key_at(6).is_none()), "the old frame's keys went with the pin");
        assert!(m.is_sane());
    }

    #[test]
    fn between_pins_the_eases_time_the_object_as_before() {
        // A straight path with an anchor at x = 30 pinned to frame 10: it reproduces the X curve
        // exactly, eases and overshoot included, on both legs.
        let base = el(1, 0.0, 0.0);
        let mut p = Path::new();
        p.move_to(Vec2::new(0.0, 0.0)).line_to(Vec2::new(30.0, 0.0)).line_to(Vec2::new(100.0, 0.0));
        for second in [Easing::QuadIn, Easing::CubicBezier(0.2, 0.9, 0.4, 1.3)] {
            let mut m = MotionTween::new(ElementId(1));
            m.tracks = vec![track(Property::X, &[(0, 0.0, Easing::QuadIn), (20, 100.0, Easing::Linear)])];
            assert!(m.set_path(&p, &Props::of(&base), 20));
            assert_eq!(m.pin(1, 10, &Props::of(&base)), Ok(()));
            if let Some(t) = m.track_mut(Property::X) {
                t.keys[1].ease = second;
            }
            let x = m.track(Property::X).cloned().unwrap();
            for i in 0..=80 {
                let f = i as f32 / 4.0;
                let q = at(&m, &base, f);
                assert!((q.x - x.value_at(f).unwrap()).abs() < 2e-3 && q.y.abs() < 1e-3, "{second:?} frame {f}: {q:?} vs {:?}", x.value_at(f));
            }
        }
    }

    #[test]
    fn unpinning_gives_back_the_unpinned_motion() {
        let (m, base) = along_s(Easing::CubicOut);
        let props = Props::of(&base);
        let mut pinned = m.clone();
        assert_eq!(pinned.pin(1, 8, &props), Ok(()));
        assert!(at(&pinned, &base, 5.0).distance(at(&m, &base, 5.0)) > 1.0, "pinning retimes");
        assert_eq!(pinned.unpin(8), Some(Pin { frame: 8, anchor: 1 }));
        assert_eq!(pinned, m, "pin then unpin: the same tween, keys and all");
        assert_eq!(pinned.unpin(8), None);
        // A pin added where the object was leaves its anchor behind, shaping only: the motion
        // is ADR 0038's again (the split curve is the same curve).
        let mut here = m.clone();
        assert!(here.pin_here(12, &props).is_ok());
        assert!(here.unpin(12).is_some());
        assert_eq!(here.path.as_ref().map(|p| p.cmds.len()), Some(4));
        for f in 0..=20 {
            assert!(at(&here, &base, f as f32).distance(at(&m, &base, f as f32)) < 0.01, "frame {f}");
        }
        // Removing the X (or Y) key of a pinned frame lets the pin go, with both keys.
        assert_eq!(pinned.pin(1, 8, &props), Ok(()));
        assert!(pinned.remove_key(Property::X, 8));
        assert!(pinned.pins.is_empty() && pinned.track(Property::Y).is_some_and(|t| t.key_at(8).is_none()));
        assert_eq!(pinned, m);
        // Removing the end's keys lets a pin that is no longer between the ends go; its keys
        // stay as plain keys, removed one by one.
        assert_eq!(pinned.pin(1, 8, &props), Ok(()));
        assert!(pinned.remove_key(Property::X, 20) && pinned.remove_key(Property::Y, 20));
        assert!(pinned.pins.is_empty(), "frame 8 is the last position key now: an end");
        assert!(pinned.remove_key(Property::X, 8));
        assert!(pinned.track(Property::X).is_some_and(|t| t.key_at(8).is_none()) && pinned.track(Property::Y).is_some_and(|t| t.key_at(8).is_some()));
        assert!(pinned.is_sane());
    }

    /// ADR 0030's timing as it was before pins: what files without pins must still give.
    fn legacy_point(m: &MotionTween, base: &Props, frame: f32) -> Vec2 {
        let walk = CurveWalk::new(m.path.as_ref().unwrap()).unwrap();
        let mut frames: Vec<u32> = [Property::X, Property::Y].into_iter().filter_map(|p| m.track(p)).flat_map(|t| t.keys.iter().map(|k| k.frame)).collect();
        frames.sort_unstable();
        frames.dedup();
        let keys: Vec<(f32, Vec2)> = frames.into_iter().map(|f| (f as f32, m.trace_at(base, f as f32))).collect();
        let mut cum = Vec::new();
        let mut d = 0.0f32;
        for (i, k) in keys.iter().enumerate() {
            d += i.checked_sub(1).and_then(|p| keys.get(p)).map_or(0.0, |p| p.1.distance(k.1));
            cum.push(d);
        }
        let progress = (|| {
            let (first, last) = (keys.first()?, keys.last()?);
            if !frame.is_finite() || frame <= first.0 || last.0 <= first.0 {
                return Some(0.0);
            }
            if frame >= last.0 {
                return Some(1.0);
            }
            let total = *cum.last()?;
            if total <= 1e-4 {
                return Some((frame - first.0) / (last.0 - first.0));
            }
            let i = keys.partition_point(|k| k.0 <= frame).saturating_sub(1);
            let (a, b) = (keys.get(i)?, keys.get(i + 1)?);
            let chord = b.1 - a.1;
            let len = chord.length();
            let along = if len > 1e-6 { (m.trace_at(base, frame) - a.1).dot(chord) / len } else { 0.0 };
            let p = (cum[i] + along) / total;
            Some(if p.is_finite() { p } else { 0.0 })
        })()
        .unwrap_or(1.0);
        let (s, l) = (progress * walk.length(), walk.length());
        if s < 0.0 {
            walk.at(0.0) + walk.tangent(0.0) * s
        } else if s > l {
            walk.at(l) + walk.tangent(l) * (s - l)
        } else {
            walk.at(s)
        }
    }

    #[test]
    fn tweens_without_pins_evaluate_exactly_as_before() {
        let base = el(1, 0.0, 0.0);
        let props = Props::of(&base);
        let mut cases = Vec::new();
        for ease in [Easing::Linear, Easing::QuadOut, Easing::CubicBezier(0.2, 0.9, 0.4, 1.3), Easing::CubicBezier(0.5, -0.6, 0.5, 1.6)] {
            let mut m = diagonal(ease);
            assert!(m.set_path(&quarter(100.0), &props, 20));
            cases.push(m.clone());
            // Position keys in the middle are timing keys, not pins, in ADR 0030 files.
            m.set_key(Property::Y, 8, 30.0, 0.0);
            m.set_key(Property::X, 13, 70.0, 0.0);
            cases.push(m.clone());
            let mut s = along_s(ease).0;
            s.set_key(Property::X, 5, 20.0, 0.0);
            cases.push(s);
        }
        let mut ring = Path::new();
        ring.move_to(Vec2::new(0.0, 0.0)).line_to(Vec2::new(40.0, 0.0)).line_to(Vec2::new(40.0, 40.0)).line_to(Vec2::new(0.0, 40.0)).close();
        let mut l = MotionTween::new(ElementId(1));
        assert!(l.set_path(&ring, &props, 16));
        cases.push(l);
        for m in &cases {
            assert!(m.pins.is_empty() && m.is_sane());
            let op = m.on_path(&props).unwrap();
            for i in -4..=120 {
                let f = i as f32 / 4.0;
                assert_eq!(op.point(f), legacy_point(m, &props, f), "frame {f}");
            }
            let json = serde_json::to_string(m).unwrap();
            assert!(!json.contains("pins"), "nothing new is written for them");
        }
    }

    #[test]
    fn a_path_made_from_the_tracks_keeps_their_keys_pinned() {
        // Right 100 over 10 frames, then down 100: no path yet, but an anchor at the corner.
        let base = el(1, 0.0, 0.0);
        let props = Props::of(&base);
        let mut m = MotionTween::new(ElementId(1));
        m.tracks = vec![track(Property::X, &[(0, 0.0, Easing::Linear), (10, 100.0, Easing::Linear)]), track(Property::Y, &[(0, 0.0, Easing::Linear), (10, 0.0, Easing::Linear), (20, 100.0, Easing::Linear)])];
        let (mut path, pins) = m.path_and_pins_from_tracks(&props).unwrap();
        assert_eq!(pins, vec![Pin { frame: 10, anchor: 1 }]);
        assert_eq!(m.pinned_anchors(&props), vec![Pin { frame: 0, anchor: 0 }, Pin { frame: 10, anchor: 1 }, Pin { frame: 20, anchor: 2 }], "shown pinned before it is a path");
        // Dragging the corner up edits that path: the corner stays pinned to frame 10.
        assert!(path.move_anchor(1, Vec2::new(0.0, -40.0)));
        assert!(m.set_path(&path, &props, 20));
        assert_eq!(m.pins, pins);
        assert!(at(&m, &base, 10.0).distance(Vec2::new(100.0, -40.0)) < 1e-3);
        // A path of another shape is a new path: only its ends are pinned.
        let mut fresh = m.clone();
        fresh.path = None;
        fresh.pins.clear();
        assert!(fresh.set_path(&line((0.0, 0.0), (100.0, 100.0)), &props, 20));
        assert!(fresh.pins.is_empty());
    }

    #[test]
    fn splitting_keeps_the_pins_after_the_cut() {
        let (mut m, base) = along_s(Easing::Linear);
        let props = Props::of(&base);
        assert_eq!(m.pin(1, 6, &props), Ok(()));
        for cut in [3u32, 6, 10] {
            let tail = m.split_at(&props, cut);
            let tail_base = m.apply(&base, cut as f32);
            assert_eq!(tail.pins.len(), usize::from(cut < 6), "cut {cut}");
            for f in cut..=24 {
                let (a, b) = (at(&m, &base, f as f32), at(&tail, &tail_base, (f - cut) as f32));
                assert!(a.distance(b) < 0.05, "cut {cut} frame {f}: {a:?} vs {b:?}");
            }
        }
    }

    #[test]
    fn moving_on_past_the_end_carries_the_path_on() {
        let (mut m, base) = along_s(Easing::Linear);
        let props = Props::of(&base);
        let before: Vec<Vec2> = (0..=20).map(|f| at(&m, &base, f as f32)).collect();
        assert!(!m.extend_to(20, Vec2::new(250.0, 50.0), &props), "only after the last position key");
        assert!(m.extend_to(28, Vec2::new(250.0, 50.0), &props));
        assert_eq!(m.position_range(), Some((0, 28)));
        assert_eq!(m.pins, vec![Pin { frame: 20, anchor: 2 }], "the old end stays pinned to its frame");
        assert!(at(&m, &base, 28.0).distance(Vec2::new(250.0, 50.0)) < 1e-3);
        assert!(at(&m, &base, 24.0).distance(Vec2::new(225.0, 25.0)) < 1e-2, "on to the new end in a straight line");
        for (f, b) in before.iter().enumerate() {
            assert!(at(&m, &base, f as f32).distance(*b) < 1e-2, "frame {f}: as it was");
        }
    }

    #[test]
    fn loading_drops_pins_that_cannot_hold() {
        let (mut m, _) = along_s(Easing::Linear);
        m.pins = vec![Pin { frame: 5, anchor: 9 }, Pin { frame: 3, anchor: 1 }, Pin { frame: 3, anchor: 1 }, Pin { frame: 25, anchor: 1 }, Pin { frame: 0, anchor: 1 }];
        assert!(!m.is_sane());
        m.sanitize();
        assert!(m.is_sane());
        assert_eq!(m.pins, vec![Pin { frame: 3, anchor: 1 }]);
        let json = serde_json::to_string(&m).unwrap();
        assert!(json.contains("\"pins\""));
        assert_eq!(serde_json::from_str::<MotionTween>(&json).unwrap(), m);
        // Without a path nothing is pinned.
        m.path = None;
        assert!(!m.is_sane());
        m.sanitize();
        assert!(m.pins.is_empty());
    }
}
