//! Filters on movie clip and button instances, as Animate's Filters panel: Drop Shadow, Blur,
//! Glow, Bevel, Gradient Glow, Gradient Bevel and Adjust Color. Sizes are in stage pixels (they
//! don't scale with the instance, as in Animate); they scale with the view.

use crate::{GradientStop, Rgba};
use serde::{Deserialize, Serialize};

/// Blur passes: Low, Medium, High (each pass is a box blur; three approach a Gaussian).
pub const MAX_QUALITY: u8 = 3;
/// Largest blur, in stage pixels (Animate's limit).
pub const MAX_BLUR: f32 = 255.0;

fn one() -> u8 {
    1
}
fn full() -> f32 {
    1.0
}
fn black() -> Rgba {
    Rgba::BLACK
}
fn angle() -> f32 {
    45.0
}
fn four() -> f32 {
    4.0
}
fn five() -> f32 {
    5.0
}
fn white() -> Rgba {
    Rgba::WHITE
}
fn outer() -> FilterSide {
    FilterSide::Outer
}

/// Most colour stops in a gradient filter's ramp (Animate's limit).
pub const MAX_RAMP_STOPS: usize = 15;

/// Where a bevel or gradient filter draws: inside the artwork's edge, outside it, or both
/// (Animate's Type menu).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilterSide {
    #[default]
    Inner,
    Outer,
    Full,
}

impl FilterSide {
    pub const ALL: [FilterSide; 3] = [FilterSide::Inner, FilterSide::Outer, FilterSide::Full];
    pub fn label(self) -> &'static str {
        match self {
            FilterSide::Inner => "Inner",
            FilterSide::Outer => "Outer",
            FilterSide::Full => "Full",
        }
    }
}

fn stop(offset: f32, color: Rgba) -> GradientStop {
    GradientStop { offset, color }
}

/// Animate's default Gradient Glow ramp: transparent white into black.
pub fn default_glow_ramp() -> Vec<GradientStop> {
    vec![stop(0.0, Rgba::WHITE.with_alpha(0.0)), stop(1.0, Rgba::BLACK)]
}

/// Animate's default Gradient Bevel ramp: white highlight, transparent middle, black shadow.
pub fn default_bevel_ramp() -> Vec<GradientStop> {
    vec![stop(0.0, Rgba::WHITE), stop(0.5, Rgba::BLACK.with_alpha(0.0)), stop(1.0, Rgba::BLACK)]
}

/// The ramp's colour at `t` (stops sorted by offset; the ends extend).
pub fn ramp_at(stops: &[GradientStop], t: f32) -> Rgba {
    let Some(first) = stops.first() else { return Rgba::TRANSPARENT };
    if t.is_nan() || t <= first.offset {
        return first.color;
    }
    for w in stops.windows(2) {
        if let [a, b] = w
            && t <= b.offset
        {
            let span = b.offset - a.offset;
            let k = if span > 1e-6 { (t - a.offset) / span } else { 1.0 };
            return a.color.lerp(b.color, k);
        }
    }
    stops.last().map_or(first.color, |s| s.color)
}

/// A ramp with finite stops in order, at most [`MAX_RAMP_STOPS`], and a transparent stop where
/// the filter rests (`rest`: no glow, or a flat bevel). Animate pins that stop, so a gradient
/// filter paints nothing away from the edges.
fn sanitized_ramp(stops: &[GradientStop], rest: f32, fallback: fn() -> Vec<GradientStop>) -> Vec<GradientStop> {
    let mut v: Vec<GradientStop> = stops.iter().filter(|s| s.offset.is_finite()).map(|s| stop(s.offset.clamp(0.0, 1.0), s.color.sanitized())).collect();
    if v.is_empty() {
        v = fallback();
    }
    v.sort_by(|a, b| a.offset.total_cmp(&b.offset));
    v.truncate(MAX_RAMP_STOPS);
    let far = |s: &GradientStop| (s.offset - rest).abs();
    // The stop nearest the rest point, if it is close enough, becomes the pinned one; others
    // that close step aside, so exactly one stop is pinned and sanitizing again changes nothing.
    if let Some(pin) = (0..v.len()).filter(|&i| far(&v[i]) < REST_SNAP).min_by(|&a, &b| far(&v[a]).total_cmp(&far(&v[b]))) {
        for (i, s) in v.iter_mut().enumerate() {
            if i == pin {
                s.offset = rest;
                s.color.a = 0.0;
            } else if far(s) < REST_SNAP {
                s.offset = if i < pin { rest - REST_SNAP } else { rest + REST_SNAP }.clamp(0.0, 1.0);
            }
        }
        v.sort_by(|a, b| a.offset.total_cmp(&b.offset));
        return v;
    }
    // Insert a transparent stop of the ramp's colour there, keeping within the limit (the
    // stop nearest the rest point makes room).
    let c = ramp_at(&v, rest);
    if v.len() >= MAX_RAMP_STOPS
        && let Some(i) = (0..v.len()).min_by(|&a, &b| far(&v[a]).total_cmp(&far(&v[b])))
    {
        v.remove(i);
    }
    let at = v.iter().position(|s| s.offset > rest).unwrap_or(v.len());
    v.insert(at, stop(rest, c.with_alpha(0.0)));
    v
}

