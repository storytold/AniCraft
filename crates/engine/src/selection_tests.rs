//! Selection across layers (ADR 0044): every command that reads the selection, with a selection
//! on two layers; the marquee across layers with raw parts; locked and hidden layers.

use crate::Session;
use anicraft_doc::{Element, ElementId, ElementKind, LayerId};
use anicraft_geom::Rect;
use serde_json::{Value, json};
use std::sync::Arc;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn el_id(v: &Value) -> ElementId {
    ElementId(v["element"].as_u64().unwrap())
}

/// Two layers, "Top" over "Bottom", one drawing object on each: Top's at (0, 0) 100 × 100,
/// Bottom's at (200, 100) 50 × 50. Both are selected; Top is active.
fn two_layers() -> (Session, LayerId, LayerId, ElementId, ElementId) {
    let mut s = Session::default();
    let bottom = s.layer;
    run(&mut s, "layer.rename", json!({"name": "Bottom"}));
    let b = el_id(&run(&mut s, "draw.rect", json!({"x": 200, "y": 100, "w": 50, "h": 50, "fill": "#00ff00"})));
    let top = LayerId(run(&mut s, "layer.new", json!({"name": "Top"}))["layer"].as_u64().unwrap());
    let a = el_id(&run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 100, "h": 100})));
    run(&mut s, "element.select", json!({"ids": [a.0, b.0]}));
    assert_eq!(s.selected_layers(), vec![top, bottom], "top to bottom");
    (s, top, bottom, a, b)
}

/// `id` on `layer` at the playhead.
fn element(s: &Session, layer: LayerId, id: ElementId) -> Option<Arc<Element>> {
    s.elements_on(layer).into_iter().find(|e| e.id == id)
}

fn pos(s: &Session, layer: LayerId, id: ElementId) -> (f32, f32) {
    let t = element(s, layer, id).unwrap().transform.translation();
    (t.x, t.y)
}

fn bounds(s: &Session, layer: LayerId, id: ElementId) -> Rect {
    s.element_bounds_on(layer, id)
}

// ----- the model ---------------------------------------------------------------------------------

#[test]
fn select_takes_ids_on_any_layer_and_reports_them() {
    let (mut s, top, bottom, a, b) = two_layers();
    assert!(s.selection.contains(top, a) && s.selection.contains(bottom, b));
    assert_eq!(s.layer, top, "the active layer had some of them: it stays");
    let info = s.info();
    assert_eq!(info["selection"], json!([a.0, b.0]));
    assert_eq!(info["selection_layers"], json!([top.0, bottom.0]));
    // Only the bottom one: its layer becomes active, as a click on it would.
    run(&mut s, "element.select", json!({"ids": [b.0]}));
    assert_eq!(s.layer, bottom);
    // `add` keeps what was selected; `layer` says where to look.
    run(&mut s, "element.select", json!({"ids": [a.0], "add": true, "layer": top.0}));
    assert_eq!(s.selection.len(), 2);
    assert!(s.execute("element.select", &json!({"ids": [a.0], "layer": bottom.0})).is_err(), "a is not on Bottom");
    assert!(s.execute("element.select", &json!({"ids": [999_999]})).is_err());
}

#[test]
fn shift_click_adds_and_removes_across_layers() {
    let (mut s, top, bottom, a, b) = two_layers();
    run(&mut s, "edit.deselect", json!({}));
    run(&mut s, "element.pick", json!({"x": 50, "y": 50}));
    assert_eq!(s.selection.iter().collect::<Vec<_>>(), vec![(top, a)]);
    run(&mut s, "element.pick", json!({"x": 225, "y": 125, "add": true}));
    assert_eq!(s.selection.iter().collect::<Vec<_>>(), vec![(top, a), (bottom, b)], "Shift keeps the other layer's");
    assert_eq!(s.layer, bottom, "the clicked layer becomes active");
    // A plain click on a selected element keeps the selection (so a drag moves it all).
    run(&mut s, "element.pick", json!({"x": 50, "y": 50}));
    assert_eq!(s.selection.len(), 2);
    assert_eq!(s.layer, top);
    // Shift-click again lets go of it; a plain click elsewhere selects only that.
    run(&mut s, "element.pick", json!({"x": 50, "y": 50, "add": true}));
    assert_eq!(s.selection.iter().collect::<Vec<_>>(), vec![(bottom, b)]);
    run(&mut s, "element.pick", json!({"x": 50, "y": 50}));
    assert_eq!(s.selection.iter().collect::<Vec<_>>(), vec![(top, a)]);
}

