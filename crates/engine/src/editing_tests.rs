//! Editing a symbol's timeline in place, with raw drawings inside it (ADR 0045).

use crate::Session;
use anicraft_doc::{Element, ElementId, ElementKind, SymbolId};
use anicraft_geom::Vec2;
use serde_json::{Value, json};
use std::sync::Arc;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn raw(s: &Session) -> Vec<Arc<Element>> {
    s.visible_elements().into_iter().filter(|e| matches!(&e.kind, ElementKind::Shape(sh) if sh.merge)).collect()
}

fn has(points: &[Vec2], p: (f32, f32)) -> bool {
    points.iter().any(|q| q.distance(Vec2::new(p.0, p.1)) < 0.3)
}

/// A raw outlined rectangle (100 × 80) turned into a symbol, its instance moved to stand at
/// (250, 140): the symbol's space runs from (−50, −40) to (50, 40).
fn boxed() -> (Session, ElementId, SymbolId) {
    let mut s = Session::new(anicraft_doc::Document::default());
    run(&mut s, "draw.object_drawing", json!({"on": false}));
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 100, "h": 80, "fill": "#ff0000", "stroke": "#000000"}));
    let v = run(&mut s, "modify.convert_to_symbol", json!({"name": "Box"}));
    let (inst, sym) = (ElementId(v["instance"].as_u64().unwrap_or(0)), SymbolId(v["symbol"].as_u64().unwrap_or(0)));
    run(&mut s, "element.translate", json!({"dx": 200, "dy": 100}));
    (s, inst, sym)
}

#[test]
fn raw_shapes_inside_a_symbol_merge_and_take_the_selection_and_subselection_tools() {
    let (mut s, inst, sym) = boxed();
    let steps = s.history_len();
    let v = run(&mut s, "symbol.edit", json!({}));
    assert_eq!(v["editing"][0]["symbol"], json!(sym.0), "{v}");
    assert_eq!(s.history_len(), steps, "opening is not an edit");
    let view = s.edit_view().unwrap_or_else(|| panic!("open"));
    assert!(view.in_place && view.origin.distance(Vec2::new(250.0, 140.0)) < 1e-3 && (view.scale - 1.0).abs() < 1e-6, "{view:?}");
    // Inside: the symbol's own layer, its raw drawing of five shapes, in its space.
    assert_eq!(s.timeline().layers.len(), 1);
    assert_eq!(raw(&s).len(), 5);
    assert!(s.hit_test(Vec2::new(0.0, 0.0), 1.0).is_some(), "picking is in the symbol's space");
    // Subselection: everything connected shows its four corners.
    let v = run(&mut s, "raw.subselect", json!({"x": 0, "y": -40}));
    assert_eq!(v["anchors"].as_array().map(Vec::len), Some(4), "{v}");
    // The Selection tool: a corner moves with every shape on it.
    run(&mut s, "raw.move_point", json!({"x": 50, "y": -40, "to_x": 60, "to_y": -50}));
    assert!(has(&s.raw_points(&raw(&s).iter().map(|e| e.id).collect::<Vec<_>>()).anchors, (60.0, -50.0)));
    // Merge drawing: a line right across cuts the fill in two.
    run(&mut s, "draw.line", json!({"x0": -70, "y0": 0, "x1": 70, "y1": 0, "stroke": "#000000"}));
    let fills = raw(&s).into_iter().filter(|e| matches!(&e.kind, ElementKind::Shape(sh) if sh.fill.is_some())).count();
    assert_eq!(fills, 2);
    // Points: added and deleted inside, shared as on the stage.
    run(&mut s, "raw.add_point", json!({"x": -50, "y": 20}));
    run(&mut s, "raw.delete_points", json!({"points": [[-50, 20]]}));
    // Back on the stage: the instance is selected and shows the edits where it stands.
    run(&mut s, "symbol.exit", json!({}));
    assert!(s.editing.is_empty());
    assert_eq!(s.selection.ids().collect::<Vec<_>>(), vec![inst]);
    let b = s.element_bounds(inst);
    assert!(b.max.x > 309.0 && b.min.y < 91.0, "the moved corner at (310, 90): {b:?}");
}

