//! AniCraft engine (layer L3): the session, undo history and the command registry.
//!
//! Every user-visible action is a [`CommandSpec`]. The egui shell, the CLI, the JSON-lines
//! control channel and future MCP server all call [`Session::execute`] with a command id and
//! JSON params, so headless and interactive behaviour cannot drift apart.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod cmds_draw;
mod cmds_file;
mod cmds_frames;
mod cmds_modify;
mod cmds_paint;
mod cmds_path;
mod cmds_timeline;
pub mod confirm;
pub mod format;
pub mod history;
pub mod render_service;
pub mod params;

use anicraft_doc::{DisplayList, Document, Element, ElementId, ElementKind, Keyframe, Layer, LayerId, evaluate, evaluate_layer_elements};
use anicraft_geom::{Rect, Vec2};
use serde_json::{Value, json};
use history::History;
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::Arc;

pub use anicraft_doc as doc;
pub use anicraft_geom as geom;
pub use anicraft_render as render;

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error("unknown command '{0}'")]
    UnknownCommand(String),
    #[error("{0}")]
    Param(String),
    #[error("{0}")]
    State(String),
    #[error("file format: {0}")]
    Format(String),
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("render: {0}")]
    Render(#[from] anicraft_render::RenderError),
    #[error("{0}")]
    NeedsConfirmation(String),
}

pub type Result<T> = std::result::Result<T, EngineError>;

/// Signature of a command implementation.
pub type RunFn = fn(&mut Session, &Value) -> Result<Value>;

/// Description of one command.
#[derive(Clone, Copy)]
pub struct CommandSpec {
    /// Stable id, e.g. `timeline.insert_keyframe`.
    pub id: &'static str,
    pub label: &'static str,
    /// Menu path, e.g. `Insert/Timeline`. Empty = not in a menu.
    pub menu: &'static str,
    pub shortcut: Option<&'static str>,
    /// Human-readable params description.
    pub params: &'static str,
    /// Whether it changes the document (and therefore records an undo step).
    pub mutates: bool,
    pub run: RunFn,
}

impl CommandSpec {
    pub fn to_json(&self) -> Value {
        json!({ "id": self.id, "label": self.label, "menu": self.menu, "shortcut": self.shortcut, "params": self.params, "mutates": self.mutates })
    }
}

/// All commands, in menu order.
pub fn commands() -> Vec<CommandSpec> {
    let mut v = Vec::new();
    v.extend(cmds_file::specs());
    v.extend(cmds_timeline::specs());
    v.extend(cmds_frames::specs());
    v.extend(cmds_draw::specs());
    v.extend(cmds_modify::specs());
    v.extend(cmds_path::specs());
    v.extend(cmds_paint::specs());
    v
}

/// Second keys for commands, `(keys, command id)`: Animate's where the primary key is the
/// platform's, and keys that differ by keyboard (a Mac's Delete key sends Backspace).
pub const SHORTCUT_ALIASES: &[(&str, &str)] = &[
    ("Backspace", "edit.delete"),
    ("Shift+,", "timeline.first_frame"),
    ("Shift+.", "timeline.last_frame"),
];

pub fn find_command(id: &str) -> Option<CommandSpec> {
    commands().into_iter().find(|c| c.id == id)
}

pub use history::{Snapshot, StateId};

/// Last addressable frame (0-based); about 70 minutes at 24 fps.
pub const MAX_FRAME: u32 = 100_000;

/// Drawing defaults used by the drawing commands when params omit colours.
#[derive(Clone, Copy, Debug)]
pub struct ToolStyle {
    pub fill: anicraft_doc::Rgba,
    pub stroke: anicraft_doc::Rgba,
    pub stroke_width: f32,
    pub use_fill: bool,
    pub use_stroke: bool,
}

impl Default for ToolStyle {
    fn default() -> Self {
        ToolStyle { fill: anicraft_doc::Rgba::hex(0x3d8bfd), stroke: anicraft_doc::Rgba::hex(0x1b2a4a), stroke_width: 2.0, use_fill: true, use_stroke: true }
    }
}

/// An open document plus everything the editor tracks about it.
pub struct Session {
    /// The document: a persistent tree, so cloning it (undo, render thread) is cheap.
    pub doc: Document,
    /// Playhead (0-based).
    pub frame: u32,
    /// Active layer.
    pub layer: LayerId,
    /// Selected elements on the active layer.
    pub selection: Vec<ElementId>,
    /// Bumped on every document change, undo and redo (UI invalidation).
    pub revision: u64,
    /// Identity of the current document content (equal ids ⇒ identical content).
    pub state: StateId,
    saved_state: Option<StateId>,
    pub path: Option<PathBuf>,
    pub style: ToolStyle,
    history: History,
    /// Open gesture (e.g. a drag): commands inside it coalesce into one undo step.
    gesture: Option<Snapshot>,
    /// Display list of `(state, frame)`, shared by hit testing, bounds and overlays.
    dl_cache: RefCell<Option<(StateId, u32, Arc<DisplayList>)>>,
    /// Copied elements (and the symbols they use), outside the document so undo leaves it alone.
    pub clipboard: cmds_modify::Clipboard,
    /// Copied frames (Copy Frames), separate from copied artwork as in Animate.
    pub frame_clipboard: cmds_frames::FrameClipboard,
}

