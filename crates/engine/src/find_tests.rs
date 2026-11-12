//! Find and Replace end to end (ADR 0058): each kind, scopes, text options and bounded patterns,
//! Replace and Replace All as one undo step, the confirmation, and going to what was found
//! (symbols opened in place through the instances that lead to them).

use super::*;
use anicraft_doc::{ColorEffect, Document, Element, ElementKind, FrameSound, GradientStop, Keyframe, Layer, LayerId, LoopMode, Paint, Rgba, Scene, Shape, SourceScript, Symbol, SymbolKind, TextAlign, TextBlock};
use anicraft_geom::{Affine, Path, Vec2};
use serde_json::{Value, json};
use std::sync::Arc;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    match s.execute(id, &p) {
        Ok(v) => v,
        Err(e) => panic!("{id} {p}: {e}"),
    }
}

fn err(s: &mut Session, id: &str, p: Value) -> String {
    match s.execute(id, &p) {
        Ok(v) => panic!("{id} {p} should fail, gave {v}"),
        Err(e) => e.to_string(),
    }
}

/// Builds documents by hand, ids from the document.
pub(crate) struct Build {
    pub doc: Document,
}

impl Build {
    pub fn new() -> Build {
        let mut doc = Document::default();
        doc.scenes.clear();
        doc.next_id = 1;
        Build { doc }
    }
    pub fn id(&mut self) -> u64 {
        self.doc.alloc_id()
    }
    pub fn text(&mut self, t: &str, color: Rgba) -> Element {
        let block = TextBlock { text: t.into(), font: "Test Sans".into(), size: 20.0, bold: false, italic: false, color, align: TextAlign::Left, letter_spacing: 0.0, line_spacing: 0.0, width: None, outline: Arc::new(Path::new()), box_width: 10.0, box_height: 10.0, field: None };
        Element { id: ElementId(self.id()), name: String::new(), transform: Affine::IDENTITY, color: ColorEffect::default(), kind: ElementKind::Text(Arc::new(block)) }
    }
    pub fn rect(&mut self, fill: Paint, stroke: Option<Rgba>) -> Element {
        let mut sh = Shape::filled(Path::rect(0.0, 0.0, 10.0, 10.0), fill);
        if let Some(c) = stroke {
            sh = sh.with_stroke(c, 2.0);
        }
        Element { id: ElementId(self.id()), name: String::new(), transform: Affine::IDENTITY, color: ColorEffect::default(), kind: ElementKind::Shape(sh) }
    }
    pub fn instance(&mut self, symbol: SymbolId, name: &str) -> Element {
        Element {
            id: ElementId(self.id()),
            name: name.into(),
            transform: Affine::translate(100.0, 50.0),
            color: ColorEffect::default(),
            kind: ElementKind::Symbol { symbol, loop_mode: LoopMode::Loop, first_frame: 0, blend: Default::default(), filters: Vec::new(), events: Vec::new() },
        }
    }
    /// A layer with keyframes `(start, duration, label, elements)`.
    pub fn layer(&mut self, name: &str, keys: Vec<(u32, u32, &str, Vec<Element>)>) -> Layer {
        let mut l = Layer::new(LayerId(self.id()), name);
        l.keyframes = keys
            .into_iter()
            .map(|(start, dur, label, els)| {
                let mut k = Keyframe::new(start, dur);
                k.label = label.into();
                k.elements = els.into_iter().map(Arc::new).collect();
                Arc::new(k)
            })
            .collect();
        l
    }
    pub fn scene(&mut self, name: &str, layers: Vec<Layer>) {
        let mut sc = Scene::new(name);
        sc.timeline.layers = layers.into_iter().map(Arc::new).collect();
        self.doc.scenes.push(Arc::new(sc));
    }
    pub fn symbol(&mut self, name: &str, kind: SymbolKind) -> SymbolId {
        let id = SymbolId(self.id());
        self.doc.library.push(Arc::new(Symbol { id, name: name.into(), kind, linkage: None, scale_grid_kept: None, scale_grid: None, timeline: Timeline::default() }));
        id
    }
    pub fn fill(&mut self, id: SymbolId, layers: Vec<Layer>) {
        if let Some(s) = self.doc.library.iter_mut().find(|s| s.id == id) {
            Arc::make_mut(s).timeline.layers = layers.into_iter().map(Arc::new).collect();
        }
    }
}