#[test]
fn locked_or_hidden_layers_are_never_selected() {
    let (mut s, top, bottom, a, b) = two_layers();
    // Locking a layer lets go of what was selected on it.
    run(&mut s, "layer.toggle_lock", json!({"layer": bottom.0}));
    assert_eq!(s.selection.iter().collect::<Vec<_>>(), vec![(top, a)]);
    assert!(s.execute("element.select", &json!({"ids": [b.0], "add": true})).is_err(), "locked");
    assert_eq!(run(&mut s, "element.pick", json!({"x": 225, "y": 125, "add": true}))["element"], Value::Null, "a click goes through it");
    run(&mut s, "edit.select_all", json!({}));
    assert_eq!(s.selection.iter().collect::<Vec<_>>(), vec![(top, a)], "Select All skips it");
    run(&mut s, "raw.select_rect", json!({"x0": -10, "y0": -10, "x1": 300, "y1": 300}));
    assert_eq!(s.selection.iter().collect::<Vec<_>>(), vec![(top, a)], "so does the marquee");
    // Hiding works the same.
    run(&mut s, "layer.toggle_lock", json!({"layer": bottom.0}));
    run(&mut s, "edit.select_all", json!({}));
    assert_eq!(s.selection.len(), 2);
    run(&mut s, "layer.toggle_visible", json!({"layer": top.0}));
    assert_eq!(s.selection.iter().collect::<Vec<_>>(), vec![(bottom, b)]);
    assert!(s.execute("element.select", &json!({"ids": [a.0]})).is_err(), "hidden");
    // A locked folder locks the layers in it.
    run(&mut s, "layer.toggle_visible", json!({"layer": top.0}));
    run(&mut s, "layer.select", json!({"layer": top.0}));
    let folder = LayerId(run(&mut s, "layer.new_folder", json!({}))["layer"].as_u64().unwrap());
    s.doc.timeline_mut(0).layer_mut(top).unwrap().parent = Some(folder);
    run(&mut s, "layer.toggle_lock", json!({"layer": folder.0}));
    run(&mut s, "edit.select_all", json!({}));
    assert_eq!(s.selection.iter().collect::<Vec<_>>(), vec![(bottom, b)]);
}

#[test]
fn moving_to_another_frame_keeps_what_is_there() {
    let (mut s, top, bottom, a, _) = two_layers();
    // Bottom ends at frame 1; Top goes on to frame 10.
    run(&mut s, "timeline.insert_keyframe", json!({"layer": top.0, "frame": 9}));
    run(&mut s, "timeline.goto", json!({"frame": 5}));
    assert_eq!(s.selection.iter().collect::<Vec<_>>(), vec![(top, a)], "Bottom has nothing at frame 6");
    assert!(!s.selected_layers().contains(&bottom));
}

// ----- per-layer edits ------------------------------------------------------------------------

#[test]
fn move_and_nudge_move_both_layers_in_one_step() {
    let (mut s, top, bottom, a, b) = two_layers();
    let steps = s.history_len();
    let (pa, pb) = (pos(&s, top, a), pos(&s, bottom, b));
    let v = run(&mut s, "element.translate", json!({"dx": 5, "dy": -3}));
    assert_eq!(v["moved"], 2);
    assert_eq!(pos(&s, top, a), (pa.0 + 5.0, pa.1 - 3.0));
    assert_eq!(pos(&s, bottom, b), (pb.0 + 5.0, pb.1 - 3.0));
    assert_eq!(s.history_len(), steps + 1, "one undo step");
    // A drag is one gesture: many moves, one step.
    s.begin_gesture("Move");
    for _ in 0..10 {
        run(&mut s, "element.translate", json!({"dx": 1, "dy": 0}));
    }
    s.end_gesture();
    assert_eq!(s.history_len(), steps + 2);
    assert_eq!(pos(&s, bottom, b).0, pb.0 + 15.0);
    s.undo().unwrap();
    s.undo().unwrap();
    assert_eq!((pos(&s, top, a), pos(&s, bottom, b)), (pa, pb));
    assert_eq!(s.selection.len(), 2, "undo brings the selection back with it");
}

