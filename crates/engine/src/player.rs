//! The player's engine side (ADR 0024): plug-in frame scripts run on the WebAssembly host, a
//! JSON view of the player state for scripts and agents, and headless test runs.
//!
//! The player itself is pure (`anicraft_doc::player`); this module gives it a
//! [`ScriptHost`] over the session's plug-in registry. A frame script is a `frame-script`
//! plug-in entry point: it runs in a fresh instance when the playhead enters its keyframe,
//! reads the player state (`{"op":"player"}`), emits actions (`{"op":"action"}`) and logs to
//! the Output panel; it can't call commands. Every frame shares one instruction budget among
//! its scripts, so a script that loops forever costs one frame's budget, then stops with a
//! message.

use anicraft_doc::actions::as3::Problem;
use anicraft_doc::actions::{Action, CompileResult, FrameScript, MAX_ACTIONS, PluginScript, check_actions};
use anicraft_doc::player::{Event, Node, NodeKind, PlayerState, ScriptContext, ScriptHost, ScriptRun, Step, TimelineRef};
use anicraft_doc::field::{CaretLine, FieldLook};
use anicraft_doc::{Document, LayerKind, SymbolKind, TextBlock, Timeline, player};
use anicraft_geom::Vec2;
use anicraft_plugins::{Host, Kind, Limits, Registry};
use serde_json::{Map, Value, json};

/// Most movie clips the `player` view lists.
const MAX_VIEW_CLIPS: usize = 1_000;

/// Runs frame scripts with the session's installed plug-ins.
pub struct ScriptRunner<'a> {
    plugins: &'a Registry,
    limits: Limits,
    /// Instructions left for this frame's scripts.
    fuel_left: u64,
    /// Instructions frame scripts have used since the runner was made.
    pub fuel_used: u64,
    /// Frame-script runs since the runner was made.
    pub runs: u64,
}

impl<'a> ScriptRunner<'a> {
    pub fn new(plugins: &'a Registry) -> ScriptRunner<'a> {
        let limits = Limits::frame_script();
        ScriptRunner { plugins, fuel_left: limits.fuel, limits, fuel_used: 0, runs: 0 }
    }

    /// Other limits (tests use a small budget); `limits.fuel` is the per-frame budget.
    pub fn with_limits(mut self, limits: Limits) -> Self {
        self.fuel_left = limits.fuel;
        self.limits = limits;
        self
    }
}

impl ScriptHost for ScriptRunner<'_> {
    fn begin_step(&mut self) {
        self.fuel_left = self.limits.fuel;
    }

    fn run(&mut self, script: &PluginScript, cx: &ScriptContext<'_>) -> ScriptRun {
        let fail = |msg: String| ScriptRun { actions: Vec::new(), output: vec![msg] };
        let Some(p) = self.plugins.get(&script.id) else { return fail(format!("Frame script: no plug-in “{}” is installed", script.id)) };
        let m = p.manifest();
        if m.kind != Kind::FrameScript {
            return fail(format!("Frame script: “{}” is a command plug-in, not a frame script", m.name));
        }
        if !m.entries.contains(&script.entry) {
            return fail(format!("Frame script: {} has no entry point “{}” (it has {})", m.name, script.entry, m.entries.join(", ")));
        }
        if self.fuel_left == 0 {
            return fail(format!("Frame script {} · {} not run: this frame's script budget is used up", m.name, script.entry));
        }
        let mut params = Map::new();
        params.insert("entry".into(), json!(script.entry));
        params.insert("frame".into(), json!(cx.frame));
        params.insert("label".into(), json!(cx.label));
        params.insert("timeline".into(), json!(path_name(cx.nodes, cx.timeline)));
        let mut host = FrameHost { cx, actions: Vec::new() };
        let mut limits = self.limits.clone();
        limits.fuel = self.fuel_left;
        self.runs += 1;
        match p.run_with(&mut host, &params, &limits) {
            Ok(rep) => {
                self.fuel_left = self.fuel_left.saturating_sub(rep.fuel);
                self.fuel_used = self.fuel_used.saturating_add(rep.fuel);
                let mut output = rep.log;
                if rep.log_dropped > 0 {
                    output.push(format!("… {} more lines from {}", rep.log_dropped, m.name));
                }
                ScriptRun { actions: host.actions, output }
            }
            Err(e) => {
                // A failed script does nothing: its actions are dropped, and it used this
                // frame's budget.
                self.fuel_used = self.fuel_used.saturating_add(self.fuel_left);
                self.fuel_left = 0;
                fail(format!("Frame script {} · {}: {e}", m.name, script.entry))
            }
        }
    }

