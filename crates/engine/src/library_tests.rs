//! Library management end to end (ADR 0050): folders, names, moves, duplicates, Swap Symbol,
//! deletion with its confirmations, unused items, smoothing, the file format and undo.

use crate::Session;
use anicraft_doc::{BitmapId, ElementKind, FolderId, LibraryEntry, LibraryItem, SymbolId, sample};
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

fn sym(s: &Session, name: &str) -> u64 {
    s.doc.library.iter().find(|x| x.name == name).map(|x| x.id.0).unwrap_or_else(|| panic!("no symbol {name}"))
}

fn folder(v: &Value) -> u64 {
    v["folder"].as_u64().unwrap()
}

fn path(s: &Session, symbol: u64) -> String {
    s.doc.entry_path(LibraryEntry::Item(LibraryItem::Symbol(SymbolId(symbol))))
}

/// Instances of `symbol` on every timeline.
fn instances(s: &Session, symbol: u64) -> usize {
    s.doc.timelines().flat_map(|t| &t.layers).flat_map(|l| &l.keyframes).flat_map(|k| &k.elements).filter(|e| matches!(e.kind, ElementKind::Symbol { symbol: x, .. } if x.0 == symbol)).count()
}

#[test]
fn folders_are_made_named_and_nested() {
    let mut s = Session::new(sample::demo());
    let a = run(&mut s, "library.new_folder", json!({}));
    assert_eq!(a["name"], "Folder 1");
    let b = run(&mut s, "library.new_folder", json!({}));
    assert_eq!(b["name"], "Folder 2");
    let inner = run(&mut s, "library.new_folder", json!({"name": "Hero", "parent": folder(&a)}));
    assert_eq!(s.doc.entry_path(LibraryEntry::Folder(FolderId(folder(&inner)))), "Folder 1/Hero");
    // Names: unique per folder without case, no slashes, not empty.
    assert!(err(&mut s, "library.new_folder", json!({"name": "folder 1"})).contains("already has"));
    assert!(err(&mut s, "library.new_folder", json!({"name": "a/b"})).contains("/"));
    assert!(err(&mut s, "library.new_folder", json!({"name": "  "})).contains("empty"));
    assert!(err(&mut s, "library.new_folder", json!({"parent": 999_999})).contains("no folder"));
    run(&mut s, "library.new_folder", json!({"name": "Hero"}));
    // Renaming.
    run(&mut s, "library.rename", json!({"folder": folder(&a), "name": "Characters"}));
    assert_eq!(s.doc.folder(FolderId(folder(&a))).unwrap().name, "Characters");
    let ball = sym(&s, "Ball");
    assert!(err(&mut s, "library.rename", json!({"symbol": ball, "name": "star"})).contains("already has"));
    run(&mut s, "library.rename", json!({"symbol": ball, "name": "BALL"}));
    run(&mut s, "library.rename", json!({"symbol": ball, "name": "Ball"}));
    assert!(err(&mut s, "library.rename", json!({"symbol": ball, "folder": 1, "name": "x"})).contains("one Library entry"));
    // One undo step each.
    s.undo().unwrap();
    assert_eq!(s.doc.symbol(SymbolId(ball)).unwrap().name, "BALL");
}

