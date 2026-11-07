//! Timeline evaluation: `(document, frame) -> DisplayList`. Pure, deterministic, no caching.

use crate::{ColorEffect, Document, Element, ElementId, ElementKind, Layer, LayerId, LayerKind, Paint, Rgba, Shape, Stroke, SymbolId, SymbolKind, Timeline, TweenKind};
use anicraft_geom::walk::PathWalk;
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
    /// Guide artwork: shown while editing, never in output.
    pub guide: bool,
    /// The innermost group this item fades with (see [`DisplayList::groups`]); `color.alpha`
    /// leaves out the groups' opacity.
    pub group: Option<u32>,
}

/// Back-to-front list of shapes.
#[derive(Clone, Debug, Default)]
pub struct DisplayList {
    pub items: Vec<DisplayItem>,
    /// One entry per clip: the enclosing clip it is intersected with. A parent always has a
    /// lower index than its children. A clip with no mask artwork hides everything in it.
    pub clip_parents: Vec<Option<u32>>,
    /// Groups that fade as one: a symbol instance with an opacity below 1 composites its
    /// items together, then blends the result at `alpha`, so overlapping parts don't show
    /// through each other. A group's items are one unbroken run of `items`, nested runs for
    /// nested groups; a parent always has a lower index.
    pub groups: Vec<Group>,
}

/// A group that fades as one (see [`DisplayList::groups`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Group {
    pub parent: Option<u32>,
    pub alpha: f32,
}

impl DisplayList {
    /// The opacity an item ends up with: its own times every group's it is in (what an item
    /// looks like when nothing overlaps it; also what picking and outlines use).
    pub fn effective_alpha(&self, item: &DisplayItem) -> f32 {
        let mut a = item.color.alpha;
        let mut g = item.group;
        for _ in 0..self.groups.len() {
            let Some(id) = g else { break };
            let Some(grp) = self.groups.get(id as usize) else { break };
            a *= grp.alpha;
            g = grp.parent.filter(|&p| p < id);
        }
        a
    }

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

    /// Appends `other`'s items (drawn on top), renumbering its clips and groups.
    pub fn append(&mut self, other: DisplayList) {
        let base = self.clip_parents.len() as u32;
        let shift = |c: Option<u32>| c.map(|c| c + base);
        let gbase = self.groups.len() as u32;
        let gshift = |g: Option<u32>| g.map(|g| g + gbase);
        self.clip_parents.extend(other.clip_parents.into_iter().map(shift));
        self.groups.extend(other.groups.into_iter().map(|g| Group { parent: gshift(g.parent), alpha: g.alpha }));
        self.items.extend(other.items.into_iter().map(|mut i| {
            i.clip = shift(i.clip);
            i.masks = shift(i.masks);
            i.group = gshift(i.group);
            i
        }));
    }
}

/// Evaluates movie frame `frame` (0-based): the scenes play one after the other.
pub fn evaluate(doc: &Document, frame: u32) -> DisplayList {
    let (scene, local) = doc.locate(frame);
    evaluate_scene(doc, scene, local)
}

/// Evaluates scene `scene`'s timeline at its own `frame` (what the stage shows while editing).
pub fn evaluate_scene(doc: &Document, scene: usize, frame: u32) -> DisplayList {
    let mut out = DisplayList::default();
    let cx = Ctx { xf: Affine::IDENTITY, color: ColorEffect::default(), depth: 0, root: None, outline: None, clip: None, masks: None, guide: false, group: None };
    eval_timeline(doc, doc.timeline(scene), frame, &cx, &mut out);
    out
}

/// Elements of `layer` as they appear at `frame`, with tweens applied, plus the start frame of
/// the covering keyframe. `None` when the frame is empty on this layer.
pub fn evaluate_layer_elements(layer: &Layer, frame: u32) -> Option<(u32, Cow<'_, [Arc<Element>]>)> {
    evaluate_guided(layer, frame, None)
}

