//! HTML canvas export (ADR 0028): the movie as one self-contained web page that plays in any
//! modern browser with `<canvas>` 2D, buttons and frame actions included.
//!
//! The page holds three things: the movie's data as JSON, the player (`html/player.js`, a port
//! of the Rust player in `anicraft-doc`) and the page code (`html/canvas.js`: drawing, timing,
//! pointer, sound). Both scripts are our own source files, embedded at build time.
//!
//! The data is baked, not re-keyed: every timeline (each scene's and each symbol's that the
//! movie uses) is evaluated frame by frame, so classic, shape and motion tweens, easing, motion
//! guides and shape hints come out exactly as AniCraft computes them, and the browser never
//! interpolates. A frame is its layers (bottom first) and their items: shapes (a path, a paint,
//! a stroke, a colour effect) and symbol instances (which the player resolves at run time: movie
//! clips have their own playheads, buttons their pointer states). Identical paths, paints and
//! frames are stored once, so a still keyframe over many frames costs one entry.
//!
//! What doesn't carry across is counted in the report: filters, 9-slice scaling (instances
//! scale as a whole), Subtract and Invert blending,
//! plug-in frame scripts (they need the plug-in host) and kept scripts
//! outside the ActionScript subset. Scripts in the subset (ADR 0013 tiers 2 and 2.5) come
//! across as the same script IR the Rust player runs (as JSON), with each timeline's functions
//! and variable types, and `player.js` evaluates it the same way.
//!
//! Dynamic and input text fields (ADR 0066) are instances the player names, like movie clips.
//! Their authored text is its outline, as everywhere; text a script or the viewer changes is
//! drawn by the page with the browser's fonts, and the viewer types into an input field through
//! a real `<input>` or `<textarea>` over the canvas (so editing, selection, IME and screen
//! readers are the browser's own).

use crate::stroke::stroke_outline;
use anicraft_doc::actions::as3::Decls;
use anicraft_doc::actions::{Action, FrameRef, Seg, Target};
use anicraft_doc::player::TimelineRef;
use anicraft_doc::{BlendMode, ColorTransform, Document, Element, ElementKind, FIELD_GUTTER, FrameScript, Join, Layer, LayerKind, MAX_FIELD_CHARS, Paint, Rgba, SoundFormat, SoundSync, SymbolId, SymbolKind, TextAlign, Timeline, base64_encode, evaluate_guided, guide_walk, instance_start};
use anicraft_geom::{Affine, FillRule, Path, PathCmd};
use serde_json::{Map, Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::sync::Arc;

/// The player core: instance tree, frame actions, button events (no page access).
pub const PLAYER_JS: &str = include_str!("html/player.js");
/// The page: canvas drawing, timing, pointer and sound.
pub const CANVAS_JS: &str = include_str!("html/canvas.js");
/// Version of the embedded data (`"v"`), raised when the player must change to read it.
pub const FORMAT_VERSION: u32 = 1;
/// Largest page written.
pub const MAX_HTML_BYTES: usize = 512 << 20;

/// What to export.
#[derive(Clone, Debug)]
pub struct HtmlOptions {
    /// The page title.
    pub title: String,
    /// Embed the movie's sounds (played with Web Audio).
    pub sound: bool,
    /// Scale the movie to fit the browser window (else it shows at its stage size).
    pub fit: bool,
}

impl Default for HtmlOptions {
    fn default() -> Self {
        HtmlOptions { title: "AniCraft movie".into(), sound: true, fit: true }
    }
}

/// Counts of what the export holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HtmlStats {
    pub timelines: usize,
    /// Frames over every timeline.
    pub frames: usize,
    /// Distinct frames stored.
    pub contents: usize,
    pub paths: usize,
    pub images: usize,
    pub sounds: usize,
    /// Bytes of movie data and of player code in the page.
    pub data_bytes: usize,
    pub code_bytes: usize,
}

/// The movie's data, as the player reads it.
pub struct Movie {
    pub json: Value,
    /// The exported paths, by index (the shapes' own paths; the JSON holds them rounded).
    pub paths: Vec<Arc<Path>>,
    /// What couldn't be carried across exactly, by kind, with how many elements (or
    /// keyframes) it touched.
    pub report: BTreeMap<String, usize>,
    pub stats: HtmlStats,
}

/// The finished page.
pub struct HtmlExport {
    pub html: String,
    pub report: BTreeMap<String, usize>,
    pub stats: HtmlStats,
}

/// Notes for the report.
const FILTERS: &str = "filters (not exported; drawn without them)";
const BLENDS: &str = "Subtract and Invert blending (drawn normally)";
const PLUGIN_SCRIPTS: &str = "plug-in frame scripts (not exported; they need the plug-in host)";
const KEPT_SCRIPTS: &str = "kept frame scripts outside the ActionScript subset (not run; kept in the document only)";
const NINE_SLICE: &str = "9-slice scaling (scaled as a whole)";
const OUTSIDE_CALLS: &str = "script calls that reach outside the movie (ignored)";
const FIELD_FONTS: &str = "text fields (their authored text keeps its outlines; text a script or the viewer changes is drawn with the browser's fonts)";

