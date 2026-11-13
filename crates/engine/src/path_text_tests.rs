//! Text on a path (ADR 0070): attaching text to a shape's outline, by selection or by picking,
//! moving it along, flipping and releasing it, edits keeping it on the path, undo, the file.

use crate::Session;
use anicraft_doc::{ElementId, ElementKind, TextBlock};
use serde_json::{Value, json};
use std::sync::Arc;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn fonts() -> bool {
    let ok = !anicraft_text::Fonts::get().families().is_empty();
    if !ok {
        eprintln!("no fonts on this computer: text on a path tests skipped");
    }
    ok
}

fn text_of(s: &Session, id: ElementId) -> (anicraft_geom::Affine, Arc<TextBlock>) {
    let e = s.visible_elements().into_iter().find(|e| e.id == id).unwrap();
    match &e.kind {
        ElementKind::Text(t) => (e.transform, t.clone()),
        other => panic!("not text: {other:?}"),
    }
}

/// A session with "Along the line" and a 400 px line from (100, 300) to (500, 300).
fn text_and_line() -> (Session, ElementId, ElementId) {
    let mut s = Session::default();
    let text = ElementId(run(&mut s, "text.create", json!({"text": "Along the line", "x": 50, "y": 50, "size": 24}))["element"].as_u64().unwrap());
    let line = ElementId(run(&mut s, "draw.line", json!({"x0": 100, "y0": 300, "x1": 500, "y1": 300}))["element"].as_u64().unwrap());
    (s, text, line)
}

#[test]
fn text_attaches_to_a_selected_shape_and_sits_on_it() {
    if !fonts() {
        return;
    }
    let (mut s, text, line) = text_and_line();
    run(&mut s, "element.select", json!({"ids": [text.0, line.0]}));
    let steps = s.history().undo_len();
    let v = run(&mut s, "text.attach_path", json!({}));
    assert_eq!(s.history().undo_len(), steps + 1, "one undo step");
    assert!((v["length"].as_f64().unwrap() - 400.0).abs() < 0.5, "{v}");
    assert_eq!(v["offset"], 0.0, "left-aligned text starts at the path's start");
    let (xf, t) = text_of(&s, text);
    let on = t.on_path.as_ref().expect("on a path");
    // In stage space the glyphs stand on the line, starting at its start.
    let b = t.outline.bounds().transformed(&xf);
    assert!(b.max.y <= 300.0 + 8.0 && b.max.y > 295.0 && b.min.y < 290.0, "standing on y 300 (descenders a little below): {b:?}");
    assert!((b.min.x - 100.0).abs() < 4.0, "{b:?}");
    let path = on.path.transformed(&xf).bounds();
    assert!((path.min.x - 100.0).abs() < 1e-3 && (path.max.x - 500.0).abs() < 1e-3);
    // The box holds the path and the glyphs from the origin.
    let local = t.outline.bounds().union(&on.path.bounds());
    assert!(local.min.x.abs() < 1e-3 && local.min.y.abs() < 1e-3 && (local.width() - t.box_width).abs() < 1e-3 && (local.height() - t.box_height).abs() < 1e-3, "{local:?}");
    // The shape is left as it is, and only the text is selected.
    assert!(s.visible_elements().iter().any(|e| e.id == line));
    assert_eq!(s.selection.on(s.layer), vec![text]);
    // Undo puts the text back in its lines.
    s.undo().unwrap();
    assert!(text_of(&s, text).1.on_path.is_none());
}

