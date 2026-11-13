//! The timeline action IR (layer L0; ADR 0013, ADR 0024).
//!
//! A small, typed vocabulary for what interactive content does: stop and play timelines, go to
//! a frame or a label (optionally in a scene), step frames and scenes, address a movie clip
//! instance by its instance-name path, react to button events, write `trace` output and set a
//! few instance properties. Every front end speaks it: frame actions edited in the Actions
//! panel, WebAssembly frame scripts (which emit actions through the plug-in ABI) and, later, the
//! ActionScript subset ([`as3`], ADR 0013 tier 2) and the HTML canvas exporter. The player in
//! `anicraft-doc` runs it. Scripts kept as written ([`SourceScript`], ADR 0013 tier 1) sit beside
//! the actions in [`FrameScript`]; those in the ActionScript 3.0 subset compile to actions once
//! and run.
//!
//! This crate knows nothing about documents: targets are name paths and frames are numbers or
//! labels, resolved by the player when an action runs. Frames are 0-based, as everywhere in
//! the engine (the app shows them 1-based).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod as3;

use serde::{Deserialize, Serialize};
use std::sync::{Arc, OnceLock};

/// Most actions in one list (a keyframe's, or one event handler's).
pub const MAX_ACTIONS: usize = 256;
/// Most event handlers on one instance (one per event).
pub const MAX_HANDLERS: usize = 16;
/// Longest instance name (and target path segment).
pub const MAX_NAME_CHARS: usize = 64;
/// Longest target path, in characters and in segments.
pub const MAX_TARGET_CHARS: usize = 256;
pub const MAX_TARGET_DEPTH: usize = 16;
/// Longest frame label or scene name an action may name.
pub const MAX_LABEL_CHARS: usize = 256;
/// Longest `trace` message.
pub const MAX_TRACE_CHARS: usize = 1_000;
/// Highest frame an action may name (the engine's last addressable frame).
pub const MAX_FRAME: u32 = 100_000;

/// Which timeline (or instance) an action acts on: a dot path of instance names relative to the
/// timeline the action belongs to. `""` or `this` is that timeline, `hero` its child instance
/// named `hero`, `hero.arm` the instance `arm` inside that, `parent` the timeline one level up
/// and `root` the main timeline (`_root` and `_parent` are accepted too).
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Target(pub String);

/// One step of a [`Target`] path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seg<'a> {
    Root,
    Parent,
    Child(&'a str),
}

impl Target {
    /// The timeline the action is on.
    pub fn this() -> Target {
        Target(String::new())
    }
    pub fn new(path: impl Into<String>) -> Target {
        Target(path.into())
    }
    pub fn is_this(&self) -> bool {
        matches!(self.0.trim(), "" | "this")
    }
    /// The path's steps, or why it isn't a valid path.
    pub fn segments(&self) -> Result<Vec<Seg<'_>>, String> {
        let s = self.0.trim();
        if s.chars().count() > MAX_TARGET_CHARS {
            return Err(format!("the target path is longer than {MAX_TARGET_CHARS} characters"));
        }
        let mut out = Vec::new();
        if s.is_empty() {
            return Ok(out);
        }
        for (i, part) in s.split('.').enumerate() {
            let seg = match part.trim() {
                "this" if i == 0 => continue,
                "root" | "_root" if i == 0 => Seg::Root,
                "parent" | "_parent" => Seg::Parent,
                "" => return Err(format!("the target '{s}' has an empty step")),
                n if valid_instance_name(n) => Seg::Child(n),
                n => return Err(format!("'{n}' in the target '{s}' is not an instance name")),
            };
            out.push(seg);
            if out.len() > MAX_TARGET_DEPTH {
                return Err(format!("the target '{s}' is deeper than {MAX_TARGET_DEPTH} steps"));
            }
        }
        Ok(out)
    }
}

impl std::fmt::Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.is_this() { f.write_str("this") } else { f.write_str(self.0.trim()) }
    }
}

/// A frame by number (0-based) or by its label.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FrameRef {
    Number(u32),
    Label(String),
}

impl Default for FrameRef {
    fn default() -> Self {
        FrameRef::Number(0)
    }
}

