//! Timeline evaluation: `(document, frame) -> DisplayList`. Pure, deterministic, no caching.

use crate::camera::Camera;
use crate::nine_slice::Slicing;
use crate::player::PlayerState;
use crate::{ColorTransform, Document, Element, ElementId, ElementKind, Layer, LayerId, LayerKind, Paint, Rgba, Shape, Stroke, SymbolId, SymbolKind, Timeline, TweenKind};
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
    /// The colour transform of every instance around it, composed (ADR 0056).
    pub color: ColorTransform,
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
    /// The innermost group this item fades with (see [`DisplayList::groups`]); `color`
    /// leaves out the groups' opacity.
    pub group: Option<u32>,
    /// The decoded pixels a [`Paint::Bitmap`] fill shows (shared, never copied).
    pub image: Option<Arc<crate::Pixels>>,
    /// Whether `image` is filtered between its pixels (the bitmap's Allow Smoothing, ADR 0050);
    /// off, each pixel is a hard-edged square.
    pub smooth: bool,
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
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub parent: Option<u32>,
    pub alpha: f32,
    /// How the group's composited pixels combine with what is below.
    pub blend: crate::BlendMode,
    /// Filters applied to the group's composited pixels, in order (empty for most groups).
    pub filters: Arc<[crate::Filter]>,
    /// The clip in force where the instance is: a filtered group is clipped after filtering.
    pub clip: Option<u32>,
}

impl DisplayList {
    /// The opacity an item ends up with: its own times every group's it is in (what an item
    /// looks like when nothing overlaps it; also what picking and outlines use).
    pub fn effective_alpha(&self, item: &DisplayItem) -> f32 {
        let mut a = item.color.opacity();
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
        self.groups.extend(other.groups.into_iter().map(|g| Group { parent: gshift(g.parent), alpha: g.alpha, blend: g.blend, filters: g.filters, clip: shift(g.clip) }));
        self.items.extend(other.items.into_iter().map(|mut i| {
            i.clip = shift(i.clip);
            i.masks = shift(i.masks);
            i.group = gshift(i.group);
            i
        }));
    }
}

/// Evaluates movie frame `frame` (0-based): the scenes play one after the other, each seen
/// through its camera (ADR 0027). This is what every export draws.
pub fn evaluate(doc: &Document, frame: u32) -> DisplayList {
    let (scene, local) = doc.locate(frame);
    evaluate_scene_camera(doc, scene, local)
}

/// Evaluates scene `scene`'s timeline at its own `frame` in stage space, the camera not
/// applied: what the stage shows while editing (and what picking and bounds use).
pub fn evaluate_scene(doc: &Document, scene: usize, frame: u32) -> DisplayList {
    let mut out = DisplayList::default();
    eval_timeline(doc, doc.timeline(scene), frame, &Ctx::top(Affine::IDENTITY), &mut out, None);
    out
}

/// A symbol's timeline at its own `frame`, in the symbol's space, as the stage edits it while the
/// symbol is open for editing (ADR 0045): what picking and bounds use then. Its layers are the
/// top-level ones, as a scene's are in [`evaluate_scene`].
pub fn evaluate_timeline(doc: &Document, tl: &Timeline, frame: u32) -> DisplayList {
    let mut out = DisplayList::default();
    eval_timeline(doc, tl, frame, &Ctx::top(Affine::IDENTITY), &mut out, None);
    out
}

/// [`evaluate_timeline`] placed by `xf` (symbol space to stage space): what an instance of the
/// timeline at `xf` shows, but with the timeline's own layers as the top-level ones, so each item
/// still says which of them it is on. The stage draws a symbol open in place this way when onion
/// skin or Edit Multiple Frames composes its frames (ADR 0051).
pub fn evaluate_timeline_at(doc: &Document, tl: &Timeline, frame: u32, xf: Affine) -> DisplayList {
    let mut out = DisplayList::default();
    eval_timeline(doc, tl, frame, &Ctx::top(xf), &mut out, None);
    out
}

/// Scene `scene` at its own `frame` as the movie shows it: through the scene's camera, each
/// layer shifted by its depth, the camera's tint over everything (ADR 0027). Without a camera
/// this is [`evaluate_scene`].
pub fn evaluate_scene_camera(doc: &Document, scene: usize, frame: u32) -> DisplayList {
    let tl = doc.timeline(scene);
    let mut out = DisplayList::default();
    eval_timeline(doc, tl, frame, &Ctx::through(tl.camera_at(frame, &doc.stage)), &mut out, None);
    out
}

