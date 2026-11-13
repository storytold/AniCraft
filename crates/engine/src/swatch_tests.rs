//! The Swatches panel's commands (ADR 0078).

use crate::Session;
use anicraft_doc::{Document, ElementId, ElementKind, Paint, Rgba, Shape, Swatch, SwatchPaint};
use serde_json::{Value, json};
use std::path::Path;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn blank() -> Session {
    Session::new(Document::default())
}

fn shape(s: &Session, id: u64) -> Shape {
    s.visible_elements()
        .iter()
        .find(|e| e.id == ElementId(id))
        .and_then(|e| match &e.kind {
            ElementKind::Shape(sh) => Some(sh.clone()),
            _ => None,
        })
        .expect("a shape")
}

/// A selected 100 × 100 square at (x, 100), red with a black stroke.
fn square(s: &mut Session, x: f32) -> u64 {
    run(s, "draw.rect", json!({"x": x, "y": 100, "w": 100, "h": 100, "fill": "#ff0000", "stroke": "#000000"}))["element"].as_u64().unwrap()
}

fn find(s: &Session, name: &str) -> usize {
    s.doc.swatches.iter().position(|w| w.name == name).unwrap_or_else(|| panic!("no swatch {name}"))
}

#[test]
fn a_new_document_has_the_default_swatches_and_older_files_open_with_them() {
    let s = blank();
    assert_eq!(s.doc.swatches.len(), 19);
    // A file from before swatches (no "swatches" key) opens with the default set; the set is
    // not written while unchanged, so such files stay as they were.
    let text = crate::format::to_json(&s.doc).unwrap();
    assert!(!text.contains("\"swatches\""), "the default set isn't written");
    let back = crate::format::from_json(&text).unwrap();
    assert!(anicraft_doc::swatch::is_default_set(&back.swatches));
}

#[test]
fn swatches_round_trip_through_the_file() {
    let mut s = blank();
    run(&mut s, "swatch.clear", json!({"confirm": true}));
    run(&mut s, "swatch.add", json!({"color": "#12345680", "name": "Ink"}));
    run(&mut s, "swatch.add", json!({"type": "radial", "stops": [{"offset": 0, "color": "#ffffff"}, {"offset": 0.7, "color": "#ff000080"}], "spread": "reflect", "linear_rgb": true, "focal": 0.5, "name": "Glow"}));
    let text = crate::format::to_json(&s.doc).unwrap();
    let back = crate::format::from_json(&text).unwrap();
    assert_eq!(*back.swatches, *s.doc.swatches);
    assert_eq!(back.swatches.len(), 4);
    let SwatchPaint::Radial { spread, linear_rgb, focal, stops } = &back.swatches[3].paint else { panic!("a radial swatch") };
    assert_eq!((spread.key(), *linear_rgb, *focal, stops.len()), ("reflect", true, 0.5, 2));
    // An explicitly empty list stays empty (it isn't the default set).
    let mut e = blank();
    run(&mut e, "swatch.clear", json!({"confirm": true}));
    run(&mut e, "swatch.remove", json!({"index": 0}));
    run(&mut e, "swatch.remove", json!({"index": 0}));
    let back = crate::format::from_json(&crate::format::to_json(&e.doc).unwrap()).unwrap();
    assert!(back.swatches.is_empty());
    // A damaged file's swatches are repaired or dropped on open.
    let mut v: Value = serde_json::from_str(&text).unwrap();
    v["document"]["swatches"] = json!([{"paint": {"type": "linear", "stops": [{"offset": 0, "color": {"r": 0, "g": 0, "b": 0, "a": 1}}]}}, {"name": "ok", "paint": {"type": "solid", "color": {"r": 9, "g": 0.5, "b": 0, "a": 1}}}]);
    let fixed = crate::format::from_json(&v.to_string()).unwrap();
    assert_eq!(fixed.swatches.len(), 1);
    assert_eq!(fixed.swatches[0].paint.primary_color().r, 1.0);
}

