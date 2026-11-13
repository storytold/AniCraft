//! Rulers, grid and guides (ADR 0076) through the commands: guides per scene with undo, the
//! settings, persistence, and the snap engine's priorities at several zooms.

use crate::Session;
use anicraft_doc::{Document, GuideAxis};
use anicraft_geom::Vec2;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn guides(s: &Session) -> Vec<(GuideAxis, f32)> {
    s.guides().iter().map(|g| (g.axis, g.at)).collect()
}

/// A blank document drawing raw shapes, with a 100 × 80 rectangle at the origin.
fn rect_session() -> Session {
    let mut s = Session::new(Document::default());
    run(&mut s, "draw.object_drawing", json!({"on": false}));
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 100, "h": 80, "fill": "#ff0000", "stroke": "#000000"}));
    run(&mut s, "edit.deselect", json!({}));
    s
}

fn snap_at(s: &mut Session, x: f32, y: f32, r: f32) -> Option<(Vec2, String)> {
    let v = run(s, "draw.snap", json!({"x": x, "y": y, "snap": r}));
    let sn = v.get("snapped").filter(|v| v.is_object())?;
    let f = |k: &str| sn[k].as_f64().unwrap_or(f64::NAN) as f32;
    Some((Vec2::new(f("x"), f("y")), sn["to"].as_str().unwrap_or("").to_string()))
}

#[test]
fn guides_are_added_moved_deleted_and_cleared_with_undo() {
    let mut s = Session::new(Document::default());
    let v = run(&mut s, "view.guide_add", json!({"axis": "vertical", "at": 120}));
    assert_eq!(v["index"], 0);
    run(&mut s, "view.guide_add", json!({"axis": "h", "at": 64.5}));
    assert_eq!(guides(&s), [(GuideAxis::Vertical, 120.0), (GuideAxis::Horizontal, 64.5)]);
    assert!(s.is_dirty(), "guides are part of the document");
    run(&mut s, "view.guide_move", json!({"index": 0, "at": 300}));
    assert_eq!(guides(&s)[0], (GuideAxis::Vertical, 300.0));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(guides(&s)[0], (GuideAxis::Vertical, 120.0), "a move is one undo step");
    run(&mut s, "view.guide_delete", json!({"index": 0}));
    assert_eq!(guides(&s), [(GuideAxis::Horizontal, 64.5)]);
    // Locked guides can't be moved or deleted one by one; new ones can still come.
    run(&mut s, "view.guides_lock", json!({}));
    assert!(s.execute("view.guide_move", &json!({"index": 0, "at": 1})).is_err());
    assert!(s.execute("view.guide_delete", &json!({"index": 0})).is_err());
    run(&mut s, "view.guide_add", json!({"axis": "vertical", "at": -40}));
    run(&mut s, "view.guides_lock", json!({"on": false}));
    assert_eq!(run(&mut s, "view.guides_clear", json!({}))["cleared"], 2);
    assert!(guides(&s).is_empty());
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(guides(&s).len(), 2, "Clear Guides is undoable");
    // Bad input says why and changes nothing.
    for (id, p) in [
        ("view.guide_add", json!({"axis": "diagonal", "at": 1})),
        ("view.guide_add", json!({"axis": "vertical"})),
        ("view.guide_add", json!({"axis": "vertical", "at": 1e9})),
        ("view.guide_move", json!({"index": 9, "at": 1})),
        ("view.guide_delete", json!({})),
    ] {
        assert!(s.execute(id, &p).is_err(), "{id} {p}");
    }
    assert_eq!(guides(&s).len(), 2);
}

#[test]
fn guides_belong_to_their_scene() {
    let mut s = Session::new(Document::default());
    run(&mut s, "view.guide_add", json!({"axis": "vertical", "at": 10}));
    run(&mut s, "scene.new", json!({}));
    assert!(guides(&s).is_empty(), "a new scene has no guides");
    run(&mut s, "view.guide_add", json!({"axis": "horizontal", "at": 20}));
    run(&mut s, "scene.select", json!({"scene": 0}));
    assert_eq!(guides(&s), [(GuideAxis::Vertical, 10.0)]);
    run(&mut s, "scene.duplicate", json!({}));
    assert_eq!(guides(&s), [(GuideAxis::Vertical, 10.0)], "a duplicated scene keeps its guides");
}

