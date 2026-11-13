//! Colour effects (ADR 0056): what Animate's Properties calls an instance's colour effect
//! *style* (None, Brightness, Tint, Advanced, Alpha), and the one colour transform every style
//! is (a multiply and an offset per channel).
//!
//! An element stores its [`ColorEffect`] as authored, so Properties shows the style and values
//! the artist chose (a tint keeps its colour at 0 %). Everything that draws works on its
//! [`ColorTransform`]: nested instances, the camera and onion skin compose their transforms
//! exactly (`concat` is the composition of the two maps), and every renderer applies the same
//! arithmetic to straight colour: `c ↦ clamp(c·multiply + offset, 0, 1)`.
//!
//! Brightness `b` is a tint towards white (`b > 0`) or black (`b < 0`) by `|b|`; a tint of
//! colour `t` by amount `k` multiplies each colour channel by `1 − k` and adds `k·t`; alpha
//! multiplies the alpha channel; Advanced is the transform itself (Animate's per-channel
//! percent and −255..255 offset, here as fractions: offset 1 is 255).

use crate::Rgba;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A colour transform on straight (not premultiplied) colour: channel `i` (red, green, blue,
/// alpha) becomes `clamp(c·mul[i] + add[i], 0, 1)`. Offsets are fractions of the full range.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ColorTransform {
    pub mul: [f32; 4],
    pub add: [f32; 4],
}

impl Default for ColorTransform {
    fn default() -> Self {
        ColorTransform::IDENTITY
    }
}

/// Animate's Advanced ranges: multipliers −100..=100 %, offsets −255..=255.
pub const ADVANCED_MUL: std::ops::RangeInclusive<f32> = -1.0..=1.0;
pub const ADVANCED_ADD: std::ops::RangeInclusive<f32> = -1.0..=1.0;

fn fin(v: f32, or: f32) -> f32 {
    if v.is_finite() { v } else { or }
}

impl ColorTransform {
    pub const IDENTITY: ColorTransform = ColorTransform { mul: [1.0; 4], add: [0.0; 4] };

    /// Multiplies alpha by `a`.
    pub fn alpha(a: f32) -> ColorTransform {
        ColorTransform { mul: [1.0, 1.0, 1.0, a], add: [0.0; 4] }
    }
    /// Mixes colour `t` in by `k` (alpha untouched).
    pub fn tint(t: Rgba, k: f32) -> ColorTransform {
        let j = 1.0 - k;
        ColorTransform { mul: [j, j, j, 1.0], add: [k * t.r, k * t.g, k * t.b, 0.0] }
    }
    /// Brightness `b` in −1..=1: a tint towards white or black by `|b|`.
    pub fn brightness(b: f32) -> ColorTransform {
        if b >= 0.0 { ColorTransform::tint(Rgba::WHITE, b) } else { ColorTransform::tint(Rgba::BLACK, -b) }
    }

    pub fn is_identity(&self) -> bool {
        *self == ColorTransform::IDENTITY
    }

    /// `self` applied after `inner`: the effect of `inner` nested inside `self`. Exact (no clamp
    /// between the two), so composing then applying is applying one then the other on colours
    /// that stay in range.
    pub fn concat(&self, inner: &ColorTransform) -> ColorTransform {
        let mut out = ColorTransform::IDENTITY;
        for i in 0..4 {
            out.mul[i] = self.mul[i] * inner.mul[i];
            out.add[i] = self.mul[i] * inner.add[i] + self.add[i];
        }
        out
    }

    pub fn lerp(&self, o: &ColorTransform, t: f32) -> ColorTransform {
        let mut out = ColorTransform::IDENTITY;
        for i in 0..4 {
            out.mul[i] = anicraft_geom::lerp(self.mul[i], o.mul[i], t);
            out.add[i] = anicraft_geom::lerp(self.add[i], o.add[i], t);
        }
        out
    }