#[test]
fn add_takes_the_current_fill_or_stroke_or_a_given_swatch() {
    let mut s = blank();
    let n = s.doc.swatches.len();
    s.style.set_fill_color(Rgba::hex(0x3366ff));
    s.style.stroke = Rgba::hex(0x00aa00);
    assert_eq!(run(&mut s, "swatch.add", json!({}))["index"], n);
    assert_eq!(s.doc.swatches[n].paint, SwatchPaint::Solid { color: Rgba::hex(0x3366ff) });
    run(&mut s, "swatch.add", json!({"from": "stroke", "at": 0, "name": "Leaf"}));
    assert_eq!((s.doc.swatches[0].name.as_str(), s.doc.swatches[0].paint.primary_color()), ("Leaf", Rgba::hex(0x00aa00)));
    // A gradient fill swatch adds its ramp and options.
    run(&mut s, "paint.set_fill", json!({"type": "radial", "spread": "repeat", "selection": false}));
    let i = run(&mut s, "swatch.add", json!({"from": "fill"}))["index"].as_u64().unwrap() as usize;
    assert!(matches!(&s.doc.swatches[i].paint, SwatchPaint::Radial { spread: anicraft_doc::Spread::Repeat, .. }));
    // Each add is one undo step.
    let len = s.doc.swatches.len();
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc.swatches.len(), len - 1);
    for bad in [json!({"from": "middle"}), json!({"type": "solid"}), json!({"type": "linear"}), json!({"type": "conic", "color": "#000000"}), json!({"from": "fill", "color": "#000000"}), json!({"color": "#00"}), json!({"color": "#000000", "at": 9999})] {
        assert!(s.execute("swatch.add", &bad).is_err(), "{bad}");
    }
}

#[test]
fn remove_move_duplicate_and_rename() {
    let mut s = blank();
    let n = s.doc.swatches.len();
    let red = s.doc.swatches[6].clone();
    run(&mut s, "swatch.move", json!({"from": 6, "to": 0}));
    assert_eq!(s.doc.swatches[0], red);
    assert_eq!(s.doc.swatches.len(), n);
    run(&mut s, "swatch.move", json!({"from": 0, "to": n - 1}));
    assert_eq!(s.doc.swatches[n - 1], red, "to the end");
    assert_eq!(run(&mut s, "swatch.duplicate", json!({"index": n - 1}))["index"], n);
    assert_eq!(s.doc.swatches[n], red);
    run(&mut s, "swatch.rename", json!({"index": n, "name": "  Fire engine  "}));
    assert_eq!(s.doc.swatches[n].name, "Fire engine");
    run(&mut s, "swatch.remove", json!({"index": n}));
    assert_eq!(s.doc.swatches.len(), n);
    for (id, bad) in [("swatch.remove", json!({"index": n})), ("swatch.remove", json!({})), ("swatch.move", json!({"from": 0, "to": n})), ("swatch.duplicate", json!({"index": -1})), ("swatch.rename", json!({"index": 0}))] {
        assert!(s.execute(id, &bad).is_err(), "{id} {bad}");
    }
}