#[test]
fn guides_and_grid_settings_are_saved_and_older_files_open() {
    let mut s = Session::new(Document::default());
    run(&mut s, "view.guide_add", json!({"axis": "vertical", "at": 33.25}));
    run(&mut s, "view.grid_set", json!({"spacing_x": 16, "spacing_y": 24, "color": "#ff000080", "over_objects": true, "show": true}));
    run(&mut s, "view.guides_set", json!({"color": "#00ff00"}));
    let text = crate::format::to_json(&s.doc).unwrap();
    let back = crate::format::from_json(&text).unwrap();
    assert_eq!(back, s.doc);
    assert_eq!(back.scenes[0].guides.len(), 1);
    assert_eq!((back.aids.grid.spacing_x, back.aids.grid.spacing_y, back.aids.grid.over_objects), (16.0, 24.0, true));
    assert!((back.aids.grid.color.a - 128.0 / 255.0).abs() < 1e-3);
    // Whether the grid shows is the editor's, not the file's.
    assert!(!text.contains("show_grid"));
    // A document without any of it writes none of it, and a file from before opens as before.
    let plain = crate::format::to_json(&Document::default()).unwrap();
    assert!(!plain.contains("guides") && !plain.contains("aids"), "{plain}");
    let old = crate::format::from_json(&plain).unwrap();
    assert!(old.scenes[0].guides.is_empty() && old.aids.is_default());
}

#[test]
fn settings_are_checked_and_toggles_take_no_undo_step() {
    let mut s = Session::new(Document::default());
    let before = s.state;
    let v = run(&mut s, "view.rulers", json!({}));
    assert_eq!(v["rulers"], true);
    run(&mut s, "view.grid_show", json!({}));
    run(&mut s, "view.grid_snap", json!({"on": true}));
    run(&mut s, "view.snap_pixels", json!({}));
    run(&mut s, "view.grid_set", json!({"accuracy": "always"}));
    run(&mut s, "view.guides_set", json!({"accuracy": "close", "snap": false}));
    assert_eq!(s.state, before, "showing and snapping change no document");
    assert!(s.undo_label().is_none());
    let a = run(&mut s, "view.aids", json!({}));
    assert_eq!((a["grid"]["show"].clone(), a["grid"]["snap"].clone(), a["grid"]["accuracy"].clone()), (json!(true), json!(true), json!("always")));
    assert_eq!((a["guides"]["snap"].clone(), a["guides"]["accuracy"].clone(), a["snap_pixels"].clone()), (json!(false), json!("close"), json!(true)));
    for p in [json!({"spacing": 0}), json!({"spacing_x": 5000}), json!({"accuracy": "sometimes"}), json!({"color": "teal"})] {
        assert!(s.execute("view.grid_set", &p).is_err(), "{p}");
    }
    assert!(s.execute("view.guides_set", &json!({"accuracy": "always"})).is_err(), "guides never always snap");
    assert_eq!(*s.doc.aids, anicraft_doc::Aids::default(), "nothing changed on a refusal");
    run(&mut s, "view.grid_set", json!({"spacing": 50}));
    assert_eq!(s.undo_label(), Some("Edit Grid…"));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc.aids.grid.spacing_x, 20.0);
}

#[test]
fn snapping_priorities_hold_at_every_zoom() {
    for zoom in [0.5f32, 1.0, 2.0, 8.0] {
        let r = 10.0 / zoom;
        // Distances in screen pixels, so each case means the same at every zoom.
        let d = |px: f32| px / zoom;
        let mut s = rect_session();
        // A point beats a guide nearer still: the rectangle's corner (100, 0).
        run(&mut s, "view.guide_add", json!({"axis": "vertical", "at": 100.0 + d(4.0)}));
        assert_eq!(snap_at(&mut s, 100.0 + d(3.0), d(2.0), r), Some((Vec2::new(100.0, 0.0), "point".into())), "zoom {zoom}");
        // Away from corners, the guide beats the rectangle's right edge (a line).
        let (q, to) = snap_at(&mut s, 100.0 + d(2.0), 40.0, r).unwrap();
        assert_eq!(to, "guide", "zoom {zoom}");
        assert!((q.x - (100.0 + d(4.0))).abs() < 1e-3 && (q.y - 40.0).abs() < 1e-3, "zoom {zoom}: {q:?}");
        // Without guides snapping, the edge.
        run(&mut s, "view.guides_snap", json!({"on": false}));
        let (q, to) = snap_at(&mut s, 100.0 + d(2.0), 40.0, r).unwrap();
        assert_eq!((to.as_str(), q.x), ("line", 100.0), "zoom {zoom}");
        // The grid, where no artwork is near: 100-pixel lines.
        run(&mut s, "view.grid_set", json!({"spacing": 100, "snap": true}));
        assert_eq!(snap_at(&mut s, 300.0 + d(4.0), 300.0 - d(6.0), r), Some((Vec2::new(300.0, 300.0), "grid".into())), "zoom {zoom}");
        assert_eq!(snap_at(&mut s, 300.0 + d(12.0), 300.0 + d(12.0), r), None, "zoom {zoom}: beyond 10 screen pixels");
        // The artwork's points still beat the grid.
        assert_eq!(snap_at(&mut s, 100.0 - d(3.0), 80.0 + d(3.0), r), Some((Vec2::new(100.0, 80.0), "point".into())), "zoom {zoom}");
        // Shift (no radius) snaps nothing.
        assert_eq!(snap_at(&mut s, 300.0 + d(4.0), 300.0, 0.0), None);
    }
}