#[test]
fn a_move_snaps_onto_artwork_on_the_selected_layers() {
    let (mut s, top, bottom, a, b) = two_layers();
    // Something unselected on Bottom, just right of Top's square (whose right edge is x = 100).
    run(&mut s, "layer.select", json!({"layer": bottom.0}));
    let target = el_id(&run(&mut s, "draw.rect", json!({"x": 104, "y": 0, "w": 20, "h": 20})));
    run(&mut s, "element.select", json!({"ids": [a.0, b.0]}));
    assert!(!s.selection.contains(bottom, target));
    let (pa, pb) = (pos(&s, top, a), pos(&s, bottom, b));
    let v = run(&mut s, "element.translate", json!({"dx": 1, "dy": 0, "snap": 5, "from": [100, 0]}));
    assert_eq!(v["snapped"]["to"], "point", "{v}");
    assert!((pos(&s, top, a).0 - (pa.0 + 4.0)).abs() < 1e-3, "the corner at (100, 0) met the target's at (104, 0)");
    assert!((pos(&s, bottom, b).0 - (pb.0 + 4.0)).abs() < 1e-3, "and the other layer moved by as much");
}

#[test]
fn delete_duplicate_and_arrange_work_on_each_layer() {
    let (mut s, top, bottom, a, b) = two_layers();
    // Something unselected on each layer, above the selected ones.
    let above_a = el_id(&run(&mut s, "draw.rect", json!({"x": 500, "y": 0, "w": 10, "h": 10})));
    run(&mut s, "layer.select", json!({"layer": bottom.0}));
    let above_b = el_id(&run(&mut s, "draw.rect", json!({"x": 500, "y": 50, "w": 10, "h": 10})));
    run(&mut s, "element.select", json!({"ids": [a.0, b.0]}));
    let order = |s: &Session, l: LayerId| s.elements_on(l).iter().map(|e| e.id).collect::<Vec<_>>();
    run(&mut s, "arrange.bring_to_front", json!({}));
    assert_eq!(order(&s, top), vec![above_a, a]);
    assert_eq!(order(&s, bottom), vec![above_b, b]);
    run(&mut s, "arrange.send_backward", json!({}));
    assert_eq!(order(&s, top), vec![a, above_a]);
    assert_eq!(order(&s, bottom), vec![b, above_b]);

    let v = run(&mut s, "edit.duplicate", json!({"dx": 10, "dy": 10}));
    assert_eq!(v["created"].as_array().unwrap().len(), 2, "{v}");
    assert_eq!(s.elements_on(top).len(), 3);
    assert_eq!(s.elements_on(bottom).len(), 3);
    let copies: Vec<(LayerId, ElementId)> = s.selection.iter().collect();
    assert_eq!(copies.iter().map(|c| c.0).collect::<Vec<_>>(), vec![top, bottom], "the copies are selected, each on its layer");

    let steps = s.history_len();
    let v = run(&mut s, "edit.delete", json!({}));
    assert_eq!(v["deleted"], 2);
    assert!(s.selection.is_empty());
    assert_eq!((s.elements_on(top).len(), s.elements_on(bottom).len()), (2, 2));
    assert_eq!(s.history_len(), steps + 1, "one step");
    s.undo().unwrap();
    assert_eq!((s.elements_on(top).len(), s.elements_on(bottom).len()), (3, 3));
}

#[test]
fn properties_edits_apply_to_each_layer() {
    let (mut s, top, bottom, a, b) = two_layers();
    run(&mut s, "element.set_alpha", json!({"alpha": 0.5}));
    run(&mut s, "element.set_fill", json!({"color": "#ff0000"}));
    run(&mut s, "element.set_stroke", json!({"width": 7}));
    run(&mut s, "element.rename", json!({"name": "both"}));
    run(&mut s, "element.set_transform", json!({"rotation": 45}));
    for (l, id) in [(top, a), (bottom, b)] {
        let e = element(&s, l, id).unwrap();
        assert_eq!(e.color, anicraft_doc::ColorEffect::Alpha(0.5));
        assert_eq!(e.name, "both");
        assert!((e.transform.decompose().rotation.to_degrees() - 45.0).abs() < 1e-3);
        let ElementKind::Shape(sh) = &e.kind else { panic!() };
        assert_eq!(sh.fill.as_ref().map(anicraft_doc::Paint::primary_color), Some(anicraft_doc::Rgba::hex(0xff0000)));
        assert_eq!(sh.stroke.as_ref().map(|st| st.width), Some(7.0));
    }
}