/// How near the rest point a stop counts as the pinned one.
pub const REST_SNAP: f32 = 1e-3;

/// The offset of the ramp stop that can't move, be removed or be seen: the Gradient Glow's
/// start, the Gradient Bevel's middle.
pub fn pinned_stop(f: &Filter) -> Option<f32> {
    match f {
        Filter::GradientGlow { .. } => Some(0.0),
        Filter::GradientBevel { .. } => Some(0.5),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Filter {
    Blur {
        #[serde(default = "five")]
        blur_x: f32,
        #[serde(default = "five")]
        blur_y: f32,
        #[serde(default = "one")]
        quality: u8,
    },
    DropShadow {
        #[serde(default = "five")]
        blur_x: f32,
        #[serde(default = "five")]
        blur_y: f32,
        /// Opacity multiplier of the shadow (1 = 100%).
        #[serde(default = "full")]
        strength: f32,
        #[serde(default = "one")]
        quality: u8,
        /// Degrees, clockwise from the right (45 = down and right).
        #[serde(default = "angle")]
        angle: f32,
        #[serde(default = "four")]
        distance: f32,
        #[serde(default = "black")]
        color: Rgba,
        #[serde(default)]
        inner: bool,
        #[serde(default)]
        knockout: bool,
        #[serde(default)]
        hide_object: bool,
    },
    Glow {
        #[serde(default = "five")]
        blur_x: f32,
        #[serde(default = "five")]
        blur_y: f32,
        #[serde(default = "full")]
        strength: f32,
        #[serde(default = "one")]
        quality: u8,
        #[serde(default = "black")]
        color: Rgba,
        #[serde(default)]
        inner: bool,
        #[serde(default)]
        knockout: bool,
    },
    /// A raised edge: a highlight on the side the light comes from and a shadow on the other,
    /// from the blurred opacity offset both ways along the angle.
    Bevel {
        #[serde(default = "five")]
        blur_x: f32,
        #[serde(default = "five")]
        blur_y: f32,
        #[serde(default = "full")]
        strength: f32,
        #[serde(default = "one")]
        quality: u8,
        /// Degrees, clockwise from the right: the shadow falls this way, the highlight opposite
        /// (45 = light from the top left).
        #[serde(default = "angle")]
        angle: f32,
        #[serde(default = "five")]
        distance: f32,
        #[serde(default = "black")]
        shadow_color: Rgba,
        #[serde(default = "white")]
        highlight_color: Rgba,
        #[serde(default)]
        side: FilterSide,
        #[serde(default)]
        knockout: bool,
    },
    /// A glow coloured by a ramp: the glow's intensity (0 where it fades out, 1 at full)
    /// picks the colour. The ramp's start is pinned transparent.
    GradientGlow {
        #[serde(default = "five")]
        blur_x: f32,
        #[serde(default = "five")]
        blur_y: f32,
        #[serde(default = "full")]
        strength: f32,
        #[serde(default = "one")]
        quality: u8,
        #[serde(default = "angle")]
        angle: f32,
        #[serde(default = "five")]
        distance: f32,
        #[serde(default = "default_glow_ramp")]
        stops: Vec<GradientStop>,
        #[serde(default = "outer")]
        side: FilterSide,
        #[serde(default)]
        knockout: bool,
    },
    /// A bevel coloured by a ramp: the highlight at 0, flat at the middle (pinned
    /// transparent), the shadow at 1.
    GradientBevel {
        #[serde(default = "five")]
        blur_x: f32,
        #[serde(default = "five")]
        blur_y: f32,
        #[serde(default = "full")]
        strength: f32,
        #[serde(default = "one")]
        quality: u8,
        #[serde(default = "angle")]
        angle: f32,
        #[serde(default = "five")]
        distance: f32,
        #[serde(default = "default_bevel_ramp")]
        stops: Vec<GradientStop>,
        #[serde(default)]
        side: FilterSide,
        #[serde(default)]
        knockout: bool,
    },
    /// Each in −100..=100 (hue in −180..=180 degrees), 0 = unchanged.
    AdjustColor {
        #[serde(default)]
        brightness: f32,
        #[serde(default)]
        contrast: f32,
        #[serde(default)]
        saturation: f32,
        #[serde(default)]
        hue: f32,
    },
}

impl Filter {
    /// The filter's name in Animate's Filters menu.
    pub fn label(&self) -> &'static str {
        match self {
            Filter::Blur { .. } => "Blur",
            Filter::DropShadow { .. } => "Drop Shadow",
            Filter::Glow { .. } => "Glow",
            Filter::Bevel { .. } => "Bevel",
            Filter::GradientGlow { .. } => "Gradient Glow",
            Filter::GradientBevel { .. } => "Gradient Bevel",
            Filter::AdjustColor { .. } => "Adjust Color",
        }
    }

    /// A new filter of `kind` (`blur`, `drop_shadow`, `glow`, `bevel`, `gradient_glow`,
    /// `gradient_bevel`, `adjust_color`) with Animate's defaults.
    pub fn new(kind: &str) -> Option<Filter> {
        Some(match kind {
            "blur" => Filter::Blur { blur_x: five(), blur_y: five(), quality: one() },
            "drop_shadow" => Filter::DropShadow { blur_x: five(), blur_y: five(), strength: full(), quality: one(), angle: angle(), distance: four(), color: black(), inner: false, knockout: false, hide_object: false },
            "glow" => Filter::Glow { blur_x: five(), blur_y: five(), strength: full(), quality: one(), color: Rgba::hex(0xff0000), inner: false, knockout: false },
            "bevel" => Filter::Bevel { blur_x: five(), blur_y: five(), strength: full(), quality: one(), angle: angle(), distance: five(), shadow_color: black(), highlight_color: white(), side: FilterSide::Inner, knockout: false },
            "gradient_glow" => Filter::GradientGlow { blur_x: five(), blur_y: five(), strength: full(), quality: one(), angle: angle(), distance: five(), stops: default_glow_ramp(), side: FilterSide::Outer, knockout: false },
            "gradient_bevel" => Filter::GradientBevel { blur_x: five(), blur_y: five(), strength: full(), quality: one(), angle: angle(), distance: five(), stops: default_bevel_ramp(), side: FilterSide::Inner, knockout: false },
            "adjust_color" => Filter::AdjustColor { brightness: 0.0, contrast: 0.0, saturation: 0.0, hue: 0.0 },
            _ => return None,
        })
    }

    /// The kinds [`Filter::new`] makes, in Animate's menu order.
    pub const KINDS: [(&'static str, &'static str); 7] = [
        ("drop_shadow", "Drop Shadow"),
        ("blur", "Blur"),
        ("glow", "Glow"),
        ("bevel", "Bevel"),
        ("gradient_glow", "Gradient Glow"),
        ("gradient_bevel", "Gradient Bevel"),
        ("adjust_color", "Adjust Color"),
    ];

    /// The same filter with every value finite and in range.
    pub fn sanitized(&self) -> Filter {
        let fin = |v: f32, d: f32| if v.is_finite() { v } else { d };
        let blur = |v: f32| fin(v, 0.0).clamp(0.0, MAX_BLUR);
        let q = |v: u8| v.clamp(1, MAX_QUALITY);
        let strength = |v: f32| fin(v, 1.0).clamp(0.0, 10.0);
        let angle = |v: f32| fin(v, 45.0).rem_euclid(360.0);
        let distance = |v: f32| fin(v, 0.0).clamp(-1000.0, 1000.0);
        match self {
            Filter::Blur { blur_x, blur_y, quality } => Filter::Blur { blur_x: blur(*blur_x), blur_y: blur(*blur_y), quality: q(*quality) },
            Filter::DropShadow { blur_x, blur_y, strength: s, quality, angle: a, distance: d, color, inner, knockout, hide_object } => Filter::DropShadow {
                blur_x: blur(*blur_x),
                blur_y: blur(*blur_y),
                strength: strength(*s),
                quality: q(*quality),
                angle: angle(*a),
                distance: distance(*d),
                color: color.sanitized(),
                inner: *inner,
                knockout: *knockout,
                hide_object: *hide_object,
            },
            Filter::Glow { blur_x, blur_y, strength: s, quality, color, inner, knockout } => {
                Filter::Glow { blur_x: blur(*blur_x), blur_y: blur(*blur_y), strength: strength(*s), quality: q(*quality), color: color.sanitized(), inner: *inner, knockout: *knockout }
            }
            Filter::Bevel { blur_x, blur_y, strength: s, quality, angle: a, distance: d, shadow_color, highlight_color, side, knockout } => Filter::Bevel {
                blur_x: blur(*blur_x),
                blur_y: blur(*blur_y),
                strength: strength(*s),
                quality: q(*quality),
                angle: angle(*a),
                distance: distance(*d),
                shadow_color: shadow_color.sanitized(),
                highlight_color: highlight_color.sanitized(),
                side: *side,
                knockout: *knockout,
            },
            Filter::GradientGlow { blur_x, blur_y, strength: s, quality, angle: a, distance: d, stops, side, knockout } => Filter::GradientGlow {
                blur_x: blur(*blur_x),
                blur_y: blur(*blur_y),
                strength: strength(*s),
                quality: q(*quality),
                angle: angle(*a),
                distance: distance(*d),
                stops: sanitized_ramp(stops, 0.0, default_glow_ramp),
                side: *side,
                knockout: *knockout,
            },
            Filter::GradientBevel { blur_x, blur_y, strength: s, quality, angle: a, distance: d, stops, side, knockout } => Filter::GradientBevel {
                blur_x: blur(*blur_x),
                blur_y: blur(*blur_y),
                strength: strength(*s),
                quality: q(*quality),
                angle: angle(*a),
                distance: distance(*d),
                stops: sanitized_ramp(stops, 0.5, default_bevel_ramp),
                side: *side,
                knockout: *knockout,
            },
            Filter::AdjustColor { brightness, contrast, saturation, hue } => Filter::AdjustColor {
                brightness: fin(*brightness, 0.0).clamp(-100.0, 100.0),
                contrast: fin(*contrast, 0.0).clamp(-100.0, 100.0),
                saturation: fin(*saturation, 0.0).clamp(-100.0, 100.0),
                hue: fin(*hue, 0.0).clamp(-180.0, 180.0),
            },
        }
    }

    /// How far the filter can spread pixels beyond the artwork, in stage pixels.
    pub fn margin(&self) -> f32 {
        match self.sanitized() {
            Filter::Blur { blur_x, blur_y, quality } => blur_x.max(blur_y) * 0.5 * f32::from(quality) + 1.0,
            // Inner effects need the room too: their blur reads the transparency around the
            // artwork (and a bevel reads a distance both ways), which must be in the buffer.
            Filter::Glow { blur_x, blur_y, quality, .. } => blur_x.max(blur_y) * 0.5 * f32::from(quality) + 1.0,
            Filter::DropShadow { blur_x, blur_y, quality, distance, .. }
            | Filter::Bevel { blur_x, blur_y, quality, distance, .. }
            | Filter::GradientGlow { blur_x, blur_y, quality, distance, .. }
            | Filter::GradientBevel { blur_x, blur_y, quality, distance, .. } => blur_x.max(blur_y) * 0.5 * f32::from(quality) + distance.abs() + 1.0,
            Filter::AdjustColor { .. } => 0.0,
        }
    }
}

/// A number of a filter a motion tween can animate (ADR 0092), in the units the Motion Editor
/// and the Filters panel show: pixels, percent (strength) and degrees.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum FilterParam {
    BlurX,
    BlurY,
    /// Percent (100 = the filter's full strength).
    Strength,
    /// Degrees, clockwise from the right; not wrapped in keys, so 0 → 720 turns twice.
    Angle,
    Distance,
    /// Adjust Color's four.
    Brightness,
    Contrast,
    Saturation,
    Hue,
}

