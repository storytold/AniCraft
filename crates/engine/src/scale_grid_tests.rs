//! 9-slice scaling end to end (ADR 0055): the command, undo, the file format, plug-ins and what
//! a scaled instance renders.

use crate::Session;
use anicraft_doc::{ScaleGrid, SymbolId};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn err(s: &mut Session, id: &str, p: Value) -> String {
    match s.execute(id, &p) {
        Ok(v) => panic!("{id} {p} should fail, gave {v}"),
        Err(e) => e.to_string(),
    }
}

/// A 100 × 40 rounded drawing object turned into a symbol (its space runs from (−50, −20) to
/// (50, 20)), its instance selected.
fn panel() -> (Session, SymbolId) {
    let mut s = Session::new(anicraft_doc::Document::default());
    run(&mut s, "draw.rect", json!({"x": 100, "y": 100, "w": 100, "h": 40, "radius": 10, "fill": "#3366cc", "stroke": "#000000", "stroke_width": 2}));
    let v = run(&mut s, "modify.convert_to_symbol", json!({"name": "Panel"}));
    (s, SymbolId(v["symbol"].as_u64().unwrap()))
}

fn grid(s: &Session, id: SymbolId) -> Option<ScaleGrid> {
    s.doc.symbol(id).and_then(|x| x.scale_grid)
}

#[test]
fn turning_guides_on_puts_them_a_quarter_in_and_moving_one_keeps_the_rest() {
    let (mut s, id) = panel();
    let v = run(&mut s, "symbol.set_scale_grid", json!({"on": true}));
    assert_eq!(v["symbol"], json!(id.0));
    assert_eq!(grid(&s, id), Some(ScaleGrid { left: -25.0, right: 25.0, top: -10.0, bottom: 10.0 }));
    run(&mut s, "symbol.set_scale_grid", json!({"left": -40, "right": 40, "top": -10, "bottom": 10}));
    run(&mut s, "symbol.set_scale_grid", json!({"top": -12}));
    assert_eq!(grid(&s, id), Some(ScaleGrid { left: -40.0, right: 40.0, top: -12.0, bottom: 10.0 }));
    // Off hides them: the symbol scales as a whole (ADR 0057).
    let v = run(&mut s, "symbol.set_scale_grid", json!({"on": false}));
    assert_eq!(v["scale_grid"], Value::Null);
    assert_eq!(grid(&s, id), None);
    // Moving a guide turns them on, from where they were.
    run(&mut s, "symbol.set_scale_grid", json!({"symbol": id.0, "left": -45}));
    assert_eq!(grid(&s, id), Some(ScaleGrid { left: -45.0, right: 40.0, top: -12.0, bottom: 10.0 }));
}

#[test]
fn guides_switched_off_and_on_again_come_back_where_they_were() {
    let (mut s, id) = panel();
    let moved = ScaleGrid { left: -40.0, right: 30.0, top: -12.0, bottom: 6.0 };
    run(&mut s, "symbol.set_scale_grid", json!({"left": -40, "right": 30, "top": -12, "bottom": 6}));
    run(&mut s, "symbol.set_scale_grid", json!({"on": false}));
    let kept = |s: &Session| s.doc.symbol(id).and_then(|x| x.scale_grid_kept);
    assert_eq!((grid(&s, id), kept(&s)), (None, Some(moved)), "kept, not applied");
    // Off twice keeps them still; saved and opened, they are still there.
    run(&mut s, "symbol.set_scale_grid", json!({"on": false}));
    let file = crate::format::to_json(&s.doc).unwrap();
    assert!(file.contains("scale_grid_kept"));
    let mut s = Session::new(crate::format::from_json(&file).unwrap());
    assert_eq!(kept(&s), Some(moved));
    run(&mut s, "symbol.set_scale_grid", json!({"symbol": id.0, "on": true}));
    assert_eq!((grid(&s, id), kept(&s)), (Some(moved), None), "on again: back where they were");
    // Undo goes back to off with the guides kept.
    s.undo().unwrap();
    assert_eq!((grid(&s, id), kept(&s)), (None, Some(moved)));
    // A kept grid that isn't usable, or kept beside guides that are on, is dropped on load.
    let mut d = s.doc.clone();
    let sym = std::sync::Arc::make_mut(&mut d.library[0]);
    sym.scale_grid_kept = Some(ScaleGrid { left: 5.0, right: -5.0, top: 0.0, bottom: 1.0 });
    let d = crate::format::from_json(&crate::format::to_json(&d).unwrap()).unwrap();
    assert_eq!(d.library[0].scale_grid_kept, None);
    let mut d = s.doc.clone();
    let sym = std::sync::Arc::make_mut(&mut d.library[0]);
    sym.scale_grid = Some(moved);
    let d = crate::format::from_json(&crate::format::to_json(&d).unwrap()).unwrap();
    assert_eq!((d.library[0].scale_grid, d.library[0].scale_grid_kept), (Some(moved), None));
}