/// [`evaluate_layer_elements`] with a motion guide: tweened elements travel along `guide` (in
/// the layer's own space) from the point nearest their start to the point nearest their end,
/// turning with it when the tween orients to the path.
pub fn evaluate_guided<'a>(layer: &'a Layer, frame: u32, guide: Option<&PathWalk>) -> Option<(u32, Cow<'a, [Arc<Element>]>)> {
    let idx = layer.keyframe_index_at(frame)?;
    let kf = layer.keyframes.get(idx)?;
    let Some(tween) = kf.tween else { return Some((kf.start, Cow::Borrowed(&kf.elements))) };
    let Some(next) = layer.keyframes.get(idx + 1) else { return Some((kf.start, Cow::Borrowed(&kf.elements))) };
    if next.start != kf.end() || frame == kf.start {
        return Some((kf.start, Cow::Borrowed(&kf.elements)));
    }
    let t = tween.easing.apply((frame - kf.start) as f32 / kf.duration.max(1) as f32);
    let morph = tween.kind == TweenKind::Shape;
    let path = guide.map(|g| (g, tween.orient));
    let els = kf.elements.iter().enumerate().map(|(i, a)| next.elements.get(i).and_then(|b| interpolate(a, b, t, morph, path)).map_or_else(|| a.clone(), Arc::new)).collect();
    Some((kf.start, Cow::Owned(els)))
}

/// Interpolates two elements; `None` when they cannot tween. With `morph` (a shape tween),
/// shapes of different structure blend through [`anicraft_geom::morph`]; otherwise a shape
/// changes only when both keyframes share its structure.
fn interpolate(a: &Element, b: &Element, t: f32, morph: bool, guide: Option<(&PathWalk, bool)>) -> Option<Element> {
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
    let mut transform = a.transform.decompose().lerp(&b.transform.decompose(), t).compose();
    if let Some((g, orient)) = guide {
        transform = follow(g, orient, &a.transform, &b.transform, t, transform);
    }
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
    /// Set while evaluating guide artwork.
    guide: bool,
    /// The group everything inside fades with.
    group: Option<u32>,
}

/// The first frame of the unbroken run of keyframes on `layer`, up to `frame`, that hold
/// element `id` as an instance of `symbol`: where a movie clip instance started playing.
fn instance_start(layer: &Layer, frame: u32, id: ElementId, symbol: SymbolId) -> u32 {
    let Some(mut i) = layer.keyframe_index_at(frame) else { return frame };
    let holds = |k: &crate::Keyframe| k.elements.iter().any(|e| e.id == id && matches!(e.kind, ElementKind::Symbol { symbol: s, .. } if s == symbol));
    let mut start = layer.keyframes.get(i).map_or(frame, |k| k.start);
    while let Some(prev) = i.checked_sub(1).and_then(|p| layer.keyframes.get(p)) {
        if prev.end() != start || !holds(prev) {
            break;
        }
        start = prev.start;
        i -= 1;
    }
    start
}

/// The path a guide layer's first shape traces at `frame`, in the layer's own space.
fn guide_walk(guide: &Layer, frame: u32) -> Option<PathWalk> {
    let (_, els) = evaluate_layer_elements(guide, frame)?;
    els.iter().find_map(|e| match &e.kind {
        ElementKind::Shape(s) => {
            let line = s.path.flatten(&e.transform, 0.25).into_iter().next()?;
            let mut pts = line.points;
            if line.closed
                && let Some(&first) = pts.first()
            {
                pts.push(first);
            }
            PathWalk::new(&pts)
        }
        ElementKind::Symbol { .. } => None,
    })
}