#[test]
fn items_move_in_and_out_of_folders() {
    let mut s = Session::new(sample::demo());
    let (ball, star) = (sym(&s, "Ball"), sym(&s, "Star"));
    let chars = folder(&run(&mut s, "library.new_folder", json!({"name": "Characters"})));
    let hero = folder(&run(&mut s, "library.new_folder", json!({"name": "Hero", "parent": chars})));
    let v = run(&mut s, "library.move", json!({"items": [{"symbol": ball}, {"symbol": star}], "to": hero}));
    assert_eq!(v["moved"], 2);
    assert_eq!(path(&s, ball), "Characters/Hero/Ball");
    // Out again, one step for the whole move.
    run(&mut s, "library.move", json!({"items": [{"symbol": ball}], "to": null}));
    assert_eq!(path(&s, ball), "Ball");
    s.undo().unwrap();
    assert_eq!(path(&s, ball), "Characters/Hero/Ball");
    // A folder moves with what it holds, but never into itself.
    let props = folder(&run(&mut s, "library.new_folder", json!({"name": "Props"})));
    run(&mut s, "library.move", json!({"items": [{"folder": hero}], "to": props}));
    assert_eq!(path(&s, star), "Props/Hero/Star");
    assert!(err(&mut s, "library.move", json!({"items": [{"folder": props}], "to": hero})).contains("inside itself"));
    assert!(err(&mut s, "library.move", json!({"items": [{"folder": props}], "to": props})).contains("inside itself"));
    // A name clash stops the move and leaves everything where it was.
    run(&mut s, "library.move", json!({"items": [{"symbol": ball}], "to": null}));
    let star2 = run(&mut s, "library.duplicate", json!({"symbol": star, "name": "Ball"}));
    let before = s.doc.library_tree.clone();
    let e = err(&mut s, "library.move", json!({"items": [{"symbol": star}, {"symbol": star2["symbol"]}], "to": null}));
    assert!(e.contains("already has an item named “Ball”"), "{e}");
    assert_eq!(s.doc.library_tree, before);
    assert!(err(&mut s, "library.move", json!({"items": [{"symbol": ball}]})).contains("'to'"));
    assert!(err(&mut s, "library.move", json!({"items": [{"symbol": 424_242}], "to": null})).contains("no symbol"));
}

#[test]
fn folders_survive_save_and_open() {
    let mut s = Session::new(sample::demo());
    let ball = sym(&s, "Ball");
    let f = folder(&run(&mut s, "library.new_folder", json!({"name": "Characters"})));
    run(&mut s, "library.move", json!({"items": [{"symbol": ball}], "to": f}));
    let text = crate::format::to_json(&s.doc).unwrap();
    let back = Session::new(crate::format::from_json(&text).unwrap());
    assert_eq!(path(&back, ball), "Characters/Ball");
    assert_eq!(back.doc.library_tree, s.doc.library_tree);
    assert!(back.doc.next_id > f);
}

#[test]
fn duplicates_are_named_after_their_original() {
    let mut s = Session::new(sample::demo());
    let ball = sym(&s, "Ball");
    let f = folder(&run(&mut s, "library.new_folder", json!({"name": "Props"})));
    run(&mut s, "library.move", json!({"items": [{"symbol": ball}], "to": f}));
    let a = run(&mut s, "library.duplicate", json!({"symbol": ball}));
    assert_eq!(a["name"], "Ball copy");
    let b = run(&mut s, "library.duplicate", json!({"symbol": ball}));
    assert_eq!(b["name"], "Ball copy 2");
    assert_eq!(path(&s, a["symbol"].as_u64().unwrap()), "Props/Ball copy");
    assert!(err(&mut s, "library.duplicate", json!({"symbol": ball, "name": "ball COPY"})).contains("already has"));
    assert!(err(&mut s, "library.duplicate", json!({"symbol": 424_242})).contains("no symbol"));
    assert_eq!(instances(&s, a["symbol"].as_u64().unwrap()), 0, "a copy isn't placed");
    s.undo().unwrap();
    assert!(s.doc.symbol(SymbolId(b["symbol"].as_u64().unwrap())).is_none());
}