impl FilterParam {
    pub const ALL: [FilterParam; 9] = [
        FilterParam::BlurX,
        FilterParam::BlurY,
        FilterParam::Strength,
        FilterParam::Angle,
        FilterParam::Distance,
        FilterParam::Brightness,
        FilterParam::Contrast,
        FilterParam::Saturation,
        FilterParam::Hue,
    ];

    /// The name in files and commands (`blur_x`, `strength`…), as the filter's own field.
    pub fn key(self) -> &'static str {
        match self {
            FilterParam::BlurX => "blur_x",
            FilterParam::BlurY => "blur_y",
            FilterParam::Strength => "strength",
            FilterParam::Angle => "angle",
            FilterParam::Distance => "distance",
            FilterParam::Brightness => "brightness",
            FilterParam::Contrast => "contrast",
            FilterParam::Saturation => "saturation",
            FilterParam::Hue => "hue",
        }
    }

    pub fn from_key(s: &str) -> Option<FilterParam> {
        let s = s.trim().to_ascii_lowercase().replace([' ', '-'], "_");
        let s = match s.as_str() {
            "blurx" => "blur_x",
            "blury" => "blur_y",
            other => other,
        };
        FilterParam::ALL.into_iter().find(|p| p.key() == s)
    }

    pub fn label(self) -> &'static str {
        match self {
            FilterParam::BlurX => "Blur X",
            FilterParam::BlurY => "Blur Y",
            FilterParam::Strength => "Strength",
            FilterParam::Angle => "Angle",
            FilterParam::Distance => "Distance",
            FilterParam::Brightness => "Brightness",
            FilterParam::Contrast => "Contrast",
            FilterParam::Saturation => "Saturation",
            FilterParam::Hue => "Hue",
        }
    }

    pub fn unit(self) -> &'static str {
        match self {
            FilterParam::BlurX | FilterParam::BlurY | FilterParam::Distance => "px",
            FilterParam::Strength => "%",
            FilterParam::Angle | FilterParam::Hue => "°",
            FilterParam::Brightness | FilterParam::Contrast | FilterParam::Saturation => "",
        }
    }

    /// Values a key may hold (the filter itself then brings the value into its own range).
    pub fn range(self) -> std::ops::RangeInclusive<f32> {
        match self {
            FilterParam::BlurX | FilterParam::BlurY => 0.0..=MAX_BLUR,
            FilterParam::Strength => 0.0..=1000.0,
            FilterParam::Angle => -100_000.0..=100_000.0,
            FilterParam::Distance => -1000.0..=1000.0,
            FilterParam::Brightness | FilterParam::Contrast | FilterParam::Saturation => -100.0..=100.0,
            FilterParam::Hue => -180.0..=180.0,
        }
    }

    fn index(self) -> usize {
        FilterParam::ALL.iter().position(|p| *p == self).unwrap_or(0)
    }
}

