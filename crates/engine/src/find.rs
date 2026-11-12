//! Find and Replace (ADR 0058), as Animate's Edit › Find and Replace: text (in text fields,
//! frame labels, layer names, instance names and frame scripts), fonts, colours, symbols, sounds
//! and bitmaps, in the current scene (and the symbols it uses) or the whole document.
//!
//! `find.search` lists located hits; `find.replace` replaces one hit or all of them as one undo
//! step (asking first above [`REPLACE_ASK`] places, ADR 0004); `find.goto` jumps to a place: the
//! scene or symbol (opened in place through the instances that lead to it, ADR 0045), the layer,
//! the keyframe and the element, selected. The Movie Explorer (`explorer.tree`) navigates through
//! `find.goto` too.
//!
//! Text is matched with the `regex` crate, whose engine runs in time linear in the text and
//! refuses patterns that would compile too large, so no pattern can hang the editor. Plain text,
//! whole words and case are the same engine with an escaped pattern.

use crate::params::*;
use crate::{CommandSpec, EngineError, Result, Selection, Session};
use anicraft_doc::{BitmapId, Document, ElementId, ElementKind, FrameScript, LayerId, Paint, Rgba, SoundId, SourceScript, Symbol, SymbolId, Timeline};
use regex::{Regex, RegexBuilder};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

pub fn specs() -> Vec<CommandSpec> {
    let c = |id, label, menu, shortcut, params, mutates, run| CommandSpec { id, label, menu, shortcut, params, mutates, run };
    vec![
        c(
            "find.search",
            "Find and Replace",
            "Edit",
            Some("Ctrl+F"),
            "{kind: text | font | color | symbol | sound | bitmap, text? (kind text: what to find), match_case?, whole_word?, regex? (a pattern; linear-time engine, size-limited), font? (family), color? (#rrggbb; alpha is ignored), symbol? | sound? | bitmap?: library id, in?: [text, labels, layers, instances, scripts] (kind text) | [fills, strokes, text] (kind color) (default: all), scope?: scene (the current scene and the symbols it uses; default) | document, offset?, limit? (default 500, at most 10000)} → {hits: [{kind, field, scene | symbol, layer, layer_name, frame (keyframe start; null on a layer name), element?, start, end (characters, text only), found, context}], total, truncated}",
            false,
            search_cmd as crate::RunFn,
        ),
        c(
            "find.replace",
            "Replace",
            "",
            None,
            "{the find.search query, with: text (kind text; $1 and ${name} insert groups when regex) | family (font) | #rrggbb (color; each place keeps its alpha) | library id (symbol, sound, bitmap), at?: one hit as find.search returned it (Replace); without `at`, every hit (Replace All, one undo step; asks first above 100 places)} → {replaced, skipped, notes}",
            true,
            replace_cmd,
        ),
        c(
            "find.goto",
            "Go to Found Item",
            "",
            None,
            "{layer?: id (its scene or symbol is found), scene?: index, symbol?: id, path?: [{layer, frame, element}] (instances to open in place from `scene`, or from `symbol` opened alone), frame?, element?} goes there: closes open symbols, switches scene or opens the symbol (in place through an instance that uses it when there is one), picks the layer and frame and selects the element",
            false,
            goto_cmd,
        ),
    ]
}

/// Longest text to find (characters).
pub const MAX_QUERY_CHARS: usize = 1_000;
/// Most hits one search keeps.
pub const MAX_HITS: usize = 100_000;
/// Replace All asks first when it would change more places than this (ADR 0004).
pub const REPLACE_ASK: usize = 100;
/// Most a compiled pattern may take (bytes), and its lazy DFA.
const REGEX_LIMIT: usize = 1 << 20;
const DEFAULT_LIMIT: usize = 500;
const MAX_LIMIT: usize = 10_000;
/// Characters of context shown on each side of a text match.
const CONTEXT: usize = 32;
/// Most instances opened in place to reach a symbol (`editing`'s limit).
const MAX_ROUTE: usize = 32;

/// What is searched for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Text,
    Font,
    Color,
    Symbol,
    Sound,
    Bitmap,
}

impl Kind {
    pub const ALL: [Kind; 6] = [Kind::Text, Kind::Font, Kind::Color, Kind::Symbol, Kind::Sound, Kind::Bitmap];

    pub fn key(self) -> &'static str {
        match self {
            Kind::Text => "text",
            Kind::Font => "font",
            Kind::Color => "color",
            Kind::Symbol => "symbol",
            Kind::Sound => "sound",
            Kind::Bitmap => "bitmap",
        }
    }

    pub fn from_key(s: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.key() == s || (s == "colour" && *k == Kind::Color))
    }
}

/// Where a search looks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Scope {
    /// The current scene, and every symbol it uses (however deep).
    #[default]
    Scene,
    /// Every scene and every symbol in the Library, used or not.
    Document,
}

/// A timeline of the document: a scene's, or a Library symbol's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Where {
    Scene(usize),
    Symbol(SymbolId),
}

/// The part of the document a hit is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Field {
    /// A text field's text.
    Text,
    /// A keyframe's label.
    Label,
    LayerName,
    /// An element's instance name.
    InstanceName,
    /// A keyframe's script source.
    Script,
    /// A text field's font.
    Font,
    /// A shape's fill (solid or a gradient stop), stroke, or a text field's colour.
    Fill,
    Stroke,
    TextColor,
    /// A symbol instance.
    Instance,
    /// A keyframe's sound.
    Sound,
    /// A shape filled with a bitmap (a placed bitmap is one).
    Bitmap,
}

impl Field {
    pub const ALL: [Field; 12] = [Field::Text, Field::Label, Field::LayerName, Field::InstanceName, Field::Script, Field::Font, Field::Fill, Field::Stroke, Field::TextColor, Field::Instance, Field::Sound, Field::Bitmap];