    /// Non-finite values drawn as a channel of 0 (as a NaN colour always was).
    pub fn sanitized(&self) -> ColorTransform {
        let mut out = *self;
        for i in 0..4 {
            if !(out.mul[i].is_finite() && out.add[i].is_finite()) {
                out.mul[i] = 0.0;
                out.add[i] = 0.0;
            }
        }
        out
    }

    /// The transform applied to straight colour `c`, clamped to `0..=1` (NaN to 0).
    pub fn apply(&self, c: Rgba) -> Rgba {
        let f = self.sanitized();
        let ch = |v: f32, i: usize| {
            let x = fin(v, 0.0) * f.mul[i] + f.add[i];
            if x.is_finite() { x.clamp(0.0, 1.0) } else { 0.0 }
        };
        Rgba::new(ch(c.r, 0), ch(c.g, 1), ch(c.b, 2), ch(c.a, 3))
    }

    /// The alpha multiplier when alpha is only multiplied (no alpha offset): what can fade a
    /// whole group at once.
    pub fn alpha_factor(&self) -> Option<f32> {
        (self.add[3] == 0.0).then(|| fin(self.mul[3], 0.0).clamp(0.0, 1.0))
    }

    /// The same transform with alpha untouched.
    pub fn without_alpha(&self) -> ColorTransform {
        let mut out = *self;
        out.mul[3] = 1.0;
        out.add[3] = 0.0;
        out
    }

    /// Whether colour channels change (anything beyond alpha).
    pub fn changes_colour(&self) -> bool {
        self.mul[..3].iter().any(|m| *m != 1.0) || self.add[..3].iter().any(|a| *a != 0.0)
    }

    /// The alpha multiplier as an opacity estimate `0..=1` (picking, outlines).
    pub fn opacity(&self) -> f32 {
        fin(self.mul[3] + self.add[3], 0.0).clamp(0.0, 1.0)
    }
}

/// The colour effect styles of Properties' Style menu, in menu order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ColorStyle {
    None,
    Brightness,
    Tint,
    Advanced,
    Alpha,
}

impl ColorStyle {
    pub const ALL: [ColorStyle; 5] = [ColorStyle::None, ColorStyle::Brightness, ColorStyle::Tint, ColorStyle::Advanced, ColorStyle::Alpha];

    /// The name used in files and commands.
    pub fn key(self) -> &'static str {
        match self {
            ColorStyle::None => "none",
            ColorStyle::Brightness => "brightness",
            ColorStyle::Tint => "tint",
            ColorStyle::Advanced => "advanced",
            ColorStyle::Alpha => "alpha",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            ColorStyle::None => "None",
            ColorStyle::Brightness => "Brightness",
            ColorStyle::Tint => "Tint",
            ColorStyle::Advanced => "Advanced",
            ColorStyle::Alpha => "Alpha",
        }
    }
    pub fn from_key(s: &str) -> Option<ColorStyle> {
        let s = s.trim().to_ascii_lowercase();
        ColorStyle::ALL.into_iter().find(|x| x.key() == s)
    }
}

/// An element's colour effect, as authored: one style and its values. Draw it through
/// [`ColorEffect::transform`].
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum ColorEffect {
    #[default]
    None,
    /// Brightness −1..=1 (−100..=100 %).
    Brightness(f32),
    /// Tint colour (opaque) and amount 0..=1.
    Tint { color: Rgba, amount: f32 },
    /// Per-channel multiply and offset (Animate's Advanced).
    Advanced(ColorTransform),
    /// Alpha multiplier 0..=1.
    Alpha(f32),
}

impl ColorEffect {
    pub fn style(&self) -> ColorStyle {
        match self {
            ColorEffect::None => ColorStyle::None,
            ColorEffect::Brightness(_) => ColorStyle::Brightness,
            ColorEffect::Tint { .. } => ColorStyle::Tint,
            ColorEffect::Advanced(_) => ColorStyle::Advanced,
            ColorEffect::Alpha(_) => ColorStyle::Alpha,
        }
    }

