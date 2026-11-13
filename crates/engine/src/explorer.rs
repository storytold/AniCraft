//! The Movie Explorer (ADR 0058), as Animate's: the movie as a tree of scenes › layers ›
//! keyframes › what is on them (symbol instances, text, bitmaps, sounds, frame scripts and
//! instance actions), each instance opening into its symbol's own layers, and the Library's
//! symbol definitions. Show filters hide kinds of rows; a text filter keeps the rows whose names
//! hold it, with the rows above them.
//!
//! [`tree`] gives the rows that are showing, in order, each with its depth, so a list can draw
//! only the rows in view. Only open rows' children are made, so the work follows what is shown,
//! and it stops at [`MAX_ROWS`] and [`MAX_DEPTH`]. Every row says where it is as `find.goto`
//! params, which is how a double-click jumps there.

use crate::find::{Scope, Step};
use crate::params::*;
use crate::{CommandSpec, EngineError, Result, Session};
use anicraft_doc::{Document, ElementId, ElementKind, FrameScript, Keyframe, LayerId, LayerKind, Paint, Symbol, SymbolId, SymbolKind, Timeline};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};

pub fn specs() -> Vec<CommandSpec> {
    let c = |id, label, menu, shortcut, params, mutates, run| CommandSpec { id, label, menu, shortcut, params, mutates, run };
    vec![c(
        "explorer.tree",
        "Movie Explorer",
        "",
        None,
        "{scope?: document (default) | scene, show?: {text?, symbols?, scripts?, media?, frames? (layers and keyframes)} (each default true), definitions?: bool (the Library's symbols; default true), expanded?: [row keys] (default: the top rows), all?: bool (everything open), filter?: text (rows whose names hold it, and the rows above them), offset?, limit? (default 1000, at most 10000)} → {rows: [{key, depth, kind, type?, name, extra, frame?, children, open, go: find.goto params}], total, truncated}",
        false,
        tree_cmd as crate::RunFn,
    )]
}

/// Most rows one tree makes.
pub const MAX_ROWS: usize = 200_000;
/// Deepest row (instances inside instances).
pub const MAX_DEPTH: usize = 48;
const DEFAULT_LIMIT: usize = 1_000;
const MAX_LIMIT: usize = 10_000;

/// What a row is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    Scene,
    /// The Library's symbols, under one row.
    Definitions,
    /// A symbol in the Library.
    Symbol(SymbolKind),
    Layer(LayerKind),
    /// A keyframe (its name is its label).
    Frame,
    /// A keyframe's script or actions.
    Script,
    /// A button or movie clip instance's actions for one event.
    Handler,
    Sound,
    Instance(SymbolKind),
    Text,
    Bitmap,
}

impl NodeKind {
    pub fn key(self) -> &'static str {
        match self {
            NodeKind::Scene => "scene",
            NodeKind::Definitions => "definitions",
            NodeKind::Symbol(_) => "symbol",
            NodeKind::Layer(_) => "layer",
            NodeKind::Frame => "frame",
            NodeKind::Script => "script",
            NodeKind::Handler => "handler",
            NodeKind::Sound => "sound",
            NodeKind::Instance(_) => "instance",
            NodeKind::Text => "text",
            NodeKind::Bitmap => "bitmap",
        }
    }
}

/// Which kinds of rows show (Animate's Show buttons).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Show {
    /// Text fields.
    pub text: bool,
    /// Buttons, movie clips and graphics.
    pub symbols: bool,
    /// Frame scripts and instance actions.
    pub scripts: bool,
    /// Sounds and bitmaps.
    pub media: bool,
    /// Layers and keyframes; off, what is on them is listed straight under the scene or symbol.
    pub frames: bool,
}

impl Default for Show {
    fn default() -> Self {
        Show { text: true, symbols: true, scripts: true, media: true, frames: true }
    }
}

/// What the tree shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Options {
    pub scope: Scope,
    pub show: Show,
    pub definitions: bool,
    /// Open rows, by key. `None`: only the top rows.
    pub expanded: Option<HashSet<String>>,
    /// Every row open.
    pub all: bool,
    pub filter: String,
}

impl Default for Options {
    fn default() -> Self {
        Options { scope: Scope::Document, show: Show::default(), definitions: true, expanded: None, all: false, filter: String::new() }
    }
}