/// A number rounded to `1/scale`, written as an integer when it is one.
fn num(v: f32, scale: f64) -> Value {
    let r = (f64::from(v) * scale).round() / scale;
    if !r.is_finite() {
        json!(0)
    } else if r.fract() == 0.0 && r.abs() < 9e15 {
        json!(r as i64)
    } else {
        json!(r)
    }
}

/// A stroke's style for the page (ADR 0046): `c` cap, `j` join, `ml` miter limit, and for a
/// broken line `d` its on/off lengths and `o` the offset into them (local units, scaled by
/// the page like the width; dots are zero-length round-capped dashes).
fn line_style(st: &anicraft_doc::Stroke) -> Value {
    let style = &st.style;
    let mut k = json!({"c": style.effective_cap().key(), "j": style.join.key(), "ml": num(style.miter_limit, 1e3)});
    if let Some(d) = &style.dash {
        let (lengths, offset) = d.pattern(st.width);
        k["d"] = Value::Array(lengths.into_iter().map(|l| num(l, 1e3)).collect());
        k["o"] = num(offset, 1e3);
    }
    k
}

fn coord(v: f32) -> Value {
    num(v, 1000.0)
}

fn color(c: Rgba) -> Vec<Value> {
    let c = c.sanitized();
    [c.r, c.g, c.b, c.a].into_iter().map(|v| num(v, 10_000.0)).collect()
}

/// `[a, b, c, d, e, f]`, the linear part finer than the offset; `None` for the identity.
fn matrix(m: &Affine) -> Option<Value> {
    if *m == Affine::IDENTITY {
        return None;
    }
    let [a, b, c, d, e, f] = m.m;
    Some(json!([num(a, 1e5), num(b, 1e5), num(c, 1e5), num(d, 1e5), coord(e), coord(f)]))
}

/// A colour transform (ADR 0056): `[red, green, blue, alpha multipliers, red, green, blue,
/// alpha offsets]`; `None` for no effect.
fn effect(fx: &ColorTransform) -> Option<Value> {
    let fx = fx.sanitized();
    if fx.is_identity() {
        return None;
    }
    Some(Value::Array(fx.mul.iter().chain(fx.add.iter()).map(|v| num(*v, 1e4)).collect()))
}

/// SVG path data (what `Path2D` reads), three decimals; empty when a point isn't finite.
fn path_data(p: &Path) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(p.cmds.len() * 14);
    let pt = |s: &mut String, x: f32, y: f32| {
        let _ = write!(s, "{} {}", coord(x), coord(y));
    };
    for c in &p.cmds {
        match *c {
            PathCmd::MoveTo(a) => {
                s.push('M');
                pt(&mut s, a.x, a.y);
            }
            PathCmd::LineTo(a) => {
                s.push('L');
                pt(&mut s, a.x, a.y);
            }
            PathCmd::QuadTo(a, b) => {
                s.push('Q');
                pt(&mut s, a.x, a.y);
                s.push(' ');
                pt(&mut s, b.x, b.y);
            }
            PathCmd::CubicTo(a, b, e) => {
                s.push('C');
                pt(&mut s, a.x, a.y);
                s.push(' ');
                pt(&mut s, b.x, b.y);
                s.push(' ');
                pt(&mut s, e.x, e.y);
            }
            PathCmd::Close => s.push('Z'),
        }
    }
    let finite = p.cmds.iter().all(|c| match *c {
        PathCmd::MoveTo(a) | PathCmd::LineTo(a) => a.x.is_finite() && a.y.is_finite(),
        PathCmd::QuadTo(a, b) => [a, b].iter().all(|q| q.x.is_finite() && q.y.is_finite()),
        PathCmd::CubicTo(a, b, e) => [a, b, e].iter().all(|q| q.x.is_finite() && q.y.is_finite()),
        PathCmd::Close => true,
    });
    if finite { s } else { String::new() }
}

/// A target as the player reads it: `t`, its steps (`^` the main timeline, `..` up a level,
/// else an instance name), or `e` when it isn't a valid path.
fn target(o: &mut Map<String, Value>, t: &Target) {
    {
        match t.segments() {
            Ok(segs) => {
                if !segs.is_empty() {
                    let steps: Vec<Value> = segs
                        .iter()
                        .map(|s| match s {
                            Seg::Root => json!("^"),
                            Seg::Parent => json!(".."),
                            Seg::Child(n) => json!(n),
                        })
                        .collect();
                    o.insert("t".into(), Value::Array(steps));
                }
            }
            Err(e) => {
                o.insert("e".into(), json!(e));
            }
        }
    }
}

