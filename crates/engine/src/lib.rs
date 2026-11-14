//! AniCraft engine (layer L3): the session, undo history and the command registry.
//!
//! Every user-visible action is a [`CommandSpec`]. The egui shell, the CLI, the JSON-lines
//! control channel and future MCP server all call [`Session::execute`] with a command id and
//! JSON params, so headless and interactive behaviour cannot drift apart.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod anim;
mod cmds_actions;
mod cmds_bone;
mod cmds_warp;
pub mod cmds_brush;
mod cmds_camera;
mod cmds_draw;
pub mod cmds_filter;
pub mod cmds_ease;
pub mod cmds_envelope;
pub mod cmds_drop;
pub mod cmds_erase;
mod cmds_file;
mod cmds_frames;
mod cmds_group;
mod cmds_modify;
mod cmds_motion;
mod cmds_retime;
mod cmds_hints;
mod cmds_layer;
pub mod cmds_library;
pub mod cmds_paint;
mod cmds_path;
pub mod cmds_raw;
pub mod cmds_rawsub;
mod cmds_rawpoints;
pub mod brush_library;
pub mod filter_library;
pub mod cmds_recent;
pub mod cmds_template;
mod cmds_recovery;
mod cmds_script;
pub mod cmds_plugin;
mod cmds_scene;
pub mod cmds_sound;
pub mod cmds_text;
mod cmds_timeline;
mod cmds_view;
mod cmds_width;
mod cmds_stroke;
pub mod cmds_swatch;
pub mod confirm;
pub mod editing;
pub mod explorer;
pub mod find;
pub mod format;
pub mod history;
mod merge;
pub mod multi;
pub mod onion;
pub mod recovery;
pub mod render_service;
pub mod selection;
pub mod snap;
pub mod params;
pub mod plugin_export;
pub mod player;
pub mod publish;
pub mod vfs;

use anicraft_doc::{DisplayList, Document, Element, ElementId, ElementKind, Keyframe, Layer, LayerId, LayerKind, evaluate_layer_elements, evaluate_scene};
use anicraft_geom::{Affine, Rect, Vec2};
use serde_json::{Value, json};
use history::History;
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::Arc;

pub use anicraft_doc as doc;
pub use anicraft_plugins as plugins;
pub use anicraft_text as text;
pub use anicraft_geom as geom;
pub use anicraft_render as render;
/// Whether a path is an Animate document (`.fla`, `.xfl`, or an XFL folder) that `file.open`
/// imports.
pub use cmds_file::is_animate;

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
    /// A running publish was cancelled (ADR 0060).
    #[error("cancelled")]
    Cancelled,
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
    v.extend(plugin_export::specs());
    v.extend(cmds_envelope::specs());
    v.extend(publish::specs());
    v.extend(cmds_recent::specs());
    v.extend(cmds_drop::specs());
    v.extend(cmds_recovery::specs());
    v.extend(cmds_timeline::specs());
    v.extend(cmds_layer::specs());
    v.extend(cmds_camera::specs());
    v.extend(cmds_bone::specs());
    v.extend(cmds_warp::specs());
    v.extend(cmds_motion::specs());
    v.extend(cmds_retime::specs());
    v.extend(cmds_ease::specs());
    v.extend(cmds_hints::specs());
    v.extend(cmds_scene::specs());
    v.extend(cmds_frames::specs());
    v.extend(onion::specs());
    v.extend(multi::specs());
    v.extend(cmds_draw::specs());
    v.extend(cmds_view::specs());
    v.extend(cmds_group::specs());
    v.extend(cmds_library::specs());
    v.extend(merge::specs());
    v.extend(cmds_modify::specs());
    v.extend(cmds_path::specs());
    v.extend(cmds_raw::specs());
    v.extend(cmds_rawsub::specs());
    v.extend(cmds_rawpoints::specs());
    v.extend(editing::specs());
    v.extend(find::specs());
    v.extend(explorer::specs());
    v.extend(cmds_brush::specs());
    v.extend(cmds_filter::specs());
    v.extend(cmds_erase::specs());
    v.extend(cmds_paint::specs());
    v.extend(cmds_swatch::specs());
    v.extend(cmds_text::specs());
    v.extend(cmds_width::specs());
    v.extend(cmds_stroke::specs());
    v.extend(cmds_plugin::specs());
    v.extend(cmds_sound::specs());
    v.extend(cmds_actions::specs());
    v.extend(cmds_script::specs());
    v
}

/// Second keys for commands, `(keys, command id)`: Animate's where the primary key is the
/// platform's, and keys that differ by keyboard (a Mac's Delete key sends Backspace).
pub const SHORTCUT_ALIASES: &[(&str, &str)] = &[
    ("Backspace", "edit.delete"),
    ("Shift+,", "timeline.first_frame"),
    ("Shift+.", "timeline.last_frame"),
    // Snap to Objects is Animate's Ctrl+Shift+/ (ADR 0045). Where / and ? share a key (US, UK),
    // the key arrives as ?, so that spelling is bound too.
    ("Ctrl+Shift+?", "draw.snap_to_objects"),
];

pub fn find_command(id: &str) -> Option<CommandSpec> {
    commands().into_iter().find(|c| c.id == id)
}

pub use history::{Snapshot, StateId};
pub use selection::Selection;
pub use snap::{Accuracy, SnapTo, ViewAids};
pub use cmds_layer::LAYER_COLORS;

/// The command namespaces that change a timeline's layers and frames, refused while a group is
/// open for editing (ADR 0073): a group is one layer and one frame. A name ending in `.` is a
/// namespace, any other one command.
const GROUP_REFUSES: &[&str] = &["timeline.", "layer.", "camera.", "bone.", "motion.", "tween.", "frames.", "sound.place", "script.set", "script.clear"];