/// Where a row is, as `find.goto` params.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Go {
    pub scene: Option<usize>,
    /// The symbol opened alone (from the Library), or to open.
    pub symbol: Option<SymbolId>,
    /// Instances opened in place from the scene or the symbol.
    pub path: Vec<Step>,
    pub layer: Option<LayerId>,
    pub frame: Option<u32>,
    pub element: Option<ElementId>,
}

impl Go {
    pub fn to_json(&self) -> Value {
        let mut v = json!({});
        if let Some(s) = self.scene {
            v["scene"] = json!(s);
        }
        if let Some(s) = self.symbol {
            v["symbol"] = json!(s.0);
        }
        if !self.path.is_empty() {
            v["path"] = Value::Array(self.path.iter().map(Step::to_json).collect());
        }
        if let Some(l) = self.layer {
            v["layer"] = json!(l.0);
        }
        if let Some(f) = self.frame {
            v["frame"] = json!(f);
        }
        if let Some(e) = self.element {
            v["element"] = json!(e.0);
        }
        v
    }
}

/// One row of the tree.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    /// Stable while the document keeps its ids: what `expanded` names.
    pub key: String,
    pub depth: usize,
    pub kind: NodeKind,
    /// Document text: a scene, layer, symbol or sound name, a frame label, a text field's first
    /// line, an instance's symbol, a script's summary.
    pub name: String,
    /// More document text: an instance's name, a text field's font and size.
    pub extra: String,
    /// The keyframe's first frame (keyframes and what is on them).
    pub frame: Option<u32>,
    pub children: bool,
    pub open: bool,
    pub go: Go,
}

impl Row {
    pub fn to_json(&self) -> Value {
        let mut v = json!({"key": self.key, "depth": self.depth, "kind": self.kind.key(), "name": self.name, "extra": self.extra, "frame": self.frame, "children": self.children, "open": self.open, "go": self.go.to_json()});
        match self.kind {
            NodeKind::Symbol(k) | NodeKind::Instance(k) => v["type"] = json!(crate::cmds_library::kind_key(k)),
            NodeKind::Layer(k) => v["type"] = json!(format!("{k:?}").to_lowercase()),
            _ => {}
        }
        v
    }
}

/// The tree's rows and whether it stopped at [`MAX_ROWS`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tree {
    pub rows: Vec<Row>,
    pub truncated: bool,
}

struct Walk<'a> {
    doc: &'a Document,
    syms: HashMap<SymbolId, &'a Symbol>,
    o: &'a Options,
    /// Everything is open (all, or a filter is looking through it all).
    every: bool,
    rows: Vec<Row>,
    truncated: bool,
}

/// The first line of `t`, at most 80 characters.
fn line(t: &str) -> String {
    let first = t.lines().next().unwrap_or("").trim();
    let mut out: String = first.chars().take(80).collect();
    if out.len() < first.len() || t.trim().lines().nth(1).is_some() {
        out.push('…');
    }
    out
}

impl<'a> Walk<'a> {
    fn open(&self, key: &str, depth: usize) -> bool {
        self.every || self.o.expanded.as_ref().map_or(depth == 0, |e| e.contains(key))
    }

    /// Adds a row; false once the tree is full.
    fn push(&mut self, row: Row) -> bool {
        if self.rows.len() >= MAX_ROWS {
            self.truncated = true;
            return false;
        }
        self.rows.push(row);
        true
    }

    fn full(&self) -> bool {
        self.truncated
    }

    /// Whether a keyframe shows anything (with the frames shown, a labelled keyframe does too).
    fn frame_items(&self, k: &Keyframe) -> bool {
        let s = self.o.show;
        (s.scripts && k.script.is_some())
            || (s.media && k.sound.is_some())
            || anicraft_doc::expand_groups(&k.elements).iter().any(|e| match &e.kind {
                ElementKind::Symbol { .. } => s.symbols,
                ElementKind::Text(_) => s.text,
                ElementKind::Shape(sh) => s.media && matches!(sh.fill, Some(Paint::Bitmap { .. })),
                ElementKind::Camera | ElementKind::Group { .. } => false,
            })
            || (s.frames && !k.label.is_empty())
    }

    fn has_items(&self, tl: &Timeline) -> bool {
        tl.layers.iter().any(|l| self.o.show.frames || l.keyframes.iter().any(|k| self.frame_items(k)))
    }

