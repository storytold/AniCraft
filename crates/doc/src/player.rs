//! The interactive player (ADR 0013, ADR 0024): playback with frame actions, frame scripts and
//! button events, as a pure function of the document.
//!
//! [`PlayerState`] holds what changes while a movie plays: the main timeline's scene, frame and
//! playing flag, each movie clip instance's playhead, the properties actions have set, and the
//! pointer. [`step`] advances it by one frame: every playing timeline moves on, the actions on
//! the keyframes they enter run, then that frame's pointer events go to the buttons under the
//! pointer and run their handlers. [`crate::evaluate_player`] draws a state.
//!
//! Nothing here changes the document, so the authoring stage and every export stay
//! script-free; and nothing reads a clock or a random source, so the same document, state and
//! events always give the same next state. Budgets bound what one step may do (actions run,
//! frames entered, instances walked, output lines), so a `goto` loop ends in a message instead
//! of a hang.

use crate::actions::{Action, ButtonEvent, EventHandler, FrameRef, FrameScript, PluginScript, Property, Seg, Target};
use crate::eval::{Play, evaluate_guided, evaluate_symbol, guide_walk};
use crate::sound::Started;
use crate::{Document, Element, ElementId, ElementKind, LayerKind, MAX_SYMBOL_DEPTH, SymbolId, SymbolKind, Timeline};
use anicraft_geom::{Affine, Vec2};
use std::collections::{BTreeMap, VecDeque};

/// An instance's place in the movie: the element ids from the main timeline down to it (empty
/// for the main timeline itself). Element ids stay the same across keyframes that hold the same
/// instance, so a movie clip keeps its playhead while it stays on the stage.
pub type InstancePath = Vec<ElementId>;

/// Most actions one step runs.
pub const MAX_ACTIONS_PER_STEP: usize = 10_000;
/// Most timeline frames one step enters (a `goto` enters one too).
pub const MAX_ENTERS_PER_STEP: usize = 1_000;
/// Most symbol instances the player follows.
pub const MAX_NODES: usize = 10_000;
/// Most output lines one step keeps (more are counted).
pub const MAX_OUTPUT_PER_STEP: usize = 1_000;
/// Most pointer events one step handles.
pub const MAX_EVENTS_PER_STEP: usize = 256;

/// A timeline's playhead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClipState {
    pub frame: u32,
    pub playing: bool,
}

impl Default for ClipState {
    fn default() -> Self {
        ClipState { frame: 0, playing: true }
    }
}

/// Properties actions set on an instance; each replaces what the timeline gives it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Overrides {
    pub x: Option<f32>,
    pub y: Option<f32>,
    pub alpha: Option<f32>,
    /// Degrees.
    pub rotation: Option<f32>,
    pub scale_x: Option<f32>,
    pub scale_y: Option<f32>,
    pub visible: Option<bool>,
}

impl Overrides {
    pub fn shown(&self) -> bool {
        self.visible != Some(false)
    }

    pub fn set(&mut self, p: Property, value: f64) {
        let (lo, hi) = p.range();
        let v = if value.is_finite() { value.clamp(lo, hi) as f32 } else { return };
        match p {
            Property::X => self.x = Some(v),
            Property::Y => self.y = Some(v),
            Property::Alpha => self.alpha = Some(v),
            Property::Visible => self.visible = Some(v != 0.0),
            Property::Rotation => self.rotation = Some(v),
            Property::ScaleX => self.scale_x = Some(v),
            Property::ScaleY => self.scale_y = Some(v),
        }
    }

    /// `el` with these properties in place of its own.
    pub fn apply(&self, el: &Element) -> Element {
        let mut e = el.clone();
        if self.x.is_some() || self.y.is_some() || self.rotation.is_some() || self.scale_x.is_some() || self.scale_y.is_some() {
            let mut d = e.transform.decompose();
            d.tx = self.x.unwrap_or(d.tx);
            d.ty = self.y.unwrap_or(d.ty);
            d.rotation = self.rotation.map_or(d.rotation, f32::to_radians);
            d.sx = self.scale_x.unwrap_or(d.sx);
            d.sy = self.scale_y.unwrap_or(d.sy);
            let m = d.compose();
            if m.is_finite() {
                e.transform = m;
            }
        }
        if let Some(a) = self.alpha {
            e.color.alpha = a;
        }
        e
    }
}