#[test]
fn filters_apply_where_they_can_and_fail_where_nothing_takes_them() {
    let (mut s, top, bottom, a, b) = two_layers();
    // Top's square becomes a movie clip; Bottom's stays a shape.
    run(&mut s, "element.select", json!({"ids": [a.0]}));
    let clip = ElementId(run(&mut s, "modify.convert_to_symbol", json!({}))["instance"].as_u64().unwrap());
    run(&mut s, "symbol.set_kind", json!({"kind": "movie_clip"}));
    run(&mut s, "element.select", json!({"ids": [clip.0, b.0]}));
    let v = run(&mut s, "element.set_filters", json!({"filters": [{"type": "blur"}]}));
    assert_eq!((v["set"].as_u64(), v["filters"].as_u64()), (Some(1), Some(1)), "one instance, one filter: {v}");
    assert!(matches!(&element(&s, top, clip).unwrap().kind, ElementKind::Symbol { filters, .. } if filters.len() == 1));
    run(&mut s, "element.set_blend", json!({"mode": "multiply"}));
    // Shapes only: nothing to filter anywhere.
    run(&mut s, "element.select", json!({"ids": [b.0]}));
    assert!(s.execute("element.set_filters", &json!({"filters": []})).is_err());
    assert!(element(&s, bottom, b).is_some());
}

#[test]
fn a_tweened_frame_gets_a_keyframe_on_each_layer() {
    let (mut s, top, bottom, a, b) = two_layers();
    // Classic tweens over frames 1–10 on both layers.
    for l in [top, bottom] {
        run(&mut s, "timeline.insert_keyframe", json!({"layer": l.0, "frame": 9}));
        run(&mut s, "timeline.create_tween", json!({"layer": l.0, "frame": 0}));
    }
    run(&mut s, "timeline.goto", json!({"frame": 4}));
    run(&mut s, "element.select", json!({"ids": [a.0, b.0]}));
    assert_eq!(s.selection.len(), 2);
    run(&mut s, "element.translate", json!({"dx": 0, "dy": 30}));
    for l in [top, bottom] {
        let layer = s.doc.timeline(0).layer(l).unwrap();
        assert!(layer.keyframes.iter().any(|k| k.start == 4), "{}: a key at the playhead", layer.name);
    }
}

// ----- across the layers ----------------------------------------------------------------------

#[test]
fn align_and_distribute_work_across_layers() {
    let (mut s, top, bottom, a, b) = two_layers();
    run(&mut s, "align.left", json!({}));
    assert!((bounds(&s, top, a).min.x - bounds(&s, bottom, b).min.x).abs() < 1e-3);
    run(&mut s, "align.bottom", json!({}));
    assert!((bounds(&s, top, a).max.y - bounds(&s, bottom, b).max.y).abs() < 1e-3);
    // A third on Bottom: the three centres spread evenly.
    run(&mut s, "layer.select", json!({"layer": bottom.0}));
    let c = el_id(&run(&mut s, "draw.rect", json!({"x": 400, "y": 400, "w": 20, "h": 20})));
    run(&mut s, "element.select", json!({"ids": [a.0, b.0, c.0]}));
    run(&mut s, "align.distribute_v", json!({}));
    let mut ys: Vec<f32> = [(top, a), (bottom, b), (bottom, c)].iter().map(|&(l, id)| bounds(&s, l, id).center().y).collect();
    ys.sort_by(f32::total_cmp);
    assert!(((ys[1] - ys[0]) - (ys[2] - ys[1])).abs() < 1e-3, "{ys:?}");
}

#[test]
fn free_transform_turns_the_combined_box() {
    let (mut s, top, bottom, a, b) = two_layers();
    let before = s.selection_bounds();
    assert_eq!(before, bounds(&s, top, a).union(&bounds(&s, bottom, b)), "one box for the whole selection");
    let c = before.center();
    run(&mut s, "modify.rotate_cw", json!({}));
    let after = s.selection_bounds();
    assert!((after.center().x - c.x).abs() < 1e-2 && (after.center().y - c.y).abs() < 1e-2, "about the combined centre");
    assert!((after.width() - before.height()).abs() < 1e-2, "turned a quarter");
    // modify.transform applies the same matrix on both layers.
    let (pa, pb) = (pos(&s, top, a), pos(&s, bottom, b));
    run(&mut s, "modify.transform", json!({"a": 1, "b": 0, "c": 0, "d": 1, "e": 3, "f": 4}));
    assert_eq!(pos(&s, top, a), (pa.0 + 3.0, pa.1 + 4.0));
    assert_eq!(pos(&s, bottom, b), (pb.0 + 3.0, pb.1 + 4.0));
    run(&mut s, "modify.flip_h", json!({}));
    run(&mut s, "modify.remove_transform", json!({}));
    for (l, id) in [(top, a), (bottom, b)] {
        assert!((element(&s, l, id).unwrap().transform.decompose().rotation).abs() < 1e-4);
    }
}