#[test]
fn swap_symbol_keeps_everything_but_the_symbol() {
    let mut s = Session::new(sample::demo());
    let (ball, star) = (sym(&s, "Ball"), sym(&s, "Star"));
    let el = run(&mut s, "library.place", json!({"symbol": ball, "x": 100, "y": 50}))["element"].as_u64().unwrap();
    run(&mut s, "symbol.set_kind", json!({"symbol": ball, "kind": "movie_clip"}));
    run(&mut s, "element.set_transform", json!({"sx": 2, "rotation": 30}));
    run(&mut s, "element.set_alpha", json!({"alpha": 0.5}));
    run(&mut s, "element.set_filters", json!({"filters": [{"type": "blur"}]}));
    run(&mut s, "element.rename", json!({"name": "hero"}));
    let find = |s: &Session| s.elements_on(s.layer).iter().find(|e| e.id.0 == el).map(|e| (**e).clone()).unwrap();
    let before = find(&s);
    let v = run(&mut s, "element.swap_symbol", json!({"symbol": star}));
    assert_eq!(v["swapped"], 1);
    let after = find(&s);
    assert_eq!((after.transform, after.color, &after.name), (before.transform, before.color, &before.name));
    match (&before.kind, &after.kind) {
        (ElementKind::Symbol { symbol: a, loop_mode: la, filters: fa, blend: ba, .. }, ElementKind::Symbol { symbol: b, loop_mode: lb, filters: fb, blend: bb, .. }) => {
            assert_eq!((a.0, b.0), (ball, star));
            assert_eq!((la, fa, ba), (lb, fb, bb));
        }
        _ => panic!("still an instance"),
    }
    assert_eq!(s.selection.on(s.layer).first().map(|e| e.0), Some(el), "it stays selected");
    assert!(err(&mut s, "element.swap_symbol", json!({"symbol": 424_242})).contains("no symbol"));
    assert!(err(&mut s, "element.swap_symbol", json!({})).contains("'symbol'"));
    run(&mut s, "edit.deselect", json!({}));
    assert!(s.execute("element.swap_symbol", &json!({"symbol": ball})).is_err(), "nothing selected");
    s.undo().unwrap();
    assert!(matches!(find(&s).kind, ElementKind::Symbol { symbol, .. } if symbol.0 == ball));
}

#[test]
fn deleting_asks_when_it_takes_uses_or_folder_contents() {
    let mut s = Session::new(sample::demo());
    let (ball, star) = (sym(&s, "Ball"), sym(&s, "Star"));
    assert!(instances(&s, star) > 0, "the sample places Star (inside Ball too)");
    // An unused item goes without asking.
    let copy = run(&mut s, "library.duplicate", json!({"symbol": star}))["symbol"].as_u64().unwrap();
    let p = json!({"items": [{"symbol": copy}]});
    assert!(s.confirmation("library.delete", &p).is_none());
    run(&mut s, "library.delete", p);
    assert!(s.doc.symbol(SymbolId(copy)).is_none());
    // A used one asks (undoable tier), and its instances go everywhere, inside symbols too.
    let p = json!({"items": [{"symbol": star}]});
    let c = s.confirmation("library.delete", &p).expect("asks");
    assert!(!c.irreversible && c.message.contains("used") && c.message.contains("undo"), "{c:?}");
    run(&mut s, "library.delete", p);
    assert_eq!(instances(&s, star), 0);
    assert!(s.doc.symbol(SymbolId(ball)).is_some());
    s.undo().unwrap();
    assert!(instances(&s, star) > 0);
    // A folder that holds items asks, and takes them with it.
    let f = folder(&run(&mut s, "library.new_folder", json!({"name": "Old"})));
    let inner = folder(&run(&mut s, "library.new_folder", json!({"name": "Older", "parent": f})));
    let spare = run(&mut s, "library.duplicate", json!({"symbol": ball}))["symbol"].as_u64().unwrap();
    run(&mut s, "library.move", json!({"items": [{"symbol": spare}], "to": inner}));
    let p = json!({"items": [{"folder": f}]});
    let c = s.confirmation("library.delete", &p).expect("asks");
    assert!(c.title.contains("folder") && c.message.contains("holds 2 items"), "{c:?}");
    let v = run(&mut s, "library.delete", p);
    assert_eq!((v["deleted"].as_u64(), v["folders"].as_u64()), (Some(1), Some(2)));
    assert!(s.doc.library_tree.is_empty() && s.doc.symbol(SymbolId(spare)).is_none());
    // An empty folder goes without asking.
    let empty = folder(&run(&mut s, "library.new_folder", json!({})));
    assert!(s.confirmation("library.delete", &json!({"items": [{"folder": empty}]})).is_none());
    assert!(err(&mut s, "library.delete", json!({"items": []})).contains("nothing"));
    assert!(err(&mut s, "library.delete", json!({"items": [{"folder": 1}]})).contains("no folder"));
}

