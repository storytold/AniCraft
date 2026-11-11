//! The Movie Explorer's tree (ADR 0058): its shape for scenes and nested symbols, open rows,
//! the show filters and the text filter, folders, symbols that hold themselves, paging, jumping
//! to a row, and a large document.

use super::*;
use crate::find::tests::{Build, movie};
use anicraft_doc::{LayerKind, Paint, Rgba, SymbolKind};
use serde_json::json;

fn keys_of(t: &Tree) -> Vec<(usize, NodeKind, String)> {
    t.rows.iter().map(|r| (r.depth, r.kind, r.name.clone())).collect()
}

fn all() -> Options {
    Options { all: true, ..Options::default() }
}

#[test]
fn the_top_rows_show_scenes_their_layers_and_the_library() {
    let (s, ..) = movie();
    let t = tree(&s.doc, 0, &Options::default());
    let names: Vec<(usize, &str)> = t.rows.iter().map(|r| (r.depth, r.name.as_str())).collect();
    assert_eq!(names, [(0, "Scene 1"), (1, "Actions"), (1, "Art"), (0, "Scene 2"), (1, "Back"), (0, ""), (1, "Outer"), (1, "Inner"), (1, "Unused")]);
    assert!(t.rows[1].children && !t.rows[1].open);
    // Scene 2's only layer holds a plain shape: nothing to list under it.
    assert!(!t.rows[4].children);
    assert_eq!(t.rows[5].kind, NodeKind::Definitions);
    assert_eq!(t.rows[6].kind, NodeKind::Symbol(SymbolKind::MovieClip));
    // Scope: one scene.
    let one = tree(&s.doc, 1, &Options { scope: Scope::Scene, definitions: false, ..Options::default() });
    assert_eq!(one.rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["Scene 2", "Back"]);
}

#[test]
fn instances_open_into_their_symbols_layers_however_deep() {
    let (s, outer, inner, _) = movie();
    let t = tree(&s.doc, 0, &all());
    let scene1: Vec<(usize, NodeKind, String)> = keys_of(&t).into_iter().take_while(|r| r.2 != "Scene 2").collect();
    let want = [
        (0, NodeKind::Scene, "Scene 1"),
        (1, NodeKind::Layer(LayerKind::Normal), "Actions"),
        (2, NodeKind::Frame, ""),
        (3, NodeKind::Script, "stop(); // say hello"),
        (1, NodeKind::Layer(LayerKind::Normal), "Art"),
        (2, NodeKind::Frame, ""),
        (3, NodeKind::Text, "Hello world"),
        (3, NodeKind::Instance(SymbolKind::MovieClip), "Outer"),
        (4, NodeKind::Layer(LayerKind::Normal), "Outer art"),
        (5, NodeKind::Frame, ""),
        (6, NodeKind::Instance(SymbolKind::Graphic), "Inner"),
        (7, NodeKind::Layer(LayerKind::Normal), "Inner art"),
        (8, NodeKind::Frame, ""),
        (9, NodeKind::Text, "Hello inside"),
        (2, NodeKind::Frame, "intro hello"),
        (3, NodeKind::Text, "hello hello again"),
    ];
    let want: Vec<(usize, NodeKind, String)> = want.iter().map(|(d, k, n)| (*d, *k, n.to_string())).collect();
    assert_eq!(scene1, want);
    // The instance row names the instance too, and its frame.
    let hero = t.rows.iter().find(|r| r.extra == "hero").unwrap();
    assert_eq!((hero.frame, hero.children, hero.open), (Some(0), true, true));
    // The deepest text: reached by opening hero, then wheel, in place.
    let deep = t.rows.iter().find(|r| r.depth == 9).unwrap();
    assert_eq!(deep.go.path.len(), 2);
    assert_eq!(deep.go.scene, Some(0));
    let mut s = s;
    let v = s.execute("find.goto", &deep.go.to_json()).unwrap();
    assert_eq!(s.editing.iter().map(|l| l.symbol).collect::<Vec<_>>(), [outer, inner], "{v}");
    assert_eq!(s.selection.id_list(), [deep.go.element.unwrap().0]);
    // Under the Library's Outer: its own layers, then Inner inside it.
    let defs: Vec<(usize, String)> = t.rows.iter().skip_while(|r| r.kind != NodeKind::Definitions).map(|r| (r.depth, r.name.clone())).take(8).collect();
    assert_eq!(defs, [(0, String::new()), (1, "Outer".into()), (2, "Outer art".into()), (3, String::new()), (4, "Inner".into()), (5, "Inner art".into()), (6, String::new()), (7, "Hello inside".into())]);
    // Keys are unique, so open rows can be remembered.
    let mut keys: Vec<&str> = t.rows.iter().map(|r| r.key.as_str()).collect();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), t.rows.len());
}