    /// What it does to colour (its values clamped into the style's range first).
    pub fn transform(&self) -> ColorTransform {
        match self.sanitized() {
            ColorEffect::None => ColorTransform::IDENTITY,
            ColorEffect::Brightness(b) => ColorTransform::brightness(b),
            ColorEffect::Tint { color, amount } => ColorTransform::tint(color, amount),
            ColorEffect::Advanced(t) => t,
            ColorEffect::Alpha(a) => ColorTransform::alpha(a),
        }
    }

    /// The values clamped into each style's range, non-finite ones to the style's rest value.
    pub fn sanitized(&self) -> ColorEffect {
        match *self {
            ColorEffect::None => ColorEffect::None,
            ColorEffect::Brightness(b) => ColorEffect::Brightness(fin(b, 0.0).clamp(-1.0, 1.0)),
            ColorEffect::Tint { color, amount } => ColorEffect::Tint { color: color.sanitized().with_alpha(1.0), amount: fin(amount, 0.0).clamp(0.0, 1.0) },
            ColorEffect::Advanced(t) => {
                let mut o = t;
                for i in 0..4 {
                    o.mul[i] = fin(t.mul[i], 1.0).clamp(*ADVANCED_MUL.start(), *ADVANCED_MUL.end());
                    o.add[i] = fin(t.add[i], 0.0).clamp(*ADVANCED_ADD.start(), *ADVANCED_ADD.end());
                }
                ColorEffect::Advanced(o)
            }
            ColorEffect::Alpha(a) => ColorEffect::Alpha(fin(a, 1.0).clamp(0.0, 1.0)),
        }
    }

    /// The alpha multiplier the effect has (1 for styles that leave alpha alone).
    pub fn alpha(&self) -> f32 {
        self.transform().mul[3]
    }

    /// The effect with its alpha multiplier set to `a`, the rest kept (what ActionScript's
    /// `alpha` and `element.set_alpha` do): None and Alpha become Alpha(a); Advanced keeps
    /// its other channels; Brightness and Tint become Advanced when `a` isn't 1.
    pub fn with_alpha(&self, a: f32) -> ColorEffect {
        match *self {
            ColorEffect::None | ColorEffect::Alpha(_) => ColorEffect::Alpha(a),
            ColorEffect::Advanced(mut t) => {
                t.mul[3] = a;
                ColorEffect::Advanced(t)
            }
            ColorEffect::Brightness(_) | ColorEffect::Tint { .. } if a == 1.0 => *self,
            ColorEffect::Brightness(_) | ColorEffect::Tint { .. } => {
                let mut t = self.transform();
                t.mul[3] = a;
                ColorEffect::Advanced(t)
            }
        }
    }

    /// The effect in `style`, as close to this one as that style can be: Advanced is exactly
    /// this transform; Alpha keeps the alpha multiplier; a tint keeps its colour; Brightness and
    /// Tint otherwise start where they change nothing (the look doesn't jump when the style
    /// changes, except to None).
    pub fn to_style(&self, style: ColorStyle) -> ColorEffect {
        if self.style() == style {
            return *self;
        }
        match style {
            ColorStyle::None => ColorEffect::None,
            ColorStyle::Advanced => ColorEffect::Advanced(self.transform()),
            ColorStyle::Alpha => ColorEffect::Alpha(fin(self.alpha(), 1.0).clamp(0.0, 1.0)),
            ColorStyle::Brightness => ColorEffect::Brightness(0.0),
            ColorStyle::Tint => ColorEffect::Tint { color: Rgba::WHITE, amount: 0.0 },
        }
    }

