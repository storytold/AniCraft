//! Editing a symbol's timeline in place (ADR 0045), as Animate's double-click on an instance does.
//!
//! The session keeps a stack of open symbols. While one is open, every command works on its
//! timeline instead of the scene's ([`Session::timeline`]): its layers are the active layers, the
//! playhead is its playhead, and positions are in its own space. So drawing, merge drawing (ADR
//! 0031), the Selection tool (ADR 0041) and Subselection (ADR 0042) behave inside a symbol exactly
//! as on the stage: they are the same commands on another timeline. Which symbol is open is view
//! state, but undo snapshots keep it, so an undo goes back to where its change was made.
//!
//! The stage draws [`Session::stage_picture`]: the open symbol where its instance stands, over the
//! timelines it sits in, dimmed.

use crate::history::StateId;
use crate::params::*;
use crate::{CommandSpec, EngineError, Result, Selection, Session};
use anicraft_doc::{ColorEffect, Document, Element, ElementId, ElementKind, Keyframe, Layer, LayerId, LayerKind, LoopMode, Symbol, SymbolId, SymbolKind, Timeline};
use anicraft_geom::{Affine, Vec2};
use serde_json::{Value, json};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

pub fn specs() -> Vec<CommandSpec> {
    let c = |id, label, menu, shortcut, params, mutates, run| CommandSpec { id, label, menu, shortcut, params, mutates, run };
    vec![
        c(
            "symbol.edit",
            "Edit Symbols",
            "Edit",
            Some("Ctrl+E"),
            "{id?: instance element id, symbol?: library symbol id} opens a symbol's timeline for editing: the instance `id` (default: the selected instance) in place, or a Library symbol at the stage's centre. A group (ADR 0073) or a drawing object (ADR 0080) opens in place the same way: inside, its raw shapes merge only with each other, and a drawing object closes as one object again (a group of its pieces when an edit left several). Every command then works inside it. With nothing to open while a symbol is open, goes back to the document (Animate's Ctrl+E)",
            false,
            edit as crate::RunFn,
        ),
        c("symbol.exit", "Edit Document", "Edit", None, "{all?: bool} closes the open symbol (default: one level; all: back to the scene), selecting the instance it was opened from", false, exit),
    ]
}

/// Most symbols open inside each other at once.
const MAX_LEVELS: usize = 32;
/// How much the artwork around an open symbol shows (Animate dims it).
const DIM: f32 = 0.35;

/// What a level of the editing stack has open.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Opened {
    /// A symbol's timeline (ADR 0045).
    Symbol(SymbolId),
    /// A group's members (ADR 0073): the group is [`EditLevel::instance`] on the timeline below,
    /// on the keyframe at [`EditLevel::frame`].
    Group,
    /// A drawing object opened in place (ADR 0080): while it is open it is a group of its raw
    /// shapes, as [`Opened::Group`]; closing it makes it one drawing object again when it holds
    /// one shape (a group of its pieces when it holds several, as Combine Objects' Union).
    Drawing,
}

impl Opened {
    /// Whether this level opens a group's members (a group, or a drawing object being edited).
    pub fn is_group(self) -> bool {
        matches!(self, Opened::Group | Opened::Drawing)
    }
}

/// A symbol or group open for editing, and what to go back to.
#[derive(Clone, Debug, PartialEq)]
pub struct EditLevel {
    pub open: Opened,
    /// The instance it was opened from, on the timeline below (none: from the Library).
    pub instance: Option<(LayerId, ElementId)>,
    /// Stage space from the symbol's space: where the instance stands (through every level).
    pub transform: Affine,
    /// The timeline below as it was: its active layer, playhead and selection.
    pub layer: LayerId,
    pub frame: u32,
    pub selection: Selection,
    /// A drawing object opened in place (ADR 0080): the document and state from before it was
    /// opened, and the state right after, so closing it without an edit puts both back.
    pub back: Option<Arc<(Document, StateId, StateId)>>,
}