#[test]
fn apply_sets_the_fill_or_the_stroke_of_the_selection_and_the_tool() {
    let mut s = blank();
    let id = square(&mut s, 50.0);
    let blue = find_colour(&s, 0x0000ff);
    // Fill: the selection and the fill swatch.
    assert_eq!(run(&mut s, "swatch.apply", json!({"index": blue}))["changed"], 1);
    assert_eq!(shape(&s, id).fill, Some(Paint::Solid(Rgba::hex(0x0000ff))));
    assert_eq!(s.style.fill_paint(), Paint::Solid(Rgba::hex(0x0000ff)));
    assert_eq!(shape(&s, id).stroke.unwrap().color, Rgba::BLACK, "the stroke is untouched");
    // Stroke: the selection's lines and the stroke colour; the fill stays.
    let yellow = find_colour(&s, 0xffff00);
    run(&mut s, "swatch.apply", json!({"index": yellow, "target": "stroke"}));
    assert_eq!(shape(&s, id).stroke.unwrap().color, Rgba::hex(0xffff00));
    assert_eq!(s.style.stroke, Rgba::hex(0xffff00));
    assert_eq!(shape(&s, id).fill, Some(Paint::Solid(Rgba::hex(0x0000ff))));
    // A gradient swatch is fitted to the shape, and the swatch holds it.
    let glow = find(&s, "Red ball");
    run(&mut s, "swatch.apply", json!({"index": glow}));
    let Some(Paint::Radial { center, radius, stops, .. }) = shape(&s, id).fill else { panic!("a radial fill") };
    assert_eq!(stops[0].color, Rgba::hex(0xff8080));
    assert!(center.x.abs() < 1e-3 && center.y.abs() < 1e-3 && (radius - 50.0).abs() < 1.0, "fitted to the 100-unit square: {center:?} {radius}");
    assert!(matches!(s.style.fill_paint, Some(Paint::Radial { .. })));
    // Strokes are one colour.
    assert!(s.execute("swatch.apply", &json!({"index": glow, "target": "stroke"})).is_err());
    // With `selection: false` only the tool colour changes.
    let white = find_colour(&s, 0xffffff);
    assert_eq!(run(&mut s, "swatch.apply", json!({"index": white, "selection": false}))["changed"], 0);
    assert_eq!(s.style.fill_paint(), Paint::Solid(Rgba::WHITE));
    assert!(matches!(shape(&s, id).fill, Some(Paint::Radial { .. })));
    // Nothing selected: the tool colour.
    run(&mut s, "edit.deselect", json!({}));
    run(&mut s, "swatch.apply", json!({"index": blue, "target": "stroke"}));
    assert_eq!(s.style.stroke, Rgba::hex(0x0000ff));
    assert_eq!(shape(&s, id).stroke.unwrap().color, Rgba::hex(0xffff00));
    // On the selection: one undo step (the tool colour alone is no document change).
    run(&mut s, "edit.select_all", json!({}));
    let before = s.history_len();
    run(&mut s, "swatch.apply", json!({"index": blue, "target": "fill"}));
    assert_eq!(s.history_len(), before + 1);
    assert!(s.execute("swatch.apply", &json!({"index": 0, "target": "outline"})).is_err());
}

fn find_colour(s: &Session, rgb: u32) -> usize {
    s.doc.swatches.iter().position(|w| w.paint == SwatchPaint::Solid { color: Rgba::hex(rgb) }).expect("a default colour")
}

#[test]
fn clear_and_reset_ask_first_and_undo() {
    let mut s = blank();
    let c = s.confirmation("swatch.clear", &json!({})).expect("clearing 19 swatches asks");
    assert!(!c.irreversible, "undoable, so it asks by default only");
    run(&mut s, "swatch.clear", json!({}));
    assert_eq!(*s.doc.swatches, vec![Swatch::solid(Rgba::BLACK), Swatch::solid(Rgba::WHITE)]);
    assert!(s.confirmation("swatch.clear", &json!({})).is_none(), "nothing to lose");
    assert!(s.confirmation("swatch.reset", &json!({})).is_some());
    run(&mut s, "swatch.reset", json!({}));
    assert!(anicraft_doc::swatch::is_default_set(&s.doc.swatches));
    assert!(s.confirmation("swatch.reset", &json!({})).is_none(), "already the default set");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc.swatches.len(), 2);
}

/// Runs `f` on a thread of its own with the in-memory file store, so no other test's files
/// (on a reused test thread) see it.
fn in_memory(f: impl FnOnce() + Send + 'static) {
    std::thread::spawn(move || {
        crate::vfs::use_memory();
        f();
    })
    .join()
    .unwrap();
}

#[test]
fn swatch_files_export_and_import() {
    in_memory(swatch_files_export_and_import_in_memory);
}