/// Everything that changes while a movie plays.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PlayerState {
    /// The scene the main timeline is in, and its playhead there.
    pub scene: usize,
    pub root: ClipState,
    /// Each movie clip instance on the stage.
    pub clips: BTreeMap<InstancePath, ClipState>,
    /// Properties actions have set, per instance.
    pub props: BTreeMap<InstancePath, Overrides>,
    /// The pointer in stage space (`None` when it is outside the movie), and whether it is down.
    pub pointer: Option<Vec2>,
    pub down: bool,
    /// The button (or movie clip with event handlers) under the pointer, and the one the
    /// pointer went down on.
    pub hover: Option<InstancePath>,
    pub pressed: Option<InstancePath>,
    /// Steps taken since the start.
    pub tick: u64,
}

impl PlayerState {
    /// The frame a movie clip instance shows (`len` frames long).
    pub fn clip_frame(&self, path: &[ElementId], len: u32) -> u32 {
        self.clips.get(path).map_or(0, |c| c.frame).min(len.max(1) - 1)
    }

    /// The frame a button shows: Up (0), Over (1) under the pointer, Down (2) while pressed on
    /// it; a button whose timeline is shorter shows its last frame.
    pub fn button_frame(&self, path: &[ElementId], len: u32) -> u32 {
        let over = self.hover.as_deref() == Some(path);
        let f = match (over, self.down && self.pressed.as_deref() == Some(path)) {
            (true, true) => 2,
            (true, false) => 1,
            (false, _) => 0,
        };
        f.min(len.max(1) - 1)
    }

    /// A timeline's playhead: the main timeline's for the empty path.
    pub fn playhead(&self, path: &[ElementId]) -> Option<ClipState> {
        if path.is_empty() { Some(self.root) } else { self.clips.get(path).copied() }
    }
}

/// A pointer event, in stage coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event {
    Move(Vec2),
    Down(Vec2),
    Up(Vec2),
    /// The pointer left the movie.
    Leave,
}

/// The state after a step, what the movie wrote to the Output panel (traces, and why an
/// action could not run), and the sounds of the keyframes it entered (ADR 0039).
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub state: PlayerState,
    pub output: Vec<String>,
    pub sounds: Vec<Started>,
}

/// Most sounds one step starts.
pub const MAX_SOUNDS_PER_STEP: usize = 256;

/// What a node of the movie's instance tree is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    Root,
    Graphic,
    MovieClip,
    Button,
}

/// One symbol instance on the stage (or the main timeline), as the player sees it at a state.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub path: InstancePath,
    /// The index of the node it is inside.
    pub parent: Option<usize>,
    /// The index of the timeline it belongs to (the main timeline or a movie clip; graphics
    /// are see-through, as in Animate): where names are looked up and its handlers run.
    pub owner: usize,
    pub name: String,
    pub symbol: Option<SymbolId>,
    pub kind: NodeKind,
    /// The frame it shows, and its timeline's length.
    pub frame: u32,
    pub frames: u32,
    /// Local → stage.
    pub xf: Affine,
    /// Pointer events can reach it (visible, not mask artwork).
    pub hittable: bool,
    pub events: Vec<EventHandler>,
    /// The volume its sounds get: the layers holding it, from the main timeline down.
    pub gain: f32,
}

/// Runs plug-in frame scripts for the player (the engine implements it over the plug-in host).
pub trait ScriptHost {
    /// A new step starts (per-step budgets reset here).
    fn begin_step(&mut self) {}
    /// Runs `script` for the timeline described by `cx`; returns the actions it emitted.
    fn run(&mut self, script: &PluginScript, cx: &ScriptContext<'_>) -> ScriptRun;
}

/// What a frame script sees.
pub struct ScriptContext<'a> {
    pub doc: &'a Document,
    pub state: &'a PlayerState,
    /// Every instance on the stage (index 0 is the main timeline).
    pub nodes: &'a [Node],
    /// The index in `nodes` of the timeline whose keyframe holds the script.
    pub timeline: usize,
    /// The frame being entered, and the label there (empty when none).
    pub frame: u32,
    pub label: &'a str,
}