impl EditLevel {
    /// The symbol this level has open (`None` for a group).
    pub fn symbol(&self) -> Option<SymbolId> {
        match self.open {
            Opened::Symbol(id) => Some(id),
            Opened::Group | Opened::Drawing => None,
        }
    }
}

/// The members of the group level `l` opens, on timeline `below`.
fn group_in<'a>(below: &'a Timeline, l: &EditLevel) -> Option<&'a Timeline> {
    let (layer, id) = l.instance?;
    let k = below.layer(layer)?.keyframe_at(l.frame)?;
    match &k.elements.iter().find(|e| e.id == id)?.kind {
        ElementKind::Group { members } => Some(members),
        _ => None,
    }
}

/// [`group_in`], copy-on-write.
fn group_in_mut<'a>(below: &'a mut Timeline, l: &EditLevel) -> Option<&'a mut Timeline> {
    let (layer, id) = l.instance?;
    let layer = below.layer_mut(layer)?;
    let k = layer.keyframe_index_at(l.frame)?;
    let el = layer.keyframe_mut(k)?.elements_mut_where(|e| e == id).into_iter().next()?;
    match &mut el.kind {
        ElementKind::Group { members } => Some(members),
        _ => None,
    }
}

/// How many levels of `editing`, from the outermost, still open something, and the timeline the
/// last of them opens.
fn walk<'a>(doc: &'a Document, scene: usize, editing: &[EditLevel]) -> (usize, &'a Timeline) {
    let mut tl = doc.timeline(scene);
    for (i, l) in editing.iter().enumerate() {
        let next = match l.open {
            Opened::Symbol(id) => doc.symbol(id).map(|s| &s.timeline),
            Opened::Group | Opened::Drawing => group_in(tl, l),
        };
        match next {
            Some(t) => tl = t,
            None => return (i, tl),
        }
    }
    (editing.len(), tl)
}

/// The timeline being edited: the open symbol's or group's, else the scene's.
pub(crate) fn edited<'a>(doc: &'a Document, scene: usize, editing: &[EditLevel]) -> &'a Timeline {
    walk(doc, scene, editing).1
}

/// [`edited`] ([`Session::timeline`]), copy-on-write. Takes the fields, so a caller can keep using
/// the session's others. Fails only when an open group is gone, which [`Session::fix_editing`]
/// never leaves for long.
pub(crate) fn edited_mut<'a>(doc: &'a mut Document, scene: usize, editing: &[EditLevel]) -> Result<&'a mut Timeline> {
    let (valid, _) = walk(doc, scene, editing);
    let levels = editing.get(..valid).unwrap_or_default();
    // From the innermost open symbol (or the scene), through the groups opened inside it.
    let base = levels.iter().rposition(|l| matches!(l.open, Opened::Symbol(_)));
    let open = base.and_then(|b| levels.get(b)).and_then(EditLevel::symbol).and_then(|id| doc.library.iter().position(|s| s.id == id));
    let groups = levels.get(base.map_or(0, |b| b + 1)..).unwrap_or_default();
    // (Borrowed only on the branch that returns it.)
    let mut tl = match open {
        Some(i) if i < doc.library.len() => &mut Arc::make_mut(&mut doc.library[i]).timeline,
        _ => doc.timeline_mut(scene),
    };
    for l in groups {
        tl = group_in_mut(tl, l).ok_or_else(|| EngineError::State("the group open for editing is gone".into()))?;
    }
    Ok(tl)
}

/// How the stage shows the open symbol: its space on the stage is `translate(origin) ·
/// scale(scale)`. `in_place` when that is exactly where its instance stands, so the artwork
/// around it shows too.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EditView {
    pub origin: Vec2,
    pub scale: f32,
    pub in_place: bool,
}