/// An action as the player reads it: `op`, its target (see [`target`]), and `d`, its
/// description (for messages).
fn action(a: &Action) -> Value {
    let mut o = Map::new();
    o.insert("op".into(), json!(a.op()));
    if let Some(t) = a.target() {
        target(&mut o, t);
    }
    match a {
        Action::Trace { message } => {
            o.insert("msg".into(), json!(message));
        }
        Action::GotoAndPlay { frame, scene, .. } | Action::GotoAndStop { frame, scene, .. } => {
            o.insert("f".into(), match frame {
                FrameRef::Number(n) => json!(n),
                FrameRef::Label(l) => json!(l),
            });
            if let Some(s) = scene {
                o.insert("s".into(), json!(s));
            }
        }
        Action::SetProperty { property, value, .. } => {
            o.insert("p".into(), json!(property.key()));
            o.insert("v".into(), if value.is_finite() { json!(value) } else { json!(0) });
            o.insert("pl".into(), json!(property.label().to_lowercase()));
        }
        _ => {}
    }
    if !matches!(a, Action::Trace { .. }) {
        o.insert("d".into(), json!(a.describe()));
    }
    Value::Object(o)
}

fn actions(list: &[Action]) -> Value {
    Value::Array(list.iter().map(action).collect())
}


/// Whether timeline layer `index` takes part in playback (what the player walks and draws):
/// shown, not a folder or guide, not in a hidden folder.
fn plays(tl: &Timeline, index: usize) -> bool {
    tl.layers.get(index).is_some_and(|l| l.visible && !matches!(l.kind, LayerKind::Folder | LayerKind::Guide)) && !tl.hidden_by_folder(index)
}

/// Interning tables and the report, while baking.
struct Bake<'a> {
    doc: &'a Document,
    sound: bool,
    paths: Vec<Arc<Path>>,
    path_d: Vec<String>,
    by_ptr: HashMap<*const Path, usize>,
    /// Every path met, kept alive so no pointer in `by_ptr` is reused by a later path.
    keep: Vec<Arc<Path>>,
    by_d: HashMap<String, usize>,
    paints: Vec<Value>,
    paint_ix: HashMap<String, usize>,
    contents: Vec<Value>,
    content_ix: HashMap<String, usize>,
    symbols: HashMap<SymbolId, usize>,
    images: Vec<String>,
    image_ix: HashMap<u64, usize>,
    sounds: Vec<Value>,
    sound_ix: HashMap<u64, usize>,
    notes: BTreeMap<&'static str, BTreeSet<u64>>,
}

impl<'a> Bake<'a> {
    fn path(&mut self, p: &Arc<Path>) -> usize {
        // The table keeps every path alive, so a pointer is never reused while it is a key.
        if let Some(&i) = self.by_ptr.get(&Arc::as_ptr(p)) {
            return i;
        }
        let d = path_data(p);
        let i = match self.by_d.get(&d) {
            Some(&i) => i,
            None => {
                self.path_d.push(d.clone());
                self.paths.push(p.clone());
                self.by_d.insert(d, self.paths.len() - 1);
                self.paths.len() - 1
            }
        };
        self.by_ptr.insert(Arc::as_ptr(p), i);
        self.keep.push(p.clone());
        i
    }

    fn paint(&mut self, p: &Paint) -> usize {
        let stops = |s: &[anicraft_doc::GradientStop]| -> Vec<Value> {
            s.iter()
                .map(|g| {
                    let o = if g.offset.is_finite() { g.offset.clamp(0.0, 1.0) } else { 0.0 };
                    let mut v = vec![num(o, 1e4)];
                    v.extend(color(g.color));
                    Value::Array(v)
                })
                .collect()
        };
        let v = match p {
            Paint::Solid(c) => json!({"s": color(*c)}),
            Paint::Linear { start, end, stops: s } => json!({"l": [coord(start.x), coord(start.y), coord(end.x), coord(end.y)], "st": stops(s)}),
            Paint::Radial { center, radius, stops: s } => json!({"r": [coord(center.x), coord(center.y), coord(radius.max(0.0))], "st": stops(s)}),
            Paint::Bitmap { bitmap, repeat, matrix: m } => match self.doc.bitmap(*bitmap) {
                Some(b) => {
                    let n = self.images.len();
                    let i = *self.image_ix.entry(b.id.0).or_insert(n);
                    if i == n {
                        self.images.push(format!("data:image/png;base64,{}", base64_encode(&b.png)));
                    }
                    let mut o = json!({"b": i, "rep": repeat});
                    if !b.smoothing {
                        o["px"] = json!(true);
                    }
                    if let Some(m) = matrix(m) {
                        o["m"] = m;
                    }
                    o
                }
                // An image that can't be found shows grey, as on the stage.
                None => json!({"s": color(Rgba::hex(0x808080))}),
            },
        };
        let key = v.to_string();
        let n = self.paints.len();
        let i = *self.paint_ix.entry(key).or_insert(n);
        if i == n {
            self.paints.push(v);
        }
        i
    }

