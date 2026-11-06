//! XFL document → AniCraft document.
//!
//! What maps (first slice of M4): stage size, frame rate and colour; the first scene's
//! layers (name, colour, visibility, lock, outline; guide layers; folders flattened); frames
//! and keyframes with durations, labels and classic tweens with their ease; shapes (solid,
//! linear and radial fills, solid strokes), groups (flattened), rectangle and oval
//! primitives; symbol instances with their matrix, alpha and tint, loop mode and first frame;
//! library symbols (graphic, movie clip, button) with their own timelines, nested to any
//! depth. Everything else (bitmaps, text, sound, masks, motion tweens with property curves,
//! filters) is counted in the report instead of guessed at.

use crate::edges::{self, Edge};
use crate::xml::{self, Element as X};
use anicraft_doc::{
    ColorEffect, Document, Easing, Element, ElementId, ElementKind, GradientStop, Keyframe, Layer, LayerId, LayerKind, LoopMode, Paint, Rgba, Shape, Stage, Stroke, Symbol, SymbolId,
    SymbolKind, Timeline, Tween, TweenKind,
};
use anicraft_geom::{Affine, FillRule, Path, Vec2};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

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

/// What the import couldn't bring across, by kind ("3 bitmaps").
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Report {
    pub skipped: BTreeMap<String, usize>,
    pub notes: Vec<String>,
}

impl Report {
    fn skip(&mut self, what: &str) {
        *self.skipped.entry(what.to_string()).or_default() += 1;
    }
    /// One line for a status bar: "Imported. Not yet supported: 2 bitmaps, 1 text field."
    pub fn summary(&self) -> String {
        if self.skipped.is_empty() && self.notes.is_empty() {
            return "Imported everything in the file.".into();
        }
        let mut parts: Vec<String> = self.skipped.iter().map(|(k, n)| format!("{n} {k}")).collect();
        parts.extend(self.notes.iter().cloned());
        format!("Imported. Not yet supported: {}.", parts.join(", "))
    }
}

/// Reads files of the XFL package by their path inside it (`DOMDocument.xml`, `LIBRARY/x.xml`).
pub type Reader<'a> = dyn Fn(&str) -> Result<Option<Vec<u8>>, XflError> + 'a;

const MAX_SYMBOLS: usize = 20_000;
const MAX_LAYERS: usize = 5_000;
const MAX_FRAME: u32 = 100_000;

struct Builder<'a> {
    doc: Document,
    read: &'a Reader<'a>,
    /// Library item name → symbol id (allocated before timelines are built, so instances can
    /// refer to any symbol).
    ids: HashMap<String, SymbolId>,
    /// Parsed symbol files waiting to be built.
    pending: Vec<(SymbolId, String, SymbolKind, X)>,
    report: Report,
}