/// What the stage draws: a document, the movie frame of it to render, and a state id for caching
/// the picture (equal ids, equal pictures).
#[derive(Clone)]
pub struct StagePicture {
    pub doc: Document,
    pub frame: u32,
    pub state: StateId,
    /// The open symbol in front, when one is open: onion skin and Edit Multiple Frames compose
    /// its own frames (ADR 0051).
    pub front: Option<Arc<Front>>,
}

/// The open symbol in a [`StagePicture`]: the picture's top layer `layer` shows `timeline` at its
/// `frame`, placed by `at`. With onion skin on, that layer is drawn instead as the timeline's
/// frames composed by [`crate::onion::compose_timeline`], placed the same way.
#[derive(Clone, Debug)]
pub struct Front {
    pub layer: LayerId,
    pub timeline: Timeline,
    pub frame: u32,
    pub at: Affine,
    /// The timeline the symbol was opened from, when it shows around it (in place): with onion
    /// skin on, its frames around its own playhead show dimmed too (ADR 0061).
    pub parent: Option<Around>,
}

/// A timeline drawn dimmed around an open symbol: the picture's layer `layer` shows `timeline`
/// (without the instance that was opened) at its `frame`, placed by `at`, at opacity `dim`.
#[derive(Clone, Debug)]
pub struct Around {
    pub layer: LayerId,
    pub timeline: Timeline,
    pub frame: u32,
    pub at: Affine,
    pub dim: f32,
}

impl Session {
    /// The timeline being edited: the open symbol's (ADR 0045), else the scene's.
    pub fn timeline(&self) -> &Timeline {
        edited(&self.doc, self.scene, &self.editing)
    }

    /// The symbol open for editing, if any (`None` on the stage and while a group is open).
    pub fn editing_symbol(&self) -> Option<SymbolId> {
        self.editing.last().and_then(EditLevel::symbol)
    }

    /// Whether a group is open for editing (ADR 0073).
    pub fn editing_group(&self) -> bool {
        self.editing.last().is_some_and(|l| l.open.is_group())
    }

    /// Whether a drawing object is open for editing (ADR 0080).
    pub fn editing_drawing(&self) -> bool {
        self.editing.last().is_some_and(|l| l.open == Opened::Drawing)
    }

    /// Closes levels whose symbol or group is gone (an undo, a Library deletion, an ungroup),
    /// from the first such.
    pub(crate) fn fix_editing(&mut self) {
        let (valid, _) = walk(&self.doc, self.scene, &self.editing);
        while self.editing.len() > valid {
            self.exit_level();
        }
    }

    fn enter_level(&mut self, open: Opened, instance: Option<(LayerId, ElementId)>, transform: Affine, frame: u32) {
        self.editing.push(EditLevel { open, instance, transform, layer: self.layer, frame: self.frame, selection: std::mem::take(&mut self.selection), back: None });
        // The top layer inside, at the frame the instance shows.
        self.layer = self.timeline().layers.first().map_or(LayerId(0), |l| l.id);
        self.frame = frame.min(self.timeline().frame_count().saturating_sub(1));
        self.revision += 1;
        self.dl_cache.replace(None);
    }

    /// Closes the open symbol; the instance it was opened from is selected again. False when
    /// none is open.
    pub(crate) fn exit_level(&mut self) -> bool {
        let Some(l) = self.editing.pop() else { return false };
        let gone = l.open == Opened::Drawing && self.close_drawing(&l);
        self.layer = l.layer;
        self.frame = l.frame;
        self.selection = match l.instance {
            Some(_) if gone => Selection::default(),
            Some((layer, id)) => Selection::only(layer, [id]),
            None => l.selection,
        };
        self.revision += 1;
        self.dl_cache.replace(None);
        true
    }

    /// Closes every open symbol (scene changes, opening a document).
    pub(crate) fn exit_all(&mut self) {
        while self.exit_level() {}
    }