#[test]
fn deleting_a_bitmap_or_a_sound_takes_its_uses() {
    let mut s = Session::new(sample::demo());
    let id = BitmapId(s.doc.alloc_id());
    let mut px = Vec::new();
    for _ in 0..4 {
        px.extend_from_slice(&[255, 0, 0, 255]);
    }
    let bytes = anicraft_render::encode_png(2, 2, &px).unwrap();
    s.doc.bitmaps.push(Arc::new(anicraft_doc::Bitmap::from_png(id, "red.png", bytes).unwrap()));
    let el = run(&mut s, "library.place", json!({"bitmap": id.0}))["element"].as_u64().unwrap();
    let item = LibraryItem::Bitmap(id);
    assert_eq!(s.doc.library_uses().get(&item), Some(&1));
    // Smoothing toggles, is one undo step, and is kept by the file.
    assert_eq!(run(&mut s, "library.set_smoothing", json!({"bitmap": id.0}))["smoothing"], false);
    let back = crate::format::from_json(&crate::format::to_json(&s.doc).unwrap()).unwrap();
    assert!(!back.bitmap(id).unwrap().smoothing);
    s.undo().unwrap();
    assert!(s.doc.bitmap(id).unwrap().smoothing);
    assert!(!crate::format::to_json(&s.doc).unwrap().contains("smoothing"), "the default is written as nothing");
    run(&mut s, "library.delete", json!({"items": [{"bitmap": id.0}]}));
    assert!(s.doc.bitmap(id).is_none());
    assert!(s.elements_on(s.layer).iter().all(|e| e.id.0 != el), "the placed bitmap goes with it");
    assert!(err(&mut s, "library.set_smoothing", json!({"bitmap": id.0})).contains("no bitmap"));
}

#[test]
fn unused_items_are_listed_and_deleted_after_asking() {
    let mut s = Session::new(sample::demo());
    let star = sym(&s, "Star");
    assert_eq!(run(&mut s, "library.unused", json!({}))["items"], json!([]));
    assert!(s.confirmation("library.delete_unused", &json!({})).is_none());
    let a = run(&mut s, "library.duplicate", json!({"symbol": star}))["symbol"].as_u64().unwrap();
    let b = run(&mut s, "library.duplicate", json!({"symbol": star}))["symbol"].as_u64().unwrap();
    // In Library order: each copy sits right after its original, so the later one comes first.
    assert_eq!(run(&mut s, "library.unused", json!({}))["items"], json!([{"symbol": b}, {"symbol": a}]));
    let c = s.confirmation("library.delete_unused", &json!({})).expect("asks");
    assert!(c.message.contains("these 2 items") && c.message.contains("Star copy"), "{c:?}");
    let v = run(&mut s, "library.delete_unused", json!({}));
    assert_eq!(v["deleted"], 2);
    assert!(s.doc.symbol(SymbolId(a)).is_none() && s.doc.symbol(SymbolId(star)).is_some());
    s.undo().unwrap();
    assert!(s.doc.symbol(SymbolId(a)).is_some());
}

#[test]
fn the_listing_has_paths_kinds_and_uses() {
    let mut s = Session::new(sample::demo());
    let ball = sym(&s, "Ball");
    let f = folder(&run(&mut s, "library.new_folder", json!({"name": "Characters"})));
    run(&mut s, "library.move", json!({"items": [{"symbol": ball}], "to": f}));
    let v = run(&mut s, "library.items", json!({}));
    assert_eq!(v["folders"], json!([{"folder": f, "name": "Characters", "parent": null, "path": "Characters"}]));
    let b = v["items"].as_array().unwrap().iter().find(|i| i["symbol"] == ball).unwrap();
    assert_eq!((b["path"].as_str(), b["folder"].as_u64(), b["kind"].as_str(), b["unused"].as_bool()), (Some("Characters/Ball"), Some(f), Some("graphic"), Some(false)));
    assert!(b["uses"].as_u64().unwrap() > 0);
    assert_eq!(v["unused"], 0);
}

