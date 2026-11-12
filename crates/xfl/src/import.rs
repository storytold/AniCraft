//! XFL document → AniCraft document.
//!
//! What maps (first slice of M4): stage size, frame rate and colour; the first scene's
//! layers (name, colour, visibility, lock, outline; guide layers; folders flattened); frames
//! and keyframes with durations, labels and classic tweens with their ease; shapes (solid,
//! linear and radial fills, solid, dashed and dotted strokes with their caps, joints and
//! miter limit), groups (flattened), rectangle and oval
//! primitives; symbol instances with their matrix, colour effect (every style, ADR 0056), loop mode and first frame;
//! library symbols (graphic, movie clip, button) with their own timelines, nested to any
//! depth; bitmaps (library items, instances and bitmap fills) whose image is stored as PNG or
//! JPEG in the package; text fields, laid out with this computer's fonts (`text`, ADR 0014);
//! frame scripts, symbol linkage classes and the document class, kept verbatim (ADR 0013 tier
//! 1); frame scripts in the ActionScript 3.0 subset run in Test Movie (tier 2), and the report
//! lists which run and which are only kept; sounds whose file is in the package as WAV, Ogg
//! Vorbis or MP3, and the sounds on frames that use them, buttons' states included (ADR 0057).
//! Everything else (sounds and bitmaps kept only in Animate's own storage, sound effects,
//! motion tweens with property curves, unknown filters) is counted in the report instead of guessed at,
//! and so is whatever is drawn the nearest way AniCraft can (ragged, stipple and hatched
//! strokes, reflecting gradients, …). The self-authored corpus in `tests/corpus/` covers
//! every feature listed here, with golden renders (ADR 0035).

use crate::edges::{self, Edge};
use crate::xml::{self, Element as X};
use anicraft_doc::{
    Cap, Dash, Join, StrokeStyle, Bitmap, BitmapId, BlendMode, ColorEffect, ColorTransform, Filter, FilterSide, Document, Easing, Element, ElementId, ElementKind, GradientStop, Keyframe, Layer, LayerId, LayerKind, LoopMode, Paint, Rgba, Shape, Stage, Stroke, Symbol, SymbolId,
    Scene, ShapeHint, SymbolKind, Timeline, Tween, TweenKind, FrameScript, FrameSound, Language, Linkage, ScaleGrid, SoundSync, SourceScript,
};
use anicraft_geom::{Affine, FillRule, Path, Vec2};
use anicraft_text::{FaceMatch, Fonts};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

/// A shape hint as one keyframe of a file lists it (see `Importer::raw_hints`).
struct RawHint {
    order: usize,
    /// Its point on this keyframe, and the end point when the file gives both together.
    here: Vec2,
    end: Option<Vec2>,
}

#[derive(Debug, thiserror::Error)]
pub enum XflError {
    #[error("{0}")]
    Xml(#[from] xml::XmlError),
    #[error("{0}")]
    Zip(#[from] crate::zip::ZipError),
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("this isn't an Animate document: {0}")]
    NotXfl(String),
}

/// What the import couldn't bring across, by kind ("3 bitmaps"), the scripts it brought that
/// run and those it kept without running, and the fonts it couldn't find.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Report {
    pub skipped: BTreeMap<String, usize>,
    /// Frame scripts in the ActionScript 3.0 subset, which run in Test Movie and the HTML
    /// canvas export ("2 frame scripts in ActionScript 3.0"; ADR 0013 tier 2).
    pub runs: BTreeMap<String, usize>,
    /// Kept in the document but not run, by kind ("2 frame scripts in ActionScript 3.0", for
    /// those outside the subset; ADR 0013 tier 1).
    pub kept: BTreeMap<String, usize>,
    pub notes: Vec<String>,
    /// Font families the document's text asks for that aren't on this computer.
    pub missing_fonts: BTreeSet<String>,
    /// The family that text was laid out in instead.
    pub fallback_font: Option<String>,
}

impl Report {
    pub(crate) fn skip(&mut self, what: &str) {
        *self.skipped.entry(what.to_string()).or_default() += 1;
    }
    pub(crate) fn keep(&mut self, what: &str) {
        *self.kept.entry(what.to_string()).or_default() += 1;
    }
    pub(crate) fn run(&mut self, what: &str) {
        *self.runs.entry(what.to_string()).or_default() += 1;
    }
    /// One line for a status bar: "Imported. Runs in Test Movie: 2 frame scripts in
    /// ActionScript 3.0. Kept, not run: 1 frame scripts in ActionScript 3.0. Not yet supported:
    /// 2 sounds. Fonts not on this computer, drawn in Inter: Futura, Gotham."
    pub fn summary(&self) -> String {
        let list = |m: &BTreeMap<String, usize>| m.iter().map(|(k, n)| format!("{n} {k}")).collect::<Vec<String>>().join(", ");
        let mut kept = String::new();
        if !self.runs.is_empty() {
            kept.push_str(&format!(" Runs in Test Movie: {}.", list(&self.runs)));
        }
        if !self.kept.is_empty() {
            kept.push_str(&format!(" Kept, not run: {}.", list(&self.kept)));
        }
        if self.skipped.is_empty() && self.notes.is_empty() && self.missing_fonts.is_empty() {
            return format!("Imported everything in the file.{kept}");
        }
        let mut out = format!("Imported.{kept}");
        let mut parts: Vec<String> = self.skipped.iter().map(|(k, n)| format!("{n} {k}")).collect();
        parts.extend(self.notes.iter().cloned());
        if !parts.is_empty() {
            out.push_str(&format!(" Not yet supported: {}.", parts.join(", ")));
        }
        if !self.missing_fonts.is_empty() {
            let names: Vec<&str> = self.missing_fonts.iter().map(String::as_str).collect();
            let instead = self.fallback_font.as_deref().map(|f| format!(", drawn in {f}")).unwrap_or_default();
            out.push_str(&format!(" Fonts not on this computer{instead}: {}.", names.join(", ")));
        }
        out
    }
}

/// Reads files of the XFL package by their path inside it (`DOMDocument.xml`, `LIBRARY/x.xml`).
pub type Reader<'a> = dyn Fn(&str) -> Result<Option<Vec<u8>>, XflError> + 'a;

const MAX_SYMBOLS: usize = 20_000;
const MAX_LAYERS: usize = 5_000;
const MAX_FRAME: u32 = 100_000;
const MAX_BITMAPS: usize = 10_000;
const MAX_SOUNDS: usize = 10_000;

/// Bitmap fill matrices scale image pixels by twips (1/20 px): `a = 20` is one pixel per pixel.
const TWIPS: f32 = 20.0;

struct Builder<'a> {
    doc: Document,
    read: &'a Reader<'a>,
    /// Library item name → symbol id (allocated before timelines are built, so instances can
    /// refer to any symbol).
    ids: HashMap<String, SymbolId>,
    /// Parsed symbol files waiting to be built, with their script class.
    pending: Vec<(SymbolId, String, SymbolKind, Option<Linkage>, X)>,
    /// The language the document's frame scripts are written in (from its publish settings).
    language: Language,
    /// Library item name → bitmap id, for the bitmaps that could be read.
    bitmaps: HashMap<String, BitmapId>,
    /// Library item name → sound id, for the sounds whose file is in the package (ADR 0057).
    sounds: HashMap<String, anicraft_doc::SoundId>,
    report: Report,
    /// The fonts text is laid out with; the computer's, found on the first text field.
    fonts: Option<&'a Fonts>,
    /// Face name → what it is on this computer, looked up once per name.
    faces: HashMap<String, FaceMatch>,
}