#[test]
fn bad_guides_are_refused_and_change_nothing() {
    let (mut s, id) = panel();
    run(&mut s, "symbol.set_scale_grid", json!({"on": true}));
    let before = grid(&s, id);
    assert!(err(&mut s, "symbol.set_scale_grid", json!({"left": 30})).contains("left"), "past the right guide");
    assert!(err(&mut s, "symbol.set_scale_grid", json!({"bottom": -20})).contains("top"));
    err(&mut s, "symbol.set_scale_grid", json!({"left": 1e30}));
    err(&mut s, "symbol.set_scale_grid", json!({}));
    err(&mut s, "symbol.set_scale_grid", json!({"on": false, "left": 1}));
    err(&mut s, "symbol.set_scale_grid", json!({"on": true, "symbol": 999_999}));
    assert_eq!(grid(&s, id), before);
    // With nothing selected and no symbol open there is nothing to set it on.
    run(&mut s, "edit.deselect", json!({}));
    err(&mut s, "symbol.set_scale_grid", json!({"on": true}));
}

#[test]
fn each_change_is_one_undo_step_and_a_drag_of_moves_is_one_too() {
    let (mut s, id) = panel();
    run(&mut s, "symbol.set_scale_grid", json!({"on": true}));
    assert_eq!(s.undo_label(), Some("9-Slice Scaling"));
    // A guide dragged: many moves, one step.
    s.begin_gesture("Move 9-Slice Guide");
    for x in [-26, -30, -35, -38] {
        run(&mut s, "symbol.set_scale_grid", json!({"left": x}));
    }
    s.end_gesture();
    assert_eq!(grid(&s, id).map(|g| g.left), Some(-38.0));
    s.undo().unwrap();
    assert_eq!(grid(&s, id).map(|g| g.left), Some(-25.0));
    s.undo().unwrap();
    assert_eq!(grid(&s, id), None);
    s.redo().unwrap();
    assert!(grid(&s, id).is_some());
}

#[test]
fn inside_the_open_symbol_it_needs_no_selection() {
    let (mut s, id) = panel();
    run(&mut s, "symbol.edit", json!({}));
    run(&mut s, "edit.deselect", json!({}));
    run(&mut s, "symbol.set_scale_grid", json!({"on": true}));
    assert!(grid(&s, id).is_some());
}

#[test]
fn the_guides_survive_the_file_and_older_files_open_without_them() {
    let (mut s, id) = panel();
    run(&mut s, "symbol.set_scale_grid", json!({"left": -40, "right": 40, "top": -10, "bottom": 10}));
    let text = crate::format::to_json(&s.doc).unwrap();
    assert!(text.contains("\"scale_grid\""));
    let back = crate::format::from_json(&text).unwrap();
    assert_eq!(back.symbol(id).and_then(|x| x.scale_grid), grid(&s, id));
    // A symbol whose guides are off keeps them apart (ADR 0057), and one that never had any
    // writes nothing for them, so older files read the same.
    run(&mut s, "symbol.set_scale_grid", json!({"on": false}));
    let text = crate::format::to_json(&s.doc).unwrap();
    assert!(!text.contains("\"scale_grid\"") && text.contains("\"scale_grid_kept\""));
    assert!(!crate::format::to_json(&panel().0.doc).unwrap().contains("scale_grid"));
    assert_eq!(crate::format::from_json(&text).unwrap().symbol(id).and_then(|x| x.scale_grid), None);
    // Unusable guides in a file are dropped, not drawn.
    let mut doc = s.doc.clone();
    if let Some(sym) = doc.library.first_mut() {
        std::sync::Arc::make_mut(sym).scale_grid = Some(ScaleGrid { left: 9.0, right: -9.0, top: 0.0, bottom: 1.0 });
    }
    let text = crate::format::to_json(&doc).unwrap();
    assert_eq!(crate::format::from_json(&text).unwrap().symbol(id).and_then(|x| x.scale_grid), None);
}

#[test]
fn a_scaled_instance_renders_with_unscaled_corners() {
    // The CPU renderer (the oracle) draws the sliced instance: at 3× wide its top-left corner
    // pixels match the unscaled instance's, while plain scaling stretches them.
    let (mut s, _) = panel();
    s.doc.stage = anicraft_doc::Stage { width: 400, height: 200, ..s.doc.stage };
    let corner = |s: &Session| {
        let img = anicraft_render::render_frame(&s.doc, 0, 1.0).unwrap();
        // The instance's top-left corner is at (100, 100): the 12 × 12 pixels there.
        (0..12u32).flat_map(|y| (0..12u32).map(move |x| (x, y))).map(|(x, y)| format!("{:?}", img.pixel(100 + x, 100 + y))).collect::<Vec<_>>()
    };
    let at_one = corner(&s);
    // Scale about the left edge: 3× wide.
    run(&mut s, "modify.transform", json!({"a": 3, "b": 0, "c": 0, "d": 1, "e": -200, "f": 0}));
    let plain = corner(&s);
    assert_ne!(plain, at_one, "plain scaling stretches the corner");
    run(&mut s, "symbol.set_scale_grid", json!({"left": -40, "right": 40, "top": -10, "bottom": 10}));
    assert_eq!(corner(&s), at_one, "9-slice keeps it");
}

#[test]
fn plug_ins_may_set_guides() {
    assert!(crate::cmds_plugin::plugin_may_call("symbol.set_scale_grid").is_ok());
}