#[test]
fn undo_goes_back_into_the_symbol_where_the_change_was_made() {
    let (mut s, _, sym) = boxed();
    run(&mut s, "symbol.edit", json!({}));
    run(&mut s, "raw.delete_points", json!({"points": [[-50, -40]]}));
    assert_eq!(raw(&s).iter().filter(|e| matches!(&e.kind, ElementKind::Shape(sh) if sh.fill.is_some())).count(), 1);
    run(&mut s, "symbol.exit", json!({}));
    assert!(s.undo().is_ok());
    assert_eq!(s.editing_symbol(), Some(sym), "undo opens the symbol again");
    assert_eq!(s.raw_points(&raw(&s).iter().map(|e| e.id).collect::<Vec<_>>()).anchors.len(), 4, "with the corner back");
    // Undoing the conversion takes the symbol away: editing closes.
    while s.undo().is_ok() {}
    assert!(s.editing.is_empty() && s.doc.library.is_empty());
}

#[test]
fn opening_from_the_library_and_guards() {
    let (mut s, inst, sym) = boxed();
    run(&mut s, "edit.deselect", json!({}));
    assert!(s.execute("symbol.edit", &json!({})).is_err(), "nothing selected to open");
    assert!(s.execute("symbol.exit", &json!({})).is_err(), "nothing open");
    run(&mut s, "symbol.edit", json!({"symbol": sym.0}));
    let view = s.edit_view().unwrap_or_else(|| panic!("open"));
    let centre = Vec2::new(s.doc.stage.width as f32 * 0.5, s.doc.stage.height as f32 * 0.5);
    assert!(!view.in_place && view.origin == centre, "alone, at the stage's centre: {view:?}");
    assert!(s.execute("library.place", &json!({"symbol": sym.0})).is_err(), "not inside itself");
    assert!(s.execute("symbol.edit", &json!({"symbol": sym.0})).is_err(), "open already");
    // Ctrl+E with nothing to open goes back to the document.
    run(&mut s, "edit.deselect", json!({}));
    run(&mut s, "symbol.edit", json!({}));
    assert!(s.editing.is_empty());
    // A rotated instance opens upright where it stands, the artwork around it hidden.
    run(&mut s, "element.select", json!({"ids": [inst.0]}));
    run(&mut s, "modify.transform", json!({"a": 0.0, "b": 1.0, "c": -1.0, "d": 0.0, "e": 0.0, "f": 0.0}));
    run(&mut s, "symbol.edit", json!({"id": inst.0}));
    let view = s.edit_view().unwrap_or_else(|| panic!("open"));
    assert!(!view.in_place && (view.scale - 1.0).abs() < 1e-6, "{view:?}");
    // A scene change closes it.
    run(&mut s, "scene.new", json!({}));
    assert!(s.editing.is_empty());
}

/// The sample's Play button open from the Library (at the stage's centre, 640 × 360): its face is
/// 220 × 80, its Hit frame 240 × 100.
fn open_button(hit: Option<(anicraft_doc::Rgba, f32)>) -> Session {
    let mut s = Session::new(anicraft_doc::sample::interactive());
    let id = s.doc.library.iter().find(|x| x.kind == anicraft_doc::SymbolKind::Button).map(|x| x.id).unwrap_or_else(|| panic!("a button"));
    run(&mut s, "symbol.edit", json!({"symbol": id.0}));
    s.hit_area = hit;
    s
}

