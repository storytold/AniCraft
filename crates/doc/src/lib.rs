//! AniCraft document model (layer L1).
//!
//! A document is plain data. Time is a function: [`evaluate`] turns a document and a frame into
//! a flat [`DisplayList`], which renderers draw. Nothing here mutates during playback.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub use anicraft_audio as audio;
pub use anicraft_actions as actions;

pub mod armature;
mod bitmap;
mod color_effect;
pub mod camera;
mod filter;
mod easing;
mod eval;
pub mod field;
mod hints;
pub mod nine_slice;
pub mod library;
pub mod motion;
pub mod path_table;
pub mod width;
pub mod stroke_style;
pub mod player;
pub mod publish;
pub mod sample;
pub mod templates;
pub mod sound;

pub use filter::{Filter, FilterSide, MAX_BLUR, MAX_QUALITY, MAX_RAMP_STOPS, REST_SNAP, default_bevel_ramp, default_glow_ramp, pinned_stop, ramp_at};
pub use bitmap::{Bitmap, BitmapId, ImageError, MAX_BITMAP_EDGE, Pixels, base64_decode, base64_encode, decode_png, image_to_png};
pub use easing::Easing;
pub use color_effect::{ADVANCED_ADD, ADVANCED_MUL, ColorEffect, ColorStyle, ColorTransform};
pub use hints::{HINT_TOLERANCE, HintSide, MAX_SHAPE_HINTS, ShapeHint, hint_index, hint_letter, nearest_outline, placed_on, shapes_centre};
pub use library::{FolderId, LibraryEntry, LibraryFolder, LibraryItem, LibraryTree, NameError};
pub use motion::{MotionTween, OnPath, PROPERTIES, Pin, PinError, Property, PropertyKey, Props, Track, motion_path};
pub use sound::{EnvelopePoint, FrameSound, Sound, SoundEffect, SoundFormat, SoundId, SoundSync};
pub use width::{MAX_WIDTH_FACTOR, MAX_WIDTH_POINTS, ProfilePreset, WidthPoint, WidthProfile};
pub use stroke_style::{Cap, Dash, DashKind, Join, StrokeStyle};
pub use camera::Camera;
pub use nine_slice::symbol_box;
pub use armature::{Armature, Bone, BoneId, JointAngle, Limits, Pose};
pub use eval::{DisplayItem, DisplayList, Group, MAX_SYMBOL_DEPTH, evaluate, evaluate_guided, evaluate_layer_elements, evaluate_player, evaluate_scene, evaluate_scene_camera, evaluate_timeline, evaluate_timeline_at, guide_walk, instance_start};
pub use actions::{Action, ButtonEvent, EventHandler, FrameScript, Language, PluginScript, SourceScript};

use anicraft_geom::{Affine, FillRule, Path, Rect, Vec2};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Straight-alpha sRGB colour, components in `0..=1`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    pub const BLACK: Rgba = Rgba::new(0.0, 0.0, 0.0, 1.0);
    pub const WHITE: Rgba = Rgba::new(1.0, 1.0, 1.0, 1.0);
    pub const TRANSPARENT: Rgba = Rgba::new(0.0, 0.0, 0.0, 0.0);

    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Rgba { r, g, b, a }
    }
    /// From `0xRRGGBB`.
    pub fn hex(rgb: u32) -> Self {
        Rgba::new(((rgb >> 16) & 0xff) as f32 / 255.0, ((rgb >> 8) & 0xff) as f32 / 255.0, (rgb & 0xff) as f32 / 255.0, 1.0)
    }
    pub fn with_alpha(self, a: f32) -> Self {
        Rgba { a, ..self }
    }
    pub fn lerp(self, o: Rgba, t: f32) -> Rgba {
        use anicraft_geom::lerp;
        Rgba::new(lerp(self.r, o.r, t), lerp(self.g, o.g, t), lerp(self.b, o.b, t), lerp(self.a, o.a, t))
    }
    /// Clamped to `0..=1`, NaN mapped to 0.
    pub fn sanitized(self) -> Rgba {
        let c = |v: f32| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
        Rgba::new(c(self.r), c(self.g), c(self.b), c(self.a))
    }
    pub fn to_hex(self) -> String {
        let s = self.sanitized();
        let b = |v: f32| (v * 255.0).round() as u8;
        format!("#{:02x}{:02x}{:02x}", b(s.r), b(s.g), b(s.b))
    }
    /// Parses `#rrggbb`, `rrggbb` or `#rrggbbaa`.
    pub fn parse_hex(s: &str) -> Option<Rgba> {
        let h = s.trim().trim_start_matches('#');
        if !h.is_ascii() || (h.len() != 6 && h.len() != 8) {
            return None;
        }
        let byte = |i: usize| h.get(i..i + 2).and_then(|x| u8::from_str_radix(x, 16).ok()).map(|v| v as f32 / 255.0);
        let a = if h.len() == 8 { byte(6)? } else { 1.0 };
        Some(Rgba::new(byte(0)?, byte(2)?, byte(4)?, a))
    }
}

/// A gradient colour stop.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GradientStop {
    pub offset: f32,
    pub color: Rgba,
}

/// How a region is painted. Gradient geometry is in the element's local space.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Paint {
    Solid(Rgba),
    Linear { start: Vec2, end: Vec2, stops: Vec<GradientStop> },
    Radial { center: Vec2, radius: f32, stops: Vec<GradientStop> },
    /// A bitmap, its pixel (x, y) at local `matrix · (x, y)`; `repeat` tiles it, otherwise it
    /// shows once (an imported image is a rectangle of exactly its size, with no matrix).
    Bitmap {
        bitmap: BitmapId,
        #[serde(default)]
        repeat: bool,
        /// Image pixels → shape space (identity: one pixel per unit, at the origin).
        #[serde(default, skip_serializing_if = "is_identity")]
        matrix: Affine,
    },
}

fn is_identity(m: &Affine) -> bool {
    *m == Affine::IDENTITY
}

impl Paint {
    /// A representative colour (first stop for gradients) for swatches and the properties panel.
    pub fn primary_color(&self) -> Rgba {
        match self {
            Paint::Solid(c) => *c,
            Paint::Linear { stops, .. } | Paint::Radial { stops, .. } => stops.first().map_or(Rgba::BLACK, |s| s.color),
            Paint::Bitmap { .. } => Rgba::hex(0x808080),
        }
    }

    /// The paint carried by `m` (its geometry moved from one space to another), so a shape in
    /// the new space shows it where it was: gradient ends and centre are mapped, a radial
    /// gradient's radius scales by the mean scale, a bitmap's matrix is composed.
    pub fn transformed(&self, m: &Affine) -> Paint {
        match self {
            Paint::Solid(_) => self.clone(),
            Paint::Linear { start, end, stops } => Paint::Linear { start: m.apply(*start), end: m.apply(*end), stops: stops.clone() },
            Paint::Radial { center, radius, stops } => Paint::Radial { center: m.apply(*center), radius: radius * m.mean_scale(), stops: stops.clone() },
            Paint::Bitmap { bitmap, repeat, matrix } => Paint::Bitmap { bitmap: *bitmap, repeat: *repeat, matrix: m.mul(matrix) },
        }
    }

    pub fn lerp(&self, o: &Paint, t: f32) -> Paint {
        let lerp_stops = |a: &[GradientStop], b: &[GradientStop]| -> Option<Vec<GradientStop>> {
            (a.len() == b.len()).then(|| {
                a.iter().zip(b).map(|(x, y)| GradientStop { offset: anicraft_geom::lerp(x.offset, y.offset, t), color: x.color.lerp(y.color, t) }).collect()
            })
        };
        match (self, o) {
            (Paint::Solid(a), Paint::Solid(b)) => Paint::Solid(a.lerp(*b, t)),
            (Paint::Linear { start: s0, end: e0, stops: a }, Paint::Linear { start: s1, end: e1, stops: b }) => match lerp_stops(a, b) {
                Some(stops) => Paint::Linear { start: s0.lerp(*s1, t), end: e0.lerp(*e1, t), stops },
                None => self.clone(),
            },
            (Paint::Radial { center: c0, radius: r0, stops: a }, Paint::Radial { center: c1, radius: r1, stops: b }) => match lerp_stops(a, b) {
                Some(stops) => Paint::Radial { center: c0.lerp(*c1, t), radius: anicraft_geom::lerp(*r0, *r1, t), stops },
                None => self.clone(),
            },
            _ => self.clone(),
        }
    }
}