    fn lay_out_field(&mut self, block: &TextBlock, shown: &str) -> Option<FieldLook> {
        field_look(block, shown)
    }
}

/// A field's text laid out with this computer's fonts, as the player draws and edits it
/// (ADR 0066).
pub fn field_look(block: &TextBlock, shown: &str) -> Option<FieldLook> {
    let f = block.field.as_ref()?;
    let l = crate::cmds_text::field_layout(anicraft_text::Fonts::get(), &crate::cmds_text::style_of(block), f, block.box_width, shown).ok()?;
    let mut lines: Vec<CaretLine> = l.carets.into_iter().map(|c| CaretLine { start: c.start, end: c.end, stops: c.stops }).collect();
    if lines.is_empty() {
        lines.push(CaretLine { start: 0, end: 0, stops: vec![(0, 0.0)] });
    }
    Some(FieldLook { outline: std::sync::Arc::new(l.outline), lines: lines.into(), line_height: l.line_height, width: l.width, height: l.height })
}

/// What a running frame script talks to.
struct FrameHost<'c, 'd> {
    cx: &'c ScriptContext<'d>,
    actions: Vec<Action>,
}

impl Host for FrameHost<'_, '_> {
    fn request(&mut self, op: &str, req: &Value) -> Result<Value, String> {
        match op {
            "player" => Ok(state_view(self.cx.doc, self.cx.state, self.cx.nodes, Some((self.cx.timeline, self.cx.frame, self.cx.label)))),
            "action" | "actions" => {
                let list: Vec<Action> = if op == "action" {
                    vec![serde_json::from_value(req.get("action").cloned().unwrap_or(Value::Null)).map_err(|e| format!("not an action: {e} (e.g. {{\"op\":\"action\",\"action\":{{\"op\":\"stop\"}}}})"))?]
                } else {
                    serde_json::from_value(req.get("actions").cloned().unwrap_or(Value::Null)).map_err(|e| format!("not a list of actions: {e}"))?
                };
                check_actions(&list)?;
                if self.actions.len() + list.len() > MAX_ACTIONS {
                    return Err(format!("a frame script emits at most {MAX_ACTIONS} actions"));
                }
                let n = list.len();
                self.actions.extend(list);
                Ok(json!({"queued": n}))
            }
            "command" | "document" | "elements" => Err("frame scripts run while the movie plays and can't read or change the document; read {\"op\":\"player\"} and emit {\"op\":\"action\"}".into()),
            _ => Err(format!("unknown op '{op}' (frame scripts have player, action, actions and log)")),
        }
    }
}

/// The dot path of instance names from the main timeline to node `i` (`""` for the main
/// timeline; an unnamed instance shows as `?`).
pub fn path_name(nodes: &[Node], i: usize) -> String {
    let mut names = Vec::new();
    let mut cur = i;
    for _ in 0..=anicraft_doc::MAX_SYMBOL_DEPTH {
        let Some(n) = nodes.get(cur) else { break };
        if n.kind == NodeKind::Root {
            break;
        }
        names.push(if n.name.is_empty() { "?" } else { n.name.as_str() });
        // Up to the timeline it is on (graphics are see-through).
        cur = n.owner;
    }
    names.reverse();
    names.join(".")
}