    /// A shape item: `p` path, `f` paint, `s` stroke `[r, g, b, a, width]`, `k` its style when
    /// not round and solid (`c` cap, `j` join, `ml` miter limit, `d` dash lengths and `o` their
    /// offset, in local units like the width), `eo` even-odd, `m` transform, `c` colour effect.
    fn shape(&mut self, path: &Arc<Path>, m: &Affine, fx: &ColorTransform, fill: Option<&Paint>, stroke: Option<&anicraft_doc::Stroke>, rule: FillRule) -> Value {
        let mut o = Map::new();
        o.insert("p".into(), json!(self.path(path)));
        if let Some(p) = fill {
            o.insert("f".into(), json!(self.paint(p)));
        }
        if let Some(st) = stroke {
            let mut s = color(st.color);
            s.push(num(st.width, 1e3));
            o.insert("s".into(), Value::Array(s));
            if !st.style.is_default() {
                o.insert("k".into(), line_style(st));
            }
        }
        if rule == FillRule::EvenOdd {
            o.insert("eo".into(), json!(1));
        }
        if let Some(m) = matrix(m) {
            o.insert("m".into(), m);
        }
        if let Some(c) = effect(fx) {
            o.insert("c".into(), c);
        }
        Value::Object(o)
    }

    /// A dynamic or input text field (ADR 0066): `tf` holds what the player and the page need,
    /// `id`, `n` (its instance name), `m` and `c` as on a symbol instance. In `tf`: `k` its kind
    /// (`dynamic` or `input`), `ln` its line type, `w` × `h` its box, `sel` selectable, `mx` the
    /// most characters the viewer may type (when limited), `t` the authored text and `p` its
    /// outline (drawn like any path until the text changes), the paints `f` (text), `kp` (the
    /// box as a clip) and `bg` (background, when on), `bd` the border `[r, g, b, a, 1]` and `bk`
    /// its style (when on), the paths `r` (the box) and `cr` (the caret, as the player draws it
    /// before a layout), and `font` what the page draws changed text with: `family`, `size`,
    /// `bold`, `italic`, `align`, `ls` letter and `lsp` line spacing.
    fn field(&mut self, el: &Element, t: &anicraft_doc::TextBlock, f: &anicraft_doc::TextField) -> Value {
        // The browser lays out text a script or the viewer changed with its own fonts.
        self.notes.entry(FIELD_FONTS).or_default().insert(el.id.0);
        // As the evaluator draws a field (eval.rs `field_items`).
        let (w, h) = (t.box_width.max(0.0), t.box_height.max(0.0));
        let rect = Arc::new(Path::rect(0.0, 0.0, w, h));
        let caret = Arc::new(Path::rect(FIELD_GUTTER - 0.5, FIELD_GUTTER, 1.0, t.size * 1.2));
        let align = match t.align {
            TextAlign::Left => "left",
            TextAlign::Center => "center",
            TextAlign::Right => "right",
        };
        let size = if t.size.is_finite() && t.size > 0.0 { t.size } else { 12.0 };
        let mut o = json!({
            "k": f.kind.key(),
            "ln": f.lines.key(),
            "w": coord(w),
            "h": coord(h),
            "sel": f.selectable,
            "t": t.text,
            "p": self.path(&t.outline),
            "r": self.path(&rect),
            "cr": self.path(&caret),
            "f": self.paint(&Paint::Solid(t.color)),
            "kp": self.paint(&Paint::Solid(Rgba::BLACK)),
            "font": {"family": t.font, "size": num(size, 1e3), "bold": t.bold, "italic": t.italic, "align": align, "ls": num(t.letter_spacing, 1e3), "lsp": num(t.line_spacing, 1e3)},
        });
        if f.max_chars > 0 {
            o["mx"] = json!(f.max_chars.min(MAX_FIELD_CHARS as u32));
        }
        if f.background {
            o["bg"] = json!(self.paint(&Paint::Solid(f.background_color)));
        }
        if f.border {
            let mut s = anicraft_doc::Stroke::new(f.border_color, 1.0);
            s.style.join = Join::Miter;
            let mut bd = color(s.color);
            bd.push(num(s.width, 1e3));
            o["bd"] = Value::Array(bd);
            o["bk"] = line_style(&s);
        }
        let mut item = Map::new();
        item.insert("tf".into(), o);
        item.insert("id".into(), json!(el.id.0));
        if !el.name.is_empty() {
            item.insert("n".into(), json!(el.name));
        }
        if let Some(m) = matrix(&el.transform) {
            item.insert("m".into(), m);
        }
        if let Some(c) = effect(&el.color.transform()) {
            item.insert("c".into(), c);
        }
        Value::Object(item)
    }