impl Default for Session {
    fn default() -> Self {
        Session::new(Document::default())
    }
}

impl Session {
    pub fn new(mut doc: Document) -> Session {
        doc.sanitize();
        if doc.timeline.layers.is_empty() {
            let id = LayerId(doc.alloc_id());
            doc.timeline.layers.push(Arc::new(Layer::new(id, "Layer 1")));
        }
        let layer = doc.timeline.layers.first().map_or(LayerId(0), |l| l.id);
        let state = history::fresh_state();
        Session {
            doc,
            frame: 0,
            layer,
            selection: Vec::new(),
            revision: 1,
            state,
            saved_state: Some(state),
            path: None,
            style: ToolStyle::default(),
            history: History::default(),
            gesture: None,
            dl_cache: RefCell::new(None),
            clipboard: cmds_modify::Clipboard::default(),
            frame_clipboard: cmds_frames::FrameClipboard::default(),
        }
    }

    /// Whether the document differs from the last saved (or opened) state. Undoing back to
    /// the saved state makes it clean again.
    pub fn is_dirty(&self) -> bool {
        self.saved_state != Some(self.state)
    }

    /// Marks the current state as saved.
    pub fn mark_saved(&mut self) {
        self.saved_state = Some(self.state);
    }

    /// The confirmation running `id` with `params` needs right now (ADR 0004).
    pub fn confirmation(&self, id: &str, params: &Value) -> Option<confirm::Confirmation> {
        confirm::confirmation(self, id, params)
    }

    pub fn history(&self) -> &History {
        &self.history
    }

    /// Sets the undo memory budget (bytes); older steps are evicted beyond it.
    pub fn set_history_budget(&mut self, bytes: usize) {
        self.history.budget_bytes = bytes;
    }

    /// Starts coalescing: every mutating command until [`Session::end_gesture`] becomes a single
    /// undo step labelled `label` (used for drags, so a move is one step, not sixty).
    pub fn begin_gesture(&mut self, label: &str) {
        if self.gesture.is_none() {
            self.gesture = Some(self.snapshot(label));
        }
    }

    /// Ends the open gesture, recording one undo step if the document changed.
    pub fn end_gesture(&mut self) {
        if let Some(before) = self.gesture.take()
            && self.doc != before.doc
        {
            self.history.record(before, &self.doc);
        }
    }

    pub fn in_gesture(&self) -> bool {
        self.gesture.is_some()
    }

    /// Replaces the document (new/open), clearing history.
    pub fn replace_document(&mut self, doc: Document, path: Option<PathBuf>) {
        let style = self.style;
        let rev = self.revision;
        let budget = self.history.budget_bytes;
        let clipboard = std::mem::take(&mut self.clipboard);
        *self = Session::new(doc);
        // The clipboard survives New/Open, so artwork can move between documents.
        self.clipboard = clipboard;
        self.style = style;
        self.revision = rev + 1;
        self.path = path;
        self.history.budget_bytes = budget;
    }

    /// Runs a command by id. Mutating commands are atomic: on error the document is restored.
    pub fn execute(&mut self, id: &str, params: &Value) -> Result<Value> {
        let spec = find_command(id).ok_or_else(|| EngineError::UnknownCommand(id.to_string()))?;
        // Irreversible loss (unsaved work, files on disk) needs explicit consent, whoever asks.
        if let Some(c) = self.confirmation(id, params)
            && c.irreversible
            && !confirm::confirmed(params)
        {
            return Err(EngineError::NeedsConfirmation(format!("{} {} (pass \"confirm\": true to proceed)", c.title, c.message)));
        }
        if !spec.mutates {
            return (spec.run)(self, params);
        }
        let before = self.snapshot(spec.label);
        // A command must never observe a display list of the state it is changing.
        self.dl_cache.replace(None);
        let result = (spec.run)(self, params);
        self.dl_cache.replace(None);
        match result {
            Ok(v) => {
                // O(changed): unchanged subtrees compare by pointer.
                if self.doc != before.doc {
                    self.state = history::fresh_state();
                    self.revision += 1;
                    if self.gesture.is_none() {
                        self.history.record(before, &self.doc);
                    }
                }
                self.fix_selection();
                Ok(v)
            }
            Err(e) => {
                self.restore(before);
                Err(e)
            }
        }
    }