/// The player state as JSON (for frame scripts, `control.test_movie` and the app's readouts).
/// `here` is the script's own timeline, frame and label.
pub fn state_view(doc: &Document, st: &PlayerState, nodes: &[Node], here: Option<(usize, u32, &str)>) -> Value {
    let clips: Vec<Value> = nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.kind == NodeKind::MovieClip)
        .take(MAX_VIEW_CLIPS)
        .map(|(i, n)| json!({"path": path_name(nodes, i), "name": n.name, "frame": n.frame, "frames": n.frames, "playing": st.clips.get(&n.path).is_some_and(|c| c.playing)}))
        .collect();
    let path_of = |p: &Option<Vec<anicraft_doc::ElementId>>| p.as_ref().and_then(|p| nodes.iter().position(|n| &n.path == p)).map(|i| path_name(nodes, i));
    let mut v = json!({
        "scene": st.scene,
        "scene_name": doc.scenes.get(st.scene).map(|s| s.name.clone()),
        "scenes": doc.scenes.iter().map(|s| s.name.clone()).collect::<Vec<_>>(),
        "frame": st.root.frame,
        "frames": doc.timeline(st.scene).frame_count(),
        "playing": st.root.playing,
        "clips": clips,
        "pointer": st.pointer.map(|p| json!({"x": p.x, "y": p.y, "down": st.down})),
        "hover": path_of(&st.hover),
        "tick": st.tick,
    });
    // Text fields (ADR 0066): their text as it is now, and which has the keyboard.
    let fields: Vec<Value> = nodes
        .iter()
        .enumerate()
        .filter_map(|(i, n)| n.field().map(|(t, f)| (i, n, t, f)))
        .take(MAX_VIEW_CLIPS)
        .map(|(i, n, t, f)| {
            let fs = st.fields.get(&n.path);
            let text = fs.map_or(t.text.as_str(), |x| x.value(t));
            let mut o = json!({"path": path_name(nodes, i), "name": n.name, "type": f.kind.key(), "text": text});
            if st.focus.as_ref() == Some(&n.path) {
                let (a, b) = fs.map_or((0, 0), |x| x.selection());
                o["focus"] = json!({"caret": fs.map_or(0, |x| x.caret), "selection": [a, b]});
            }
            o
        })
        .collect();
    if !fields.is_empty() {
        v["fields"] = Value::Array(fields);
    }
    if let Some((i, frame, label)) = here {
        let n = nodes.get(i);
        v["timeline"] = json!({"path": path_name(nodes, i), "frame": frame, "frames": n.map_or(1, |n| n.frames), "label": label, "playing": n.and_then(|n| st.playhead(&n.path)).is_some_and(|c| c.playing)});
    }
    v
}

/// A movie playing headless: the player, the frame scripts and the Output panel's lines.
pub struct Movie {
    pub state: PlayerState,
    /// Every output line so far, with the step it was written in.
    pub output: Vec<(u64, String)>,
    /// Sounds started since the owner last took them ([`Movie::take_sounds`]), with the step
    /// that started each (ADR 0039).
    sounds: Vec<(u64, anicraft_doc::sound::Started)>,
}

/// Most output lines a [`Movie`] keeps.
pub const MAX_MOVIE_OUTPUT: usize = 10_000;
/// Most started sounds a [`Movie`] holds for its owner (older ones are dropped).
pub const MAX_MOVIE_SOUNDS: usize = 4_096;

impl Movie {
    /// The movie at its first frame. The Output starts with a line for each kept script: which
    /// run and which are skipped and why (ADR 0013 tier 2), said once rather than each time the
    /// playhead passes.
    pub fn start(doc: &Document, scripts: &mut dyn ScriptHost) -> Movie {
        Movie::start_seeded(doc, player::DEFAULT_SEED, scripts)
    }

    /// [`Movie::start`] with `Math.random()` seeded by `seed`.
    pub fn start_seeded(doc: &Document, seed: u64, scripts: &mut dyn ScriptHost) -> Movie {
        let mut m = Movie { state: PlayerState::default(), output: Vec::new(), sounds: Vec::new() };
        m.output.extend(script_summary(doc).into_iter().map(|l| (0, l)));
        let s = player::start_seeded(doc, seed, scripts);
        m.take(s);
        m
    }

    pub fn step(&mut self, doc: &Document, events: &[Event], scripts: &mut dyn ScriptHost) {
        let s = player::step(doc, &self.state, events, scripts);
        self.take(s);
    }

    /// Pointer events between frames (no playhead moves).
    pub fn input(&mut self, doc: &Document, events: &[Event], scripts: &mut dyn ScriptHost) {
        if !events.is_empty() {
            let s = player::input(doc, &self.state, events, scripts);
            self.take(s);
        }
    }