    /// The items of one element on `layer` at frame `f` (its keyframe starts at `start`).
    fn element(&mut self, el: &Element, layer: &Layer, f: u32, start: u32, out: &mut Vec<Value>) {
        if !el.transform.is_finite() {
            return;
        }
        match &el.kind {
            ElementKind::Shape(s) => {
                let stroke = s.stroke.as_ref().filter(|st| st.width.is_finite() && st.width > 0.0);
                match stroke.filter(|st| st.profile.is_some()) {
                    // A variable-width stroke is the filled outline the renderer fills (its
                    // caps, joins and dashes included).
                    Some(st) => {
                        if let Some(p) = &s.fill {
                            out.push(self.shape(&s.path, &el.transform, &el.color.transform(), Some(p), None, s.fill_rule));
                        }
                        let outline = stroke_outline(&s.path, &el.transform, st, crate::TOLERANCE);
                        if !outline.is_empty() {
                            let fill = Paint::Solid(st.color);
                            out.push(self.shape(&Arc::new(outline), &Affine::IDENTITY, &el.color.transform(), Some(&fill), None, FillRule::NonZero));
                        }
                    }
                    None => out.push(self.shape(&s.path, &el.transform, &el.color.transform(), s.fill.as_ref(), stroke, s.fill_rule)),
                }
            }
            // The camera is the view, not artwork (ADR 0027): it is baked into the layers.
            ElementKind::Camera => {}
            ElementKind::Text(t) => match &t.field {
                Some(f) => {
                    let item = self.field(el, t, f);
                    out.push(item);
                }
                None => {
                    let fill = Paint::Solid(t.color);
                    out.push(self.shape(&t.outline, &el.transform, &el.color.transform(), Some(&fill), None, FillRule::NonZero));
                }
            },
            ElementKind::Symbol { symbol, loop_mode, first_frame, blend, filters, events } => {
                let (Some(&y), Some(sym)) = (self.symbols.get(symbol), self.doc.symbol(*symbol)) else { return };
                let len = sym.timeline.frame_count();
                // The page's player places symbols itself and doesn't slice them yet (ADR 0055).
                if anicraft_doc::nine_slice::slices(self.doc, sym, &el.transform) {
                    self.notes.entry(NINE_SLICE).or_default().insert(el.id.0);
                }
                let mut o = Map::new();
                o.insert("y".into(), json!(y));
                o.insert("id".into(), json!(el.id.0));
                if !el.name.is_empty() {
                    o.insert("n".into(), json!(el.name));
                }
                if let Some(m) = matrix(&el.transform) {
                    o.insert("m".into(), m);
                }
                if let Some(c) = effect(&el.color.transform()) {
                    o.insert("c".into(), c);
                }
                match sym.kind {
                    SymbolKind::Graphic => {
                        let g = loop_mode.local_frame(f.saturating_sub(start), *first_frame, len);
                        if g > 0 {
                            o.insert("g".into(), json!(g));
                        }
                    }
                    SymbolKind::MovieClip => {
                        // Where the clip is outside the player (inside a button's hit area).
                        let af = f.saturating_sub(instance_start(layer, f, el.id, *symbol)) % len.max(1);
                        if af > 0 {
                            o.insert("af".into(), json!(af));
                        }
                    }
                    SymbolKind::Button => {}
                }
                if !blend.is_normal() {
                    o.insert("bl".into(), json!(blend.key()));
                    if matches!(blend, BlendMode::Subtract | BlendMode::Invert) {
                        self.notes.entry(BLENDS).or_default().insert(el.id.0);
                    }
                }
                if !filters.is_empty() {
                    o.insert("fx".into(), json!(1));
                    self.notes.entry(FILTERS).or_default().insert(el.id.0);
                }
                if !events.is_empty() {
                    o.insert("ev".into(), Value::Array(events.iter().map(|h| json!([h.event.key(), actions(&h.actions)])).collect()));
                }
                out.push(Value::Object(o));
            }
        }
    }

    fn sound(&mut self, id: anicraft_doc::SoundId) -> Option<usize> {
        if !self.sound {
            return None;
        }
        let s = self.doc.sound(id)?;
        let n = self.sounds.len();
        let i = *self.sound_ix.entry(s.id.0).or_insert(n);
        if i == n {
            let mime = match s.format {
                SoundFormat::Wav => "audio/wav",
                SoundFormat::Ogg => "audio/ogg",
                SoundFormat::Mp3 => "audio/mpeg",
            };
            self.sounds.push(json!({"name": s.name, "mime": mime, "data": base64_encode(&s.data)}));
        }
        Some(i)
    }