/// How many [`FilterParam`]s there are.
pub const FILTER_PARAMS: usize = 9;

impl Filter {
    /// The numbers of this filter a motion tween can animate, in the Filters panel's order.
    pub fn params(&self) -> &'static [FilterParam] {
        use FilterParam::*;
        match self {
            Filter::Blur { .. } => &[BlurX, BlurY],
            Filter::Glow { .. } => &[BlurX, BlurY, Strength],
            Filter::DropShadow { .. } | Filter::Bevel { .. } | Filter::GradientGlow { .. } | Filter::GradientBevel { .. } => &[BlurX, BlurY, Strength, Angle, Distance],
            Filter::AdjustColor { .. } => &[Brightness, Contrast, Saturation, Hue],
        }
    }

    /// The filter's value of `p` in the Motion Editor's units; `None` when it has no such number.
    pub fn param(&self, p: FilterParam) -> Option<f32> {
        if !self.params().contains(&p) {
            return None;
        }
        let mut out = None;
        self.visit(p, |v, scale| out = Some(*v * scale));
        out
    }

    /// Sets `p` to `v` (Motion Editor units, brought into the filter's range when drawn); false
    /// when the filter has no such number.
    pub fn set_param(&mut self, p: FilterParam, v: f32) -> bool {
        if !self.params().contains(&p) || !v.is_finite() {
            return false;
        }
        let mut done = false;
        self.visit_mut(p, |x, scale| {
            *x = v / scale;
            done = true;
        });
        done
    }

    /// The field holding `p` and the factor to the Motion Editor's units (strength in percent).
    fn visit(&self, p: FilterParam, f: impl FnOnce(&f32, f32)) {
        let mut c = self.clone();
        c.visit_mut(p, |x, s| f(x, s));
    }

    fn visit_mut(&mut self, p: FilterParam, f: impl FnOnce(&mut f32, f32)) {
        use FilterParam as P;
        let field: Option<(&mut f32, f32)> = match (self, p) {
            (Filter::Blur { blur_x, .. }, P::BlurX) => Some((blur_x, 1.0)),
            (Filter::Blur { blur_y, .. }, P::BlurY) => Some((blur_y, 1.0)),
            (Filter::DropShadow { blur_x, .. } | Filter::Glow { blur_x, .. } | Filter::Bevel { blur_x, .. } | Filter::GradientGlow { blur_x, .. } | Filter::GradientBevel { blur_x, .. }, P::BlurX) => Some((blur_x, 1.0)),
            (Filter::DropShadow { blur_y, .. } | Filter::Glow { blur_y, .. } | Filter::Bevel { blur_y, .. } | Filter::GradientGlow { blur_y, .. } | Filter::GradientBevel { blur_y, .. }, P::BlurY) => Some((blur_y, 1.0)),
            (Filter::DropShadow { strength, .. } | Filter::Glow { strength, .. } | Filter::Bevel { strength, .. } | Filter::GradientGlow { strength, .. } | Filter::GradientBevel { strength, .. }, P::Strength) => Some((strength, 100.0)),
            (Filter::DropShadow { angle, .. } | Filter::Bevel { angle, .. } | Filter::GradientGlow { angle, .. } | Filter::GradientBevel { angle, .. }, P::Angle) => Some((angle, 1.0)),
            (Filter::DropShadow { distance, .. } | Filter::Bevel { distance, .. } | Filter::GradientGlow { distance, .. } | Filter::GradientBevel { distance, .. }, P::Distance) => Some((distance, 1.0)),
            (Filter::AdjustColor { brightness, .. }, P::Brightness) => Some((brightness, 1.0)),
            (Filter::AdjustColor { contrast, .. }, P::Contrast) => Some((contrast, 1.0)),
            (Filter::AdjustColor { saturation, .. }, P::Saturation) => Some((saturation, 1.0)),
            (Filter::AdjustColor { hue, .. }, P::Hue) => Some((hue, 1.0)),
            _ => None,
        };
        if let Some((x, s)) = field {
            f(x, s);
        }
    }
}