    fn take(&mut self, s: Step) {
        self.state = s.state;
        for line in s.output {
            if self.output.len() < MAX_MOVIE_OUTPUT {
                self.output.push((self.state.tick, line));
            }
        }
        let tick = self.state.tick;
        self.sounds.extend(s.sounds.into_iter().map(|x| (tick, x)));
        if self.sounds.len() > MAX_MOVIE_SOUNDS {
            let extra = self.sounds.len() - MAX_MOVIE_SOUNDS;
            self.sounds.drain(..extra);
        }
    }

    /// The sounds started since the last call, with the step (tick) that started each: what
    /// a window playing the movie should start hearing.
    pub fn take_sounds(&mut self) -> Vec<(u64, anicraft_doc::sound::Started)> {
        std::mem::take(&mut self.sounds)
    }
}

/// The ActionScript key code a key event names: a number (1-255), a `Keyboard` constant
/// (`"LEFT"`, `"SPACE"`) or one letter or digit.
pub fn parse_key(v: &Value) -> Result<u32, String> {
    match v {
        Value::Number(n) => n.as_u64().filter(|k| (1..=255).contains(k)).map(|k| k as u32).ok_or_else(|| "a key code is 1-255".to_string()),
        Value::String(s) => {
            let t = s.trim();
            let one = t.chars().count() == 1 && t.chars().all(|c| c.is_ascii_alphanumeric());
            if one {
                let c = t.to_ascii_uppercase();
                return anicraft_doc::actions::as3::ir::key_code(&c).or_else(|| anicraft_doc::actions::as3::ir::key_code(&format!("NUMBER_{c}"))).ok_or_else(|| format!("unknown key '{t}'"));
            }
            anicraft_doc::actions::as3::ir::key_code(&t.to_ascii_uppercase()).ok_or_else(|| format!("unknown key '{t}' (a key code, a letter, or a Keyboard name such as LEFT or SPACE)"))
        }
        _ => Err("a key event needs a \"key\": a key code, a letter, or a Keyboard name such as LEFT".into()),
    }
}

/// Most characters one `text` event types.
pub const MAX_TYPED: usize = 10_000;

/// Parses an event: a pointer event `{type: move|down|up|click|leave, x, y}` (a click is a
/// press and a release at one point), a key event `{type: key_down|key_up|key, key}` (`key`
/// is a press and a release), or typing into the focused text field (ADR 0066):
/// `{type: text, text}` (each character typed in turn) and `{type: select_all}`.
pub fn parse_events(v: &Value) -> Result<Vec<Event>, String> {
    let kind = v.get("type").and_then(Value::as_str).ok_or("an event needs a \"type\": move, down, up, click, leave, key_down, key_up, key, text or select_all")?;
    if kind == "text" {
        let t = v.get("text").and_then(Value::as_str).ok_or("a text event needs a \"text\" to type")?;
        if t.chars().count() > MAX_TYPED {
            return Err(format!("a text event types at most {MAX_TYPED} characters"));
        }
        return Ok(t.chars().map(Event::Char).collect());
    }
    if kind == "select_all" {
        return Ok(vec![Event::SelectAll]);
    }
    if matches!(kind, "key_down" | "key_up" | "key") {
        let k = parse_key(v.get("key").unwrap_or(&Value::Null))?;
        return Ok(match kind {
            "key_down" => vec![Event::KeyDown(k)],
            "key_up" => vec![Event::KeyUp(k)],
            _ => vec![Event::KeyDown(k), Event::KeyUp(k)],
        });
    }
    let num = |k: &str| -> Result<f32, String> { v.get(k).and_then(Value::as_f64).filter(|f| f.is_finite() && f.abs() <= 1e7).map(|f| f as f32).ok_or(format!("a {kind} event needs a finite \"{k}\"")) };
    Ok(match kind {
        "leave" => vec![Event::Leave],
        "move" => vec![Event::Move(Vec2::new(num("x")?, num("y")?))],
        "down" => vec![Event::Down(Vec2::new(num("x")?, num("y")?))],
        "up" => vec![Event::Up(Vec2::new(num("x")?, num("y")?))],
        "click" => {
            let p = Vec2::new(num("x")?, num("y")?);
            vec![Event::Move(p), Event::Down(p), Event::Up(p)]
        }
        other => return Err(format!("unknown event type '{other}' (move, down, up, click, leave, key_down, key_up, key, text or select_all)")),
    })
}