#[test]
fn copy_and_paste_keep_each_layer() {
    let (mut s, top, bottom, ..) = two_layers();
    let v = run(&mut s, "edit.copy", json!({}));
    assert_eq!((v["copied"].as_u64(), v["layers"].as_u64()), (Some(2), Some(2)));
    // Back where they came from, each on its layer.
    let v = run(&mut s, "edit.paste_in_place", json!({}));
    assert_eq!(v["layers"], json!([top.0, bottom.0]), "{v}");
    assert_eq!((s.elements_on(top).len(), s.elements_on(bottom).len()), (2, 2));
    assert_eq!(s.selected_layers(), vec![top, bottom], "the pasted copies are selected");
    // Cut takes both away in one step.
    let steps = s.history_len();
    run(&mut s, "edit.cut", json!({}));
    assert_eq!((s.elements_on(top).len(), s.elements_on(bottom).len()), (1, 1));
    assert_eq!(s.history_len(), steps + 1);
    // Into another document: layers named after the copied ones are made for them.
    run(&mut s, "file.new", json!({"confirm": true}));
    let first = s.layer;
    let v = run(&mut s, "edit.paste", json!({"x": 300, "y": 200}));
    let layers = s.doc.timeline(0).layers.clone();
    assert_eq!(layers.len(), 2, "{v}");
    assert_eq!(layers[0].id, first, "the first copied layer lands on the active one");
    assert_eq!(layers[1].name, "Bottom", "the next is made, named after the copied one");
    assert_eq!(s.elements_on(layers[0].id).len(), 1);
    assert_eq!(s.elements_on(layers[1].id).len(), 1);
    // One copied layer always goes on the active layer, as before.
    let lower = s.elements_on(layers[1].id)[0].id;
    run(&mut s, "element.select", json!({"ids": [lower.0]}));
    run(&mut s, "edit.copy", json!({}));
    run(&mut s, "layer.select", json!({"layer": first.0}));
    run(&mut s, "edit.paste_in_place", json!({}));
    assert_eq!(s.elements_on(first).len(), 2);
    assert_eq!(s.selected_layers(), vec![first]);
}

#[test]
fn convert_to_symbol_keeps_the_layers_inside_and_lands_on_the_topmost() {
    let (mut s, top, bottom, a, b) = two_layers();
    let center = s.selection_bounds().center();
    let px = |s: &Session| anicraft_render::render_frame(&s.doc, 0, 1.0).unwrap();
    let was = px(&s);
    let steps = s.history_len();
    let v = run(&mut s, "modify.convert_to_symbol", json!({"name": "Both"}));
    assert_eq!(s.history_len(), steps + 1);
    assert_eq!(v["layer"], top.0);
    let inst = ElementId(v["instance"].as_u64().unwrap());
    assert_eq!(s.selection.iter().collect::<Vec<_>>(), vec![(top, inst)]);
    assert_eq!(s.layer, top);
    assert!(element(&s, top, a).is_none() && element(&s, bottom, b).is_none(), "both moved into the symbol");
    assert!(s.elements_on(bottom).is_empty());
    let t = element(&s, top, inst).unwrap().transform.translation();
    assert!((t.x - center.x).abs() < 1e-3 && (t.y - center.y).abs() < 1e-3, "registered at the combined centre");
    let sym = s.doc.library.iter().find(|x| x.name == "Both").unwrap();
    let names: Vec<&str> = sym.timeline.layers.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, vec!["Top", "Bottom"], "the layers, as they were stacked");
    let inside: Vec<ElementId> = sym.timeline.layers.iter().flat_map(|l| l.keyframes[0].elements.iter().map(|e| e.id)).collect();
    assert_eq!(inside, vec![a, b]);
    // It looks the same as before.
    let now = px(&s);
    for (x, y) in [(225, 125), (50, 50), (150, 150)] {
        assert_eq!(now.pixel(x, y), was.pixel(x, y), "at ({x}, {y})");
    }
    // Break Apart brings both back onto the top layer, selected.
    let v = run(&mut s, "modify.break_apart", json!({}));
    assert_eq!(v["created"], 2);
    assert_eq!(s.selection.len(), 2);
}

#[test]
fn break_apart_works_on_each_layer() {
    let (mut s, top, bottom, a, b) = two_layers();
    for (l, id) in [(top, a), (bottom, b)] {
        run(&mut s, "element.select", json!({"ids": [id.0], "layer": l.0}));
        run(&mut s, "modify.convert_to_symbol", json!({}));
    }
    run(&mut s, "edit.select_all", json!({}));
    assert_eq!(s.selected_layers(), vec![top, bottom]);
    let v = run(&mut s, "modify.break_apart", json!({}));
    assert_eq!(v["created"], 2, "{v}");
    assert!(s.elements_on(top).iter().chain(s.elements_on(bottom).iter()).all(|e| matches!(e.kind, ElementKind::Shape(_))));
    assert_eq!(s.selected_layers(), vec![top, bottom]);
}

