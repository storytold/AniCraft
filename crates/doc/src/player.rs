//! The interactive player (ADR 0013, ADR 0024): playback with frame actions, frame scripts and
//! button events, as a pure function of the document.
//!
//! [`PlayerState`] holds what changes while a movie plays: the main timeline's scene, frame and
//! playing flag, each movie clip instance's playhead, the properties actions have set, and the
//! pointer. [`step`] advances it by one frame: every playing timeline moves on, the actions on
//! the keyframes they enter run (and kept ActionScript in the subset that compiles, ADR 0013
//! tier 2, with the mouse listeners it adds), then that frame's pointer events go to the buttons
//! under the pointer and run their handlers. [`crate::evaluate_player`] draws a state.
//!
//! Nothing here changes the document, so the authoring stage and every export stay
//! script-free; and nothing reads a clock or a random source, so the same document, state and
//! events always give the same next state. Budgets bound what one step may do (actions run,
//! frames entered, instances walked, output lines), so a `goto` loop ends in a message instead
//! of a hang.

use crate::actions::as3::ir::{Base, Function, ListenEvent, Place, Prop, Value};
use crate::actions::as3::run::{self, Stop, World};
use crate::actions::as3::{Compiled, Decls, Problem};
use crate::actions::{Action, ButtonEvent, EventHandler, FrameRef, FrameScript, PluginScript, Property, Seg, Target};
use crate::eval::{Play, evaluate_shown, evaluate_symbol, guide_walk};
use crate::field::{Changed, Edit, Editor, FieldLook, FieldState, chars, fit, scroll_to_caret};
use crate::sound::Started;
use crate::{BlendMode, ColorTransform, DisplayList, Document, Element, ElementId, ElementKind, FIELD_GUTTER, Group, LayerKind, MAX_SYMBOL_DEPTH, Rgba, SoundSync, SymbolId, SymbolKind, TextBlock, TextField, TextKind, Timeline};
use anicraft_geom::{Affine, Vec2};
use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;

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
/// Most listeners scripts add to one instance.
pub const MAX_LISTENERS_PER_INSTANCE: usize = 16;
/// Most listeners scripts add to the stage.
pub const MAX_STAGE_LISTENERS: usize = 64;
/// Script fuel one step may use, all scripts and listeners together (each run has at most
/// [`run::FUEL_PER_RUN`]).
pub const MAX_FUEL_PER_STEP: u64 = 1_000_000;
/// The random generator's seed when none is given.
pub const DEFAULT_SEED: u64 = 0;

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
            e.color = e.color.with_alpha(a);
        }
        e
    }
}

/// A listener a script added to an instance or the stage (ADR 0013 tiers 2 and 2.5): it lives
/// while the instance (and the script's timeline) stay on the stage.
#[derive(Clone, Debug, PartialEq)]
pub struct Listening {
    pub event: ListenEvent,
    /// The timeline whose script added it: the function runs there.
    pub owner: InstancePath,
    /// The function's name: the same function for the same event is added once.
    pub function: String,
    pub func: Arc<Function>,
    /// When it was added (enter-frame listeners run in this order).
    pub seq: u64,
}

/// The movie's random numbers (`Math.random()`): Marsaglia's xorshift128, seeded, so a movie
/// with the same seed and input always plays the same.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rng(pub [u32; 4]);

impl Rng {
    pub fn seeded(seed: u64) -> Rng {
        let lo = (seed & 0xffff_ffff) as u32;
        let hi = (seed >> 32) as u32;
        let mut s = [lo ^ 0x9e37_79b9, hi ^ 0x243f_6a88, 0xb7e1_5162, 0x3c6e_f372];
        if s == [0; 4] {
            s[0] = 1;
        }
        Rng(s)
    }
    fn next_u32(&mut self) -> u32 {
        let [x, y, z, w] = self.0;
        let t = x ^ (x << 11);
        let w2 = w ^ (w >> 19) ^ (t ^ (t >> 8));
        self.0 = [y, z, w, w2];
        w2
    }
    /// The next number in [0, 1), from 53 random bits.
    pub fn next_f64(&mut self) -> f64 {
        let a = f64::from(self.next_u32() >> 5);
        let b = f64::from(self.next_u32() >> 6);
        (a * 67_108_864.0 + b) / 9_007_199_254_740_992.0
    }
}

impl Default for Rng {
    fn default() -> Self {
        Rng::seeded(DEFAULT_SEED)
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
    /// Listeners scripts added, per instance, in the order added.
    pub listeners: BTreeMap<InstancePath, Vec<Listening>>,
    /// Listeners on the stage (keyboard and enter-frame), in the order added.
    pub stage: Vec<Listening>,
    /// Each timeline instance's variables (ADR 0052), by name.
    pub vars: BTreeMap<InstancePath, BTreeMap<String, Value>>,
    /// The random generator.
    pub rng: Rng,
    /// The last pointer position seen (`mouseX`/`mouseY` keep it when the pointer leaves).
    pub mouse: Vec2,
    /// Listeners added so far (each one's `seq`).
    pub seq: u64,
    /// The pointer in stage space (`None` when it is outside the movie), and whether it is down.
    pub pointer: Option<Vec2>,
    pub down: bool,
    /// The button (or movie clip with event handlers) under the pointer, and the one the
    /// pointer went down on.
    pub hover: Option<InstancePath>,
    pub pressed: Option<InstancePath>,
    /// Steps taken since the start.
    pub tick: u64,
    /// Each text field a script or the viewer has touched (ADR 0066), by instance path.
    pub fields: BTreeMap<InstancePath, FieldState>,
    /// The field that has the keyboard (an input field, or a selectable dynamic one).
    pub focus: Option<InstancePath>,
    /// Shift is held (key code 16 down): caret moves extend the selection.
    pub shift: bool,
    /// The pointer went down in the focused field and is selecting as it drags.
    pub selecting: bool,
}

impl PlayerState {
    /// A state before the first frame, with the random generator seeded.
    pub fn seeded(seed: u64) -> PlayerState {
        PlayerState { rng: Rng::seeded(seed), ..PlayerState::default() }
    }

    /// The frame a movie clip instance shows (`len` frames long).
    pub fn clip_frame(&self, path: &[ElementId], len: u32) -> u32 {
        self.clips.get(path).map_or(0, |c| c.frame).min(len.max(1) - 1)
    }

    /// The frame a button shows: Up (0), Over (1) under the pointer, Down (2) while pressed on
    /// it; a button whose timeline is shorter shows its last frame.
    pub fn button_frame(&self, path: &[ElementId], len: u32) -> u32 {
        let f = match self.button_state(path) {
            ButtonState::Down => 2,
            ButtonState::Over => 1,
            ButtonState::Up | ButtonState::OutDown => 0,
        };
        f.min(len.max(1) - 1)
    }

