//! The selection (ADR 0044): elements at the playhead, each named with its layer, so one
//! selection holds artwork on several layers as in Animate.
//!
//! Commands see the selection in one of three ways:
//! * **per layer** ([`Session::each_layer`]): the command's single-layer body runs once for every
//!   layer that has selected items, with that layer active and the selection narrowed to it
//!   (move, delete, duplicate, arrange, filters, colour effects, break apart, …). A selection on
//!   the active layer alone runs the body exactly as before;
//! * **across** the layers: the command reads every selected item with its layer (align and
//!   distribute, the Free Transform box, copy, convert to symbol, snapping a move);
//! * **on the active layer** only, where Animate does the same (Subselection, the Width tool,
//!   the Eraser's selected fills): [`Selection::on`] with [`Session::layer`].

use crate::{EngineError, Result, Session};
use anicraft_doc::{Element, ElementId, LayerId, LayerKind, evaluate_layer_elements};
use anicraft_geom::Rect;
use serde_json::{Map, Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

/// The selected elements, in the order they were selected, each with its layer.
#[derive(Clone, Debug, Default)]
pub struct Selection {
    items: Vec<(LayerId, ElementId)>,
    /// The same pairs, for constant-time membership.
    set: HashSet<(LayerId, ElementId)>,
}

impl PartialEq for Selection {
    fn eq(&self, other: &Self) -> bool {
        self.items == other.items
    }
}

impl Eq for Selection {}

impl Selection {
    /// `ids` on `layer`.
    pub fn only(layer: LayerId, ids: impl IntoIterator<Item = ElementId>) -> Selection {
        let mut s = Selection::default();
        s.extend_on(layer, ids);
        s
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.set.clear();
    }

    /// Every `(layer, element)`, in selection order.
    pub fn iter(&self) -> impl Iterator<Item = (LayerId, ElementId)> + '_ {
        self.items.iter().copied()
    }

    /// Every selected element id (ids are unique within a document), in selection order.
    pub fn ids(&self) -> impl Iterator<Item = ElementId> + '_ {
        self.items.iter().map(|(_, e)| *e)
    }

    /// The ids as plain numbers, for command results.
    pub fn id_list(&self) -> Vec<u64> {
        self.ids().map(|e| e.0).collect()
    }

    pub fn first(&self) -> Option<(LayerId, ElementId)> {
        self.items.first().copied()
    }

    /// Whether `id` on `layer` is selected.
    pub fn contains(&self, layer: LayerId, id: ElementId) -> bool {
        self.set.contains(&(layer, id))
    }

    /// Whether `id` is selected on any layer.
    pub fn has_id(&self, id: ElementId) -> bool {
        self.items.iter().any(|(_, e)| *e == id)
    }

    /// The selected ids on `layer`, in selection order.
    pub fn on(&self, layer: LayerId) -> Vec<ElementId> {
        self.items.iter().filter(|(l, _)| *l == layer).map(|(_, e)| *e).collect()
    }

    /// The part of the selection on `layer`.
    pub fn narrowed(&self, layer: LayerId) -> Selection {
        Selection::only(layer, self.on(layer))
    }

    /// The layers with selected items, in the order they were first selected on.
    pub fn layers(&self) -> Vec<LayerId> {
        let mut out: Vec<LayerId> = Vec::new();
        for (l, _) in &self.items {
            if !out.contains(l) {
                out.push(*l);
            }
        }
        out
    }

    /// Adds `id` on `layer`; false when it already was selected.
    pub fn insert(&mut self, layer: LayerId, id: ElementId) -> bool {
        if !self.set.insert((layer, id)) {
            return false;
        }
        self.items.push((layer, id));
        true
    }

    pub fn extend_on(&mut self, layer: LayerId, ids: impl IntoIterator<Item = ElementId>) {
        for id in ids {
            self.insert(layer, id);
        }
    }

    pub fn remove(&mut self, layer: LayerId, id: ElementId) -> bool {
        if !self.set.remove(&(layer, id)) {
            return false;
        }
        self.items.retain(|x| *x != (layer, id));
        true
    }

    /// Selects `id` on `layer`, or lets go of it when it was selected (Shift-click).
    pub fn toggle(&mut self, layer: LayerId, id: ElementId) {
        if !self.remove(layer, id) {
            self.insert(layer, id);
        }
    }

    pub fn retain(&mut self, mut keep: impl FnMut(LayerId, ElementId) -> bool) {
        let set = &mut self.set;
        self.items.retain(|&(l, e)| {
            let k = keep(l, e);
            if !k {
                set.remove(&(l, e));
            }
            k
        });
    }

    /// Replaces what is selected on `layer` with `ids`, leaving the other layers alone.
    pub fn set_on(&mut self, layer: LayerId, ids: impl IntoIterator<Item = ElementId>) {
        self.retain(|l, _| l != layer);
        self.extend_on(layer, ids);
    }
}