#[test]
fn an_open_button_shows_its_hit_area_over_its_other_states() {
    let red = anicraft_doc::Rgba::new(1.0, 0.0, 0.0, 1.0);
    let render = |s: &Session| {
        let pic = s.stage_picture();
        anicraft_render::render_frame(&pic.doc, pic.frame, 1.0).unwrap_or_else(|e| panic!("{e}"))
    };
    let plain = render(&open_button(None));
    let mut s = open_button(Some((red, 0.5)));
    assert_eq!(s.hit_overlay(), Some((red, 0.5)), "on the Up frame");
    let shown = render(&s);
    let px = |p: &anicraft_render::Pixmap, x: u32, y: u32| p.pixel(x, y).unwrap_or_else(|| panic!("({x}, {y}) in the picture"));
    // Inside the Hit frame but outside the face (x = 640 + 115): the background, half red now.
    let (bg, over) = (px(&plain, 755, 360), px(&shown, 755, 360));
    for c in 0..3 {
        let want = bg[c] * 0.5 + if c == 0 { 0.5 } else { 0.0 };
        assert!((over[c] - want).abs() < 0.02, "channel {c}: {over:?} vs {want} (background {bg:?})");
    }
    // Over the face (blue) and its arrow (white) alike: half red over what the state shows.
    for (x, y) in [(600, 360), (640, 360)] {
        let (art, over) = (px(&plain, x, y), px(&shown, x, y));
        for c in 0..3 {
            let want = art[c] * 0.5 + if c == 0 { 0.5 } else { 0.0 };
            assert!((over[c] - want).abs() < 0.02, "({x}, {y}) channel {c}: {over:?} vs {want}");
        }
    }
    // Outside the Hit frame: untouched.
    assert_eq!(px(&shown, 775, 360), px(&plain, 775, 360));
    // On the Hit frame itself, nothing over it; it isn't in the document either.
    s.set_frame(3);
    assert_eq!(s.hit_overlay(), None);
    let on_hit = render(&s);
    s.hit_area = None;
    assert_eq!(on_hit.to_rgba8(), render(&s).to_rgba8());
    assert_eq!(s.doc.library.len(), anicraft_doc::sample::interactive().library.len());
    // Only buttons: a movie clip open shows none.
    let mut s = Session::new(anicraft_doc::sample::interactive());
    let clip = s.doc.library.iter().find(|x| x.kind == anicraft_doc::SymbolKind::MovieClip).map(|x| x.id).unwrap_or_else(|| panic!("a clip"));
    run(&mut s, "symbol.edit", json!({"symbol": clip.0}));
    s.hit_area = Some((red, 0.5));
    assert_eq!(s.hit_overlay(), None);
}

#[test]
fn the_stage_shows_the_open_symbol_in_place_over_the_dimmed_stage() {
    let (mut s, _, _) = boxed();
    // Something else on the stage, beside the instance.
    run(&mut s, "draw.rect", json!({"x": 400, "y": 300, "w": 50, "h": 50, "fill": "#0000ff", "stroke": null, "object": true}));
    run(&mut s, "element.pick", json!({"x": 250, "y": 140}));
    run(&mut s, "symbol.edit", json!({}));
    let pic = s.stage_picture();
    assert_eq!(pic.state, s.stage_picture().state, "the same picture twice");
    let px = anicraft_render::render_frame(&pic.doc, pic.frame, 1.0).unwrap_or_else(|e| panic!("{e}"));
    let at = |x: u32, y: u32| px.pixel(x, y).unwrap_or_else(|| panic!("in the picture"));
    // The symbol's fill in full red where its instance stands...
    let red = at(250, 140);
    assert!(red[0] > 0.95 && red[1] < 0.05 && red[2] < 0.05, "{red:?}");
    // ...and the stage around it dimmed (blue at about a third over white).
    let blue = at(425, 325);
    assert!(blue[2] > 0.95 && blue[0] > 0.5 && blue[0] < 0.8, "{blue:?}");
    // The document itself is untouched by the picture.
    assert_eq!(s.doc.library.len(), 1);
    // Back on the stage, the picture is the document.
    run(&mut s, "symbol.exit", json!({}));
    assert_eq!(s.stage_picture().state, s.state);
}