#[test]
fn only_open_rows_show_their_children() {
    let (s, ..) = movie();
    let art = s.doc.timeline(0).layers[1].id.0;
    let t = tree(&s.doc, 0, &Options { expanded: Some(["s0".to_string(), format!("s0/l{art}")].into()), definitions: false, ..Options::default() });
    let names: Vec<&str> = t.rows.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["Scene 1", "Actions", "Art", "", "intro hello", "Scene 2"]);
    assert!(t.rows[3].children && !t.rows[3].open);
}

#[test]
fn show_filters_hide_kinds_and_frames_off_lists_items_straight_under_the_scene() {
    let (s, ..) = movie();
    let no_symbols = tree(&s.doc, 0, &Options { show: Show { symbols: false, ..Show::default() }, definitions: false, ..all() });
    assert!(no_symbols.rows.iter().all(|r| !matches!(r.kind, NodeKind::Instance(_))));
    assert!(!no_symbols.rows.iter().any(|r| r.name == "Hello inside"));
    let flat = tree(&s.doc, 0, &Options { show: Show { frames: false, ..Show::default() }, definitions: false, scope: Scope::Scene, ..all() });
    let names: Vec<(usize, &str)> = flat.rows.iter().map(|r| (r.depth, r.name.as_str())).collect();
    assert_eq!(names, [(0, "Scene 1"), (1, "stop(); // say hello"), (1, "Hello world"), (1, "Outer"), (2, "Inner"), (3, "Hello inside"), (1, "hello hello again")]);
    let no_scripts = tree(&s.doc, 0, &Options { show: Show { scripts: false, ..Show::default() }, ..all() });
    assert!(no_scripts.rows.iter().all(|r| r.kind != NodeKind::Script && r.name != "Actions" || !r.children));
}

#[test]
fn the_text_filter_keeps_matches_and_the_rows_above_them() {
    let (s, ..) = movie();
    let t = tree(&s.doc, 0, &Options { filter: "INSIDE".into(), ..Options::default() });
    let texts = t.rows.iter().filter(|r| r.kind == NodeKind::Text).count();
    assert_eq!(texts, 3, "{:?}", keys_of(&t));
    // Each row is a match or above one: its next row is deeper, or it matches.
    for (i, r) in t.rows.iter().enumerate() {
        let matches = r.name.to_lowercase().contains("inside");
        let parent = t.rows.get(i + 1).is_some_and(|n| n.depth > r.depth);
        assert!(matches || parent, "{r:?}");
    }
    assert!(tree(&s.doc, 0, &Options { filter: "zebra".into(), ..Options::default() }).rows.is_empty());
    // The instance name counts too.
    assert!(tree(&s.doc, 0, &Options { filter: "wheel".into(), ..Options::default() }).rows.iter().any(|r| r.extra == "wheel"));
}

#[test]
fn closed_folders_hide_the_layers_in_them() {
    let mut b = Build::new();
    let t1 = b.text("one", Rgba::BLACK);
    let t2 = b.text("two", Rgba::BLACK);
    let mut folder = b.layer("Folder", vec![(0, 1, "", vec![])]);
    folder.kind = LayerKind::Folder;
    let mut a = b.layer("A", vec![(0, 1, "", vec![t1])]);
    a.parent = Some(folder.id);
    let c = b.layer("C", vec![(0, 1, "", vec![t2])]);
    let fid = folder.id.0;
    b.scene("S", vec![folder, a, c]);
    let doc = crate::Session::new(b.doc).doc;
    let closed = tree(&doc, 0, &Options { expanded: Some(["s0".to_string()].into()), ..Options::default() });
    assert_eq!(closed.rows.iter().map(|r| (r.depth, r.name.as_str())).collect::<Vec<_>>(), [(0, "S"), (1, "Folder"), (1, "C")]);
    let open = tree(&doc, 0, &Options { expanded: Some(["s0".to_string(), format!("s0/l{fid}")].into()), ..Options::default() });
    assert_eq!(open.rows.iter().map(|r| (r.depth, r.name.as_str())).collect::<Vec<_>>(), [(0, "S"), (1, "Folder"), (2, "A"), (1, "C")]);
}