/// A filter's number addressed in an instance's list (ADR 0092): the filter's place (0 is the
/// first, applied first) and the number.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FilterProp {
    pub slot: u8,
    pub param: FilterParam,
}

/// Most filter slots a motion tween addresses (an instance holds at most 16 filters).
pub const MAX_FILTER_SLOTS: u8 = 16;

impl FilterProp {
    /// Its place among the filter properties: by slot, then in [`FilterParam::ALL`] order.
    pub fn index(self) -> usize {
        usize::from(self.slot) * FILTER_PARAMS + self.param.index()
    }
}

fn yes() -> bool {
    true
}
fn is_on(b: &bool) -> bool {
    *b
}

/// One filter in an instance's list (ADR 0091): the filter and whether it is on. A filter
/// switched off keeps its settings and its place but draws nothing: [`active`] leaves it out,
/// so every renderer and export sees exactly the list without it. In files it is the filter's
/// own object with `"enabled": false` added (left out when on), so files from before open as
/// they were.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FilterEntry {
    #[serde(flatten)]
    pub filter: Filter,
    #[serde(default = "yes", skip_serializing_if = "is_on")]
    pub enabled: bool,
}

impl From<Filter> for FilterEntry {
    fn from(filter: Filter) -> FilterEntry {
        FilterEntry { filter, enabled: true }
    }
}