impl std::fmt::Display for FrameRef {
    /// As the app shows it: numbers 1-based, labels quoted.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FrameRef::Number(n) => write!(f, "frame {}", u64::from(*n) + 1),
            FrameRef::Label(l) => write!(f, "“{l}”"),
        }
    }
}

/// An instance property an action may set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Property {
    /// Position of the instance's registration point, in the parent's space (pixels).
    X,
    Y,
    /// Opacity, 0 to 1.
    Alpha,
    /// 0 hides the instance (it keeps playing), anything else shows it.
    Visible,
    /// Degrees, clockwise.
    Rotation,
    /// Scale factors (1 = 100 %).
    ScaleX,
    ScaleY,
}

impl Property {
    pub const ALL: [Property; 7] = [Property::X, Property::Y, Property::Alpha, Property::Visible, Property::Rotation, Property::ScaleX, Property::ScaleY];

    pub fn key(self) -> &'static str {
        match self {
            Property::X => "x",
            Property::Y => "y",
            Property::Alpha => "alpha",
            Property::Visible => "visible",
            Property::Rotation => "rotation",
            Property::ScaleX => "scale_x",
            Property::ScaleY => "scale_y",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Property::X => "X",
            Property::Y => "Y",
            Property::Alpha => "Alpha",
            Property::Visible => "Visible",
            Property::Rotation => "Rotation",
            Property::ScaleX => "Scale X",
            Property::ScaleY => "Scale Y",
        }
    }
    pub fn from_key(s: &str) -> Option<Property> {
        Property::ALL.into_iter().find(|p| p.key() == s.trim())
    }
    /// The values the property accepts; others are clamped into it.
    pub fn range(self) -> (f64, f64) {
        match self {
            Property::X | Property::Y => (-1e6, 1e6),
            Property::Alpha | Property::Visible => (0.0, 1.0),
            Property::Rotation => (-36_000.0, 36_000.0),
            Property::ScaleX | Property::ScaleY => (-1_000.0, 1_000.0),
        }
    }
}

/// One timeline action.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Action {
    /// Stops the target timeline's playhead.
    Stop {
        #[serde(default, skip_serializing_if = "Target::is_this")]
        target: Target,
    },
    /// Starts it again.
    Play {
        #[serde(default, skip_serializing_if = "Target::is_this")]
        target: Target,
    },
    /// Moves the playhead to a frame and plays from there. `scene` (main timeline only) names
    /// the scene the frame is in.
    GotoAndPlay {
        #[serde(default, skip_serializing_if = "Target::is_this")]
        target: Target,
        #[serde(default)]
        frame: FrameRef,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scene: Option<String>,
    },
    /// Moves the playhead to a frame and stops there.
    GotoAndStop {
        #[serde(default, skip_serializing_if = "Target::is_this")]
        target: Target,
        #[serde(default)]
        frame: FrameRef,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scene: Option<String>,
    },
    /// One frame on (or back), and stop.
    NextFrame {
        #[serde(default, skip_serializing_if = "Target::is_this")]
        target: Target,
    },
    PrevFrame {
        #[serde(default, skip_serializing_if = "Target::is_this")]
        target: Target,
    },
    /// The main timeline's next (or previous) scene, from its first frame.
    NextScene,
    PrevScene,
    /// A line in the Output panel.
    Trace {
        #[serde(default)]
        message: String,
    },
    /// Sets an instance property (the timeline no longer animates it on that instance).
    SetProperty {
        #[serde(default, skip_serializing_if = "Target::is_this")]
        target: Target,
        property: Property,
        value: f64,
    },
}

/// The kinds of action, `(op, label)`, in the order the Actions panel offers them.
pub const KINDS: [(&str, &str); 10] = [
    ("stop", "Stop"),
    ("play", "Play"),
    ("goto_and_play", "Go to and play"),
    ("goto_and_stop", "Go to and stop"),
    ("next_frame", "Next frame"),
    ("prev_frame", "Previous frame"),
    ("next_scene", "Next scene"),
    ("prev_scene", "Previous scene"),
    ("trace", "Trace"),
    ("set_property", "Set property"),
];