    /// One timeline: its length `n`, frames as runs `f: [[content, count]…]`, mask layer count
    /// `nm`, labels `lb`, and per frame the scripts `a` and sounds `snd` of keyframes starting
    /// there (top layer first). A scene's frames seen through its camera (ADR 0027) carry the
    /// camera's tint (`cc`) and each layer its view (`v`: the camera and the layer's depth).
    /// `decls`: the declarations its frame scripts share (ADR 0052), for timelines that
    /// run scripts; their functions go out as `fn` and their variables' types as `vt`.
    fn timeline(&mut self, tl: &Timeline, scene: bool, decls: Option<&Decls>) -> Value {
        let n = tl.frame_count();
        // One clip slot per visible mask layer, top first (as the evaluator numbers them).
        let slots: HashMap<u64, usize> = tl.layers.iter().filter(|l| l.kind == LayerKind::Mask && l.visible).enumerate().map(|(i, l)| (l.id.0, i)).collect();
        let playing: Vec<usize> = (0..tl.layers.len()).filter(|&i| plays(tl, i)).collect();
        let stage = self.doc.stage;
        let mut runs: Vec<(usize, u32)> = Vec::new();
        let mut scripts = Map::new();
        let mut sounds = Map::new();
        for f in 0..n {
            let camera = if scene { tl.camera_at(f, &stage) } else { None };
            let mut layers = Vec::new();
            for &index in playing.iter().rev() {
                let Some(layer) = tl.layers.get(index).filter(|l| l.kind != LayerKind::Camera) else { continue };
                let mut entry = Map::new();
                if let Some(v) = camera.and_then(|c| matrix(&c.view(&stage, layer.depth))) {
                    entry.insert("v".into(), v);
                }
                // The layer's volume, for the sounds inside the symbols on it.
                if layer.sound_gain() != 1.0 {
                    entry.insert("sg".into(), num(layer.sound_gain(), 10_000.0));
                }
                if layer.kind == LayerKind::Mask {
                    entry.insert("mk".into(), json!(slots.get(&layer.id.0).copied().unwrap_or(0)));
                } else if let Some(&slot) = layer.parent.and_then(|p| slots.get(&p.0)) {
                    entry.insert("c".into(), json!(slot));
                }
                let walk = layer.parent.and_then(|p| tl.layer(p)).filter(|g| g.kind == LayerKind::Guide).and_then(|g| guide_walk(g, f));
                let mut items = Vec::new();
                if let Some((start, els)) = evaluate_guided(layer, f, walk.as_ref()) {
                    for el in els.iter() {
                        self.element(el, layer, f, start, &mut items);
                    }
                }
                if items.is_empty() && layer.kind != LayerKind::Mask {
                    continue;
                }
                entry.insert("e".into(), Value::Array(items));
                layers.push(Value::Object(entry));
            }
            let mut content = json!({"l": layers});
            if let Some(cc) = camera.and_then(|c| effect(&c.color())) {
                content["cc"] = cc;
            }
            let key = content.to_string();
            let next = self.contents.len();
            let ci = *self.content_ix.entry(key).or_insert(next);
            if ci == next {
                self.contents.push(content);
            }
            match runs.last_mut() {
                Some((c, count)) if *c == ci => *count += 1,
                _ => runs.push((ci, 1)),
            }
            // What entering this frame runs and plays.
            let mut a = Vec::new();
            let mut snd = Vec::new();
            for &index in &playing {
                let Some(k) = tl.layers.get(index).and_then(|l| l.keyframe_starting_at(f).and_then(|k| l.keyframes.get(k))) else { continue };
                match &k.script {
                    Some(FrameScript::Actions(list)) => a.push(actions(list)),
                    Some(FrameScript::Plugin(p)) => {
                        self.notes.entry(PLUGIN_SCRIPTS).or_default().insert(u64::from(f) << 20 ^ tl.layers.get(index).map_or(0, |l| l.id.0));
                        a.push(json!({"plugin": format!("{} · {}", p.id, p.entry)}));
                    }
                    // ActionScript in the subset runs as the player runs it; the rest is only
                    // kept (ADR 0013).
                    Some(FrameScript::Source(src)) => {
                        let key = u64::from(f) << 20 ^ tl.layers.get(index).map_or(0, |l| l.id.0);
                        match (src.compiled(), decls) {
                            // It runs where its names from other frames are all there.
                            (Ok(c), Some(d)) if d.problems(c).is_empty() => {
                                let mut o = json!({"b": c.body});
                                if !c.inner.is_empty() {
                                    o["i"] = json!(c.inner);
                                }
                                a.push(json!({"as": o}));
                                if !c.notes.is_empty() {
                                    self.notes.entry(OUTSIDE_CALLS).or_default().insert(key);
                                }
                            }
                            _ => {
                                self.notes.entry(KEPT_SCRIPTS).or_default().insert(key);
                            }
                        }
                    }
                    None => {}
                }
                if let Some(fs) = &k.sound
                    && let Some(i) = self.sound(fs.sound)
                {
                    let sync = match fs.sync {
                        SoundSync::Event => "event",
                        SoundSync::Start => "start",
                        SoundSync::Stop => "stop",
                        SoundSync::Stream => "stream",
                    };
                    // [sound, sync, repeat, loop, span, gain?, envelope?]: the layer's volume,
                    // and the effect's points ([seconds, left, right]) over the time it is heard.
                    let mut entry = vec![json!(i), json!(sync), json!(fs.repeat.max(1)), json!(fs.looping), json!(k.duration.max(1))];
                    let env: Vec<Value> = match self.doc.sound(fs.sound) {
                        Some(s) if !fs.effect.is_none() => {
                            let heard = fs.heard_seconds(s, k.duration, n.saturating_sub(k.start), stage.fps);
                            fs.effect.points(heard).iter().map(|p| json!([num(p.at as f32, 10_000.0), num(p.left, 10_000.0), num(p.right, 10_000.0)])).collect()
                        }
                        _ => Vec::new(),
                    };
                    let gain = tl.layers.get(index).map_or(1.0, |l| l.sound_gain());
                    if gain != 1.0 || !env.is_empty() {
                        entry.push(num(gain, 10_000.0));
                    }
                    if !env.is_empty() {
                        entry.push(Value::Array(env));
                    }
                    snd.push(Value::Array(entry));
                }
            }
            if !a.is_empty() {
                scripts.insert(f.to_string(), Value::Array(a));
            }
            if !snd.is_empty() {
                sounds.insert(f.to_string(), Value::Array(snd));
            }
        }
        // A label names its first frame, over every layer.
        let mut labels: BTreeMap<&str, u32> = BTreeMap::new();
        for k in tl.layers.iter().flat_map(|l| l.keyframes.iter()).filter(|k| !k.label.is_empty()) {
            let e = labels.entry(k.label.as_str()).or_insert(k.start);
            *e = (*e).min(k.start);
        }
        let mut o = json!({"n": n, "f": runs.iter().map(|(c, k)| json!([c, k])).collect::<Vec<_>>()});
        if !slots.is_empty() {
            o["nm"] = json!(slots.len());
        }
        if !labels.is_empty() {
            o["lb"] = Value::Array(labels.into_iter().map(|(l, f)| json!([l, f])).collect());
        }
        if !scripts.is_empty() {
            o["a"] = Value::Object(scripts);
        }
        if !sounds.is_empty() {
            o["snd"] = Value::Object(sounds);
        }
        if let Some(d) = decls {
            if !d.functions.is_empty() {
                o["fn"] = json!(d.functions);
            }
            if !d.vars.is_empty() {
                o["vt"] = json!(d.vars);
            }
        }
        o
    }
}