#[test]
fn symbols_that_hold_themselves_stop() {
    let mut b = Build::new();
    let a = b.symbol("A", SymbolKind::Graphic);
    let c = b.symbol("B", SymbolKind::Graphic);
    let (ia, ib, self_a) = (b.instance(c, ""), b.instance(a, ""), b.instance(a, "me"));
    let l = b.layer("in A", vec![(0, 1, "", vec![ia, self_a])]);
    b.fill(a, vec![l]);
    let l = b.layer("in B", vec![(0, 1, "", vec![ib])]);
    b.fill(c, vec![l]);
    let top = b.instance(a, "");
    let l = b.layer("L", vec![(0, 1, "", vec![top])]);
    b.scene("S", vec![l]);
    let t = tree(&b.doc, 0, &all());
    assert!(t.rows.len() < 40 && !t.truncated, "{}", t.rows.len());
    assert!(t.rows.iter().all(|r| r.depth <= MAX_DEPTH));
}

#[test]
fn the_command_pages_rows_with_where_each_goes() {
    let (mut s, ..) = movie();
    let v = s.execute("explorer.tree", &json!({"all": true, "offset": 2, "limit": 3})).unwrap();
    assert_eq!(v["rows"].as_array().map(Vec::len), Some(3));
    assert!(v["total"].as_u64().unwrap() > 20);
    let r = &v["rows"][1];
    assert_eq!((r["kind"].as_str(), r["depth"].as_u64(), r["name"].as_str()), (Some("script"), Some(3), Some("stop(); // say hello")));
    assert_eq!(r["go"]["frame"], 0);
    let v = s.execute("explorer.tree", &json!({"all": true, "show": {"text": false, "scripts": false, "media": false, "frames": false}, "definitions": false})).unwrap();
    let kinds: Vec<&str> = v["rows"].as_array().unwrap().iter().map(|r| r["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["scene", "instance", "instance", "scene"]);
    assert_eq!(v["rows"][1]["type"], "movie_clip");
    assert!(s.execute("explorer.tree", &json!({"scope": "galaxy"})).is_err());
    assert!(s.execute("explorer.tree", &json!({"expanded": [1]})).is_err());
}

#[test]
fn a_large_movie_makes_its_tree_quickly() {
    // 100 layers × 120 keyframes, each with a text and an instance of a symbol of 3 layers.
    let mut b = Build::new();
    let sym = b.symbol("Prop", SymbolKind::MovieClip);
    let inner: Vec<_> = (0..3).map(|i| {
        let r = b.rect(Paint::Solid(Rgba::BLACK), None);
        let t = b.text("label", Rgba::BLACK);
        b.layer(&format!("p{i}"), vec![(0, 1, "", vec![r, t])])
    }).collect();
    b.fill(sym, inner);
    let mut layers = Vec::new();
    for li in 0..100 {
        let keys: Vec<_> = (0..120).map(|k| (k, 1, "", vec![b.text(&format!("t {li} {k}"), Rgba::BLACK), b.instance(sym, "")])).collect();
        layers.push(b.layer(&format!("Layer {li}"), keys));
    }
    b.scene("Big", layers);
    let doc = b.doc;
    let t0 = std::time::Instant::now();
    let open = tree(&doc, 0, &all());
    let every = t0.elapsed();
    let t0 = std::time::Instant::now();
    let top = tree(&doc, 0, &Options::default());
    let closed = t0.elapsed();
    let t0 = std::time::Instant::now();
    let found = tree(&doc, 0, &Options { filter: "t 99 119".into(), ..Options::default() });
    let filtered = t0.elapsed();
    println!("Movie Explorer on 24,000 elements: everything open {every:?} ({} rows), top rows {closed:?} ({} rows), filtered {filtered:?} ({} rows)", open.rows.len(), top.rows.len(), found.rows.len());
    assert!(open.rows.len() > 100_000 && !open.truncated);
    assert_eq!(found.rows.iter().filter(|r| r.kind == NodeKind::Text).count(), 1);
    assert!(every.as_secs_f64() < 3.0 && closed.as_secs_f64() < 0.5);
}