    pub fn key(self) -> &'static str {
        match self {
            Field::Text => "text",
            Field::Label => "label",
            Field::LayerName => "layer_name",
            Field::InstanceName => "instance_name",
            Field::Script => "script",
            Field::Font => "font",
            Field::Fill => "fill",
            Field::Stroke => "stroke",
            Field::TextColor => "text_color",
            Field::Instance => "instance",
            Field::Sound => "sound",
            Field::Bitmap => "bitmap",
        }
    }

    pub fn from_key(s: &str) -> Option<Field> {
        Field::ALL.into_iter().find(|f| f.key() == s)
    }
}

/// Which places a text or colour search looks in (the others ignore it).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Within {
    pub text: bool,
    pub labels: bool,
    pub layers: bool,
    pub instances: bool,
    pub scripts: bool,
    pub fills: bool,
    pub strokes: bool,
    pub text_color: bool,
}

impl Default for Within {
    fn default() -> Self {
        Within { text: true, labels: true, layers: true, instances: true, scripts: true, fills: true, strokes: true, text_color: true }
    }
}

/// What a query matches.
#[derive(Clone, Debug)]
pub enum Needle {
    Text { re: Regex, expand: bool },
    /// A family name, lower-cased and trimmed.
    Font(String),
    Color([u8; 3]),
    Symbol(SymbolId),
    Sound(SoundId),
    Bitmap(BitmapId),
}

/// A checked search.
#[derive(Clone, Debug)]
pub struct Query {
    pub needle: Needle,
    pub within: Within,
    pub scope: Scope,
}

/// The text options of a text search, as the Find and Replace window holds them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct TextOptions {
    pub match_case: bool,
    pub whole_word: bool,
    pub regex: bool,
}

/// Compiles `text` as a bounded pattern.
pub fn text_needle(text: &str, o: TextOptions) -> Result<Needle> {
    if text.is_empty() {
        return Err(EngineError::Param("type something to find".into()));
    }
    if text.chars().count() > MAX_QUERY_CHARS {
        return Err(EngineError::Param(format!("what to find is at most {MAX_QUERY_CHARS} characters")));
    }
    let body = if o.regex { text.to_string() } else { regex::escape(text) };
    let pattern = if o.whole_word { format!(r"\b(?:{body})\b") } else { body };
    let re = RegexBuilder::new(&pattern)
        .case_insensitive(!o.match_case)
        .size_limit(REGEX_LIMIT)
        .dfa_size_limit(REGEX_LIMIT)
        .nest_limit(64)
        .build()
        .map_err(|e| EngineError::Param(match e {
            regex::Error::CompiledTooBig(_) => "that pattern is too large; make it simpler".to_string(),
            regex::Error::Syntax(s) => format!("that isn't a pattern: {}", s.lines().last().unwrap_or("").trim()),
            _ => "that isn't a pattern".to_string(),
        }))?;
    Ok(Needle::Text { re, expand: o.regex })
}

fn rgb8(c: Rgba) -> [u8; 3] {
    let c = c.sanitized();
    let b = |v: f32| (v * 255.0).round() as u8;
    [b(c.r), b(c.g), b(c.b)]
}

fn font_key(f: &str) -> String {
    f.trim().to_lowercase()
}

fn opt_id(p: &Value, key: &str) -> Result<u64> {
    opt_u64(p, key)?.ok_or_else(|| EngineError::Param(format!("param '{key}' is required: a Library id")))
}

impl Query {
    /// The query in `p` (`find.search`'s params), checked against the document.
    pub fn from_params(s: &Session, p: &Value) -> Result<Query> {
        let kind = str_req(p, "kind")?;
        let kind = Kind::from_key(kind).ok_or_else(|| EngineError::Param(format!("unknown kind '{kind}' (text, font, color, symbol, sound or bitmap)")))?;
        let needle = match kind {
            Kind::Text => {
                let o = TextOptions { match_case: opt_bool(p, "match_case")?.unwrap_or(false), whole_word: opt_bool(p, "whole_word")?.unwrap_or(false), regex: opt_bool(p, "regex")?.unwrap_or(false) };
                text_needle(str_req(p, "text")?, o)?
            }
            Kind::Font => {
                let f = font_key(str_req(p, "font")?);
                if f.is_empty() || f.chars().count() > 256 {
                    return Err(EngineError::Param("'font' must be a family name".into()));
                }
                Needle::Font(f)
            }
            Kind::Color => Needle::Color(rgb8(opt_color(p, "color")?.ok_or_else(|| EngineError::Param("param 'color' is required: #rrggbb".into()))?)),
            Kind::Symbol => {
                let id = SymbolId(opt_id(p, "symbol")?);
                s.doc.symbol(id).ok_or_else(|| EngineError::State(format!("no symbol with id {}", id.0)))?;
                Needle::Symbol(id)
            }
            Kind::Sound => {
                let id = SoundId(opt_id(p, "sound")?);
                s.doc.sound(id).ok_or_else(|| EngineError::State(format!("no sound with id {}", id.0)))?;
                Needle::Sound(id)
            }
            Kind::Bitmap => {
                let id = BitmapId(opt_id(p, "bitmap")?);
                s.doc.bitmap(id).ok_or_else(|| EngineError::State(format!("no bitmap with id {}", id.0)))?;
                Needle::Bitmap(id)
            }
        };
        let within = match p.get("in") {
            None | Some(Value::Null) => Within::default(),
            Some(Value::Array(list)) => {
                let mut w = Within { text: false, labels: false, layers: false, instances: false, scripts: false, fills: false, strokes: false, text_color: false };
                for v in list.iter().take(16) {
                    match (kind, v.as_str()) {
                        (Kind::Text, Some("text")) => w.text = true,
                        (Kind::Text, Some("labels")) => w.labels = true,
                        (Kind::Text, Some("layers")) => w.layers = true,
                        (Kind::Text, Some("instances")) => w.instances = true,
                        (Kind::Text, Some("scripts")) => w.scripts = true,
                        (Kind::Color, Some("fills")) => w.fills = true,
                        (Kind::Color, Some("strokes")) => w.strokes = true,
                        (Kind::Color, Some("text")) => w.text_color = true,
                        (_, other) => return Err(EngineError::Param(format!("'in' can't hold {}", other.map_or_else(|| v.to_string(), |o| format!("'{o}'"))))),
                    }
                }
                w
            }
            Some(_) => return Err(EngineError::Param("'in' must be a list".into())),
        };
        let scope = match opt_str(p, "scope")? {
            None | Some("scene") => Scope::Scene,
            Some("document") => Scope::Document,
            Some(o) => return Err(EngineError::Param(format!("'scope' is scene or document, not '{o}'"))),
        };
        Ok(Query { needle, within, scope })
    }

