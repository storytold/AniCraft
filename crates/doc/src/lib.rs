//! AniCraft document model (layer L1).
//!
//! A document is plain data. Time is a function: [`evaluate`] turns a document and a frame into
//! a flat [`DisplayList`], which renderers draw. Nothing here mutates during playback.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod easing;
mod eval;
pub mod path_table;
pub mod sample;

pub use easing::Easing;
pub use eval::{DisplayItem, DisplayList, MAX_SYMBOL_DEPTH, evaluate, evaluate_layer_elements};

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
}

impl Paint {
    /// A representative colour (first stop for gradients) for swatches and the properties panel.
    pub fn primary_color(&self) -> Rgba {
        match self {
            Paint::Solid(c) => *c,
            Paint::Linear { stops, .. } | Paint::Radial { stops, .. } => stops.first().map_or(Rgba::BLACK, |s| s.color),
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

/// Stroke style. M0: round joins and caps.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    pub color: Rgba,
    pub width: f32,
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
}

impl PartialEq for Shape {
    fn eq(&self, o: &Shape) -> bool {
        arc_eq(&self.path, &o.path) && self.fill == o.fill && self.stroke == o.stroke && self.fill_rule == o.fill_rule
    }
}

impl Shape {
    pub fn filled(path: Path, fill: Paint) -> Shape {
        Shape { path: Arc::new(path), fill: Some(fill), stroke: None, fill_rule: FillRule::NonZero }
    }
    pub fn with_stroke(mut self, color: Rgba, width: f32) -> Shape {
        self.stroke = Some(Stroke { color, width });
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
    },
}

/// Colour effect applied to an element and everything inside it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ColorEffect {
    /// Alpha multiplier `0..=1`.
    pub alpha: f32,
    /// Tint colour mixed in by `tint_amount`.
    pub tint: Rgba,
    pub tint_amount: f32,
}

impl Default for ColorEffect {
    fn default() -> Self {
        ColorEffect { alpha: 1.0, tint: Rgba::WHITE, tint_amount: 0.0 }
    }
}

impl ColorEffect {
    /// Effect of `inner` nested inside `self`.
    pub fn concat(&self, inner: &ColorEffect) -> ColorEffect {
        // Tints compose by keeping the stronger one; alpha multiplies.
        let (tint, amount) = if inner.tint_amount >= self.tint_amount { (inner.tint, inner.tint_amount) } else { (self.tint, self.tint_amount) };
        ColorEffect { alpha: self.alpha * inner.alpha, tint, tint_amount: amount }
    }
    pub fn lerp(&self, o: &ColorEffect, t: f32) -> ColorEffect {
        ColorEffect {
            alpha: anicraft_geom::lerp(self.alpha, o.alpha, t),
            tint: self.tint.lerp(o.tint, t),
            tint_amount: anicraft_geom::lerp(self.tint_amount, o.tint_amount, t),
        }
    }
    pub fn apply(&self, c: Rgba) -> Rgba {
        let k = self.tint_amount.clamp(0.0, 1.0);
        let mut out = c.lerp(self.tint.with_alpha(c.a), k);
        out.a = c.a * self.alpha.clamp(0.0, 1.0);
        out
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
            let pad = s.stroke.map_or(0.0, |st| st.width * 0.5);
            s.path.bounds().inflate(pad).transformed(xf)
        }
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
    #[serde(default)]
    pub label: String,
}

impl PartialEq for Keyframe {
    fn eq(&self, o: &Keyframe) -> bool {
        self.start == o.start && self.duration == o.duration && self.tween == o.tween && self.label == o.label && arcs_eq(&self.elements, &o.elements)
    }
}

impl Keyframe {
    pub fn new(start: u32, duration: u32) -> Self {
        Keyframe { start, duration: duration.max(1), elements: Vec::new(), tween: None, label: String::new() }
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
    /// Authoring aid; never rendered in output.
    Guide,
    /// Its fills are not drawn: they reveal the layers masked by it (see [`Layer::parent`]).
    Mask,
}

/// A timeline layer. Keyframes are `Arc`-shared (see [`Keyframe`]).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Layer {
    pub id: LayerId,
    pub name: String,
    #[serde(default)]
    pub kind: LayerKind,
    /// The mask layer this layer is masked by. Masked layers sit directly under their mask, in
    /// one unbroken run ([`Timeline::repair_masks`] keeps it so); any other link is dropped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<LayerId>,
    #[serde(default = "yes")]
    pub visible: bool,
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub outline: bool,
    #[serde(default = "default_layer_color")]
    pub color: Rgba,
    #[serde(default)]
    pub keyframes: Vec<Arc<Keyframe>>,
}

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
            keyframes: vec![Arc::new(Keyframe::new(0, 1))],
        }
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

    /// The mask that a layer placed at `index` would join: the mask directly above it, or the
    /// mask of the masked layer directly above it.
    pub fn mask_above(&self, index: usize) -> Option<LayerId> {
        let above = self.layers.get(index.checked_sub(1)?)?;
        match above.kind {
            LayerKind::Mask => Some(above.id),
            _ => above.parent,
        }
    }

    /// Indices `i..j` of the layer at `i` plus, for a mask layer, the masked layers under it.
    pub fn group_range(&self, i: usize) -> std::ops::Range<usize> {
        let Some(l) = self.layers.get(i) else { return i..i };
        let mut j = i + 1;
        if l.kind == LayerKind::Mask {
            while self.layers.get(j).is_some_and(|m| m.parent == Some(l.id)) {
                j += 1;
            }
        }
        i..j
    }

    /// Drops mask links that no longer hold: a masked layer must be a normal layer directly
    /// under its mask layer or under another layer masked by it. Touches only layers that change.
    pub fn repair_masks(&mut self) {
        for i in 0..self.layers.len() {
            let Some(l) = self.layers.get(i) else { continue };
            let Some(p) = l.parent else { continue };
            // (Layers above are already repaired, so `mask_above` sees valid links.)
            let holds = l.kind == LayerKind::Normal && self.mask_above(i) == Some(p);
            if !holds && let Some(l) = self.layers.get_mut(i)
            {
                Arc::make_mut(l).parent = None;
            }
        }
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
    pub timeline: Timeline,
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

/// Last addressable frame (0-based); about 70 minutes at 24 fps.
pub const MAX_FRAME: u32 = 100_000;

/// Drops keyframes past [`MAX_FRAME`] and trims durations to it (files skip command checks).
fn clamp_frames(l: &mut Layer) {
    l.keyframes.retain(|k| k.start <= MAX_FRAME);
    for k in &mut l.keyframes {
        if k.end() > MAX_FRAME + 1 {
            Arc::make_mut(k).duration = MAX_FRAME + 1 - k.start;
        }
    }
}

/// A whole AniCraft document: a persistent (structurally shared) tree.
///
/// `clone()` is O(layers + symbols) pointer copies, which makes undo snapshots and render-thread
/// hand-off cheap, and `==` costs O(changed) because shared subtrees compare by pointer.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Document {
    #[serde(default)]
    pub stage: Stage,
    #[serde(default)]
    pub timeline: Timeline,
    #[serde(default)]
    pub library: Vec<Arc<Symbol>>,
    /// Next free id for layers, elements and symbols.
    #[serde(default = "first_id")]
    pub next_id: u64,
}