const RED: u32 = 0xff0000;

/// Scene 1: an Art layer (a text field, a red rect with a blue stroke, an instance of Outer
/// named "hero"; a second keyframe labelled "intro hello" with more text), an Actions layer with a
/// script. Outer holds an instance of Inner and a red rect; Inner holds text. Unused holds text
/// and is in no scene. Scene 2 has a red rect.
pub(crate) fn movie() -> (Session, SymbolId, SymbolId, SymbolId) {
    let mut b = Build::new();
    let outer = b.symbol("Outer", SymbolKind::MovieClip);
    let inner = b.symbol("Inner", SymbolKind::Graphic);
    let unused = b.symbol("Unused", SymbolKind::Graphic);
    let t = b.text("Hello inside", Rgba::hex(RED));
    let l = b.layer("Inner art", vec![(0, 5, "", vec![t])]);
    b.fill(inner, vec![l]);
    let i = b.instance(inner, "wheel");
    let r = b.rect(Paint::Solid(Rgba::hex(RED)), None);
    let l = b.layer("Outer art", vec![(0, 1, "", vec![r]), (3, 2, "", vec![i])]);
    b.fill(outer, vec![l]);
    let t = b.text("hello unused", Rgba::BLACK);
    let l = b.layer("U", vec![(0, 1, "", vec![t])]);
    b.fill(unused, vec![l]);
    let t1 = b.text("Hello world", Rgba::BLACK);
    let r = b.rect(Paint::Solid(Rgba::hex(RED).with_alpha(0.5)), Some(Rgba::hex(0x0000ff)));
    let hero = b.instance(outer, "hero");
    let t2 = b.text("hello hello again", Rgba::hex(RED));
    let art = b.layer("Art", vec![(0, 10, "", vec![t1, r, hero]), (10, 5, "intro hello", vec![t2])]);
    let mut actions = b.layer("Actions", vec![(0, 15, "", vec![])]);
    if let Some(k) = actions.keyframe_mut(0) {
        k.script = Some(FrameScript::Source(Arc::new(SourceScript::new(Default::default(), "stop(); // say hello"))));
    }
    b.scene("Scene 1", vec![actions, art]);
    let r = b.rect(Paint::Linear { start: Vec2::new(0.0, 0.0), end: Vec2::new(10.0, 0.0), stops: vec![GradientStop { offset: 0.0, color: Rgba::hex(RED) }, GradientStop { offset: 1.0, color: Rgba::WHITE }] }, None);
    let l = b.layer("Back", vec![(0, 1, "", vec![r])]);
    b.scene("Scene 2", vec![l]);
    (Session::new(b.doc), outer, inner, unused)
}

fn hits(s: &mut Session, p: Value) -> Vec<Value> {
    run(s, "find.search", p)["hits"].as_array().cloned().unwrap_or_default()
}

fn fields(h: &[Value]) -> Vec<String> {
    h.iter().map(|h| h["field"].as_str().unwrap_or("").to_string()).collect()
}