    /// How the stage shows the open symbol, `None` on the stage itself. In place when its
    /// instance is only moved and scaled evenly (the editor's view can follow that exactly);
    /// otherwise upright and at full size, with its registration point where the instance's is.
    pub fn edit_view(&self) -> Option<EditView> {
        let l = self.editing.last()?;
        let [a, b, c, d, e, f] = l.transform.m;
        let origin = Vec2::new(e, f);
        let even = b.abs() < 1e-5 && c.abs() < 1e-5 && a > 1e-4 && (a - d).abs() <= 1e-4 * a.max(1.0);
        let in_place = even && self.editing.iter().all(|l| l.instance.is_some());
        Some(if in_place { EditView { origin, scale: a, in_place } } else { EditView { origin, scale: 1.0, in_place: false } })
    }

    /// What the stage draws (ADR 0045): the document at the playhead; while a symbol is open, its
    /// timeline at its playhead where its instance stands, over the timelines it sits in dimmed.
    pub fn stage_picture(&self) -> Arc<StagePicture> {
        let Some(view) = self.edit_view() else {
            return Arc::new(StagePicture { doc: self.doc.clone(), frame: self.doc.scene_start(self.scene).saturating_add(self.frame), state: self.state, front: None });
        };
        let mut h = std::collections::hash_map::DefaultHasher::new();
        (self.state, self.scene, self.frame).hash(&mut h);
        for l in &self.editing {
            (l.open, l.instance, l.frame, l.transform.m.map(f32::to_bits)).hash(&mut h);
        }
        let hit = self.hit_overlay();
        hit.map(|(c, a)| [c.r, c.g, c.b, c.a, a].map(f32::to_bits)).hash(&mut h);
        let key = h.finish();
        if let Some((k, pic)) = &*self.picture_cache.borrow()
            && *k == key
        {
            return pic.clone();
        }
        let mut doc = self.doc.clone();
        let mut tl = Timeline::default();
        // The open symbol, in front.
        let at = Affine::translate(view.origin.x, view.origin.y).mul(&Affine::scale(view.scale, view.scale));
        let open = self.timeline().clone();
        // A button's hit area over the state being edited, translucent (ADR 0057).
        if let Some((tint, alpha)) = hit {
            let frame = anicraft_doc::player::hit_frame(open.frame_count());
            tl.layers.push(placed(&mut doc, hit_timeline(&open), at, frame, ColorEffect::Advanced(hit_tint(tint, alpha))));
        }
        let layer = placed(&mut doc, open.clone(), at, self.frame, ColorEffect::default());
        let mut front = Front { layer: layer.id, timeline: open, frame: self.frame, at, parent: None };
        tl.layers.push(layer);
        // Around it, dimmed: each timeline below, without the instance opened from it.
        if view.in_place {
            for (i, l) in self.editing.iter().enumerate().rev() {
                let parent = i.checked_sub(1).and_then(|j| self.editing.get(j));
                let below = edited(&self.doc, self.scene, self.editing.get(..i).unwrap_or_default()).clone();
                let xf = parent.map_or(Affine::IDENTITY, |p| p.transform);
                let around = without(below, l.instance);
                let layer = placed(&mut doc, around.clone(), xf, l.frame, ColorEffect::opacity(DIM));
                // The timeline right under the open symbol is the one onion skin shows around it.
                if front.parent.is_none() {
                    front.parent = Some(Around { layer: layer.id, timeline: around, frame: l.frame, at: xf, dim: DIM });
                }
                tl.layers.push(layer);
            }
        }
        let front = Arc::new(front);
        *doc.timeline_mut(self.scene) = tl;
        // Apart from document states (counted up from 1), and the same for the same picture.
        let pic = Arc::new(StagePicture { frame: doc.scene_start(self.scene), doc, state: key | 1 << 63, front: Some(front) });
        self.picture_cache.replace(Some((key, pic.clone())));
        pic
    }
}