impl FilterEntry {
    /// The same entry with its filter in range.
    pub fn sanitized(&self) -> FilterEntry {
        FilterEntry { filter: self.filter.sanitized(), enabled: self.enabled }
    }
}

/// The filters that draw, in order and in range: the ones switched on.
pub fn active(list: &[FilterEntry]) -> Vec<Filter> {
    list.iter().filter(|e| e.enabled).map(|e| e.filter.sanitized()).collect()
}

/// Whether any filter in the list draws.
pub fn any_active(list: &[FilterEntry]) -> bool {
    list.iter().any(|e| e.enabled)
}

/// Whether two lists pair up for a tween: the same kinds in the same order (Animate's rule;
/// switched-off filters count, so switching one off never breaks a tween).
pub fn pairs_up(a: &[FilterEntry], b: &[FilterEntry]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| std::mem::discriminant(&x.filter) == std::mem::discriminant(&y.filter))
}

/// The classic tween between two filter lists (ADR 0081, 0091): when both have the same kinds
/// in the same order, each filter's numbers and colours travel (switches such as Inner and
/// Knockout, the quality, and whether the filter is on stay the first keyframe's; ramps travel
/// stop by stop when they have as many stops). `None` when the lists are equal or don't pair
/// up: the first list holds.
pub fn lerp_filters(a: &[FilterEntry], b: &[FilterEntry], t: f32) -> Option<Vec<FilterEntry>> {
    if a == b || !pairs_up(a, b) {
        return None;
    }
    a.iter().zip(b).map(|(x, y)| Some(FilterEntry { filter: x.filter.lerp(&y.filter, t)?, enabled: x.enabled })).collect()
}

fn ramp_lerp(a: &[GradientStop], b: &[GradientStop], t: f32) -> Vec<GradientStop> {
    if a.len() != b.len() {
        return a.to_vec();
    }
    a.iter().zip(b).map(|(x, y)| GradientStop { offset: anicraft_geom::lerp(x.offset, y.offset, t), color: x.color.lerp(y.color, t) }).collect()
}