/// The symbols the movie uses, in the order first met (scenes first, then inside symbols).
fn used_symbols(doc: &Document) -> Vec<SymbolId> {
    let mut seen: BTreeSet<SymbolId> = BTreeSet::new();
    let mut order = Vec::new();
    let mut queue: VecDeque<&Timeline> = doc.scenes.iter().map(|s| &s.timeline).collect();
    while let Some(tl) = queue.pop_front() {
        for el in tl.layers.iter().flat_map(|l| l.keyframes.iter()).flat_map(|k| k.elements.iter()) {
            if let ElementKind::Symbol { symbol, .. } = &el.kind
                && let Some(sym) = doc.symbol(*symbol)
                && seen.insert(*symbol)
            {
                order.push(*symbol);
                queue.push_back(&sym.timeline);
            }
        }
    }
    order
}

/// The movie's data: the stage, tables (paths, paints, images, sounds), symbols, scenes, and
/// every used timeline's baked frames (scenes' first, then symbols' in `symbols` order).
pub fn movie(doc: &Document, opts: &HtmlOptions) -> Movie {
    let used = used_symbols(doc);
    let mut b = Bake {
        doc,
        sound: opts.sound,
        paths: Vec::new(),
        path_d: Vec::new(),
        by_ptr: HashMap::new(),
        keep: Vec::new(),
        by_d: HashMap::new(),
        paints: Vec::new(),
        paint_ix: HashMap::new(),
        contents: Vec::new(),
        content_ix: HashMap::new(),
        symbols: used.iter().enumerate().map(|(i, s)| (*s, i)).collect(),
        images: Vec::new(),
        image_ix: HashMap::new(),
        sounds: Vec::new(),
        sound_ix: HashMap::new(),
        notes: BTreeMap::new(),
    };
    let scenes = doc.scenes.len();
    let mut timelines = Vec::with_capacity(scenes + used.len());
    let mut frames = 0usize;
    // The main timeline's scripts share one set of declarations over every scene.
    let root = anicraft_doc::player::decls(doc, TimelineRef::Root);
    for s in &doc.scenes {
        frames += s.timeline.frame_count() as usize;
        timelines.push(b.timeline(&s.timeline, true, Some(&root)));
    }
    let mut symbols = Vec::with_capacity(used.len());
    for (i, id) in used.iter().enumerate() {
        let Some(sym) = doc.symbol(*id) else { continue };
        frames += sym.timeline.frame_count() as usize;
        let decls = (sym.kind == SymbolKind::MovieClip).then(|| anicraft_doc::player::decls(doc, TimelineRef::Symbol(*id)));
        timelines.push(b.timeline(&sym.timeline, false, decls.as_ref()));
        let kind = match sym.kind {
            SymbolKind::Graphic => "graphic",
            SymbolKind::MovieClip => "movieclip",
            SymbolKind::Button => "button",
        };
        symbols.push(json!({"name": sym.name, "k": kind, "tl": scenes + i}));
    }
    let stage = doc.stage;
    let fps = if stage.fps.is_finite() && stage.fps > 0.0 { stage.fps } else { 24.0 };
    let json = json!({
        "v": FORMAT_VERSION,
        "generator": format!("AniCraft {}", env!("CARGO_PKG_VERSION")),
        "stage": {"w": stage.width.max(1), "h": stage.height.max(1), "fps": num(fps, 1000.0), "bg": stage.background.to_hex()},
        "fit": opts.fit,
        "scenes": doc.scenes.iter().enumerate().map(|(i, s)| json!({"name": s.name, "tl": i})).collect::<Vec<_>>(),
        "symbols": symbols,
        "timelines": timelines,
        "contents": b.contents,
        "paths": b.path_d,
        "paints": b.paints,
        "images": b.images,
        "sounds": b.sounds,
    });
    let stats = HtmlStats {
        timelines: json["timelines"].as_array().map_or(0, Vec::len),
        frames,
        contents: json["contents"].as_array().map_or(0, Vec::len),
        paths: b.paths.len(),
        images: json["images"].as_array().map_or(0, Vec::len),
        sounds: json["sounds"].as_array().map_or(0, Vec::len),
        data_bytes: 0,
        code_bytes: PLAYER_JS.len() + CANVAS_JS.len(),
    };
    let report = b.notes.into_iter().map(|(k, v)| (k.to_string(), v.len())).collect();
    Movie { json, paths: b.paths, report, stats }
}

