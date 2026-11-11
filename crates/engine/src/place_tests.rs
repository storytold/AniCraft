//! Placing Library items (a drop on the stage, Place on Stage), Swap Bitmap and Convert to Symbol
//! into a folder (ADR 0053).

use crate::Session;
use anicraft_doc::{BitmapId, Element, ElementKind, FolderId, LibraryEntry, LibraryItem, Paint, SymbolId, SymbolKind, sample};
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

fn sym(s: &Session, name: &str) -> u64 {
    s.doc.library.iter().find(|x| x.name == name).map(|x| x.id.0).unwrap_or_else(|| panic!("no symbol {name}"))
}

/// A `w`×`h` bitmap of one colour, added to the Library.
fn add_bitmap(s: &mut Session, name: &str, w: u32, h: u32) -> BitmapId {
    let id = BitmapId(s.doc.alloc_id());
    let bytes = anicraft_render::encode_png(w, h, &[255, 0, 0, 255].repeat((w * h) as usize)).unwrap();
    s.doc.bitmaps.push(Arc::new(anicraft_doc::Bitmap::from_png(id, name, bytes).unwrap()));
    id
}

fn add_sound(s: &mut Session, name: &str) -> u64 {
    let id = anicraft_doc::SoundId(s.doc.alloc_id());
    let wav = crate::cmds_sound::test_wav(8000, &[0.25; 8000]);
    s.doc.sounds.push(Arc::new(anicraft_doc::Sound::from_data(id, name, wav).unwrap()));
    id.0
}

/// Element `id` on the timeline being edited.
fn element(s: &Session, id: u64) -> Element {
    s.timeline().layers.iter().flat_map(|l| &l.keyframes).flat_map(|k| &k.elements).find(|e| e.id.0 == id).map(|e| (**e).clone()).unwrap_or_else(|| panic!("no element {id}"))
}

fn shape(e: &Element) -> &anicraft_doc::Shape {
    match &e.kind {
        ElementKind::Shape(sh) => sh,
        k => panic!("not a shape: {k:?}"),
    }
}

#[test]
fn a_drop_places_symbols_bitmaps_and_a_sound_in_one_undo_step() {
    let mut s = Session::new(sample::demo());
    let (ball, star) = (sym(&s, "Ball"), sym(&s, "Star"));
    let bitmap = add_bitmap(&mut s, "red.png", 4, 2);
    let sound = add_sound(&mut s, "beep.wav");
    run(&mut s, "timeline.goto", json!({"frame": 3}));
    let layer = s.layer;
    let before = s.doc.clone();
    let steps = s.history.undo_len();
    let v = run(&mut s, "library.place", json!({"items": [{"symbol": ball}, {"symbol": star}, {"bitmap": bitmap.0}, {"sound": sound}], "x": 200, "y": 120}));
    assert_eq!(s.history.undo_len(), steps + 1, "one undo step");
    let ids: Vec<u64> = v["elements"].as_array().unwrap().iter().map(|x| x.as_u64().unwrap()).collect();
    assert_eq!(ids.len(), 3);
    assert_eq!(v["element"], ids[0]);
    assert_eq!(s.selection.ids().map(|e| e.0).collect::<Vec<_>>(), ids, "what was placed is selected");
    // Symbols stand with their registration point on the drop point; a bitmap is centred on it.
    for &id in &ids[..2] {
        assert_eq!(element(&s, id).transform.apply(Vec2::ZERO), Vec2::new(200.0, 120.0));
    }
    assert_eq!(element(&s, ids[2]).transform.apply(Vec2::new(2.0, 1.0)), Vec2::new(200.0, 120.0));
    // All on the active layer's keyframe at the playhead, the sound too.
    assert_eq!(s.layer, layer);
    let k = s.active_layer().unwrap().keyframe_at(3).unwrap();
    assert!(ids.iter().all(|id| k.elements.iter().any(|e| e.id.0 == *id)));
    assert_eq!(k.sound.as_ref().map(|f| f.sound.0), Some(sound));
    assert_eq!(v["sound"]["sound"], sound);
    s.undo().unwrap();
    assert_eq!(s.doc, before, "one undo takes the whole drop back");
}

#[test]
fn placing_one_item_keeps_the_old_params_and_takes_a_sound_alone() {
    let mut s = Session::new(sample::demo());
    let sound = add_sound(&mut s, "beep.wav");
    let selected: Vec<_> = s.selection.ids().collect();
    let v = run(&mut s, "library.place", json!({"sound": sound}));
    assert_eq!(v["elements"], json!([]));
    assert!(v["element"].is_null());
    assert_eq!(s.selection.ids().collect::<Vec<_>>(), selected, "a sound alone leaves the selection");
    let k = s.active_layer().unwrap().keyframe_at(s.frame).unwrap();
    assert_eq!(k.sound.as_ref().map(|f| f.sound.0), Some(sound));
    // Without a position: the stage centre.
    let ball = sym(&s, "Ball");
    let el = run(&mut s, "library.place", json!({"symbol": ball}))["element"].as_u64().unwrap();
    let c = Vec2::new(s.doc.stage.width as f32 / 2.0, s.doc.stage.height as f32 / 2.0);
    assert_eq!(element(&s, el).transform.apply(Vec2::ZERO), c);
}