#[test]
fn convert_to_symbol_picks_a_free_name() {
    let mut s = Session::default();
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 10, "h": 10}));
    let first = run(&mut s, "modify.convert_to_symbol", json!({}));
    assert_eq!(first["name"], "Symbol 1");
    run(&mut s, "library.rename", json!({"symbol": first["symbol"], "name": "Symbol 2"}));
    run(&mut s, "draw.rect", json!({"x": 20, "y": 0, "w": 10, "h": 10}));
    let second = run(&mut s, "modify.convert_to_symbol", json!({}));
    assert_eq!(second["name"], "Symbol 3", "“Symbol 2” is taken");
}

#[test]
fn inside_an_open_symbol_swap_never_nests_and_deleting_it_closes_it() {
    let mut s = Session::default();
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 10, "h": 10}));
    let outer = run(&mut s, "modify.convert_to_symbol", json!({"name": "Outer"}))["symbol"].as_u64().unwrap();
    run(&mut s, "draw.rect", json!({"x": 40, "y": 0, "w": 10, "h": 10}));
    let other = run(&mut s, "modify.convert_to_symbol", json!({"name": "Other"}))["symbol"].as_u64().unwrap();
    run(&mut s, "symbol.edit", json!({"symbol": outer}));
    assert_eq!(s.editing.len(), 1);
    run(&mut s, "library.place", json!({"symbol": other}));
    assert!(err(&mut s, "element.swap_symbol", json!({"symbol": outer})).contains("inside itself"));
    // Deleting the open symbol takes the editor back to the scene.
    run(&mut s, "library.delete", json!({"items": [{"symbol": outer}], "confirm": true}));
    assert!(s.editing.is_empty() && s.doc.symbol(SymbolId(outer)).is_none());
}

#[test]
fn library_counts_stay_quick_on_a_large_document() {
    // 100 layers × 100 shapes keyed every 12 frames over 240 frames (200,000 element keys),
    // plus 1,000 symbols in 50 folders, each placed once. Measured, loosely bounded (CI varies);
    // the shell works these out once per document change, not per frame.
    let mut s = Session::new(anicraft_doc::sample::stress(100, 100, 240));
    // A new layer, so the symbol holds only the new rectangle (drawing selects it).
    run(&mut s, "layer.new", json!({}));
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 5, "h": 5}));
    let sym = run(&mut s, "modify.convert_to_symbol", json!({}))["symbol"].as_u64().unwrap();
    for i in 0..50 {
        run(&mut s, "library.new_folder", json!({"name": format!("F{i}")}));
    }
    let folders: Vec<u64> = s.doc.library_tree.folders.iter().map(|f| f.id.0).collect();
    for i in 0..999 {
        let c = run(&mut s, "library.duplicate", json!({"symbol": sym, "name": format!("S{i}")}))["symbol"].as_u64().unwrap();
        run(&mut s, "library.move", json!({"items": [{"symbol": c}], "to": folders[i % folders.len()]}));
    }
    let t = std::time::Instant::now();
    let uses = s.doc.library_uses();
    let counted = t.elapsed();
    let t = std::time::Instant::now();
    let unused = s.doc.unused_items();
    let reach = t.elapsed();
    let t = std::time::Instant::now();
    let listed = run(&mut s, "library.items", json!({}));
    let listing = t.elapsed();
    println!("library of {} items, {} folders: use counts {counted:?}, unused {reach:?} ({} unused), library.items {listing:?}", s.doc.library.len(), folders.len(), unused.len());
    assert_eq!(listed["items"].as_array().map(Vec::len), Some(1000));
    assert_eq!(unused.len(), 999);
    assert!(uses.values().sum::<usize>() >= 1);
    assert!(counted.as_millis() < 500 && reach.as_millis() < 500, "counted {counted:?}, unused {reach:?}");
}