/// A frame script's result.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScriptRun {
    pub actions: Vec<Action>,
    pub output: Vec<String>,
}

/// A host without plug-ins: frame scripts are reported, not run.
pub struct NoScripts;

impl ScriptHost for NoScripts {
    fn run(&mut self, script: &PluginScript, _: &ScriptContext<'_>) -> ScriptRun {
        ScriptRun { actions: Vec::new(), output: vec![format!("Frame script {} · {} not run: no plug-ins here", script.id, script.entry)] }
    }
}

/// The movie at its first frame, with the first frame's actions run.
pub fn start(doc: &Document, host: &mut dyn ScriptHost) -> Step {
    host.begin_step();
    let mut r = Run::new(doc, PlayerState::default(), host);
    r.refresh();
    r.pending.push_front(Vec::new());
    r.settle();
    r.finish()
}

/// One frame on from `state`: timelines that play advance (and run the actions of the frames
/// they enter), then `events` are handled in order.
pub fn step(doc: &Document, state: &PlayerState, events: &[Event], host: &mut dyn ScriptHost) -> Step {
    host.begin_step();
    let mut r = Run::new(doc, state.clone(), host);
    r.st.tick = r.st.tick.saturating_add(1);
    r.refresh();
    r.remember_graphics();
    r.advance();
    r.settle();
    for e in events.iter().take(MAX_EVENTS_PER_STEP) {
        r.event(*e);
    }
    r.finish()
}

/// Pointer events between frames: routed to buttons (and their handlers run) at once, without
/// moving any playhead, so a window can answer the pointer as it moves. `step` handles the
/// same events the same way.
pub fn input(doc: &Document, state: &PlayerState, events: &[Event], host: &mut dyn ScriptHost) -> Step {
    host.begin_step();
    let mut r = Run::new(doc, state.clone(), host);
    r.refresh();
    r.remember_graphics();
    for e in events.iter().take(MAX_EVENTS_PER_STEP) {
        r.event(*e);
    }
    r.finish()
}

/// The instance tree at `state`: the main timeline (index 0), then every symbol instance in
/// drawing order (parents before their contents).
pub fn tree(doc: &Document, st: &PlayerState) -> Vec<Node> {
    let tl = doc.timeline(st.scene);
    let mut out = vec![Node { path: Vec::new(), parent: None, owner: 0, name: String::new(), symbol: None, kind: NodeKind::Root, frame: st.root.frame, frames: tl.frame_count(), xf: Affine::IDENTITY, hittable: true, events: Vec::new(), gain: 1.0 }];
    // The main timeline is seen through its camera (ADR 0027): instances are where the pointer
    // sees them.
    let camera = tl.camera_at(st.root.frame, &doc.stage);
    walk(doc, st, tl, st.root.frame, 0, 0, Affine::IDENTITY, 1, true, camera, &mut out);
    out
}

/// The button (or movie clip with event handlers) at stage point `p`, topmost first.
pub fn hit(doc: &Document, st: &PlayerState, p: Vec2) -> Option<InstancePath> {
    hit_in(doc, st, &tree(doc, st), p)
}

/// Whether timeline layer `index` takes part in playback: shown, not a folder or guide, not in
/// a hidden folder (what is drawn is what plays).
fn plays(tl: &Timeline, index: usize) -> bool {
    tl.layers.get(index).is_some_and(|l| l.visible && !matches!(l.kind, LayerKind::Folder | LayerKind::Guide)) && !tl.hidden_by_folder(index)
}