    /// The rows of timeline `tl` under `key`, at `depth`. `go` is where the timeline is (scene or
    /// symbol, and the instances opened to reach it); `stack` the symbols it is inside.
    fn timeline(&mut self, tl: &Timeline, key: &str, depth: usize, go: &Go, stack: &[SymbolId]) {
        if depth > MAX_DEPTH {
            return;
        }
        let mut skip_to = 0usize;
        for (i, l) in tl.layers.iter().enumerate() {
            if self.full() {
                return;
            }
            if i < skip_to {
                continue;
            }
            let lkey = format!("{key}/l{}", l.id.0);
            let lgo = Go { layer: Some(l.id), ..go.clone() };
            if self.o.show.frames {
                let d = depth + tl.depth(i).min(MAX_DEPTH);
                let folder = l.kind == LayerKind::Folder;
                let children = if folder { tl.group_range(i).len() > 1 } else { l.keyframes.iter().any(|k| self.frame_items(k)) };
                let open = children && self.open(&lkey, d);
                if !self.push(Row { key: lkey.clone(), depth: d, kind: NodeKind::Layer(l.kind), name: l.name.clone(), extra: String::new(), frame: None, children, open, go: lgo.clone() }) {
                    return;
                }
                if folder {
                    if !open {
                        skip_to = tl.group_range(i).end;
                    }
                    continue;
                }
                if !open {
                    continue;
                }
                for k in &l.keyframes {
                    if !self.frame_items(k) {
                        continue;
                    }
                    let kkey = format!("{lkey}/k{}", k.start);
                    let kgo = Go { frame: Some(k.start), ..lgo.clone() };
                    let children = self.keyframe_has(k);
                    let open = children && self.open(&kkey, d + 1);
                    if !self.push(Row { key: kkey.clone(), depth: d + 1, kind: NodeKind::Frame, name: k.label.clone(), extra: String::new(), frame: Some(k.start), children, open, go: kgo.clone() }) {
                        return;
                    }
                    if open {
                        self.keyframe(k, &kkey, d + 2, &kgo, stack);
                    }
                }
            } else {
                for k in &l.keyframes {
                    let kkey = format!("{lkey}/k{}", k.start);
                    let kgo = Go { frame: Some(k.start), ..lgo.clone() };
                    self.keyframe(k, &kkey, depth, &kgo, stack);
                }
            }
        }
    }

    /// Whether a keyframe row has children (its label alone makes the row, not children).
    fn keyframe_has(&self, k: &Keyframe) -> bool {
        let s = self.o.show;
        (s.scripts && k.script.is_some()) || (s.media && k.sound.is_some()) || anicraft_doc::expand_groups(&k.elements).iter().any(|e| self.element_shows(e))
    }

    fn element_shows(&self, e: &anicraft_doc::Element) -> bool {
        let s = self.o.show;
        match &e.kind {
            ElementKind::Symbol { .. } => s.symbols,
            ElementKind::Text(_) => s.text,
            ElementKind::Shape(sh) => s.media && matches!(sh.fill, Some(Paint::Bitmap { .. })),
            ElementKind::Camera | ElementKind::Group { .. } => false,
        }
    }