/// A line: colour, width, an optional width profile (ADR 0018) and its style: caps, joins,
/// miter limit, dashes (ADR 0046).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(from = "StrokeFile")]
pub struct Stroke {
    pub color: Rgba,
    /// Nominal width; a profile varies it along the stroke.
    pub width: f32,
    /// How the width varies along each subpath (`None`: the same everywhere). Never uniform.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<WidthProfile>,
    /// Caps, joins and dashes. The default (round, solid) is written as nothing, so strokes
    /// without a style are stored as before.
    #[serde(default, skip_serializing_if = "StrokeStyle::is_default")]
    pub style: StrokeStyle,
}

/// What a file may hold for a stroke (a uniform profile is dropped on load, the style is
/// sanitized).
#[derive(Deserialize)]
struct StrokeFile {
    color: Rgba,
    width: f32,
    #[serde(default)]
    profile: Option<WidthProfile>,
    #[serde(default)]
    style: StrokeStyle,
}

impl From<StrokeFile> for Stroke {
    fn from(f: StrokeFile) -> Stroke {
        Stroke { color: f.color, width: f.width, profile: f.profile.filter(|p| !p.is_uniform()), style: f.style.sanitized() }
    }
}

impl Stroke {
    /// A solid stroke of one width with round caps and joins.
    pub fn new(color: Rgba, width: f32) -> Stroke {
        Stroke { color, width, profile: None, style: StrokeStyle::default() }
    }
    pub fn with_profile(self, profile: Option<WidthProfile>) -> Stroke {
        Stroke { profile: profile.filter(|p| !p.is_uniform()), ..self }
    }
    /// The same stroke with caps, joins and dashes from `style` (sanitized).
    pub fn with_style(self, style: StrokeStyle) -> Stroke {
        Stroke { style: style.sanitized(), ..self }
    }
    /// The farthest the stroke reaches from its path (local units): half its nominal width
    /// times the profile's widest factor, times what miter corners and square caps add.
    pub fn reach(&self) -> f32 {
        let w = if self.width.is_finite() { self.width.max(0.0) } else { 0.0 };
        w * 0.5 * self.profile.as_ref().map_or(1.0, WidthProfile::max_factor) * self.style.reach_factor()
    }
    /// The stroke of a shape scaled by `k` (mean scale): the width and the dashes scale.
    pub fn scaled(&self, k: f32) -> Stroke {
        Stroke { width: self.width * k, style: self.style.scaled(k), ..self.clone() }
    }
    /// The stroke `t` of the way to `o` (shape tweens): colour, width, profile and style blend.
    pub fn lerp(&self, o: &Stroke, t: f32) -> Stroke {
        Stroke {
            color: self.color.lerp(o.color, t),
            width: anicraft_geom::lerp(self.width, o.width, t),
            profile: WidthProfile::lerp(self.profile.as_ref(), o.profile.as_ref(), t),
            style: self.style.lerp(&o.style, t),
        }
    }
}

/// `Arc` equality that short-circuits on shared pointers. Unchanged subtrees of two document
/// versions share their `Arc`s, so comparing versions costs O(changed), not O(document).
pub fn arc_eq<T: PartialEq>(a: &Arc<T>, b: &Arc<T>) -> bool {
    Arc::ptr_eq(a, b) || **a == **b
}

/// [`arc_eq`] over slices.
pub fn arcs_eq<T: PartialEq>(a: &[Arc<T>], b: &[Arc<T>]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| arc_eq(x, y))
}

/// Vector shape content.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Shape {
    #[serde(with = "path_table")]
    pub path: Arc<Path>,
    #[serde(default)]
    pub fill: Option<Paint>,
    #[serde(default)]
    pub stroke: Option<Stroke>,
    #[serde(default)]
    pub fill_rule: FillRule,
    /// Part of its layer's merged drawing (ADR 0031): cut and joined by what is drawn over it.
    /// Otherwise a drawing object, which keeps to itself.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub merge: bool,
}

impl PartialEq for Shape {
    fn eq(&self, o: &Shape) -> bool {
        arc_eq(&self.path, &o.path) && self.fill == o.fill && self.stroke == o.stroke && self.fill_rule == o.fill_rule && self.merge == o.merge
    }
}

impl Shape {
    pub fn filled(path: Path, fill: Paint) -> Shape {
        Shape { path: Arc::new(path), fill: Some(fill), stroke: None, fill_rule: FillRule::NonZero, merge: false }
    }
    pub fn with_stroke(mut self, color: Rgba, width: f32) -> Shape {
        self.stroke = Some(Stroke::new(color, width));
        self
    }
}

/// How a graphic symbol instance picks its internal frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LoopMode {
    #[default]
    Loop,
    PlayOnce,
    SingleFrame,
}

impl LoopMode {
    /// Local frame for `offset` frames into the instance's span.
    pub fn local_frame(&self, offset: u32, first: u32, len: u32) -> u32 {
        let len = len.max(1);
        match self {
            LoopMode::Loop => first.saturating_add(offset) % len,
            LoopMode::PlayOnce => first.saturating_add(offset).min(len - 1),
            LoopMode::SingleFrame => first.min(len - 1),
        }
    }
}

/// Unique id of a symbol in the library.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct SymbolId(pub u64);

/// Unique id of an element instance within a document.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ElementId(pub u64);

/// Unique id of a layer within a document.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct LayerId(pub u64);

/// What an element draws.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ElementKind {
    Shape(Shape),
    Symbol {
        symbol: SymbolId,
        #[serde(default)]
        loop_mode: LoopMode,
        #[serde(default)]
        first_frame: u32,
        /// How the instance blends with what is below it (movie clips and buttons).
        #[serde(default, skip_serializing_if = "BlendMode::is_normal")]
        blend: BlendMode,
        /// Filters, applied in order (movie clips and buttons).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        filters: Vec<Filter>,
        /// What the instance does on pointer events in the player (buttons and movie clips;
        /// ADR 0024), one handler per event.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        events: Vec<EventHandler>,
    },
    /// A block of text (ADR 0012).
    Text(Arc<TextBlock>),
    /// The camera, on a camera layer (ADR 0027): the element's transform is the camera on the
    /// stage and its tint colours the view. It draws nothing itself.
    Camera,
}

/// How an instance's pixels combine with what is below it, as Animate's Blending menu. Every
/// mode except Normal composites the instance on its own first (as Layer does), then blends
/// the result.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BlendMode {
    #[default]
    Normal,
    Layer,
    Darken,
    Multiply,
    Lighten,
    Screen,
    Overlay,
    #[serde(rename = "hardlight")]
    HardLight,
    Add,
    Subtract,
    Difference,
    Invert,
    /// Shows what is below only where the instance is opaque.
    Alpha,
    /// Clears what is below where the instance is opaque.
    Erase,
}

impl BlendMode {
    pub const ALL: [BlendMode; 14] = [
        BlendMode::Normal,
        BlendMode::Layer,
        BlendMode::Darken,
        BlendMode::Multiply,
        BlendMode::Lighten,
        BlendMode::Screen,
        BlendMode::Overlay,
        BlendMode::HardLight,
        BlendMode::Add,
        BlendMode::Subtract,
        BlendMode::Difference,
        BlendMode::Invert,
        BlendMode::Alpha,
        BlendMode::Erase,
    ];

    pub fn is_normal(&self) -> bool {
        *self == BlendMode::Normal
    }