impl Session {
    /// The tint and opacity the stage draws the open button's hit area in (ADR 0057): while a
    /// button is open and its playhead is on another frame than the one that answers the
    /// pointer (the Hit frame, or Up when it has none), and [`Session::hit_area`] is set.
    pub fn hit_overlay(&self) -> Option<(anicraft_doc::Rgba, f32)> {
        let (tint, alpha) = self.hit_area?;
        let sym = self.editing_symbol().and_then(|id| self.doc.symbol(id))?;
        let frame = anicraft_doc::player::hit_frame(sym.timeline.frame_count());
        (sym.kind == SymbolKind::Button && self.frame != frame && alpha.is_finite() && alpha > 0.0).then_some((tint, alpha.min(1.0)))
    }
}

/// What answers the pointer on a button with timeline `tl`, as a timeline to draw (the hit test's
/// rules, `doc::player::hit_area`): mask layers go and what they masked is whole again, guide
/// layers are hidden (they still steer what follows them).
fn hit_timeline(tl: &Timeline) -> Timeline {
    let masks: Vec<LayerId> = tl.layers.iter().filter(|l| l.kind == LayerKind::Mask).map(|l| l.id).collect();
    let layers = tl
        .layers
        .iter()
        .filter(|l| l.kind != LayerKind::Mask)
        .map(|l| {
            if l.kind != LayerKind::Guide && !l.parent.is_some_and(|p| masks.contains(&p)) {
                return l.clone();
            }
            let mut l = (**l).clone();
            if l.kind == LayerKind::Guide {
                l.visible = false;
            }
            if l.parent.is_some_and(|p| masks.contains(&p)) {
                l.parent = None;
            }
            Arc::new(l)
        })
        .collect();
    Timeline { layers }
}

/// A layer holding `timeline` as a graphic symbol's instance at `transform`, showing its `frame`
/// with colour effect `color` (a symbol added to `doc`, which is only drawn).
fn placed(doc: &mut Document, timeline: Timeline, transform: Affine, frame: u32, color: ColorEffect) -> Arc<Layer> {
    let symbol = SymbolId(doc.alloc_id());
    doc.library.push(Arc::new(Symbol { id: symbol, name: String::new(), kind: SymbolKind::Graphic, linkage: None, scale_grid_kept: None, scale_grid: None, timeline }));
    let mut k = Keyframe::new(0, 1);
    k.elements.push(Arc::new(Element {
        id: ElementId(doc.alloc_id()),
        name: String::new(),
        transform,
        color,
        kind: ElementKind::Symbol { symbol, loop_mode: LoopMode::SingleFrame, first_frame: frame, blend: Default::default(), filters: Vec::new(), events: Vec::new() },
    }));
    let mut layer = Layer::new(LayerId(doc.alloc_id()), "");
    layer.keyframes.push(Arc::new(k));
    Arc::new(layer)
}

impl Session {
    /// Opens the drawing object `id` on `layer` in place (ADR 0080): it becomes a group of one
    /// raw shape (its own, unmoved), which the level opens. The picture doesn't change, so this
    /// is no undo step; [`Session::close_drawing`] turns it back.
    fn open_drawing(&mut self, layer: LayerId, id: ElementId) -> Result<()> {
        let inner = ElementId(self.doc.alloc_id());
        let inner_layer = LayerId(self.doc.alloc_id());
        let frame = self.frame;
        let tl = edited_mut(&mut self.doc, self.scene, &self.editing)?;
        let l = tl.layer_mut(layer).ok_or_else(|| EngineError::State("that drawing object's layer is gone".into()))?;
        let k = l.keyframe_index_at(frame).ok_or_else(|| EngineError::State("no keyframe here".into()))?;
        let kf = l.keyframe_mut(k).ok_or_else(|| EngineError::State("no keyframe here".into()))?;
        let at = kf.elements.iter().position(|e| e.id == id).ok_or_else(|| EngineError::State("that drawing object is not on this keyframe".into()))?;
        let Some(el) = kf.elements.get(at).cloned() else { return Err(EngineError::State("that drawing object is not on this keyframe".into())) };
        let ElementKind::Shape(sh) = &el.kind else { return Err(EngineError::State("that is not a drawing object".into())) };
        let shape = Element { id: inner, name: String::new(), transform: Affine::IDENTITY, color: ColorEffect::None, kind: ElementKind::Shape(anicraft_doc::Shape { merge: true, ..sh.clone() }) };
        let group = Element { id, name: el.name.clone(), transform: el.transform, color: el.color, kind: ElementKind::group(inner_layer, vec![Arc::new(shape)]) };
        if let Some(slot) = kf.elements.get_mut(at) {
            *slot = Arc::new(group);
        }
        Ok(())
    }