/// The tweened transform `tw` moved onto the guide: its origin travels from the point on the
/// guide nearest the start keyframe's origin to the point nearest the end's; with `orient`, it
/// also turns as the path turns.
fn follow(g: &PathWalk, orient: bool, from: &Affine, to: &Affine, t: f32, tw: Affine) -> Affine {
    let (s0, s1) = (g.project(from.translation()), g.project(to.translation()));
    let s = s0 + (s1 - s0) * t;
    let p = g.at(s);
    let [a, b, c, d, _, _] = tw.m;
    let mut lin = Affine { m: [a, b, c, d, 0.0, 0.0] };
    if orient {
        let (d0, d1) = (g.tangent(s0), g.tangent(s));
        lin = Affine::rotate(d1.y.atan2(d1.x) - d0.y.atan2(d0.x)).mul(&lin);
    }
    let [a, b, c, d, _, _] = lin.m;
    let out = Affine { m: [a, b, c, d, p.x, p.y] };
    if out.is_finite() { out } else { tw }
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
    // Folders draw nothing, and a hidden folder hides everything in it.
    let has_folders = tl.layers.iter().any(|l| l.kind == LayerKind::Folder);
    for (index, layer) in tl.layers.iter().enumerate().rev() {
        // Guide artwork is listed (marked, for editing) but never drawn; inside mask artwork
        // it plays no part at all.
        let is_guide = layer.kind == LayerKind::Guide;
        if !layer.visible || layer.kind == LayerKind::Folder || (is_guide && cx.masks.is_some()) || (has_folders && tl.hidden_by_folder(index)) {
            continue;
        }
        let (clip, masks) = match (cx.masks, layer.kind) {
            (Some(_), _) => (cx.clip, cx.masks),
            (None, LayerKind::Mask) => (cx.clip, clip_of(layer.id)),
            (None, LayerKind::Guide) => (cx.clip, None),
            (None, _) => (layer.parent.and_then(clip_of).or(cx.clip), None),
        };
        let guide = cx.guide || is_guide;
        // A layer guided by a guide layer: its tweens follow the guide's first path.
        let walk = layer.parent.and_then(|p| tl.layer(p)).filter(|g| g.kind == LayerKind::Guide).and_then(|g| guide_walk(g, frame));
        let Some((kf_start, elements)) = evaluate_guided(layer, frame, walk.as_ref()) else { continue };
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
                    guide,
                    group: cx.group,
                }),
                ElementKind::Symbol { symbol, loop_mode, first_frame } => {
                    let Some(sym) = doc.symbol(*symbol) else { continue };
                    let len = sym.timeline.frame_count();
                    let local = match sym.kind {
                        // A graphic keeps step with this timeline: loop mode and first frame apply.
                        SymbolKind::Graphic => loop_mode.local_frame(frame.saturating_sub(kf_start), *first_frame, len),
                        // A movie clip plays its own timeline on a loop from when the instance
                        // appeared, carrying on across keyframes that hold the same instance.
                        SymbolKind::MovieClip => frame.saturating_sub(instance_start(layer, frame, el.id, *symbol)) % len.max(1),
                        // A button shows its Up frame (states come with interactivity).
                        SymbolKind::Button => 0,
                    };
                    // An instance below full opacity fades as one: its contents composite in a
                    // group at full opacity, and the group blends at the instance's alpha. (Mask
                    // artwork ignores opacity, so it never needs one.)
                    let fade = el.color.alpha.clamp(0.0, 1.0);
                    let (color, group) = if masks.is_none() && fade < 1.0 - 1e-4 {
                        let gid = out.groups.len() as u32;
                        out.groups.push(Group { parent: cx.group, alpha: fade });
                        (cx.color.concat(&ColorEffect { alpha: 1.0, ..el.color }), Some(gid))
                    } else {
                        (color, cx.group)
                    };
                    let inner = Ctx { xf, color, depth: cx.depth + 1, root: Some((root_layer, root_el)), outline: layer_outline, clip, masks, guide, group };
                    eval_timeline(doc, &sym.timeline, local, &inner, out);
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "eval_tests.rs"]
mod tests;