#[test]
fn text_is_found_in_every_place_and_scope() {
    let (mut s, ..) = movie();
    // The scene and the symbols it uses: the script, text fields (Inner's too), the label.
    let h = hits(&mut s, json!({"kind": "text", "text": "hello"}));
    let found: Vec<&str> = h.iter().map(|h| h["found"].as_str().unwrap()).collect();
    assert_eq!(fields(&h), ["script", "text", "label", "text", "text", "text"], "{h:?}");
    assert_eq!(found, ["hello", "Hello", "hello", "hello", "hello", "Hello"]);
    assert_eq!(h[1]["scene"], 0);
    assert_eq!(h[1]["layer_name"], "Art");
    assert_eq!(h[3]["start"], 0);
    assert_eq!(h[4]["start"], 6);
    assert_eq!(h[4]["context"], "hello hello again");
    assert_eq!(h[5]["symbol_name"], "Inner");
    // The whole document adds the unused symbol (and Scene 2 has none).
    assert_eq!(hits(&mut s, json!({"kind": "text", "text": "hello", "scope": "document"})).len(), 7);
    // Case, whole words and where to look.
    assert_eq!(hits(&mut s, json!({"kind": "text", "text": "Hello", "match_case": true})).len(), 2);
    assert!(hits(&mut s, json!({"kind": "text", "text": "hell", "whole_word": true})).is_empty());
    assert_eq!(hits(&mut s, json!({"kind": "text", "text": "hell"})).len(), 6);
    assert_eq!(fields(&hits(&mut s, json!({"kind": "text", "text": "hello", "in": ["labels"]}))), ["label"]);
    assert_eq!(fields(&hits(&mut s, json!({"kind": "text", "text": "her", "in": ["instances"]}))), ["instance_name"]);
    assert_eq!(fields(&hits(&mut s, json!({"kind": "text", "text": "art", "in": ["layers"]}))), ["layer_name", "layer_name", "layer_name"]);
    assert!(err(&mut s, "find.search", json!({"kind": "text", "text": "x", "in": ["fills"]})).contains("'in'"));
    assert!(err(&mut s, "find.search", json!({"kind": "text", "text": ""})).contains("find"));
    // Paging.
    let v = run(&mut s, "find.search", json!({"kind": "text", "text": "hello", "offset": 4, "limit": 1}));
    assert_eq!((v["total"].as_u64(), v["hits"].as_array().map(Vec::len)), (Some(6), Some(1)));
}

#[test]
fn patterns_are_bounded() {
    let (mut s, ..) = movie();
    let h = hits(&mut s, json!({"kind": "text", "text": r"h\w+o", "regex": true, "in": ["text"]}));
    assert_eq!(h.len(), 4);
    assert!(err(&mut s, "find.search", json!({"kind": "text", "text": "(", "regex": true})).contains("pattern"));
    // Too large to compile is refused, not run.
    assert!(err(&mut s, "find.search", json!({"kind": "text", "text": "a{1000}{1000}", "regex": true})).contains("too large"));
    // What backtracking engines take exponential time over runs in linear time.
    let mut b = Build::new();
    let t = b.text(&format!("{}b", "a".repeat(50_000)), Rgba::BLACK);
    let l = b.layer("L", vec![(0, 1, "", vec![t])]);
    b.scene("S", vec![l]);
    let mut big = Session::new(b.doc);
    let t0 = std::time::Instant::now();
    assert_eq!(hits(&mut big, json!({"kind": "text", "text": "(a+)+$", "regex": true})).len(), 0);
    assert!(t0.elapsed().as_secs_f64() < 2.0, "{:?}", t0.elapsed());
    // An empty match finds nothing, rather than every gap.
    assert!(hits(&mut s, json!({"kind": "text", "text": "x*", "regex": true})).is_empty());
}

#[test]
fn colours_fonts_and_library_items_are_found() {
    let (mut s, outer, inner, _) = movie();
    // Red: a text colour (Scene 1's second text), a fill with alpha, Outer's fill, Inner's text.
    let h = hits(&mut s, json!({"kind": "color", "color": "#FF0000"}));
    assert_eq!(fields(&h), ["fill", "text_color", "fill", "text_color"], "{h:?}");
    assert_eq!(fields(&hits(&mut s, json!({"kind": "color", "color": "#0000ff"}))), ["stroke"]);
    assert_eq!(fields(&hits(&mut s, json!({"kind": "color", "color": "#ff0000", "in": ["strokes"]}))).len(), 0);
    // Gradient stops count (Scene 2).
    assert_eq!(hits(&mut s, json!({"kind": "color", "color": "#ff0000", "in": ["fills"], "scope": "document"})).len(), 3);
    assert_eq!(hits(&mut s, json!({"kind": "font", "font": " test sans "})).len(), 3);
    let h = hits(&mut s, json!({"kind": "symbol", "symbol": inner.0}));
    assert_eq!((h.len(), h[0]["symbol"].as_u64(), h[0]["found"].as_str()), (1, Some(outer.0), Some("Inner")));
    assert!(err(&mut s, "find.search", json!({"kind": "symbol", "symbol": 999_999})).contains("no symbol"));
    assert!(err(&mut s, "find.search", json!({"kind": "sparkle"})).contains("unknown kind"));
}