fn swatch_files_export_and_import_in_memory() {
    let mut s = blank();
    run(&mut s, "swatch.add", json!({"type": "linear", "stops": [{"offset": 0, "color": "#000000"}, {"offset": 1, "color": "#ffffff00"}], "name": "Fade"}));
    let v = run(&mut s, "swatch.export", json!({"path": "set.json"}));
    assert_eq!(v["swatches"], 20);
    let text = crate::vfs::read_to_string(Path::new("set.json")).unwrap();
    assert!(text.contains("\"format\": \"anicraft-swatches\"") && text.contains("\"Fade\"") && text.contains("#ffffff00"));
    assert!(s.confirmation("swatch.export", &json!({"path": "set.json"})).is_some_and(|c| c.irreversible), "overwriting a file asks");
    // Another document: added at the end, or replacing (which asks by default).
    let mut t = blank();
    assert_eq!(run(&mut t, "swatch.import", json!({"path": "set.json"}))["count"], 39);
    assert!(t.confirmation("swatch.import", &json!({"path": "set.json", "replace": true})).is_some_and(|c| !c.irreversible));
    run(&mut t, "swatch.import", json!({"path": "set.json", "replace": true}));
    assert_eq!(*t.doc.swatches, *s.doc.swatches, "the same swatches, exactly");
    // Unknown swatches in a newer file are skipped; junk is refused.
    crate::vfs::put(Path::new("new.json"), br##"{"format": "anicraft-swatches", "version": 2, "swatches": [{"type": "conic"}, {"type": "solid", "color": "#010203"}]}"##.to_vec()).unwrap();
    let before = t.doc.swatches.len();
    assert_eq!(run(&mut t, "swatch.import", json!({"path": "new.json"}))["imported"], 1);
    assert_eq!(t.doc.swatches.len(), before + 1);
    for (name, body) in [("a.json", "{}"), ("b.json", "[1, 2]"), ("c.json", "not json"), ("d.json", r#"{"format": "anicraft-brushes"}"#), ("e.json", r#"{"format": "anicraft-swatches", "swatches": []}"#)] {
        crate::vfs::put(Path::new(name), body.as_bytes().to_vec()).unwrap();
        assert!(t.execute("swatch.import", &json!({"path": name})).is_err(), "{name}");
    }
    assert!(t.execute("swatch.import", &json!({"path": "missing.json"})).is_err());
}

#[test]
fn gimp_palettes_import() {
    in_memory(gimp_palettes_import_in_memory);
}

fn gimp_palettes_import_in_memory() {
    let mut s = blank();
    crate::vfs::put(Path::new("brand.gpl"), b"GIMP Palette\nName: Brand\nColumns: 3\n#\n230  57  70\tRed\n 29  53  87\tNavy\n241 250 238 Untitled\n".to_vec()).unwrap();
    run(&mut s, "swatch.import", json!({"path": "brand.gpl", "replace": true, "confirm": true}));
    assert_eq!(s.doc.swatches.len(), 3);
    assert_eq!(s.doc.swatches[0].name, "Red");
    assert_eq!(s.doc.swatches[1].paint.primary_color().to_hex(), "#1d3557");
    assert_eq!(s.doc.swatches[2].name, "");
    // Then they paint like any swatch.
    let id = square(&mut s, 0.0);
    run(&mut s, "swatch.apply", json!({"index": 1}));
    assert_eq!(shape(&s, id).fill.map(|p| p.primary_color().to_hex()), Some("#1d3557".into()));
}

#[test]
fn swatches_are_bounded() {
    let mut s = blank();
    s.doc.swatches = std::sync::Arc::new(vec![Swatch::solid(Rgba::BLACK); anicraft_doc::MAX_SWATCHES]);
    assert!(s.execute("swatch.add", &json!({"color": "#ffffff"})).is_err());
    assert!(s.execute("swatch.duplicate", &json!({"index": 0})).is_err());
    let list = run(&mut s, "swatch.list", json!({}));
    assert_eq!(list["count"], anicraft_doc::MAX_SWATCHES);
    assert_eq!(list["swatches"][3]["index"], 3);
}

#[test]
fn plug_ins_edit_swatches_but_not_swatch_files() {
    use crate::cmds_plugin::plugin_may_call;
    for id in ["swatch.add", "swatch.apply", "swatch.remove", "swatch.move", "swatch.list", "swatch.clear"] {
        assert!(plugin_may_call(id).is_ok(), "{id}");
    }
    assert!(plugin_may_call("swatch.import").is_err() && plugin_may_call("swatch.export").is_err());
}