#[allow(clippy::too_many_arguments)]
fn walk(doc: &Document, st: &PlayerState, tl: &Timeline, frame: u32, parent: usize, owner: usize, xf: Affine, depth: usize, hittable: bool, camera: Option<crate::Camera>, out: &mut Vec<Node>) {
    if depth > MAX_SYMBOL_DEPTH {
        return;
    }
    let base: InstancePath = out.get(parent).map(|n| n.path.clone()).unwrap_or_default();
    let parent_gain = out.get(parent).map_or(1.0, |n| n.gain);
    for (index, layer) in tl.layers.iter().enumerate().rev() {
        if !plays(tl, index) {
            continue;
        }
        let gain = parent_gain * layer.sound_gain();
        let hit = hittable && layer.kind != LayerKind::Mask;
        let path_walk = layer.parent.and_then(|p| tl.layer(p)).filter(|g| g.kind == LayerKind::Guide).and_then(|g| guide_walk(g, frame));
        let Some((kf_start, els)) = evaluate_guided(layer, frame, path_walk.as_ref()) else { continue };
        let layer_xf = camera.map_or(xf, |c| c.view(&doc.stage, layer.depth));
        for el in els.iter() {
            if out.len() >= MAX_NODES {
                return;
            }
            let ElementKind::Symbol { symbol, loop_mode, first_frame, events, .. } = &el.kind else { continue };
            let Some(sym) = doc.symbol(*symbol) else { continue };
            let mut path = base.clone();
            path.push(el.id);
            let ov = st.props.get(&path);
            let shown = ov.is_none_or(Overrides::shown);
            let set = ov.map(|o| o.apply(el));
            let e: &Element = set.as_ref().unwrap_or(el);
            let len = sym.timeline.frame_count();
            let (kind, local) = match sym.kind {
                SymbolKind::Graphic => (NodeKind::Graphic, loop_mode.local_frame(frame.saturating_sub(kf_start), *first_frame, len)),
                SymbolKind::MovieClip => (NodeKind::MovieClip, st.clip_frame(&path, len)),
                SymbolKind::Button => (NodeKind::Button, st.button_frame(&path, len)),
            };
            let idx = out.len();
            let node_xf = layer_xf.mul(&e.transform);
            out.push(Node { path, parent: Some(parent), owner, name: e.name.clone(), symbol: Some(*symbol), kind, frame: local, frames: len, xf: node_xf, hittable: hit && shown, events: events.clone(), gain });
            // Names inside a movie clip (or a button's states) are its own; a graphic's
            // contents belong to the timeline the graphic is on.
            let inner_owner = if kind == NodeKind::Graphic { owner } else { idx };
            walk(doc, st, &sym.timeline, local, idx, inner_owner, node_xf, depth + 1, hit && shown, None, out);
        }
    }
}

fn hit_in(doc: &Document, st: &PlayerState, nodes: &[Node], p: Vec2) -> Option<InstancePath> {
    for n in nodes.iter().rev() {
        let interactive = n.kind == NodeKind::Button || (n.kind == NodeKind::MovieClip && !n.events.is_empty());
        if !n.hittable || !interactive {
            continue;
        }
        let Some(sym) = n.symbol.and_then(|s| doc.symbol(s)) else { continue };
        let dl = if n.kind == NodeKind::Button {
            // The Hit frame (the fourth) when the button has one; otherwise the Up frame.
            let f = if n.frames > 3 { 3 } else { 0 };
            evaluate_symbol(doc, &sym.timeline, f, n.xf, None)
        } else {
            evaluate_symbol(doc, &sym.timeline, n.frame, n.xf, Some(&Play { state: st, path: n.path.clone() }))
        };
        let inside = dl.items.iter().filter(|i| !i.guide && i.masks.is_none()).any(|i| {
            let fill = i.fill.is_some() && i.path.contains(p, &i.transform, i.fill_rule);
            fill || i.stroke.as_ref().is_some_and(|s| i.path.distance_to_edge(p, &i.transform) <= s.width * 0.5 * i.transform.mean_scale())
        });
        if inside {
            return Some(n.path.clone());
        }
    }
    None
}

/// The label a timeline's frame is under: the first layer's (top down) covering keyframe with
/// a label.
fn label_at(tl: &Timeline, frame: u32) -> &str {
    tl.layers.iter().filter_map(|l| l.keyframe_at(frame)).map(|k| k.label.as_str()).find(|l| !l.is_empty()).unwrap_or("")
}

/// The first frame labelled `label` on a timeline.
fn find_label(tl: &Timeline, label: &str) -> Option<u32> {
    tl.layers.iter().flat_map(|l| l.keyframes.iter()).filter(|k| k.label == label).map(|k| k.start).min()
}