    /// The name used in files, commands and Animate documents (`hardlight`, `multiply`…).
    pub fn key(self) -> &'static str {
        match self {
            BlendMode::Normal => "normal",
            BlendMode::Layer => "layer",
            BlendMode::Darken => "darken",
            BlendMode::Multiply => "multiply",
            BlendMode::Lighten => "lighten",
            BlendMode::Screen => "screen",
            BlendMode::Overlay => "overlay",
            BlendMode::HardLight => "hardlight",
            BlendMode::Add => "add",
            BlendMode::Subtract => "subtract",
            BlendMode::Difference => "difference",
            BlendMode::Invert => "invert",
            BlendMode::Alpha => "alpha",
            BlendMode::Erase => "erase",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            BlendMode::HardLight => "Hard Light",
            m => {
                const L: [&str; 14] = ["Normal", "Layer", "Darken", "Multiply", "Lighten", "Screen", "Overlay", "Hard Light", "Add", "Subtract", "Difference", "Invert", "Alpha", "Erase"];
                L.get(BlendMode::ALL.iter().position(|x| *x == m).unwrap_or(0)).copied().unwrap_or("Normal")
            }
        }
    }

    pub fn from_key(s: &str) -> Option<BlendMode> {
        let s = s.trim().to_ascii_lowercase().replace([' ', '_', '-'], "");
        BlendMode::ALL.into_iter().find(|m| m.key() == s)
    }
}

/// Horizontal alignment of a text block's lines.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TextAlign {
    #[default]
    Left,
    Center,
    Right,
}

/// A block of text: its source and style, and the outlines the engine laid out from them
/// (ADR 0012). Rendering uses only the outlines, so the block looks the same on computers
/// without its font.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextBlock {
    pub text: String,
    /// Font family the outlines were laid out with.
    pub font: String,
    /// Font size in pixels.
    pub size: f32,
    #[serde(default)]
    pub bold: bool,
    #[serde(default)]
    pub italic: bool,
    pub color: Rgba,
    #[serde(default)]
    pub align: TextAlign,
    /// Extra space between characters and between lines, in pixels.
    #[serde(default)]
    pub letter_spacing: f32,
    #[serde(default)]
    pub line_spacing: f32,
    /// Lines wrap to this width; `None` keeps each paragraph on one line.
    #[serde(default)]
    pub width: Option<f32>,
    /// Glyph outlines, local coordinates, the box's top-left corner at the origin.
    #[serde(with = "path_table")]
    pub outline: Arc<Path>,
    /// The layout box.
    pub box_width: f32,
    pub box_height: f32,
    /// A dynamic or input text field (ADR 0066); `None`: static text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<TextField>,
}

impl TextBlock {
    /// What kind of text it is: static, dynamic or input.
    pub fn kind(&self) -> TextKind {
        self.field.as_ref().map_or(TextKind::Static, |f| f.kind)
    }
}

/// Animate's three text types (ADR 0066).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TextKind {
    /// Drawn as laid out; scripts can't reach it.
    #[default]
    Static,
    /// Scripts set and read its text (`field.text`) while the movie plays.
    Dynamic,
    /// The viewer types into it while the movie plays; scripts read what was typed.
    Input,
}

impl TextKind {
    pub const ALL: [TextKind; 3] = [TextKind::Static, TextKind::Dynamic, TextKind::Input];
    pub fn key(self) -> &'static str {
        match self {
            TextKind::Static => "static",
            TextKind::Dynamic => "dynamic",
            TextKind::Input => "input",
        }
    }
    pub fn from_key(s: &str) -> Option<TextKind> {
        TextKind::ALL.into_iter().find(|k| k.key() == s.trim().to_ascii_lowercase())
    }
}

/// A field's line type, as Animate's Properties name them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineType {
    /// One line: Enter types nothing, the text scrolls sideways to keep the caret in view.
    #[default]
    Single,
    /// Lines wrap at the field's width.
    Multiline,
    /// Lines break only where the text does.
    MultilineNoWrap,
    /// One line, drawn as `*` (input fields).
    Password,
}

impl LineType {
    pub const ALL: [LineType; 4] = [LineType::Single, LineType::Multiline, LineType::MultilineNoWrap, LineType::Password];
    pub fn key(self) -> &'static str {
        match self {
            LineType::Single => "single",
            LineType::Multiline => "multiline",
            LineType::MultilineNoWrap => "multiline_no_wrap",
            LineType::Password => "password",
        }
    }
    pub fn from_key(s: &str) -> Option<LineType> {
        let s = s.trim().to_ascii_lowercase().replace([' ', '-'], "_");
        LineType::ALL.into_iter().find(|k| k.key() == s)
    }
    /// More than one line (Enter starts a new one).
    pub fn multiline(self) -> bool {
        matches!(self, LineType::Multiline | LineType::MultilineNoWrap)
    }
}

/// The inset of text in a field's box, on every side, in pixels (as Animate's fields have).
pub const FIELD_GUTTER: f32 = 2.0;
/// Longest text a field holds (characters).
pub const MAX_FIELD_CHARS: usize = 16_384;
/// Longest instance or variable name (characters).
pub const MAX_FIELD_NAME: usize = 256;

/// A dynamic or input text field (ADR 0066): how it is drawn and how the viewer may use it. The
/// element's name is its instance name; the block's text is what it shows before a script or the
/// viewer changes it. Its box is the block's `box_width` × `box_height` (fixed, not the text's
/// size), and its outline is the shown text laid out [`FIELD_GUTTER`] inside the box, so
/// everything that draws or measures text works on fields unchanged.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextField {
    pub kind: TextKind,
    #[serde(default)]
    pub lines: LineType,
    /// The viewer can select its text (dynamic fields; input fields always can).
    #[serde(default = "yes")]
    pub selectable: bool,
    /// A line around the box, and a fill behind the text.
    #[serde(default)]
    pub border: bool,
    #[serde(default = "black")]
    pub border_color: Rgba,
    #[serde(default)]
    pub background: bool,
    #[serde(default = "white")]
    pub background_color: Rgba,
    /// Most characters the viewer may type (0: no limit).
    #[serde(default)]
    pub max_chars: u32,
    /// The ActionScript 1/2 variable the field shows (kept from Animate documents, not run).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub variable: String,
}

fn black() -> Rgba {
    Rgba::BLACK
}
fn white() -> Rgba {
    Rgba::WHITE
}

impl TextField {
    /// A single-line field of `kind`.
    pub fn new(kind: TextKind) -> TextField {
        TextField { kind, lines: LineType::Single, selectable: true, border: false, border_color: Rgba::BLACK, background: false, background_color: Rgba::WHITE, max_chars: 0, variable: String::new() }
    }

    /// The width text wraps at in a box `box_width` wide, if its lines wrap.
    pub fn wrap_width(&self, box_width: f32) -> Option<f32> {
        (self.lines == LineType::Multiline).then(|| (box_width - 2.0 * FIELD_GUTTER).max(1.0))
    }

    /// What a field shows for `text`: `*` for each character of a password.
    pub fn shown(&self, text: &str) -> String {
        if self.lines == LineType::Password && self.kind == TextKind::Input { text.chars().map(|_| '*').collect() } else { text.to_string() }
    }

    /// The viewer can focus it: an input field, or a selectable dynamic one.
    pub fn focusable(&self) -> bool {
        self.kind == TextKind::Input || (self.kind == TextKind::Dynamic && self.selectable)
    }

    /// Values in range: names cut to length, at most [`MAX_FIELD_CHARS`] typed.
    pub fn sanitized(&self) -> TextField {
        TextField { max_chars: self.max_chars.min(MAX_FIELD_CHARS as u32), variable: self.variable.chars().take(MAX_FIELD_NAME).collect(), ..self.clone() }
    }
}

/// An instance on a keyframe.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Element {
    pub id: ElementId,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub transform: Affine,
    #[serde(default)]
    pub color: ColorEffect,
    pub kind: ElementKind,
}

impl Element {
    /// Local-space bounds (control-point bounds for shapes, `None` for symbols).
    pub fn local_bounds(&self, doc: &Document) -> Rect {
        element_bounds(doc, self, &Affine::IDENTITY, 0)
    }
    /// Bounds in the parent's space.
    pub fn bounds(&self, doc: &Document) -> Rect {
        element_bounds(doc, self, &self.transform, 0)
    }
}