/// #253: a drawing object (Object Drawing) breaks apart into raw shapes that merge with the raw
/// drawing they touch, so it can be edited as a merged shape again; one undo step brings the
/// object back.
#[test]
fn break_apart_turns_a_drawing_object_into_raw_shapes() {
    let mut s = Session::default();
    s.style.object_drawing = false;
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 100, "h": 100, "fill": "#00ff00", "stroke": null}));
    let raw = s.visible_elements().len();
    let obj = el_id(&run(&mut s, "draw.rect", json!({"x": 50, "y": 50, "w": 100, "h": 100, "fill": "#ff0000", "stroke": null, "object": true})));
    let squiggle = el_id(&run(&mut s, "draw.freehand", json!({"points": [[300, 0], [320, 30], [340, 0], [360, 30]], "object": true})));
    let shapes = |s: &Session| -> Vec<(ElementId, bool)> {
        s.visible_elements().iter().filter_map(|e| match &e.kind {
            ElementKind::Shape(sh) => Some((e.id, sh.merge)),
            _ => None,
        }).collect()
    };
    assert!(shapes(&s).iter().filter(|(id, _)| [obj, squiggle].contains(id)).all(|(_, m)| !m), "drawn as objects");
    let before = s.doc.clone();
    let steps = s.history_len();
    run(&mut s, "element.select", json!({"ids": [obj.0, squiggle.0]}));
    let v = run(&mut s, "modify.break_apart", json!({}));
    assert!(v["created"].as_u64().unwrap() >= 2, "{v}");
    assert!(shapes(&s).iter().all(|(_, m)| *m), "every shape is raw now: {:?}", shapes(&s));
    assert!(!shapes(&s).iter().any(|(id, _)| *id == obj), "the object merged into the drawing");
    assert!(s.visible_elements().len() > raw + 1, "the red square cut the green one");
    assert!(!s.selection.is_empty(), "what came out of it is selected");
    assert_eq!(s.history_len(), steps + 1);
    s.undo().unwrap();
    assert!(s.doc == before, "one undo brings the objects back");
    // Nothing to break apart: an error, not a no-op step.
    run(&mut s, "edit.deselect", json!({}));
    assert!(s.execute("modify.break_apart", &json!({})).is_err());
}

// ----- the marquee across layers with raw parts (ADR 0041) ------------------------------------

/// Raw squares on two layers: Top's 0..100, Bottom's 200..300 (both 0..100 high).
fn raw_two_layers() -> (Session, LayerId, LayerId) {
    let mut s = Session::default();
    s.style.object_drawing = false;
    let bottom = s.layer;
    run(&mut s, "draw.rect", json!({"x": 200, "y": 0, "w": 100, "h": 100, "stroke": null}));
    let top = LayerId(run(&mut s, "layer.new", json!({"name": "Top"}))["layer"].as_u64().unwrap());
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 100, "h": 100, "stroke": null}));
    run(&mut s, "edit.deselect", json!({}));
    s.mark_saved();
    (s, top, bottom)
}

fn area(e: &Element) -> f64 {
    let ElementKind::Shape(sh) = &e.kind else { return 0.0 };
    sh.path
        .flatten(&e.transform, 0.05)
        .iter()
        .map(|pl| {
            let n = pl.points.len();
            let twice: f64 = (0..n).filter_map(|i| Some(f64::from(pl.points.get(i)?.cross(*pl.points.get((i + 1) % n)?)))).sum();
            twice * 0.5
        })
        .sum::<f64>()
        .abs()
}