#[test]
fn sounds_and_bitmaps_are_found_and_replaced() {
    let (mut s, ..) = movie();
    let wav = crate::cmds_sound::test_wav(8000, &[0.25; 800]);
    let (a, b) = (anicraft_doc::SoundId(s.doc.alloc_id()), anicraft_doc::SoundId(s.doc.alloc_id()));
    for (id, n) in [(a, "a.wav"), (b, "b.wav")] {
        s.doc.sounds.push(Arc::new(anicraft_doc::Sound::from_data(id, n, wav.clone()).unwrap()));
    }
    let k = Arc::make_mut(&mut s.doc.timeline_mut(0).layers[0]).keyframe_mut(0).unwrap();
    k.sound = Some(FrameSound::new(a));
    let h = hits(&mut s, json!({"kind": "sound", "sound": a.0}));
    assert_eq!((fields(&h), h[0]["found"].as_str()), (vec!["sound".to_string()], Some("a.wav")));
    run(&mut s, "find.replace", json!({"kind": "sound", "sound": a.0, "with": b.0}));
    assert_eq!(s.doc.timeline(0).layers[0].keyframes[0].sound.as_ref().map(|f| f.sound), Some(b));
    // A placed bitmap takes the new one's size.
    let png = |w, h| anicraft_render::encode_png(w, h, &[0, 0, 255, 255].repeat((w * h) as usize)).unwrap();
    let (p, q) = (anicraft_doc::BitmapId(s.doc.alloc_id()), anicraft_doc::BitmapId(s.doc.alloc_id()));
    s.doc.bitmaps.push(Arc::new(anicraft_doc::Bitmap::from_png(p, "p.png", png(3, 2)).unwrap()));
    s.doc.bitmaps.push(Arc::new(anicraft_doc::Bitmap::from_png(q, "q.png", png(5, 4)).unwrap()));
    run(&mut s, "library.place", json!({"bitmap": p.0}));
    assert_eq!(hits(&mut s, json!({"kind": "bitmap", "bitmap": p.0})).len(), 1);
    run(&mut s, "find.replace", json!({"kind": "bitmap", "bitmap": p.0, "with": q.0}));
    assert!(hits(&mut s, json!({"kind": "bitmap", "bitmap": p.0})).is_empty());
    let placed = s.doc.timeline(0).layers.iter().flat_map(|l| &l.keyframes).flat_map(|k| &k.elements).find_map(|e| match &e.kind {
        ElementKind::Shape(sh) if matches!(sh.fill, Some(Paint::Bitmap { bitmap, .. }) if bitmap == q) => Some(sh.path.bounds()),
        _ => None,
    });
    assert_eq!(placed.map(|r| (r.width(), r.height())), Some((5.0, 4.0)));
}

#[test]
fn replace_all_is_one_undo_step_and_undo_brings_everything_back() {
    let (mut s, ..) = movie();
    let before = s.doc.clone();
    let steps = s.history_len();
    let v = run(&mut s, "find.replace", json!({"kind": "text", "text": "hello", "with": "bye", "in": ["text", "labels", "scripts"]}));
    assert_eq!(v["replaced"], 5, "{v}");
    assert_eq!(s.history_len(), steps + 1);
    let texts: Vec<String> = s.doc.timelines().flat_map(|t| &t.layers).flat_map(|l| &l.keyframes).flat_map(|k| &k.elements).filter_map(|e| match &e.kind {
        ElementKind::Text(t) => Some(t.text.clone()),
        _ => None,
    }).collect();
    // Case-insensitive, every match; the unused symbol is outside the scene's scope.
    assert_eq!(texts, ["bye world", "bye bye again", "bye inside", "hello unused"]);
    assert_eq!(s.doc.timeline(0).layers[1].keyframes[1].label, "intro bye");
    match &s.doc.timeline(0).layers[0].keyframes[0].script {
        Some(FrameScript::Source(src)) => assert_eq!(src.text, "stop(); // say bye"),
        other => panic!("{other:?}"),
    }
    // A replaced text is laid out again (its outline was empty).
    assert!(s.doc.timeline(0).layers[1].keyframes[1].elements.iter().any(|e| matches!(&e.kind, ElementKind::Text(t) if t.text == "bye bye again" && t.box_width != 10.0)));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc, before);
    // Nothing to replace is an error, and records nothing.
    assert!(err(&mut s, "find.replace", json!({"kind": "text", "text": "zebra", "with": "x"})).contains("nothing"));
    assert_eq!(s.history_len(), steps);
}