/// Text for HTML content and attributes.
fn escape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// The movie as one web page: the data, the player and the page code inline, images and
/// sounds embedded, so the file plays from a disk, a server or an e-mail attachment alike.
pub fn to_html(doc: &Document, opts: &HtmlOptions) -> Result<HtmlExport, String> {
    let mut m = movie(doc, opts);
    // In a <script> element only `</script` and `<!--` could end or confuse it; every `<` in
    // the JSON is inside a string, where `<` reads the same.
    let data = m.json.to_string().replace('<', "\\u003c");
    let title = escape_html(&opts.title);
    let bg = doc.stage.background.to_hex();
    let (w, h) = (doc.stage.width.max(1), doc.stage.height.max(1));
    let mut html = String::with_capacity(data.len() + PLAYER_JS.len() + CANVAS_JS.len() + 1024);
    html.push_str("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    html.push_str(&format!("<meta name=\"generator\" content=\"AniCraft {}\">\n<title>{title}</title>\n", env!("CARGO_PKG_VERSION")));
    html.push_str(&format!("<style>html,body{{margin:0;height:100%;background:{bg}}}body{{display:flex;align-items:center;justify-content:center;overflow:hidden}}canvas{{display:block;touch-action:none;user-select:none}}</style>\n</head>\n<body>\n"));
    html.push_str(&format!("<canvas id=\"anicraft-stage\" width=\"{w}\" height=\"{h}\" role=\"img\" aria-label=\"{title}\"></canvas>\n"));
    html.push_str("<script type=\"application/json\" id=\"anicraft-movie\">");
    html.push_str(&data);
    html.push_str("</script>\n<script>\n");
    html.push_str(PLAYER_JS);
    html.push('\n');
    html.push_str(CANVAS_JS);
    html.push_str("</script>\n</body>\n</html>\n");
    if html.len() > MAX_HTML_BYTES {
        return Err(format!("the page would be {} MB, more than the {} MB limit", html.len() >> 20, MAX_HTML_BYTES >> 20));
    }
    m.stats.data_bytes = data.len();
    Ok(HtmlExport { html, report: m.report, stats: m.stats })
}

#[cfg(test)]
#[path = "html_tests.rs"]
mod tests;