/// Last addressable frame (0-based); about 70 minutes at 24 fps.
pub const MAX_FRAME: u32 = 100_000;

/// The index of the topmost item of `dl` under `p` (stage space) that `ok` accepts (given its
/// index). A bounding-box test rejects almost every item before any curve is flattened;
/// masked-away parts cannot be picked.
pub(crate) fn pick_index(dl: &DisplayList, p: Vec2, slop: f32, ok: impl Fn(usize, &anicraft_doc::DisplayItem) -> bool) -> Option<usize> {
    for (i, item) in dl.items.iter().enumerate().rev() {
        let hw = item.stroke.as_ref().map_or(0.0, |s| s.reach() * item.transform.mean_scale());
        let bounds = item.path.bounds().transformed(&item.transform).inflate(hw + slop);
        if !bounds.contains(p) || !ok(i, item) {
            continue;
        }
        if item.clip.is_some_and(|c| !dl.clip_contains(c, p)) {
            continue;
        }
        let inside = item.fill.is_some() && item.path.contains(p, &item.transform, item.fill_rule);
        if inside || item.path.distance_to_edge(p, &item.transform) <= hw + slop {
            return Some(i);
        }
    }
    None
}

/// Drawing defaults used by the drawing commands when params omit colours.
#[derive(Clone, Debug)]
pub struct ToolStyle {
    pub fill: anicraft_doc::Rgba,
    /// The fill swatch holds a gradient or a bitmap (set in the Colour panel or picked with the
    /// Eyedropper, ADR 0034 and 0074), in stage space; `fill` is then its representative colour.
    /// `None`: the solid `fill`.
    pub fill_paint: Option<anicraft_doc::Paint>,
    /// Lock Fill (ADR 0074): a gradient or bitmap from the swatch is poured where it lies on the
    /// stage, so it runs on across shapes. Off, each shape gets it fitted to its own bounds.
    pub lock_fill: bool,
    pub stroke: anicraft_doc::Rgba,
    pub stroke_width: f32,
    pub use_fill: bool,
    pub use_stroke: bool,
    /// The width profile new strokes get (ADR 0018).
    pub stroke_profile: anicraft_doc::ProfilePreset,
    /// A picked width profile that is none of the presets (it wins over `stroke_profile`).
    pub custom_profile: Option<anicraft_doc::WidthProfile>,
    /// The caps, joins and dashes new strokes get (ADR 0046).
    pub line: anicraft_doc::StrokeStyle,
    /// Object Drawing: new shapes are drawing objects. Off, they merge with the raw shapes they
    /// touch (ADR 0031). On by default, so a script's draw command makes one element; the editor
    /// starts with it off, as Animate does.
    pub object_drawing: bool,
    /// Snap to Objects (Animate's magnet, ADR 0042): dragged points and selections snap to
    /// points, lines and fill edges nearby. On by default; off, the `snap` params do nothing.
    pub snap_objects: bool,
}

impl Default for ToolStyle {
    fn default() -> Self {
        ToolStyle { fill: anicraft_doc::Rgba::hex(0x3d8bfd), fill_paint: None, lock_fill: false, stroke: anicraft_doc::Rgba::hex(0x1b2a4a), stroke_width: 2.0, use_fill: true, use_stroke: true, stroke_profile: anicraft_doc::ProfilePreset::Uniform, custom_profile: None, line: anicraft_doc::StrokeStyle::default(), object_drawing: true, snap_objects: true }
    }
}

impl ToolStyle {
    /// The fill swatch's paint, in stage space.
    pub fn fill_paint(&self) -> anicraft_doc::Paint {
        self.fill_paint.clone().unwrap_or(anicraft_doc::Paint::Solid(self.fill))
    }
    /// The fill swatch's paint for a shape whose space is `to_local` from the stage and whose
    /// outline has the local `bounds`: with Lock Fill, the stage-space paint carried into the
    /// shape; without, a gradient or bitmap fitted to the bounds (as Animate pours them).
    pub fn fill_paint_for(&self, to_local: &anicraft_geom::Affine, bounds: &anicraft_geom::Rect) -> anicraft_doc::Paint {
        let paint = self.fill_paint();
        if self.lock_fill || matches!(paint, anicraft_doc::Paint::Solid(_)) { paint.transformed(to_local) } else { paint.fitted(bounds) }
    }
    /// Sets the fill swatch to a solid colour (dropping a picked gradient or bitmap).
    pub fn set_fill_color(&mut self, c: anicraft_doc::Rgba) {
        self.fill = c;
        self.fill_paint = None;
    }
    /// The width profile new strokes get.
    pub fn profile(&self) -> Option<anicraft_doc::WidthProfile> {
        self.custom_profile.clone().or_else(|| self.stroke_profile.profile())
    }
    /// A new stroke of `color` and `width` with the current profile and line style.
    pub fn new_stroke(&self, color: anicraft_doc::Rgba, width: f32) -> anicraft_doc::Stroke {
        anicraft_doc::Stroke::new(color, width).with_profile(self.profile()).with_style(self.line.clone())
    }
    /// Sets the profile to a preset (dropping a picked custom one).
    pub fn set_profile(&mut self, p: anicraft_doc::ProfilePreset) {
        self.stroke_profile = p;
        self.custom_profile = None;
    }
}

/// An open document plus everything the editor tracks about it.
/// A display list and the `(state, scene, frame, open symbol)` it was evaluated for.
type CachedList = (StateId, usize, u32, Option<(editing::Opened, Option<(LayerId, ElementId)>, usize)>, Arc<DisplayList>);