#[test]
fn replace_one_changes_only_that_match() {
    let (mut s, ..) = movie();
    let h = hits(&mut s, json!({"kind": "text", "text": "hello", "in": ["text"]}));
    let second = h.iter().find(|h| h["start"] == 6).cloned().unwrap();
    run(&mut s, "find.replace", json!({"kind": "text", "text": "hello", "in": ["text"], "with": "HI", "at": second}));
    let t = s.doc.timeline(0).layers[1].keyframes[1].elements.iter().find_map(|e| match &e.kind {
        ElementKind::Text(t) => Some(t.text.clone()),
        _ => None,
    });
    assert_eq!(t.as_deref(), Some("hello HI again"));
    // The same hit again is gone.
    assert!(err(&mut s, "find.replace", json!({"kind": "text", "text": "hello", "in": ["text"], "with": "HI", "at": second})).contains("isn't there"));
    // Groups expand with a pattern, not with plain text.
    run(&mut s, "find.replace", json!({"kind": "text", "text": r"(\w+) world", "regex": true, "with": "$1, $1 world"}));
    let first = s.doc.timeline(0).layers[1].keyframes[0].elements.iter().find_map(|e| match &e.kind {
        ElementKind::Text(t) => Some(t.text.clone()),
        _ => None,
    });
    assert_eq!(first.as_deref(), Some("Hello, Hello world"));
    run(&mut s, "find.replace", json!({"kind": "text", "text": "world", "with": "$0$1"}));
    assert!(s.doc.timeline(0).layers[1].keyframes[0].elements.iter().any(|e| matches!(&e.kind, ElementKind::Text(t) if t.text == "Hello, Hello $0$1")));
    // A layer name can't become empty: skipped, and said why.
    assert!(err(&mut s, "find.replace", json!({"kind": "text", "text": "Actions", "in": ["layers"], "with": ""})).contains("can't be empty"));
}

#[test]
fn colours_keep_alpha_and_fonts_lay_out_again() {
    let (mut s, ..) = movie();
    let v = run(&mut s, "find.replace", json!({"kind": "color", "color": "#ff0000", "with": "#00ff00", "scope": "document"}));
    assert_eq!(v["replaced"], 5, "{v}");
    assert!(hits(&mut s, json!({"kind": "color", "color": "#ff0000", "scope": "document"})).is_empty());
    let r = &s.doc.timeline(0).layers[1].keyframes[0].elements[1];
    match &r.kind {
        ElementKind::Shape(sh) => assert_eq!(sh.fill, Some(Paint::Solid(Rgba::hex(0x00ff00).with_alpha(0.5)))),
        other => panic!("{other:?}"),
    }
    match &s.doc.timeline(1).layers[0].keyframes[0].elements[0].kind {
        ElementKind::Shape(sh) => assert!(matches!(&sh.fill, Some(Paint::Linear { stops, .. }) if stops[0].color == Rgba::hex(0x00ff00) && stops[1].color == Rgba::WHITE)),
        other => panic!("{other:?}"),
    }
    let v = run(&mut s, "find.replace", json!({"kind": "font", "font": "Test Sans", "with": "Inter"}));
    assert_eq!(v["replaced"], 3);
    assert!(hits(&mut s, json!({"kind": "font", "font": "test sans"})).is_empty());
}

#[test]
fn a_symbol_that_would_contain_itself_is_skipped() {
    let (mut s, outer, inner, unused) = movie();
    // Inner's instance is inside Outer: swapping it for Outer would nest Outer in itself.
    assert!(err(&mut s, "find.replace", json!({"kind": "symbol", "symbol": inner.0, "with": outer.0})).contains("contain itself"));
    // Into a symbol that doesn't hold the place: fine (and the scene's Outer becomes Unused).
    let v = run(&mut s, "find.replace", json!({"kind": "symbol", "symbol": outer.0, "with": unused.0}));
    assert_eq!(v["replaced"], 1);
    let v = run(&mut s, "find.replace", json!({"kind": "symbol", "symbol": inner.0, "with": unused.0, "scope": "document"}));
    assert_eq!(v["replaced"], 1);
}

