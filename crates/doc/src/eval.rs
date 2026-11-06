//! Timeline evaluation: `(document, frame) -> DisplayList`. Pure, deterministic, no caching.

use crate::{ColorEffect, Document, Element, ElementId, ElementKind, Layer, LayerId, LayerKind, Paint, Rgba, Shape, Stroke, Timeline, TweenKind};
use anicraft_geom::{Affine, FillRule, Path};
use std::borrow::Cow;
use std::sync::Arc;

/// Symbols nested deeper than this are not drawn (guards cyclic libraries).
pub const MAX_SYMBOL_DEPTH: usize = 32;

/// One shape to draw, fully resolved to stage space.
#[derive(Clone, Debug)]
pub struct DisplayItem {
    pub path: Arc<Path>,
    /// Local → stage transform.
    pub transform: Affine,
    pub fill: Option<Paint>,
    pub stroke: Option<Stroke>,
    pub fill_rule: FillRule,
    pub color: ColorEffect,
    /// The top-level layer and element this item came from (for picking).
    pub layer: LayerId,
    pub element: ElementId,
    /// Outline-only display (layer outline mode).
    pub outline: Option<Rgba>,
    /// Drawn only inside this clip (an index into [`DisplayList::clip_parents`]).
    pub clip: Option<u32>,
    /// Mask artwork: not drawn; its fill area (strokes and opacity ignored) forms this clip.
    pub masks: Option<u32>,
}

/// Back-to-front list of shapes.
#[derive(Clone, Debug, Default)]
pub struct DisplayList {
    pub items: Vec<DisplayItem>,
    /// One entry per clip: the enclosing clip it is intersected with. A parent always has a
    /// lower index than its children. A clip with no mask artwork hides everything in it.
    pub clip_parents: Vec<Option<u32>>,
}

impl DisplayList {
    /// Whether stage point `p` lies inside clip `clip` (and every clip enclosing it): the test
    /// that keeps masked-away parts of a shape from being picked.
    pub fn clip_contains(&self, clip: u32, p: anicraft_geom::Vec2) -> bool {
        let mut c = Some(clip);
        for _ in 0..self.clip_parents.len() {
            let Some(id) = c else { return true };
            if !self.items.iter().any(|i| i.masks == Some(id) && i.fill.is_some() && i.path.contains(p, &i.transform, i.fill_rule)) {
                return false;
            }
            c = self.clip_parents.get(id as usize).copied().flatten().filter(|&q| q < id);
        }
        c.is_none()
    }

    /// Appends `other`'s items (drawn on top), renumbering its clips.
    pub fn append(&mut self, other: DisplayList) {
        let base = self.clip_parents.len() as u32;
        let shift = |c: Option<u32>| c.map(|c| c + base);
        self.clip_parents.extend(other.clip_parents.into_iter().map(shift));
        self.items.extend(other.items.into_iter().map(|mut i| {
            i.clip = shift(i.clip);
            i.masks = shift(i.masks);
            i
        }));
    }
}

/// Evaluates the main timeline at `frame` (0-based).
pub fn evaluate(doc: &Document, frame: u32) -> DisplayList {
    let mut out = DisplayList::default();
    let cx = Ctx { xf: Affine::IDENTITY, color: ColorEffect::default(), depth: 0, root: None, outline: None, clip: None, masks: None };
    eval_timeline(doc, &doc.timeline, frame, &cx, &mut out);
    out
}

/// Elements of `layer` as they appear at `frame`, with tweens applied, plus the start frame of
/// the covering keyframe. `None` when the frame is empty on this layer.
pub fn evaluate_layer_elements(layer: &Layer, frame: u32) -> Option<(u32, Cow<'_, [Arc<Element>]>)> {
    let idx = layer.keyframe_index_at(frame)?;
    let kf = layer.keyframes.get(idx)?;
    let Some(tween) = kf.tween else { return Some((kf.start, Cow::Borrowed(&kf.elements))) };
    let Some(next) = layer.keyframes.get(idx + 1) else { return Some((kf.start, Cow::Borrowed(&kf.elements))) };
    if next.start != kf.end() || frame == kf.start {
        return Some((kf.start, Cow::Borrowed(&kf.elements)));
    }
    let t = tween.easing.apply((frame - kf.start) as f32 / kf.duration.max(1) as f32);
    let morph = tween.kind == TweenKind::Shape;
    let els = kf.elements.iter().enumerate().map(|(i, a)| next.elements.get(i).and_then(|b| interpolate(a, b, t, morph)).map_or_else(|| a.clone(), Arc::new)).collect();
    Some((kf.start, Cow::Owned(els)))
}