#[test]
fn placing_refuses_what_it_cannot_do_and_changes_nothing() {
    let mut s = Session::new(sample::demo());
    let ball = sym(&s, "Ball");
    let (a, b) = (add_sound(&mut s, "a.wav"), add_sound(&mut s, "b.wav"));
    let f = run(&mut s, "library.new_folder", json!({}))["folder"].as_u64().unwrap();
    let before = s.doc.clone();
    assert!(err(&mut s, "library.place", json!({"items": [{"symbol": ball}, {"folder": f}]})).contains("folder"));
    assert!(err(&mut s, "library.place", json!({"items": [{"sound": a}, {"sound": b}]})).contains("one sound"));
    assert!(err(&mut s, "library.place", json!({"items": [{"symbol": ball}, {"symbol": 999_999}]})).contains("no symbol"));
    assert!(err(&mut s, "library.place", json!({"items": []})).contains("nothing"));
    assert!(err(&mut s, "library.place", json!({"items": "x"})).contains("list"));
    assert!(err(&mut s, "library.place", json!({"items": (0..1001).map(|_| json!({"symbol": ball})).collect::<Vec<_>>()})).contains("at most"));
    assert!(err(&mut s, "library.place", json!({"symbol": ball, "bitmap": 1})).contains("one item"));
    err(&mut s, "library.place", json!({"symbol": ball, "x": 1e300}));
    assert!(err(&mut s, "library.place", json!({"symbol": ball, "x": 2e6})).contains("million"));
    assert!(err(&mut s, "library.place", json!({"sound": 999_999})).contains("no sound"));
    assert_eq!(s.doc, before);
    // A locked layer takes neither artwork nor a sound.
    let layer = s.layer.0;
    run(&mut s, "layer.toggle_lock", json!({"layer": layer}));
    let locked = s.doc.clone();
    assert!(err(&mut s, "library.place", json!({"symbol": ball})).contains("locked"));
    assert!(err(&mut s, "library.place", json!({"sound": a})).contains("locked"));
    assert_eq!(s.doc, locked);
}

#[test]
fn placing_inside_a_symbol_open_for_editing_uses_its_space() {
    let mut s = Session::new(sample::demo());
    let (ball, star) = (sym(&s, "Ball"), sym(&s, "Star"));
    run(&mut s, "symbol.edit", json!({"symbol": ball}));
    let el = run(&mut s, "library.place", json!({"symbol": star, "x": 5, "y": -7}))["element"].as_u64().unwrap();
    let inside = s.doc.symbol(SymbolId(ball)).unwrap().timeline.layers.iter().flat_map(|l| &l.keyframes).flat_map(|k| &k.elements).find(|e| e.id.0 == el).cloned().expect("in the open symbol");
    assert_eq!(inside.transform.apply(Vec2::ZERO), Vec2::new(5.0, -7.0));
    assert!(err(&mut s, "library.place", json!({"items": [{"symbol": star}, {"symbol": ball}]})).contains("inside itself"));
}