/// One step in progress.
struct Run<'a, 'h> {
    doc: &'a Document,
    st: PlayerState,
    host: &'h mut dyn ScriptHost,
    nodes: Vec<Node>,
    /// The state changed since `nodes` was built.
    dirty: bool,
    /// Timelines that entered a frame whose actions haven't run yet.
    pending: VecDeque<InstancePath>,
    actions: usize,
    enters: usize,
    /// A budget ran out: nothing more runs this step.
    halted: bool,
    output: Vec<String>,
    dropped: usize,
    sounds: Vec<Started>,
    /// The frame each graphic showed when the step began: one that shows another frame at
    /// its end has entered it (and starts its sounds).
    graphics: BTreeMap<InstancePath, u32>,
}

impl<'a, 'h> Run<'a, 'h> {
    fn new(doc: &'a Document, st: PlayerState, host: &'h mut dyn ScriptHost) -> Self {
        Run { doc, st, host, nodes: Vec::new(), dirty: true, pending: VecDeque::new(), actions: 0, enters: 0, halted: false, output: Vec::new(), dropped: 0, sounds: Vec::new(), graphics: BTreeMap::new() }
    }

    fn remember_graphics(&mut self) {
        self.graphics = self.nodes.iter().filter(|n| n.kind == NodeKind::Graphic).map(|n| (n.path.clone(), n.frame)).collect();
    }

    /// The sounds on keyframes of `tl` starting at `frame` (every layer but guides, as the
    /// soundtrack has them), at volume `gain` times their layer's.
    fn sounds_at(&mut self, tl: &Timeline, frame: u32, gain: f32) {
        for layer in tl.layers.iter().filter(|l| !matches!(l.kind, LayerKind::Guide | LayerKind::Folder | LayerKind::Camera)) {
            let Some(k) = layer.keyframe_starting_at(frame).and_then(|i| layer.keyframes.get(i)) else { continue };
            if let Some(fs) = &k.sound
                && self.sounds.len() < MAX_SOUNDS_PER_STEP
            {
                self.sounds.push(Started { sound: fs.clone(), gain: gain * layer.sound_gain(), span: k.duration });
            }
        }
    }

    fn out(&mut self, line: impl Into<String>) {
        if self.output.len() < MAX_OUTPUT_PER_STEP {
            self.output.push(line.into());
        } else {
            self.dropped += 1;
        }
    }

    fn halt(&mut self, why: &str) {
        if !self.halted {
            self.halted = true;
            self.out(format!("Stopped running actions this frame: {why} (a go-to loop?)"));
        }
    }