/// Interpolates two elements; `None` when they cannot tween. With `morph` (a shape tween),
/// shapes of different structure blend through [`anicraft_geom::morph`]; otherwise a shape
/// changes only when both keyframes share its structure.
fn interpolate(a: &Element, b: &Element, t: f32, morph: bool) -> Option<Element> {
    let kind = match (&a.kind, &b.kind) {
        (ElementKind::Shape(sa), ElementKind::Shape(sb)) => {
            let path = if Arc::ptr_eq(&sa.path, &sb.path) {
                sa.path.clone()
            } else if morph {
                anicraft_geom::morph::morph(&sa.path, &sb.path, t).map(Arc::new).unwrap_or_else(|| sa.path.clone())
            } else {
                sa.path.lerp(&sb.path, t).map(Arc::new).unwrap_or_else(|| sa.path.clone())
            };
            let fill = match (&sa.fill, &sb.fill) {
                (Some(x), Some(y)) => Some(x.lerp(y, t)),
                (x, _) => x.clone(),
            };
            let stroke = match (sa.stroke, sb.stroke) {
                (Some(x), Some(y)) => Some(Stroke { color: x.color.lerp(y.color, t), width: anicraft_geom::lerp(x.width, y.width, t) }),
                (x, _) => x,
            };
            ElementKind::Shape(Shape { path, fill, stroke, fill_rule: sa.fill_rule })
        }
        (ElementKind::Symbol { symbol: s1, .. }, ElementKind::Symbol { symbol: s2, .. }) if s1 == s2 => a.kind.clone(),
        _ => return None,
    };
    let transform = a.transform.decompose().lerp(&b.transform.decompose(), t).compose();
    Some(Element { id: a.id, name: a.name.clone(), transform, color: a.color.lerp(&b.color, t), kind })
}

/// What a timeline is evaluated inside of.
#[derive(Clone, Copy)]
struct Ctx {
    /// Local → stage transform and colour effect of the enclosing instance.
    xf: Affine,
    color: ColorEffect,
    depth: usize,
    /// The top-level layer and element everything inside belongs to (for picking).
    root: Option<(LayerId, ElementId)>,
    outline: Option<Rgba>,
    /// The clip everything inside is drawn within.
    clip: Option<u32>,
    /// Set while evaluating mask artwork: everything inside forms this clip.
    masks: Option<u32>,
}

fn eval_timeline(doc: &Document, tl: &Timeline, frame: u32, cx: &Ctx, out: &mut DisplayList) {
    if cx.depth > MAX_SYMBOL_DEPTH {
        return;
    }
    // One clip per visible mask layer, nested in the enclosing clip. Inside mask artwork every
    // layer is artwork, so masks there do not mask again. A hidden mask layer masks nothing.
    let mut clips: Vec<(LayerId, u32)> = Vec::new();
    if cx.masks.is_none() {
        for layer in tl.layers.iter().filter(|l| l.kind == LayerKind::Mask && l.visible) {
            clips.push((layer.id, out.clip_parents.len() as u32));
            out.clip_parents.push(cx.clip);
        }
    }
    let clip_of = |id: LayerId| clips.iter().find(|(l, _)| *l == id).map(|(_, c)| *c);
    for layer in tl.layers.iter().rev() {
        if !layer.visible || layer.kind == LayerKind::Guide {
            continue;
        }
        let (clip, masks) = match (cx.masks, layer.kind) {
            (Some(_), _) => (cx.clip, cx.masks),
            (None, LayerKind::Mask) => (cx.clip, clip_of(layer.id)),
            (None, _) => (layer.parent.and_then(clip_of).or(cx.clip), None),
        };
        let Some((kf_start, elements)) = evaluate_layer_elements(layer, frame) else { continue };
        let layer_outline = cx.outline.or(if layer.outline { Some(layer.color) } else { None });
        for el in elements.iter() {
            let (root_layer, root_el) = cx.root.unwrap_or((layer.id, el.id));
            let xf = cx.xf.mul(&el.transform);
            let color = cx.color.concat(&el.color);
            match &el.kind {
                ElementKind::Shape(s) => out.items.push(DisplayItem {
                    path: s.path.clone(),
                    transform: xf,
                    fill: s.fill.clone(),
                    stroke: s.stroke,
                    fill_rule: s.fill_rule,
                    color,
                    layer: root_layer,
                    element: root_el,
                    outline: layer_outline,
                    clip,
                    masks,
                }),
                ElementKind::Symbol { symbol, loop_mode, first_frame } => {
                    let Some(sym) = doc.symbol(*symbol) else { continue };
                    let local = loop_mode.local_frame(frame.saturating_sub(kf_start), *first_frame, sym.timeline.frame_count());
                    let inner = Ctx { xf, color, depth: cx.depth + 1, root: Some((root_layer, root_el)), outline: layer_outline, clip, masks };
                    eval_timeline(doc, &sym.timeline, local, &inner, out);
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "eval_tests.rs"]
mod tests;