fn element_bounds(doc: &Document, el: &Element, xf: &Affine, depth: usize) -> Rect {
    if depth > MAX_SYMBOL_DEPTH {
        return Rect::EMPTY;
    }
    match &el.kind {
        ElementKind::Shape(s) => {
            let pad = s.stroke.as_ref().map_or(0.0, Stroke::reach);
            s.path.bounds().inflate(pad).transformed(xf)
        }
        ElementKind::Text(t) => Rect::new(0.0, 0.0, t.box_width.max(0.0), t.box_height.max(0.0)).transformed(xf),
        ElementKind::Camera => Rect::EMPTY,
        ElementKind::Symbol { symbol, .. } => {
            let Some(sym) = doc.symbol(*symbol) else { return Rect::EMPTY };
            let mut r = Rect::EMPTY;
            for layer in &sym.timeline.layers {
                if let Some(kf) = layer.keyframes.first() {
                    for child in &kf.elements {
                        let m = xf.mul(&child.transform);
                        r = r.union(&element_bounds(doc, child, &m, depth + 1));
                    }
                }
            }
            r
        }
    }
}

/// What a tween interpolates.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TweenKind {
    /// Transform, colour effect and same-structure shape edits (symbols, moved points).
    #[default]
    Classic,
    /// Also morphs between shapes of any structure (a square into a star).
    Shape,
}

/// A tween from a keyframe to the next one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Tween {
    #[serde(default)]
    pub easing: Easing,
    #[serde(default, skip_serializing_if = "is_classic")]
    pub kind: TweenKind,
    /// On a guided layer: turn with the motion guide as it bends.
    #[serde(default, skip_serializing_if = "is_false")]
    pub orient: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn is_classic(k: &TweenKind) -> bool {
    *k == TweenKind::Classic
}

/// A keyframe and the span of frames it covers.
///
/// Elements are `Arc`-shared: copying a keyframe (F6), taking an undo snapshot or handing the
/// document to the render thread shares them, and editing one element clones only that element
/// (`Arc::make_mut`), never its siblings.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Keyframe {
    /// First frame (0-based).
    pub start: u32,
    /// Number of frames covered, at least 1.
    pub duration: u32,
    #[serde(default)]
    pub elements: Vec<Arc<Element>>,
    #[serde(default)]
    pub tween: Option<Tween>,
    /// A motion tween over this span (ADR 0016). A span has a classic or shape tween (`tween`)
    /// or a motion tween, never both; commands keep it so, and a motion tween wins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motion: Option<MotionTween>,
    /// Shape hints for the shape tween this keyframe starts (ADR 0023), `a` first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hints: Vec<ShapeHint>,
    #[serde(default)]
    pub label: String,
    /// The sound that starts (or, for Stop, ends) on this keyframe (ADR 0020).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sound: Option<FrameSound>,
    /// What runs when the player enters this keyframe: frame actions or a plug-in frame
    /// script (ADR 0024). Rendering and exports ignore it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script: Option<FrameScript>,
    /// On an armature layer: the joint angles this keyframe poses the armature with (ADR 0043).
    #[serde(default, skip_serializing_if = "Pose::is_empty")]
    pub pose: Pose,
}

impl PartialEq for Keyframe {
    fn eq(&self, o: &Keyframe) -> bool {
        self.start == o.start && self.duration == o.duration && self.tween == o.tween && self.motion == o.motion && self.hints == o.hints && self.label == o.label && self.sound == o.sound && self.script == o.script && self.pose == o.pose && arcs_eq(&self.elements, &o.elements)
    }
}

impl Keyframe {
    pub fn new(start: u32, duration: u32) -> Self {
        Keyframe { start, duration: duration.max(1), elements: Vec::new(), tween: None, motion: None, hints: Vec::new(), label: String::new(), sound: None, script: None, pose: Pose::default() }
    }
    /// Mutable access to the elements whose ids satisfy `pick`, cloning only those.
    pub fn elements_mut_where(&mut self, mut pick: impl FnMut(ElementId) -> bool) -> Vec<&mut Element> {
        self.elements.iter_mut().filter(|e| pick(e.id)).map(Arc::make_mut).collect()
    }
    pub fn end(&self) -> u32 {
        self.start.saturating_add(self.duration.max(1))
    }
    pub fn covers(&self, frame: u32) -> bool {
        frame >= self.start && frame < self.end()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LayerKind {
    #[default]
    Normal,
    /// Authoring aid, never in output: a plain guide, or a motion guide whose first path the
    /// tweens on the layers it guides (its children) follow.
    Guide,
    /// Its fills are not drawn: they reveal the layers masked by it (see [`Layer::parent`]).
    Mask,
    /// Holds layers (its children name it as their [`Layer::parent`]); draws nothing itself.
    /// Hiding or locking a folder hides or locks everything in it.
    Folder,
    /// The camera (ADR 0027): at most one per scene timeline, always the top layer; its
    /// keyframes hold the camera, which views every other layer. Draws nothing itself.
    Camera,
    /// An armature (ADR 0043): symbol instances joined by bones ([`Layer::armature`]), posed
    /// on each keyframe ([`Keyframe::pose`]) and tweened between poses.
    Armature,
}

/// Deepest nesting of folders (and a mask inside them) that is kept.
pub const MAX_LAYER_DEPTH: usize = 16;

/// A timeline layer. Keyframes are `Arc`-shared (see [`Keyframe`]).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Layer {
    pub id: LayerId,
    pub name: String,
    #[serde(default)]
    pub kind: LayerKind,
    /// The folder this layer is in, or the mask layer it is masked by. A layer sits below its
    /// parent, inside the parent's unbroken run of descendants ([`Timeline::repair_masks`]
    /// keeps it so); any other link is dropped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<LayerId>,
    #[serde(default = "yes")]
    pub visible: bool,
    #[serde(default)]
    pub locked: bool,
    /// View as Outlines (ADR 0061): the stage draws the layer's artwork as hairlines in
    /// [`Layer::color`]. Never in the movie or an export.
    #[serde(default)]
    pub outline: bool,
    #[serde(default = "default_layer_color")]
    pub color: Rgba,
    /// Onion skin draws this layer's frames around the playhead (ADR 0061); off, only its
    /// playhead frame shows.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub onion: bool,
    /// Row height on the timeline as a multiple of a normal row (ADR 0061): 1, 2 or 3 (100,
    /// 200 or 300 %), as Animate's Layer Properties offer. Read through [`Layer::height`].
    #[serde(default = "one", skip_serializing_if = "is_one", rename = "height")]
    pub height_rows: u8,
    #[serde(default)]
    pub keyframes: Vec<Arc<Keyframe>>,
    /// Layer depth (ADR 0027), pixels behind the stage (negative: in front): the camera's pans
    /// move a deeper layer less (parallax). Not the nesting depth ([`Timeline::depth`]).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub depth: f32,
    /// Volume of the sounds on this layer (and in the symbols on it), `0..=1` (ADR 0039).
    #[serde(default = "full", skip_serializing_if = "is_full")]
    pub volume: f32,
    /// The layer's sounds are silent (in playback and every export).
    #[serde(default, skip_serializing_if = "is_false")]
    pub muted: bool,
    /// An armature layer's bones (ADR 0043); `None` on every other kind of layer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub armature: Option<Arc<Armature>>,
}

fn is_zero(v: &f32) -> bool {
    *v == 0.0
}
fn full() -> f32 {
    1.0
}
fn is_full(v: &f32) -> bool {
    *v == 1.0
}
fn is_true(v: &bool) -> bool {
    *v
}
fn one() -> u8 {
    1
}
fn is_one(v: &u8) -> bool {
    *v == 1
}

/// Tallest a layer's timeline row gets, in normal rows (300 %).
pub const MAX_LAYER_HEIGHT: u8 = 3;

impl PartialEq for Layer {
    fn eq(&self, o: &Layer) -> bool {
        self.id == o.id
            && self.name == o.name
            && self.kind == o.kind
            && self.parent == o.parent
            && self.visible == o.visible
            && self.locked == o.locked
            && self.outline == o.outline
            && self.color == o.color
            && self.onion == o.onion
            && self.height_rows == o.height_rows
            && self.depth.to_bits() == o.depth.to_bits()
            && self.volume.to_bits() == o.volume.to_bits()
            && self.muted == o.muted
            && match (&self.armature, &o.armature) {
                (Some(a), Some(b)) => arc_eq(a, b),
                (a, b) => a.is_none() && b.is_none(),
            }
            && arcs_eq(&self.keyframes, &o.keyframes)
    }
}

fn yes() -> bool {
    true
}
fn default_layer_color() -> Rgba {
    Rgba::hex(0x4fa3ff)
}

impl Layer {
    pub fn new(id: LayerId, name: impl Into<String>) -> Self {
        Layer {
            id,
            name: name.into(),
            kind: LayerKind::Normal,
            parent: None,
            visible: true,
            locked: false,
            outline: false,
            color: default_layer_color(),
            onion: true,
            height_rows: 1,
            keyframes: vec![Arc::new(Keyframe::new(0, 1))],
            depth: 0.0,
            volume: 1.0,
            muted: false,
            armature: None,
        }
    }
    /// The row height in normal rows, `1..=MAX_LAYER_HEIGHT` whatever a file says.
    pub fn height(&self) -> u8 {
        self.height_rows.clamp(1, MAX_LAYER_HEIGHT)
    }
    /// The gain this layer gives its sounds: its volume, or silence when muted.
    pub fn sound_gain(&self) -> f32 {
        if self.muted {
            0.0
        } else if self.volume.is_finite() {
            self.volume.clamp(0.0, 1.0)
        } else {
            1.0
        }
    }
    /// Whether a keyframe of this layer holds a sound.
    pub fn has_sounds(&self) -> bool {
        self.keyframes.iter().any(|k| k.sound.is_some())
    }
    /// Mutable keyframe `i` (copy-on-write: clones it only if it is shared).
    pub fn keyframe_mut(&mut self, i: usize) -> Option<&mut Keyframe> {
        self.keyframes.get_mut(i).map(Arc::make_mut)
    }
    /// Index of the keyframe whose span covers `frame`.
    pub fn keyframe_index_at(&self, frame: u32) -> Option<usize> {
        // keyframes are sorted by start: binary search on start, then check the span.
        let idx = self.keyframes.partition_point(|k| k.start <= frame);
        let i = idx.checked_sub(1)?;
        self.keyframes.get(i).filter(|k| k.covers(frame)).map(|_| i)
    }
    pub fn keyframe_at(&self, frame: u32) -> Option<&Keyframe> {
        self.keyframe_index_at(frame).and_then(|i| self.keyframes.get(i)).map(|k| &**k)
    }
    /// Index of the keyframe starting exactly at `frame`.
    pub fn keyframe_starting_at(&self, frame: u32) -> Option<usize> {
        self.keyframes.iter().position(|k| k.start == frame)
    }
    /// One past the last frame with content.
    pub fn end_frame(&self) -> u32 {
        self.keyframes.last().map_or(0, |k| k.end())
    }
    /// Restores invariants after edits: sorted, durations ≥ 1, no overlaps. Touches (and so
    /// un-shares) only keyframes whose duration actually changes.
    pub fn normalize(&mut self) {
        // A loaded motion tween is repaired, and it replaces a classic tween on the same span.
        for i in 0..self.keyframes.len() {
            let bad = self.keyframes.get(i).is_some_and(|k| k.motion.as_ref().is_some_and(|m| !m.is_sane() || k.tween.is_some()));
            if bad && let Some(k) = self.keyframe_mut(i) {
                k.tween = None;
                if let Some(m) = k.motion.as_mut() {
                    m.sanitize();
                }
            }
        }
        if !self.keyframes.is_sorted_by_key(|k| k.start) {
            self.keyframes.sort_by_key(|k| k.start);
        }
        self.keyframes.dedup_by_key(|k| k.start);
        let n = self.keyframes.len();
        for i in 0..n {
            let next_start = self.keyframes.get(i + 1).map(|k| k.start);
            let Some(k) = self.keyframes.get(i) else { continue };
            let mut want = k.duration.max(1);
            if let Some(ns) = next_start {
                want = want.min(ns.saturating_sub(k.start)).max(1);
            }
            if want != k.duration
                && let Some(k) = self.keyframe_mut(i)
            {
                k.duration = want;
            }
        }
    }
}

/// An ordered stack of layers. Index 0 is the top layer. Layers are `Arc`-shared.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Timeline {
    pub layers: Vec<Arc<Layer>>,
}