fn first_id() -> u64 {
    1
}

impl PartialEq for Document {
    fn eq(&self, o: &Document) -> bool {
        self.stage == o.stage && self.next_id == o.next_id && self.timeline == o.timeline && arcs_eq(&self.library, &o.library)
    }
}

impl Default for Document {
    fn default() -> Self {
        let mut d = Document { stage: Stage::default(), timeline: Timeline::default(), library: Vec::new(), next_id: 1 };
        let id = LayerId(d.alloc_id());
        d.timeline.layers.push(Arc::new(Layer::new(id, "Layer 1")));
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
    pub fn symbol(&self, id: SymbolId) -> Option<&Symbol> {
        self.library.iter().find(|s| s.id == id).map(|s| &**s)
    }
    pub fn frame_count(&self) -> u32 {
        self.timeline.frame_count()
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
            for k in &l.keyframes {
                for e in &k.elements {
                    *max_id = (*max_id).max(e.id.0);
                }
            }
        };
        for l in &mut self.timeline.layers {
            clamp_frames(Arc::make_mut(l));
            Arc::make_mut(l).normalize();
            scan(l, &mut max_id);
        }
        self.timeline.repair_masks();
        for sym in &mut self.library {
            max_id = max_id.max(sym.id.0);
            let tl = &mut Arc::make_mut(sym).timeline;
            for l in &mut tl.layers {
                clamp_frames(Arc::make_mut(l));
                Arc::make_mut(l).normalize();
                scan(l, &mut max_id);
            }
            tl.repair_masks();
        }
        self.next_id = self.next_id.max(max_id.saturating_add(1));
    }
}

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
        assert!(a.timeline.layers.iter().zip(&b.timeline.layers).all(|(x, y)| Arc::ptr_eq(x, y)));
        // Edit one element on the top layer.
        let layer = Arc::make_mut(&mut b.timeline.layers[0]);
        let kf = layer.keyframe_mut(1).unwrap();
        Arc::make_mut(&mut kf.elements[0]).name = "edited".into();
        assert!(!Arc::ptr_eq(&a.timeline.layers[0], &b.timeline.layers[0]));
        // siblings stay shared
        assert!(Arc::ptr_eq(&a.timeline.layers[0].keyframes[0], &b.timeline.layers[0].keyframes[0]));
        assert!(a.timeline.layers[1..].iter().zip(&b.timeline.layers[1..]).all(|(x, y)| Arc::ptr_eq(x, y)));
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
        let layer = Arc::make_mut(&mut b.timeline.layers[0]);
        layer.normalize();
        assert!(a.timeline.layers[0].keyframes.iter().zip(&layer.keyframes).all(|(x, y)| Arc::ptr_eq(x, y)));
    }

    #[test]
    fn sanitize_repairs_hostile_document() {
        let mut d = Document::default();
        d.stage.width = 0;
        d.stage.fps = f32::NAN;
        d.next_id = 0;
        Arc::make_mut(&mut d.timeline.layers[0]).keyframes = [Keyframe::new(0, u32::MAX), Keyframe::new(u32::MAX - 1, 5)].map(Arc::new).to_vec();
        d.sanitize();
        assert_eq!(d.frame_count(), MAX_FRAME + 1, "frames past the limit are dropped or trimmed");
        let keys = &d.timeline.layers[0].keyframes;
        assert_eq!(keys.len(), 1);
        assert_eq!((keys[0].start, keys[0].duration), (0, MAX_FRAME + 1));
        assert_eq!(d.stage.width, 1);
        assert_eq!(d.stage.fps, 24.0);
        assert!(d.next_id > d.timeline.layers[0].id.0);
    }

    #[test]
    fn sanitize_clamps_main_and_symbol_timelines_at_the_inclusive_limit() {
        let cases = [
            (vec![Keyframe::new(MAX_FRAME, 10), Keyframe::new(MAX_FRAME + 1, 1)], vec![(MAX_FRAME, 1)]),
            (vec![Keyframe::new(0, MAX_FRAME + 1)], vec![(0, MAX_FRAME + 1)]),
            (vec![Keyframe::new(0, u32::MAX), Keyframe::new(u32::MAX - 1, 5)], vec![(0, MAX_FRAME + 1)]),
        ];
        for (keys, expected) in cases {
            let mut d = Document::default();
            Arc::make_mut(&mut d.timeline.layers[0]).keyframes = keys.into_iter().map(Arc::new).collect();
            d.library.push(Arc::new(Symbol {
                id: SymbolId(100), name: "boundary".into(), kind: SymbolKind::Graphic, timeline: d.timeline.clone(),
            }));
            d.sanitize();
            for timeline in [&d.timeline, &d.library[0].timeline] {
                let actual: Vec<_> = timeline.layers[0].keyframes.iter().map(|k| (k.start, k.duration)).collect();
                assert_eq!(actual, expected);
                assert_eq!(timeline.frame_count(), MAX_FRAME + 1);
            }
        }
    }
}