    /// Where the pointer leaves a button: [`ButtonState`].
    pub fn button_state(&self, path: &[ElementId]) -> ButtonState {
        ButtonState::of(self.hover.as_deref(), self.pressed.as_deref(), self.down, path)
    }

    /// A timeline's playhead: the main timeline's for the empty path.
    pub fn playhead(&self, path: &[ElementId]) -> Option<ClipState> {
        if path.is_empty() { Some(self.root) } else { self.clips.get(path).copied() }
    }
}

/// Where the pointer is for one button, as the SWF file format names a button's states: away
/// (Up), over it (Over), pressed on it (Down), or pressed on it and dragged off (shown Up).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonState {
    Up,
    Over,
    Down,
    OutDown,
}

impl ButtonState {
    /// The state of the button at `path` with the pointer over `hover`, pressed on `pressed`,
    /// and down or not.
    pub fn of(hover: Option<&[ElementId]>, pressed: Option<&[ElementId]>, down: bool, path: &[ElementId]) -> ButtonState {
        match (hover == Some(path), down && pressed == Some(path)) {
            (true, true) => ButtonState::Down,
            (true, false) => ButtonState::Over,
            (false, true) => ButtonState::OutDown,
            (false, false) => ButtonState::Up,
        }
    }
}

/// The frame of a button whose sound plays when the button goes from state `from` to `to`
/// (ADR 0057), as the SWF format's button sounds are kept: the Over frame's going over it, the
/// Up frame's leaving it, the Down frame's pressing it, and the Hit frame's letting go over it.
/// Dragging off a pressed button and back plays nothing.
pub fn button_sound_frame(from: ButtonState, to: ButtonState) -> Option<u32> {
    use ButtonState::*;
    match (from, to) {
        (Up, Over) => Some(1),
        (Over, Up) => Some(0),
        (Over, Down) => Some(2),
        (Down, Over) => Some(3),
        _ => None,
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
    /// A key went down (or repeats) and came up, by its ActionScript key code.
    KeyDown(u32),
    KeyUp(u32),
    /// A character typed (or pasted) into the focused input field (ADR 0066).
    Char(char),
    /// Select All in the focused field.
    SelectAll,
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
    /// A dynamic or input text field (ADR 0066).
    Text,
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
    /// The instance's own transform and alpha in its parent, as the timeline gives them (before
    /// properties scripts set).
    pub local: Affine,
    pub alpha: f32,
    /// Pointer events can reach it (visible, not mask artwork).
    pub hittable: bool,
    pub events: Vec<EventHandler>,
    /// The volume its sounds get: the layers holding it, from the main timeline down.
    pub gain: f32,
    /// A text field's block (its style, box and authored text).
    pub text: Option<Arc<TextBlock>>,
}

impl Node {
    /// A text field's block and field settings.
    pub fn field(&self) -> Option<(&TextBlock, &TextField)> {
        let t = self.text.as_deref()?;
        Some((t, t.field.as_ref()?))
    }
}

/// Runs plug-in frame scripts for the player (the engine implements it over the plug-in host).
pub trait ScriptHost {
    /// A new step starts (per-step budgets reset here).
    fn begin_step(&mut self) {}
    /// Runs `script` for the timeline described by `cx`; returns the actions it emitted.
    fn run(&mut self, script: &PluginScript, cx: &ScriptContext<'_>) -> ScriptRun;
    /// Lays out `shown` (a field's text as it is drawn) in `block`'s style and box (ADR 0066):
    /// the fonts live above this crate. `None`: it can't, and the field keeps its authored
    /// outline until its text changes (then it draws only its box).
    fn lay_out_field(&mut self, block: &TextBlock, shown: &str) -> Option<FieldLook> {
        let _ = (block, shown);
        None
    }
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
    start_seeded(doc, DEFAULT_SEED, host)
}

/// [`start`], with `Math.random()` seeded by `seed`.
pub fn start_seeded(doc: &Document, seed: u64, host: &mut dyn ScriptHost) -> Step {
    host.begin_step();
    let mut r = Run::new(doc, PlayerState::seeded(seed), host);
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
    // Enter-frame listeners hear the new frame before its frame scripts run (as ActionScript
    // dispatches `enterFrame` before frame scripts).
    r.enter_frame();
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
    let mut out = vec![Node { path: Vec::new(), parent: None, owner: 0, name: String::new(), symbol: None, kind: NodeKind::Root, frame: st.root.frame, frames: tl.frame_count(), xf: Affine::IDENTITY, local: Affine::IDENTITY, alpha: 1.0, hittable: true, events: Vec::new(), gain: 1.0, text: None }];
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

/// The text field at stage point `p` the viewer can click into, topmost first (ADR 0066): what
/// a window shows a text cursor over.
pub fn field_at(doc: &Document, st: &PlayerState, p: Vec2) -> Option<InstancePath> {
    field_hit(&tree(doc, st), p).map(|(_, path)| path)
}

/// The topmost focusable field node at `p`, with its index.
fn field_hit(nodes: &[Node], p: Vec2) -> Option<(usize, InstancePath)> {
    for (i, n) in nodes.iter().enumerate().rev() {
        if n.kind != NodeKind::Text || !n.hittable {
            continue;
        }
        let Some((t, f)) = n.field() else { continue };
        if !f.focusable() {
            continue;
        }
        let Some(q) = n.xf.inverse().map(|m| m.apply(p)) else { continue };
        if q.x >= 0.0 && q.y >= 0.0 && q.x <= t.box_width && q.y <= t.box_height {
            return Some((i, n.path.clone()));
        }
    }
    None
}

/// The frame of a button's timeline, `len` frames long, that answers the pointer: the Hit
/// frame (the fourth) when it has one, otherwise the Up frame.
pub fn hit_frame(len: u32) -> u32 {
    if len > 3 { 3 } else { 0 }
}

/// The area of a button with timeline `tl` that answers the pointer, placed by `xf`: its
/// [`hit_frame`]'s artwork without guides and mask artwork (masks don't cut hit areas, and
/// shapes count however faint they are drawn). The hit test and Test Movie's hit-area overlay
/// both use it.
pub fn hit_area(doc: &Document, tl: &Timeline, xf: Affine) -> DisplayList {
    let mut dl = evaluate_symbol(doc, tl, hit_frame(tl.frame_count()), xf, None);
    dl.items.retain(|i| !i.guide && i.masks.is_none());
    for i in &mut dl.items {
        i.clip = None;
    }
    for g in &mut dl.groups {
        g.clip = None;
    }
    dl
}

/// `dl` drawn as one translucent area: every item filled and stroked in `tint` at full
/// strength, composited together and laid over what is below at `alpha`, so overlapping shapes
/// don't darken each other.
pub fn tinted(dl: DisplayList, tint: Rgba, alpha: f32) -> DisplayList {
    let mut out = DisplayList::default();
    let alpha = if alpha.is_finite() { alpha.clamp(0.0, 1.0) } else { 1.0 };
    out.groups.push(Group { parent: None, alpha, blend: BlendMode::default(), filters: Arc::from(Vec::new()), clip: None });
    out.append(dl);
    for g in out.groups.iter_mut().skip(1) {
        g.alpha = 1.0;
        g.blend = BlendMode::default();
        g.filters = Arc::from(Vec::new());
        g.parent = Some(g.parent.unwrap_or(0));
    }
    for it in &mut out.items {
        it.color = ColorTransform::tint(tint, 1.0);
        it.outline = None;
        it.group = Some(it.group.unwrap_or(0));
    }
    out
}

/// Every button's hit area at `st`, each a translucent area in `tint` (Test Movie's Show Hit
/// Areas): where the pointer finds the buttons, drawn over the movie.
pub fn hit_overlay(doc: &Document, st: &PlayerState, tint: Rgba, alpha: f32) -> DisplayList {
    let mut out = DisplayList::default();
    for n in tree(doc, st).iter().filter(|n| n.kind == NodeKind::Button && n.hittable) {
        if let Some(sym) = n.symbol.and_then(|s| doc.symbol(s)) {
            out.append(tinted(hit_area(doc, &sym.timeline, n.xf), tint, alpha));
        }
    }
    out
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
    // Drawn deepest first (ADR 0082), so the topmost hit is the one drawn last.
    // (Only sorted when some layer has a depth: nested timelines allocate nothing.)
    let sorted = (depth == 1 && tl.layers.iter().any(|l| l.has_depth())).then(|| tl.draw_order(frame));
    let n = tl.layers.len();
    for k in 0..n {
        let index = sorted.as_ref().map_or(n - 1 - k, |o| o.get(k).copied().unwrap_or(n));
        let Some(layer) = tl.layers.get(index) else { continue };
        if !plays(tl, index) {
            continue;
        }
        let gain = parent_gain * layer.sound_gain();
        let hit = hittable && layer.kind != LayerKind::Mask;
        let path_walk = layer.parent.and_then(|p| tl.layer(p)).filter(|g| g.kind == LayerKind::Guide).and_then(|g| guide_walk(g, frame));
        let Some((kf_start, els)) = evaluate_shown(layer, frame, path_walk.as_ref(), doc.stage.fps) else { continue };
        let layer_xf = match camera {
            Some(c) => match c.layer_view(&doc.stage, layer, frame) {
                Some(v) => v,
                None => continue,
            },
            None => xf,
        };
        // Instances inside a group belong to the group's timeline (ADR 0073).
        let els = crate::expand_groups(&els).into_owned();
        for el in els.iter() {
            if out.len() >= MAX_NODES {
                return;
            }
            // Dynamic and input text fields are instances scripts can name (static text isn't).
            if let ElementKind::Text(t) = &el.kind
                && t.field.is_some()
            {
                let mut path = base.clone();
                path.push(el.id);
                let ov = st.props.get(&path);
                let shown = ov.is_none_or(Overrides::shown);
                let set = ov.map(|o| o.apply(el));
                let e: &Element = set.as_ref().unwrap_or(el);
                out.push(Node { path, parent: Some(parent), owner, name: e.name.clone(), symbol: None, kind: NodeKind::Text, frame: 0, frames: 0, xf: layer_xf.mul(&e.transform), local: el.transform, alpha: el.color.alpha(), hittable: hit && shown, events: Vec::new(), gain, text: Some(t.clone()) });
                continue;
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
            out.push(Node { path, parent: Some(parent), owner, name: e.name.clone(), symbol: Some(*symbol), kind, frame: local, frames: len, xf: node_xf, local: el.transform, alpha: el.color.alpha(), hittable: hit && shown, events: events.clone(), gain, text: None });
            // Names inside a movie clip (or a button's states) are its own; a graphic's
            // contents belong to the timeline the graphic is on.
            let inner_owner = if kind == NodeKind::Graphic { owner } else { idx };
            walk(doc, st, &sym.timeline, local, idx, inner_owner, node_xf, depth + 1, hit && shown, None, out);
        }
    }
}

fn hit_in(doc: &Document, st: &PlayerState, nodes: &[Node], p: Vec2) -> Option<InstancePath> {
    for n in nodes.iter().rev() {
        let interactive = n.kind == NodeKind::Button || (n.kind == NodeKind::MovieClip && (!n.events.is_empty() || st.listeners.contains_key(&n.path)));
        if !n.hittable || !interactive {
            continue;
        }
        let Some(sym) = n.symbol.and_then(|s| doc.symbol(s)) else { continue };
        let dl = if n.kind == NodeKind::Button {
            hit_area(doc, &sym.timeline, n.xf)
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
    /// Each timeline's declarations (ADR 0052), made the first time a script there
    /// needs them.
    decls: BTreeMap<TimelineRef, Arc<Decls>>,
    /// Script fuel left this step.
    fuel: u64,
}

impl<'a, 'h> Run<'a, 'h> {
    fn new(doc: &'a Document, st: PlayerState, host: &'h mut dyn ScriptHost) -> Self {
        Run { doc, st, host, nodes: Vec::new(), dirty: true, pending: VecDeque::new(), actions: 0, enters: 0, halted: false, output: Vec::new(), dropped: 0, sounds: Vec::new(), graphics: BTreeMap::new(), decls: BTreeMap::new(), fuel: MAX_FUEL_PER_STEP }
    }

    fn remember_graphics(&mut self) {
        self.graphics = self.nodes.iter().filter(|n| n.kind == NodeKind::Graphic).map(|n| (n.path.clone(), n.frame)).collect();
    }

    /// The sounds on keyframes of `tl` starting at `frame` (every layer but guides, as the
    /// soundtrack has them), at volume `gain` times their layer's.
    fn sounds_at(&mut self, tl: &Timeline, frame: u32, gain: f32) {
        self.sounds_on(tl, frame, gain, false);
    }

    /// [`Run::sounds_at`]; on a button's frame (`button`) a stream sound plays as an event,
    /// since a button's frame has no span to stream over (ADR 0057).
    fn sounds_on(&mut self, tl: &Timeline, frame: u32, gain: f32, button: bool) {
        for layer in tl.layers.iter().filter(|l| !matches!(l.kind, LayerKind::Guide | LayerKind::Folder | LayerKind::Camera)) {
            let Some(k) = layer.keyframe_starting_at(frame).and_then(|i| layer.keyframes.get(i)) else { continue };
            if let Some(fs) = &k.sound
                && self.sounds.len() < MAX_SOUNDS_PER_STEP
            {
                let mut sound = fs.clone();
                if button && sound.sync == SoundSync::Stream {
                    sound.sync = SoundSync::Event;
                }
                self.sounds.push(Started { sound, gain: gain * layer.sound_gain(), span: k.duration });
            }
        }
    }

    /// Where the pointer is, for [`Run::button_sounds`]: what it is over, what it went down
    /// on, and whether it is down.
    fn pointer_now(&self) -> (Option<InstancePath>, Option<InstancePath>, bool) {
        (self.st.hover.clone(), self.st.pressed.clone(), self.st.down)
    }

    /// Starts the sounds of the buttons whose state changed since `before` ([`pointer_now`]
    /// then): the frame [`button_sound_frame`] gives for each change.
    ///
    /// [`pointer_now`]: Run::pointer_now
    fn button_sounds(&mut self, before: (Option<InstancePath>, Option<InstancePath>, bool)) {
        if before == self.pointer_now() {
            return;
        }
        self.refresh();
        let (h, p, d) = &before;
        let mut paths: Vec<InstancePath> = Vec::new();
        for path in [h, p, &self.st.hover, &self.st.pressed].into_iter().flatten() {
            if !paths.contains(path) {
                paths.push(path.clone());
            }
        }
        for path in paths {
            let from = ButtonState::of(h.as_deref(), p.as_deref(), *d, &path);
            let Some(frame) = button_sound_frame(from, self.st.button_state(&path)) else { continue };
            let Some(n) = self.index_of(&path).and_then(|i| self.nodes.get(i)).filter(|n| n.kind == NodeKind::Button) else { continue };
            let (gain, symbol) = (n.gain, n.symbol);
            if let Some(sym) = symbol.and_then(|s| self.doc.symbol(s)) {
                self.sounds_on(&sym.timeline, frame, gain, true);
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
        // A listener goes with its instance, and with the timeline whose script added it.
        self.st.listeners.retain(|p, list| {
            list.retain(|l| all.contains(&l.owner));
            all.contains(p) && !list.is_empty()
        });
        self.st.stage.retain(|l| all.contains(&l.owner));
        // A timeline's variables go with its instance, and a field's text with the field.
        self.st.vars.retain(|p, _| all.contains(p));
        self.st.fields.retain(|p, _| all.contains(p));
        if self.st.focus.as_ref().is_some_and(|p| !all.contains(p)) {
            self.st.focus = None;
            self.st.selecting = false;
        }
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
            NodeKind::Graphic | NodeKind::Button | NodeKind::Text => None,
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
                FrameScript::Actions(list) => self.run_list(path, list, false),
                FrameScript::Plugin(p) => {
                    let Some(i) = self.index_of(path) else { return };
                    let label = label_at(tl, frame);
                    let cx = ScriptContext { doc: self.doc, state: &self.st, nodes: &self.nodes, timeline: i, frame, label };
                    let run = self.host.run(p, &cx);
                    for line in run.output {
                        self.out(line);
                    }
                    let list: Vec<Action> = run.actions.into_iter().take(crate::actions::MAX_ACTIONS).collect();
                    self.run_list(path, &list, false);
                }
                // ActionScript in the subset runs, whole (ADR 0013 tier 2); a script outside it
                // doesn't run at all. Test Movie lists which is which once, at the start, rather
                // than a line every time the playhead passes.
                FrameScript::Source(src) => {
                    if let Ok(c) = src.compiled() {
                        self.run_script(path, frame, c);
                    }
                }
            }
        }
    }

    /// Counts one action against the step's budget; `false` when nothing more may run.
    fn budget(&mut self) -> bool {
        if self.halted {
            return false;
        }
        self.actions += 1;
        if self.actions > MAX_ACTIONS_PER_STEP {
            self.halt(&format!("more than {MAX_ACTIONS_PER_STEP} actions"));
            return false;
        }
        true
    }

    /// Runs a list of actions from timeline `owner`; `script`: they come from ActionScript.
    fn run_list(&mut self, owner: &[ElementId], list: &[Action], script: bool) {
        for a in list {
            if !self.budget() {
                return;
            }
            self.exec(owner, a, script);
        }
    }

    /// The declarations of the timeline node `i` is (the main timeline's span every scene).
    fn decls_of(&mut self, i: usize) -> Arc<Decls> {
        let key = match self.nodes.get(i) {
            Some(Node { kind: NodeKind::MovieClip, symbol: Some(s), .. }) => TimelineRef::Symbol(*s),
            _ => TimelineRef::Root,
        };
        let doc = self.doc;
        self.decls.entry(key).or_insert_with(|| Arc::new(decls(doc, key))).clone()
    }

    /// Runs a compiled frame script on timeline `owner` (ADR 0052): when every name
    /// it takes from other frames is there, its statements run with the fuel the step has left.
    fn run_script(&mut self, owner: &[ElementId], frame: u32, c: &'a Compiled) {
        let Some(i) = self.index_of(owner) else { return };
        let d = self.decls_of(i);
        // A script whose names don't link doesn't run (Test Movie lists why at the start).
        if !d.problems(c).is_empty() {
            return;
        }
        let where_ = format!("Script on frame {} of {}", u64::from(frame) + 1, self.timeline_name(i));
        self.run_code(owner, d, None, &where_, |w, fuel| run::run_script(w, &c.body, &c.inner, fuel));
    }

    /// Runs code on timeline `owner` with this step's fuel; writes why it stopped early.
    fn run_code(&mut self, owner: &[ElementId], decls: Arc<Decls>, ev: Option<EventInfo>, where_: &str, go: impl FnOnce(&mut dyn World, u64) -> (u64, Result<(), Stop>)) {
        if self.halted {
            return;
        }
        if self.fuel == 0 {
            self.halt(&format!("scripts ran more than {MAX_FUEL_PER_STEP} steps"));
            return;
        }
        let fuel = self.fuel.min(run::FUEL_PER_RUN);
        let mut w = Script { r: self, owner: owner.to_vec(), decls, ev };
        let (used, r) = go(&mut w, fuel);
        self.fuel = self.fuel.saturating_sub(used);
        match r {
            Ok(()) | Err(Stop::Halt) => {}
            Err(e) => self.out(format!("{where_} stopped: {}", e.describe())),
        }
    }

    /// How Output names a timeline: the main timeline, or the instance-name path of a clip.
    fn timeline_name(&self, i: usize) -> String {
        if i == 0 {
            return "the main timeline".into();
        }
        let mut names = Vec::new();
        let mut cur = i;
        for _ in 0..=MAX_SYMBOL_DEPTH {
            let Some(n) = self.nodes.get(cur) else { break };
            if n.kind == NodeKind::Root {
                break;
            }
            names.push(if n.name.is_empty() { "?" } else { n.name.as_str() });
            cur = n.owner;
        }
        names.reverse();
        names.join(".")
    }

    /// Calls listener `l` with the event.
    fn call_listener(&mut self, l: &Listening, ev: EventInfo) {
        self.refresh();
        let Some(i) = self.index_of(&l.owner) else { return };
        let d = self.decls_of(i);
        let f = l.func.clone();
        let where_ = format!("Listener {}", l.function);
        self.run_code(&l.owner, d, Some(ev), &where_, |w, fuel| run::run_listener(w, &f, fuel));
    }

    /// Every enter-frame listener, in the order they were added.
    fn enter_frame(&mut self) {
        self.refresh();
        let mut all: Vec<(Option<InstancePath>, Listening)> = self.st.listeners.iter().flat_map(|(p, list)| list.iter().filter(|l| l.event == ListenEvent::EnterFrame).map(|l| (Some(p.clone()), l.clone()))).collect();
        all.extend(self.st.stage.iter().filter(|l| l.event == ListenEvent::EnterFrame).map(|l| (None, l.clone())));
        all.sort_by_key(|(_, l)| l.seq);
        for (at, l) in all {
            if self.halted {
                return;
            }
            self.call_listener(&l, EventInfo { key: None, target: at.clone(), current: at });
            self.settle();
        }
    }

    /// A key event: the stage's listeners for it, in the order added.
    fn key(&mut self, event: ListenEvent, code: u32) {
        let list: Vec<Listening> = self.st.stage.iter().filter(|l| l.event == event).cloned().collect();
        for l in list {
            if self.halted {
                return;
            }
            self.call_listener(&l, EventInfo { key: Some(code), target: None, current: None });
            self.settle();
        }
    }

    /// The node an instance name stands for, from timeline node `from` (`Ok(None)`: the stage).
    fn place(&self, from: usize, ev: Option<&EventInfo>, at: &Place) -> Result<Option<usize>, String> {
        let base = match at.base {
            Base::This => from,
            Base::Stage => {
                return if at.path.is_this() { Ok(None) } else { Err("instances on the stage are named from the main timeline (root)".into()) };
            }
            Base::Target | Base::Current => {
                let Some(ev) = ev else { return Err("there's no event here".into()) };
                let p = if at.base == Base::Target { &ev.target } else { &ev.current };
                match p {
                    None if at.path.is_this() => return Ok(None),
                    None => return Err("the event's target is the stage".into()),
                    Some(p) => self.index_of(p).ok_or("the event's target has left the stage")?,
                }
            }
        };
        self.resolve(base, &at.path).map(Some)
    }

    /// The path of the node `base` starts from (for actions on the event's target).
    fn base_path(&self, owner: &[ElementId], ev: Option<&EventInfo>, base: Base) -> Result<InstancePath, String> {
        match base {
            Base::This => Ok(owner.to_vec()),
            Base::Stage => Err("the stage has no playhead".into()),
            Base::Target | Base::Current => {
                let Some(ev) = ev else { return Err("there's no event here".into()) };
                let p = if base == Base::Target { &ev.target } else { &ev.current };
                p.clone().ok_or_else(|| "the event's target is the stage".into())
            }
        }
    }

    /// A property of node `i` (`None`: the stage).
    fn property(&self, i: Option<usize>, p: Prop) -> Result<Value, String> {
        let num = |v: f64| Ok(Value::Num(v));
        let Some(i) = i else {
            return match p {
                Prop::MouseX => num(short(self.st.mouse.x)),
                Prop::MouseY => num(short(self.st.mouse.y)),
                _ => Err(format!("the stage's `{}` isn't supported here", p.name())),
            };
        };
        let Some(n) = self.nodes.get(i) else { return Err("the instance is gone".into()) };
        if n.kind == NodeKind::Root {
            return match p {
                Prop::X | Prop::Y | Prop::Rotation => num(0.0),
                Prop::Alpha | Prop::ScaleX | Prop::ScaleY => num(1.0),
                Prop::Visible => Ok(Value::Bool(true)),
                Prop::CurrentFrame => num(f64::from(self.st.root.frame) + 1.0),
                Prop::TotalFrames => num(self.doc.scenes.iter().map(|s| f64::from(s.timeline.frame_count())).sum()),
                Prop::CurrentLabel => Ok(current_label(self.doc.timeline(self.st.scene), self.st.root.frame)),
                Prop::MouseX => num(short(self.st.mouse.x)),
                Prop::MouseY => num(short(self.st.mouse.y)),
                Prop::Name => Ok(Value::str("root1")),
                Prop::Text | Prop::Length => Err("the main timeline isn't a text field".into()),
            };
        }
        if matches!(p, Prop::Text | Prop::Length) {
            let Some((t, _)) = n.field() else { return Err(format!("“{}” is {}, not a text field", n.name, kind_name(n.kind))) };
            let v = self.st.fields.get(&n.path).map_or(t.text.as_str(), |f| f.value(t));
            return Ok(if p == Prop::Text { Value::str(v) } else { Value::Num(crate::actions::as3::ir::text_len(v) as f64) });
        }
        let ov = self.st.props.get(&n.path).copied().unwrap_or_default();
        let d = decompose64(&n.local);
        let deg = 180.0 / std::f64::consts::PI;
        let pick = |o: Option<f32>, v: f64| num(o.map_or(fround_short(v), short));
        match p {
            Prop::X => pick(ov.x, d[0]),
            Prop::Y => pick(ov.y, d[1]),
            Prop::ScaleX => pick(ov.scale_x, d[2]),
            Prop::ScaleY => pick(ov.scale_y, d[3]),
            Prop::Rotation => pick(ov.rotation, d[4] * deg),
            Prop::Alpha => num(short(ov.alpha.unwrap_or(n.alpha))),
            Prop::Visible => Ok(Value::Bool(ov.visible != Some(false))),
            Prop::Name => Ok(Value::str(&n.name)),
            Prop::MouseX | Prop::MouseY => {
                let m = n.xf.m.map(f64::from);
                let det = m[0] * m[3] - m[1] * m[2];
                if det == 0.0 || !det.is_finite() {
                    return num(0.0);
                }
                let (x, y) = (f64::from(self.st.mouse.x) - m[4], f64::from(self.st.mouse.y) - m[5]);
                let v = if p == Prop::MouseX { (m[3] * x - m[2] * y) / det } else { (m[0] * y - m[1] * x) / det };
                num(fround_short(v))
            }
            Prop::Text | Prop::Length => Err("not a text field".into()),
            Prop::CurrentFrame | Prop::TotalFrames | Prop::CurrentLabel => {
                if n.kind != NodeKind::MovieClip {
                    return Err(format!("“{}” is {}; only movie clips and the main timeline have frames", n.name, kind_name(n.kind)));
                }
                match p {
                    Prop::CurrentFrame => num(f64::from(n.frame) + 1.0),
                    Prop::TotalFrames => num(f64::from(n.frames)),
                    _ => Ok(n.symbol.and_then(|s| self.doc.symbol(s)).map_or(Value::Null, |s| current_label(&s.timeline, n.frame))),
                }
            }
        }
    }

    /// `addEventListener` / `removeEventListener` from timeline `owner`.
    fn listen(&mut self, owner: &[ElementId], ev: Option<&EventInfo>, at: &Place, event: ListenEvent, f: &Arc<Function>, add: bool) {
        self.refresh();
        let Some(from) = self.index_of(owner) else { return };
        let what = format!("{} for {} ({at})", if add { "Listen" } else { "Stop listening" }, event.key());
        let i = match self.place(from, ev, at) {
            Ok(i) => i,
            Err(e) => return self.out(format!("{what}: {e}")),
        };
        let seq = self.st.seq;
        let item = Listening { event, owner: owner.to_vec(), function: f.name.clone(), func: f.clone(), seq };
        let same = |x: &Listening| x.event == event && x.owner == owner && x.function == f.name;
        let Some(i) = i else {
            // The stage: keyboard and enter-frame listeners.
            if add {
                if self.st.stage.iter().any(same) {
                    return;
                }
                if self.st.stage.len() >= MAX_STAGE_LISTENERS {
                    return self.out(format!("{what}: the stage already has {MAX_STAGE_LISTENERS} listeners"));
                }
                self.st.seq += 1;
                self.st.stage.push(item);
            } else {
                self.st.stage.retain(|x| !same(x));
            }
            return;
        };
        let Some(node) = self.nodes.get(i) else { return };
        let ok = match event {
            ListenEvent::EnterFrame => node.kind != NodeKind::Graphic,
            _ => matches!(node.kind, NodeKind::Button | NodeKind::MovieClip),
        };
        if !ok {
            let why = if node.kind == NodeKind::Root { "the main timeline doesn't get pointer events here".to_string() } else { format!("“{}” is {}; only buttons and movie clips get pointer events", node.name, kind_name(node.kind)) };
            return self.out(format!("{what}: {why}"));
        }
        let (path, name) = (node.path.clone(), node.name.clone());
        if !add {
            if let Some(list) = self.st.listeners.get_mut(&path) {
                list.retain(|x| !same(x));
                if list.is_empty() {
                    self.st.listeners.remove(&path);
                }
            }
            return;
        }
        let list = self.st.listeners.entry(path).or_default();
        if list.iter().any(same) {
            return;
        }
        if list.len() >= MAX_LISTENERS_PER_INSTANCE {
            return self.out(format!("{what}: “{name}” already has {MAX_LISTENERS_PER_INSTANCE} listeners"));
        }
        list.push(item);
        self.st.seq += 1;
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

    /// Runs one action from timeline `owner`. `script`: it comes from ActionScript, where
    /// `nextScene()` on a movie clip's timeline has no other scene to go to.
    fn exec(&mut self, owner: &[ElementId], a: &Action, script: bool) {
        self.refresh();
        let Some(from) = self.index_of(owner) else { return };
        match a {
            Action::Trace { message } => self.out(message.clone()),
            Action::NextScene | Action::PrevScene if script && !owner.is_empty() => {
                self.out(format!("{}: a movie clip's timeline has one scene; call it in the main timeline's script", a.describe()));
            }
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
                    return self.out(format!("{}: “{}” is {}; only movie clips and the main timeline have a playhead", a.describe(), node.name, kind_name(node.kind)));
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
            Event::Move(p) => {
                self.pointer(Some(p));
                // Dragging in the focused field selects.
                if self.st.selecting && self.st.down {
                    self.point_field(p, true);
                }
            }
            Event::Down(p) => {
                self.pointer(Some(p));
                // A field the viewer can use takes the press (unless a button is above it): it
                // gets the keyboard and the caret goes where it was clicked. Anywhere else the
                // focused field loses the keyboard.
                let hover_at = self.st.hover.as_ref().and_then(|h| self.index_of(h));
                match self.field_at(p).filter(|(i, _)| hover_at.is_none_or(|h| *i > h)) {
                    Some((_, path)) => {
                        self.st.focus = Some(path);
                        self.st.down = true;
                        self.st.selecting = true;
                        self.point_field(p, false);
                        return;
                    }
                    None => {
                        self.st.focus = None;
                        self.st.selecting = false;
                    }
                }
                let before = self.pointer_now();
                self.st.down = true;
                self.st.pressed = self.st.hover.clone();
                self.dirty = true;
                self.button_sounds(before);
                if let Some(h) = self.st.hover.clone() {
                    self.fire(&h, ButtonEvent::Press);
                }
            }
            Event::Up(p) => {
                if self.st.selecting {
                    self.st.selecting = false;
                    self.st.down = false;
                    self.pointer(Some(p));
                    return;
                }
                self.pointer(Some(p));
                let before = self.pointer_now();
                self.st.down = false;
                let pressed = self.st.pressed.take();
                self.dirty = true;
                self.button_sounds(before);
                if let Some(h) = self.st.hover.clone() {
                    self.fire(&h, ButtonEvent::Release);
                    if pressed.as_ref() == Some(&h) {
                        self.fire(&h, ButtonEvent::Click);
                    }
                }
            }
            Event::Leave => self.pointer(None),
            // Keys edit the focused field, then go to the stage's listeners as well (as they
            // reach the stage from a focused field in ActionScript).
            Event::KeyDown(k) => {
                if k == SHIFT {
                    self.st.shift = true;
                }
                self.field_key(k);
                self.key(ListenEvent::KeyDown, k);
            }
            Event::KeyUp(k) => {
                if k == SHIFT {
                    self.st.shift = false;
                }
                self.key(ListenEvent::KeyUp, k);
            }
            Event::Char(c) => {
                let mut b = [0u8; 4];
                let s: &str = c.encode_utf8(&mut b);
                self.edit_field(|ed, _| ed.insert(s));
            }
            Event::SelectAll => self.edit_field(|ed, look| ed.key(Edit::SelectAll, false, look)),
        }
    }

    /// The topmost field at stage point `p` the viewer can use, with its node's index.
    fn field_at(&self, p: Vec2) -> Option<(usize, InstancePath)> {
        field_hit(&self.nodes, p)
    }

    /// Field `path`'s block and its place on the stage.
    fn field_node(&self, path: &[ElementId]) -> Option<(Arc<TextBlock>, Affine)> {
        let n = self.index_of(path).and_then(|i| self.nodes.get(i))?;
        let t = n.text.clone().filter(|t| t.field.is_some())?;
        Some((t, n.xf))
    }

    /// Lays field `path`'s text out as it is now (with the host's fonts) and keeps the caret
    /// in the text and in view.
    fn relayout(&mut self, path: &[ElementId]) {
        let Some((t, _)) = self.field_node(path) else { return };
        let Some(f) = t.field.as_ref() else { return };
        let st = self.st.fields.entry(path.to_vec()).or_default();
        let shown = f.shown(st.value(&t));
        st.look = self.host.lay_out_field(&t, &shown).map(Arc::new);
        let len = chars(st.value(&t));
        st.caret = st.caret.min(len);
        st.anchor = st.anchor.min(len);
        scroll_to_caret(&t, f, st);
    }

    /// A script sets field `path`'s text.
    fn set_field_text(&mut self, path: &[ElementId], text: &str) {
        let Some((t, _)) = self.field_node(path) else { return };
        let Some(f) = t.field.as_ref() else { return };
        self.st.fields.entry(path.to_vec()).or_default().text = Some(fit(f, text));
        self.relayout(path);
    }

    /// Runs an edit on the focused field (laying it out first if it never was).
    fn edit_field(&mut self, go: impl FnOnce(&mut Editor<'_>, Option<&FieldLook>) -> Changed) {
        let Some(path) = self.st.focus.clone() else { return };
        let Some((t, _)) = self.field_node(&path) else { return };
        let Some(f) = t.field.clone() else { return };
        if self.st.fields.get(&path).is_none_or(|s| s.look.is_none()) {
            self.relayout(&path);
        }
        let st = self.st.fields.entry(path.clone()).or_default();
        let look = st.look.clone();
        let changed = go(&mut Editor { block: &t, field: &f, st }, look.as_deref());
        if changed.text {
            self.relayout(&path);
        } else if changed.caret {
            scroll_to_caret(&t, &f, self.st.fields.entry(path).or_default());
        }
    }

    /// The pointer at stage point `p` in the focused field: places the caret (`extend`: moves
    /// it, keeping the selection's other end).
    fn point_field(&mut self, p: Vec2, extend: bool) {
        let Some(path) = self.st.focus.clone() else { return };
        let Some((_, xf)) = self.field_node(&path) else { return };
        let Some(q) = xf.inverse().map(|m| m.apply(p)) else { return };
        let q = Vec2::new(q.x - FIELD_GUTTER, q.y - FIELD_GUTTER);
        self.edit_field(|ed, look| ed.point(q, extend, look));
    }

    /// A key down for the focused field: an edit, or Tab to the next input field.
    fn field_key(&mut self, code: u32) {
        if self.st.focus.is_none() {
            return;
        }
        if code == TAB {
            let inputs: Vec<InstancePath> = self.nodes.iter().filter(|n| n.hittable && n.field().is_some_and(|(_, f)| f.kind == TextKind::Input)).map(|n| n.path.clone()).collect();
            let at = inputs.iter().position(|p| Some(p) == self.st.focus.as_ref());
            let n = inputs.len();
            if n > 0 {
                let next = match at {
                    Some(i) if self.st.shift => (i + n - 1) % n,
                    Some(i) => (i + 1) % n,
                    None => 0,
                };
                self.st.focus = inputs.get(next).cloned();
                self.edit_field(|ed, look| ed.key(Edit::SelectAll, false, look));
            }
            return;
        }
        let Some(e) = Edit::from_key(code) else { return };
        let shift = self.st.shift;
        self.edit_field(|ed, look| ed.key(e, shift, look));
    }

    /// Moves the pointer: what it leaves gets Out, what it reaches gets Over.
    fn pointer(&mut self, p: Option<Vec2>) {
        self.refresh();
        self.st.pointer = p;
        if let Some(p) = p {
            self.st.mouse = p;
        }
        let now = p.and_then(|p| hit_in(self.doc, &self.st, &self.nodes, p));
        if now != self.st.hover {
            let before = self.pointer_now();
            let old = std::mem::replace(&mut self.st.hover, now.clone());
            self.dirty = true;
            self.button_sounds(before);
            if let Some(o) = old {
                self.fire(&o, ButtonEvent::Out);
            }
            if let Some(n) = now {
                self.fire(&n, ButtonEvent::Over);
            }
        }
    }

    /// Runs an instance's handler for `event` (in the timeline the instance is on), then the
    /// listeners scripts added for it (each in the timeline of its script), in the order added.
    fn fire(&mut self, path: &[ElementId], event: ButtonEvent) {
        self.refresh();
        let Some(n) = self.index_of(path).and_then(|i| self.nodes.get(i)) else { return };
        let handler = n.events.iter().find(|h| h.event == event).map(|h| h.actions.clone());
        let owner_path = self.nodes.get(n.owner).map(|o| o.path.clone());
        let le = ListenEvent::from_mouse(event);
        let listeners: Vec<Listening> = self.st.listeners.get(path).map(|l| l.iter().filter(|l| l.event == le).cloned().collect()).unwrap_or_default();
        if handler.is_none() && listeners.is_empty() {
            return;
        }
        if let (Some(actions), Some(owner)) = (handler, owner_path) {
            self.run_list(&owner, &actions, false);
        }
        for l in listeners {
            self.call_listener(&l, EventInfo { key: None, target: Some(path.to_vec()), current: Some(path.to_vec()) });
        }
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

/// The key codes of Shift and Tab.
const SHIFT: u32 = 16;
const TAB: u32 = 9;

/// How messages name a kind of instance.
fn kind_name(k: NodeKind) -> &'static str {
    match k {
        NodeKind::Root => "the main timeline",
        NodeKind::Graphic => "a graphic",
        NodeKind::MovieClip => "a movie clip",
        NodeKind::Button => "a button",
        NodeKind::Text => "a text field",
    }
}

/// The event a listener is handling: the key code of a keyboard event, and its target and
/// current target (`None`: the stage).
#[derive(Clone, Debug, PartialEq)]
struct EventInfo {
    key: Option<u32>,
    target: Option<InstancePath>,
    current: Option<InstancePath>,
}

/// A timeline whose scripts share declarations: the main timeline (every scene) or a movie
/// clip symbol's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TimelineRef {
    Root,
    Symbol(SymbolId),
}

/// The compiled frame scripts of a timeline that plays them, in frame order (top layer first
/// within a frame).
pub fn timeline_scripts(tl: &Timeline) -> Vec<(u32, usize, &Compiled)> {
    let mut out = Vec::new();
    for (index, layer) in tl.layers.iter().enumerate() {
        if !plays(tl, index) {
            continue;
        }
        for k in &layer.keyframes {
            if let Some(FrameScript::Source(src)) = &k.script
                && let Ok(c) = src.compiled()
            {
                out.push((k.start, index, c));
            }
        }
    }
    out.sort_by_key(|(f, l, _)| (*f, *l));
    out
}

/// A timeline's declarations: every function and variable its frame scripts declare (the main
/// timeline's from every scene, in scene order).
pub fn decls(doc: &Document, which: TimelineRef) -> Decls {
    match which {
        TimelineRef::Root => Decls::build(doc.scenes.iter().flat_map(|s| timeline_scripts(&s.timeline)).map(|(_, _, c)| c)),
        TimelineRef::Symbol(id) => match doc.symbol(id) {
            Some(s) if s.kind == SymbolKind::MovieClip => Decls::build(timeline_scripts(&s.timeline).into_iter().map(|(_, _, c)| c)),
            _ => Decls::default(),
        },
    }
}

/// Why a compiled script on timeline `which` doesn't run there: names it takes from other
/// frames that the timeline doesn't declare, and declarations that clash.
pub fn link_problems(doc: &Document, which: TimelineRef, c: &Compiled) -> Vec<Problem> {
    decls(doc, which).problems(c)
}

/// The label at or before `frame` (ActionScript's `currentLabel`), or `null`.
fn current_label(tl: &Timeline, frame: u32) -> Value {
    let mut firsts: BTreeMap<&str, u32> = BTreeMap::new();
    for k in tl.layers.iter().flat_map(|l| l.keyframes.iter()).filter(|k| !k.label.is_empty()) {
        let e = firsts.entry(k.label.as_str()).or_insert(k.start);
        *e = (*e).min(k.start);
    }
    let mut best: Option<(&str, u32)> = None;
    for (l, f) in firsts {
        if f <= frame && best.is_none_or(|(_, b)| f > b) {
            best = Some((l, f));
        }
    }
    best.map_or(Value::Null, |(l, _)| Value::str(l))
}

/// An `f32` as the shortest decimal that reads back as it (what the HTML player sees after
/// the export writes it as JSON): reads give `0.1`, not `0.10000000149011612`.
fn short(v: f32) -> f64 {
    if !v.is_finite() {
        return f64::from(v);
    }
    format!("{v}").parse().unwrap_or(f64::from(v))
}

/// A computed value at `f32` precision, written shortest.
fn fround_short(v: f64) -> f64 {
    short(v as f32)
}

/// `[tx, ty, sx, sy, rotation]` of a transform in `f64` (the same steps as `Affine::decompose`
/// and the HTML player's `decompose`), from its components written shortest.
fn decompose64(m: &Affine) -> [f64; 5] {
    let [a, b, c, d, e, f] = m.m.map(short);
    let sx = (a * a + b * b).sqrt();
    let r = b.atan2(a);
    let det = a * d - b * c;
    let sy = if sx > 1e-12 { det / sx } else { (c * c + d * d).sqrt() };
    [e, f, sx, sy, r]
}

/// A running script's view of the movie (ADR 0052).
struct Script<'r, 'a, 'h> {
    r: &'r mut Run<'a, 'h>,
    /// The timeline the script belongs to.
    owner: InstancePath,
    decls: Arc<Decls>,
    ev: Option<EventInfo>,
}

impl Script<'_, '_, '_> {
    fn from(&mut self) -> Result<usize, String> {
        self.r.refresh();
        self.r.index_of(&self.owner).ok_or_else(|| "the script's timeline has left the stage".into())
    }
}

impl World for Script<'_, '_, '_> {
    fn act(&mut self, base: Base, action: &Action) -> Result<(), Option<String>> {
        if !self.r.budget() {
            return Err(None);
        }
        let path = self.r.base_path(&self.owner, self.ev.as_ref(), base).map_err(Some)?;
        self.r.exec(&path, action, true);
        if self.r.halted { Err(None) } else { Ok(()) }
    }

    fn get(&mut self, at: &Place, p: Prop) -> Result<Value, String> {
        let from = self.from()?;
        let i = self.r.place(from, self.ev.as_ref(), at)?;
        self.r.property(i, p)
    }

    fn set(&mut self, at: &Place, p: Property, v: f64) -> Result<(), Option<String>> {
        if at.base == Base::Stage {
            return Err(Some("the stage's properties can't be set here".into()));
        }
        // In range (as the property keeps it), and never -0, so messages read the same here
        // and in the HTML player.
        let (lo, hi) = p.range();
        let v = v.clamp(lo, hi) + 0.0;
        self.act(at.base, &Action::SetProperty { target: at.path.clone(), property: p, value: v })
    }

    fn set_text(&mut self, at: &Place, text: String) -> Result<(), Option<String>> {
        if !self.r.budget() {
            return Err(None);
        }
        let from = self.from().map_err(Some)?;
        let i = self.r.place(from, self.ev.as_ref(), at).map_err(Some)?;
        let Some(n) = i.and_then(|i| self.r.nodes.get(i)) else { return Err(Some("the stage isn't a text field".into())) };
        if n.field().is_none() {
            return Err(Some(format!("“{}” is {}, not a text field", n.name, kind_name(n.kind))));
        }
        let path = n.path.clone();
        self.r.set_field_text(&path, &text);
        Ok(())
    }

    fn trace(&mut self, line: String) {
        if self.r.budget() {
            self.r.out(line);
        }
    }

    fn read(&mut self, name: &str) -> Result<Value, String> {
        if let Some(v) = self.r.st.vars.get(&self.owner).and_then(|m| m.get(name)) {
            return Ok(v.clone());
        }
        self.decls.vars.get(name).map(|t| t.default_value()).ok_or_else(|| format!("`{name}` isn't a variable of this timeline"))
    }

    fn write(&mut self, name: &str, v: Value) -> Result<(), String> {
        let t = *self.decls.vars.get(name).ok_or_else(|| format!("`{name}` isn't a variable of this timeline"))?;
        self.r.st.vars.entry(self.owner.clone()).or_default().insert(name.to_string(), t.coerce(v));
        Ok(())
    }

    fn function(&mut self, name: &str) -> Option<Arc<Function>> {
        self.decls.functions.get(name).cloned()
    }

    fn listen(&mut self, at: &Place, ev: ListenEvent, f: &Arc<Function>, add: bool) -> Result<(), Option<String>> {
        if !self.r.budget() {
            return Err(None);
        }
        let owner = self.owner.clone();
        self.r.listen(&owner, self.ev.as_ref(), at, ev, f, add);
        Ok(())
    }

    fn random(&mut self) -> f64 {
        self.r.st.rng.next_f64()
    }

    fn same(&mut self, a: &Place, b: &Place) -> Result<bool, String> {
        let from = self.from()?;
        let x = self.r.place(from, self.ev.as_ref(), a)?;
        let y = self.r.place(from, self.ev.as_ref(), b)?;
        Ok(x == y)
    }

    fn key_code(&mut self) -> Result<f64, String> {
        self.ev.as_ref().and_then(|e| e.key).map(f64::from).ok_or_else(|| "this isn't a keyboard event".into())
    }
}

#[cfg(test)]
#[path = "player_tests.rs"]
mod tests;