impl PartialEq for Timeline {
    fn eq(&self, o: &Timeline) -> bool {
        arcs_eq(&self.layers, &o.layers)
    }
}

impl Timeline {
    /// Number of frames (at least 1).
    pub fn frame_count(&self) -> u32 {
        self.layers.iter().map(|l| l.end_frame()).max().unwrap_or(1).max(1)
    }
    pub fn layer(&self, id: LayerId) -> Option<&Layer> {
        self.layers.iter().find(|l| l.id == id).map(|l| &**l)
    }
    /// Mutable layer (copy-on-write).
    pub fn layer_mut(&mut self, id: LayerId) -> Option<&mut Layer> {
        self.layers.iter_mut().find(|l| l.id == id).map(Arc::make_mut)
    }

    fn index_of(&self, id: LayerId) -> Option<usize> {
        self.layers.iter().position(|l| l.id == id)
    }

    /// Whether layer `ancestor` contains the layer at `index` (directly or through folders).
    pub fn is_inside(&self, index: usize, ancestor: LayerId) -> bool {
        let mut cur = self.layers.get(index).and_then(|l| l.parent);
        for _ in 0..=MAX_LAYER_DEPTH {
            let Some(p) = cur else { return false };
            if p == ancestor {
                return true;
            }
            cur = self.layer(p).and_then(|l| l.parent);
        }
        false
    }

    /// How many folders and masks contain the layer at `index` (its indent in the timeline).
    pub fn depth(&self, index: usize) -> usize {
        let mut cur = self.layers.get(index).and_then(|l| l.parent);
        let mut d = 0;
        while let Some(p) = cur {
            if d >= MAX_LAYER_DEPTH {
                break;
            }
            d += 1;
            cur = self.layer(p).and_then(|l| l.parent);
        }
        d
    }

    /// The parent a layer of `kind` gets when placed at `index`, from the layer directly above:
    /// under a folder it goes in the folder; under a mask (or a masked layer) a normal layer is
    /// masked, anything else sits beside the mask; otherwise it shares the above layer's parent.
    pub fn container_at(&self, index: usize, kind: LayerKind) -> Option<LayerId> {
        let above = self.layers.get(index.checked_sub(1)?)?;
        let slot = match above.kind {
            LayerKind::Folder | LayerKind::Mask | LayerKind::Guide => Some(above.id),
            _ => above.parent,
        };
        let p = self.layer(slot?)?;
        if matches!(p.kind, LayerKind::Mask | LayerKind::Guide) && kind != LayerKind::Normal {
            return p.parent;
        }
        let too_deep = self.index_of(p.id).is_none_or(|pi| self.depth(pi) + 1 >= MAX_LAYER_DEPTH);
        if too_deep { p.parent } else { Some(p.id) }
    }

    /// The mask a normal layer placed at `index` would be masked by.
    pub fn mask_above(&self, index: usize) -> Option<LayerId> {
        self.container_at(index, LayerKind::Normal).filter(|p| self.layer(*p).is_some_and(|l| l.kind == LayerKind::Mask))
    }

    /// The motion guide a normal layer placed at `index` would be guided by.
    pub fn guide_above(&self, index: usize) -> Option<LayerId> {
        self.container_at(index, LayerKind::Normal).filter(|p| self.layer(*p).is_some_and(|l| l.kind == LayerKind::Guide))
    }

    /// Indices `i..j` of the layer at `i` and everything inside it (a folder's layers, a mask's
    /// masked layers).
    pub fn group_range(&self, i: usize) -> std::ops::Range<usize> {
        let Some(l) = self.layers.get(i) else { return i..i };
        let mut j = i + 1;
        while j < self.layers.len() && self.is_inside(j, l.id) {
            j += 1;
        }
        i..j
    }

    /// Whether a folder containing the layer at `index` is hidden (so the layer is too).
    pub fn hidden_by_folder(&self, index: usize) -> bool {
        self.folders_of(index).any(|f| !f.visible)
    }