impl Action {
    /// A new action of kind `op` (see [`KINDS`]) with default settings.
    pub fn new(op: &str) -> Option<Action> {
        let t = Target::this;
        Some(match op {
            "stop" => Action::Stop { target: t() },
            "play" => Action::Play { target: t() },
            "goto_and_play" => Action::GotoAndPlay { target: t(), frame: FrameRef::default(), scene: None },
            "goto_and_stop" => Action::GotoAndStop { target: t(), frame: FrameRef::default(), scene: None },
            "next_frame" => Action::NextFrame { target: t() },
            "prev_frame" => Action::PrevFrame { target: t() },
            "next_scene" => Action::NextScene,
            "prev_scene" => Action::PrevScene,
            "trace" => Action::Trace { message: String::new() },
            "set_property" => Action::SetProperty { target: t(), property: Property::Alpha, value: 1.0 },
            _ => return None,
        })
    }

    /// Its kind, as in [`KINDS`].
    pub fn op(&self) -> &'static str {
        match self {
            Action::Stop { .. } => "stop",
            Action::Play { .. } => "play",
            Action::GotoAndPlay { .. } => "goto_and_play",
            Action::GotoAndStop { .. } => "goto_and_stop",
            Action::NextFrame { .. } => "next_frame",
            Action::PrevFrame { .. } => "prev_frame",
            Action::NextScene => "next_scene",
            Action::PrevScene => "prev_scene",
            Action::Trace { .. } => "trace",
            Action::SetProperty { .. } => "set_property",
        }
    }

    pub fn label(&self) -> &'static str {
        let op = self.op();
        KINDS.iter().find(|(k, _)| *k == op).map_or("Action", |(_, l)| l)
    }

    /// The target, for actions that have one.
    pub fn target(&self) -> Option<&Target> {
        match self {
            Action::Stop { target } | Action::Play { target } | Action::NextFrame { target } | Action::PrevFrame { target } => Some(target),
            Action::GotoAndPlay { target, .. } | Action::GotoAndStop { target, .. } | Action::SetProperty { target, .. } => Some(target),
            Action::NextScene | Action::PrevScene | Action::Trace { .. } => None,
        }
    }

    pub fn target_mut(&mut self) -> Option<&mut Target> {
        match self {
            Action::Stop { target } | Action::Play { target } | Action::NextFrame { target } | Action::PrevFrame { target } => Some(target),
            Action::GotoAndPlay { target, .. } | Action::GotoAndStop { target, .. } | Action::SetProperty { target, .. } => Some(target),
            Action::NextScene | Action::PrevScene | Action::Trace { .. } => None,
        }
    }

    /// Whether the action is well formed; the error says what is wrong.
    pub fn check(&self) -> Result<(), String> {
        if let Some(t) = self.target() {
            t.segments()?;
        }
        match self {
            Action::GotoAndPlay { frame, scene, .. } | Action::GotoAndStop { frame, scene, .. } => {
                match frame {
                    FrameRef::Number(n) if *n > MAX_FRAME => return Err(format!("frame {n} is past the last frame ({MAX_FRAME})")),
                    FrameRef::Label(l) if !printable(l, MAX_LABEL_CHARS) => return Err(format!("a frame label must be 1-{MAX_LABEL_CHARS} printable characters")),
                    _ => {}
                }
                if let Some(s) = scene
                    && !printable(s, MAX_LABEL_CHARS)
                {
                    return Err(format!("a scene name must be 1-{MAX_LABEL_CHARS} printable characters"));
                }
            }
            Action::Trace { message } if message.chars().count() > MAX_TRACE_CHARS => return Err(format!("a trace message is at most {MAX_TRACE_CHARS} characters")),
            Action::SetProperty { value, .. } if !value.is_finite() => return Err("the value must be a finite number".into()),
            _ => {}
        }
        Ok(())
    }

    /// The action repaired for loading from a file: strings shortened, numbers clamped;
    /// `None` when it can't be repaired (a malformed target).
    pub fn sanitized(mut self) -> Option<Action> {
        let cut = |s: &mut String, n: usize| {
            if s.chars().count() > n {
                *s = s.chars().take(n).collect();
            }
        };
        match &mut self {
            Action::GotoAndPlay { frame, scene, .. } | Action::GotoAndStop { frame, scene, .. } => {
                match frame {
                    FrameRef::Number(n) => *n = (*n).min(MAX_FRAME),
                    FrameRef::Label(l) => cut(l, MAX_LABEL_CHARS),
                }
                if let Some(s) = scene {
                    cut(s, MAX_LABEL_CHARS);
                }
                if scene.as_deref().is_some_and(|s| s.trim().is_empty()) {
                    *scene = None;
                }
            }
            Action::Trace { message } => cut(message, MAX_TRACE_CHARS),
            Action::SetProperty { property, value, .. } => {
                let (lo, hi) = property.range();
                *value = if value.is_finite() { value.clamp(lo, hi) } else { lo.max(0.0).min(hi) };
            }
            _ => {}
        }
        self.check().is_ok().then_some(self)
    }

    /// A one-line description, as the Actions panel and `actions.list` show it.
    pub fn describe(&self) -> String {
        let on = |t: &Target| if t.is_this() { String::new() } else { format!(" ({t})") };
        let scene = |s: &Option<String>| s.as_ref().map_or_else(String::new, |s| format!(" in “{s}”"));
        match self {
            Action::Stop { target } => format!("Stop{}", on(target)),
            Action::Play { target } => format!("Play{}", on(target)),
            Action::GotoAndPlay { target, frame, scene: s } => format!("Go to {frame}{} and play{}", scene(s), on(target)),
            Action::GotoAndStop { target, frame, scene: s } => format!("Go to {frame}{} and stop{}", scene(s), on(target)),
            Action::NextFrame { target } => format!("Next frame{}", on(target)),
            Action::PrevFrame { target } => format!("Previous frame{}", on(target)),
            Action::NextScene => "Next scene".into(),
            Action::PrevScene => "Previous scene".into(),
            Action::Trace { message } => format!("Trace “{message}”"),
            Action::SetProperty { target, property, value } => format!("Set {} to {value}{}", property.label().to_lowercase(), on(target)),
        }
    }
}