    fn snapshot(&self, label: &str) -> Snapshot {
        Snapshot { label: label.to_string(), doc: self.doc.clone(), state: self.state, frame: self.frame, layer: self.layer, selection: self.selection.clone(), bytes: 0 }
    }

    fn restore(&mut self, s: Snapshot) {
        self.doc = s.doc;
        self.state = s.state;
        self.frame = s.frame;
        self.layer = s.layer;
        self.selection = s.selection;
        self.dl_cache.replace(None);
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo() || self.gesture.as_ref().is_some_and(|g| g.doc != self.doc)
    }
    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }
    pub fn undo_label(&self) -> Option<&str> {
        self.history.undo_label()
    }
    pub fn history_len(&self) -> usize {
        self.history.undo_len()
    }

    pub fn undo(&mut self) -> Result<Value> {
        self.end_gesture();
        let cur = self.snapshot("");
        let s = self.history.undo(cur).ok_or_else(|| EngineError::State("nothing to undo".into()))?;
        let label = s.label.clone();
        self.restore(s);
        self.revision += 1;
        self.fix_selection();
        Ok(json!({ "undone": label }))
    }

    pub fn redo(&mut self) -> Result<Value> {
        self.end_gesture();
        let cur = self.snapshot("");
        let s = self.history.redo(cur).ok_or_else(|| EngineError::State("nothing to redo".into()))?;
        let label = s.label.clone();
        self.restore(s);
        self.revision += 1;
        self.fix_selection();
        Ok(json!({ "redone": label }))
    }

    /// Moves through history until `undo_len()` equals `target` (for a History panel click).
    pub fn travel_to(&mut self, target: usize) -> Result<Value> {
        let mut moved = 0usize;
        while self.history.undo_len() > target && self.history.can_undo() {
            self.undo()?;
            moved += 1;
        }
        while self.history.undo_len() < target && self.history.can_redo() {
            self.redo()?;
            moved += 1;
        }
        Ok(json!({ "steps": moved, "position": self.history.undo_len() }))
    }

    // ----- queries ---------------------------------------------------------------------------

    pub fn frame_count(&self) -> u32 {
        self.doc.frame_count()
    }

    pub fn active_layer(&self) -> Option<&Layer> {
        self.doc.timeline.layer(self.layer)
    }

    pub fn active_layer_index(&self) -> Option<usize> {
        self.doc.timeline.layers.iter().position(|l| l.id == self.layer)
    }

    /// The evaluated frame at the playhead, cached per `(state, frame)`.
    pub fn display_list(&self) -> Arc<DisplayList> {
        if let Some((s, f, dl)) = &*self.dl_cache.borrow()
            && *s == self.state
            && *f == self.frame
        {
            return dl.clone();
        }
        let dl = Arc::new(evaluate(&self.doc, self.frame));
        self.dl_cache.replace(Some((self.state, self.frame, dl.clone())));
        dl
    }

    /// Elements on the active layer at the playhead (tweens applied). Shares the document's
    /// element `Arc`s on non-tweened frames.
    pub fn visible_elements(&self) -> Vec<Arc<Element>> {
        self.active_layer().and_then(|l| evaluate_layer_elements(l, self.frame)).map(|(_, e)| e.into_owned()).unwrap_or_default()
    }

    /// Topmost element under `p` (stage space) on a visible, unlocked layer. A bounding-box
    /// test rejects almost every item before any curve is flattened.
    pub fn hit_test(&self, p: Vec2, slop: f32) -> Option<(LayerId, ElementId)> {
        let dl = self.display_list();
        for item in dl.items.iter().rev() {
            let hw = item.stroke.map_or(0.0, |s| s.width * 0.5 * item.transform.mean_scale());
            let bounds = item.path.bounds().transformed(&item.transform).inflate(hw + slop);
            if !bounds.contains(p) {
                continue;
            }
            let Some(layer) = self.doc.timeline.layer(item.layer) else { continue };
            if layer.locked {
                continue;
            }
            // Masked-away parts cannot be picked.
            if item.clip.is_some_and(|c| !dl.clip_contains(c, p)) {
                continue;
            }
            let inside = item.fill.is_some() && item.path.contains(p, &item.transform, item.fill_rule);
            if inside || item.path.distance_to_edge(p, &item.transform) <= hw + slop {
                return Some((item.layer, item.element));
            }
        }
        None
    }

    /// Stage-space bounds of a top-level element at the playhead.
    pub fn element_bounds(&self, id: ElementId) -> Rect {
        let mut r = Rect::EMPTY;
        for item in self.display_list().items.iter().filter(|i| i.element == id) {
            let pad = item.stroke.map_or(0.0, |s| s.width * 0.5);
            r = r.union(&item.path.bounds().inflate(pad).transformed(&item.transform));
        }
        r
    }

    /// Bounds of the whole selection.
    pub fn selection_bounds(&self) -> Rect {
        self.selection.iter().fold(Rect::EMPTY, |r, id| r.union(&self.element_bounds(*id)))
    }

    /// Drops selected ids that no longer exist on the active layer at the playhead.
    pub fn fix_selection(&mut self) {
        if self.doc.timeline.layer(self.layer).is_none() {
            self.layer = self.doc.timeline.layers.first().map_or(LayerId(0), |l| l.id);
        }
        if self.selection.is_empty() {
            return;
        }
        let ids: Vec<ElementId> = self.visible_elements().iter().map(|e| e.id).collect();
        self.selection.retain(|s| ids.contains(s));
    }

    pub fn set_frame(&mut self, frame: u32) {
        self.frame = frame.min(MAX_FRAME);
        self.fix_selection();
    }

    // ----- mutation helpers used by commands ------------------------------------------------

    pub(crate) fn layer_mut(&mut self, id: Option<LayerId>) -> Result<&mut Layer> {
        let id = id.unwrap_or(self.layer);
        self.doc.timeline.layer_mut(id).ok_or_else(|| EngineError::State(format!("no layer with id {}", id.0)))
    }

    /// The keyframe on the active layer that edits at the playhead should change. Creates a
    /// keyframe when the playhead is inside a tween (auto-key) or past the layer's end.
    pub(crate) fn editable_keyframe(&mut self) -> Result<&mut Keyframe> {
        let frame = self.frame;
        let layer = self.layer_mut(None)?;
        if layer.locked {
            return Err(EngineError::State(format!("layer '{}' is locked", layer.name)));
        }
        let needs_key = match layer.keyframe_at(frame) {
            None => true,
            Some(k) => k.tween.is_some() && k.start != frame,
        };
        if needs_key {
            // Inside a tween: capture the tweened state. On an empty frame: start blank.
            let copy = layer.keyframe_at(frame).is_some();
            cmds_timeline::insert_keyframe_at(layer, frame, copy)?;
        }
        let idx = layer.keyframe_index_at(frame).ok_or_else(|| EngineError::State("no keyframe at the playhead".into()))?;
        layer.keyframe_mut(idx).ok_or_else(|| EngineError::State("no keyframe at the playhead".into()))
    }

    /// Selected elements in the editable keyframe (copy-on-write: only these are cloned).
    pub(crate) fn selected_elements_mut(&mut self) -> Result<Vec<&mut Element>> {
        let sel = self.selection.clone();
        if sel.is_empty() {
            return Err(EngineError::State("nothing is selected".into()));
        }
        let kf = self.editable_keyframe()?;
        Ok(kf.elements_mut_where(|id| sel.contains(&id)))
    }

    /// JSON summary used by `doc.info` and the CLI.
    pub fn info(&self) -> Value {
        let layers: Vec<Value> = self
            .doc
            .timeline
            .layers
            .iter()
            .map(|l| {
                json!({
                    "id": l.id.0, "name": l.name, "visible": l.visible, "locked": l.locked,
                    "keyframes": l.keyframes.iter().map(|k| json!({"start": k.start, "duration": k.duration, "elements": k.elements.len(), "tween": k.tween.map(|t| t.easing.label())})).collect::<Vec<_>>()
                })
            })
            .collect();
        let symbols: Vec<Value> = self.doc.library.iter().map(|s| json!({"id": s.id.0, "name": s.name, "kind": format!("{:?}", s.kind), "frames": s.timeline.frame_count()})).collect();
        json!({
            "stage": {"width": self.doc.stage.width, "height": self.doc.stage.height, "fps": self.doc.stage.fps, "background": self.doc.stage.background.to_hex()},
            "frames": self.frame_count(),
            "frame": self.frame,
            "layer": self.layer.0,
            "selection": self.selection.iter().map(|e| e.0).collect::<Vec<_>>(),
            "layers": layers,
            "library": symbols,
            "revision": self.revision,
            "dirty": self.is_dirty(),
            "history": {"undo": self.history.undo_len(), "redo": self.history.redo_len(), "bytes": self.history.bytes(), "evicted": self.history.evicted},
            "state": self.state,
        })
    }
}

/// Short human label for an element (properties panel, library, CLI).
pub fn element_label(doc: &Document, e: &Element) -> String {
    let kind = match &e.kind {
        ElementKind::Shape(_) => "Shape".to_string(),
        ElementKind::Symbol { symbol, .. } => format!("Graphic: {}", doc.symbol(*symbol).map_or("missing", |s| s.name.as_str())),
    };
    if e.name.is_empty() { kind } else { format!("{} ({kind})", e.name) }
}

#[cfg(test)]
mod tests;