/// A bevel or gradient filter's `type`: `inner`, `outer` or `full` (`default` when missing).
fn filter_side(el: &X, default: FilterSide) -> FilterSide {
    match el.attr("type").map(str::trim) {
        Some("inner") => FilterSide::Inner,
        Some("outer") => FilterSide::Outer,
        Some("full") => FilterSide::Full,
        _ => default,
    }
}

/// A gradient filter's ramp: its `GradientEntry` children (directly, or in a wrapper), with
/// ratios 0..1 (or Flash's 0..255).
fn ramp(el: &X) -> Vec<GradientStop> {
    let entries = el.children.iter().flat_map(|c| if c.name == "GradientEntry" { std::slice::from_ref(c) } else { c.children.as_slice() }).filter(|g| g.name == "GradientEntry");
    let raw: Vec<(f32, Rgba)> = entries.take(anicraft_doc::MAX_RAMP_STOPS).map(|g| (g.f32("ratio").filter(|r| r.is_finite()).unwrap_or(0.0), color(g, "color", "alpha"))).collect();
    // One scale for the whole ramp: any ratio past 1 means bytes.
    let scale = if raw.iter().any(|(r, _)| *r > 1.0) { 1.0 / 255.0 } else { 1.0 };
    let mut stops: Vec<GradientStop> = raw.into_iter().map(|(r, c)| GradientStop { offset: (r * scale).clamp(0.0, 1.0), color: c }).collect();
    stops.sort_by(|a, b| a.offset.total_cmp(&b.offset));
    stops
}

pub(crate) fn color(el: &X, key: &str, alpha_key: &str) -> Rgba {
    let rgb = el.attr(key).and_then(|c| u32::from_str_radix(c.trim().trim_start_matches('#'), 16).ok()).unwrap_or(0);
    Rgba::hex(rgb).with_alpha(el.f32(alpha_key).unwrap_or(1.0).clamp(0.0, 1.0))
}

fn matrix(el: &X) -> Affine {
    let Some(m) = el.child("matrix").and_then(|m| m.child("Matrix")) else { return Affine::IDENTITY };
    let a = Affine { m: [m.f32("a").unwrap_or(1.0), m.f32("b").unwrap_or(0.0), m.f32("c").unwrap_or(0.0), m.f32("d").unwrap_or(1.0), m.f32("tx").unwrap_or(0.0), m.f32("ty").unwrap_or(0.0)] };
    if a.is_finite() { a } else { Affine::IDENTITY }
}

/// Half the side of the square XFL gradients are defined on, in pixels (16384 twips / 20).
const GRADIENT_HALF: f32 = 819.2;

/// A fill definition (`SolidColor`, `LinearGradient`, `RadialGradient`, `BitmapFill`) as a
/// paint.
fn paint(def: &X, bitmaps: &HashMap<String, BitmapId>, report: &mut Report) -> Option<Paint> {
    match def.name.as_str() {
        "SolidColor" => Some(Paint::Solid(color(def, "color", "alpha"))),
        "LinearGradient" | "RadialGradient" => {
            let m = matrix(def);
            let stops: Vec<GradientStop> = def.children.iter().filter(|c| c.name == "GradientEntry").map(|g| GradientStop { offset: g.f32("ratio").unwrap_or(0.0).clamp(0.0, 1.0), color: color(g, "color", "alpha") }).collect();
            if stops.is_empty() {
                return None;
            }
            // What AniCraft's gradients can't draw yet is drawn the nearest way, and said.
            if def.attr("spreadMethod").is_some_and(|s| s.trim() != "pad") {
                report.skip("repeating and reflecting gradients (drawn padded)");
            }
            if def.attr("interpolationMethod").is_some_and(|s| s.trim() == "linearRGB") {
                report.skip("linear-RGB gradients (blended in sRGB)");
            }
            if def.name == "RadialGradient" {
                if def.f32("focalPointRatio").is_some_and(|f| f != 0.0) {
                    report.skip("radial gradient focal points (drawn centred)");
                }
                let (x, y) = (m.apply_vector(Vec2::new(1.0, 0.0)), m.apply_vector(Vec2::new(0.0, 1.0)));
                let (lx, ly) = (x.length(), y.length());
                // Columns of different lengths, or not at right angles: an ellipse.
                if (lx - ly).abs() > 0.01 * lx.max(ly) || x.dot(y).abs() > 0.01 * lx * ly {
                    report.skip("elliptical radial gradients (drawn round)");
                }
            }
            Some(if def.name == "LinearGradient" {
                Paint::Linear { start: m.apply(Vec2::new(-GRADIENT_HALF, 0.0)), end: m.apply(Vec2::new(GRADIENT_HALF, 0.0)), stops }
            } else {
                let c = m.apply(Vec2::ZERO);
                let r = (m.apply_vector(Vec2::new(GRADIENT_HALF, 0.0)).length() + m.apply_vector(Vec2::new(0.0, GRADIENT_HALF)).length()) * 0.5;
                Paint::Radial { center: c, radius: r, stops }
            })
        }
        "BitmapFill" => {
            let Some(&bitmap) = def.attr("bitmapPath").and_then(|n| bitmaps.get(n)) else {
                report.skip("bitmap fills of missing bitmaps");
                return None;
            };
            let m = matrix(def).m;
            let matrix = Affine { m: [m[0] / TWIPS, m[1] / TWIPS, m[2] / TWIPS, m[3] / TWIPS, m[4], m[5]] };
            // Animate repeats a bitmap fill unless it is clipped (then the edge pixels extend,
            // which AniCraft draws as the image shown once).
            Some(Paint::Bitmap { bitmap, repeat: !def.bool("bitmapIsClipped").unwrap_or(false), matrix })
        }
        _ => None,
    }
}

/// A stroke definition (`SolidStroke` and friends) as a stroke: weight, its fill colour, caps,
/// joints and miter limit, and the dashes of `DashedStroke` and dots of `DottedStroke`
/// (ADR 0046). Ragged, stipple and hatched strokes are drawn solid.
pub(crate) fn stroke(def: &X, bitmaps: &HashMap<String, BitmapId>, report: &mut Report) -> Option<Stroke> {
    if !matches!(def.name.as_str(), "SolidStroke" | "DashedStroke" | "DottedStroke" | "RaggedStroke" | "StippleStroke" | "HatchedStroke") {
        return None;
    }
    // Round caps and joints are the format's default, and a miter limit of 3.
    let cap = match def.attr("caps").map(str::trim) {
        Some("none") => Cap::Butt,
        Some("square") => Cap::Square,
        _ => Cap::Round,
    };
    let join = match def.attr("joints").map(str::trim) {
        Some("miter") => Join::Miter,
        Some("bevel") => Join::Bevel,
        _ => Join::Round,
    };
    let miter_limit = def.f32("miterLimit").unwrap_or(anicraft_doc::stroke_style::DEFAULT_MITER_LIMIT);
    let dash = match def.name.as_str() {
        "DashedStroke" => Dash::dashed(def.f32("dash1").unwrap_or(6.0), def.f32("dash2").unwrap_or(3.0)),
        "DottedStroke" => Dash::dotted(def.f32("dotSpace").unwrap_or(3.0)),
        "SolidStroke" => None,
        _ => {
            report.skip("ragged, stipple and hatched strokes (drawn solid)");
            None
        }
    };
    let style = StrokeStyle { cap, join, miter_limit, dash };
    let width = def.f32("weight").unwrap_or(1.0).clamp(0.05, 1000.0);
    let fill = def.child("fill").and_then(|f| f.children.first());
    let c = fill.map_or(Rgba::BLACK, |f| {
        if f.name == "SolidColor" {
            color(f, "color", "alpha")
        } else {
            report.skip("gradient and bitmap strokes (drawn in one colour)");
            paint(f, bitmaps, report).map_or(Rgba::BLACK, |p| p.primary_color())
        }
    });
    Some(Stroke::new(c, width).with_style(style))
}