impl Filter {
    /// This filter part way to `o` (see [`lerp_filters`]); `None` when `o` is another kind.
    pub fn lerp(&self, o: &Filter, t: f32) -> Option<Filter> {
        use anicraft_geom::lerp as l;
        Some(match (self, o) {
            (Filter::Blur { blur_x, blur_y, quality }, Filter::Blur { blur_x: bx, blur_y: by, .. }) => Filter::Blur { blur_x: l(*blur_x, *bx, t), blur_y: l(*blur_y, *by, t), quality: *quality },
            (Filter::DropShadow { blur_x, blur_y, strength, quality, angle, distance, color, inner, knockout, hide_object }, Filter::DropShadow { blur_x: bx, blur_y: by, strength: s, angle: an, distance: d, color: c, .. }) => {
                Filter::DropShadow { blur_x: l(*blur_x, *bx, t), blur_y: l(*blur_y, *by, t), strength: l(*strength, *s, t), quality: *quality, angle: l(*angle, *an, t), distance: l(*distance, *d, t), color: color.lerp(*c, t), inner: *inner, knockout: *knockout, hide_object: *hide_object }
            }
            (Filter::Glow { blur_x, blur_y, strength, quality, color, inner, knockout }, Filter::Glow { blur_x: bx, blur_y: by, strength: s, color: c, .. }) => {
                Filter::Glow { blur_x: l(*blur_x, *bx, t), blur_y: l(*blur_y, *by, t), strength: l(*strength, *s, t), quality: *quality, color: color.lerp(*c, t), inner: *inner, knockout: *knockout }
            }
            (Filter::Bevel { blur_x, blur_y, strength, quality, angle, distance, shadow_color, highlight_color, side, knockout }, Filter::Bevel { blur_x: bx, blur_y: by, strength: s, angle: an, distance: d, shadow_color: sc, highlight_color: hc, .. }) => Filter::Bevel {
                blur_x: l(*blur_x, *bx, t),
                blur_y: l(*blur_y, *by, t),
                strength: l(*strength, *s, t),
                quality: *quality,
                angle: l(*angle, *an, t),
                distance: l(*distance, *d, t),
                shadow_color: shadow_color.lerp(*sc, t),
                highlight_color: highlight_color.lerp(*hc, t),
                side: *side,
                knockout: *knockout,
            },
            (Filter::GradientGlow { blur_x, blur_y, strength, quality, angle, distance, stops, side, knockout }, Filter::GradientGlow { blur_x: bx, blur_y: by, strength: s, angle: an, distance: d, stops: st, .. }) => {
                Filter::GradientGlow { blur_x: l(*blur_x, *bx, t), blur_y: l(*blur_y, *by, t), strength: l(*strength, *s, t), quality: *quality, angle: l(*angle, *an, t), distance: l(*distance, *d, t), stops: ramp_lerp(stops, st, t), side: *side, knockout: *knockout }
            }
            (Filter::GradientBevel { blur_x, blur_y, strength, quality, angle, distance, stops, side, knockout }, Filter::GradientBevel { blur_x: bx, blur_y: by, strength: s, angle: an, distance: d, stops: st, .. }) => {
                Filter::GradientBevel { blur_x: l(*blur_x, *bx, t), blur_y: l(*blur_y, *by, t), strength: l(*strength, *s, t), quality: *quality, angle: l(*angle, *an, t), distance: l(*distance, *d, t), stops: ramp_lerp(stops, st, t), side: *side, knockout: *knockout }
            }
            (Filter::AdjustColor { brightness, contrast, saturation, hue }, Filter::AdjustColor { brightness: b, contrast: c, saturation: s, hue: h }) => Filter::AdjustColor { brightness: l(*brightness, *b, t), contrast: l(*contrast, *c, t), saturation: l(*saturation, *s, t), hue: l(*hue, *h, t) },
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_default_like_animate_and_stay_in_range() {
        assert_eq!(Filter::new("blur"), Some(Filter::Blur { blur_x: 5.0, blur_y: 5.0, quality: 1 }));
        let Some(Filter::DropShadow { angle, distance, color, strength, .. }) = Filter::new("drop_shadow") else { panic!() };
        assert_eq!((angle, distance, color, strength), (45.0, 4.0, Rgba::BLACK, 1.0));
        assert!(Filter::new("sparkle").is_none());
        let wild = Filter::Blur { blur_x: f32::NAN, blur_y: 1e9, quality: 9 }.sanitized();
        assert_eq!(wild, Filter::Blur { blur_x: 0.0, blur_y: MAX_BLUR, quality: 3 });
        assert_eq!(Filter::Glow { blur_x: 10.0, blur_y: 4.0, strength: 1.0, quality: 2, color: Rgba::WHITE, inner: false, knockout: false }.margin(), 11.0);
        let json = serde_json::to_string(&Filter::new("adjust_color").unwrap()).unwrap();
        assert_eq!(json, r#"{"type":"adjust_color","brightness":0.0,"contrast":0.0,"saturation":0.0,"hue":0.0}"#);
    }

    #[test]
    fn bevels_and_gradient_filters_default_like_animate_and_old_files_open() {
        let Some(Filter::Bevel { distance, side, shadow_color, highlight_color, knockout, .. }) = Filter::new("bevel") else { panic!() };
        assert_eq!((distance, side, shadow_color, highlight_color, knockout), (5.0, FilterSide::Inner, Rgba::BLACK, Rgba::WHITE, false));
        assert!(matches!(Filter::new("gradient_glow"), Some(Filter::GradientGlow { side: FilterSide::Outer, .. })));
        // Every kind round-trips, and a bare type opens with the defaults.
        for (kind, label) in Filter::KINDS {
            let f = Filter::new(kind).unwrap();
            assert_eq!(f.label(), label);
            assert_eq!(f.sanitized(), f, "{kind}: defaults are already in range");
            let back: Filter = serde_json::from_str(&serde_json::to_string(&f).unwrap()).unwrap();
            assert_eq!(back, f);
        }
        for kind in ["bevel", "gradient_glow", "gradient_bevel"] {
            let bare: Filter = serde_json::from_str(&format!(r#"{{"type":"{kind}"}}"#)).unwrap();
            assert_eq!(Some(bare), Filter::new(kind), "{kind}: missing values take the defaults");
        }
        let side: FilterSide = serde_json::from_str(r#""full""#).unwrap();
        assert_eq!(side, FilterSide::Full);
        // Every side reaches out by blur and distance (inner ones read that far).
        let bevel = |side| Filter::Bevel { blur_x: 4.0, blur_y: 8.0, strength: 1.0, quality: 2, angle: 45.0, distance: -3.0, shadow_color: Rgba::BLACK, highlight_color: Rgba::WHITE, side, knockout: false };
        assert_eq!(bevel(FilterSide::Inner).margin(), 12.0);
        assert_eq!(bevel(FilterSide::Full).margin(), 12.0);
        let Some(Filter::GradientGlow { blur_x, blur_y, strength, quality, angle, distance, stops, knockout, .. }) = Filter::new("gradient_glow") else { panic!() };
        assert_eq!(Filter::GradientGlow { blur_x, blur_y, strength, quality, angle, distance, stops, knockout, side: FilterSide::Inner }.margin(), 8.5);
    }

    #[test]
    fn ramps_keep_their_rest_stop_transparent_and_in_range() {
        let st = |offset: f32, c: u32, a: f32| GradientStop { offset, color: Rgba::hex(c).with_alpha(a) };
        // A glow ramp with an opaque start gets it pinned transparent; order and range fixed.
        let Filter::GradientGlow { stops, .. } = (Filter::GradientGlow { blur_x: 5.0, blur_y: 5.0, strength: 1.0, quality: 1, angle: 45.0, distance: 5.0, stops: vec![st(2.0, 0xff0000, 1.0), st(0.0, 0x00ff00, 1.0), st(f32::NAN, 0, 1.0)], side: FilterSide::Outer, knockout: false }).sanitized() else { panic!() };
        assert_eq!(stops, vec![st(0.0, 0x00ff00, 0.0), st(1.0, 0xff0000, 1.0)]);
        // A bevel ramp without a middle gets one, of the ramp's colour there.
        let Filter::GradientBevel { stops, .. } = (Filter::GradientBevel { blur_x: 5.0, blur_y: 5.0, strength: 1.0, quality: 1, angle: 45.0, distance: 5.0, stops: vec![st(0.0, 0xffffff, 1.0), st(1.0, 0x000000, 1.0)], side: FilterSide::Inner, knockout: false }).sanitized() else { panic!() };
        assert_eq!(stops.len(), 3);
        assert_eq!(stops[1].offset, 0.5);
        assert_eq!(stops[1].color.a, 0.0);
        assert!((stops[1].color.r - 0.5).abs() < 1e-6);
        // Empty ramps take the default; long ones stay within the limit, the pin included.
        let Filter::GradientBevel { stops, .. } = (Filter::GradientBevel { blur_x: 5.0, blur_y: 5.0, strength: 1.0, quality: 1, angle: 45.0, distance: 5.0, stops: vec![], side: FilterSide::Inner, knockout: false }).sanitized() else { panic!() };
        assert_eq!(stops, default_bevel_ramp());
        let many: Vec<GradientStop> = (0..40).map(|i| st(i as f32 / 41.0, 0x336699, 1.0)).collect();
        let Filter::GradientBevel { stops, .. } = (Filter::GradientBevel { blur_x: 5.0, blur_y: 5.0, strength: 1.0, quality: 1, angle: 45.0, distance: 5.0, stops: many, side: FilterSide::Inner, knockout: false }).sanitized() else { panic!() };
        assert_eq!(stops.len(), MAX_RAMP_STOPS);
        assert!(stops.iter().any(|s| s.offset == 0.5 && s.color.a == 0.0));
        assert!(stops.windows(2).all(|w| w[0].offset <= w[1].offset));
        assert_eq!(ramp_at(&default_bevel_ramp(), 0.25), Rgba::new(0.5, 0.5, 0.5, 0.5));
        // Stops crowding the rest point: the nearest is pinned, the others step aside, in
        // order, and sanitizing again changes nothing.
        let crowd = vec![st(0.0, 0xffffff, 1.0), st(0.4995, 0xff0000, 1.0), st(0.4998, 0x00ff00, 1.0), st(0.5, 0x0000ff, 1.0), st(1.0, 0, 1.0)];
        let f = Filter::GradientBevel { blur_x: 5.0, blur_y: 5.0, strength: 1.0, quality: 1, angle: 45.0, distance: 5.0, stops: crowd, side: FilterSide::Inner, knockout: false }.sanitized();
        let Filter::GradientBevel { stops, .. } = &f else { panic!() };
        assert_eq!(stops.iter().filter(|s| (s.offset - 0.5).abs() < 1e-4).count(), 1, "{stops:?}");
        assert!(stops.iter().any(|s| s.offset == 0.5 && s.color == Rgba::hex(0x0000ff).with_alpha(0.0)));
        assert!(stops.windows(2).all(|w| w[0].offset <= w[1].offset));
        assert_eq!(f.sanitized(), f);
        assert_eq!(ramp_at(&[], 0.5), Rgba::TRANSPARENT);
    }
}