#[test]
fn swap_bitmap_resizes_a_placed_bitmap_and_keeps_a_filled_shape() {
    let mut s = Session::new(sample::demo());
    let small = add_bitmap(&mut s, "small.png", 2, 2);
    let big = add_bitmap(&mut s, "big.png", 6, 4);
    let placed = run(&mut s, "library.place", json!({"bitmap": small.0, "x": 100, "y": 100}))["element"].as_u64().unwrap();
    let corner = element(&s, placed).transform.apply(Vec2::ZERO);
    // A drawn rectangle filled with the small bitmap, tiled, on a second layer.
    run(&mut s, "layer.new", json!({}));
    let drawn = run(&mut s, "draw.rect", json!({"x": 10, "y": 10, "w": 30, "h": 30, "object": true}))["element"].as_u64().unwrap();
    let tiled = Paint::Bitmap { bitmap: small, repeat: true, matrix: Affine::scale(2.0, 2.0) };
    for e in s.editable_keyframe().unwrap().elements_mut_where(|id| id.0 == drawn) {
        if let ElementKind::Shape(sh) = &mut e.kind {
            sh.fill = Some(tiled.clone());
        }
    }
    let drawn_path = shape(&element(&s, drawn)).path.clone();
    run(&mut s, "edit.select_all", json!({}));
    let steps = s.history.undo_len();
    let v = run(&mut s, "element.swap_bitmap", json!({"bitmap": big.0}));
    assert_eq!(v["swapped"], 2, "on both layers");
    assert_eq!(s.history.undo_len(), steps + 1);
    let p = element(&s, placed);
    assert_eq!(p.name, "big.png", "the name follows the bitmap");
    assert_eq!(p.transform.apply(Vec2::ZERO), corner, "the top-left corner stays");
    assert_eq!(*shape(&p).path, Path::rect(0.0, 0.0, 6.0, 4.0), "the new bitmap's size");
    assert_eq!(shape(&p).fill, Some(Paint::Bitmap { bitmap: big, repeat: false, matrix: Affine::IDENTITY }));
    let d = element(&s, drawn);
    assert_eq!(shape(&d).path, drawn_path, "a filled shape keeps its outline");
    assert_eq!(shape(&d).fill, Some(Paint::Bitmap { bitmap: big, repeat: true, matrix: Affine::scale(2.0, 2.0) }), "and its fill transform");
    // The old bitmap is now unused; one undo brings it back.
    assert!(s.doc.unused_items().contains(&LibraryItem::Bitmap(small)));
    s.undo().unwrap();
    assert!(!s.doc.unused_items().contains(&LibraryItem::Bitmap(small)));
    // Refusals.
    assert!(err(&mut s, "element.swap_bitmap", json!({"bitmap": 999_999})).contains("no bitmap"));
    assert!(err(&mut s, "element.swap_bitmap", json!({})).contains("'bitmap'"));
    run(&mut s, "edit.deselect", json!({}));
    assert!(err(&mut s, "element.swap_bitmap", json!({"bitmap": big.0})).contains("nothing is selected"));
    let other = run(&mut s, "draw.rect", json!({"x": 300, "y": 300, "w": 5, "h": 5, "object": true}))["element"].as_u64().unwrap();
    run(&mut s, "element.select", json!({"ids": [other]}));
    assert!(err(&mut s, "element.swap_bitmap", json!({"bitmap": big.0})).contains("filled with one"));
}

#[test]
fn convert_to_symbol_goes_into_the_folder_asked_for_with_a_free_name() {
    let mut s = Session::new(sample::demo());
    let props = run(&mut s, "library.new_folder", json!({"name": "Props"}))["folder"].as_u64().unwrap();
    let entry = |v: &Value| LibraryEntry::Item(LibraryItem::Symbol(SymbolId(v["symbol"].as_u64().unwrap())));
    run(&mut s, "draw.rect", json!({"x": 10, "y": 10, "w": 20, "h": 20, "object": true}));
    let v = run(&mut s, "modify.convert_to_symbol", json!({"name": "Ball", "kind": "movie_clip", "folder": props}));
    assert_eq!(v["folder"], props);
    assert_eq!(v["name"], "Ball", "Props has no Ball yet (the top level's doesn't count)");
    assert_eq!(s.doc.parent_of(entry(&v)), Some(FolderId(props)));
    assert_eq!(s.doc.entry_path(entry(&v)), "Props/Ball");
    assert_eq!(s.doc.symbol(SymbolId(v["symbol"].as_u64().unwrap())).unwrap().kind, SymbolKind::MovieClip);
    // The same name again in that folder gets a number; no name gives the next free “Symbol n”.
    run(&mut s, "draw.rect", json!({"x": 50, "y": 10, "w": 20, "h": 20, "object": true}));
    assert_eq!(run(&mut s, "modify.convert_to_symbol", json!({"name": "ball", "folder": props}))["name"], "ball 2");
    run(&mut s, "draw.rect", json!({"x": 90, "y": 10, "w": 20, "h": 20, "object": true}));
    let v = run(&mut s, "modify.convert_to_symbol", json!({"folder": props}));
    assert!(v["name"].as_str().unwrap().starts_with("Symbol "));
    assert_eq!(s.doc.symbol(SymbolId(v["symbol"].as_u64().unwrap())).unwrap().kind, SymbolKind::Graphic, "graphic by default");
    // Without a folder: the top level, as before.
    run(&mut s, "draw.rect", json!({"x": 130, "y": 10, "w": 20, "h": 20, "object": true}));
    let v = run(&mut s, "modify.convert_to_symbol", json!({}));
    assert!(v["folder"].is_null());
    assert_eq!(s.doc.parent_of(entry(&v)), None);
    // Refusals change nothing.
    run(&mut s, "draw.rect", json!({"x": 170, "y": 10, "w": 20, "h": 20, "object": true}));
    let before = s.doc.clone();
    assert!(err(&mut s, "modify.convert_to_symbol", json!({"folder": 999_999})).contains("no folder"));
    assert!(err(&mut s, "modify.convert_to_symbol", json!({"kind": "sprite"})).contains("symbol type"));
    assert_eq!(s.doc, before);
    // The file keeps where it went.
    let back = crate::format::from_json(&crate::format::to_json(&s.doc).unwrap()).unwrap();
    assert_eq!(back.library_tree, s.doc.library_tree);
}