    /// What is on keyframe `k`: its script, its sound, then its elements.
    fn keyframe(&mut self, k: &Keyframe, key: &str, depth: usize, go: &Go, stack: &[SymbolId]) {
        let s = self.o.show;
        let leaf = |key: String, kind: NodeKind, name: String, extra: String, go: Go| Row { key, depth, kind, name, extra, frame: Some(k.start), children: false, open: false, go };
        if s.scripts
            && let Some(script) = &k.script
        {
            let name = match script {
                FrameScript::Source(src) => line(&src.text),
                other => line(&other.describe()),
            };
            if !self.push(leaf(format!("{key}/x"), NodeKind::Script, name, String::new(), go.clone())) {
                return;
            }
        }
        if s.media
            && let Some(fs) = &k.sound
        {
            let name = self.doc.sound(fs.sound).map(|x| x.name.clone()).unwrap_or_default();
            if !self.push(leaf(format!("{key}/a"), NodeKind::Sound, name, fs.sync.key().to_string(), go.clone())) {
                return;
            }
        }
        // What is inside a group is listed as on its keyframe (ADR 0073).
        for e in anicraft_doc::expand_groups(&k.elements).iter() {
            if self.full() || !self.element_shows(e) {
                continue;
            }
            let ekey = format!("{key}/e{}", e.id.0);
            let ego = Go { element: Some(e.id), ..go.clone() };
            match &e.kind {
                ElementKind::Symbol { symbol, events, .. } => {
                    let sym = self.syms.get(symbol).copied();
                    let nested = sym.filter(|_| !stack.contains(symbol) && stack.len() < MAX_DEPTH);
                    let children = (s.scripts && !events.is_empty()) || nested.is_some_and(|sym| self.has_items(&sym.timeline));
                    let open = children && self.open(&ekey, depth);
                    let row = Row {
                        key: ekey.clone(),
                        depth,
                        kind: NodeKind::Instance(sym.map_or(SymbolKind::Graphic, |x| x.kind)),
                        name: sym.map_or_else(String::new, |x| x.name.clone()),
                        extra: e.name.clone(),
                        frame: Some(k.start),
                        children,
                        open,
                        go: ego.clone(),
                    };
                    if !self.push(row) || !open {
                        continue;
                    }
                    if s.scripts {
                        for (n, h) in events.iter().enumerate() {
                            let desc = h.actions.iter().map(anicraft_doc::Action::describe).collect::<Vec<_>>().join("; ");
                            if !self.push(Row { key: format!("{ekey}/h{n}"), depth: depth + 1, kind: NodeKind::Handler, name: h.event.key().to_string(), extra: line(&desc), frame: Some(k.start), children: false, open: false, go: ego.clone() }) {
                                return;
                            }
                        }
                    }
                    if let Some(sym) = nested {
                        // Inside: the symbol's own timeline, reached by opening this instance.
                        let (Some(layer), Some(frame)) = (go.layer, go.frame) else { continue };
                        let mut inner = Go { scene: go.scene, symbol: go.symbol, path: go.path.clone(), layer: None, frame: None, element: None };
                        inner.path.push(Step { layer, frame, element: e.id });
                        let mut st = stack.to_vec();
                        st.push(*symbol);
                        self.timeline(&sym.timeline, &ekey, depth + 1, &inner, &st);
                    }
                }
                ElementKind::Text(t) => {
                    if !self.push(leaf(ekey, NodeKind::Text, line(&t.text), format!("{} {}", t.font, t.size), ego)) {
                        return;
                    }
                }
                ElementKind::Shape(sh) => {
                    let name = match &sh.fill {
                        Some(Paint::Bitmap { bitmap, .. }) => self.doc.bitmap(*bitmap).map(|b| b.name.clone()).unwrap_or_default(),
                        _ => String::new(),
                    };
                    if !self.push(leaf(ekey, NodeKind::Bitmap, name, e.name.clone(), ego)) {
                        return;
                    }
                }
                ElementKind::Camera | ElementKind::Group { .. } => {}
            }
        }
    }
}

/// The rows showing for `o`, from scene `scene` (the scope's scene).
pub fn tree(doc: &Document, scene: usize, o: &Options) -> Tree {
    let filter = o.filter.trim().to_lowercase();
    let mut w = Walk { doc, syms: doc.library.iter().map(|s| (s.id, &**s)).collect(), o, every: o.all || !filter.is_empty(), rows: Vec::new(), truncated: false };
    let scenes: Vec<usize> = match o.scope {
        Scope::Document => (0..doc.scenes.len()).collect(),
        Scope::Scene => vec![scene.min(doc.scenes.len().saturating_sub(1))],
    };
    for i in scenes {
        let Some(sc) = doc.scenes.get(i) else { continue };
        let key = format!("s{i}");
        let children = w.has_items(&sc.timeline);
        let open = children && w.open(&key, 0);
        let go = Go { scene: Some(i), ..Go::default() };
        if !w.push(Row { key: key.clone(), depth: 0, kind: NodeKind::Scene, name: sc.name.clone(), extra: String::new(), frame: None, children, open, go: go.clone() }) {
            break;
        }
        if open {
            w.timeline(&sc.timeline, &key, 1, &go, &[]);
        }
    }
    if o.definitions && !doc.library.is_empty() && !w.full() {
        let open = w.open("d", 0);
        w.push(Row { key: "d".into(), depth: 0, kind: NodeKind::Definitions, name: String::new(), extra: String::new(), frame: None, children: true, open, go: Go::default() });
        if open {
            for sym in &doc.library {
                let key = format!("d/y{}", sym.id.0);
                let children = w.has_items(&sym.timeline);
                let open = children && w.open(&key, 1);
                let go = Go { symbol: Some(sym.id), ..Go::default() };
                if !w.push(Row { key: key.clone(), depth: 1, kind: NodeKind::Symbol(sym.kind), name: sym.name.clone(), extra: String::new(), frame: None, children, open, go: go.clone() }) {
                    break;
                }
                if open {
                    w.timeline(&sym.timeline, &key, 2, &go, &[sym.id]);
                }
            }
        }
    }
    let Walk { mut rows, truncated, .. } = w;
    if !filter.is_empty() {
        rows = filtered(rows, &filter);
    }
    Tree { rows, truncated }
}