/// Checks a list of actions (count and each action).
pub fn check_actions(list: &[Action]) -> Result<(), String> {
    if list.len() > MAX_ACTIONS {
        return Err(format!("at most {MAX_ACTIONS} actions in one list"));
    }
    for (i, a) in list.iter().enumerate() {
        a.check().map_err(|e| format!("action {}: {e}", i + 1))?;
    }
    Ok(())
}

/// Repairs a loaded list: at most [`MAX_ACTIONS`], malformed actions dropped.
pub fn sanitize_actions(list: &mut Vec<Action>) {
    if list.len() > MAX_ACTIONS || list.iter().any(|a| a.check().is_err()) {
        let old = std::mem::take(list);
        *list = old.into_iter().take(MAX_ACTIONS).filter_map(Action::sanitized).collect();
    }
}

/// A pointer event on a button (or a movie clip with event handlers).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ButtonEvent {
    /// Pressed and released on the instance.
    Click,
    /// The pointer goes down on it.
    Press,
    /// The pointer comes up over it (wherever it went down).
    Release,
    /// The pointer moves onto it.
    Over,
    /// The pointer moves off it.
    Out,
}

impl ButtonEvent {
    pub const ALL: [ButtonEvent; 5] = [ButtonEvent::Click, ButtonEvent::Press, ButtonEvent::Release, ButtonEvent::Over, ButtonEvent::Out];

    pub fn key(self) -> &'static str {
        match self {
            ButtonEvent::Click => "click",
            ButtonEvent::Press => "press",
            ButtonEvent::Release => "release",
            ButtonEvent::Over => "over",
            ButtonEvent::Out => "out",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            ButtonEvent::Click => "Click",
            ButtonEvent::Press => "Press",
            ButtonEvent::Release => "Release",
            ButtonEvent::Over => "Pointer over",
            ButtonEvent::Out => "Pointer out",
        }
    }
    pub fn hint(self) -> &'static str {
        match self {
            ButtonEvent::Click => "Pressed and released on it",
            ButtonEvent::Press => "The pointer goes down on it",
            ButtonEvent::Release => "The pointer comes up over it",
            ButtonEvent::Over => "The pointer moves onto it",
            ButtonEvent::Out => "The pointer moves off it",
        }
    }
    pub fn from_key(s: &str) -> Option<ButtonEvent> {
        ButtonEvent::ALL.into_iter().find(|e| e.key() == s.trim().to_ascii_lowercase())
    }
}

/// What an instance does on one event.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EventHandler {
    pub event: ButtonEvent,
    #[serde(default)]
    pub actions: Vec<Action>,
}