    /// Whether a folder containing layer `id` is locked (so the layer is too).
    pub fn locked_by_folder(&self, id: LayerId) -> bool {
        self.index_of(id).is_some_and(|i| self.folders_of(i).any(|f| f.locked))
    }

    /// The folders containing the layer at `index`, innermost first.
    fn folders_of(&self, index: usize) -> impl Iterator<Item = &Layer> {
        let mut cur = self.layers.get(index).and_then(|l| l.parent);
        let mut n = 0;
        std::iter::from_fn(move || {
            while let Some(p) = cur {
                n += 1;
                if n > MAX_LAYER_DEPTH {
                    return None;
                }
                let l = self.layer(p)?;
                cur = l.parent;
                if l.kind == LayerKind::Folder {
                    return Some(l);
                }
            }
            None
        })
    }

    /// Drops parent links that no longer hold. A layer in a folder sits below the folder inside
    /// its unbroken run; a masked layer is a normal layer directly under its mask or another
    /// layer masked by it; nesting stops at [`MAX_LAYER_DEPTH`]. Touches only layers that change.
    pub fn repair_masks(&mut self) {
        // The camera layer sits on top, outside any folder (ADR 0027).
        self.repair_camera();
        for i in 0..self.layers.len() {
            let Some(l) = self.layers.get(i) else { continue };
            let Some(p) = l.parent else { continue };
            // A broken link falls back to the nearest enclosing parent that still holds (a layer
            // whose mask stops being a mask stays in the folder the mask is in).
            let mut cand = Some(p);
            for _ in 0..=MAX_LAYER_DEPTH {
                let Some(c) = cand else { break };
                if self.link_holds(i, c) {
                    break;
                }
                cand = self.layer(c).and_then(|x| x.parent);
            }
            let cand = cand.filter(|c| self.link_holds(i, *c));
            if cand != Some(p)
                && let Some(l) = self.layers.get_mut(i)
            {
                Arc::make_mut(l).parent = cand;
            }
        }
    }

    /// Whether the layer at `i` may name `p` as its parent. (Layers above `i` are already
    /// repaired, so their chains are valid.)
    fn link_holds(&self, i: usize, p: LayerId) -> bool {
        let Some(l) = self.layers.get(i) else { return false };
        let above = i.checked_sub(1).and_then(|a| self.layers.get(a));
        match self.layer(p).filter(|_| self.index_of(p).is_some_and(|pi| pi < i)).map(|x| x.kind) {
            Some(LayerKind::Folder) => above.is_some_and(|a| a.id == p || self.is_inside(i - 1, p)) && self.depth_of_parent(p) + 1 < MAX_LAYER_DEPTH,
            Some(LayerKind::Mask | LayerKind::Guide) => l.kind == LayerKind::Normal && above.is_some_and(|a| a.id == p || a.parent == Some(p)),
            _ => false,
        }
    }

    /// The folder directly containing the layer at `index`, if any.
    pub fn folder_of(&self, index: usize) -> Option<LayerId> {
        self.folders_of(index).next().map(|f| f.id)
    }

    fn depth_of_parent(&self, p: LayerId) -> usize {
        self.index_of(p).map_or(0, |i| self.depth(i))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SymbolKind {
    #[default]
    Graphic,
    MovieClip,
    Button,
}

/// A reusable symbol in the library.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Symbol {
    // (PartialEq is fast: `Timeline` compares its layers by pointer first.)
    pub id: SymbolId,
    pub name: String,
    #[serde(default)]
    pub kind: SymbolKind,
    /// The class scripts know the symbol by (Animate's "linkage"), kept as text (ADR 0013).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linkage: Option<Linkage>,
    /// Guides for 9-slice scaling (ADR 0055): an instance scaled keeps the corners outside them
    /// unscaled. `None`: the symbol scales as a whole.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale_grid: Option<ScaleGrid>,
    /// The guides as they were when 9-slice scaling was switched off (ADR 0057): switching it on
    /// again brings them back where they were. Only kept, never drawn or applied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scale_grid_kept: Option<ScaleGrid>,
    pub timeline: Timeline,
}

/// The four 9-slice guides of a symbol (ADR 0055), in the symbol's own space: two vertical
/// lines at `x = left` and `x = right`, two horizontal ones at `y = top` and `y = bottom`.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScaleGrid {
    pub left: f32,
    pub right: f32,
    pub top: f32,
    pub bottom: f32,
}

impl ScaleGrid {
    /// Guides a quarter of the way in from each edge of `bounds` (what turning guides on gives).
    pub fn inset(bounds: &Rect) -> ScaleGrid {
        let (w, h) = (bounds.width() * 0.25, bounds.height() * 0.25);
        ScaleGrid { left: bounds.min.x + w, right: bounds.max.x - w, top: bounds.min.y + h, bottom: bounds.max.y - h }
    }

    /// Whether the guides are usable: finite, within the format's range, and in order.
    pub fn check(&self) -> Result<(), String> {
        let v = [self.left, self.right, self.top, self.bottom];
        if !v.iter().all(|x| x.is_finite() && x.abs() <= MAX_GRID_COORD) {
            return Err(format!("9-slice guides must be numbers within ±{MAX_GRID_COORD}"));
        }
        if self.left > self.right {
            return Err("the left 9-slice guide must not be right of the right one".into());
        }
        if self.top > self.bottom {
            return Err("the top 9-slice guide must not be below the bottom one".into());
        }
        Ok(())
    }
}

/// Largest 9-slice guide coordinate accepted from files and commands.
pub const MAX_GRID_COORD: f32 = 1_000_000.0;

/// A symbol's script class: what scripts create it by, and the class it extends. Kept with
/// the document and shown, never run (ADR 0013 tier 1).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Linkage {
    /// A dotted class name (`com.example.Hero`), or an older document's linkage identifier.
    pub class: String,
    /// The class it extends, when the file names one (`flash.display.MovieClip`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub base: String,
}

impl Linkage {
    pub fn check(&self) -> Result<(), String> {
        if !actions::valid_class_name(&self.class) {
            return Err(format!("“{}” isn't a class name: letters, digits, _ and $ joined by dots, at most {} characters", self.class, actions::MAX_CLASS_CHARS));
        }
        if !self.base.is_empty() && !actions::valid_class_name(&self.base) {
            return Err(format!("“{}” isn't a class name", self.base));
        }
        Ok(())
    }
}

/// Stage settings.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stage {
    pub width: u32,
    pub height: u32,
    pub fps: f32,
    pub background: Rgba,
}

impl Default for Stage {
    fn default() -> Self {
        Stage { width: 1280, height: 720, fps: 24.0, background: Rgba::WHITE }
    }
}

/// Largest stage edge accepted from files and commands.
pub const MAX_STAGE_EDGE: u32 = 16_384;

/// A scene: one part of the movie with its own main timeline. Scenes play in order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Scene {
    pub name: String,
    #[serde(default)]
    pub timeline: Timeline,
}

impl Scene {
    pub fn new(name: impl Into<String>) -> Scene {
        Scene { name: name.into(), timeline: Timeline::default() }
    }
}

/// Most scenes in a document.
pub const MAX_SCENES: usize = 1_000;

/// A whole AniCraft document: a persistent (structurally shared) tree.
///
/// `clone()` is O(scenes + layers + symbols) pointer copies, which makes undo snapshots and
/// render-thread hand-off cheap, and `==` costs O(changed) because shared subtrees compare by
/// pointer. There is always at least one scene ([`Document::sanitize`] keeps it so).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(from = "DocumentFile")]
pub struct Document {
    pub stage: Stage,
    pub scenes: Vec<Arc<Scene>>,
    pub library: Vec<Arc<Symbol>>,
    /// Images, shown through [`Paint::Bitmap`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bitmaps: Vec<Arc<Bitmap>>,
    /// Sounds in the library, placed on keyframes through [`FrameSound`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sounds: Vec<Arc<Sound>>,
    /// The Library's folders and which folder each item is in (ADR 0050).
    #[serde(default, skip_serializing_if = "library_tree_is_empty")]
    pub library_tree: Arc<LibraryTree>,
    /// The class scripts run the main timeline as (Animate's "document class"), kept as text
    /// (ADR 0013 tier 1); empty when there is none.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub document_class: String,
    /// File › Publish Settings: the formats Publish writes and their options (ADR 0060).
    #[serde(default, skip_serializing_if = "publish_is_default")]
    pub publish: Arc<publish::PublishProfile>,
    /// Next free id for layers, elements and symbols.
    pub next_id: u64,
}