impl<'a> Builder<'a> {
    fn element(&mut self, kind: ElementKind, transform: Affine, color: ColorEffect) -> Arc<Element> {
        Arc::new(Element { id: ElementId(self.doc.alloc_id()), name: String::new(), transform, color, kind })
    }

    /// Shapes for one `DOMShape`: a fill-only shape per fill style (bottom), then a stroke-only
    /// shape per stroke style, as Animate draws them. A shape loose on a frame (not in a group,
    /// not a drawing object) is raw artwork: its shapes are marked to merge (ADR 0042), and the
    /// engine merges each keyframe's raw shapes into one drawing when it opens the file.
    fn shape(&mut self, el: &X, parent: &Affine, out: &mut Vec<Arc<Element>>, loose: bool) {
        let m = parent.mul(&matrix(el));
        let merge = loose && !el.bool("isDrawingObject").unwrap_or(false);
        let fills: Vec<(u32, Option<Paint>)> = el.list("fills").iter().map(|f| (f.u32("index").unwrap_or(0), f.children.first().and_then(|d| paint(d, &self.bitmaps, &mut self.report)))).collect();
        let strokes: Vec<(u32, Option<Stroke>)> = el.list("strokes").iter().map(|s| (s.u32("index").unwrap_or(0), s.children.first().and_then(|d| stroke(d, &self.bitmaps, &mut self.report)))).collect();
        let edges: Vec<Edge> = el
            .list("edges")
            .iter()
            .filter(|e| e.name == "Edge")
            .map(|e| Edge { fill_left: e.u32("fillStyle0").unwrap_or(0), fill_right: e.u32("fillStyle1").unwrap_or(0), stroke: e.u32("strokeStyle").unwrap_or(0), segs: e.attr("edges").map(edges::parse).unwrap_or_default() })
            .collect();
        for (index, p) in fills {
            let (Some(p), true) = (p, index > 0) else { continue };
            let path = edges::fill_path(&edges, index);
            if !path.is_empty() {
                out.push(self.element(ElementKind::Shape(Shape { path: Arc::new(path), fill: Some(p), stroke: None, fill_rule: FillRule::EvenOdd, merge }), m, ColorEffect::default()));
            }
        }
        for (index, s) in strokes {
            let (Some(s), true) = (s, index > 0) else { continue };
            let path = edges::stroke_path(&edges, index);
            if !path.is_empty() {
                out.push(self.element(ElementKind::Shape(Shape { path: Arc::new(path), fill: None, stroke: Some(s), fill_rule: FillRule::NonZero, merge }), m, ColorEffect::default()));
            }
        }
    }

    /// Rectangle and oval primitives.
    fn primitive(&mut self, el: &X, parent: &Affine, out: &mut Vec<Arc<Element>>) {
        let (x, y) = (el.f32("x").unwrap_or(0.0), el.f32("y").unwrap_or(0.0));
        let (w, h) = (el.f32("objectWidth").unwrap_or(0.0), el.f32("objectHeight").unwrap_or(0.0));
        if !(w > 0.0 && h > 0.0) {
            return;
        }
        let path = if el.name == "DOMOvalObject" { Path::ellipse(x + w / 2.0, y + h / 2.0, w / 2.0, h / 2.0) } else { Path::rounded_rect(x, y, w, h, el.f32("topLeftRadius").unwrap_or(0.0).max(0.0)) };
        let fill = el.child("fill").and_then(|f| f.children.first()).and_then(|d| paint(d, &self.bitmaps, &mut self.report));
        let stroke = el.child("stroke").and_then(|s| s.children.first()).and_then(|d| stroke(d, &self.bitmaps, &mut self.report));
        if fill.is_some() || stroke.is_some() {
            let m = parent.mul(&matrix(el));
            out.push(self.element(ElementKind::Shape(Shape { path: Arc::new(path), fill, stroke, fill_rule: FillRule::NonZero, merge: false }), m, ColorEffect::default()));
        }
    }

    /// A bitmap instance: a rectangle of the image's size showing it, as Import to Stage makes.
    fn bitmap_instance(&mut self, el: &X, parent: &Affine, out: &mut Vec<Arc<Element>>) {
        let found = el.attr("libraryItemName").and_then(|n| self.bitmaps.get(n)).and_then(|&id| self.doc.bitmap(id).map(|b| (id, b.width as f32, b.height as f32)));
        let Some((bitmap, w, h)) = found else {
            self.report.skip("instances of missing bitmaps");
            return;
        };
        let shape = Shape { path: Arc::new(Path::rect(0.0, 0.0, w, h)), fill: Some(Paint::Bitmap { bitmap, repeat: false, matrix: Affine::IDENTITY }), stroke: None, fill_rule: FillRule::NonZero, merge: false };
        out.push(self.element(ElementKind::Shape(shape), parent.mul(&matrix(el)), ColorEffect::default()));
    }