/// Repairs loaded handlers: one per event (the first), at most [`MAX_HANDLERS`], their actions
/// repaired.
pub fn sanitize_handlers(list: &mut Vec<EventHandler>) {
    let mut seen = Vec::new();
    list.retain(|h| {
        let fresh = !seen.contains(&h.event);
        seen.push(h.event);
        fresh
    });
    list.truncate(MAX_HANDLERS);
    for h in list.iter_mut() {
        sanitize_actions(&mut h.actions);
    }
}

/// A plug-in entry point a keyframe runs: a frame-script plug-in's id and one of its entries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginScript {
    pub id: String,
    #[serde(default = "main_entry")]
    pub entry: String,
}

fn main_entry() -> String {
    "main".into()
}

impl PluginScript {
    pub fn check(&self) -> Result<(), String> {
        if !valid_plugin_id(&self.id) {
            return Err(format!("invalid plug-in id {:?}: 1-64 characters from A-Z a-z 0-9 . _ -", self.id));
        }
        if !valid_entry(&self.entry) {
            return Err(format!("invalid entry point {:?}: 1-32 characters from A-Z a-z 0-9 _", self.entry));
        }
        Ok(())
    }
}

/// Longest kept script, in bytes of UTF-8 (1 MiB: far more than any frame script people
/// write by hand, small enough that every edit can copy it).
pub const MAX_SOURCE_BYTES: usize = 1 << 20;
/// Longest linkage or document class name (a dotted, package-qualified name).
pub const MAX_CLASS_CHARS: usize = 256;

/// The language a kept script is written in. The names are said only to tell people what a
/// script is written in (`docs/legal/compliance.md`); the feature itself is "frame scripts".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Language {
    /// ActionScript 3.0, the only language current Animate documents run.
    #[default]
    #[serde(rename = "actionscript3")]
    ActionScript3,
    /// ActionScript 1.0 and 2.0, from older documents.
    #[serde(rename = "actionscript1or2")]
    ActionScript1or2,
    /// Anything else (kept as text).
    #[serde(rename = "other")]
    Other,
}

impl Language {
    pub const ALL: [Language; 3] = [Language::ActionScript3, Language::ActionScript1or2, Language::Other];

    pub fn key(self) -> &'static str {
        match self {
            Language::ActionScript3 => "actionscript3",
            Language::ActionScript1or2 => "actionscript1or2",
            Language::Other => "other",
        }
    }
    /// As the app shows it.
    pub fn label(self) -> &'static str {
        match self {
            Language::ActionScript3 => "ActionScript 3.0",
            Language::ActionScript1or2 => "ActionScript 1.0 & 2.0",
            Language::Other => "Other language",
        }
    }
    pub fn from_key(s: &str) -> Option<Language> {
        let k = s.trim().to_ascii_lowercase();
        Language::ALL.into_iter().find(|l| l.key() == k)
    }
}

/// A script kept verbatim with the document (ADR 0013 tier 1): its text exactly as written,
/// and its language. AniCraft shows and edits it, and runs it when it is in the ActionScript 3.0
/// subset ([`SourceScript::compiled`], tier 2).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceScript {
    #[serde(default)]
    pub language: Language,
    #[serde(default)]
    pub text: String,
    /// The script compiled, made the first time it is asked for. A pure function of the
    /// language and text, so it is never saved and never compared.
    #[serde(skip)]
    compiled: CompileCache,
}

/// What [`SourceScript::compiled`] gives: the script's steps, or why it doesn't run.
pub type CompileResult = Result<as3::Compiled, Vec<as3::Problem>>;

/// A lazily filled slot. Cloning gives an empty one, so a copy whose text is then changed
/// compiles afresh; comparing ignores it.
#[derive(Default)]
struct CompileCache(OnceLock<CompileResult>);

impl Clone for CompileCache {
    fn clone(&self) -> Self {
        CompileCache::default()
    }
}

impl PartialEq for CompileCache {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Eq for CompileCache {}

impl std::fmt::Debug for CompileCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.get().is_some() { "compiled" } else { "not compiled" })
    }
}

impl SourceScript {
    pub fn new(language: Language, text: impl Into<String>) -> SourceScript {
        SourceScript { language, text: text.into(), compiled: CompileCache::default() }
    }