    pub fn kind(&self) -> Kind {
        match self.needle {
            Needle::Text { .. } => Kind::Text,
            Needle::Font(_) => Kind::Font,
            Needle::Color(_) => Kind::Color,
            Needle::Symbol(_) => Kind::Symbol,
            Needle::Sound(_) => Kind::Sound,
            Needle::Bitmap(_) => Kind::Bitmap,
        }
    }
}

/// One place a search found.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub at: Where,
    pub layer: LayerId,
    /// The keyframe's first frame; `None` for a layer's name.
    pub frame: Option<u32>,
    pub element: Option<ElementId>,
    pub field: Field,
    /// A text match's characters in the field (`start..end`); `0..0` otherwise.
    pub start: usize,
    pub end: usize,
    /// What was found: the matched text, the font, the colour (#rrggbb), or the item's name.
    pub found: String,
    /// The text around a match, on one line, for lists (empty for other kinds).
    pub context: String,
}

impl Hit {
    pub fn to_json(&self, doc: &Document, kind: Kind) -> Value {
        let mut v = json!({
            "kind": kind.key(), "field": self.field.key(), "layer": self.layer.0, "frame": self.frame,
            "element": self.element.map(|e| e.0), "start": self.start, "end": self.end, "found": self.found, "context": self.context,
        });
        match self.at {
            Where::Scene(i) => {
                v["scene"] = json!(i);
                v["scene_name"] = json!(doc.scenes.get(i).map(|s| s.name.as_str()));
            }
            Where::Symbol(id) => {
                v["symbol"] = json!(id.0);
                v["symbol_name"] = json!(doc.symbol(id).map(|s| s.name.as_str()));
            }
        }
        v["layer_name"] = json!(timeline_of(doc, self.at).and_then(|t| t.layer(self.layer)).map(|l| l.name.as_str()));
        v
    }

    /// The hit `v` names (as [`Hit::to_json`] wrote it), without what was found.
    fn target(v: &Value) -> Result<Target> {
        let bad = || EngineError::Param("'at' must be a hit as find.search returns it".into());
        if !v.is_object() {
            return Err(bad());
        }
        let at = match (opt_u64(v, "scene")?, opt_u64(v, "symbol")?) {
            (Some(i), None) => Where::Scene(usize::try_from(i).map_err(|_| bad())?),
            (None, Some(id)) => Where::Symbol(SymbolId(id)),
            _ => return Err(bad()),
        };
        let field = opt_str(v, "field")?.and_then(Field::from_key).ok_or_else(bad)?;
        let layer = LayerId(opt_u64(v, "layer")?.ok_or_else(bad)?);
        Ok(Target { at, layer, frame: opt_u32(v, "frame")?, element: opt_u64(v, "element")?.map(ElementId), field, start: opt_u64(v, "start")?.and_then(|n| usize::try_from(n).ok()).unwrap_or(0) })
    }
}

/// A hit's place (what `find.replace`'s `at` names).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Target {
    at: Where,
    layer: LayerId,
    frame: Option<u32>,
    element: Option<ElementId>,
    field: Field,
    start: usize,
}

impl Target {
    fn is(&self, h: &Hit) -> bool {
        self.at == h.at && self.layer == h.layer && self.frame == h.frame && self.element == h.element && self.field == h.field && (!matches!(h.field, Field::Text | Field::Label | Field::LayerName | Field::InstanceName | Field::Script) || self.start == h.start)
    }
}

/// The timeline `at` names.
pub fn timeline_of(doc: &Document, at: Where) -> Option<&Timeline> {
    match at {
        Where::Scene(i) => doc.scenes.get(i).map(|s| &s.timeline),
        Where::Symbol(id) => doc.symbol(id).map(|s| &s.timeline),
    }
}

fn timeline_of_mut(doc: &mut Document, at: Where) -> Option<&mut Timeline> {
    match at {
        Where::Scene(i) => doc.scenes.get_mut(i).map(|s| &mut Arc::make_mut(s).timeline),
        Where::Symbol(id) => doc.library.iter_mut().find(|s| s.id == id).map(|s| &mut Arc::make_mut(s).timeline),
    }
}

/// The symbols an instance on `tl` shows (directly).
fn used_symbols(tl: &Timeline, mut f: impl FnMut(SymbolId)) {
    for k in tl.layers.iter().flat_map(|l| l.keyframes.iter()) {
        for e in &k.elements {
            if let ElementKind::Symbol { symbol, .. } = e.kind {
                f(symbol);
            }
        }
    }
}