    /// An instance's filters, by Animate's element names; switched-off filters are left out,
    /// and unknown ones reported.
    fn filters(&mut self, el: &X) -> Vec<Filter> {
        let mut out = Vec::new();
        for f in el.list("filters") {
            if f.bool("enabled") == Some(false) {
                continue;
            }
            let num = |k: &str, d: f32| f.f32(k).unwrap_or(d);
            let flag = |k: &str| f.bool(k).unwrap_or(false);
            let quality = f.u32("quality").unwrap_or(1).clamp(1, 3) as u8;
            let filter = match f.name.as_str() {
                "BlurFilter" => Filter::Blur { blur_x: num("blurX", 5.0), blur_y: num("blurY", 5.0), quality },
                "DropShadowFilter" => Filter::DropShadow {
                    blur_x: num("blurX", 5.0),
                    blur_y: num("blurY", 5.0),
                    strength: num("strength", 1.0),
                    quality,
                    angle: num("angle", 45.0),
                    distance: num("distance", 4.0),
                    color: color(f, "color", "alpha"),
                    inner: flag("inner"),
                    knockout: flag("knockout"),
                    hide_object: flag("hideObject"),
                },
                "GlowFilter" => Filter::Glow { blur_x: num("blurX", 5.0), blur_y: num("blurY", 5.0), strength: num("strength", 1.0), quality, color: color(f, "color", "alpha"), inner: flag("inner"), knockout: flag("knockout") },
                "AdjustColorFilter" => Filter::AdjustColor { brightness: num("brightness", 0.0), contrast: num("contrast", 0.0), saturation: num("saturation", 0.0), hue: num("hue", 0.0) },
                "BevelFilter" => Filter::Bevel {
                    blur_x: num("blurX", 5.0),
                    blur_y: num("blurY", 5.0),
                    strength: num("strength", 1.0),
                    quality,
                    angle: num("angle", 45.0),
                    distance: num("distance", 5.0),
                    shadow_color: color(f, "shadowColor", "shadowAlpha"),
                    highlight_color: if f.attr("highlightColor").is_some() { color(f, "highlightColor", "highlightAlpha") } else { Rgba::WHITE.with_alpha(num("highlightAlpha", 1.0).clamp(0.0, 1.0)) },
                    side: filter_side(f, FilterSide::Inner),
                    knockout: flag("knockout"),
                },
                "GradientGlowFilter" | "GradientBevelFilter" => {
                    let glow = f.name == "GradientGlowFilter";
                    let (blur_x, blur_y, strength, angle, distance, knockout) = (num("blurX", 5.0), num("blurY", 5.0), num("strength", 1.0), num("angle", 45.0), num("distance", 5.0), flag("knockout"));
                    let stops = ramp(f);
                    if glow {
                        Filter::GradientGlow { blur_x, blur_y, strength, quality, angle, distance, stops: if stops.is_empty() { anicraft_doc::default_glow_ramp() } else { stops }, side: filter_side(f, FilterSide::Outer), knockout }
                    } else {
                        Filter::GradientBevel { blur_x, blur_y, strength, quality, angle, distance, stops: if stops.is_empty() { anicraft_doc::default_bevel_ramp() } else { stops }, side: filter_side(f, FilterSide::Inner), knockout }
                    }
                }
                _ => {
                    self.report.skip("unknown filters");
                    continue;
                }
            };
            out.push(filter.sanitized());
        }
        out
    }