    /// The script compiled to actions (ADR 0013 tier 2), or the problems that keep it from
    /// running. Compiled once per script: the document shares scripts by `Arc`, so undo
    /// snapshots, the render thread and Test Movie all reuse it.
    pub fn compiled(&self) -> &CompileResult {
        self.compiled.0.get_or_init(|| match self.language {
            Language::ActionScript3 => as3::compile(&self.text),
            Language::ActionScript1or2 => Err(vec![as3::Problem::whole("ActionScript 1.0 and 2.0 don't run; AniCraft runs a subset of ActionScript 3.0")]),
            Language::Other => Err(vec![as3::Problem::whole("scripts in other languages don't run; AniCraft runs a subset of ActionScript 3.0")]),
        })
    }

    /// Whether it runs (it compiled).
    pub fn runs(&self) -> bool {
        self.compiled().is_ok()
    }
    pub fn check(&self) -> Result<(), String> {
        if self.text.trim().is_empty() {
            return Err("a script needs some text".into());
        }
        if self.text.len() > MAX_SOURCE_BYTES {
            return Err(format!("a script is at most {} KiB", MAX_SOURCE_BYTES / 1024));
        }
        Ok(())
    }
    /// Lines of text (a trailing newline doesn't start another).
    pub fn lines(&self) -> usize {
        self.text.lines().count()
    }
}

/// What runs when the playhead enters a keyframe: a list of actions, a plug-in's entry point
/// (a frame script), or a script kept verbatim (ADR 0013 tier 1), which runs when it is in the
/// ActionScript 3.0 subset (tier 2). One field keeps "one of these" true by construction.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameScript {
    Actions(Vec<Action>),
    Plugin(PluginScript),
    /// Shared: copying the keyframe, an undo snapshot or the render thread's copy doesn't copy
    /// the text.
    Source(Arc<SourceScript>),
}

impl FrameScript {
    /// Repairs a loaded script; `false` when nothing is left of it. A kept script is never cut
    /// (that would change its code): one too long or empty is dropped whole.
    pub fn sanitize(&mut self) -> bool {
        match self {
            FrameScript::Actions(list) => {
                sanitize_actions(list);
                !list.is_empty()
            }
            FrameScript::Plugin(p) => p.check().is_ok(),
            FrameScript::Source(s) => s.check().is_ok(),
        }
    }
    /// Whether loading would repair it (so loading un-shares only what it must).
    pub fn needs_repair(&self) -> bool {
        match self {
            FrameScript::Actions(a) => a.is_empty() || check_actions(a).is_err(),
            FrameScript::Plugin(p) => p.check().is_err(),
            FrameScript::Source(s) => s.check().is_err(),
        }
    }
    /// The actions, when it is a list.
    pub fn actions(&self) -> &[Action] {
        match self {
            FrameScript::Actions(a) => a,
            FrameScript::Plugin(_) | FrameScript::Source(_) => &[],
        }
    }
    /// The kept script, when it is one.
    pub fn source(&self) -> Option<&SourceScript> {
        match self {
            FrameScript::Source(s) => Some(s),
            FrameScript::Actions(_) | FrameScript::Plugin(_) => None,
        }
    }
    pub fn describe(&self) -> String {
        match self {
            FrameScript::Actions(a) => a.iter().map(Action::describe).collect::<Vec<_>>().join("; "),
            FrameScript::Plugin(p) => format!("Frame script {} · {}", p.id, p.entry),
            FrameScript::Source(s) => {
                let n = s.lines();
                let state = if s.runs() { "runs" } else { "kept, not run" };
                format!("Script in {} ({n} {}), {state}", s.language.label(), if n == 1 { "line" } else { "lines" })
            }
        }
    }
}

/// A class name as scripts write it: identifiers (letters, digits, `_`, `$`; not starting with
/// a digit) joined by dots (`com.example.Hero`), at most [`MAX_CLASS_CHARS`] characters.
pub fn valid_class_name(s: &str) -> bool {
    let ident = |p: &str| {
        let mut c = p.chars();
        c.next().is_some_and(|c| c.is_alphabetic() || c == '_' || c == '$') && c.all(|c| c.is_alphanumeric() || c == '_' || c == '$')
    };
    !s.is_empty() && s.chars().count() <= MAX_CLASS_CHARS && s.split('.').all(ident)
}