/// The rows whose name or extra text holds `needle` (lower case), with every row above them.
fn filtered(rows: Vec<Row>, needle: &str) -> Vec<Row> {
    let hit = |r: &Row| r.name.to_lowercase().contains(needle) || r.extra.to_lowercase().contains(needle);
    let mut keep = vec![false; rows.len()];
    // The nearest row above at each depth.
    let mut above: Vec<usize> = Vec::new();
    for (i, r) in rows.iter().enumerate() {
        above.truncate(r.depth);
        if hit(r) {
            if let Some(k) = keep.get_mut(i) {
                *k = true;
            }
            for &a in &above {
                if let Some(k) = keep.get_mut(a) {
                    *k = true;
                }
            }
        }
        // (Depths can skip, inside folders: keep the stack's indices in order.)
        while above.len() < r.depth {
            above.push(above.last().copied().unwrap_or(i));
        }
        above.push(i);
    }
    rows.into_iter().zip(keep).filter_map(|(r, k)| k.then_some(r)).collect()
}

fn opt_flag(v: &Value, key: &str) -> Result<bool> {
    Ok(opt_bool(v, key)?.unwrap_or(true))
}

/// The options in `p` (`explorer.tree`'s params).
pub fn options_of(p: &Value) -> Result<Options> {
    let scope = match opt_str(p, "scope")? {
        None | Some("document") => Scope::Document,
        Some("scene") => Scope::Scene,
        Some(o) => return Err(EngineError::Param(format!("'scope' is document or scene, not '{o}'"))),
    };
    let show = match p.get("show") {
        None | Some(Value::Null) => Show::default(),
        Some(v) if v.is_object() => Show { text: opt_flag(v, "text")?, symbols: opt_flag(v, "symbols")?, scripts: opt_flag(v, "scripts")?, media: opt_flag(v, "media")?, frames: opt_flag(v, "frames")? },
        Some(_) => return Err(EngineError::Param("'show' must be {text?, symbols?, scripts?, media?, frames?}".into())),
    };
    let expanded = match p.get("expanded") {
        None | Some(Value::Null) => None,
        Some(Value::Array(list)) => {
            if list.len() > MAX_ROWS {
                return Err(EngineError::Param(format!("'expanded' names at most {MAX_ROWS} rows")));
            }
            Some(list.iter().map(|v| v.as_str().map(str::to_string).ok_or_else(|| EngineError::Param("'expanded' must be a list of row keys".into()))).collect::<Result<HashSet<_>>>()?)
        }
        Some(_) => return Err(EngineError::Param("'expanded' must be a list of row keys".into())),
    };
    let filter: String = opt_str(p, "filter")?.unwrap_or("").chars().take(256).collect();
    Ok(Options { scope, show, definitions: opt_bool(p, "definitions")?.unwrap_or(true), expanded, all: opt_bool(p, "all")?.unwrap_or(false), filter })
}

fn tree_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let o = options_of(p)?;
    let offset = opt_u64(p, "offset")?.map_or(0, |n| usize::try_from(n).unwrap_or(usize::MAX));
    let limit = opt_u64(p, "limit")?.map_or(DEFAULT_LIMIT, |n| usize::try_from(n).unwrap_or(MAX_LIMIT)).min(MAX_LIMIT);
    let t = tree(&s.doc, s.scene, &o);
    let rows: Vec<Value> = t.rows.iter().skip(offset).take(limit).map(Row::to_json).collect();
    Ok(json!({"rows": rows, "total": t.rows.len(), "offset": offset, "truncated": t.truncated}))
}

#[cfg(test)]
#[path = "explorer_tests.rs"]
mod tests;