    /// A text field: one text element per block (`text::field`), placed by the field's matrix.
    fn text_field(&mut self, el: &X, parent: &Affine, out: &mut Vec<Arc<Element>>) {
        let fonts: &'a Fonts = match self.fonts {
            Some(f) => f,
            None => Fonts::get(),
        };
        self.fonts = Some(fonts);
        let m = parent.mul(&matrix(el));
        let name: String = el.attr("name").unwrap_or("").chars().take(256).collect();
        for (i, p) in crate::text::field(el, fonts, &mut self.faces, &mut self.report).into_iter().enumerate() {
            let mut e = Element { id: ElementId(self.doc.alloc_id()), name: String::new(), transform: m.mul(&Affine::translate(p.offset.x, p.offset.y)), color: ColorEffect::default(), kind: ElementKind::Text(Arc::new(p.block)) };
            // A dynamic or input field's instance name goes on it (its first block for static
            // text, which keeps the name for a later conversion).
            if i == 0 {
                e.name.clone_from(&name);
            }
            out.push(Arc::new(e));
        }
    }

    /// An instance's `Color` (ADR 0056): Animate's style comes from which attributes it has: a
    /// tint (`tintMultiplier`, `tintColor`), a brightness (`brightness`, −1..1), Advanced (any
    /// channel's multiplier other than 1 or an offset, −255..255), or alpha alone. Values are
    /// clamped into Animate's ranges.
    fn color_effect(&mut self, c: &X) -> ColorEffect {
        let mul = |k: &str| c.f32(k).filter(|v| v.is_finite()).unwrap_or(1.0);
        let off = |k: &str| c.f32(k).filter(|v| v.is_finite()).unwrap_or(0.0) / 255.0;
        let alpha = mul("alphaMultiplier");
        let fx = if let Some(t) = c.f32("tintMultiplier") {
            let tint = ColorEffect::Tint { color: color(c, "tintColor", "__none").with_alpha(1.0), amount: t };
            if alpha != 1.0 { tint.with_alpha(alpha) } else { tint }
        } else if let Some(b) = c.f32("brightness").filter(|b| *b != 0.0) {
            let bright = ColorEffect::Brightness(b);
            if alpha != 1.0 { bright.with_alpha(alpha) } else { bright }
        } else {
            let t = ColorTransform {
                mul: [mul("redMultiplier"), mul("greenMultiplier"), mul("blueMultiplier"), alpha],
                add: [off("redOffset"), off("greenOffset"), off("blueOffset"), off("alphaOffset")],
            };
            if t.changes_colour() || t.add[3] != 0.0 {
                ColorEffect::Advanced(t)
            } else if alpha != 1.0 {
                ColorEffect::Alpha(alpha)
            } else {
                ColorEffect::None
            }
        };
        fx.sanitized()
    }

    fn instance(&mut self, el: &X, parent: &Affine, out: &mut Vec<Arc<Element>>) {
        let Some(&symbol) = el.attr("libraryItemName").and_then(|n| self.ids.get(n)) else {
            self.report.skip("instances of missing symbols");
            return;
        };
        let loop_mode = match el.attr("loop").map(str::trim) {
            Some("play once") => LoopMode::PlayOnce,
            Some("single frame") => LoopMode::SingleFrame,
            Some(r @ ("play once reverse" | "loop reverse")) => {
                self.report.skip("reverse loop modes (played forward)");
                if r == "loop reverse" { LoopMode::Loop } else { LoopMode::PlayOnce }
            }
            _ => LoopMode::Loop,
        };
        let first_frame = el.u32("firstFrame").unwrap_or(0).min(MAX_FRAME);
        let fx = el.child("color").and_then(|c| c.child("Color")).map_or_else(ColorEffect::default, |c| self.color_effect(c));
        let m = parent.mul(&matrix(el));
        // Blend modes by Animate's names; unknown ones stay Normal.
        let blend = el.attr("blendMode").and_then(BlendMode::from_key).unwrap_or_default();
        let filters = self.filters(el);
        let mut e = Element { id: ElementId(self.doc.alloc_id()), name: String::new(), transform: m, color: fx, kind: ElementKind::Symbol { symbol, loop_mode, first_frame, blend, filters, events: Vec::new() } };
        // The instance name actions address it by (ADR 0024); one they couldn't is reported.
        if let Some(name) = el.attr("name").map(str::trim).filter(|n| !n.is_empty()) {
            if anicraft_doc::actions::valid_instance_name(name) {
                e.name = name.to_string();
            } else {
                self.report.skip("instance names actions can't use (left out)");
            }
        }
        // Older documents put handlers (`on (release) { … }`) on instances; only frame
        // scripts have a place yet.
        if el.child("Actionscript").is_some() {
            self.report.skip("scripts on instances (left out; frame scripts are kept)");
        }
        out.push(Arc::new(e));
    }

    fn elements(&mut self, list: &[X], parent: &Affine, out: &mut Vec<Arc<Element>>, depth: usize) {
        if depth > 64 {
            return;
        }
        for el in list {
            match el.name.as_str() {
                "DOMShape" => self.shape(el, parent, out, depth == 0),
                "DOMGroup" => {
                    let m = parent.mul(&matrix(el));
                    self.elements(el.list("members"), &m, out, depth + 1);
                }
                "DOMSymbolInstance" => self.instance(el, parent, out),
                "DOMRectangleObject" | "DOMOvalObject" => self.primitive(el, parent, out),
                "DOMBitmapInstance" => self.bitmap_instance(el, parent, out),
                "DOMStaticText" | "DOMDynamicText" | "DOMInputText" => self.text_field(el, parent, out),
                "DOMVideoInstance" => self.report.skip("videos"),
                _ => self.report.skip("other elements"),
            }
        }
    }

    fn timeline(&mut self, tl: &X) -> Timeline {
        let mut layers = Vec::new();
        // Layers by their position in the file's layer list: `parentLayerIndex` points at a
        // layer's folder or mask. Links that don't fit AniCraft's rules are dropped by
        // `repair_masks` below.
        let mut by_index: Vec<LayerId> = Vec::new();
        for l in tl.list("layers").iter().filter(|l| l.name == "DOMLayer").take(MAX_LAYERS) {
            let kind = match l.attr("layerType") {
                Some("folder") => LayerKind::Folder,
                Some("guide") => LayerKind::Guide,
                Some("mask") => LayerKind::Mask,
                _ => LayerKind::Normal,
            };
            let id = LayerId(self.doc.alloc_id());
            by_index.push(id);
            let mut layer = Layer::new(id, l.attr("name").unwrap_or("Layer"));
            layer.kind = kind;
            layer.parent = l.u32("parentLayerIndex").and_then(|i| by_index.get(i as usize).copied()).filter(|p| *p != id);
            if kind == LayerKind::Folder {
                // A folder holds layers, never frames.
                layer.keyframes.clear();
                layer.visible = l.bool("visible").unwrap_or(true);
                layer.locked = l.bool("locked").unwrap_or(false);
                layer.color = color(l, "color", "__none");
                layers.push(Arc::new(layer));
                continue;
            }
            layer.visible = l.bool("visible").unwrap_or(true);
            layer.locked = l.bool("locked").unwrap_or(false);
            layer.outline = l.bool("outline").unwrap_or(false);
            layer.color = color(l, "color", "__none");
            // Layer Properties' height, 100 / 200 / 300 % (ADR 0061), as a multiple of a row.
            layer.height_rows = l.u32("heightMultiplier").map_or(1, |h| h.clamp(1, u32::from(anicraft_doc::MAX_LAYER_HEIGHT)) as u8);
            let mut keys = Vec::new();
            let mut raw_hints: HashMap<u32, Vec<RawHint>> = HashMap::new();
            for f in l.list("frames").iter().filter(|f| f.name == "DOMFrame") {
                let start = f.u32("index").unwrap_or(0).min(MAX_FRAME);
                let duration = f.u32("duration").unwrap_or(1).clamp(1, MAX_FRAME);
                let mut k = Keyframe::new(start, duration);
                // A frame's name is a label (or an anchor, a label too); a comment is a note
                // to the animator that nothing can go to, which AniCraft has no place for yet.
                if f.attr("labelType").map(str::trim) == Some("comment") {
                    self.report.skip("frame comments");
                } else {
                    k.label = f.attr("name").unwrap_or("").chars().take(256).collect();
                }
                match f.attr("tweenType") {
                    Some(t @ ("motion" | "shape")) => {
                        let kind = if t == "shape" { TweenKind::Shape } else { TweenKind::Classic };
                        let orient = f.bool("motionTweenOrientToPath").unwrap_or(false);
                        if kind == TweenKind::Classic && f.attr("motionTweenRotate").is_some_and(|r| matches!(r.trim(), "clockwise" | "counter-clockwise")) {
                            self.report.skip("tween spins (turned the shortest way)");
                        }
                        k.tween = Some(Tween { easing: self.easing(f), kind, orient });
                    }
                    Some("motion object") => self.report.skip("motion tweens (kept as keyframes)"),
                    _ => {}
                }
                // Shape hints sit on both keyframes of a shape tween; they are paired up below.
                if let Some(h) = f.child("shapeHints").filter(|h| !h.children.is_empty()) {
                    raw_hints.insert(start, self.raw_hints(h));
                }
                // Frame scripts are kept verbatim (ADR 0013 tier 1); those in the subset run.
                if let Some(a) = f.child("Actionscript") {
                    k.script = self.frame_script(a);
                }
                if let Some(name) = f.attr("soundName").filter(|n| !n.trim().is_empty()) {
                    k.sound = self.frame_sound(f, name);
                }
                let mut els = Vec::new();
                self.elements(f.list("elements"), &Affine::IDENTITY, &mut els, 0);
                k.elements = els;
                keys.push(Arc::new(k));
            }
            if !keys.is_empty() {
                layer.keyframes = keys;
            }
            layer.normalize();
            if !raw_hints.is_empty() {
                self.pair_hints(&mut layer, &raw_hints);
            }
            layers.push(Arc::new(layer));
        }
        if layers.is_empty() {
            layers.push(Arc::new(Layer::new(LayerId(self.doc.alloc_id()), "Layer 1")));
        }
        let mut tl = Timeline { layers };
        // Links that do not form an unbroken run under their mask are dropped.
        tl.repair_masks();
        tl
    }

    /// A frame's `<Actionscript><script>` text, exactly as written (its `<script>` elements in
    /// order, though files have one), in the document's language.
    fn frame_script(&mut self, a: &X) -> Option<FrameScript> {
        let parts: Vec<&str> = a.children.iter().filter(|c| c.name == "script").map(|c| c.text.as_str()).collect();
        let text = parts.join("\n");
        if text.trim().is_empty() {
            return None;
        }
        let script = SourceScript::new(self.language, text);
        if script.check().is_err() {
            self.report.skip("frame scripts over 1 MiB (left out)");
            return None;
        }
        // Compiling here also fills the script's cache for Test Movie (ADR 0013 tier 2).
        let what = format!("frame scripts in {}", self.language.label());
        if script.runs() { self.report.run(&what) } else { self.report.keep(&what) }
        Some(FrameScript::Source(Arc::new(script)))
    }

    /// A tween's ease: the classic single-slider `acceleration`, or a custom ease curve. A
    /// curve of one segment (four points from (0, 0) to (1, 1)) is a cubic Bézier exactly; a
    /// longer one, or one per property, is drawn with the slider's ease and reported.
    fn easing(&mut self, f: &X) -> Easing {
        let acc = f.f32("acceleration").unwrap_or(0.0).clamp(-100.0, 100.0);
        let classic = if acc == 0.0 { Easing::Linear } else { Easing::Classic(acc) };
        if f.bool("hasCustomEase") != Some(true) {
            return classic;
        }
        let curves: Vec<&X> = f.list("tweens").iter().filter(|c| c.name == "CustomEase").collect();
        let one_curve = f.bool("useSingleEaseCurve") != Some(false) || curves.len() == 1;
        let bezier = curves.first().filter(|_| one_curve).and_then(|c| {
            let p: Vec<Vec2> = c.children.iter().filter(|p| p.name == "Point").take(5).map(|p| Vec2::new(p.f32("x").unwrap_or(0.0), p.f32("y").unwrap_or(0.0))).collect();
            let [a, b, c, d] = p.as_slice() else { return None };
            let near = |p: &Vec2, x: f32, y: f32| (p.x - x).abs() < 1e-3 && (p.y - y).abs() < 1e-3;
            (near(a, 0.0, 0.0) && near(d, 1.0, 1.0)).then(|| Easing::CubicBezier(b.x.clamp(0.0, 1.0), b.y.clamp(-10.0, 10.0), c.x.clamp(0.0, 1.0), c.y.clamp(-10.0, 10.0)))
        });
        bezier.unwrap_or_else(|| {
            self.report.skip("custom ease curves (drawn with the simple ease)");
            classic
        })
    }

    /// The hints listed in a frame's `shapeHints`, in letter order (ADR 0023). Each child is a
    /// hint; it gives its point on this keyframe as `x`/`y` (or `startPoint="x, y"`, or
    /// `startX`/`startY`), and may carry its end point too (`endPoint`, or `endX`/`endY`);
    /// otherwise the end comes from the same hint on the next keyframe. Points are in pixels.
    /// The order is `index` (or a letter `name`), else the order in the file.
    fn raw_hints(&mut self, list: &X) -> Vec<RawHint> {
        let pt = |h: &X, joined: &str, x: &str, y: &str| -> Option<Vec2> {
            let p = match h.attr(joined) {
                Some(v) => {
                    let mut it = v.split([',', ' ']).filter(|s| !s.trim().is_empty()).map(|s| s.trim().parse::<f32>().ok());
                    Vec2::new(it.next()??, it.next()??)
                }
                None => Vec2::new(h.f32(x)?, h.f32(y)?),
            };
            (p.is_finite() && p.x.abs() < 1e6 && p.y.abs() < 1e6).then_some(p)
        };
        let mut out: Vec<RawHint> = Vec::new();
        for (n, h) in list.children.iter().enumerate().take(anicraft_doc::MAX_SHAPE_HINTS * 4) {
            let order = h.u32("index").map(|i| i as usize).or_else(|| h.attr("name").and_then(anicraft_doc::hint_index)).unwrap_or(n);
            let Some(here) = pt(h, "startPoint", "startX", "startY").or_else(|| pt(h, "point", "x", "y")) else {
                self.report.skip("shape hints without a position");
                continue;
            };
            out.push(RawHint { order, here, end: pt(h, "endPoint", "endX", "endY") });
        }
        out.sort_by_key(|h| h.order);
        if out.len() > anicraft_doc::MAX_SHAPE_HINTS {
            self.report.skip("shape hints past z");
            out.truncate(anicraft_doc::MAX_SHAPE_HINTS);
        }
        out
    }

    /// Gives each shape tween on `layer` the hints its keyframes list, snapped onto the edges
    /// when they sit within a couple of pixels of one (the file's points are rounded).
    fn pair_hints(&mut self, layer: &mut Layer, raw: &HashMap<u32, Vec<RawHint>>) {
        const SNAP: f32 = 2.0;
        let snap = |els: &[Arc<anicraft_doc::Element>], p: Vec2| match anicraft_doc::nearest_outline(els, p) {
            Some((_, q, d)) if d <= SNAP => q,
            _ => p,
        };
        for i in 0..layer.keyframes.len() {
            let (Some(k), Some(next)) = (layer.keyframes.get(i), layer.keyframes.get(i + 1)) else { continue };
            let Some(list) = raw.get(&k.start) else { continue };
            if !layer.is_shape_tween(i) {
                continue;
            }
            let ends = raw.get(&next.start);
            let mut hints = Vec::new();
            for (n, h) in list.iter().enumerate() {
                // The same hint on the end keyframe: by position in its list.
                let Some(end) = h.end.or_else(|| ends.and_then(|e| e.get(n)).map(|e| e.here)) else {
                    self.report.skip("shape hints missing on the end keyframe");
                    continue;
                };
                hints.push(ShapeHint { start: snap(&k.elements, h.here), end: snap(&next.elements, end) });
            }
            if let Some(k) = layer.keyframes.get_mut(i) {
                Arc::make_mut(k).hints = hints;
            }
        }
    }

    /// Reads the document's bitmap library items. Each one's image is looked for as the
    /// original file in `LIBRARY/` (XFL folders keep it there) and then as its data file in
    /// `bin/`; either is used when it is a PNG or JPEG. Animate's own encodings of the data
    /// file are not public, so those bitmaps are reported instead.
    fn register_bitmaps(&mut self, root: &X) -> Result<(), XflError> {
        for item in root.list("media").iter().filter(|i| i.name == "DOMBitmapItem").take(MAX_BITMAPS) {
            let Some(name) = item.attr("name").filter(|n| !self.bitmaps.contains_key(*n)) else { continue };
            let mut candidates = Vec::new();
            if let Some(href) = item.attr("href") {
                candidates.push(format!("LIBRARY/{href}"));
            }
            if let Some(data) = item.attr("bitmapDataHRef") {
                candidates.push(format!("bin/{data}"));
            }
            let short: String = name.rsplit('/').next().unwrap_or(name).chars().take(256).collect();
            let mut found_data = false;
            let mut bitmap = None;
            for path in candidates {
                let Some(bytes) = (self.read)(&path)? else { continue };
                found_data = true;
                if let Ok(b) = Bitmap::from_image(BitmapId(0), short.clone(), bytes) {
                    bitmap = Some(b);
                    break;
                }
            }
            match bitmap {
                Some(mut b) => {
                    b.id = BitmapId(self.doc.alloc_id());
                    // Animate leaves out the default (smoothing on in current versions).
                    b.smoothing = item.bool("allowSmoothing").unwrap_or(true);
                    self.place_in_folder(name, anicraft_doc::LibraryItem::Bitmap(b.id));
                    self.bitmaps.insert(name.to_string(), b.id);
                    self.doc.bitmaps.push(Arc::new(b));
                }
                None if found_data => self.report.skip("bitmaps stored in Animate's own format"),
                None => self.report.skip("missing bitmap files"),
            }
        }
        Ok(())
    }

    /// Reads the document's sound library items (ADR 0057). Like a bitmap, each one's sound is
    /// looked for as the original file in `LIBRARY/` and then as its data file in `bin/`, and is
    /// used when it is a WAV, Ogg Vorbis or MP3 file. Animate's own encoding of the data file is
    /// not public, so those sounds are reported instead (ADR 0039).
    fn register_sounds(&mut self, root: &X) -> Result<(), XflError> {
        for item in root.list("media").iter().filter(|i| i.name == "DOMSoundItem").take(MAX_SOUNDS) {
            let Some(name) = item.attr("name").filter(|n| !self.sounds.contains_key(*n)) else { continue };
            let mut candidates = Vec::new();
            if let Some(href) = item.attr("href") {
                candidates.push(format!("LIBRARY/{href}"));
            }
            if let Some(data) = item.attr("soundDataHRef") {
                candidates.push(format!("bin/{data}"));
            }
            let short: String = name.rsplit('/').next().unwrap_or(name).chars().take(256).collect();
            let mut found_data = false;
            let mut sound = None;
            for path in candidates {
                let Some(bytes) = (self.read)(&path)? else { continue };
                found_data = true;
                if let Ok(s) = anicraft_doc::Sound::from_data(anicraft_doc::SoundId(0), short.clone(), bytes) {
                    sound = Some(s);
                    break;
                }
            }
            match sound {
                Some(mut s) => {
                    s.id = anicraft_doc::SoundId(self.doc.alloc_id());
                    self.place_in_folder(name, anicraft_doc::LibraryItem::Sound(s.id));
                    self.sounds.insert(name.to_string(), s.id);
                    self.doc.sounds.push(Arc::new(s));
                }
                None if found_data => self.report.skip("sounds stored in Animate's own format"),
                None => self.report.skip("missing sound files"),
            }
        }
        Ok(())
    }

    /// The sound on a frame (`soundName`, the library item), with its sync (`soundSync`: event,
    /// start, stop or stream) and repeats (`soundLoopMode` loop or repeat, `soundLoop` times).
    /// Buttons' frames included: they play as the pointer changes the button's state.
    fn frame_sound(&mut self, f: &X, name: &str) -> Option<FrameSound> {
        let Some(id) = self.sounds.get(name).copied() else {
            self.report.skip("frame sounds whose sound isn't in the package");
            return None;
        };
        let mut fs = FrameSound::new(id);
        fs.sync = match f.attr("soundSync").map(str::trim) {
            Some("start") => SoundSync::Start,
            Some("stop") => SoundSync::Stop,
            Some("stream") => SoundSync::Stream,
            _ => SoundSync::Event,
        };
        fs.looping = f.attr("soundLoopMode").map(str::trim) == Some("loop");
        fs.repeat = f.u32("soundLoop").unwrap_or(1).clamp(1, anicraft_doc::sound::MAX_REPEAT);
        if f.attr("soundEffect").is_some_and(|e| !matches!(e.trim(), "" | "none")) || f.child("SoundEnvelope").is_some() {
            self.report.skip("sound effects and envelopes (played without)");
        }
        if f.attr("inPoint44").is_some() || f.attr("outPoint44").is_some() {
            self.report.skip("sound in and out points (played whole)");
        }
        Some(fs)
    }

    /// Allocates ids for every symbol the document lists, parsing their files.
    fn register_symbols(&mut self, root: &X) -> Result<(), XflError> {
        let mut seen = HashSet::new();
        for inc in root.list("symbols").iter().filter(|i| i.name == "Include").take(MAX_SYMBOLS) {
            let Some(href) = inc.attr("href") else { continue };
            if !seen.insert(href.to_string()) {
                continue;
            }
            let Some(bytes) = (self.read)(&format!("LIBRARY/{href}"))? else {
                self.report.skip("missing library files");
                continue;
            };
            let item = xml::parse(&bytes)?;
            if item.name != "DOMSymbolItem" {
                continue;
            }
            let name = item.attr("name").map(str::to_string).unwrap_or_else(|| href.trim_end_matches(".xml").to_string());
            // The format leaves out default values, and a symbol's default type is movie clip:
            // only graphics and buttons say what they are.
            let kind = match item.attr("symbolType").map(str::trim) {
                Some("graphic") => SymbolKind::Graphic,
                Some("button") => SymbolKind::Button,
                _ => SymbolKind::MovieClip,
            };
            let id = SymbolId(self.doc.alloc_id());
            let linkage = self.linkage(&item);
            self.ids.insert(name.clone(), id);
            self.pending.push((id, name, kind, linkage, item));
        }
        Ok(())
    }

    /// A library symbol's script class (`linkageClassName`, or an older document's
    /// `linkageIdentifier`, when `linkageExportForAS` doesn't turn it off) and its base class.
    fn linkage(&mut self, item: &X) -> Option<Linkage> {
        if item.attr("linkageExportForAS").is_some_and(|v| v.trim() != "true") {
            return None;
        }
        let class = ["linkageClassName", "linkageIdentifier"].iter().find_map(|k| item.attr(k).map(str::trim).filter(|c| !c.is_empty()))?;
        let base = item.attr("linkageBaseClass").map(str::trim).filter(|b| anicraft_doc::actions::valid_class_name(b)).unwrap_or("");
        let l = Linkage { class: class.to_string(), base: base.to_string() };
        if l.check().is_err() {
            self.report.skip("linkage classes that aren't class names (left out)");
            return None;
        }
        self.report.keep("linkage classes");
        Some(l)
    }

    /// A symbol's 9-slice guides (ADR 0055): `scaleGridLeft`, `scaleGridRight`, `scaleGridTop`
    /// and `scaleGridBottom` on the `DOMSymbolItem`, in the symbol's space, written when its
    /// guides are on. All four or none; guides given the wrong way round are put in order.
    fn scale_grid(&mut self, item: &X) -> Option<ScaleGrid> {
        let keys = ["scaleGridLeft", "scaleGridRight", "scaleGridTop", "scaleGridBottom"];
        if keys.iter().all(|k| item.attr(k).is_none()) {
            return None;
        }
        let v: Vec<f32> = keys.iter().filter_map(|k| item.f32(k)).collect();
        let grid = match v[..] {
            [l, r, t, b] => Some(ScaleGrid { left: l.min(r), right: l.max(r), top: t.min(b), bottom: t.max(b) }),
            _ => None,
        };
        let grid = grid.filter(|g| g.check().is_ok());
        if grid.is_none() {
            self.report.skip("9-slice guides that aren't usable (left out)");
        }
        grid
    }

    /// The script settings of the package's publish profile (`PublishSettings.xml`): the
    /// language its frame scripts are written in (`ActionScriptVersion`; 3 when the file
    /// doesn't say) and the document class (`DocumentClass`). The profile marked current is
    /// used, else the first. A missing or unreadable file changes nothing: it is settings, not
    /// content.
    fn publish_settings(&mut self) -> Result<(), XflError> {
        let Some(bytes) = (self.read)("PublishSettings.xml")? else { return Ok(()) };
        let Ok(root) = xml::parse(&bytes) else { return Ok(()) };
        let profiles: Vec<&X> = root.children.iter().filter(|c| c.name == "flash_profile").collect();
        let profile = profiles.iter().find(|p| p.bool("current") == Some(true)).or(profiles.first()).copied().unwrap_or(&root);
        let flash = profile.child("PublishFlashProperties").unwrap_or(profile);
        let text = |name: &str| flash.child(name).map(|e| e.text.trim().to_string()).filter(|t| !t.is_empty());
        self.language = match text("ActionScriptVersion").as_deref() {
            Some("1" | "2") => Language::ActionScript1or2,
            _ => Language::ActionScript3,
        };
        if let Some(class) = text("DocumentClass") {
            if anicraft_doc::actions::valid_class_name(&class) {
                self.doc.document_class = class;
                self.report.keep("document class");
            } else {
                self.report.skip("document classes that aren't class names (left out)");
            }
        }
        Ok(())
    }

    fn build_symbols(&mut self) {
        for (id, name, kind, linkage, item) in std::mem::take(&mut self.pending) {
            let timeline = match item.child("timeline").and_then(|t| t.child("DOMTimeline")) {
                Some(tl) => self.timeline(tl),
                None => Timeline { layers: vec![Arc::new(Layer::new(LayerId(self.doc.alloc_id()), "Layer 1"))] },
            };
            let short = name.rsplit('/').next().unwrap_or(&name).to_string();
            let scale_grid = self.scale_grid(&item);
            self.doc.library.push(Arc::new(Symbol { id, name: short, kind, linkage, scale_grid, scale_grid_kept: None, timeline }));
            self.place_in_folder(&name, anicraft_doc::LibraryItem::Symbol(id));
        }
    }

    /// The Library's folders (`<folders>`, so empty ones come across too), each a path such as
    /// `Characters/Hero` (ADR 0050).
    fn register_folders(&mut self, root: &X) {
        for f in root.list("folders").iter().filter(|f| f.name == "DOMFolderItem").take(anicraft_doc::library::MAX_FOLDERS) {
            if let Some(path) = f.attr("name") {
                self.doc.ensure_folder_path(path);
            }
        }
    }

    /// Puts `item` in the folder its library name's path names (`Characters/Ball` is in
    /// `Characters`), made if `<folders>` didn't list it.
    fn place_in_folder(&mut self, name: &str, item: anicraft_doc::LibraryItem) {
        if let Some((dir, _)) = name.rsplit_once('/')
            && let Some(f) = self.doc.ensure_folder_path(dir)
        {
            self.doc.place_entry(anicraft_doc::LibraryEntry::Item(item), Some(f));
        }
    }
}