pub struct Session {
    /// The document: a persistent tree, so cloning it (undo, render thread) is cheap.
    pub doc: Document,
    /// The scene being edited (index into `doc.scenes`).
    pub scene: usize,
    /// Symbols open for editing, outermost first (ADR 0045). While one is open, commands work on
    /// its timeline: `frame`, `layer` and `selection` are its.
    pub editing: Vec<editing::EditLevel>,
    /// Playhead within the scene (0-based).
    pub frame: u32,
    /// Active layer.
    pub layer: LayerId,
    /// Selected elements at the playhead, each with its layer (ADR 0044).
    pub selection: Selection,
    /// Bumped on every document change, undo and redo (UI invalidation).
    pub revision: u64,
    /// Identity of the current document content (equal ids ⇒ identical content).
    pub state: StateId,
    saved_state: Option<StateId>,
    pub path: Option<PathBuf>,
    pub style: ToolStyle,
    /// Whether the rulers, grid and guides show and snap (ADR 0076): the editor's, not the
    /// document's.
    pub aids: ViewAids,
    /// The style new text gets (the last one used).
    pub text_style: cmds_text::TextStyle,
    history: History,
    /// Open gesture (e.g. a drag): commands inside it coalesce into one undo step.
    gesture: Option<Snapshot>,
    /// Display list of `(state, scene, frame)`, shared by hit testing, bounds and overlays.
    dl_cache: RefCell<Option<CachedList>>,
    /// Copied elements (and the symbols they use), outside the document so undo leaves it alone.
    pub clipboard: cmds_modify::Clipboard,
    /// Copied frames (Copy Frames), separate from copied artwork as in Animate.
    pub frame_clipboard: cmds_frames::FrameClipboard,
    /// A copied ease (Copy Ease, ADR 0081), for Paste Ease.
    pub ease_clipboard: Option<anicraft_doc::Easing>,
    /// A command is editing a motion tween's target (auto-key, ADR 0016).
    motion_edit: Option<cmds_motion::MotionEdit>,
    /// Installed WebAssembly plug-ins (ADR 0017). They belong to the session, not the
    /// document: they survive New and Open.
    pub plugins: anicraft_plugins::Registry,
    /// A name for the undo step the running command records, set by the command
    /// (`plugin.run` names its step after the plug-in).
    pub(crate) step_label: Option<String>,
    /// A marquee's cut that nothing has used yet (ADR 0041).
    provisional: Option<Provisional>,
    /// Set by `raw.select_rect` when it cut: each piece and the shape it was cut from, and the
    /// pieces inside the marquee.
    pub(crate) pending_cut: Option<(CutOrigin, Vec<ElementId>)>,
    /// The open gesture began on a provisional cut: the document with the cut.
    gesture_cut: Option<Document>,
    /// How a button open for editing shows its hit area over its other states (ADR 0057): the
    /// tint and opacity, or `None` for no overlay (the default; the app sets it from its theme).
    /// View state, like the playhead: never in the document or its undo history.
    pub hit_area: Option<(anicraft_doc::Rgba, f32)>,
    /// The last picture of an open symbol in place, by its key (ADR 0045).
    picture_cache: RefCell<Option<(u64, Arc<editing::StagePicture>)>>,
    /// A publish this session runs: its Cancel and frame count, which exports check (ADR 0060).
    pub job: Option<Arc<publish::Control>>,
    /// Files this session's publishes wrote: publishing again replaces them without asking.
    pub published: std::collections::BTreeSet<PathBuf>,
    /// The files opened and saved last, and pinned favourites (ADR 0059). They belong to the
    /// session, not the document: they survive New and Open. The desktop app keeps them in its
    /// preferences.
    pub recent: cmds_recent::RecentFiles,
    /// The user's own brush presets (ADR 0068). Like recent files they belong to the session
    /// and survive New and Open; the desktop app keeps them in its preferences.
    pub brushes: brush_library::BrushLibrary,
    /// The user's filter presets (ADR 0091). Like brushes they belong to the session and
    /// survive New and Open; the desktop app keeps them in its preferences.
    pub filter_presets: filter_library::FilterLibrary,
    /// Copied filters (Copy Filters, ADR 0091), for Paste Filters.
    pub filter_clipboard: Option<Vec<anicraft_doc::FilterEntry>>,
    /// The open envelope (ADR 0080): view state over the shapes it bends, like a gesture.
    pub(crate) envelope: Option<cmds_envelope::Envelope>,
}

/// Each piece of a marquee's cut, with the shape it was cut from.
pub(crate) type CutOrigin = Vec<(ElementId, ElementId)>;

/// A marquee's cut that nothing has used yet (ADR 0041). In Animate a marquee only selects: the
/// drawing is cut where it crosses once the selection is moved or changed. Here the cut happens
/// at once (the parts must be elements to be selected) and records no undo step. It is taken
/// back as soon as the selection lets go of its pieces with nothing else changed; the first edit
/// that uses it records one step from before the cut.
struct Provisional {
    /// Before the cut.
    before: history::Snapshot,
    /// The document with the cut, and its state.
    cut: Document,
    state: history::StateId,
    /// Each piece and the shape it was cut from.
    origin: Vec<(ElementId, ElementId)>,
    /// The pieces inside the marquee(s): while one of them is selected, the cut is in use.
    inside: Vec<ElementId>,
}

impl Default for Session {
    fn default() -> Self {
        Session::new(Document::default())
    }
}