/// A kept script (ADR 0013) and where it is.
pub struct KeptScript<'a> {
    /// "Scene 1 › Actions, frame 1" (frames 1-based, as the app shows them).
    pub place: String,
    /// The timeline runs frame scripts there (a scene or a movie clip, on a layer that plays).
    pub plays: bool,
    pub compiled: &'a CompileResult,
    /// Why it doesn't run on its timeline although it compiles: names from other frames that
    /// aren't there (ADR 0052).
    pub links: Vec<Problem>,
}

impl KeptScript<'_> {
    /// It runs when the movie plays.
    pub fn runs(&self) -> bool {
        self.plays && self.compiled.is_ok() && self.links.is_empty()
    }
}

/// Most kept scripts listed.
pub const MAX_LISTED_SCRIPTS: usize = 1_000;

fn layer_plays(tl: &Timeline, index: usize) -> bool {
    tl.layers.get(index).is_some_and(|l| l.visible && !matches!(l.kind, LayerKind::Folder | LayerKind::Guide)) && !tl.hidden_by_folder(index)
}

/// Every kept script in the document: the scenes' first, then the symbols'.
pub fn kept_scripts(doc: &Document) -> Vec<KeptScript<'_>> {
    let scenes = doc.scenes.iter().map(|s| (s.name.as_str(), &s.timeline, Some(TimelineRef::Root)));
    let symbols = doc.library.iter().map(|s| (s.name.as_str(), &s.timeline, (s.kind == SymbolKind::MovieClip).then_some(TimelineRef::Symbol(s.id))));
    let mut out = Vec::new();
    let mut root = None;
    for (name, tl, which) in scenes.chain(symbols) {
        let runs = which.is_some();
        let decls = match which {
            Some(TimelineRef::Root) => root.get_or_insert_with(|| player::decls(doc, TimelineRef::Root)).clone(),
            Some(w) => player::decls(doc, w),
            None => Default::default(),
        };
        for (index, layer) in tl.layers.iter().enumerate() {
            for k in &layer.keyframes {
                let Some(FrameScript::Source(src)) = &k.script else { continue };
                if out.len() >= MAX_LISTED_SCRIPTS {
                    return out;
                }
                let links = match src.compiled() {
                    Ok(c) if runs => decls.problems(c),
                    _ => Vec::new(),
                };
                out.push(KeptScript { place: format!("{name} › {}, frame {}", layer.name, u64::from(k.start) + 1), plays: runs && layer_plays(tl, index), compiled: src.compiled(), links });
            }
        }
    }
    out
}

/// The Output's opening lines: each kept script that runs (and what in it is ignored), and
/// each that doesn't, with why.
pub fn script_summary(doc: &Document) -> Vec<String> {
    let mut lines = Vec::new();
    for k in kept_scripts(doc) {
        match (k.plays, k.compiled) {
            (false, _) => lines.push(format!("Frame script not run: {}: graphic and button timelines, guide layers and hidden layers don't run frame scripts", k.place)),
            (true, Ok(c)) if k.links.is_empty() => {
                lines.push(format!("Frame script runs: {}", k.place));
                lines.extend(c.notes.iter().map(|n| format!("Frame script at {}: {n}", k.place)));
            }
            (true, Ok(_)) => lines.push(not_run(&k.place, &k.links)),
            (true, Err(problems)) => lines.push(not_run(&k.place, problems)),
        }
    }
    lines
}

/// "Frame script not run: …" with the first problem and how many more there are.
fn not_run(place: &str, problems: &[Problem]) -> String {
    let first = problems.first().map_or_else(String::new, ToString::to_string);
    let more = match problems.len() {
        0 | 1 => String::new(),
        2 => " (and 1 more problem: see the Actions panel)".into(),
        n => format!(" (and {} more problems: see the Actions panel)", n - 1),
    };
    format!("Frame script not run: {place}: {first}{more}")
}