/// Imports an XFL package through `read` (paths inside the package). Text is laid out with
/// the fonts on this computer.
pub fn import_with(read: &Reader<'_>) -> Result<(Document, Report), XflError> {
    import_from(read, None)
}

/// Imports an XFL package, laying text out with `fonts` (tests, and hosts with their own font
/// library).
pub fn import_with_fonts(read: &Reader<'_>, fonts: &Fonts) -> Result<(Document, Report), XflError> {
    import_from(read, Some(fonts))
}

fn import_from<'a>(read: &'a Reader<'a>, fonts: Option<&'a Fonts>) -> Result<(Document, Report), XflError> {
    let bytes = read("DOMDocument.xml")?.ok_or_else(|| XflError::NotXfl("no DOMDocument.xml".into()))?;
    let root = xml::parse(&bytes)?;
    if root.name != "DOMDocument" {
        return Err(XflError::NotXfl(format!("the root element is <{}>", root.name)));
    }
    let mut b = Builder { doc: Document::default(), read, ids: HashMap::new(), pending: Vec::new(), language: Language::ActionScript3, bitmaps: HashMap::new(), sounds: HashMap::new(), report: Report::default(), fonts, faces: HashMap::new() };
    b.publish_settings()?;
    // Only symbols keep a script class yet.
    for _ in root.list("media").iter().filter(|m| m.attr("linkageClassName").is_some_and(|c| !c.trim().is_empty())) {
        b.report.skip("linkage classes on bitmaps and sounds (left out)");
    }
    b.doc.library.clear();
    b.doc.scenes.clear();
    b.doc.stage = Stage {
        width: root.f32("width").unwrap_or(550.0).clamp(1.0, 8192.0) as u32,
        height: root.f32("height").unwrap_or(400.0).clamp(1.0, 8192.0) as u32,
        fps: root.f32("frameRate").unwrap_or(24.0).clamp(1.0, 120.0),
        background: color(&root, "backgroundColor", "__none"),
    };
    b.register_folders(&root);
    b.register_bitmaps(&root)?;
    b.register_sounds(&root)?;
    b.register_symbols(&root)?;
    b.build_symbols();
    // Every DOMTimeline is a scene, in order.
    let scenes: Vec<&X> = root.list("timelines").iter().filter(|t| t.name == "DOMTimeline").take(anicraft_doc::MAX_SCENES).collect();
    for (i, tl) in scenes.iter().enumerate() {
        let name: String = tl.attr("name").filter(|n| !n.trim().is_empty()).map_or_else(|| format!("Scene {}", i + 1), |n| n.chars().take(256).collect());
        let timeline = b.timeline(tl);
        b.doc.scenes.push(Arc::new(Scene { name, timeline }));
    }
    if !b.doc.library_tree.folders.is_empty() {
        b.report.keep("library folders");
    }
    if b.doc.scenes.is_empty() {
        let timeline = Timeline { layers: vec![Arc::new(Layer::new(LayerId(b.doc.alloc_id()), "Layer 1"))] };
        b.doc.scenes.push(Arc::new(Scene { name: "Scene 1".into(), timeline }));
    }
    Ok((b.doc, b.report))
}