    /// Rebuilds the instance tree after a change, starting movie clips that appeared (they
    /// enter their first frame) and forgetting those that left.
    fn refresh(&mut self) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        if self.st.scene >= self.doc.scenes.len() {
            self.st.scene = 0;
        }
        let n = self.doc.timeline(self.st.scene).frame_count();
        self.st.root.frame = self.st.root.frame.min(n.saturating_sub(1));
        self.nodes = tree(self.doc, &self.st);
        let live: std::collections::BTreeSet<&InstancePath> = self.nodes.iter().filter(|n| n.kind == NodeKind::MovieClip).map(|n| &n.path).collect();
        self.st.clips.retain(|p, _| live.contains(p));
        for p in live {
            if !self.st.clips.contains_key(p) {
                self.st.clips.insert(p.clone(), ClipState::default());
                if !self.pending.contains(p) {
                    self.pending.push_back(p.clone());
                }
            }
        }
        let all: std::collections::BTreeSet<&InstancePath> = self.nodes.iter().map(|n| &n.path).collect();
        self.st.props.retain(|p, _| all.contains(p));
        for slot in [&mut self.st.hover, &mut self.st.pressed] {
            if slot.as_ref().is_some_and(|p| !all.contains(p)) {
                *slot = None;
            }
        }
    }

    /// Queues a timeline whose new frame's actions must run (once, however often it moved).
    fn queue(&mut self, path: InstancePath) {
        if !self.pending.contains(&path) {
            self.pending.push_back(path);
        }
    }

    fn index_of(&self, path: &[ElementId]) -> Option<usize> {
        self.nodes.iter().position(|n| n.path == path)
    }

    /// Every playing timeline one frame on: the main timeline into the next scene at the end
    /// of one (and back to the first at the end of the movie), movie clips round their loop.
    fn advance(&mut self) {
        if self.st.root.playing {
            let n = self.doc.timeline(self.st.scene).frame_count();
            let (scene, frame) = if self.st.root.frame + 1 < n {
                (self.st.scene, self.st.root.frame + 1)
            } else if self.st.scene + 1 < self.doc.scenes.len() {
                (self.st.scene + 1, 0)
            } else {
                (0, 0)
            };
            if (scene, frame) != (self.st.scene, self.st.root.frame) {
                self.st.scene = scene;
                self.st.root.frame = frame;
                self.queue(Vec::new());
            }
        }
        let lens: BTreeMap<&InstancePath, u32> = self.nodes.iter().filter(|n| n.kind == NodeKind::MovieClip).map(|n| (&n.path, n.frames)).collect();
        let mut moved = Vec::new();
        for (path, c) in self.st.clips.iter_mut().filter(|(_, c)| c.playing) {
            let len = lens.get(path).copied().unwrap_or(1).max(1);
            let next = (c.frame + 1) % len;
            if next != c.frame {
                c.frame = next;
                moved.push(path.clone());
            }
        }
        for p in moved {
            self.queue(p);
        }
        self.dirty = true;
    }

    /// Runs the actions of every frame entered, until none is left (or a budget runs out).
    fn settle(&mut self) {
        loop {
            self.refresh();
            if self.halted {
                self.pending.clear();
                return;
            }
            let Some(path) = self.pending.pop_front() else { return };
            self.enters += 1;
            if self.enters > MAX_ENTERS_PER_STEP {
                self.halt(&format!("more than {MAX_ENTERS_PER_STEP} frames entered"));
                continue;
            }
            self.enter(&path);
        }
    }

    /// The timeline of node `i` (the main timeline or a movie clip's).
    fn timeline_of(&self, i: usize) -> Option<&'a Timeline> {
        let n = self.nodes.get(i)?;
        match n.kind {
            NodeKind::Root => Some(self.doc.timeline(self.st.scene)),
            NodeKind::MovieClip => n.symbol.and_then(|s| self.doc.symbol(s)).map(|s| &s.timeline),
            NodeKind::Graphic | NodeKind::Button => None,
        }
    }

    /// Runs what the keyframes starting at timeline `path`'s current frame hold, top layer
    /// first.
    fn enter(&mut self, path: &[ElementId]) {
        let Some(i) = self.index_of(path) else { return };
        let Some(tl) = self.timeline_of(i) else { return };
        let frame = self.nodes.get(i).map_or(0, |n| n.frame);
        let gain = self.nodes.get(i).map_or(1.0, |n| n.gain);
        self.sounds_at(tl, frame, gain);
        let scripts: Vec<&'a FrameScript> = tl.layers.iter().enumerate().filter(|(ix, _)| plays(tl, *ix)).filter_map(|(_, l)| l.keyframe_starting_at(frame).and_then(|k| l.keyframes.get(k))).filter_map(|k| k.script.as_ref()).collect();
        for script in scripts {
            match script {
                FrameScript::Actions(list) => self.run_list(path, list),
                FrameScript::Plugin(p) => {
                    let Some(i) = self.index_of(path) else { return };
                    let label = label_at(tl, frame);
                    let cx = ScriptContext { doc: self.doc, state: &self.st, nodes: &self.nodes, timeline: i, frame, label };
                    let run = self.host.run(p, &cx);
                    for line in run.output {
                        self.out(line);
                    }
                    let list: Vec<Action> = run.actions.into_iter().take(crate::actions::MAX_ACTIONS).collect();
                    self.run_list(path, &list);
                }
                // Kept with the document, not run (ADR 0013 tier 1). The Test Movie window says
                // so once, rather than a line every time the playhead passes.
                FrameScript::Source(_) => {}
            }
        }
    }

    fn run_list(&mut self, owner: &[ElementId], list: &[Action]) {
        for a in list {
            if self.halted {
                return;
            }
            self.actions += 1;
            if self.actions > MAX_ACTIONS_PER_STEP {
                self.halt(&format!("more than {MAX_ACTIONS_PER_STEP} actions"));
                return;
            }
            self.exec(owner, a);
        }
    }

    /// The node `target` names, seen from timeline node `from`.
    fn resolve(&self, from: usize, target: &Target) -> Result<usize, String> {
        let mut cur = from;
        for seg in target.segments()? {
            cur = match seg {
                Seg::Root => 0,
                Seg::Parent => match self.nodes.get(cur) {
                    Some(n) if n.kind != NodeKind::Root => n.owner,
                    _ => return Err("the main timeline has no parent".into()),
                },
                Seg::Child(name) => self.nodes.iter().enumerate().find(|(i, n)| *i != cur && n.owner == cur && n.name == name).map(|(i, _)| i).ok_or_else(|| format!("no instance named “{name}” here"))?,
            };
        }
        Ok(cur)
    }

    fn exec(&mut self, owner: &[ElementId], a: &Action) {
        self.refresh();
        let Some(from) = self.index_of(owner) else { return };
        match a {
            Action::Trace { message } => self.out(message.clone()),
            Action::NextScene | Action::PrevScene => {
                let next = if matches!(a, Action::NextScene) { self.st.scene.checked_add(1) } else { self.st.scene.checked_sub(1) };
                if let Some(s) = next.filter(|s| *s < self.doc.scenes.len()) {
                    self.goto_root(s, 0);
                }
            }
            Action::SetProperty { target, property, value } => match self.resolve(from, target) {
                Ok(i) if i != 0 => {
                    if let Some(path) = self.nodes.get(i).map(|n| n.path.clone()) {
                        self.st.props.entry(path).or_default().set(*property, *value);
                        self.dirty = true;
                    }
                }
                Ok(_) => self.out(format!("{}: the main timeline has no {} to set", a.describe(), property.label().to_lowercase())),
                Err(e) => self.out(format!("{}: {e}", a.describe())),
            },
            _ => {
                let Some(target) = a.target() else { return };
                let i = match self.resolve(from, target) {
                    Ok(i) => i,
                    Err(e) => return self.out(format!("{}: {e}", a.describe())),
                };
                let Some(node) = self.nodes.get(i) else { return };
                if !matches!(node.kind, NodeKind::Root | NodeKind::MovieClip) {
                    let what = if node.kind == NodeKind::Button { "a button" } else { "a graphic" };
                    return self.out(format!("{}: “{}” is {what}; only movie clips and the main timeline have a playhead", a.describe(), node.name));
                }
                let (path, frame, frames) = (node.path.clone(), node.frame, node.frames.max(1));
                match a {
                    Action::Stop { .. } => self.set_playing(&path, false),
                    Action::Play { .. } => self.set_playing(&path, true),
                    Action::NextFrame { .. } | Action::PrevFrame { .. } => {
                        let f = if matches!(a, Action::NextFrame { .. }) { (frame + 1).min(frames - 1) } else { frame.saturating_sub(1) };
                        self.set_playing(&path, false);
                        self.goto(&path, None, f);
                    }
                    Action::GotoAndPlay { frame: to, scene, .. } | Action::GotoAndStop { frame: to, scene, .. } => {
                        let play = matches!(a, Action::GotoAndPlay { .. });
                        match self.goto_target(&path, to, scene.as_deref()) {
                            Ok((s, f)) => {
                                self.set_playing(&path, play);
                                self.goto(&path, s, f);
                            }
                            Err(e) => self.out(format!("{}: {e}", a.describe())),
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    /// Where a go-to lands: the scene (main timeline only) and frame.
    fn goto_target(&self, path: &[ElementId], to: &FrameRef, scene: Option<&str>) -> Result<(Option<usize>, u32), String> {
        let tl_of = |s: usize| self.doc.timeline(s);
        let (scene_ix, tl) = if path.is_empty() {
            let s = match scene {
                Some(name) => self.doc.scenes.iter().position(|x| x.name == name).or_else(|| self.doc.scenes.iter().position(|x| x.name.eq_ignore_ascii_case(name))).ok_or_else(|| format!("no scene named “{name}”"))?,
                None => self.st.scene,
            };
            (Some(s), tl_of(s))
        } else {
            if scene.is_some() {
                return Err("only the main timeline has scenes".into());
            }
            let i = self.index_of(path).ok_or("the timeline is gone")?;
            (None, self.timeline_of(i).ok_or("not a timeline")?)
        };
        match to {
            FrameRef::Number(n) => Ok((scene_ix, (*n).min(tl.frame_count().saturating_sub(1)))),
            FrameRef::Label(l) => {
                if let Some(f) = find_label(tl, l) {
                    return Ok((scene_ix, f));
                }
                // The main timeline: a label in another scene, when no scene was named.
                if path.is_empty() && scene.is_none()
                    && let Some((s, f)) = self.doc.scenes.iter().enumerate().find_map(|(s, x)| find_label(&x.timeline, l).map(|f| (s, f)))
                {
                    return Ok((Some(s), f));
                }
                Err(format!("no frame labelled “{l}”"))
            }
        }
    }

    fn set_playing(&mut self, path: &[ElementId], on: bool) {
        if path.is_empty() {
            self.st.root.playing = on;
        } else if let Some(c) = self.st.clips.get_mut(path) {
            c.playing = on;
        }
    }

    fn goto_root(&mut self, scene: usize, frame: u32) {
        if (scene, frame) != (self.st.scene, self.st.root.frame) {
            self.st.scene = scene;
            self.st.root.frame = frame;
            self.queue(Vec::new());
            self.dirty = true;
        }
    }

    /// Moves a timeline's playhead; entering a new frame runs its actions.
    fn goto(&mut self, path: &[ElementId], scene: Option<usize>, frame: u32) {
        if path.is_empty() {
            self.goto_root(scene.unwrap_or(self.st.scene), frame);
        } else if let Some(c) = self.st.clips.get_mut(path)
            && c.frame != frame
        {
            c.frame = frame;
            self.queue(path.to_vec());
            self.dirty = true;
        }
    }

    fn event(&mut self, e: Event) {
        self.refresh();
        match e {
            Event::Move(p) => self.pointer(Some(p)),
            Event::Down(p) => {
                self.pointer(Some(p));
                self.st.down = true;
                self.st.pressed = self.st.hover.clone();
                self.dirty = true;
                if let Some(h) = self.st.hover.clone() {
                    self.fire(&h, ButtonEvent::Press);
                }
            }
            Event::Up(p) => {
                self.pointer(Some(p));
                self.st.down = false;
                let pressed = self.st.pressed.take();
                self.dirty = true;
                if let Some(h) = self.st.hover.clone() {
                    self.fire(&h, ButtonEvent::Release);
                    if pressed.as_ref() == Some(&h) {
                        self.fire(&h, ButtonEvent::Click);
                    }
                }
            }
            Event::Leave => self.pointer(None),
        }
    }

    /// Moves the pointer: what it leaves gets Out, what it reaches gets Over.
    fn pointer(&mut self, p: Option<Vec2>) {
        self.refresh();
        self.st.pointer = p;
        let now = p.and_then(|p| hit_in(self.doc, &self.st, &self.nodes, p));
        if now != self.st.hover {
            let old = std::mem::replace(&mut self.st.hover, now.clone());
            self.dirty = true;
            if let Some(o) = old {
                self.fire(&o, ButtonEvent::Out);
            }
            if let Some(n) = now {
                self.fire(&n, ButtonEvent::Over);
            }
        }
    }

    /// Runs an instance's handler for `event` (in the timeline the instance is on).
    fn fire(&mut self, path: &[ElementId], event: ButtonEvent) {
        self.refresh();
        let Some(n) = self.index_of(path).and_then(|i| self.nodes.get(i)) else { return };
        let Some(h) = n.events.iter().find(|h| h.event == event) else { return };
        let (actions, owner) = (h.actions.clone(), n.owner);
        let Some(owner_path) = self.nodes.get(owner).map(|o| o.path.clone()) else { return };
        self.run_list(&owner_path, &actions);
        self.settle();
    }

    fn finish(mut self) -> Step {
        self.refresh();
        // Graphics keep step with their timelines: one now on another frame (or new on the
        // stage) has entered it.
        let entered: Vec<(SymbolId, u32, f32)> = self
            .nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Graphic && self.graphics.get(&n.path) != Some(&n.frame))
            .filter_map(|n| Some((n.symbol?, n.frame, n.gain)))
            .collect();
        for (symbol, frame, gain) in entered {
            if let Some(sym) = self.doc.symbol(symbol) {
                self.sounds_at(&sym.timeline, frame, gain);
            }
        }
        if self.dropped > 0 {
            let n = self.dropped;
            self.output.push(format!("… {n} more lines"));
        }
        Step { state: self.st, output: self.output, sounds: self.sounds }
    }
}

#[cfg(test)]
#[path = "player_tests.rs"]
mod tests;
