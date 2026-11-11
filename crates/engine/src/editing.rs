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
            "{id?: instance element id, symbol?: library symbol id} opens a symbol's timeline for editing: the instance `id` (default: the selected instance) in place, or a Library symbol at the stage's centre. Every command then works inside it. With nothing to open while a symbol is open, goes back to the document (Animate's Ctrl+E)",
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

/// A symbol open for editing, and what to go back to.
#[derive(Clone, Debug, PartialEq)]
pub struct EditLevel {
    pub symbol: SymbolId,
    /// The instance it was opened from, on the timeline below (none: from the Library).
    pub instance: Option<(LayerId, ElementId)>,
    /// Stage space from the symbol's space: where the instance stands (through every level).
    pub transform: Affine,
    /// The timeline below as it was: its active layer, playhead and selection.
    pub layer: LayerId,
    pub frame: u32,
    pub selection: Selection,
}

/// The timeline being edited: the open symbol's, else the scene's.
pub(crate) fn edited<'a>(doc: &'a Document, scene: usize, editing: &[EditLevel]) -> &'a Timeline {
    match editing.last().and_then(|l| doc.symbol(l.symbol)) {
        Some(sym) => &sym.timeline,
        None => doc.timeline(scene),
    }
}

/// [`edited`] ([`Session::timeline`]), copy-on-write. Takes the fields, so a caller can keep using the session's others.
pub(crate) fn edited_mut<'a>(doc: &'a mut Document, scene: usize, editing: &[EditLevel]) -> &'a mut Timeline {
    let open = editing.last().and_then(|l| doc.library.iter().position(|s| s.id == l.symbol));
    // (Borrowed only on the branch that returns it.)
    match open {
        Some(i) if i < doc.library.len() => &mut Arc::make_mut(&mut doc.library[i]).timeline,
        _ => doc.timeline_mut(scene),
    }
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

    /// The symbol open for editing, if any.
    pub fn editing_symbol(&self) -> Option<SymbolId> {
        self.editing.last().map(|l| l.symbol)
    }

    /// Closes levels whose symbol is gone (an undo, a Library deletion), from the first such.
    pub(crate) fn fix_editing(&mut self) {
        if let Some(i) = self.editing.iter().position(|l| self.doc.symbol(l.symbol).is_none()) {
            while self.editing.len() > i {
                self.exit_level();
            }
        }
    }

    fn enter_level(&mut self, symbol: SymbolId, instance: Option<(LayerId, ElementId)>, transform: Affine, frame: u32) {
        self.editing.push(EditLevel { symbol, instance, transform, layer: self.layer, frame: self.frame, selection: std::mem::take(&mut self.selection) });
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
        self.layer = l.layer;
        self.frame = l.frame;
        self.selection = match l.instance {
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
            (l.symbol, l.instance, l.frame, l.transform.m.map(f32::to_bits)).hash(&mut h);
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
                let below = match parent {
                    None => self.doc.timeline(self.scene).clone(),
                    Some(p) => self.doc.symbol(p.symbol).map(|s| s.timeline.clone()).unwrap_or_default(),
                };
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

/// Whether an instance of `sym` placed in the open symbol would put a symbol inside itself: `sym`
/// is open, or uses an open one, however deep.
pub(crate) fn would_nest(s: &Session, sym: SymbolId) -> bool {
    if s.editing.is_empty() {
        return false;
    }
    let mut seen: Vec<SymbolId> = Vec::new();
    let mut todo = vec![sym];
    while let Some(id) = todo.pop() {
        if s.editing.iter().any(|l| l.symbol == id) {
            return true;
        }
        if seen.contains(&id) || seen.len() > 100_000 {
            continue;
        }
        seen.push(id);
        let Some(sym) = s.doc.symbol(id) else { continue };
        for k in sym.timeline.layers.iter().flat_map(|l| l.keyframes.iter()) {
            todo.extend(k.elements.iter().filter_map(|e| match e.kind {
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
        if s.editing.iter().any(|l| l.symbol == symbol) {
            return Err(EngineError::State("that symbol is open already".into()));
        }
        let c = Vec2::new(s.doc.stage.width as f32 * 0.5, s.doc.stage.height as f32 * 0.5);
        s.enter_level(symbol, None, Affine::translate(c.x, c.y), 0);
        return Ok(editing_json(s));
    }
    let id = match opt_u64(p, "id")? {
        Some(i) => ElementId(i),
        None => {
            let picked: Vec<ElementId> = s.selection.on(s.layer).into_iter().filter(|id| instance(s, *id).is_some_and(|(_, e)| matches!(e.kind, ElementKind::Symbol { .. }))).collect();
            match picked.as_slice() {
                [one] => *one,
                // Nothing to open: Ctrl+E goes back to the document, as in Animate.
                [] if !s.editing.is_empty() => {
                    s.exit_all();
                    return Ok(editing_json(s));
                }
                [] => return Err(EngineError::State("select a symbol instance to edit it (or double-click it)".into())),
                _ => return Err(EngineError::State("select one symbol instance to edit".into())),
            }
        }
    };
    let (layer, el) = instance(s, id).ok_or_else(|| EngineError::State(format!("element {} is not on a layer here at this frame", id.0)))?;
    if !s.selectable(layer) {
        return Err(EngineError::State("that instance's layer is locked or hidden".into()));
    }
    let ElementKind::Symbol { symbol, loop_mode, first_frame, .. } = el.kind else { return Err(EngineError::State("that is not a symbol instance".into())) };
    let sym = s.doc.symbol(symbol).ok_or_else(|| EngineError::State("that instance's symbol is missing from the Library".into()))?;
    if s.editing.iter().any(|l| l.symbol == symbol) {
        return Err(EngineError::State("that symbol is open already".into()));
    }
    // A graphic opens at the frame the instance shows; movie clips and buttons at their first.
    let len = sym.timeline.frame_count();
    let start = s.timeline().layer(layer).and_then(|l| l.keyframe_at(s.frame)).map_or(s.frame, |k| k.start);
    let frame = if sym.kind == SymbolKind::Graphic { loop_mode.local_frame(s.frame.saturating_sub(start), first_frame, len) } else { 0 };
    let below = s.editing.last().map_or(Affine::IDENTITY, |l| l.transform);
    s.enter_level(symbol, Some((layer, id)), below.mul(&el.transform), frame);
    Ok(editing_json(s))
}

fn exit(s: &mut Session, p: &Value) -> Result<Value> {
    if s.editing.is_empty() {
        return Err(EngineError::State("no symbol is open for editing".into()));
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
    let list: Vec<Value> = s.editing.iter().map(|l| json!({"symbol": l.symbol.0, "name": s.doc.symbol(l.symbol).map(|x| x.name.clone()), "instance": l.instance.map(|(_, e)| e.0)})).collect();
    json!({"editing": list, "layer": s.layer.0, "frame": s.frame, "selected": s.selection.id_list()})
}

/// Every colour of the hit area replaced by `tint`, the whole faded to `alpha` (ADR 0057): what
/// Animate's Advanced effect calls multipliers 0 % and offsets of the tint, alpha at `alpha`.
fn hit_tint(tint: anicraft_doc::Rgba, alpha: f32) -> anicraft_doc::ColorTransform {
    anicraft_doc::ColorTransform { mul: [0.0, 0.0, 0.0, alpha], add: [tint.r, tint.g, tint.b, 0.0] }
}