/// Every symbol reachable from `roots` through instances (the roots included), however deep.
pub fn reachable(doc: &Document, roots: impl IntoIterator<Item = SymbolId>) -> HashSet<SymbolId> {
    let by_id: HashMap<SymbolId, &Symbol> = doc.library.iter().map(|s| (s.id, &**s)).collect();
    let mut seen = HashSet::new();
    let mut todo: Vec<SymbolId> = roots.into_iter().collect();
    while let Some(id) = todo.pop() {
        if !seen.insert(id) {
            continue;
        }
        if let Some(sym) = by_id.get(&id) {
            used_symbols(&sym.timeline, |u| {
                if !seen.contains(&u) {
                    todo.push(u);
                }
            });
        }
    }
    seen
}

/// The timelines a scope covers, in order: scenes first, then symbols in Library order.
pub fn timelines(doc: &Document, scene: usize, scope: Scope) -> Vec<Where> {
    match scope {
        Scope::Document => (0..doc.scenes.len()).map(Where::Scene).chain(doc.library.iter().map(|s| Where::Symbol(s.id))).collect(),
        Scope::Scene => {
            let mut used = Vec::new();
            used_symbols(doc.timeline(scene), |s| used.push(s));
            let inside = reachable(doc, used);
            std::iter::once(Where::Scene(scene.min(doc.scenes.len().saturating_sub(1)))).chain(doc.library.iter().filter(|s| inside.contains(&s.id)).map(|s| Where::Symbol(s.id))).collect()
        }
    }
}

/// Non-empty matches of `re` in `text`: byte range and character start/end.
fn text_matches<'a>(re: &'a Regex, text: &'a str) -> impl Iterator<Item = (usize, usize, usize, usize)> + 'a {
    let (mut byte, mut ch) = (0usize, 0usize);
    re.find_iter(text).filter(|m| !m.is_empty()).map(move |m| {
        ch += text.get(byte..m.start()).map_or(0, |t| t.chars().count());
        let len = m.as_str().chars().count();
        byte = m.end();
        let out = (m.start(), m.end(), ch, ch + len);
        ch += len;
        out
    })
}

/// At most `n` characters of `t`, with "…" when there were more.
fn clip(t: &str, n: usize) -> String {
    let mut out: String = t.chars().take(n).collect();
    if out.len() < t.len() {
        out.push('…');
    }
    out
}

/// `text` around bytes `a..b`, on one line (the match itself clipped too).
fn context(text: &str, a: usize, b: usize) -> String {
    let mut before: Vec<char> = text.get(..a).unwrap_or("").chars().rev().take(CONTEXT + 1).collect();
    let cut = before.len() > CONTEXT;
    before.truncate(CONTEXT);
    let mut out: String = if cut { "…".into() } else { String::new() };
    out.extend(before.into_iter().rev());
    out.push_str(&clip(text.get(a..b).unwrap_or(""), 4 * CONTEXT));
    out.push_str(&clip(text.get(b..).unwrap_or(""), CONTEXT));
    out.replace(['\n', '\r', '\t'], " ")
}

/// Collects hits as they are found, up to [`MAX_HITS`].
struct Found<'a> {
    q: &'a Query,
    doc: &'a Document,
    hits: Vec<Hit>,
    truncated: bool,
}