    /// Closes a drawing object opened in place (level `l`, just taken off the stack): one shape
    /// left inside is one drawing object again, exactly as it shows (its id, name and place);
    /// nothing left, and it is gone; several, and they stay a group of raw shapes (what Combine
    /// Objects' Union makes of shapes of several colours). Opening and closing it without an
    /// edit leaves the document as it was, state and all. True when it is gone.
    fn close_drawing(&mut self, l: &EditLevel) -> bool {
        // Nothing changed since it was opened: the document goes back exactly as it was.
        if let Some(back) = &l.back
            && back.2 == self.state
        {
            self.doc = back.0.clone();
            self.state = back.1;
            self.dl_cache.replace(None);
            return false;
        }
        let Some((layer, id)) = l.instance else { return false };
        let below = self.editing.clone();
        let Ok(tl) = edited_mut(&mut self.doc, self.scene, &below) else { return false };
        let Some(lay) = tl.layer_mut(layer) else { return false };
        let Some(k) = lay.keyframe_index_at(l.frame) else { return false };
        let Some(kf) = lay.keyframe_mut(k) else { return false };
        let Some(at) = kf.elements.iter().position(|e| e.id == id) else { return false };
        let Some(group) = kf.elements.get(at).cloned() else { return false };
        let ElementKind::Group { members } = &group.kind else { return false };
        let inside: Vec<Arc<Element>> = members.layers.iter().filter_map(|m| m.keyframe_at(0)).flat_map(|k| k.elements.iter().cloned()).collect();
        let one_layer = members.layers.len() == 1;
        let mut gone = false;
        match inside.as_slice() {
            [] if one_layer => {
                kf.elements.remove(at);
                gone = true;
            }
            [only] if one_layer => {
                // Two colour effects only nest as one when one of them is none.
                let color = match (group.color, only.color) {
                    (c, ColorEffect::None) | (ColorEffect::None, c) => Some(c),
                    _ => None,
                };
                if let (ElementKind::Shape(sh), Some(color)) = (&only.kind, color) {
                    let object = Element { id, name: group.name.clone(), transform: group.transform.mul(&only.transform), color, kind: ElementKind::Shape(anicraft_doc::Shape { merge: false, ..sh.clone() }) };
                    if let Some(slot) = kf.elements.get_mut(at) {
                        *slot = Arc::new(object);
                    }
                }
            }
            _ => {}
        }
        self.state = crate::history::fresh_state();
        self.dl_cache.replace(None);
        gone
    }
}

/// Whether an instance of `sym` placed in the open symbol would put a symbol inside itself: `sym`
/// is open, or uses an open one, however deep.
pub(crate) fn would_nest(s: &Session, sym: SymbolId) -> bool {
    if s.editing.is_empty() {
        return false;
    }
    let mut seen: Vec<SymbolId> = Vec::new();
    let mut todo = vec![sym];
    while let Some(id) = todo.pop() {
        if s.editing.iter().any(|l| l.symbol() == Some(id)) {
            return true;
        }
        if seen.contains(&id) || seen.len() > 100_000 {
            continue;
        }
        seen.push(id);
        let Some(sym) = s.doc.symbol(id) else { continue };
        for k in sym.timeline.layers.iter().flat_map(|l| l.keyframes.iter()) {
            todo.extend(anicraft_doc::expand_groups(&k.elements).iter().filter_map(|e| match e.kind {
                ElementKind::Symbol { symbol, .. } => Some(symbol),
                _ => None,
            }));
        }
    }
    false
}