    /// `inner` nested inside `self` as one effect (Break Apart): the other when either is None,
    /// alphas multiplied, else the composed transform as Advanced.
    pub fn nested(&self, inner: &ColorEffect) -> ColorEffect {
        match (self, inner) {
            (ColorEffect::None, x) | (x, ColorEffect::None) => *x,
            (ColorEffect::Alpha(a), ColorEffect::Alpha(b)) => ColorEffect::Alpha(a * b),
            (o, i) => ColorEffect::Advanced(o.transform().concat(&i.transform())),
        }
    }

    /// Alpha `a`, or None at full opacity.
    pub fn opacity(a: f32) -> ColorEffect {
        if a < 1.0 { ColorEffect::Alpha(a) } else { ColorEffect::None }
    }

    /// The classic tween between two effects: within one style its values travel (a tint's
    /// colour and amount); None meets any style as that style's rest value; any other change of
    /// style travels in the Advanced space (multiplies and offsets), as in Animate.
    pub fn lerp(&self, o: &ColorEffect, t: f32) -> ColorEffect {
        use anicraft_geom::lerp;
        let rest = |s: ColorStyle| ColorEffect::None.to_style(s);
        let (a, b) = match (self, o) {
            (ColorEffect::None, ColorEffect::None) => return ColorEffect::None,
            (ColorEffect::None, x) if x.style() != ColorStyle::Advanced => (rest(x.style()), *x),
            (x, ColorEffect::None) if x.style() != ColorStyle::Advanced => (*x, rest(x.style())),
            (x, y) => (*x, *y),
        };
        // A tint meeting None keeps its colour all the way.
        let (a, b) = match (a, b) {
            (ColorEffect::Tint { amount, .. }, ColorEffect::Tint { color, amount: k }) if self.style() == ColorStyle::None => (ColorEffect::Tint { color, amount }, ColorEffect::Tint { color, amount: k }),
            (ColorEffect::Tint { color, amount }, ColorEffect::Tint { amount: k, .. }) if o.style() == ColorStyle::None => (ColorEffect::Tint { color, amount }, ColorEffect::Tint { color, amount: k }),
            p => p,
        };
        match (a, b) {
            (ColorEffect::Brightness(x), ColorEffect::Brightness(y)) => ColorEffect::Brightness(lerp(x, y, t)),
            (ColorEffect::Alpha(x), ColorEffect::Alpha(y)) => ColorEffect::Alpha(lerp(x, y, t)),
            (ColorEffect::Tint { color: c1, amount: k1 }, ColorEffect::Tint { color: c2, amount: k2 }) => ColorEffect::Tint { color: c1.lerp(c2, t), amount: lerp(k1, k2, t) },
            (x, y) => ColorEffect::Advanced(x.transform().lerp(&y.transform(), t)),
        }
    }

    /// The effect several families make together (a motion tween keying more than one, or an
    /// older file's alpha with a tint): the one family when only one is in use, else Advanced
    /// with them composed (alpha outermost, then brightness, tint, advanced).
    pub fn compose(alpha: Option<f32>, brightness: Option<f32>, tint: Option<(Rgba, f32)>, advanced: Option<ColorTransform>) -> ColorEffect {
        let n = usize::from(alpha.is_some()) + usize::from(brightness.is_some()) + usize::from(tint.is_some()) + usize::from(advanced.is_some());
        if n == 1 {
            if let Some(a) = alpha {
                return ColorEffect::Alpha(a);
            }
            if let Some(b) = brightness {
                return ColorEffect::Brightness(b);
            }
            if let Some((color, amount)) = tint {
                return ColorEffect::Tint { color, amount };
            }
            if let Some(t) = advanced {
                return ColorEffect::Advanced(t);
            }
        }
        if n == 0 {
            return ColorEffect::None;
        }
        let mut t = advanced.unwrap_or(ColorTransform::IDENTITY);
        if let Some((c, k)) = tint {
            t = ColorTransform::tint(c, k).concat(&t);
        }
        if let Some(b) = brightness {
            t = ColorTransform::brightness(b).concat(&t);
        }
        if let Some(a) = alpha {
            t = ColorTransform::alpha(a).concat(&t);
        }
        ColorEffect::Advanced(t)
    }
}