/// Result keys that are values, not counts: they are not summed over layers.
const VALUE_KEYS: &[&str] = &["filters"];

/// One result for a command that ran on several layers: lists are joined, counts added, other
/// values come from the first layer; `layers` lists the layers it ran on.
fn merge_results(results: Vec<(LayerId, Value)>) -> Value {
    let mut out = Map::new();
    for (_, v) in &results {
        let Value::Object(m) = v else { continue };
        for (k, x) in m {
            match (out.get_mut(k), x) {
                (None, _) => {
                    out.insert(k.clone(), x.clone());
                }
                (Some(Value::Array(a)), Value::Array(b)) => a.extend(b.iter().cloned()),
                (Some(Value::Number(a)), Value::Number(b)) if !VALUE_KEYS.contains(&k.as_str()) => {
                    if let (Some(p), Some(q)) = (a.as_u64(), b.as_u64()) {
                        *a = p.saturating_add(q).into();
                    }
                }
                _ => {}
            }
        }
    }
    out.insert("layers".into(), json!(results.iter().map(|(l, _)| l.0).collect::<Vec<_>>()));
    Value::Object(out)
}

impl Session {
    /// The elements on `layer` at the playhead (tweens applied).
    pub fn elements_on(&self, layer: LayerId) -> Vec<Arc<Element>> {
        self.timeline().layer(layer).and_then(|l| evaluate_layer_elements(l, self.frame)).map(|(_, e)| e.into_owned()).unwrap_or_default()
    }

    /// Whether artwork on `layer` can be selected: it is in this scene, shown and unlocked
    /// (folders included), and not a folder.
    pub fn selectable(&self, layer: LayerId) -> bool {
        let tl = self.timeline();
        let Some(i) = tl.layers.iter().position(|l| l.id == layer) else { return false };
        tl.layers.get(i).is_some_and(|l| l.visible && !l.locked && l.kind != LayerKind::Folder) && !tl.locked_by_folder(layer) && !tl.hidden_by_folder(i)
    }

    /// Whether a marquee or Select All gathers artwork from `layer`: a selectable layer with
    /// artwork on it. The camera layer holds only the camera, and an armature layer's
    /// instances are posed by their bones, so they are gathered only while active.
    pub fn gathers(&self, layer: LayerId) -> bool {
        let kind = self.timeline().layer(layer).map(|l| l.kind);
        self.selectable(layer) && (layer == self.layer || !matches!(kind, Some(LayerKind::Camera | LayerKind::Armature)))
    }

    /// The layers with selected items, top to bottom.
    pub fn selected_layers(&self) -> Vec<LayerId> {
        let mine = self.selection.layers();
        self.timeline().layers.iter().map(|l| l.id).filter(|l| mine.contains(l)).collect()
    }

    /// Every selected element as it looks at the playhead, with its layer: layers top to
    /// bottom, each in stacking order.
    pub fn selected_elements(&self) -> Vec<(LayerId, Arc<Element>)> {
        let mut out = Vec::new();
        for l in self.selected_layers() {
            out.extend(self.elements_on(l).into_iter().filter(|e| self.selection.contains(l, e.id)).map(|e| (l, e)));
        }
        out
    }

    /// Selects `ids` on the active layer, and nothing else.
    pub fn select_only(&mut self, ids: impl IntoIterator<Item = ElementId>) {
        self.selection = Selection::only(self.layer, ids);
    }

    /// Stage-space bounds of element `id` on `layer` at the playhead.
    pub fn element_bounds_on(&self, layer: LayerId, id: ElementId) -> Rect {
        let mut r = Rect::EMPTY;
        for item in self.display_list().items.iter().filter(|i| i.element == id && i.layer == layer) {
            let pad = item.stroke.as_ref().map_or(0.0, anicraft_doc::Stroke::reach);
            r = r.union(&item.path.bounds().inflate(pad).transformed(&item.transform));
        }
        r
    }