#[test]
fn the_marquee_cuts_raw_parts_on_every_layer() {
    let (mut s, top, bottom) = raw_two_layers();
    let saved = s.doc.clone();
    // The lower half of both squares.
    let v = run(&mut s, "raw.select_rect", json!({"x0": -10, "y0": 50, "x1": 310, "y1": 110}));
    assert_eq!(v["cut"], 2, "one piece inside on each layer: {v}");
    assert_eq!(s.selected_layers(), vec![top, bottom]);
    for l in [top, bottom] {
        let picked: Vec<Arc<Element>> = s.elements_on(l).into_iter().filter(|e| s.selection.contains(l, e.id)).collect();
        assert_eq!(picked.len(), 1);
        assert!((area(&picked[0]) - 5000.0).abs() < 1.0, "half the square: {}", area(&picked[0]));
        assert_eq!(s.elements_on(l).len(), 2, "cut in two on its own layer");
    }
    assert!(!s.is_dirty(), "a marquee alone leaves the document saved");
    // Letting go takes the cut back exactly, on both layers.
    run(&mut s, "edit.deselect", json!({}));
    run(&mut s, "element.pick", json!({"x": 900, "y": 900}));
    assert!(s.doc == saved, "the cut is healed on both layers");

    // A move uses the cut: one step from before it, and the pieces merge back per layer.
    run(&mut s, "raw.select_rect", json!({"x0": -10, "y0": 50, "x1": 310, "y1": 110}));
    let steps = s.history_len();
    s.begin_gesture("Move");
    run(&mut s, "element.translate", json!({"dx": 0, "dy": 40}));
    run(&mut s, "draw.drop", json!({}));
    s.end_gesture();
    assert_eq!(s.history_len(), steps + 1, "the move and the drop are one step");
    for l in [top, bottom] {
        let total: f64 = s.elements_on(l).iter().map(|e| area(e)).sum();
        assert!((total - 10_000.0).abs() < 2.0, "both halves still there: {total}");
        let lowest = s.elements_on(l).iter().map(|e| s.element_bounds_on(l, e.id).max.y).fold(f32::MIN, f32::max);
        assert!((lowest - 140.0).abs() < 0.5, "the lower half moved down: {lowest}");
    }
    s.undo().unwrap();
    assert!(s.doc == saved, "one undo goes straight back to the whole drawings");
}

#[test]
fn the_marquee_with_shift_adds_across_layers_and_raw_merge_works_per_layer() {
    let (mut s, top, bottom) = raw_two_layers();
    run(&mut s, "raw.select_rect", json!({"x0": -10, "y0": -10, "x1": 50, "y1": 110}));
    assert_eq!(s.selected_layers(), vec![top]);
    assert_eq!(s.layer, top);
    run(&mut s, "raw.select_rect", json!({"x0": 250, "y0": -10, "x1": 310, "y1": 110, "add": true}));
    assert_eq!(s.selected_layers(), vec![top, bottom], "Shift keeps the first layer's parts");
    // Merging the selection again joins each layer's pieces on that layer only.
    run(&mut s, "element.set_fill", json!({"color": "#ff0000"}));
    run(&mut s, "raw.merge", json!({}));
    for l in [top, bottom] {
        assert!(s.elements_on(l).len() <= 2, "{} pieces", s.elements_on(l).len());
        let total: f64 = s.elements_on(l).iter().map(|e| area(e)).sum();
        assert!((total - 10_000.0).abs() < 2.0, "{total}");
    }
    // A marquee that starts over a layer with nothing makes the topmost hit layer active.
    run(&mut s, "layer.select", json!({"layer": top.0}));
    run(&mut s, "raw.select_rect", json!({"x0": 190, "y0": -10, "x1": 310, "y1": 110}));
    assert_eq!(s.layer, bottom);
}

// ----- robustness ------------------------------------------------------------------------------

#[test]
fn every_command_survives_a_selection_on_two_layers() {
    let mut cmds = crate::commands();
    cmds.retain(|c| !c.id.starts_with("file."));
    for spec in cmds {
        for p in [json!({}), json!({"dx": 3, "dy": 4, "x": 50, "y": 50, "name": "x", "alpha": 0.5, "filters": [], "mode": "multiply", "loop": "loop", "kind": "movie_clip", "color": "#123456", "profile": "bulge", "text": "t"})] {
            let (mut s, ..) = two_layers();
            let _ = s.execute(spec.id, &p);
            // Undo and redo always work, and the selection stays on visible, unlocked layers.
            let _ = s.undo();
            let _ = s.redo();
            for (l, _) in s.selection.iter() {
                assert!(s.selectable(l), "{}: selection on an unselectable layer", spec.id);
            }
            let _ = anicraft_render::render_frame(&s.doc, 0, 0.25);
        }
    }
}