// ----- files ---------------------------------------------------------------------------------
//
// Every effect is written with the fields older AniCraft files have (`alpha`, `tint`,
// `tint_amount`: the look as near as they can say it), so None, Alpha and Tint effects are
// written exactly as before. `style` (and `brightness`, `multiply`, `offset`) are added only
// when those fields alone would read back as something else. A file without `style` reads as
// it always looked: a tint, an alpha, both (Advanced), or none.

fn one() -> f32 {
    1.0
}
fn white() -> Rgba {
    Rgba::WHITE
}

#[derive(Serialize, Deserialize)]
struct Wire {
    #[serde(default = "one")]
    alpha: f32,
    #[serde(default = "white")]
    tint: Rgba,
    #[serde(default)]
    tint_amount: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    style: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    brightness: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    multiply: Option<[f32; 4]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    offset: Option<[f32; 4]>,
}

/// What the legacy fields alone say.
fn legacy(alpha: f32, tint: Rgba, amount: f32) -> ColorEffect {
    // Older builds clamped these where they drew them.
    let (alpha, amount) = (fin(alpha, 1.0).clamp(0.0, 1.0), fin(amount, 0.0).clamp(0.0, 1.0));
    let a = (alpha != 1.0).then_some(alpha);
    let t = (amount != 0.0).then_some((tint, amount));
    ColorEffect::compose(a, None, t, None)
}

impl Serialize for ColorEffect {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut w = Wire { alpha: 1.0, tint: Rgba::WHITE, tint_amount: 0.0, style: None, brightness: None, multiply: None, offset: None };
        match *self {
            ColorEffect::None => {}
            ColorEffect::Alpha(a) => w.alpha = a,
            ColorEffect::Tint { color, amount } => (w.tint, w.tint_amount) = (color, amount),
            ColorEffect::Brightness(b) => {
                (w.tint, w.tint_amount) = (if b >= 0.0 { Rgba::WHITE } else { Rgba::BLACK }, b.abs());
                w.brightness = Some(b);
            }
            ColorEffect::Advanced(t) => {
                w.alpha = t.mul[3];
                (w.multiply, w.offset) = (Some(t.mul), Some(t.add));
            }
        }
        if legacy(w.alpha, w.tint, w.tint_amount) != *self {
            w.style = Some(self.style().key().to_string());
        }
        w.serialize(s)
    }
}