#[test]
fn replace_all_over_many_places_asks_first() {
    let mut b = Build::new();
    let els: Vec<Element> = (0..(REPLACE_ASK + 1)).map(|_| b.rect(Paint::Solid(Rgba::hex(RED)), None)).collect();
    let l = b.layer("L", vec![(0, 1, "", els)]);
    b.scene("S", vec![l]);
    let mut s = Session::new(b.doc);
    let p = json!({"kind": "color", "color": "#ff0000", "with": "#000000"});
    let c = s.confirmation("find.replace", &p).expect("asks above the limit");
    assert!(!c.irreversible && c.message.contains("101 places"), "{c:?}");
    // One place (Replace) never asks.
    let one = hits(&mut s, p.clone()).remove(0);
    let mut p1 = p.clone();
    p1["at"] = one;
    assert!(s.confirmation("find.replace", &p1).is_none());
    // Undoable, so a script runs it without "confirm".
    assert_eq!(run(&mut s, "find.replace", p)["replaced"], 101);
}

#[test]
fn going_to_a_hit_opens_the_symbols_in_place_and_selects_it() {
    let (mut s, outer, inner, unused) = movie();
    // Inner's text: through hero (Outer) and wheel (Inner), in place.
    let h = hits(&mut s, json!({"kind": "text", "text": "inside"})).remove(0);
    let v = run(&mut s, "find.goto", h.clone());
    let open: Vec<SymbolId> = s.editing.iter().map(|l| l.symbol).collect();
    assert_eq!(open, [outer, inner], "{v}");
    assert!(s.editing.iter().all(|l| l.instance.is_some()));
    assert_eq!(s.selection.id_list(), [h["element"].as_u64().unwrap()]);
    assert_eq!(s.edit_view().map(|v| v.in_place), Some(true));
    // A symbol no scene uses opens alone, from the Library.
    let h = hits(&mut s, json!({"kind": "text", "text": "unused", "scope": "document"})).remove(0);
    run(&mut s, "find.goto", h.clone());
    assert_eq!((s.editing.len(), s.editing_symbol(), s.editing[0].instance), (1, Some(unused), None));
    assert_eq!(s.selection.id_list(), [h["element"].as_u64().unwrap()]);
    // Another scene: the scene switches, the frame is the keyframe's.
    let h = hits(&mut s, json!({"kind": "color", "color": "#ff0000", "scope": "document"}));
    let back = h.iter().find(|h| h["scene"] == 1).cloned().unwrap();
    run(&mut s, "find.goto", back);
    assert_eq!((s.scene, s.editing.len(), s.selection.len()), (1, 0, 1));
    // A label: the keyframe, nothing selected.
    let h = hits(&mut s, json!({"kind": "text", "text": "intro", "scope": "document"})).remove(0);
    run(&mut s, "find.goto", h);
    assert_eq!((s.scene, s.frame, s.selection.len()), (0, 10, 0));
    assert_eq!(s.active_layer().map(|l| l.name.as_str()), Some("Art"));
    // Going somewhere records nothing to undo.
    assert!(!s.can_undo());
    assert!(err(&mut s, "find.goto", json!({})).contains("where"));
    assert!(err(&mut s, "find.goto", json!({"layer": 999_999})).contains("no layer"));
}

#[test]
fn going_to_a_symbol_behind_a_locked_layer_opens_it_alone() {
    let (mut s, outer, ..) = movie();
    let art = s.doc.timeline(0).layers[1].id.0;
    run(&mut s, "layer.toggle_lock", json!({"layer": art}));
    let h = hits(&mut s, json!({"kind": "color", "color": "#ff0000", "in": ["fills"]}));
    let inside = h.iter().find(|h| h["symbol"] == outer.0).cloned().unwrap();
    run(&mut s, "find.goto", inside);
    assert_eq!((s.editing_symbol(), s.editing[0].instance), (Some(outer), None));
}