/// An instance name actions can address: 1-64 ASCII letters, digits, `_` or `$`, not starting
/// with a digit, and not one of the path words (`this`, `root`, `parent`, `_root`, `_parent`).
pub fn valid_instance_name(s: &str) -> bool {
    let mut b = s.bytes();
    let first_ok = b.next().is_some_and(|c| c.is_ascii_alphabetic() || c == b'_' || c == b'$');
    first_ok && s.len() <= MAX_NAME_CHARS && b.all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'$') && !matches!(s, "this" | "root" | "parent" | "_root" | "_parent")
}

/// A plug-in id: `[A-Za-z0-9._-]{1,64}` (the plug-in manifest's rule).
pub fn valid_plugin_id(id: &str) -> bool {
    (1..=64).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}

/// A frame-script entry point: `[A-Za-z0-9_]{1,32}`.
pub fn valid_entry(e: &str) -> bool {
    (1..=32).contains(&e.len()) && e.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// 1..=`max` characters, no control characters, at least one visible.
fn printable(s: &str, max: usize) -> bool {
    let n = s.chars().count();
    (1..=max).contains(&n) && !s.chars().any(char::is_control) && s.chars().any(|c| !c.is_whitespace())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn actions_read_and_write_as_small_json_objects() {
        let list = vec![
            Action::Stop { target: Target::this() },
            Action::GotoAndPlay { target: Target::new("hero.arm"), frame: FrameRef::Label("run".into()), scene: None },
            Action::GotoAndStop { target: Target::this(), frame: FrameRef::Number(11), scene: Some("Menu".into()) },
            Action::NextScene,
            Action::SetProperty { target: Target::new("door"), property: Property::Alpha, value: 0.5 },
        ];
        let v = serde_json::to_value(&list).unwrap();
        assert_eq!(v[0], json!({"op": "stop"}));
        assert_eq!(v[1], json!({"op": "goto_and_play", "target": "hero.arm", "frame": "run"}));
        assert_eq!(v[2], json!({"op": "goto_and_stop", "frame": 11, "scene": "Menu"}));
        assert_eq!(v[3], json!({"op": "next_scene"}));
        assert_eq!(v[4], json!({"op": "set_property", "target": "door", "property": "alpha", "value": 0.5}));
        let back: Vec<Action> = serde_json::from_value(v).unwrap();
        assert_eq!(back, list);
        assert!(serde_json::from_value::<Action>(json!({"op": "goto_and_stop", "frame": -1})).is_err());
        assert!(serde_json::from_value::<Action>(json!({"op": "explode"})).is_err());
        for (op, _) in KINDS {
            let a = Action::new(op).unwrap();
            assert_eq!(a.op(), op);
            assert!(a.check().is_ok(), "{op}");
        }
    }

    #[test]
    fn targets_are_name_paths() {
        assert_eq!(Target::new("").segments().unwrap(), []);
        assert_eq!(Target::new("this").segments().unwrap(), []);
        assert_eq!(Target::new("root.menu.btn").segments().unwrap(), [Seg::Root, Seg::Child("menu"), Seg::Child("btn")]);
        assert_eq!(Target::new("_parent._parent.x").segments().unwrap(), [Seg::Parent, Seg::Parent, Seg::Child("x")]);
        for bad in ["a..b", "1abc", "a b", "hero.this", "x.root", "é", &"a.".repeat(40)] {
            assert!(Target::new(bad).segments().is_err(), "accepted {bad:?}");
        }
        assert!(valid_instance_name("hero_mc") && valid_instance_name("$x") && !valid_instance_name("root") && !valid_instance_name(""));
        assert!(!valid_instance_name(&"a".repeat(65)));
    }

    #[test]
    fn checks_reject_and_sanitize_repairs() {
        assert!(Action::Trace { message: "x".repeat(MAX_TRACE_CHARS + 1) }.check().is_err());
        assert!(Action::GotoAndStop { target: Target::this(), frame: FrameRef::Label(String::new()), scene: None }.check().is_err());
        assert!(Action::GotoAndStop { target: Target::this(), frame: FrameRef::Number(MAX_FRAME + 1), scene: None }.check().is_err());
        assert!(Action::SetProperty { target: Target::this(), property: Property::X, value: f64::NAN }.check().is_err());
        let mut list = vec![
            Action::Trace { message: "x".repeat(5000) },
            Action::Stop { target: Target::new("bad target") },
            Action::SetProperty { target: Target::this(), property: Property::Alpha, value: 7.0 },
        ];
        sanitize_actions(&mut list);
        assert_eq!(list.len(), 2);
        assert!(matches!(&list[0], Action::Trace { message } if message.chars().count() == MAX_TRACE_CHARS));
        assert!(matches!(list[1], Action::SetProperty { value, .. } if value == 1.0));
        let mut many: Vec<Action> = (0..1000).map(|_| Action::NextScene).collect();
        sanitize_actions(&mut many);
        assert_eq!(many.len(), MAX_ACTIONS);
        let mut hs = vec![EventHandler { event: ButtonEvent::Click, actions: vec![] }, EventHandler { event: ButtonEvent::Click, actions: vec![Action::NextScene] }];
        sanitize_handlers(&mut hs);
        assert_eq!(hs.len(), 1);
    }

    #[test]
    fn frame_scripts_and_descriptions() {
        let s = FrameScript::Plugin(PluginScript { id: "org.example.menu".into(), entry: "main".into() });
        assert_eq!(serde_json::to_value(&s).unwrap(), json!({"plugin": {"id": "org.example.menu", "entry": "main"}}));
        let s: FrameScript = serde_json::from_value(json!({"plugin": {"id": "x"}})).unwrap();
        assert!(matches!(&s, FrameScript::Plugin(p) if p.entry == "main"));
        let mut bad = FrameScript::Plugin(PluginScript { id: "no spaces".into(), entry: "main".into() });
        assert!(!bad.sanitize());
        let a = FrameScript::Actions(vec![Action::Stop { target: Target::this() }, Action::GotoAndPlay { target: Target::new("hero"), frame: FrameRef::Number(4), scene: None }]);
        assert_eq!(a.describe(), "Stop; Go to frame 5 and play (hero)");
    }

    #[test]
    fn kept_scripts_read_and_write_verbatim() {
        let text = "stop();\r\n// “quoted” \t tab\nhero_mc.gotoAndPlay(\"run\");\n";
        let s = FrameScript::Source(Arc::new(SourceScript::new(Language::ActionScript3, text)));
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v, json!({"source": {"language": "actionscript3", "text": text}}));
        let back: FrameScript = serde_json::from_value(v).unwrap();
        assert_eq!(back, s);
        assert_eq!(back.source().unwrap().text, text, "byte for byte");
        assert_eq!(s.describe(), "Script in ActionScript 3.0 (3 lines), runs");
        let other = FrameScript::Source(Arc::new(SourceScript::new(Language::ActionScript1or2, "stop();")));
        assert_eq!(other.describe(), "Script in ActionScript 1.0 & 2.0 (1 line), kept, not run");
        assert!(s.actions().is_empty() && !s.needs_repair());
        // The language defaults to ActionScript 3.0; unknown ones are refused.
        let d: SourceScript = serde_json::from_value(json!({"text": "x"})).unwrap();
        assert_eq!(d.language, Language::ActionScript3);
        assert!(serde_json::from_value::<SourceScript>(json!({"language": "cobol", "text": "x"})).is_err());
        for l in Language::ALL {
            assert_eq!(Language::from_key(l.key()), Some(l));
            assert_eq!(serde_json::to_value(l).unwrap(), json!(l.key()));
        }
        // Empty or oversized scripts are dropped whole, never cut.
        let mut empty = FrameScript::Source(Arc::new(SourceScript::new(Language::Other, " \n ")));
        assert!(empty.needs_repair() && !empty.sanitize());
        let mut huge = FrameScript::Source(Arc::new(SourceScript::new(Language::Other, "x".repeat(MAX_SOURCE_BYTES + 1))));
        assert!(!huge.sanitize());
    }

    #[test]
    fn class_names_are_dotted_identifiers() {
        for ok in ["Hero", "com.example.Hero", "_Main", "$x.Y2", "Größe"] {
            assert!(valid_class_name(ok), "{ok}");
        }
        for bad in ["", "1Hero", "com..Hero", "Hero.", "a b", "a-b", &"a".repeat(MAX_CLASS_CHARS + 1)] {
            assert!(!valid_class_name(bad), "{bad:?}");
        }
    }
}