/// Largest `.fla` archive read.
pub const MAX_FLA_BYTES: u64 = crate::zip::MAX_ENTRY as u64 * 4;

/// The first bytes of a compound (OLE) file: a `.fla` saved by Flash CS4 or earlier.
const OLE_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

/// Imports a `.fla` archive from its bytes (a file the browser handed over, say). Text is laid
/// out with the fonts on this computer.
pub fn import_fla(data: &[u8]) -> Result<(Document, Report), XflError> {
    fla_from(data, None)
}

/// Imports a `.fla` archive, laying text out with `fonts` (tests, and hosts with their own
/// font library).
pub fn import_fla_with_fonts(data: &[u8], fonts: &Fonts) -> Result<(Document, Report), XflError> {
    fla_from(data, Some(fonts))
}

fn fla_from(data: &[u8], fonts: Option<&Fonts>) -> Result<(Document, Report), XflError> {
    if data.len() as u64 > MAX_FLA_BYTES {
        return Err(XflError::NotXfl("the file is too large".into()));
    }
    if data.starts_with(&OLE_MAGIC) {
        return Err(XflError::NotXfl("this .fla was saved by Flash CS4 or earlier, in a format AniCraft can't read: open it in Animate and save it again".into()));
    }
    let archive = crate::zip::Archive::open(data)?;
    let read = |p: &str| -> Result<Option<Vec<u8>>, XflError> { Ok(archive.read(p)?) };
    import_from(&read, fonts)
}