#[test]
fn search_and_replace_are_fast_on_a_large_document() {
    // 40 layers × 300 keyframes (12,000 text fields and 12,000 shapes), 500 symbols.
    let mut b = Build::new();
    let mut layers = Vec::new();
    for li in 0..40 {
        let keys: Vec<(u32, u32, &str, Vec<Element>)> = (0..300).map(|k| (k, 1, "", vec![b.text(&format!("caption {li} {k} needle"), Rgba::BLACK), b.rect(Paint::Solid(Rgba::hex(RED)), None)])).collect();
        layers.push(b.layer(&format!("Layer {li}"), keys));
    }
    b.scene("Big", layers);
    for i in 0..500 {
        let id = b.symbol(&format!("S{i}"), SymbolKind::Graphic);
        let r = b.rect(Paint::Solid(Rgba::hex(RED)), None);
        let l = b.layer("x", vec![(0, 1, "", vec![r])]);
        b.fill(id, vec![l]);
    }
    let mut s = Session::new(b.doc);
    let t0 = std::time::Instant::now();
    let n = run(&mut s, "find.search", json!({"kind": "text", "text": "needle", "whole_word": true, "limit": 1}))["total"].as_u64();
    let text = t0.elapsed();
    let t0 = std::time::Instant::now();
    let c = run(&mut s, "find.search", json!({"kind": "color", "color": "#ff0000", "scope": "document", "limit": 1}))["total"].as_u64();
    let colour = t0.elapsed();
    let t0 = std::time::Instant::now();
    let r = run(&mut s, "find.replace", json!({"kind": "color", "color": "#ff0000", "with": "#00ff00", "scope": "document"}))["replaced"].as_u64();
    let replace = t0.elapsed();
    let t0 = std::time::Instant::now();
    run(&mut s, "edit.undo", json!({}));
    let undo = t0.elapsed();
    println!("find on 24,000 elements: text {text:?}, colour {colour:?}; Replace All of 12,500 colours {replace:?}, its undo {undo:?}");
    assert_eq!((n, c, r), (Some(12_000), Some(12_500), Some(12_500)));
    assert!(text.as_secs_f64() < 2.0 && colour.as_secs_f64() < 2.0 && replace.as_secs_f64() < 5.0);
}

#[test]
fn random_queries_never_panic() {
    let (s0, outer, inner, _) = movie();
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let atoms = ["a", "(", ")", "*", "+", "?", "|", "\\b", "\\w", "[", "]", "{2,}", ".", "^", "$", "hello", "é", "\\", "$1", "${x}", "\n"];
    let kinds = ["text", "font", "color", "symbol", "sound", "bitmap", "x"];
    let mut s = s0;
    for _ in 0..400 {
        let pat: String = (0..(next() % 6)).map(|_| atoms[(next() % atoms.len() as u64) as usize]).collect();
        let with: String = (0..(next() % 4)).map(|_| atoms[(next() % atoms.len() as u64) as usize]).collect();
        let kind = kinds[(next() % kinds.len() as u64) as usize];
        let id = [outer.0, inner.0, next()][(next() % 3) as usize];
        let p = json!({"kind": kind, "text": pat, "regex": next() % 2 == 0, "whole_word": next() % 3 == 0, "match_case": next() % 2 == 0, "font": pat, "color": format!("#{:06x}", next() % 0xffffff), "symbol": id, "sound": id, "bitmap": id, "with": if next() % 2 == 0 { json!(with) } else { json!(id) }, "scope": if next() % 2 == 0 { "scene" } else { "document" }});
        if let Ok(v) = s.execute("find.search", &p)
            && let Some(h) = v["hits"].as_array().and_then(|h| h.first())
        {
            let _ = s.execute("find.goto", h);
            let mut one = p.clone();
            one["at"] = h.clone();
            let _ = s.execute("find.replace", &one);
        }
        let _ = s.execute("find.replace", &p);
        if next() % 7 == 0 {
            let _ = s.execute("edit.undo", &json!({}));
        }
    }
    let _ = anicraft_render::render_frame(&s.doc, 0, 0.1);
}

#[test]
fn the_new_namespaces_are_open_to_plug_ins() {
    for id in ["find.search", "find.replace", "find.goto", "explorer.tree"] {
        assert!(crate::cmds_plugin::plugin_may_call(id).is_ok(), "{id}");
    }
}