#[test]
fn guides_grid_and_pixels_snap_with_snap_to_objects_off() {
    let mut s = rect_session();
    run(&mut s, "draw.snap_to_objects", json!({"on": false}));
    assert_eq!(snap_at(&mut s, 103.0, 1.0, 10.0), None, "nothing to snap to");
    run(&mut s, "view.snap_pixels", json!({"on": true}));
    assert_eq!(snap_at(&mut s, 103.4, 1.6, 10.0), Some((Vec2::new(103.0, 2.0), "pixel".into())), "the corner isn't looked at, pixels are");
    // A raw line's end dragged near a grid crossing goes onto it.
    run(&mut s, "draw.line", json!({"x0": 200, "y0": 200, "x1": 260, "y1": 200, "stroke": "#0000ff"}));
    run(&mut s, "edit.deselect", json!({}));
    run(&mut s, "view.grid_set", json!({"spacing": 50, "snap": true}));
    let v = run(&mut s, "raw.move_point", json!({"x": 260, "y": 200, "to_x": 296, "to_y": 246, "snap": 10}));
    assert_eq!((v["x"].as_f64(), v["y"].as_f64(), v["snapped"]["to"].as_str()), (Some(300.0), Some(250.0), Some("grid")), "{v}");
}

#[test]
fn a_moved_symbol_snaps_its_box_to_a_guide() {
    let mut s = Session::new(Document::default());
    run(&mut s, "draw.rect", json!({"x": 10, "y": 10, "w": 40, "h": 40, "object": true}));
    run(&mut s, "modify.convert_to_symbol", json!({"name": "Box"}));
    run(&mut s, "view.guide_add", json!({"axis": "vertical", "at": 200}));
    let b = s.selection_bounds();
    // Grabbed near its top-right corner and moved so that corner lands 6 pixels short of the
    // guide.
    let dx = 200.0 - 6.0 - b.max.x;
    let v = run(&mut s, "element.translate", json!({"dx": dx, "dy": 0, "snap": 10, "from": [b.max.x + dx - 2.0, b.min.y + 3.0]}));
    assert_eq!(v["snapped"]["to"], "guide", "{v}");
    assert!((s.selection_bounds().max.x - 200.0).abs() < 1e-3, "{:?}", s.selection_bounds());
}

#[test]
fn inside_a_symbol_guides_line_up_with_the_stage() {
    let mut s = Session::new(Document::default());
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 40, "h": 40, "object": true}));
    run(&mut s, "modify.convert_to_symbol", json!({"name": "Box"}));
    run(&mut s, "element.set_transform", json!({"x": 150, "y": 90, "sx": 2, "sy": 2}));
    run(&mut s, "symbol.edit", json!({}));
    let v = s.edit_view().expect("open in place");
    assert!(v.in_place && (v.scale - 2.0).abs() < 1e-4, "{v:?}");
    run(&mut s, "view.guide_add", json!({"axis": "horizontal", "at": 130}));
    run(&mut s, "draw.snap_to_objects", json!({"on": false}));
    // In the symbol's space, a point 2 stage pixels (1 symbol unit) off the guide.
    let p = Vec2::new(5.0, (130.0 + 2.0 - v.origin.y) / v.scale);
    let (q, to) = snap_at(&mut s, p.x, p.y, 10.0 / v.scale).expect("snaps");
    assert_eq!(to, "guide");
    assert!((v.origin.y + q.y * v.scale - 130.0).abs() < 1e-3, "lands on the guide as the stage shows it: {q:?}");
    assert_eq!(q.x, 5.0);
}

#[test]
fn plug_ins_may_edit_guides_but_not_the_view() {
    use crate::cmds_plugin::plugin_may_call;
    for id in ["view.aids", "view.guide_add", "view.guide_move", "view.guide_delete", "view.guides_clear"] {
        assert!(plugin_may_call(id).is_ok(), "{id}");
    }
    for id in ["view.rulers", "view.grid_show", "view.grid_set", "view.guides_lock", "view.snap_pixels"] {
        assert!(plugin_may_call(id).is_err(), "{id}");
    }
}