/// What a document file may hold: `scenes`, or (files before scenes) a single `timeline`.
#[derive(Deserialize)]
struct DocumentFile {
    #[serde(default)]
    stage: Stage,
    #[serde(default)]
    scenes: Vec<Arc<Scene>>,
    #[serde(default)]
    timeline: Option<Timeline>,
    #[serde(default)]
    library: Vec<Arc<Symbol>>,
    #[serde(default)]
    bitmaps: Vec<Arc<Bitmap>>,
    #[serde(default)]
    sounds: Vec<Arc<Sound>>,
    #[serde(default)]
    library_tree: Arc<LibraryTree>,
    #[serde(default)]
    document_class: String,
    #[serde(default)]
    publish: Arc<publish::PublishProfile>,
    #[serde(default = "first_id")]
    next_id: u64,
}

impl From<DocumentFile> for Document {
    fn from(f: DocumentFile) -> Document {
        let mut scenes = f.scenes;
        if let Some(tl) = f.timeline {
            scenes.insert(0, Arc::new(Scene { name: "Scene 1".into(), timeline: tl }));
        }
        Document { stage: f.stage, scenes, library: f.library, bitmaps: f.bitmaps, sounds: f.sounds, library_tree: f.library_tree, document_class: f.document_class, publish: f.publish, next_id: f.next_id }
    }
}

fn first_id() -> u64 {
    1
}

fn library_tree_is_empty(t: &Arc<LibraryTree>) -> bool {
    t.is_empty()
}

fn publish_is_default(p: &Arc<publish::PublishProfile>) -> bool {
    p.is_default()
}

/// What `timeline(i)` returns for a scene that does not exist (never for a sanitized document).
static NO_TIMELINE: Timeline = Timeline { layers: Vec::new() };

impl PartialEq for Document {
    fn eq(&self, o: &Document) -> bool {
        self.stage == o.stage && self.next_id == o.next_id && self.document_class == o.document_class && arcs_eq(&self.scenes, &o.scenes) && arcs_eq(&self.library, &o.library) && arcs_eq(&self.bitmaps, &o.bitmaps) && arcs_eq(&self.sounds, &o.sounds) && arc_eq(&self.library_tree, &o.library_tree) && arc_eq(&self.publish, &o.publish)
    }
}

impl Default for Document {
    fn default() -> Self {
        let mut d = Document { stage: Stage::default(), scenes: vec![Arc::new(Scene::new("Scene 1"))], library: Vec::new(), bitmaps: Vec::new(), sounds: Vec::new(), library_tree: Arc::default(), document_class: String::new(), publish: Arc::default(), next_id: 1 };
        let id = LayerId(d.alloc_id());
        d.timeline_mut(0).layers.push(Arc::new(Layer::new(id, "Layer 1")));
        d
    }
}

impl Document {
    /// A fresh id, unique within this document.
    pub fn alloc_id(&mut self) -> u64 {
        let id = self.next_id.max(1);
        self.next_id = id.saturating_add(1);
        id
    }
    pub fn bitmap(&self, id: BitmapId) -> Option<&Bitmap> {
        self.bitmaps.iter().find(|b| b.id == id).map(|b| &**b)
    }
    pub fn sound(&self, id: SoundId) -> Option<&Sound> {
        self.sounds.iter().find(|s| s.id == id).map(|s| &**s)
    }
    pub fn symbol(&self, id: SymbolId) -> Option<&Symbol> {
        self.library.iter().find(|s| s.id == id).map(|s| &**s)
    }
    /// Scene `i`'s main timeline (an empty one when there is no such scene).
    pub fn timeline(&self, i: usize) -> &Timeline {
        self.scenes.get(i).map_or(&NO_TIMELINE, |s| &s.timeline)
    }
    /// Scene `i`'s main timeline, copy-on-write; the last scene when `i` is out of range (and a
    /// first scene is created if there is none).
    pub fn timeline_mut(&mut self, i: usize) -> &mut Timeline {
        if self.scenes.is_empty() {
            self.scenes.push(Arc::new(Scene::new("Scene 1")));
        }
        // In range: the list is non-empty and `i` is clamped to its last index.
        let i = i.min(self.scenes.len() - 1);
        &mut Arc::make_mut(&mut self.scenes[i]).timeline
    }
    /// The whole movie's length: every scene, one after the other.
    pub fn frame_count(&self) -> u32 {
        self.scenes.iter().map(|s| s.timeline.frame_count()).fold(0u32, u32::saturating_add).max(1)
    }
    /// Which scene a movie frame falls in, and the frame within that scene.
    pub fn locate(&self, movie_frame: u32) -> (usize, u32) {
        let mut f = movie_frame;
        for (i, s) in self.scenes.iter().enumerate() {
            let n = s.timeline.frame_count();
            if f < n {
                return (i, f);
            }
            f -= n;
        }
        let last = self.scenes.len().saturating_sub(1);
        (last, self.timeline(last).frame_count().saturating_sub(1))
    }
    /// The movie frame where scene `i` starts.
    pub fn scene_start(&self, i: usize) -> u32 {
        self.scenes.iter().take(i).map(|s| s.timeline.frame_count()).fold(0u32, u32::saturating_add)
    }
    /// Every timeline: the scenes', then the library symbols'.
    pub fn timelines(&self) -> impl Iterator<Item = &Timeline> {
        self.scenes.iter().map(|s| &s.timeline).chain(self.library.iter().map(|s| &s.timeline))
    }
    /// Repairs a loaded document: ids, sizes and keyframe order.
    pub fn sanitize(&mut self) {
        let s = &mut self.stage;
        s.width = s.width.clamp(1, MAX_STAGE_EDGE);
        s.height = s.height.clamp(1, MAX_STAGE_EDGE);
        if !s.fps.is_finite() || s.fps <= 0.0 {
            s.fps = 24.0;
        }
        s.fps = s.fps.clamp(1.0, 240.0);
        s.background = s.background.sanitized();
        let mut max_id = 0u64;
        let scan = |l: &Layer, max_id: &mut u64| {
            *max_id = (*max_id).max(l.id.0);
            for b in l.armature.iter().flat_map(|a| &a.bones) {
                *max_id = (*max_id).max(b.id.0);
            }
            for k in &l.keyframes {
                for e in &k.elements {
                    *max_id = (*max_id).max(e.id.0);
                }
            }
        };
        if self.scenes.is_empty() {
            self.scenes.push(Arc::new(Scene::new("Scene 1")));
        }
        self.scenes.truncate(MAX_SCENES);
        for (i, scene) in self.scenes.iter_mut().enumerate() {
            if scene.name.trim().is_empty() || scene.name.len() > 256 {
                Arc::make_mut(scene).name = format!("Scene {}", i + 1);
            }
            let tl = &mut Arc::make_mut(scene).timeline;
            for l in &mut tl.layers {
                Arc::make_mut(l).normalize();
                sanitize_scripts(l);
                sanitize_depth(l);
                sanitize_armature(l);
                scan(l, &mut max_id);
            }
            tl.repair_masks();
        }
        // Script class names are text nothing runs yet; ones that aren't names are dropped.
        if !self.document_class.is_empty() && !actions::valid_class_name(&self.document_class) {
            self.document_class.clear();
        }
        let mut publish = (*self.publish).clone();
        publish.sanitize();
        if publish != *self.publish {
            self.publish = Arc::new(publish);
        }
        for sym in &mut self.library {
            max_id = max_id.max(sym.id.0);
            if sym.linkage.as_ref().is_some_and(|l| l.check().is_err()) {
                Arc::make_mut(sym).linkage = None;
            }
            // 9-slice guides that aren't usable are dropped: the symbol scales as a whole.
            if sym.scale_grid.as_ref().is_some_and(|g| g.check().is_err()) {
                Arc::make_mut(sym).scale_grid = None;
            }
            // Kept guides (ADR 0057) only while 9-slice is off, and only usable ones.
            if sym.scale_grid_kept.as_ref().is_some_and(|g| sym.scale_grid.is_some() || g.check().is_err()) {
                Arc::make_mut(sym).scale_grid_kept = None;
            }
            let tl = &mut Arc::make_mut(sym).timeline;
            for l in &mut tl.layers {
                Arc::make_mut(l).normalize();
                sanitize_scripts(l);
                // Only a scene has a camera; a symbol's camera layer is kept as a normal layer.
                if l.kind == LayerKind::Camera {
                    Arc::make_mut(l).kind = LayerKind::Normal;
                }
                sanitize_depth(l);
                sanitize_armature(l);
                scan(l, &mut max_id);
            }
            tl.repair_masks();
        }
        for b in &self.bitmaps {
            max_id = max_id.max(b.id.0);
        }
        for s in &self.sounds {
            max_id = max_id.max(s.id.0);
        }
        self.sanitize_library_tree(&mut max_id);
        self.next_id = self.next_id.max(max_id.saturating_add(1));
    }
}