impl Found<'_> {
    fn full(&mut self) -> bool {
        if self.hits.len() >= MAX_HITS {
            self.truncated = true;
        }
        self.truncated
    }

    fn push(&mut self, h: Hit) {
        if !self.full() {
            self.hits.push(h);
        }
    }

    fn text(&mut self, place: (Where, LayerId, Option<u32>, Option<ElementId>), field: Field, text: &str) {
        let q = self.q;
        let Needle::Text { re, .. } = &q.needle else { return };
        for (a, b, ca, cb) in text_matches(re, text) {
            if self.full() {
                return;
            }
            self.hits.push(Hit { at: place.0, layer: place.1, frame: place.2, element: place.3, field, start: ca, end: cb, found: clip(text.get(a..b).unwrap_or(""), 4 * CONTEXT), context: context(text, a, b) });
        }
    }

    fn timeline(&mut self, at: Where, tl: &Timeline) {
        let q = self.q;
        let w = q.within;
        for l in &tl.layers {
            if self.full() {
                return;
            }
            if w.layers && matches!(q.needle, Needle::Text { .. }) {
                self.text((at, l.id, None, None), Field::LayerName, &l.name);
            }
            for k in &l.keyframes {
                let f = Some(k.start);
                let place = |e: Option<ElementId>| Hit { at, layer: l.id, frame: f, element: e, field: Field::Text, start: 0, end: 0, found: String::new(), context: String::new() };
                match &q.needle {
                    Needle::Text { .. } => {
                        if w.labels {
                            self.text((at, l.id, f, None), Field::Label, &k.label);
                        }
                        if w.scripts
                            && let Some(FrameScript::Source(src)) = &k.script
                        {
                            self.text((at, l.id, f, None), Field::Script, &src.text);
                        }
                    }
                    Needle::Sound(id) if k.sound.as_ref().is_some_and(|fs| fs.sound == *id) => {
                        let found = self.doc.sound(*id).map(|x| x.name.clone()).unwrap_or_default();
                        self.push(Hit { field: Field::Sound, found, ..place(None) });
                    }
                    _ => {}
                }
                for e in &k.elements {
                    if self.full() {
                        return;
                    }
                    let place = (at, l.id, f, Some(e.id));
                    match (&q.needle, &e.kind) {
                        (Needle::Text { .. }, kind) => {
                            if w.instances {
                                self.text(place, Field::InstanceName, &e.name);
                            }
                            if w.text
                                && let ElementKind::Text(t) = kind
                            {
                                self.text(place, Field::Text, &t.text);
                            }
                        }
                        (Needle::Font(f), ElementKind::Text(t)) if font_key(&t.font) == *f => self.push(Hit { field: Field::Font, found: t.font.clone(), ..hit_at(place) }),
                        (Needle::Color(c), ElementKind::Shape(sh)) => {
                            if w.fills && sh.fill.as_ref().is_some_and(|p| paint_has(p, *c)) {
                                self.push(Hit { field: Field::Fill, found: hex(*c), ..hit_at(place) });
                            }
                            if w.strokes && sh.stroke.as_ref().is_some_and(|s| rgb8(s.color) == *c) {
                                self.push(Hit { field: Field::Stroke, found: hex(*c), ..hit_at(place) });
                            }
                        }
                        (Needle::Color(c), ElementKind::Text(t)) if w.text_color && rgb8(t.color) == *c => self.push(Hit { field: Field::TextColor, found: hex(*c), ..hit_at(place) }),
                        (Needle::Symbol(id), ElementKind::Symbol { symbol, .. }) if symbol == id => {
                            let found = self.doc.symbol(*id).map(|x| x.name.clone()).unwrap_or_default();
                            self.push(Hit { field: Field::Instance, found, ..hit_at(place) });
                        }
                        (Needle::Bitmap(id), ElementKind::Shape(sh)) if matches!(&sh.fill, Some(Paint::Bitmap { bitmap, .. }) if bitmap == id) => {
                            let found = self.doc.bitmap(*id).map(|x| x.name.clone()).unwrap_or_default();
                            self.push(Hit { field: Field::Bitmap, found, ..hit_at(place) });
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}

fn hit_at(place: (Where, LayerId, Option<u32>, Option<ElementId>)) -> Hit {
    Hit { at: place.0, layer: place.1, frame: place.2, element: place.3, field: Field::Text, start: 0, end: 0, found: String::new(), context: String::new() }
}

fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

fn paint_has(p: &Paint, c: [u8; 3]) -> bool {
    match p {
        Paint::Solid(x) => rgb8(*x) == c,
        Paint::Linear { stops, .. } | Paint::Radial { stops, .. } => stops.iter().any(|s| rgb8(s.color) == c),
        Paint::Bitmap { .. } => false,
    }
}

/// Every hit of `q` in its scope (from scene `scene`), in order, and whether the list stopped at
/// [`MAX_HITS`].
pub fn search(doc: &Document, scene: usize, q: &Query) -> (Vec<Hit>, bool) {
    let mut f = Found { q, doc, hits: Vec::new(), truncated: false };
    for at in timelines(doc, scene, q.scope) {
        if let Some(tl) = timeline_of(doc, at) {
            f.timeline(at, tl);
        }
        if f.truncated {
            break;
        }
    }
    (f.hits, f.truncated)
}

fn search_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let q = Query::from_params(s, p)?;
    let offset = opt_u64(p, "offset")?.map_or(0, |n| usize::try_from(n).unwrap_or(usize::MAX));
    let limit = opt_u64(p, "limit")?.map_or(DEFAULT_LIMIT, |n| usize::try_from(n).unwrap_or(MAX_LIMIT)).min(MAX_LIMIT);
    let (hits, truncated) = search(&s.doc, s.scene, &q);
    let kind = q.kind();
    let page: Vec<Value> = hits.iter().skip(offset).take(limit).map(|h| h.to_json(&s.doc, kind)).collect();
    Ok(json!({"hits": page, "total": hits.len(), "offset": offset, "truncated": truncated}))
}

// ----- replace -----------------------------------------------------------------------------

/// What hits are replaced with.
#[derive(Clone, Debug)]
pub enum With {
    Text(String),
    Font(String),
    Color(Rgba),
    Symbol(SymbolId),
    Sound(SoundId),
    Bitmap(BitmapId),
}

impl With {
    fn from_params(s: &Session, kind: Kind, p: &Value) -> Result<With> {
        let v = p.get("with").ok_or_else(|| EngineError::Param("param 'with' is required: what to replace with".into()))?;
        let id = || v.as_u64().ok_or_else(|| EngineError::Param("'with' must be a Library id".into()));
        Ok(match kind {
            Kind::Text => {
                let t = v.as_str().ok_or_else(|| EngineError::Param("'with' must be text".into()))?;
                if t.chars().count() > MAX_QUERY_CHARS {
                    return Err(EngineError::Param(format!("what to replace with is at most {MAX_QUERY_CHARS} characters")));
                }
                With::Text(t.to_string())
            }
            Kind::Font => {
                let f = v.as_str().map(str::trim).filter(|f| !f.is_empty() && f.chars().count() <= 256).ok_or_else(|| EngineError::Param("'with' must be a font family".into()))?;
                With::Font(f.to_string())
            }
            Kind::Color => With::Color(opt_color(p, "with")?.ok_or_else(|| EngineError::Param("'with' must be a colour: #rrggbb".into()))?),
            Kind::Symbol => {
                let to = SymbolId(id()?);
                s.doc.symbol(to).ok_or_else(|| EngineError::State(format!("no symbol with id {}", to.0)))?;
                With::Symbol(to)
            }
            Kind::Sound => {
                let to = SoundId(id()?);
                s.doc.sound(to).ok_or_else(|| EngineError::State(format!("no sound with id {}", to.0)))?;
                With::Sound(to)
            }
            Kind::Bitmap => {
                let to = BitmapId(id()?);
                s.doc.bitmap(to).ok_or_else(|| EngineError::State(format!("no bitmap with id {}", to.0)))?;
                With::Bitmap(to)
            }
        })
    }
}

/// `text` with the matches of `re` replaced by `with` (only the one starting at character `only`
/// when given); `None` when nothing matched.
fn replace_text(re: &Regex, expand: bool, text: &str, with: &str, only: Option<usize>) -> Option<String> {
    let mut out = String::with_capacity(text.len());
    // `last`: copied up to here; `seen`, `ch`: characters counted up to byte `seen`.
    let (mut last, mut seen, mut ch, mut any) = (0usize, 0usize, 0usize, false);
    for caps in re.captures_iter(text) {
        let Some(m) = caps.get(0) else { continue };
        if m.is_empty() {
            continue;
        }
        ch += text.get(seen..m.start()).map_or(0, |t| t.chars().count());
        let this = ch;
        ch += m.as_str().chars().count();
        seen = m.end();
        if only.is_some_and(|o| o != this) {
            continue;
        }
        out.push_str(text.get(last..m.start()).unwrap_or(""));
        if expand {
            caps.expand(with, &mut out);
        } else {
            out.push_str(with);
        }
        last = m.end();
        any = true;
        if only.is_some() {
            break;
        }
    }
    if !any {
        return None;
    }
    out.push_str(text.get(last..).unwrap_or(""));
    Some(out)
}

/// The places Replace All would change (each field once).
pub fn places(hits: &[Hit]) -> usize {
    let mut seen = HashSet::new();
    hits.iter().filter(|h| seen.insert((h.at, h.layer, h.frame, h.element, h.field))).count()
}

/// How many places `find.replace` with `p` would change (for its confirmation); `None` when
/// the params don't make a query or name one hit.
pub(crate) fn replace_count(s: &Session, p: &Value) -> Option<usize> {
    if p.get("at").is_some_and(|v| !v.is_null()) {
        return None;
    }
    let q = Query::from_params(s, p).ok()?;
    Some(places(&search(&s.doc, s.scene, &q).0))
}

/// What a replacement did.
#[derive(Default)]
struct Tally {
    replaced: usize,
    skipped: usize,
    notes: Vec<String>,
}

impl Tally {
    fn note(&mut self, n: String) {
        if !self.notes.contains(&n) && self.notes.len() < 16 {
            self.notes.push(n);
        }
    }
}

fn replace_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let q = Query::from_params(s, p)?;
    let with = With::from_params(s, q.kind(), p)?;
    let only = match p.get("at") {
        None | Some(Value::Null) => None,
        Some(v) => Some(Hit::target(v)?),
    };
    let (mut hits, truncated) = search(&s.doc, s.scene, &q);
    if let Some(t) = &only {
        hits.retain(|h| t.is(h));
        if hits.is_empty() {
            return Err(EngineError::State("that match isn't there any more; search again".into()));
        }
    }
    if hits.is_empty() {
        return Err(EngineError::State("nothing matches".into()));
    }
    // A symbol can't end up inside itself: no instance of `to` in `to`, or in a symbol it uses.
    let blocked: HashSet<SymbolId> = match with {
        With::Symbol(to) => reachable(&s.doc, [to]),
        _ => HashSet::new(),
    };
    let sizes: HashMap<BitmapId, (String, f32, f32)> = s.doc.bitmaps.iter().map(|b| (b.id, (b.name.clone(), b.width as f32, b.height as f32))).collect();
    let mut tally = Tally::default();
    let mut by_tl: Vec<(Where, Vec<Hit>)> = Vec::new();
    for h in hits {
        match by_tl.last_mut() {
            Some((at, list)) if *at == h.at => list.push(h),
            _ => by_tl.push((h.at, vec![h])),
        }
    }
    for (at, list) in by_tl {
        if let Where::Symbol(id) = at
            && blocked.contains(&id)
        {
            tally.skipped += places(&list);
            tally.note("an instance inside the symbol it would show (or one it uses) was left as it is: a symbol can't contain itself".into());
            continue;
        }
        let Some(tl) = timeline_of_mut(&mut s.doc, at) else { continue };
        apply(tl, &list, &q, &with, only.as_ref().map(|t| t.start), &sizes, &mut tally)?;
    }
    if tally.replaced == 0 {
        return Err(EngineError::State(tally.notes.first().cloned().unwrap_or_else(|| "nothing could be replaced".into())));
    }
    // The fill swatch and the text style keep what they were: replacing edits the document only.
    Ok(json!({"replaced": tally.replaced, "skipped": tally.skipped, "notes": tally.notes, "truncated": truncated}))
}

/// Replaces `hits` (all on `tl`) in place, un-sharing only the layers, keyframes and elements
/// they are on. `only`: the one text match to replace (Replace), by its first character.
fn apply(tl: &mut Timeline, hits: &[Hit], q: &Query, with: &With, only: Option<usize>, sizes: &HashMap<BitmapId, (String, f32, f32)>, t: &mut Tally) -> Result<()> {
    let layers: HashSet<LayerId> = hits.iter().map(|h| h.layer).collect();
    let (re, expand) = match &q.needle {
        Needle::Text { re, expand } => (Some(re), *expand),
        _ => (None, false),
    };
    let new_text = |text: &str| -> Option<String> {
        match (re, with) {
            (Some(re), With::Text(w)) => replace_text(re, expand, text, w, only),
            _ => None,
        }
    };
    for l in tl.layers.iter_mut().filter(|l| layers.contains(&l.id)) {
        let lh: Vec<&Hit> = hits.iter().filter(|h| h.layer == l.id).collect();
        let l = Arc::make_mut(l);
        if lh.iter().any(|h| h.field == Field::LayerName) {
            match new_text(&l.name).map(|n| n.chars().take(128).collect::<String>()) {
                Some(n) if !n.trim().is_empty() => {
                    l.name = n;
                    t.replaced += 1;
                }
                _ => {
                    t.skipped += 1;
                    t.note("a layer name can't be empty".into());
                }
            }
        }
        let frames: HashSet<u32> = lh.iter().filter_map(|h| h.frame).collect();
        for k in l.keyframes.iter_mut().filter(|k| frames.contains(&k.start)) {
            let kh: Vec<&&Hit> = lh.iter().filter(|h| h.frame == Some(k.start)).collect();
            let k = Arc::make_mut(k);
            let has = |f: Field| kh.iter().any(|h| h.field == f && h.element.is_none());
            if has(Field::Label)
                && let Some(n) = new_text(&k.label)
            {
                k.label = n.chars().take(256).collect();
                t.replaced += 1;
            }
            if has(Field::Script)
                && let Some(FrameScript::Source(src)) = &k.script
                && let Some(n) = new_text(&src.text)
            {
                if n.len() > anicraft_doc::actions::MAX_SOURCE_BYTES {
                    t.skipped += 1;
                    t.note("a script would be too long".into());
                } else {
                    k.script = Some(FrameScript::Source(Arc::new(SourceScript::new(src.language, n))));
                    t.replaced += 1;
                }
            }
            if has(Field::Sound)
                && let (Some(fs), With::Sound(to)) = (k.sound.as_mut(), with)
            {
                fs.sound = *to;
                t.replaced += 1;
            }
            let ids: HashSet<ElementId> = kh.iter().filter_map(|h| h.element).collect();
            for e in k.elements.iter_mut().filter(|e| ids.contains(&e.id)) {
                let fields: Vec<Field> = kh.iter().filter(|h| h.element == Some(e.id)).map(|h| h.field).collect();
                let e = Arc::make_mut(e);
                for f in Field::ALL.into_iter().filter(|f| fields.contains(f)) {
                    apply_element(e, f, q, with, &new_text, sizes, t)?;
                }
            }
        }
    }
    Ok(())
}

/// Replaces field `f` of element `e`.
fn apply_element(e: &mut anicraft_doc::Element, f: Field, q: &Query, with: &With, new_text: &dyn Fn(&str) -> Option<String>, sizes: &HashMap<BitmapId, (String, f32, f32)>, t: &mut Tally) -> Result<()> {
    match (f, &mut e.kind, with) {
        (Field::InstanceName, _, _) => {
            if let Some(n) = new_text(&e.name) {
                e.name = n.chars().take(128).collect();
                t.replaced += 1;
            }
        }
        (Field::Text, ElementKind::Text(tb), _) => {
            if let Some(n) = new_text(&tb.text) {
                if n.trim().is_empty() {
                    t.skipped += 1;
                    t.note("a text field can't be left empty (delete it instead)".into());
                } else {
                    // Laid out again as it is: text on a path stays on it (ADR 0070).
                    let (block, note, shift) = crate::cmds_text::relay_static(tb, &n, &crate::cmds_text::style_of(tb))?;
                    t.note_opt(note);
                    *tb = Arc::new(block);
                    e.transform = e.transform.mul(&anicraft_geom::Affine::translate(shift.x, shift.y));
                    t.replaced += 1;
                }
            }
        }
        (Field::Font, ElementKind::Text(tb), With::Font(family)) => {
            let style = crate::cmds_text::TextStyle { font: family.clone(), ..crate::cmds_text::style_of(tb) };
            let (block, note, shift) = crate::cmds_text::relay_static(tb, &tb.text, &style)?;
            t.note_opt(note);
            *tb = Arc::new(block);
            e.transform = e.transform.mul(&anicraft_geom::Affine::translate(shift.x, shift.y));
            t.replaced += 1;
        }
        (Field::TextColor, ElementKind::Text(tb), With::Color(c)) => {
            let tb = Arc::make_mut(tb);
            tb.color = Rgba { a: tb.color.a, ..*c };
            t.replaced += 1;
        }
        (Field::Fill | Field::Stroke, ElementKind::Shape(sh), With::Color(c)) => {
            let Needle::Color(from) = q.needle else { return Ok(()) };
            let recolour = |x: &mut Rgba| {
                if rgb8(*x) == from {
                    *x = Rgba { a: x.a, ..*c };
                }
            };
            if f == Field::Fill {
                match &mut sh.fill {
                    Some(Paint::Solid(x)) => recolour(x),
                    Some(Paint::Linear { stops, .. } | Paint::Radial { stops, .. }) => stops.iter_mut().for_each(|s| recolour(&mut s.color)),
                    _ => {}
                }
            } else if let Some(st) = &mut sh.stroke {
                recolour(&mut st.color);
            }
            t.replaced += 1;
        }
        (Field::Instance, ElementKind::Symbol { symbol, .. }, With::Symbol(to)) => {
            *symbol = *to;
            t.replaced += 1;
        }
        (Field::Bitmap, ElementKind::Shape(_), With::Bitmap(to)) => {
            t.replaced += usize::from(crate::cmds_library::swap_bitmap_of(e, *to, sizes));
        }
        _ => {}
    }
    Ok(())
}

impl Tally {
    fn note_opt(&mut self, n: Option<String>) {
        if let Some(n) = n {
            self.note(n);
        }
    }
}

// ----- go to -------------------------------------------------------------------------------

/// One instance opened in place on the way to a symbol: on `layer`, at the keyframe starting at
/// `frame`, the instance `element`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Step {
    pub layer: LayerId,
    pub frame: u32,
    pub element: ElementId,
}

impl Step {
    pub fn to_json(&self) -> Value {
        json!({"layer": self.layer.0, "frame": self.frame, "element": self.element.0})
    }
}

/// Where a scene or symbol timeline holding layer `id` is.
pub fn locate_layer(doc: &Document, id: LayerId) -> Option<Where> {
    doc.scenes.iter().position(|s| s.timeline.layer(id).is_some()).map(Where::Scene).or_else(|| doc.library.iter().find(|s| s.timeline.layer(id).is_some()).map(|s| Where::Symbol(s.id)))
}

/// The shortest chain of instances from scene `scene`'s timeline to symbol `target` (each opened
/// in place in turn), when an instance in that scene leads to it.
pub fn route(doc: &Document, scene: usize, target: SymbolId) -> Option<Vec<Step>> {
    let by_id: HashMap<SymbolId, &Symbol> = doc.library.iter().map(|s| (s.id, &**s)).collect();
    let mut seen: HashSet<SymbolId> = HashSet::new();
    let mut todo: VecDeque<(&Timeline, Vec<Step>)> = VecDeque::from([(doc.timeline(scene), Vec::new())]);
    while let Some((tl, path)) = todo.pop_front() {
        if path.len() >= MAX_ROUTE {
            continue;
        }
        for l in &tl.layers {
            for k in &l.keyframes {
                for e in &k.elements {
                    let ElementKind::Symbol { symbol, .. } = e.kind else { continue };
                    let mut next = path.clone();
                    next.push(Step { layer: l.id, frame: k.start, element: e.id });
                    if symbol == target {
                        return Some(next);
                    }
                    if seen.insert(symbol)
                        && let Some(sym) = by_id.get(&symbol)
                    {
                        todo.push_back((&sym.timeline, next));
                    }
                }
            }
        }
    }
    None
}

fn steps_of(p: &Value) -> Result<Option<Vec<Step>>> {
    let Some(list) = p.get("path").filter(|v| !v.is_null()) else { return Ok(None) };
    let list = list.as_array().ok_or_else(|| EngineError::Param("'path' must be a list of {layer, frame, element}".into()))?;
    if list.len() > MAX_ROUTE {
        return Err(EngineError::Param(format!("'path' opens at most {MAX_ROUTE} instances")));
    }
    let bad = || EngineError::Param("each step of 'path' is {layer, frame, element}".into());
    list.iter()
        .map(|v| Ok(Step { layer: LayerId(opt_u64(v, "layer")?.ok_or_else(bad)?), frame: opt_u32(v, "frame")?.ok_or_else(bad)?, element: ElementId(opt_u64(v, "element")?.ok_or_else(bad)?) }))
        .collect::<Result<Vec<_>>>()
        .map(Some)
}

/// Opens each instance of `steps` in place, from the timeline being edited. False when one
/// can't be opened (a hidden or locked layer, an instance that isn't there).
fn walk(s: &mut Session, steps: &[Step]) -> bool {
    for st in steps {
        if s.timeline().layer(st.layer).is_none() {
            return false;
        }
        s.layer = st.layer;
        s.frame = st.frame.min(crate::MAX_FRAME);
        s.selection.clear();
        if crate::editing::edit(s, &json!({"id": st.element.0})).is_err() {
            return false;
        }
    }
    true
}

fn goto_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let layer = opt_u64(p, "layer")?.map(LayerId);
    let frame = opt_u32(p, "frame")?;
    let element = opt_u64(p, "element")?.map(ElementId);
    let scene = match opt_u64(p, "scene")? {
        Some(i) => Some(usize::try_from(i).ok().filter(|i| *i < s.doc.scenes.len()).ok_or_else(|| EngineError::State(format!("there is no scene {i}")))?),
        None => None,
    };
    let symbol = opt_u64(p, "symbol")?.map(SymbolId);
    if let Some(id) = symbol
        && s.doc.symbol(id).is_none()
    {
        return Err(EngineError::State(format!("no symbol with id {}", id.0)));
    }
    let steps = steps_of(p)?;
    if steps.is_some() && layer.is_none() {
        return Err(EngineError::Param("a 'path' goes to a 'layer': name it".into()));
    }
    // Where to end up: the layer's timeline, else what the params name.
    let target = match layer {
        Some(l) => locate_layer(&s.doc, l).ok_or_else(|| EngineError::State(format!("no layer with id {}", l.0)))?,
        None => match (symbol, scene) {
            (Some(id), _) => Where::Symbol(id),
            (None, Some(i)) => Where::Scene(i),
            (None, None) => return Err(EngineError::Param("name where to go: {layer}, {scene} or {symbol}".into())),
        },
    };
    let here = s.scene;
    // Open symbols close first (ADR 0045).
    s.exit_all();
    match target {
        Where::Scene(i) => crate::cmds_scene::open_scene(s, i),
        Where::Symbol(t) => {
            let opened = match &steps {
                // Through the instances named, from the scene (or from `symbol`, opened alone).
                Some(steps) => {
                    match symbol {
                        Some(b) => {
                            crate::editing::edit(s, &json!({"symbol": b.0}))?;
                        }
                        None => crate::cmds_scene::open_scene(s, scene.unwrap_or(here)),
                    }
                    walk(s, steps) && s.editing_symbol() == Some(t)
                }
                // In place through the first instance that leads there: in this scene, else another.
                None => {
                    let order = std::iter::once(scene.unwrap_or(here)).chain(0..s.doc.scenes.len());
                    let found = order.into_iter().find_map(|i| route(&s.doc, i, t).map(|r| (i, r)));
                    match found {
                        Some((i, r)) => {
                            crate::cmds_scene::open_scene(s, i);
                            walk(s, &r) && s.editing_symbol() == Some(t)
                        }
                        None => false,
                    }
                }
            };
            if !opened {
                // Alone, from the Library (no instance leads there, or one can't be opened).
                s.exit_all();
                crate::editing::edit(s, &json!({"symbol": t.0}))?;
            }
        }
    }
    if let Some(l) = layer.filter(|l| s.timeline().layer(*l).is_some()) {
        s.layer = l;
    }
    if let Some(f) = frame {
        s.frame = f.min(crate::MAX_FRAME);
    }
    s.selection = match element {
        Some(e) => Selection::only(s.layer, [e]),
        None => Selection::default(),
    };
    s.revision += 1;
    s.dl_cache.replace(None);
    s.fix_selection();
    let mut v = crate::editing::editing_json(s);
    v["scene"] = json!(s.scene);
    Ok(v)
}

#[cfg(test)]
#[path = "find_tests.rs"]
pub(crate) mod tests;