/// What the player shows (ADR 0024): the scene and frame its main timeline is on, each movie
/// clip at its own playhead, buttons in their pointer state, and properties scripts set, seen
/// through the scene's camera.
pub fn evaluate_player(doc: &Document, state: &PlayerState) -> DisplayList {
    let mut out = DisplayList::default();
    let play = Play { state, path: Vec::new() };
    let tl = doc.timeline(state.scene);
    eval_timeline(doc, tl, state.root.frame, &Ctx::through(tl.camera_at(state.root.frame, &doc.stage)), &mut out, Some(&play));
    out
}

/// A symbol's timeline at `frame`, drawn at `xf` (stage space): a button's hit area, or a movie
/// clip's artwork, as the player sees it when `play` is given.
pub(crate) fn evaluate_symbol(doc: &Document, tl: &Timeline, frame: u32, xf: Affine, play: Option<&Play<'_>>) -> DisplayList {
    let mut out = DisplayList::default();
    let mut cx = Ctx::top(xf);
    cx.depth = 1;
    cx.stage = false;
    eval_timeline(doc, tl, frame, &cx, &mut out, play);
    out
}

/// The player state an evaluation follows, and the instance path of the timeline being
/// evaluated (empty for the main timeline).
pub(crate) struct Play<'a> {
    pub state: &'a PlayerState,
    pub path: Vec<ElementId>,
}

impl<'a> Play<'a> {
    pub fn child(&self, id: ElementId) -> Play<'a> {
        let mut path = Vec::with_capacity(self.path.len() + 1);
        path.extend_from_slice(&self.path);
        path.push(id);
        Play { state: self.state, path }
    }
}

/// Elements of `layer` as they appear at `frame`, with tweens applied, plus the start frame of
/// the covering keyframe. `None` when the frame is empty on this layer.
pub fn evaluate_layer_elements(layer: &Layer, frame: u32) -> Option<(u32, Cow<'_, [Arc<Element>]>)> {
    evaluate_guided(layer, frame, None)
}

/// [`evaluate_layer_elements`] with a motion guide: tweened elements travel along `guide` (in
/// the layer's own space) from the point nearest their start to the point nearest their end,
/// turning with it when the tween orients to the path.
///
/// On an armature layer (ADR 0043) the instances are then placed by the pose at the frame
/// (tweened between pose keyframes): forward kinematics from the root's own transform.
pub fn evaluate_guided<'a>(layer: &'a Layer, frame: u32, guide: Option<&PathWalk>) -> Option<(u32, Cow<'a, [Arc<Element>]>)> {
    let (start, els) = evaluate_tweened(layer, frame, guide)?;
    let Some(arm) = layer.armature.as_deref().filter(|a| layer.kind == crate::LayerKind::Armature && !a.is_empty()) else { return Some((start, els)) };
    let pose = crate::armature::pose_at(layer, frame).unwrap_or_default();
    let mut els = els.into_owned();
    crate::armature::apply(arm, &mut els, &pose);
    Some((start, Cow::Owned(els)))
}

fn evaluate_tweened<'a>(layer: &'a Layer, frame: u32, guide: Option<&PathWalk>) -> Option<(u32, Cow<'a, [Arc<Element>]>)> {
    let idx = layer.keyframe_index_at(frame)?;
    let kf = layer.keyframes.get(idx)?;
    // A motion tween: its target follows the property curves (and its motion path, ADR 0030);
    // anything else on the key holds.
    if let Some(m) = &kf.motion {
        let Some(i) = m.target_index(&kf.elements).filter(|_| !m.tracks.is_empty() || m.path.is_some()) else { return Some((kf.start, Cow::Borrowed(&kf.elements))) };
        let mut els = kf.elements.clone();
        if let Some(slot) = els.get_mut(i) {
            *slot = Arc::new(m.apply(slot, (frame - kf.start) as f32));
        }
        return Some((kf.start, Cow::Owned(els)));
    }
    let Some(tween) = kf.tween else { return Some((kf.start, Cow::Borrowed(&kf.elements))) };
    let Some(next) = layer.keyframes.get(idx + 1) else { return Some((kf.start, Cow::Borrowed(&kf.elements))) };
    if next.start != kf.end() || frame == kf.start {
        return Some((kf.start, Cow::Borrowed(&kf.elements)));
    }
    let t = tween.easing.apply((frame - kf.start) as f32 / kf.duration.max(1) as f32);
    let morph = tween.kind == TweenKind::Shape;
    let path = guide.map(|g| (g, tween.orient));
    let pairs = if morph && !kf.hints.is_empty() { crate::hints::hint_pairs(kf, next) } else { Vec::new() };
    let els = kf
        .elements
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let hints: Vec<_> = pairs.iter().filter(|h| h.0 == i).map(|h| (h.1, h.2)).collect();
            next.elements.get(i).and_then(|b| interpolate(a, b, t, morph, &hints, path)).map_or_else(|| a.clone(), Arc::new)
        })
        .collect();
    Some((kf.start, Cow::Owned(els)))
}