    /// Bounds of the whole selection, over every layer (the Free Transform box).
    pub fn selection_bounds(&self) -> Rect {
        let sel: HashSet<(LayerId, ElementId)> = self.selection.iter().collect();
        let mut r = Rect::EMPTY;
        for item in self.display_list().items.iter().filter(|i| sel.contains(&(i.layer, i.element))) {
            let pad = item.stroke.as_ref().map_or(0.0, anicraft_doc::Stroke::reach);
            r = r.union(&item.path.bounds().inflate(pad).transformed(&item.transform));
        }
        r
    }

    /// Drops selected items that are gone at the playhead, or whose layer is gone, hidden or
    /// locked (a locked or hidden layer is never selected), and keeps the active layer valid.
    pub fn fix_selection(&mut self) {
        self.scene = self.scene.min(self.doc.scenes.len().saturating_sub(1));
        // A symbol that is gone closes (ADR 0045).
        self.fix_editing();
        if self.timeline().layer(self.layer).is_none() {
            self.layer = self.timeline().layers.first().map_or(LayerId(0), |l| l.id);
        }
        if self.selection.is_empty() {
            return;
        }
        let mut present: HashMap<LayerId, HashSet<ElementId>> = HashMap::new();
        for l in self.selection.layers() {
            if self.selectable(l) {
                present.insert(l, self.elements_on(l).iter().map(|e| e.id).collect());
            }
        }
        self.selection.retain(|l, id| present.get(&l).is_some_and(|ids| ids.contains(&id)));
    }

    /// Runs a command's single-layer body once for every layer with selected items, top to
    /// bottom, with that layer active and the selection narrowed to it; the selection is then
    /// what the runs left selected, and the active layer stays. A selection on the active layer
    /// alone (or none) runs the body once, as before.
    ///
    /// `strict`: every layer must succeed (an error undoes the whole command). Otherwise a layer
    /// where the edit does not apply (no movie clips for a filter, no text to restyle) is left
    /// as it was, and the command fails only when it applies nowhere.
    pub(crate) fn each_layer(&mut self, strict: bool, mut run: impl FnMut(&mut Session) -> Result<Value>) -> Result<Value> {
        let layers = self.selected_layers();
        match layers.as_slice() {
            [] => return run(self),
            [one] if *one == self.layer => return run(self),
            _ => {}
        }
        let active = self.layer;
        let full = self.selection.clone();
        let mut after = Selection::default();
        let mut results = Vec::new();
        let mut first_err: Option<EngineError> = None;
        for l in layers {
            let before = (!strict).then(|| self.doc.clone());
            self.layer = l;
            self.selection = full.narrowed(l);
            // Each layer's run sees what the runs before it changed.
            self.dl_cache.replace(None);
            let r = run(self);
            // A motion tween's target on this layer turns into property keys now (the session
            // does this once per command, for one layer).
            if let Some(edit) = self.motion_edit.take()
                && r.is_ok()
            {
                crate::cmds_motion::finish_edit(self, edit);
            }
            self.dl_cache.replace(None);
            match r {
                Ok(v) => {
                    for (sl, id) in self.selection.iter() {
                        after.insert(sl, id);
                    }
                    results.push((l, v));
                }
                Err(e) if strict => {
                    self.layer = active;
                    return Err(e);
                }
                Err(e) => {
                    if let Some(d) = before {
                        self.doc = d;
                    }
                    for (sl, id) in full.narrowed(l).iter() {
                        after.insert(sl, id);
                    }
                    first_err.get_or_insert(e);
                }
            }
        }
        if self.timeline().layer(active).is_some() {
            self.layer = active;
        }
        self.selection = after;
        if results.is_empty() {
            return Err(first_err.unwrap_or_else(|| EngineError::State("nothing is selected".into())));
        }
        Ok(merge_results(results))
    }

    /// Runs `f` with `layer` active, then makes the active layer what it was.
    pub(crate) fn with_layer<T>(&mut self, layer: LayerId, f: impl FnOnce(&mut Session) -> T) -> T {
        let active = self.layer;
        self.layer = layer;
        self.dl_cache.replace(None);
        let r = f(self);
        if let Some(edit) = self.motion_edit.take() {
            crate::cmds_motion::finish_edit(self, edit);
        }
        self.dl_cache.replace(None);
        if self.timeline().layer(active).is_some() {
            self.layer = active;
        }
        r
    }
}