impl<'de> Deserialize<'de> for ColorEffect {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let w = Wire::deserialize(d)?;
        let fx = match w.style.as_deref().and_then(ColorStyle::from_key) {
            None => legacy(w.alpha, w.tint, w.tint_amount),
            Some(ColorStyle::None) => ColorEffect::None,
            Some(ColorStyle::Alpha) => ColorEffect::Alpha(w.alpha),
            Some(ColorStyle::Tint) => ColorEffect::Tint { color: w.tint, amount: w.tint_amount },
            Some(ColorStyle::Brightness) => ColorEffect::Brightness(w.brightness.unwrap_or(0.0)),
            Some(ColorStyle::Advanced) => ColorEffect::Advanced(ColorTransform { mul: w.multiply.unwrap_or([1.0; 4]), add: w.offset.unwrap_or([0.0; 4]) }),
        };
        Ok(fx.sanitized())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Rgba, b: Rgba) -> bool {
        (a.r - b.r).abs() < 1e-5 && (a.g - b.g).abs() < 1e-5 && (a.b - b.b).abs() < 1e-5 && (a.a - b.a).abs() < 1e-5
    }

    #[test]
    fn styles_are_the_transforms_animate_documents() {
        let c = Rgba::new(0.4, 0.2, 0.8, 1.0);
        // Brightness +60 %: towards white; −50 %: towards black.
        assert!(close(ColorEffect::Brightness(0.6).transform().apply(c), Rgba::new(0.4 * 0.4 + 0.6, 0.2 * 0.4 + 0.6, 0.8 * 0.4 + 0.6, 1.0)));
        assert!(close(ColorEffect::Brightness(-0.5).transform().apply(c), Rgba::new(0.2, 0.1, 0.4, 1.0)));
        // Tint 25 % red.
        let t = ColorEffect::Tint { color: Rgba::new(1.0, 0.0, 0.0, 1.0), amount: 0.25 }.transform().apply(c);
        assert!(close(t, Rgba::new(0.4 * 0.75 + 0.25, 0.15, 0.6, 1.0)));
        // Advanced: red ×50 %, green +51 (0.2), alpha ×80 % −25.5 (−0.1); clamped.
        let adv = ColorTransform { mul: [0.5, 1.0, 1.0, 0.8], add: [0.0, 0.2, 0.5, -0.1] };
        assert!(close(adv.apply(c), Rgba::new(0.2, 0.4, 1.0, 0.7)));
        assert!(close(ColorEffect::Alpha(0.3).transform().apply(c), c.with_alpha(0.3)));
    }

    #[test]
    fn composition_is_exact() {
        let outer = ColorTransform::tint(Rgba::new(0.0, 0.5, 1.0, 1.0), 0.4);
        let inner = ColorTransform { mul: [0.5, -0.2, 0.9, 0.7], add: [0.1, 0.6, 0.0, 0.05] };
        let c = Rgba::new(0.3, 0.6, 0.2, 0.9);
        assert!(close(outer.concat(&inner).apply(c), outer.apply(inner.apply(c))));
        assert_eq!(ColorTransform::IDENTITY.concat(&inner), inner);
        assert_eq!(inner.concat(&ColorTransform::IDENTITY), inner);
    }

    #[test]
    fn nan_draws_a_zero_channel() {
        let t = ColorTransform { mul: [f32::NAN, 1.0, 1.0, 1.0], add: [0.0; 4] };
        assert_eq!(t.apply(Rgba::WHITE), Rgba::new(0.0, 1.0, 1.0, 1.0));
        assert_eq!(ColorEffect::Tint { color: Rgba::WHITE, amount: f32::NAN }.sanitized(), ColorEffect::Tint { color: Rgba::WHITE, amount: 0.0 });
    }

    #[test]
    fn tweens_within_a_style_and_across_styles() {
        let red = Rgba::new(1.0, 0.0, 0.0, 1.0);
        let blue = Rgba::new(0.0, 0.0, 1.0, 1.0);
        assert_eq!(ColorEffect::Alpha(1.0).lerp(&ColorEffect::Alpha(0.0), 0.25), ColorEffect::Alpha(0.75));
        assert_eq!(ColorEffect::None.lerp(&ColorEffect::Brightness(-1.0), 0.5), ColorEffect::Brightness(-0.5));
        assert_eq!(ColorEffect::None.lerp(&ColorEffect::Tint { color: red, amount: 1.0 }, 0.5), ColorEffect::Tint { color: red, amount: 0.5 });
        assert_eq!(ColorEffect::Tint { color: red, amount: 1.0 }.lerp(&ColorEffect::Tint { color: blue, amount: 1.0 }, 0.5), ColorEffect::Tint { color: Rgba::new(0.5, 0.0, 0.5, 1.0), amount: 1.0 });
        // A style change travels in the advanced space: halfway between the two transforms.
        let a = ColorEffect::Tint { color: red, amount: 1.0 };
        let b = ColorEffect::Alpha(0.5);
        let mid = a.lerp(&b, 0.5);
        assert_eq!(mid.style(), ColorStyle::Advanced);
        assert_eq!(mid.transform(), a.transform().lerp(&b.transform(), 0.5));
        // The ends look like the keyframes.
        assert_eq!(a.lerp(&b, 0.0).transform(), a.transform());
        assert_eq!(a.lerp(&b, 1.0).transform(), b.transform());
    }

    #[test]
    fn style_changes_keep_the_look_where_they_can() {
        let fx = ColorEffect::Tint { color: Rgba::new(1.0, 0.0, 0.0, 1.0), amount: 0.4 };
        assert_eq!(fx.to_style(ColorStyle::Advanced).transform(), fx.transform());
        assert_eq!(ColorEffect::Alpha(0.3).to_style(ColorStyle::Advanced).alpha(), 0.3);
        assert_eq!(ColorEffect::Advanced(ColorTransform::alpha(0.3)).to_style(ColorStyle::Alpha), ColorEffect::Alpha(0.3));
        assert!(fx.to_style(ColorStyle::Brightness).transform().is_identity());
        assert_eq!(fx.with_alpha(0.5).transform(), ColorTransform::alpha(0.5).concat(&fx.transform()));
        assert_eq!(fx.with_alpha(1.0), fx);
        assert_eq!(ColorEffect::None.with_alpha(0.2), ColorEffect::Alpha(0.2));
    }

    #[test]
    fn older_files_read_as_they_looked_and_simple_effects_write_as_before() {
        let read = |s: &str| serde_json::from_str::<ColorEffect>(s).unwrap();
        let white = r#""tint":{"r":1.0,"g":1.0,"b":1.0,"a":1.0}"#;
        assert_eq!(read(&format!(r#"{{"alpha":1.0,{white},"tint_amount":0.0}}"#)), ColorEffect::None);
        assert_eq!(read(&format!(r#"{{"alpha":0.5,{white},"tint_amount":0.0}}"#)), ColorEffect::Alpha(0.5));
        assert_eq!(read(&format!(r#"{{"alpha":1.0,{white},"tint_amount":0.25}}"#)), ColorEffect::Tint { color: Rgba::WHITE, amount: 0.25 });
        let both = read(&format!(r#"{{"alpha":0.5,{white},"tint_amount":0.25}}"#));
        assert_eq!(both.transform(), ColorTransform::alpha(0.5).concat(&ColorTransform::tint(Rgba::WHITE, 0.25)));
        // Byte for byte as before.
        assert_eq!(serde_json::to_string(&ColorEffect::None).unwrap(), format!(r#"{{"alpha":1.0,{white},"tint_amount":0.0}}"#));
        assert_eq!(serde_json::to_string(&ColorEffect::Alpha(0.5)).unwrap(), format!(r#"{{"alpha":0.5,{white},"tint_amount":0.0}}"#));
        for fx in [
            ColorEffect::None,
            ColorEffect::Alpha(1.0),
            ColorEffect::Alpha(0.25),
            ColorEffect::Brightness(-0.5),
            ColorEffect::Brightness(0.0),
            ColorEffect::Tint { color: Rgba::new(1.0, 0.0, 0.0, 1.0), amount: 0.0 },
            ColorEffect::Tint { color: Rgba::new(0.0, 0.5, 1.0, 1.0), amount: 0.75 },
            ColorEffect::Advanced(ColorTransform { mul: [0.5, -1.0, 1.0, 0.75], add: [0.25, 0.0, -0.5, 0.125] }),
            ColorEffect::Advanced(ColorTransform::IDENTITY),
        ] {
            let json = serde_json::to_string(&fx).unwrap();
            assert_eq!(read(&json), fx, "{json}");
        }
        // Brightness still reads, in older builds, as the tint it looks like.
        let json = serde_json::to_string(&ColorEffect::Brightness(-0.5)).unwrap();
        assert!(json.contains(r#""tint_amount":0.5"#) && json.contains(r#""style":"brightness""#), "{json}");
        // Out-of-range values in a file are clamped; an unknown style reads as the legacy fields.
        assert_eq!(read(r#"{"style":"brightness","brightness":7}"#), ColorEffect::Brightness(1.0));
        assert_eq!(read(r#"{"style":"sepia","alpha":0.5}"#), ColorEffect::Alpha(0.5));
    }
}