/// Keeps a loaded layer's depth finite and in range (touching it only if it isn't).
fn sanitize_depth(layer: &mut Arc<Layer>) {
    let d = camera::clamp_depth(layer.depth);
    if d.to_bits() != layer.depth.to_bits() {
        Arc::make_mut(layer).depth = d;
    }
}

/// Repairs a loaded layer's armature (ADR 0043): only an armature layer has one, its bones are
/// sane and ordered, and its keyframes' poses are finite. Touches only what needs it.
fn sanitize_armature(layer: &mut Arc<Layer>) {
    let is_armature = layer.kind == LayerKind::Armature;
    // (A pose on another layer's keyframe, pasted from an armature, is kept and plays no part.)
    if !is_armature {
        if layer.armature.is_some() {
            Arc::make_mut(layer).armature = None;
        }
        return;
    }
    if let Some(a) = &layer.armature {
        let mut fixed = (**a).clone();
        if fixed.sanitize() {
            Arc::make_mut(layer).armature = Some(Arc::new(fixed));
        }
    }
    let bad = |k: &Keyframe| {
        let mut p = k.pose.clone();
        p.sanitize()
    };
    if layer.keyframes.iter().any(|k| bad(k)) {
        for k in &mut Arc::make_mut(layer).keyframes {
            if bad(k) {
                Arc::make_mut(k).pose.sanitize();
            }
        }
    }
}

/// Repairs a loaded layer's frame scripts and instance event handlers (ADR 0024). Touches (and
/// so un-shares) only keyframes that need it.
fn sanitize_scripts(layer: &mut Arc<Layer>) {
    let bad_script = |k: &Keyframe| k.script.as_ref().is_some_and(FrameScript::needs_repair);
    let bad_events = |e: &Element| match &e.kind {
        ElementKind::Symbol { events, .. } => {
            events.len() > actions::MAX_HANDLERS || events.iter().enumerate().any(|(i, h)| events.iter().take(i).any(|o| o.event == h.event) || actions::check_actions(&h.actions).is_err())
        }
        _ => false,
    };
    if !layer.keyframes.iter().any(|k| bad_script(k) || k.elements.iter().any(|e| bad_events(e))) {
        return;
    }
    let l = Arc::make_mut(layer);
    for k in &mut l.keyframes {
        if !(bad_script(k) || k.elements.iter().any(|e| bad_events(e))) {
            continue;
        }
        let k = Arc::make_mut(k);
        if k.script.as_mut().is_some_and(|s| !s.sanitize()) {
            k.script = None;
        }
        for e in k.elements.iter_mut().filter(|e| bad_events(e)) {
            if let ElementKind::Symbol { events, .. } = &mut Arc::make_mut(e).kind {
                actions::sanitize_handlers(events);
            }
        }
    }
}

#[cfg(test)]
#[path = "camera_tests.rs"]
mod camera_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyframe_lookup() {
        let mut l = Layer::new(LayerId(1), "a");
        l.keyframes = [Keyframe::new(0, 5), Keyframe::new(5, 3), Keyframe::new(10, 2)].map(Arc::new).to_vec();
        assert_eq!(l.keyframe_index_at(0), Some(0));
        assert_eq!(l.keyframe_index_at(4), Some(0));
        assert_eq!(l.keyframe_index_at(7), Some(1));
        assert_eq!(l.keyframe_index_at(8), None); // gap
        assert_eq!(l.keyframe_index_at(11), Some(2));
        assert_eq!(l.keyframe_index_at(12), None);
        assert_eq!(l.end_frame(), 12);
    }

    #[test]
    fn normalize_removes_overlap() {
        let mut l = Layer::new(LayerId(1), "a");
        l.keyframes = [Keyframe::new(4, 10), Keyframe::new(0, 100)].map(Arc::new).to_vec();
        l.normalize();
        assert_eq!(l.keyframes[0].start, 0);
        assert_eq!(l.keyframes[0].duration, 4);
        assert_eq!(l.keyframes[1].duration, 10);
    }

    #[test]
    fn loop_modes() {
        assert_eq!(LoopMode::Loop.local_frame(7, 0, 5), 2);
        assert_eq!(LoopMode::PlayOnce.local_frame(7, 0, 5), 4);
        assert_eq!(LoopMode::SingleFrame.local_frame(7, 3, 5), 3);
        assert_eq!(LoopMode::Loop.local_frame(u32::MAX, u32::MAX, 0), 0);
    }

    #[test]
    fn hex_roundtrip() {
        let c = Rgba::parse_hex("#ff8000").unwrap();
        assert_eq!(c.to_hex(), "#ff8000");
        assert!(Rgba::parse_hex("zz").is_none());
        assert!(Rgba::parse_hex("ééé").is_none());
    }

    #[test]
    fn clones_share_structure_and_edits_copy_only_their_path() {
        let a = sample::demo();
        let mut b = a.clone();
        assert!(a.timeline(0).layers.iter().zip(&b.timeline(0).layers).all(|(x, y)| Arc::ptr_eq(x, y)));
        // Edit one element on the top layer.
        let layer = Arc::make_mut(&mut b.timeline_mut(0).layers[0]);
        let kf = layer.keyframe_mut(1).unwrap();
        Arc::make_mut(&mut kf.elements[0]).name = "edited".into();
        assert!(!Arc::ptr_eq(&a.timeline(0).layers[0], &b.timeline(0).layers[0]));
        // siblings stay shared
        assert!(Arc::ptr_eq(&a.timeline(0).layers[0].keyframes[0], &b.timeline(0).layers[0].keyframes[0]));
        assert!(a.timeline(0).layers[1..].iter().zip(&b.timeline(0).layers[1..]).all(|(x, y)| Arc::ptr_eq(x, y)));
        assert!(a.library.iter().zip(&b.library).all(|(x, y)| Arc::ptr_eq(x, y)));
        assert_ne!(a, b);
        // Equal content in fresh allocations still compares equal.
        let c: Document = serde_json::from_str(&serde_json::to_string(&a).unwrap()).unwrap();
        assert_eq!(a, c);
    }

    #[test]
    fn normalize_does_not_unshare_valid_keyframes() {
        let a = sample::demo();
        let mut b = a.clone();
        let layer = Arc::make_mut(&mut b.timeline_mut(0).layers[0]);
        layer.normalize();
        assert!(a.timeline(0).layers[0].keyframes.iter().zip(&layer.keyframes).all(|(x, y)| Arc::ptr_eq(x, y)));
    }

    #[test]
    fn sanitize_repairs_hostile_document() {
        let mut d = Document::default();
        d.stage.width = 0;
        d.stage.fps = f32::NAN;
        d.next_id = 0;
        d.sanitize();
        assert_eq!(d.stage.width, 1);
        assert_eq!(d.stage.fps, 24.0);
        assert!(d.next_id > d.timeline(0).layers[0].id.0);
    }
}