/// `tl` without the instance `which` (on every keyframe of its layer).
fn without(mut tl: Timeline, which: Option<(LayerId, ElementId)>) -> Timeline {
    let Some((layer, id)) = which else { return tl };
    if let Some(l) = tl.layer_mut(layer) {
        for k in &mut l.keyframes {
            if k.elements.iter().any(|e| e.id == id) {
                Arc::make_mut(k).elements.retain(|e| e.id != id);
            }
        }
    }
    tl
}

/// Whether `symbol.edit` opens `e`: a symbol instance, a group or a drawing object.
pub fn opens(e: &Element) -> bool {
    match &e.kind {
        ElementKind::Symbol { .. } | ElementKind::Group { .. } => true,
        // A warped shape (ADR 0079) keeps its outline as drawn under its mesh: it is edited
        // through its pins, not opened.
        ElementKind::Shape(sh) => !sh.merge && sh.warp.is_none(),
        _ => false,
    }
}

/// The instance `id` on a layer at the playhead of the timeline being edited, and its layer.
fn instance(s: &Session, id: ElementId) -> Option<(LayerId, Arc<Element>)> {
    s.timeline().layers.iter().map(|l| l.id).find_map(|l| s.elements_on(l).into_iter().find(|e| e.id == id).map(|e| (l, e)))
}

pub(crate) fn edit(s: &mut Session, p: &Value) -> Result<Value> {
    if s.editing.len() >= MAX_LEVELS {
        return Err(EngineError::State(format!("at most {MAX_LEVELS} symbols can be open inside each other")));
    }
    // From the Library: alone, its registration point at the stage's centre.
    if let Some(sym) = opt_u64(p, "symbol")? {
        let symbol = SymbolId(sym);
        if s.doc.symbol(symbol).is_none() {
            return Err(EngineError::State(format!("no symbol {sym} in the Library")));
        }
        if s.editing.iter().any(|l| l.symbol() == Some(symbol)) {
            return Err(EngineError::State("that symbol is open already".into()));
        }
        let c = Vec2::new(s.doc.stage.width as f32 * 0.5, s.doc.stage.height as f32 * 0.5);
        s.enter_level(Opened::Symbol(symbol), None, Affine::translate(c.x, c.y), 0);
        return Ok(editing_json(s));
    }
    let id = match opt_u64(p, "id")? {
        Some(i) => ElementId(i),
        None => {
            let picked: Vec<ElementId> = s.selection.on(s.layer).into_iter().filter(|id| instance(s, *id).is_some_and(|(_, e)| opens(&e))).collect();
            match picked.as_slice() {
                [one] => *one,
                // Nothing to open: Ctrl+E goes back to the document, as in Animate.
                [] if !s.editing.is_empty() => {
                    s.exit_all();
                    return Ok(editing_json(s));
                }
                [] => return Err(EngineError::State("select a symbol instance, a group or a drawing object to edit it (or double-click it)".into())),
                _ => return Err(EngineError::State("select one symbol instance, group or drawing object to edit".into())),
            }
        }
    };
    let (layer, el) = instance(s, id).ok_or_else(|| EngineError::State(format!("element {} is not on a layer here at this frame", id.0)))?;
    if !s.selectable(layer) {
        return Err(EngineError::State("that instance's layer is locked or hidden".into()));
    }
    let below = s.editing.last().map_or(Affine::IDENTITY, |l| l.transform);
    // A group opens where it stands (ADR 0073): its members at their only frame. The level finds
    // the keyframe it is on through the playhead below.
    if matches!(el.kind, ElementKind::Group { .. }) {
        s.enter_level(Opened::Group, Some((layer, id)), below.mul(&el.transform), 0);
        return Ok(editing_json(s));
    }
    // A drawing object opens the same way (ADR 0080), as a group of its one raw shape.
    if matches!(&el.kind, ElementKind::Shape(sh) if !sh.merge && sh.warp.is_some()) {
        return Err(EngineError::State("that drawing object is warped (Asset Warp): edit it with its pins, or remove its warp to edit its outline".into()));
    }
    if matches!(&el.kind, ElementKind::Shape(sh) if !sh.merge) {
        if crate::cmds_group::open_groups(s) + 1 > anicraft_doc::MAX_GROUP_DEPTH {
            return Err(EngineError::State(format!("groups nest at most {} deep", anicraft_doc::MAX_GROUP_DEPTH)));
        }
        let (was, state) = (s.doc.clone(), s.state);
        s.open_drawing(layer, id)?;
        s.state = crate::history::fresh_state();
        s.enter_level(Opened::Drawing, Some((layer, id)), below.mul(&el.transform), 0);
        if let Some(l) = s.editing.last_mut() {
            l.back = Some(Arc::new((was, state, s.state)));
        }
        return Ok(editing_json(s));
    }
    let ElementKind::Symbol { symbol, loop_mode, first_frame, .. } = el.kind else { return Err(EngineError::State("that is not a symbol instance, a group or a drawing object".into())) };
    let sym = s.doc.symbol(symbol).ok_or_else(|| EngineError::State("that instance's symbol is missing from the Library".into()))?;
    if s.editing.iter().any(|l| l.symbol() == Some(symbol)) {
        return Err(EngineError::State("that symbol is open already".into()));
    }
    // A graphic opens at the frame the instance shows; movie clips and buttons at their first.
    let len = sym.timeline.frame_count();
    let start = s.timeline().layer(layer).and_then(|l| l.keyframe_at(s.frame)).map_or(s.frame, |k| k.start);
    let frame = if sym.kind == SymbolKind::Graphic { loop_mode.local_frame(s.frame.saturating_sub(start), first_frame, len) } else { 0 };
    s.enter_level(Opened::Symbol(symbol), Some((layer, id)), below.mul(&el.transform), frame);
    Ok(editing_json(s))
}