/// Whether the file at `path` starts as a zip archive or an old binary `.fla` does.
fn looks_like_fla(path: &std::path::Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 8];
    let n = std::fs::File::open(path).and_then(|mut f| f.read(&mut head)).unwrap_or(0);
    head.get(..n).is_some_and(|h| h.starts_with(b"PK\x03\x04") || h.starts_with(b"PK\x05\x06") || h.starts_with(&OLE_MAGIC))
}

/// Imports from disk: an XFL folder, its `.xfl` file (or any file in the folder, such as its
/// `DOMDocument.xml`), or a `.fla` archive (recognised by its contents too, so a renamed one
/// opens).
pub fn import_path(path: &std::path::Path) -> Result<(Document, Report), XflError> {
    let meta = std::fs::metadata(path)?;
    let is_fla = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("fla"));
    if meta.is_file() && (is_fla || looks_like_fla(path)) {
        if meta.len() > MAX_FLA_BYTES {
            return Err(XflError::NotXfl("the file is too large".into()));
        }
        return import_fla(&std::fs::read(path)?);
    }
    let dir = if meta.is_dir() { path.to_path_buf() } else { path.parent().map(std::path::Path::to_path_buf).unwrap_or_default() };
    let read = |p: &str| -> Result<Option<Vec<u8>>, XflError> {
        // Only plain relative paths inside the package.
        if p.split('/').any(|c| c == ".." || c.is_empty()) || p.contains(':') {
            return Ok(None);
        }
        let full = dir.join(p);
        match std::fs::read(&full) {
            Ok(b) if b.len() <= xml::MAX_INPUT => Ok(Some(b)),
            Ok(_) => Err(XflError::NotXfl(format!("{p} is too large"))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    };
    import_with(&read)
}