/// Interpolates two elements; `None` when they cannot tween. With `morph` (a shape tween),
/// shapes of different structure blend through [`anicraft_geom::morph`], steered by `hints`
/// (shape hints in each element's own space, ADR 0023); otherwise a shape changes only when
/// both keyframes share its structure.
fn interpolate(a: &Element, b: &Element, t: f32, morph: bool, hints: &[(anicraft_geom::Vec2, anicraft_geom::Vec2)], guide: Option<(&PathWalk, bool)>) -> Option<Element> {
    let kind = match (&a.kind, &b.kind) {
        (ElementKind::Shape(sa), ElementKind::Shape(sb)) => {
            let path = if Arc::ptr_eq(&sa.path, &sb.path) && hints.is_empty() {
                sa.path.clone()
            } else if morph {
                anicraft_geom::morph::morph_hinted(&sa.path, &sb.path, hints, t).map(Arc::new).unwrap_or_else(|| sa.path.clone())
            } else {
                sa.path.lerp(&sb.path, t).map(Arc::new).unwrap_or_else(|| sa.path.clone())
            };
            let fill = match (&sa.fill, &sb.fill) {
                (Some(x), Some(y)) => Some(x.lerp(y, t)),
                (x, _) => x.clone(),
            };
            let stroke = match (&sa.stroke, &sb.stroke) {
                (Some(x), Some(y)) => Some(x.lerp(y, t)),
                (x, _) => x.clone(),
            };
            ElementKind::Shape(Shape { path, fill, stroke, fill_rule: sa.fill_rule, merge: sa.merge })
        }
        (ElementKind::Symbol { symbol: s1, .. }, ElementKind::Symbol { symbol: s2, .. }) if s1 == s2 => a.kind.clone(),
        // Text tweens its placement and colour effect; the words stay the first keyframe's.
        (ElementKind::Text(_), ElementKind::Text(_)) => a.kind.clone(),
        (ElementKind::Camera, ElementKind::Camera) => ElementKind::Camera,
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
    color: ColorTransform,
    depth: usize,
    /// The top-level layer and element everything inside belongs to (for picking).
    root: Option<(LayerId, ElementId)>,
    outline: Option<Rgba>,
    /// The stage as edited (ADR 0061): layers in outline mode are drawn as outlines. Off for
    /// the movie (every export, the player): View as Outlines is an editing aid.
    stage: bool,
    /// The clip everything inside is drawn within.
    clip: Option<u32>,
    /// Set while evaluating mask artwork: everything inside forms this clip.
    masks: Option<u32>,
    /// Set while evaluating guide artwork.
    guide: bool,
    /// The group everything inside fades with.
    group: Option<u32>,
    /// The camera the scene's layers are seen through (main timeline only; each layer's
    /// transform comes from it and the layer's depth, and `xf` is ignored).
    camera: Option<Camera>,
    /// The 9-slice scaling of the instance whose timeline this is (ADR 0055): its own shapes go
    /// through it, in place of the scale in `xf`. Not passed on to nested symbols.
    slice: Option<Slicing>,
}

impl Ctx {
    fn top(xf: Affine) -> Ctx {
        Ctx { xf, color: ColorTransform::IDENTITY, depth: 0, root: None, outline: None, stage: true, clip: None, masks: None, guide: false, group: None, camera: None, slice: None }
    }

    /// A main timeline seen through `camera`: its tint colours everything.
    fn through(camera: Option<Camera>) -> Ctx {
        let mut cx = Ctx::top(Affine::IDENTITY);
        cx.stage = false;
        if let Some(c) = camera {
            cx.color = c.color();
            cx.camera = Some(c);
        }
        cx
    }
}

/// The first frame of the unbroken run of keyframes on `layer`, up to `frame`, that hold
/// element `id` as an instance of `symbol`: where a movie clip instance started playing.
pub fn instance_start(layer: &Layer, frame: u32, id: ElementId, symbol: SymbolId) -> u32 {
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
pub fn guide_walk(guide: &Layer, frame: u32) -> Option<PathWalk> {
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
        ElementKind::Symbol { .. } | ElementKind::Text(_) | ElementKind::Camera => None,
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

fn eval_timeline(doc: &Document, tl: &Timeline, frame: u32, cx: &Ctx, out: &mut DisplayList, play: Option<&Play<'_>>) {
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
        // The camera layer holds the camera, not artwork (ADR 0027).
        if !layer.visible || matches!(layer.kind, LayerKind::Folder | LayerKind::Camera) || (is_guide && cx.masks.is_some()) || (has_folders && tl.hidden_by_folder(index)) {
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
        let layer_outline = cx.outline.or(if layer.outline && cx.stage { Some(layer.color) } else { None });
        // Through a camera, each layer is placed by the camera and its own depth.
        let layer_xf = cx.camera.map_or(cx.xf, |c| c.view(&doc.stage, layer.depth));
        for el in elements.iter() {
            // In the player, a symbol instance has its own path, and maybe properties a script set.
            let child = match (&el.kind, play) {
                (ElementKind::Symbol { .. }, Some(p)) => Some(p.child(el.id)),
                // So is a dynamic or input text field (ADR 0066).
                (ElementKind::Text(t), Some(p)) if t.field.is_some() => Some(p.child(el.id)),
                _ => None,
            };
            let set;
            let el: &Element = match child.as_ref().and_then(|c| c.state.props.get(&c.path)) {
                Some(ov) if !ov.shown() => continue,
                Some(ov) => {
                    set = ov.apply(el);
                    &set
                }
                None => el,
            };
            let (root_layer, root_el) = cx.root.unwrap_or((layer.id, el.id));
            let xf = layer_xf.mul(&el.transform);
            let fx = el.color.transform();
            let color = cx.color.concat(&fx);
            match &el.kind {
                ElementKind::Shape(s) => {
                    // Inside a 9-slice instance, the shape's outline is cut and mapped (bitmap
                    // fills scale normally, as in Animate).
                    let sliced = cx.slice.filter(|_| !matches!(s.fill, Some(Paint::Bitmap { .. }))).map(|sl| sliced_shape(s, &el.transform, &sl));
                    let (path, transform, fill, stroke) = match sliced {
                        Some(x) => x,
                        None => (s.path.clone(), xf, s.fill.clone(), s.stroke.clone()),
                    };
                    out.items.push(DisplayItem {
                        path,
                        transform,
                        fill,
                        stroke,
                        fill_rule: s.fill_rule,
                        color,
                        layer: root_layer,
                        element: root_el,
                        outline: layer_outline,
                        clip,
                        masks,
                        guide,
                        group: cx.group,
                        image: match &s.fill {
                            Some(Paint::Bitmap { bitmap, .. }) => doc.bitmap(*bitmap).and_then(crate::Bitmap::pixels),
                            _ => None,
                        },
                        smooth: match &s.fill {
                            Some(Paint::Bitmap { bitmap, .. }) => doc.bitmap(*bitmap).is_none_or(|b| b.smoothing),
                            _ => true,
                        },
                    });
                }
                ElementKind::Text(t) => {
                    let item = |path: Arc<Path>, transform: Affine, fill: Option<Paint>, stroke: Option<Stroke>, clip: Option<u32>, masks: Option<u32>| DisplayItem {
                        path,
                        transform,
                        fill,
                        stroke,
                        fill_rule: FillRule::NonZero,
                        color,
                        layer: root_layer,
                        element: root_el,
                        outline: layer_outline,
                        clip,
                        masks,
                        guide,
                        group: cx.group,
                        image: None,
                        smooth: true,
                    };
                    match &t.field {
                        // Mask artwork: a field's text masks; its box and border don't.
                        Some(f) if masks.is_none() => {
                            let live = child.as_ref().map(|c| (c.state.fields.get(&c.path), c.state.focus.as_deref() == Some(c.path.as_slice())));
                            field_items(t, f, live, xf, clip, out, item);
                        }
                        _ => {
                            let path = child.as_ref().and_then(|c| c.state.fields.get(&c.path)).and_then(|s| s.look.as_ref()).map_or_else(|| t.outline.clone(), |l| l.outline.clone());
                            out.items.push(item(path, xf, Some(Paint::Solid(t.color)), None, clip, masks));
                        }
                    }
                }
                ElementKind::Camera => {}
                ElementKind::Symbol { symbol, loop_mode, first_frame, blend, filters, .. } => {
                    let Some(sym) = doc.symbol(*symbol) else { continue };
                    let len = sym.timeline.frame_count();
                    let local = match (sym.kind, &child) {
                        // A graphic keeps step with this timeline: loop mode and first frame apply.
                        (SymbolKind::Graphic, _) => loop_mode.local_frame(frame.saturating_sub(kf_start), *first_frame, len),
                        // In the player, movie clips and buttons are where it has them.
                        (SymbolKind::MovieClip, Some(c)) => c.state.clip_frame(&c.path, len),
                        (SymbolKind::Button, Some(c)) => c.state.button_frame(&c.path, len),
                        // A movie clip plays its own timeline on a loop from when the instance
                        // appeared, carrying on across keyframes that hold the same instance.
                        (SymbolKind::MovieClip, None) => frame.saturating_sub(instance_start(layer, frame, el.id, *symbol)) % len.max(1),
                        // While editing, a button shows its Up frame.
                        (SymbolKind::Button, None) => 0,
                    };
                    // An instance below full opacity fades as one: its contents composite in a
                    // group at full opacity, and the group blends at the instance's alpha. (Mask
                    // artwork ignores opacity, so it never needs one.)
                    // A blend mode composites the instance on its own too, then blends it.
                    // An alpha offset (Advanced) changes each item's alpha on its own: it stays
                    // in the items' transform, and the group (for a blend or filters) is opaque.
                    let fade = fx.alpha_factor().unwrap_or(1.0);
                    let (color, group) = if masks.is_none() && (fade < 1.0 - 1e-4 || !blend.is_normal() || !filters.is_empty()) {
                        let gid = out.groups.len() as u32;
                        out.groups.push(Group { parent: cx.group, alpha: fade, blend: *blend, filters: filters.iter().map(crate::Filter::sanitized).collect(), clip });
                        let inner = if fx.alpha_factor().is_some() { fx.without_alpha() } else { fx };
                        (cx.color.concat(&inner), Some(gid))
                    } else {
                        (color, cx.group)
                    };
                    // A symbol with 9-slice guides, scaled: its shapes are sliced (ADR 0055).
                    let slice = crate::nine_slice::slicing(doc, sym, &el.transform).map(|s| Slicing { post: layer_xf.mul(&s.post), ..s });
                    let inner = Ctx { xf, color, depth: cx.depth + 1, root: Some((root_layer, root_el)), outline: layer_outline, stage: cx.stage, clip, masks, guide, group, camera: None, slice };
                    eval_timeline(doc, &sym.timeline, local, &inner, out, child.as_ref());
                }
            }
        }
    }
}

/// The highlight behind selected text in a focused field.
pub const SELECTION: Rgba = Rgba::new(0.62, 0.78, 1.0, 1.0);

/// A dynamic or input field's items (ADR 0066): its background, its text cut to its box, the
/// selection and caret when it has the keyboard, and its border. `live` is its state in the
/// player (and whether it is focused); without one, it shows its authored outline.
fn field_items(t: &crate::TextBlock, f: &crate::TextField, live: Option<(Option<&crate::field::FieldState>, bool)>, xf: Affine, clip: Option<u32>, out: &mut DisplayList, item: impl Fn(Arc<Path>, Affine, Option<Paint>, Option<Stroke>, Option<u32>, Option<u32>) -> DisplayItem) {
    let (w, h) = (t.box_width.max(0.0), t.box_height.max(0.0));
    let rect = Arc::new(Path::rect(0.0, 0.0, w, h));
    if f.background {
        out.items.push(item(rect.clone(), xf, Some(Paint::Solid(f.background_color)), None, clip, None));
    }
    // The box clips what is inside it.
    let inner = out.clip_parents.len() as u32;
    out.clip_parents.push(clip);
    out.items.push(item(rect.clone(), xf, Some(Paint::Solid(Rgba::BLACK)), None, clip, Some(inner)));
    let (state, focused) = live.unwrap_or((None, false));
    let scroll = state.map_or(anicraft_geom::Vec2::ZERO, |s| s.scroll);
    let text_xf = xf.mul(&Affine::translate(-scroll.x, -scroll.y));
    let look = state.and_then(|s| s.look.as_deref());
    // The selection, under the text.
    if let (true, Some(st), Some(look)) = (focused, state, look) {
        let (a, b) = st.selection();
        if a != b {
            for (row, l) in look.lines.iter().enumerate() {
                let (from, to) = (a.max(l.start), b.min(l.end));
                if from > to || (from == to && !(a <= l.end && b > l.end)) {
                    continue;
                }
                let (x0, x1) = (l.x_at(from), if b > l.end { l.x_at(l.end) + t.size * 0.25 } else { l.x_at(to) });
                let y = row as f32 * look.line_height;
                let r = Path::rect(crate::FIELD_GUTTER + x0, crate::FIELD_GUTTER + y, (x1 - x0).max(0.0), look.line_height);
                out.items.push(item(Arc::new(r), text_xf, Some(Paint::Solid(SELECTION)), None, Some(inner), None));
            }
        }
    }
    // The text: laid out by the player when it changed, else the authored outline (a field whose
    // text changed but couldn't be laid out shows none).
    let outline = match (state.and_then(|s| s.text.as_ref()), look) {
        (_, Some(l)) => Some(l.outline.clone()),
        (None, None) => Some(t.outline.clone()),
        (Some(_), None) => None,
    };
    if let Some(path) = outline {
        out.items.push(item(path, text_xf, Some(Paint::Solid(t.color)), None, Some(inner), None));
    }
    // The caret, in an input field with the keyboard and nothing selected.
    if let (true, Some(st), crate::TextKind::Input) = (focused, state, f.kind)
        && st.caret == st.anchor
    {
        let (at, lh) = look.map_or((anicraft_geom::Vec2::ZERO, t.size * 1.2), |l| (l.caret_at(st.caret), l.line_height));
        let r = Path::rect(crate::FIELD_GUTTER + at.x - 0.5, crate::FIELD_GUTTER + at.y, 1.0, lh);
        out.items.push(item(Arc::new(r), text_xf, Some(Paint::Solid(t.color)), None, Some(inner), None));
    }
    if f.border {
        let mut s = Stroke::new(f.border_color, 1.0);
        s.style.join = crate::Join::Miter;
        out.items.push(item(rect, xf, None, Some(s), clip, None));
    }
}

/// A shape inside a 9-slice instance (ADR 0055): its outline placed by its own transform, then
/// cut at the guides and mapped piecewise; drawn by the instance's transform without its scale.
/// The stroke keeps its width (times the shape's own scale inside the symbol); gradients follow
/// the map, their size the shape's own scale times the instance's.
fn sliced_shape(s: &Shape, own: &Affine, sl: &Slicing) -> (Arc<Path>, Affine, Option<Paint>, Option<Stroke>) {
    let path = Arc::new(anicraft_geom::slice::slice_path(&s.path, own, &sl.map));
    let k = own.mean_scale();
    let stroke = s.stroke.as_ref().map(|st| if (k - 1.0).abs() > 1e-6 { st.scaled(k) } else { st.clone() });
    let at = |p: anicraft_geom::Vec2| sl.map.map(own.apply(p));
    let size = k * sl.scale.mean_scale();
    let fill = s.fill.as_ref().map(|f| match f {
        Paint::Linear { start, end, stops } => Paint::Linear { start: at(*start), end: at(*end), stops: stops.clone() },
        Paint::Radial { center, radius, stops } => Paint::Radial { center: at(*center), radius: radius * size, stops: stops.clone() },
        Paint::Solid(_) | Paint::Bitmap { .. } => f.clone(),
    });
    (path, sl.post, fill, stroke)
}

#[cfg(test)]
#[path = "eval_tests.rs"]
mod tests;