/// Timings across layers (ADR 0044's table; run with
/// `cargo test -p anicraft-engine selection_timings -- --ignored --nocapture`).
#[test]
#[ignore]
fn selection_timings() {
    let mut s = Session::default();
    s.style.object_drawing = false;
    // 8 layers, each with 6 × 6 outlined raw squares: 288 fills and their outlines.
    for l in 0..8 {
        if l > 0 {
            run(&mut s, "layer.new", json!({}));
        }
        for i in 0..6 {
            for j in 0..6 {
                run(&mut s, "draw.rect", json!({"x": i * 50 + l * 3, "y": j * 50 + l * 3, "w": 40, "h": 40, "fill": "#ff0000", "stroke": "#000000"}));
            }
        }
    }
    run(&mut s, "edit.deselect", json!({}));
    let layers = s.doc.timeline(0).layers.len();
    let shapes: usize = s.doc.timeline(0).layers.iter().map(|l| s.elements_on(l.id).len()).sum();
    eprintln!("{shapes} raw shapes on {layers} layers");
    let time = |s: &mut Session, label: &str, reps: u32, f: &dyn Fn(&mut Session)| {
        let t = std::time::Instant::now();
        for _ in 0..reps {
            f(s);
        }
        eprintln!("{label}: {:.2} ms", t.elapsed().as_secs_f64() * 1000.0 / f64::from(reps));
    };
    time(&mut s, "select all", 20, &|s| {
        let _ = s.execute("edit.select_all", &json!({}));
    });
    time(&mut s, "fix_selection (after every command)", 20, &|s| s.fix_selection());
    s.begin_gesture("Move");
    time(&mut s, "move frame, every layer", 20, &|s| {
        let _ = s.execute("element.translate", &json!({"dx": 0.5, "dy": 0}));
    });
    time(&mut s, "snapping move frame, every layer", 20, &|s| {
        let _ = s.execute("element.translate", &json!({"dx": 0.5, "dy": 0, "snap": 10, "from": [0, 0]}));
    });
    s.end_gesture();
    s.undo().unwrap();
    time(&mut s, "marquee cut and heal, every layer", 10, &|s| {
        let _ = s.execute("raw.select_rect", &json!({"x0": 65, "y0": 65, "x1": 215, "y1": 215}));
        let _ = s.execute("edit.deselect", &json!({}));
    });
    let v = run(&mut s, "raw.select_rect", json!({"x0": 65, "y0": 65, "x1": 215, "y1": 215}));
    eprintln!("a marquee selects {} parts on {} layers ({} cut)", s.selection.len(), s.selected_layers().len(), v["cut"]);
}

#[test]
fn clicking_a_layer_name_selects_its_artwork() {
    let (mut s, top, bottom, a, b) = two_layers();
    // Another element on Bottom: the click takes both.
    run(&mut s, "layer.select", json!({"layer": bottom.0}));
    let c = el_id(&run(&mut s, "draw.rect", json!({"x": 400, "y": 400, "w": 20, "h": 20})));
    run(&mut s, "layer.select", json!({"layer": top.0}));
    assert!(s.selection.is_empty(), "without `select`, as before");
    let v = run(&mut s, "layer.select", json!({"layer": bottom.0, "select": true}));
    assert_eq!(v["selected"], json!([b.0, c.0]));
    assert_eq!(s.layer, bottom);
    assert_eq!(s.selection.iter().collect::<Vec<_>>(), vec![(bottom, b), (bottom, c)]);
    // Again on the same layer: still all of it. On a locked layer: active, nothing selected.
    run(&mut s, "element.select", json!({"ids": [a.0]}));
    run(&mut s, "layer.select", json!({"layer": top.0, "select": true}));
    assert_eq!(s.selection.iter().collect::<Vec<_>>(), vec![(top, a)]);
    run(&mut s, "layer.toggle_lock", json!({"layer": bottom.0}));
    run(&mut s, "layer.select", json!({"layer": bottom.0, "select": true}));
    assert_eq!(s.layer, bottom);
    assert!(s.selection.is_empty());
    // At a frame where the layer is empty, nothing is selected.
    run(&mut s, "layer.toggle_lock", json!({"layer": bottom.0}));
    run(&mut s, "timeline.insert_keyframe", json!({"layer": top.0, "frame": 5}));
    run(&mut s, "timeline.goto", json!({"frame": 5}));
    run(&mut s, "layer.select", json!({"layer": bottom.0, "select": true}));
    assert!(s.selection.is_empty());
}

#[test]
fn select_with_toggle_takes_one_out_and_puts_it_back_across_layers() {
    // Shift+Space on a stage element (ADR 0064): the others stay selected, on either layer.
    let (mut s, top, bottom, a, b) = two_layers();
    run(&mut s, "element.select", json!({"ids": [b.0], "toggle": true}));
    assert!(s.selection.contains(top, a) && !s.selection.contains(bottom, b));
    run(&mut s, "element.select", json!({"ids": [b.0], "toggle": true}));
    assert!(s.selection.contains(top, a) && s.selection.contains(bottom, b));
    assert!(s.execute("element.select", &json!({"ids": [b.0], "toggle": "yes"})).is_err(), "toggle is a bool");
}