#[test]
fn offset_flip_alignment_and_edits_keep_text_on_its_path() {
    if !fonts() {
        return;
    }
    let (mut s, text, line) = text_and_line();
    run(&mut s, "element.select", json!({"ids": [text.0]}));
    // Picking the shape at a stage point (the stage's pick mode does this).
    run(&mut s, "text.attach_path", json!({"x": 300, "y": 300}));
    let left = |s: &Session| {
        let (xf, t) = text_of(s, text);
        t.outline.bounds().transformed(&xf)
    };
    let at0 = left(&s);
    run(&mut s, "text.path", json!({"offset": 100}));
    let at100 = left(&s);
    assert!((at100.min.x - at0.min.x - 100.0).abs() < 0.5, "{at0:?} → {at100:?}");
    // Flipped: from the other end, below the line.
    run(&mut s, "text.path", json!({"flip": true}));
    let f = left(&s);
    assert!(f.min.y > 290.0 && f.max.y > 310.0 && (f.max.x - 400.0).abs() < 1.0, "{f:?}");
    run(&mut s, "text.path", json!({"flip": false, "offset": 200}));
    // Centred text is centred on its offset.
    run(&mut s, "text.edit", json!({"align": "center"}));
    let c = left(&s);
    assert!((c.center().x - 300.0).abs() < 3.0, "centred on x 300: {c:?}");
    // Editing the words or the size keeps it on the path, where it was.
    run(&mut s, "text.edit", json!({"text": "Hi", "size": 40}));
    let (xf, t) = text_of(&s, text);
    assert_eq!(t.text, "Hi");
    assert!(t.on_path.is_some());
    assert!((t.on_path.as_ref().unwrap().path.transformed(&xf).bounds().min.x - 100.0).abs() < 1e-3, "the path doesn't move");
    assert!((left(&s).center().x - 300.0).abs() < 3.0);
    // Text on a path is static text.
    assert!(s.execute("text.field", &json!({"type": "dynamic"})).is_err());
    // Released: back to a line of text where it started.
    let before = left(&s);
    run(&mut s, "text.release_path", json!({}));
    let (_, t) = text_of(&s, text);
    assert!(t.on_path.is_none());
    let after = left(&s);
    assert!((after.min.x - before.min.x).abs() < 6.0 && (after.min.y - before.min.y).abs() < 12.0, "{before:?} → {after:?}");
    assert!(s.execute("text.path", &json!({"offset": 5})).is_err(), "no text on a path selected");
    let _ = line;
}

#[test]
fn bad_requests_change_nothing() {
    if !fonts() {
        return;
    }
    let (mut s, text, line) = text_and_line();
    // No shape anywhere selected or there.
    run(&mut s, "element.select", json!({"ids": [text.0]}));
    assert!(s.execute("text.attach_path", &json!({})).is_err());
    assert!(s.execute("text.attach_path", &json!({"x": 5, "y": 5})).is_err());
    assert!(s.execute("text.attach_path", &json!({"x": 5})).is_err());
    assert!(s.execute("text.attach_path", &json!({"path": line.0, "offset": 1e9})).is_err());
    // Only the shape selected: no text.
    run(&mut s, "element.select", json!({"ids": [line.0]}));
    assert!(s.execute("text.attach_path", &json!({})).is_err());
    assert!(text_of(&s, text).1.on_path.is_none());
    // By id works from another layer too.
    run(&mut s, "layer.new", json!({}));
    let other = ElementId(run(&mut s, "draw.ellipse", json!({"x": 100, "y": 100, "w": 200, "h": 200}))["element"].as_u64().unwrap());
    let base = s.timeline().layers.last().unwrap().id;
    run(&mut s, "layer.select", json!({"layer": base.0}));
    run(&mut s, "element.select", json!({"ids": [text.0]}));
    run(&mut s, "text.attach_path", json!({"path": other.0}));
    assert!(text_of(&s, text).1.on_path.is_some());
}

#[test]
fn text_on_a_path_is_saved_and_older_files_open_without_it() {
    if !fonts() {
        return;
    }
    let (mut s, text, line) = text_and_line();
    run(&mut s, "element.select", json!({"ids": [text.0, line.0]}));
    run(&mut s, "text.attach_path", json!({"offset": 12.5, "flip": true}));
    let (_, t) = text_of(&s, text);
    let json = serde_json::to_string(&*t).unwrap();
    assert!(json.contains("\"on_path\"") && json.contains("\"offset\":12.5") && json.contains("\"flip\":true"), "{json}");
    let back: TextBlock = serde_json::from_str(&json).unwrap();
    assert_eq!(back, *t);
    // The whole document round-trips through the file format.
    let saved = crate::format::to_json(&s.doc).unwrap();
    let doc = crate::format::from_json(&saved).unwrap();
    assert_eq!(doc, s.doc);
    // A block saved before text on a path opens as plain text.
    let mut v: Value = serde_json::from_str(&json).unwrap();
    v.as_object_mut().unwrap().remove("on_path");
    assert!(serde_json::from_value::<TextBlock>(v).unwrap().on_path.is_none());
    // It renders from its outlines alone: the same pixels without the font.
    let a = anicraft_render::render_frame(&s.doc, 0, 0.5).unwrap();
    assert!(a.data.iter().any(|p| p[0] < 0.5), "the text draws");
}