impl Session {
    pub fn new(mut doc: Document) -> Session {
        doc.sanitize();
        // Every scene has at least one layer to draw on.
        for i in 0..doc.scenes.len() {
            if doc.timeline(i).layers.is_empty() {
                let id = LayerId(doc.alloc_id());
                doc.timeline_mut(i).layers.push(Arc::new(Layer::new(id, "Layer 1")));
            }
        }
        let layer = doc.timeline(0).layers.first().map_or(LayerId(0), |l| l.id);
        let state = history::fresh_state();
        Session {
            doc,
            scene: 0,
            editing: Vec::new(),
            frame: 0,
            layer,
            selection: Selection::default(),
            revision: 1,
            state,
            saved_state: Some(state),
            path: None,
            style: ToolStyle::default(),
            aids: ViewAids::default(),
            text_style: cmds_text::TextStyle::default(),
            history: History::default(),
            gesture: None,
            dl_cache: RefCell::new(None),
            clipboard: cmds_modify::Clipboard::default(),
            frame_clipboard: cmds_frames::FrameClipboard::default(),
            ease_clipboard: None,
            motion_edit: None,
            plugins: anicraft_plugins::Registry::default(),
            step_label: None,
            provisional: None,
            pending_cut: None,
            gesture_cut: None,
            hit_area: None,
            picture_cache: RefCell::new(None),
            job: None,
            published: std::collections::BTreeSet::new(),
            recent: cmds_recent::RecentFiles::default(),
            brushes: brush_library::BrushLibrary::default(),
            filter_presets: filter_library::FilterLibrary::default(),
            filter_clipboard: None,
            envelope: None,
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

    /// Marks the document as not saved anywhere yet (a restored autosave), so the close guard
    /// and the title treat it as unsaved work.
    pub fn mark_unsaved(&mut self) {
        self.saved_state = None;
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
        if self.gesture.is_some() {
            return;
        }
        // On a marquee's cut, the step (if the gesture makes one) starts from before the cut.
        let cut = self.live_cut().map(|p| (history::Snapshot { label: label.to_string(), ..p.before.clone() }, p.cut.clone()));
        match cut {
            Some((before, doc)) => {
                self.gesture_cut = Some(doc);
                self.gesture = Some(before);
            }
            None => self.gesture = Some(self.snapshot(label)),
        }
    }

    /// Ends the open gesture, recording one undo step if the document changed.
    pub fn end_gesture(&mut self) {
        let cut = self.gesture_cut.take();
        let Some(before) = self.gesture.take() else { return };
        if let Some(cut) = &cut {
            if self.doc == *cut {
                // Nothing used the marquee's cut: it stays provisional.
                if let Some(p) = &self.provisional {
                    self.state = p.state;
                }
                return;
            }
            self.provisional = None;
        }
        if self.doc != before.doc {
            self.history.record(before, &self.doc);
        }
    }

    pub fn in_gesture(&self) -> bool {
        self.gesture.is_some()
    }

    /// Replaces the document (new/open), clearing history.
    pub fn replace_document(&mut self, doc: Document, path: Option<PathBuf>) {
        let style = std::mem::take(&mut self.style);
        let rev = self.revision;
        let budget = self.history.budget_bytes;
        let clipboard = std::mem::take(&mut self.clipboard);
        let plugins = std::mem::take(&mut self.plugins);
        let recent = std::mem::take(&mut self.recent);
        let brushes = std::mem::take(&mut self.brushes);
        let filter_presets = std::mem::take(&mut self.filter_presets);
        let filter_clipboard = self.filter_clipboard.take();
        *self = Session::new(doc);
        self.recent = recent;
        self.brushes = brushes;
        self.filter_presets = filter_presets;
        // Like copied artwork, copied filters carry across to the next document.
        self.filter_clipboard = filter_clipboard;
        // The clipboard survives New/Open, so artwork can move between documents.
        self.clipboard = clipboard;
        self.plugins = plugins;
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
        // A group open for editing is one layer and one frame (ADR 0073).
        if spec.mutates && self.editing_group() && GROUP_REFUSES.iter().any(|ns| if ns.ends_with('.') { id.starts_with(ns) } else { id == *ns }) {
            return Err(EngineError::State("a group has one layer and one frame: close it (Edit › Edit Document) to change the timeline, or convert it to a symbol (F8) to animate inside it".into()));
        }
        self.settle_cut(id, params);
        if !spec.mutates {
            let r = (spec.run)(self, params);
            self.settle_cut(id, params);
            self.hold_group_frame();
            return r;
        }
        // An edit on a marquee's cut records its step from before the cut (ADR 0041).
        let cut = self.live_cut().map(|p| (history::Snapshot { label: spec.label.to_string(), ..p.before.clone() }, p.cut.clone()));
        let rollback = cut.as_ref().map(|_| self.snapshot(spec.label));
        let mut before = match &cut {
            Some((b, _)) => b.clone(),
            None => self.snapshot(spec.label),
        };
        // A command must never observe a display list of the state it is changing.
        self.dl_cache.replace(None);
        self.pending_cut = None;
        self.motion_edit = None;
        self.step_label = None;
        let result = (spec.run)(self, params);
        // Edits to a motion tween's target become property keys (auto-key).
        if let Some(edit) = self.motion_edit.take()
            && result.is_ok()
        {
            cmds_motion::finish_edit(self, edit);
        }
        self.dl_cache.replace(None);
        if let Some(label) = self.step_label.take() {
            before.label = label;
        }
        let pending = self.pending_cut.take();
        match result {
            Ok(v) => {
                let changed = match &cut {
                    Some((_, doc)) => self.doc != *doc,
                    // O(changed): unchanged subtrees compare by pointer.
                    None => self.doc != before.doc,
                };
                if changed {
                    self.state = history::fresh_state();
                    self.revision += 1;
                    match pending.filter(|_| self.gesture.is_none()) {
                        // A marquee's cut: provisional, no step (yet).
                        Some((origin, inside)) => self.begin_cut(before, cut.is_some(), origin, inside),
                        None => {
                            if cut.is_some() {
                                self.provisional = None;
                            }
                            if self.gesture.is_none() && self.doc != before.doc {
                                self.history.record(before, &self.doc);
                            }
                        }
                    }
                }
                self.fix_selection();
                self.hold_group_frame();
                Ok(v)
            }
            Err(e) => {
                // Back to the cut, which stays provisional, or to before the command.
                self.restore(rollback.unwrap_or(before));
                Err(e)
            }
        }
    }

    /// Inside a group (ADR 0073) the playhead stays on its only frame.
    fn hold_group_frame(&mut self) {
        if self.frame != 0 && self.editing_group() {
            self.frame = 0;
            self.fix_selection();
        }
    }

    /// The marquee's cut, while nothing has changed since it.
    fn live_cut(&self) -> Option<&Provisional> {
        self.provisional.as_ref().filter(|p| p.state == self.state && self.gesture.is_none())
    }

    /// Keeps a marquee's cut provisional. `before` is the document before it; `chained` when
    /// it cut a provisional cut again (Shift), whose pieces then lead back to the shapes the
    /// first cut took.
    fn begin_cut(&mut self, before: history::Snapshot, chained: bool, origin: Vec<(ElementId, ElementId)>, inside: Vec<ElementId>) {
        let (mut origin, mut inside) = (origin, inside);
        if chained && let Some(p) = self.provisional.take() {
            inside.extend(p.inside.iter().copied());
            for o in &mut origin {
                if let Some(&(_, first)) = p.origin.iter().find(|(piece, _)| *piece == o.1) {
                    o.1 = first;
                }
            }
            let new: Vec<ElementId> = origin.iter().map(|o| o.0).collect();
            origin.extend(p.origin.into_iter().filter(|(piece, _)| !new.contains(piece)));
        }
        // A marquee alone leaves the document as it was saved.
        if self.saved_state == Some(before.state) {
            self.saved_state = Some(self.state);
        }
        self.provisional = Some(Provisional { before, cut: self.doc.clone(), state: self.state, origin, inside });
    }

    /// Takes back a provisional cut that the selection has let go of (none of the pieces inside
    /// the marquee is selected), and before saving, undo, redo or a new marquee.
    fn settle_cut(&mut self, id: &str, params: &Value) {
        if self.gesture.is_none() && self.provisional.as_ref().is_some_and(|p| p.state != self.state) {
            self.provisional = None;
        }
        let Some(p) = self.live_cut() else { return };
        let held = self.selection.ids().any(|s| p.inside.contains(&s));
        let fresh_marquee = id == "raw.select_rect" && params.get("add").and_then(Value::as_bool) != Some(true);
        if !held || fresh_marquee || id.starts_with("file.") || id == "edit.undo" || id == "edit.redo" {
            self.heal_cut();
        }
    }

    /// Puts the document back as it was before a provisional cut. Selected pieces become the
    /// shapes they were cut from.
    fn heal_cut(&mut self) {
        let Some(p) = self.provisional.take() else { return };
        if p.state != self.state || self.gesture.is_some() {
            return;
        }
        let mut sel = Selection::default();
        for (layer, id) in self.selection.iter() {
            let id = p.origin.iter().find(|(piece, _)| *piece == id).map_or(id, |o| o.1);
            sel.insert(layer, id);
        }
        if self.saved_state == Some(p.state) {
            self.saved_state = Some(p.before.state);
        }
        self.doc = p.before.doc;
        self.state = p.before.state;
        self.selection = sel;
        self.dl_cache.replace(None);
        self.revision += 1;
        self.fix_selection();
    }

    /// Runs a command inside another one (a plug-in's requests during `plugin.run`): checked
    /// and atomic like [`Session::execute`], but it records no undo step of its own, so the
    /// outer command stays one step.
    pub(crate) fn execute_nested(&mut self, id: &str, params: &Value) -> Result<Value> {
        let spec = find_command(id).ok_or_else(|| EngineError::UnknownCommand(id.to_string()))?;
        if let Some(c) = self.confirmation(id, params)
            && c.irreversible
        {
            return Err(EngineError::NeedsConfirmation(format!("{} {}", c.title, c.message)));
        }
        if !spec.mutates {
            return (spec.run)(self, params);
        }
        let before = self.snapshot("");
        self.dl_cache.replace(None);
        let result = (spec.run)(self, params);
        self.dl_cache.replace(None);
        self.pending_cut = None;
        match result {
            Ok(v) => {
                // Later requests must see this change (display list, hit tests): new state.
                if self.doc != before.doc {
                    self.state = history::fresh_state();
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

    /// Puts the document back where the open gesture started, keeping the gesture open. A
    /// live preview re-runs from the same start on every change, and Cancel leaves no step.
    pub fn revert_gesture(&mut self) {
        if self.gesture_cut.is_some() {
            // Back to before the marquee's cut: it is gone.
            self.provisional = None;
        }
        let Some(g) = &self.gesture else { return };
        if g.doc == self.doc && g.selection == self.selection {
            return;
        }
        let back = Snapshot { label: String::new(), doc: g.doc.clone(), state: g.state, scene: g.scene, editing: g.editing.clone(), frame: g.frame, layer: g.layer, selection: g.selection.clone(), bytes: 0 };
        self.restore(back);
        self.revision += 1;
        self.fix_selection();
    }

    fn snapshot(&self, label: &str) -> Snapshot {
        Snapshot { label: label.to_string(), doc: self.doc.clone(), state: self.state, scene: self.scene, editing: self.editing.clone(), frame: self.frame, layer: self.layer, selection: self.selection.clone(), bytes: 0 }
    }

    fn restore(&mut self, s: Snapshot) {
        self.doc = s.doc;
        self.state = s.state;
        self.scene = s.scene;
        // Where the change was made: an undo inside a symbol opens it again (ADR 0045).
        self.editing = s.editing;
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
        self.heal_cut();
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
        self.heal_cut();
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
        self.timeline().frame_count()
    }

    pub fn active_layer(&self) -> Option<&Layer> {
        self.timeline().layer(self.layer)
    }

    pub fn active_layer_index(&self) -> Option<usize> {
        self.timeline().layers.iter().position(|l| l.id == self.layer)
    }

    /// The current scene evaluated at the playhead, cached per `(state, scene, frame)`.
    pub fn display_list(&self) -> Arc<DisplayList> {
        let open = self.editing.last().map(|l| (l.open, l.instance, self.editing.len()));
        if let Some((s, sc, f, o, dl)) = &*self.dl_cache.borrow()
            && *s == self.state
            && *sc == self.scene
            && *f == self.frame
            && *o == open
        {
            return dl.clone();
        }
        // An open symbol (ADR 0045) or group (ADR 0073) is edited in its own space.
        let dl = Arc::new(if open.is_some() { anicraft_doc::evaluate_timeline(&self.doc, self.timeline(), self.frame) } else { evaluate_scene(&self.doc, self.scene, self.frame) });
        self.dl_cache.replace(Some((self.state, self.scene, self.frame, open, dl.clone())));
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
        // The timeline being edited: a symbol open in place picks in its own layers (ADR 0045).
        let tl = self.timeline();
        let dl = self.display_list();
        let i = pick_index(&dl, p, slop, |_, it| tl.layer(it.layer).is_some_and(|layer| !layer.locked && !tl.locked_by_folder(layer.id)))?;
        dl.items.get(i).map(|it| (it.layer, it.element))
    }

    /// Stage → output for layer `id` at the playhead as the camera shows it (ADR 0082): the
    /// camera and the layer's depth then, the identity for an attached layer. `None` without a
    /// camera that does something, while a symbol is open, or when the camera has passed the
    /// layer. What editing through the camera maps the pointer back with.
    pub fn camera_view(&self, id: LayerId) -> Option<Affine> {
        if !self.editing.is_empty() {
            return None;
        }
        let tl = self.doc.timeline(self.scene);
        let cam = tl.camera_at(self.frame, &self.doc.stage)?;
        cam.layer_view(&self.doc.stage, tl.layer(id)?, self.frame)
    }

    /// [`Session::hit_test`] through the scene's camera: the topmost element under output
    /// point `p` (the movie's pixels) on a visible, unlocked layer, `slop` in output pixels.
    /// Without a camera that does something this is the stage's hit test.
    pub fn hit_test_camera(&self, p: Vec2, slop: f32) -> Option<(LayerId, ElementId)> {
        if !self.editing.is_empty() || self.doc.timeline(self.scene).camera_at(self.frame, &self.doc.stage).is_none() {
            return self.hit_test(p, slop);
        }
        let tl = self.timeline();
        let dl = anicraft_doc::evaluate_scene_camera(&self.doc, self.scene, self.frame);
        let i = pick_index(&dl, p, slop, |_, it| tl.layer(it.layer).is_some_and(|layer| !layer.locked && !tl.locked_by_folder(layer.id)))?;
        dl.items.get(i).map(|it| (it.layer, it.element))
    }

    /// Stage-space bounds of a top-level element at the playhead.
    pub fn element_bounds(&self, id: ElementId) -> Rect {
        let mut r = Rect::EMPTY;
        for item in self.display_list().items.iter().filter(|i| i.element == id) {
            let pad = item.stroke.as_ref().map_or(0.0, anicraft_doc::Stroke::reach);
            r = r.union(&item.path.bounds().inflate(pad).transformed(&item.transform));
        }
        r
    }

    pub fn set_frame(&mut self, frame: u32) {
        self.frame = frame.min(MAX_FRAME);
        self.fix_selection();
    }

    // ----- mutation helpers used by commands ------------------------------------------------

    pub(crate) fn layer_mut(&mut self, id: Option<LayerId>) -> Result<&mut Layer> {
        let id = id.unwrap_or(self.layer);
        crate::editing::edited_mut(&mut self.doc, self.scene, &self.editing)?.layer_mut(id).ok_or_else(|| EngineError::State(format!("no layer with id {}", id.0)))
    }

    /// The keyframe on the active layer that edits at the playhead should change. Creates a
    /// keyframe when the playhead is inside a tween (auto-key) or past the layer's end.
    pub(crate) fn editable_keyframe(&mut self) -> Result<&mut Keyframe> {
        self.editable_keyframe_in(KeyEdit::Artwork)
    }

    /// [`Session::editable_keyframe`] for `what`: the camera layer holds only the camera (ADR
    /// 0027) and an armature layer only its posed armature (ADR 0043); every other edit is
    /// refused there.
    pub(crate) fn editable_keyframe_in(&mut self, what: KeyEdit) -> Result<&mut Keyframe> {
        let frame = self.frame;
        let tl = self.timeline();
        if let Some(l) = tl.layer(self.layer) {
            if l.kind == LayerKind::Folder {
                return Err(EngineError::State(format!("'{}' is a folder; pick a layer inside it to draw on", l.name)));
            }
            if (l.kind == LayerKind::Camera) != (what == KeyEdit::Camera) {
                return Err(EngineError::State(if what == KeyEdit::Camera { "that is not the camera layer".into() } else { format!("'{}' is the camera layer: it holds the camera, not artwork. Pick another layer to draw on, or move the camera with the Camera tool", l.name) }));
            }
            if (l.kind == LayerKind::Armature) != (what == KeyEdit::Pose) {
                return Err(EngineError::State(if what == KeyEdit::Pose { "that is not an armature layer".into() } else { format!("'{}' is an armature layer: it holds the armature's instances, posed by their bones. Drag a bone (or an instance) to pose it, drag the root instance to move it all, or pick another layer to draw on", l.name) }));
            }
            if tl.locked_by_folder(l.id) {
                return Err(EngineError::State(format!("layer '{}' is in a locked folder", l.name)));
            }
        }
        // Through the fields, not `layer_mut`, so `motion_edit` can be set below.
        let (scene, active) = (self.scene, self.layer);
        let layer = crate::editing::edited_mut(&mut self.doc, scene, &self.editing)?.layer_mut(active).ok_or_else(|| EngineError::State(format!("no layer with id {}", active.0)))?;
        if layer.locked {
            return Err(EngineError::State(format!("layer '{}' is locked", layer.name)));
        }
        // Inside a motion tween edits key properties, never the timeline.
        let needs_key = match layer.keyframe_at(frame) {
            None => true,
            Some(k) => k.motion.is_none() && k.tween.is_some() && k.start != frame,
        };
        if needs_key {
            // Inside a tween: capture the tweened state. On an empty frame: start blank.
            let copy = layer.keyframe_at(frame).is_some();
            cmds_timeline::insert_keyframe_at(layer, frame, copy)?;
        }
        let idx = layer.keyframe_index_at(frame).ok_or_else(|| EngineError::State("no keyframe at the playhead".into()))?;
        let lid = layer.id;
        let kf = layer.keyframe_mut(idx).ok_or_else(|| EngineError::State("no keyframe at the playhead".into()))?;
        // In a motion tween the command edits the target as it looks at the playhead; the
        // session turns what changed into property keys when the command is done.
        if self.motion_edit.is_none()
            && let Some(edit) = cmds_motion::begin_edit(lid, kf, frame)
        {
            self.motion_edit = Some(edit);
        }
        Ok(kf)
    }

    /// Selected elements in the editable keyframe (copy-on-write: only these are cloned).
    pub(crate) fn selected_elements_mut(&mut self) -> Result<Vec<&mut Element>> {
        let sel: std::collections::HashSet<ElementId> = self.selection.on(self.layer).into_iter().collect();
        if sel.is_empty() {
            return Err(EngineError::State("nothing is selected".into()));
        }
        let kf = self.editable_keyframe()?;
        Ok(kf.elements_mut_where(|id| sel.contains(&id)))
    }

    /// [`Session::selected_elements_mut`] for the properties an instance keeps however it is
    /// placed (colour effect, blending, filters). On an armature layer (ADR 0084) the selected
    /// members are edited in place on every pose of the armature, so the property holds as the
    /// armature moves and the bones and poses are untouched; elsewhere, as any edit.
    pub(crate) fn selected_props_mut(&mut self) -> Result<Vec<&mut Element>> {
        let on_armature = self.timeline().layer(self.layer).is_some_and(|l| l.kind == LayerKind::Armature);
        if !on_armature {
            return self.selected_elements_mut();
        }
        let sel: std::collections::HashSet<ElementId> = self.selection.on(self.layer).into_iter().collect();
        if sel.is_empty() {
            return Err(EngineError::State("nothing is selected".into()));
        }
        if self.timeline().locked_by_folder(self.layer) {
            return Err(EngineError::State("the armature layer is in a locked folder".into()));
        }
        let layer = self.layer_mut(None)?;
        if layer.locked {
            return Err(EngineError::State(format!("layer '{}' is locked", layer.name)));
        }
        Ok(layer.keyframes.iter_mut().flat_map(|k| Arc::make_mut(k).elements_mut_where(|id| sel.contains(&id))).collect())
    }

    /// JSON summary used by `doc.info` and the CLI.
    pub fn info(&self) -> Value {
        let layers: Vec<Value> = self
            .timeline()
            .layers
            .iter()
            .map(|l| {
                json!({
                    "id": l.id.0, "name": l.name, "visible": l.visible, "locked": l.locked, "volume": l.volume, "muted": l.muted,
                    "keyframes": l.keyframes.iter().map(|k| json!({"start": k.start, "duration": k.duration, "elements": k.elements.len(), "tween": k.tween.as_ref().map(|t| t.easing.label()), "motion": k.motion.as_ref().map(|m| m.tracks.iter().map(|t| t.property.key()).collect::<Vec<_>>()), "sound": k.sound.as_ref().map(|f| json!({"sound": f.sound.0, "sync": f.sync.key(), "repeat": f.repeat, "loop": f.looping, "effect": f.effect.key()})), "script": k.script.as_ref().map(anicraft_doc::FrameScript::describe)})).collect::<Vec<_>>()
                })
            })
            .collect();
        let symbols: Vec<Value> = self.doc.library.iter().map(|s| json!({"id": s.id.0, "name": s.name, "kind": format!("{:?}", s.kind), "frames": s.timeline.frame_count()})).collect();
        let sounds: Vec<Value> = self.doc.sounds.iter().map(|x| json!({"id": x.id.0, "name": x.name, "format": x.format.label(), "rate": x.rate, "channels": x.channels, "seconds": x.seconds()})).collect();
        json!({
            "stage": {"width": self.doc.stage.width, "height": self.doc.stage.height, "fps": self.doc.stage.fps, "background": self.doc.stage.background.to_hex()},
            "frames": self.frame_count(),
            "frame": self.frame,
            "scene": self.scene,
            "scenes": self.doc.scenes.iter().map(|s| json!({"name": s.name, "frames": s.timeline.frame_count()})).collect::<Vec<_>>(),
            "movie_frames": self.doc.frame_count(),
            "layer": self.layer.0,
            "selection": self.selection.id_list(),
            "selection_layers": self.selected_layers().iter().map(|l| l.0).collect::<Vec<_>>(),
            // The symbols open for editing, outermost first (ADR 0045); `layers` are the open one's.
            "editing": editing::editing_json(self)["editing"],
            "layers": layers,
            "library": symbols,
            "sounds": sounds,
            "revision": self.revision,
            "dirty": self.is_dirty(),
            "history": {"undo": self.history.undo_len(), "redo": self.history.redo_len(), "bytes": self.history.bytes(), "evicted": self.history.evicted},
            "state": self.state,
        })
    }
}

/// What [`Session::editable_keyframe_in`] is asked to edit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KeyEdit {
    /// Artwork: any layer but the camera and armature layers.
    Artwork,
    /// The camera, on the camera layer.
    Camera,
    /// A pose, on an armature layer.
    Pose,
}

/// A colour effect as commands report it (ADR 0056): its style and that style's values, in the
/// units `element.set_color_effect` takes (percent; Advanced offsets −255..255).
pub fn color_effect_json(fx: &anicraft_doc::ColorEffect) -> serde_json::Value {
    use anicraft_doc::ColorEffect;
    let r = |v: f32, k: f32| f64::from((v * k * 100.0).round() / 100.0);
    let mut v = serde_json::json!({"style": fx.style().key()});
    match *fx {
        ColorEffect::None => {}
        ColorEffect::Alpha(a) => v["alpha"] = r(a, 100.0).into(),
        ColorEffect::Brightness(b) => v["brightness"] = r(b, 100.0).into(),
        ColorEffect::Tint { color, amount } => {
            v["tint"] = color.to_hex().into();
            v["tint_amount"] = r(amount, 100.0).into();
        }
        ColorEffect::Advanced(t) => {
            for (i, ch) in ["red", "green", "blue", "alpha"].iter().enumerate() {
                v[format!("{ch}_multiplier")] = r(t.mul.get(i).copied().unwrap_or(1.0), 100.0).into();
                v[format!("{ch}_offset")] = r(t.add.get(i).copied().unwrap_or(0.0), 255.0).into();
            }
        }
    }
    v
}

/// Short human label for an element (properties panel, library, CLI).
pub fn element_label(doc: &Document, e: &Element) -> String {
    let kind = match &e.kind {
        ElementKind::Shape(_) => "Shape".to_string(),
        ElementKind::Symbol { symbol, .. } => match doc.symbol(*symbol) {
            Some(s) => format!("{}: {}", match s.kind {
                anicraft_doc::SymbolKind::Graphic => "Graphic",
                anicraft_doc::SymbolKind::MovieClip => "Movie clip",
                anicraft_doc::SymbolKind::Button => "Button",
            }, s.name),
            None => "Graphic: missing".to_string(),
        },
        ElementKind::Camera => "Camera".to_string(),
        ElementKind::Group { .. } => "Group".to_string(),
        // An empty field (an input field waiting for the viewer) is named by its type.
        ElementKind::Text(t) if t.text.trim().is_empty() => match t.kind() {
            anicraft_doc::TextKind::Static => "Static text",
            anicraft_doc::TextKind::Dynamic => "Dynamic text",
            anicraft_doc::TextKind::Input => "Input text",
        }
        .to_string(),
        ElementKind::Text(t) => {
            let first: String = t.text.lines().next().unwrap_or("").chars().take(24).collect();
            format!("Text: {first}{}", if t.text.chars().count() > first.chars().count() { "…" } else { "" })
        }
    };
    if e.name.is_empty() { kind } else { format!("{} ({kind})", e.name) }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod motion_tests;
#[cfg(test)]
mod retime_tests;
#[cfg(test)]
mod ease_tests;
#[cfg(test)]
mod color_tests;
#[cfg(test)]
mod motion_path_tests;
#[cfg(test)]
mod motion_pin_tests;
#[cfg(test)]
mod plugin_tests;
#[cfg(test)]
mod plugin_kinds_tests;
#[cfg(test)]
mod tests_sound;
#[cfg(test)]
mod hint_tests;
#[cfg(test)]
mod tests_actions;
#[cfg(test)]
mod tests_script;
#[cfg(test)]
mod tests_camera;
#[cfg(test)]
mod tests_bone;
#[cfg(test)]
mod tests_bone_spring;
#[cfg(test)]
mod tests_shape_bone;
#[cfg(test)]
mod tests_shape_weights;
#[cfg(test)]
mod tests_warp;
#[cfg(test)]
mod tests_filter;
#[cfg(test)]
mod tests_filter_tracks;
#[cfg(test)]
mod merge_tests;
#[cfg(test)]
mod raw_tests;
#[cfg(test)]
mod rawsub_tests;
#[cfg(test)]
mod rawpoints_tests;
#[cfg(test)]
mod editing_tests;
#[cfg(test)]
mod brush_tests;
#[cfg(test)]
mod brush_library_tests;
#[cfg(test)]
mod web_tests;
#[cfg(test)]
mod erase_tests;
#[cfg(test)]
mod eyedropper_tests;
#[cfg(test)]
mod gradient_tests;
#[cfg(test)]
mod selection_tests;
#[cfg(test)]
mod scale_grid_tests;
#[cfg(test)]
mod drop_tests;
#[cfg(test)]
mod template_tests;
#[cfg(test)]
mod timeline_drop_tests;
#[cfg(test)]
mod timeline_key_tests;
#[cfg(test)]
mod tests_bone_chain;
#[cfg(test)]
mod view_tests;
#[cfg(test)]
mod path_text_tests;
#[cfg(test)]
mod group_tests;
#[cfg(test)]
mod swatch_tests;
#[cfg(test)]
mod parity_tests;