fn exit(s: &mut Session, p: &Value) -> Result<Value> {
    if s.editing.is_empty() {
        return Err(EngineError::State("no symbol or group is open for editing".into()));
    }
    if opt_bool(p, "all")?.unwrap_or(false) {
        s.exit_all();
    } else {
        s.exit_level();
    }
    Ok(editing_json(s))
}

/// The open symbols, outermost first.
pub(crate) fn editing_json(s: &Session) -> Value {
    let list: Vec<Value> = s
        .editing
        .iter()
        .map(|l| match l.symbol() {
            Some(id) => json!({"symbol": id.0, "name": s.doc.symbol(id).map(|x| x.name.clone()), "instance": l.instance.map(|(_, e)| e.0)}),
            None if l.open == Opened::Drawing => json!({"drawing": l.instance.map(|(_, e)| e.0), "name": "Drawing Object", "instance": l.instance.map(|(_, e)| e.0)}),
            None => json!({"group": l.instance.map(|(_, e)| e.0), "name": "Group", "instance": l.instance.map(|(_, e)| e.0)}),
        })
        .collect();
    json!({"editing": list, "layer": s.layer.0, "frame": s.frame, "selected": s.selection.id_list()})
}

/// Every colour of the hit area replaced by `tint`, the whole faded to `alpha` (ADR 0057): what
/// Animate's Advanced effect calls multipliers 0 % and offsets of the tint, alpha at `alpha`.
fn hit_tint(tint: anicraft_doc::Rgba, alpha: f32) -> anicraft_doc::ColorTransform {
    anicraft_doc::ColorTransform { mul: [0.0, 0.0, 0.0, alpha], add: [tint.r, tint.g, tint.b, 0.0] }
}