fn color(el: &X, key: &str, alpha_key: &str) -> Rgba {
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

/// A fill definition (`SolidColor`, `LinearGradient`, `RadialGradient`) as a paint.
fn paint(def: &X, report: &mut Report) -> Option<Paint> {
    match def.name.as_str() {
        "SolidColor" => Some(Paint::Solid(color(def, "color", "alpha"))),
        "LinearGradient" | "RadialGradient" => {
            let m = matrix(def);
            let stops: Vec<GradientStop> = def.children.iter().filter(|c| c.name == "GradientEntry").map(|g| GradientStop { offset: g.f32("ratio").unwrap_or(0.0).clamp(0.0, 1.0), color: color(g, "color", "alpha") }).collect();
            if stops.is_empty() {
                return None;
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
            report.skip("bitmap fills");
            None
        }
        _ => None,
    }
}

/// A stroke definition (`SolidStroke` and friends) as a stroke: weight and its fill colour.
fn stroke(def: &X, report: &mut Report) -> Option<Stroke> {
    if !matches!(def.name.as_str(), "SolidStroke" | "DashedStroke" | "DottedStroke" | "RaggedStroke" | "StippleStroke" | "HatchedStroke") {
        return None;
    }
    if def.name != "SolidStroke" {
        report.skip("stroke styles drawn as solid");
    }
    let width = def.f32("weight").unwrap_or(1.0).clamp(0.05, 1000.0);
    let fill = def.child("fill").and_then(|f| f.children.first());
    let c = fill.map_or(Rgba::BLACK, |f| if f.name == "SolidColor" { color(f, "color", "alpha") } else { paint(f, report).map_or(Rgba::BLACK, |p| p.primary_color()) });
    Some(Stroke { color: c, width })
}

impl<'a> Builder<'a> {
    fn element(&mut self, kind: ElementKind, transform: Affine, color: ColorEffect) -> Arc<Element> {
        Arc::new(Element { id: ElementId(self.doc.alloc_id()), name: String::new(), transform, color, kind })
    }

    /// Shapes for one `DOMShape`: a fill-only shape per fill style (bottom), then a stroke-only
    /// shape per stroke style, as Animate draws them.
    fn shape(&mut self, el: &X, parent: &Affine, out: &mut Vec<Arc<Element>>) {
        let m = parent.mul(&matrix(el));
        let fills: Vec<(u32, Option<Paint>)> = el.list("fills").iter().map(|f| (f.u32("index").unwrap_or(0), f.children.first().and_then(|d| paint(d, &mut self.report)))).collect();
        let strokes: Vec<(u32, Option<Stroke>)> = el.list("strokes").iter().map(|s| (s.u32("index").unwrap_or(0), s.children.first().and_then(|d| stroke(d, &mut self.report)))).collect();
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
                out.push(self.element(ElementKind::Shape(Shape { path: Arc::new(path), fill: Some(p), stroke: None, fill_rule: FillRule::EvenOdd }), m, ColorEffect::default()));
            }
        }
        for (index, s) in strokes {
            let (Some(s), true) = (s, index > 0) else { continue };
            let path = edges::stroke_path(&edges, index);
            if !path.is_empty() {
                out.push(self.element(ElementKind::Shape(Shape { path: Arc::new(path), fill: None, stroke: Some(s), fill_rule: FillRule::NonZero }), m, ColorEffect::default()));
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
        let fill = el.child("fill").and_then(|f| f.children.first()).and_then(|d| paint(d, &mut self.report));
        let stroke = el.child("stroke").and_then(|s| s.children.first()).and_then(|d| stroke(d, &mut self.report));
        if fill.is_some() || stroke.is_some() {
            let m = parent.mul(&matrix(el));
            out.push(self.element(ElementKind::Shape(Shape { path: Arc::new(path), fill, stroke, fill_rule: FillRule::NonZero }), m, ColorEffect::default()));
        }
    }

    fn instance(&mut self, el: &X, parent: &Affine, out: &mut Vec<Arc<Element>>) {
        let Some(&symbol) = el.attr("libraryItemName").and_then(|n| self.ids.get(n)) else {
            self.report.skip("instances of missing symbols");
            return;
        };
        let loop_mode = match el.attr("loop") {
            Some("play once") => LoopMode::PlayOnce,
            Some("single frame") => LoopMode::SingleFrame,
            _ => LoopMode::Loop,
        };
        let first_frame = el.u32("firstFrame").unwrap_or(0).min(MAX_FRAME);
        let mut fx = ColorEffect::default();
        if let Some(c) = el.child("color").and_then(|c| c.child("Color")) {
            fx.alpha = c.f32("alphaMultiplier").unwrap_or(1.0).clamp(0.0, 1.0);
            if let Some(t) = c.f32("tintMultiplier") {
                fx.tint = color(c, "tintColor", "__none");
                fx.tint_amount = t.clamp(0.0, 1.0);
            }
        }
        let m = parent.mul(&matrix(el));
        out.push(self.element(ElementKind::Symbol { symbol, loop_mode, first_frame }, m, fx));
    }

    fn elements(&mut self, list: &[X], parent: &Affine, out: &mut Vec<Arc<Element>>, depth: usize) {
        if depth > 64 {
            return;
        }
        for el in list {
            match el.name.as_str() {
                "DOMShape" => self.shape(el, parent, out),
                "DOMGroup" => {
                    let m = parent.mul(&matrix(el));
                    self.elements(el.list("members"), &m, out, depth + 1);
                }
                "DOMSymbolInstance" => self.instance(el, parent, out),
                "DOMRectangleObject" | "DOMOvalObject" => self.primitive(el, parent, out),
                "DOMBitmapInstance" => self.report.skip("bitmaps"),
                "DOMStaticText" | "DOMDynamicText" | "DOMInputText" => self.report.skip("text fields"),
                "DOMVideoInstance" => self.report.skip("videos"),
                _ => self.report.skip("other elements"),
            }
        }
    }

    fn timeline(&mut self, tl: &X) -> Timeline {
        let mut layers = Vec::new();
        // Mask layers by their position in the file's layer list (`parentLayerIndex` points at
        // a layer's folder or mask; folders are flattened away).
        let mut masks: Vec<Option<LayerId>> = Vec::new();
        for l in tl.list("layers").iter().filter(|l| l.name == "DOMLayer").take(MAX_LAYERS) {
            masks.push(None);
            let kind = match l.attr("layerType") {
                Some("folder") => continue,
                Some("guide") => LayerKind::Guide,
                Some("mask") => LayerKind::Mask,
                _ => LayerKind::Normal,
            };
            let id = LayerId(self.doc.alloc_id());
            let mut layer = Layer::new(id, l.attr("name").unwrap_or("Layer"));
            layer.kind = kind;
            if kind == LayerKind::Mask
                && let Some(slot) = masks.last_mut()
            {
                *slot = Some(id);
            }
            if kind == LayerKind::Normal {
                layer.parent = l.u32("parentLayerIndex").and_then(|i| masks.get(i as usize).copied().flatten());
            }
            layer.visible = l.bool("visible").unwrap_or(true);
            layer.locked = l.bool("locked").unwrap_or(false);
            layer.outline = l.bool("outline").unwrap_or(false);
            layer.color = color(l, "color", "__none");
            let mut keys = Vec::new();
            for f in l.list("frames").iter().filter(|f| f.name == "DOMFrame") {
                let start = f.u32("index").unwrap_or(0).min(MAX_FRAME);
                let duration = f.u32("duration").unwrap_or(1).clamp(1, MAX_FRAME);
                let mut k = Keyframe::new(start, duration);
                k.label = f.attr("name").unwrap_or("").chars().take(256).collect();
                match f.attr("tweenType") {
                    Some(t @ ("motion" | "shape")) => {
                        let acc = f.f32("acceleration").unwrap_or(0.0).clamp(-100.0, 100.0);
                        let kind = if t == "shape" { TweenKind::Shape } else { TweenKind::Classic };
                        k.tween = Some(Tween { easing: if acc == 0.0 { Easing::Linear } else { Easing::Classic(acc) }, kind });
                        if t == "shape" && f.child("shapeHints").is_some_and(|h| !h.children.is_empty()) {
                            self.report.skip("shape hints (the tween morphs without them)");
                        }
                    }
                    Some("motion object") => self.report.skip("motion tweens (kept as keyframes)"),
                    _ => {}
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
            let kind = match item.attr("symbolType") {
                Some("movie clip") => SymbolKind::MovieClip,
                Some("button") => SymbolKind::Button,
                _ => SymbolKind::Graphic,
            };
            let id = SymbolId(self.doc.alloc_id());
            self.ids.insert(name.clone(), id);
            self.pending.push((id, name, kind, item));
        }
        Ok(())
    }

    fn build_symbols(&mut self) {
        for (id, name, kind, item) in std::mem::take(&mut self.pending) {
            let timeline = match item.child("timeline").and_then(|t| t.child("DOMTimeline")) {
                Some(tl) => self.timeline(tl),
                None => Timeline { layers: vec![Arc::new(Layer::new(LayerId(self.doc.alloc_id()), "Layer 1"))] },
            };
            let short = name.rsplit('/').next().unwrap_or(&name).to_string();
            self.doc.library.push(Arc::new(Symbol { id, name: short, kind, timeline }));
        }
    }
}

/// Imports an XFL package through `read` (paths inside the package).
pub fn import_with(read: &Reader<'_>) -> Result<(Document, Report), XflError> {
    let bytes = read("DOMDocument.xml")?.ok_or_else(|| XflError::NotXfl("no DOMDocument.xml".into()))?;
    let root = xml::parse(&bytes)?;
    if root.name != "DOMDocument" {
        return Err(XflError::NotXfl(format!("the root element is <{}>", root.name)));
    }
    let mut b = Builder { doc: Document::default(), read, ids: HashMap::new(), pending: Vec::new(), report: Report::default() };
    b.doc.library.clear();
    b.doc.timeline.layers.clear();
    b.doc.stage = Stage {
        width: root.f32("width").unwrap_or(550.0).clamp(1.0, 8192.0) as u32,
        height: root.f32("height").unwrap_or(400.0).clamp(1.0, 8192.0) as u32,
        fps: root.f32("frameRate").unwrap_or(24.0).clamp(1.0, 120.0),
        background: color(&root, "backgroundColor", "__none"),
    };
    b.register_symbols(&root)?;
    b.build_symbols();
    let scenes: Vec<&X> = root.list("timelines").iter().filter(|t| t.name == "DOMTimeline").collect();
    if scenes.len() > 1 {
        b.report.notes.push(format!("scenes after the first ({} more)", scenes.len() - 1));
    }
    match scenes.first() {
        Some(tl) => b.doc.timeline = b.timeline(tl),
        None => b.doc.timeline = Timeline { layers: vec![Arc::new(Layer::new(LayerId(b.doc.alloc_id()), "Layer 1"))] },
    }
    Ok((b.doc, b.report))
}

/// Imports from disk: an XFL folder, its `.xfl` file, or a `.fla` archive.
pub fn import_path(path: &std::path::Path) -> Result<(Document, Report), XflError> {
    let meta = std::fs::metadata(path)?;
    let is_fla = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("fla"));
    if meta.is_file() && is_fla {
        if meta.len() as usize > crate::zip::MAX_ENTRY * 4 {
            return Err(XflError::NotXfl("the file is too large".into()));
        }
        let data = std::fs::read(path)?;
        let archive = crate::zip::Archive::open(&data)?;
        let read = |p: &str| -> Result<Option<Vec<u8>>, XflError> { Ok(archive.read(p)?) };
        return import_with(&read);
    }
    let dir = if meta.is_dir() { path.to_path_buf() } else { path.parent().map(std::path::Path::to_path_buf).unwrap_or_default() };
    let read = |p: &str| -> Result<Option<Vec<u8>>, XflError> {
        // Only plain relative paths inside the package.
        // Backslash too: Windows treats it as a separator, so `..\x` would leave the package.
        if p.split(['/', '\\']).any(|c| c == ".." || c.is_empty()) || p.contains(':') {
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
