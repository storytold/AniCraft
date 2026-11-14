//! Subselection on raw shapes and merging everywhere (ADR 0042), through the commands.

use crate::Session;
use anicraft_doc::{Element, ElementKind, GradientStop, Paint, Rgba};
use anicraft_geom::{Affine, Vec2};
use serde_json::{Value, json};
use std::sync::Arc;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn raw_session() -> Session {
    let mut s = Session::new(anicraft_doc::Document::default());
    run(&mut s, "draw.object_drawing", json!({"on": false}));
    s
}

fn raw(s: &Session) -> Vec<Arc<Element>> {
    s.visible_elements().into_iter().filter(|e| matches!(&e.kind, ElementKind::Shape(sh) if sh.merge)).collect()
}

fn fills(s: &Session) -> Vec<Arc<Element>> {
    raw(s).into_iter().filter(|e| matches!(&e.kind, ElementKind::Shape(sh) if sh.fill.is_some())).collect()
}

fn area(e: &Element) -> f32 {
    let ElementKind::Shape(sh) = &e.kind else { return 0.0 };
    sh.path
        .flatten(&e.transform, 0.01)
        .iter()
        .map(|pl| {
            let n = pl.points.len();
            (0..n).map(|i| pl.points[i].cross(pl.points[(i + 1) % n])).sum::<f32>() * 0.5
        })
        .sum::<f32>()
        .abs()
}

fn fill_color(e: &Element) -> Option<Rgba> {
    match &e.kind {
        ElementKind::Shape(sh) => match &sh.fill {
            Some(Paint::Solid(c)) => Some(*c),
            _ => None,
        },
        _ => None,
    }
}

fn ids(v: &Value, key: &str) -> Vec<u64> {
    v[key].as_array().into_iter().flatten().filter_map(Value::as_u64).collect()
}

fn xy(v: &Value) -> Vec<Vec2> {
    v.as_array().into_iter().flatten().filter_map(|p| Some(Vec2::new(p.get(0)?.as_f64()? as f32, p.get(1)?.as_f64()? as f32))).collect()
}

fn has(points: &[Vec2], p: (f32, f32)) -> bool {
    points.iter().any(|q| q.distance(Vec2::new(p.0, p.1)) < 1e-2)
}

#[test]
fn a_click_shows_the_anchors_of_everything_connected_once_each() {
    let mut s = raw_session();
    // An outlined rectangle, a line from its corner, and a line far away.
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 100, "h": 80, "fill": "#ff0000", "stroke": "#000000"}));
    run(&mut s, "draw.line", json!({"x0": 100, "y0": 80, "x1": 160, "y1": 140, "stroke": "#000000"}));
    run(&mut s, "draw.line", json!({"x0": 300, "y0": 0, "x1": 400, "y1": 0, "stroke": "#000000"}));
    // Fill, four sides, the joined line: six shapes. The far line is not connected.
    assert_eq!(raw(&s).len(), 7);
    let v = run(&mut s, "raw.subselect", json!({"x": 50, "y": 0}));
    assert_eq!(v["raw"], json!(true));
    assert_eq!(ids(&v, "selected").len(), 6, "{v}");
    let anchors = xy(&v["anchors"]);
    assert_eq!(anchors.len(), 5, "four corners shared by fill and sides, and the line's far end: {anchors:?}");
    for p in [(0.0, 0.0), (100.0, 0.0), (100.0, 80.0), (0.0, 80.0), (160.0, 140.0)] {
        assert!(has(&anchors, p), "{p:?} in {anchors:?}");
    }
    // The clicked edge's ends (whose handles show).
    let ends = xy(&v["ends"]);
    assert!(has(&ends, (0.0, 0.0)) && has(&ends, (100.0, 0.0)), "{ends:?}");
    // Straight edges have no handles.
    assert!(v["handles"].as_array().is_some_and(Vec::is_empty));
    // Inside the fill: the same set, no edge.
    let v = run(&mut s, "raw.subselect", json!({"x": 50, "y": 40}));
    assert_eq!(ids(&v, "selected").len(), 6);
    assert!(xy(&v["ends"]).is_empty());
    // The far line alone.
    let v = run(&mut s, "raw.subselect", json!({"x": 350, "y": 0}));
    assert_eq!(ids(&v, "selected").len(), 1);
    assert_eq!(xy(&v["anchors"]).len(), 2);
    // Shift adds; empty space clears.
    let v = run(&mut s, "raw.subselect", json!({"x": 50, "y": 40, "add": true}));
    assert_eq!(ids(&v, "selected").len(), 7);
    let v = run(&mut s, "raw.subselect", json!({"x": 700, "y": 700}));
    assert!(ids(&v, "selected").is_empty());
}

#[test]
fn fills_that_meet_along_an_edge_and_lines_ending_on_an_edge_are_connected() {
    let mut s = raw_session();
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 100, "h": 100, "fill": "#ff0000", "stroke": null}));
    run(&mut s, "draw.rect", json!({"x": 100, "y": 0, "w": 100, "h": 100, "fill": "#0000ff", "stroke": null}));
    // A line standing on the blue square's top edge (a T-junction, not a corner).
    run(&mut s, "draw.line", json!({"x0": 150, "y0": 0, "x1": 150, "y1": -50, "stroke": "#000000"}));
    run(&mut s, "edit.deselect", json!({}));
    let v = run(&mut s, "raw.subselect", json!({"x": 50, "y": 50}));
    assert_eq!(ids(&v, "selected").len(), 3, "red, blue and the line: {v}");
}

#[test]
fn dragging_a_handle_changes_the_curve_in_both_shapes_that_share_it() {
    let mut s = raw_session();
    run(&mut s, "draw.ellipse", json!({"x": 0, "y": 0, "w": 100, "h": 100, "fill": "#ff0000", "stroke": "#000000"}));
    let v = run(&mut s, "raw.subselect", json!({"x": 50, "y": 50}));
    let handles = v["handles"].as_array().cloned().unwrap_or_default();
    // A circle of four arcs: two handles per arc, each listed once although the fill and the
    // outline both have it.
    assert_eq!(handles.len(), 8, "{v}");
    let h = &handles[0];
    let (id, cmd, slot) = (h["id"].as_u64().unwrap(), h["cmd"].as_u64().unwrap(), h["slot"].as_u64().unwrap());
    let before: Vec<Arc<Element>> = raw(&s);
    s.begin_gesture("Move Handle");
    let r = run(&mut s, "raw.move_handle", json!({"id": id, "cmd": cmd, "slot": slot, "dx": 20, "dy": -15, "merge": false}));
    assert_eq!(ids(&r, "changed").len(), 2, "the fill and its outline: {r}");
    // The edge moved the same way in both: the outline still runs along the fill's edge (to
    // within the flattening `distance_to_edge` measures with).
    let changed: Vec<Arc<Element>> = raw(&s).into_iter().filter(|e| ids(&r, "changed").contains(&e.id.0)).collect();
    let (fill, line) = match (changed.iter().find(|e| fill_color(e).is_some()), changed.iter().find(|e| fill_color(e).is_none())) {
        (Some(f), Some(l)) => (f.clone(), l.clone()),
        _ => panic!("a fill and a line changed"),
    };
    let ElementKind::Shape(ls) = &line.kind else { panic!() };
    let ElementKind::Shape(fs) = &fill.kind else { panic!() };
    for pl in ls.path.flatten(&line.transform, 0.05) {
        for p in pl.points {
            assert!(fs.path.distance_to_edge(p, &fill.transform) < 0.3, "{p:?} off the fill's edge");
        }
    }
    assert!(area(&fill) != area(before.iter().find(|e| fill_color(e).is_some()).unwrap()));
    // Release: they merge again, still joined, in one undo step.
    run(&mut s, "raw.merge", json!({"ids": ids(&r, "changed")}));
    s.end_gesture();
    assert_eq!(fills(&s).len(), 1);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(raw(&s).iter().map(|e| e.id).collect::<Vec<_>>(), before.iter().map(|e| e.id).collect::<Vec<_>>());
    // Straight segments have no handles to move.
    let mut t = raw_session();
    run(&mut t, "draw.rect", json!({"x": 0, "y": 0, "w": 10, "h": 10, "fill": "#ff0000", "stroke": null}));
    let id = raw(&t)[0].id.0;
    assert!(t.execute("raw.move_handle", &json!({"id": id, "cmd": 1, "slot": 1, "dx": 1, "dy": 1})).is_err());
}

#[test]
fn selected_anchors_move_together_with_every_shape_that_shares_them() {
    let mut s = raw_session();
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 100, "h": 80, "fill": "#ff0000", "stroke": "#000000"}));
    // The top edge's two corners, 10 px up (the arrow keys, Shift: one step, merged).
    run(&mut s, "raw.move_point", json!({"x": 0, "y": 0, "to_x": 0, "to_y": -10, "with": [[100, 0]]}));
    let f = fills(&s);
    assert_eq!(f.len(), 1);
    assert!((area(&f[0]) - 100.0 * 90.0).abs() < 0.5, "{}", area(&f[0]));
    let v = run(&mut s, "raw.subselect", json!({"x": 50, "y": 40}));
    let anchors = xy(&v["anchors"]);
    assert!(has(&anchors, (0.0, -10.0)) && has(&anchors, (100.0, -10.0)) && anchors.len() == 4, "{anchors:?}");
}

#[test]
fn the_marquee_takes_whole_connected_shapes_on_the_active_layer_and_selects_the_anchors_inside() {
    let mut s = raw_session();
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 100, "h": 80, "fill": "#ff0000", "stroke": "#000000"}));
    run(&mut s, "draw.line", json!({"x0": 300, "y0": 0, "x1": 400, "y1": 0, "stroke": "#000000"}));
    run(&mut s, "edit.deselect", json!({}));
    // Over the bottom-right corner only.
    let v = run(&mut s, "raw.subselect_rect", json!({"x0": 90, "y0": 70, "x1": 120, "y1": 100}));
    assert_eq!(ids(&v, "selected").len(), 5, "the fill and its four sides: {v}");
    let inside = xy(&v["anchors"]);
    assert_eq!(inside.len(), 1);
    assert!(has(&inside, (100.0, 80.0)));
    // Through the middle of the fill, no anchors inside: the shapes, no anchors.
    let v = run(&mut s, "raw.subselect_rect", json!({"x0": 40, "y0": 30, "x1": 60, "y1": 50}));
    assert_eq!(ids(&v, "selected").len(), 5);
    assert!(xy(&v["anchors"]).is_empty());
    // Another layer is not reached.
    run(&mut s, "layer.new", json!({}));
    let v = run(&mut s, "raw.subselect_rect", json!({"x0": -10, "y0": -10, "x1": 500, "y1": 100}));
    assert!(ids(&v, "selected").is_empty(), "{v}");
}

#[test]
fn snapping_off_disables_snapping_for_both_tools() {
    let mut s = raw_session();
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 100, "h": 80, "fill": "#ff0000", "stroke": null}));
    run(&mut s, "draw.line", json!({"x0": 200, "y0": 0, "x1": 300, "y1": 0, "stroke": "#000000"}));
    run(&mut s, "edit.deselect", json!({}));
    let drag_point = |s: &mut Session| run(s, "raw.move_point", json!({"x": 200, "y": 0, "to_x": 104, "to_y": 3, "snap": 10, "merge": false}));
    // On: the line's end goes onto the rectangle's corner.
    s.begin_gesture("t");
    let v = drag_point(&mut s);
    assert_eq!(v["snapped"]["to"], json!("point"), "{v}");
    assert!((v["x"].as_f64().unwrap() - 100.0).abs() < 1e-3);
    s.end_gesture();
    run(&mut s, "edit.undo", json!({}));
    // Off: it goes where the pointer took it.
    assert_eq!(run(&mut s, "draw.snap_to_objects", json!({}))["snap_to_objects"], json!(false));
    let v = drag_point(&mut s);
    assert!(v["snapped"].is_null(), "{v}");
    assert!((v["x"].as_f64().unwrap() - 104.0).abs() < 1e-3);
    run(&mut s, "edit.undo", json!({}));
    // A selection move (the Selection tool's, and Subselection's on connected shapes) too.
    run(&mut s, "raw.subselect", json!({"x": 250, "y": 0}));
    let v = run(&mut s, "element.translate", json!({"dx": -96, "dy": 3, "snap": 10, "from": [200, 0]}));
    assert!(v["snapped"].is_null() && (v["dx"].as_f64().unwrap() + 96.0).abs() < 1e-3, "{v}");
    run(&mut s, "draw.snap_to_objects", json!({"on": true}));
    let v = run(&mut s, "element.translate", json!({"dx": 1, "dy": -1, "snap": 10, "from": [105, 3]}));
    assert!(!v["snapped"].is_null(), "{v}");
}

#[test]
fn a_paint_bucket_fill_merges_with_an_adjacent_fill_of_its_colour() {
    let mut s = raw_session();
    // A red square, and lines that close an area against its right edge.
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 100, "h": 100, "fill": "#ff0000", "stroke": null}));
    run(&mut s, "draw.path", json!({"d": "M100 0 L200 0 L200 100 L100 100", "fill": null, "stroke": "#000000"}));
    let v = run(&mut s, "paint.bucket", json!({"x": 150, "y": 50, "color": "#ff0000"}));
    assert_eq!(v["merged"], json!(true));
    assert_eq!(v["exact"], json!(true), "closed by raw lines and a fill edge: filled exactly");
    let f = fills(&s);
    assert_eq!(f.len(), 1, "one red fill");
    assert!((area(&f[0]) - 20_000.0).abs() < 1.0, "{}", area(&f[0]));
    // Another colour stays its own fill, up to the lines exactly.
    let mut t = raw_session();
    run(&mut t, "draw.rect", json!({"x": 0, "y": 0, "w": 100, "h": 100, "fill": null, "stroke": "#000000"}));
    run(&mut t, "draw.rect", json!({"x": 100, "y": 0, "w": 100, "h": 100, "fill": "#0000ff", "stroke": null}));
    run(&mut t, "paint.bucket", json!({"x": 50, "y": 50, "color": "#00ff00"}));
    let f = fills(&t);
    assert_eq!(f.len(), 2);
    let green = f.iter().find(|e| fill_color(e) == Rgba::parse_hex("#00ff00")).unwrap();
    assert!((area(green) - 10_000.0).abs() < 1.0, "{}", area(green));
    // Recolouring the blue square green joins the two greens (no line between them? there is
    // one: the square's right side stays, so they stay two).
    run(&mut t, "paint.bucket", json!({"x": 150, "y": 50, "color": "#00ff00"}));
    assert_eq!(fills(&t).len(), 2, "the outline's right side divides them");
    // An open area: nothing is filled.
    let mut u = raw_session();
    run(&mut u, "draw.path", json!({"d": "M0 0 L100 0 L100 100", "fill": null, "stroke": "#000000"}));
    assert!(u.execute("paint.bucket", &json!({"x": 60, "y": 30})).is_err());
    // Headless sessions (Object Drawing on) keep making an object.
    let mut o = Session::new(anicraft_doc::Document::default());
    run(&mut o, "draw.rect", json!({"x": 0, "y": 0, "w": 100, "h": 100, "fill": null, "stroke": "#000000"}));
    let v = run(&mut o, "paint.bucket", json!({"x": 50, "y": 50}));
    assert!(v.get("merged").is_none());
}

#[test]
fn recolouring_a_raw_fill_joins_the_fills_of_its_new_colour() {
    let mut s = raw_session();
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 100, "h": 100, "fill": "#ff0000", "stroke": null}));
    run(&mut s, "draw.rect", json!({"x": 100, "y": 0, "w": 100, "h": 100, "fill": "#0000ff", "stroke": null}));
    assert_eq!(fills(&s).len(), 2);
    let v = run(&mut s, "paint.bucket", json!({"x": 150, "y": 50, "color": "#ff0000"}));
    assert_eq!(v["merged"], json!(true));
    let f = fills(&s);
    assert_eq!(f.len(), 1);
    assert!((area(&f[0]) - 20_000.0).abs() < 1.0);
}

#[test]
fn a_pasted_raw_shape_cuts_what_it_lands_on() {
    let mut s = raw_session();
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 100, "h": 100, "fill": "#ff0000", "stroke": null}));
    run(&mut s, "draw.rect", json!({"x": 300, "y": 0, "w": 50, "h": 50, "fill": "#0000ff", "stroke": null}));
    run(&mut s, "edit.copy", json!({}));
    let v = run(&mut s, "edit.paste", json!({"x": 50, "y": 50}));
    assert_eq!(v["merged"], json!(true));
    let red = fills(&s).into_iter().find(|e| fill_color(e) == Rgba::parse_hex("#ff0000")).unwrap();
    assert!((area(&red) - (10_000.0 - 2_500.0)).abs() < 1.0, "the blue square cut a hole: {}", area(&red));
    assert_eq!(fills(&s).len(), 3);
    // The pasted piece is selected.
    assert_eq!(ids(&v, "pasted").len(), 1);
    // Paste in Place over itself: one blue square there, not two.
    run(&mut s, "edit.paste_in_place", json!({}));
    assert_eq!(fills(&s).len(), 3);
    // Objects on the clipboard still go on top, whole.
    run(&mut s, "draw.object_drawing", json!({"on": true}));
    run(&mut s, "draw.rect", json!({"x": 500, "y": 0, "w": 10, "h": 10, "fill": "#00ff00", "stroke": null}));
    run(&mut s, "edit.copy", json!({}));
    let v = run(&mut s, "edit.paste", json!({"x": 50, "y": 50}));
    assert_eq!(v["merged"], json!(false));
    assert_eq!(fills(&s).len(), 3);
}

fn linear() -> Paint {
    Paint::linear(Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0), vec![GradientStop { offset: 0.0, color: Rgba::hex(0xff0000) }, GradientStop { offset: 1.0, color: Rgba::hex(0x0000ff) }])
}

#[test]
fn gradient_fills_take_part_in_the_merge() {
    let mut s = raw_session();
    s.style.fill_paint = Some(linear());
    s.style.lock_fill = true;
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 200, "h": 100, "stroke": null}));
    assert_eq!(fills(&s).len(), 1, "raw, not an object");
    // A line across cuts it in two; each half shows its part of the gradient where it was.
    run(&mut s, "draw.line", json!({"x0": 100, "y0": -20, "x1": 100, "y1": 120, "stroke": "#000000"}));
    let f = fills(&s);
    assert_eq!(f.len(), 2);
    for e in &f {
        let ElementKind::Shape(sh) = &e.kind else { panic!() };
        let Some(Paint::Linear { start, end, .. }) = sh.fill.as_ref().map(|p| p.transformed(&e.transform)) else { panic!("{:?}", sh.fill) };
        assert!(start.distance(Vec2::ZERO) < 1e-3 && end.distance(Vec2::new(200.0, 0.0)) < 1e-3, "{start:?} {end:?}");
    }
    // Deleting the line joins the halves again: one gradient fill.
    let line = raw(&s).into_iter().find(|e| fill_color(e).is_none() && matches!(&e.kind, ElementKind::Shape(sh) if sh.stroke.is_some())).unwrap();
    s.select_only([line.id]);
    run(&mut s, "edit.delete", json!({}));
    let halves: Vec<u64> = fills(&s).iter().map(|e| e.id.0).collect();
    run(&mut s, "raw.merge", json!({"ids": halves}));
    assert_eq!(fills(&s).len(), 1);
    assert!((area(&fills(&s)[0]) - 20_000.0).abs() < 1.0);
    // A solid fill drawn over it cuts into the gradient as into any fill.
    run(&mut s, "draw.rect", json!({"x": 50, "y": 25, "w": 50, "h": 50, "fill": "#00ff00", "stroke": null}));
    let g = fills(&s).into_iter().find(|e| fill_color(e).is_none()).unwrap();
    assert!((area(&g) - (20_000.0 - 2_500.0)).abs() < 1.0);
    // A width profile is raw too (ADR 0093: the merge carries it into each piece).
    let mut t = raw_session();
    t.style.set_profile(anicraft_doc::ProfilePreset::TaperBoth);
    run(&mut t, "draw.line", json!({"x0": 0, "y0": 0, "x1": 100, "y1": 0}));
    assert_eq!(raw(&t).len(), 1);
    let _ = Affine::IDENTITY;
}

#[test]
fn subselection_timings() {
    // 12 × 12 outlined squares and lines across every row (ADR 0041's test drawing).
    let mut s = raw_session();
    for i in 0..12 {
        for j in 0..12 {
            run(&mut s, "draw.rect", json!({"x": i * 50, "y": j * 50, "w": 40, "h": 40, "fill": "#ff0000", "stroke": "#000000", "radius": 6}));
        }
        run(&mut s, "draw.line", json!({"x0": -10, "y0": i * 50 + 20, "x1": 610, "y1": i * 50 + 20, "stroke": "#0000ff"}));
    }
    // An empty outlined square for the Paint Bucket.
    run(&mut s, "draw.rect", json!({"x": 700, "y": 0, "w": 40, "h": 40, "fill": null, "stroke": "#000000"}));
    run(&mut s, "edit.deselect", json!({}));
    let n = s.visible_elements().len();
    let t = std::time::Instant::now();
    let v = run(&mut s, "raw.subselect", json!({"x": 320, "y": 320}));
    let click = t.elapsed();
    let sel: Vec<anicraft_doc::ElementId> = s.selection.ids().collect();
    let t = std::time::Instant::now();
    let pts = s.raw_points(&sel);
    let list = t.elapsed();
    let h = pts.handles.first().copied().unwrap();
    s.begin_gesture("Move Handle");
    let t = std::time::Instant::now();
    let r = run(&mut s, "raw.move_handle", json!({"id": h.id.0, "cmd": h.cmd, "slot": h.slot, "dx": 1, "dy": 1, "merge": false}));
    let frame = t.elapsed();
    let t = std::time::Instant::now();
    run(&mut s, "raw.merge", json!({"ids": ids(&r, "changed")}));
    let merge = t.elapsed();
    s.end_gesture();
    let t = std::time::Instant::now();
    assert_eq!(run(&mut s, "paint.bucket", json!({"x": 720, "y": 20, "color": "#00ff00"}))["exact"], json!(true));
    let bucket = t.elapsed();
    let t = std::time::Instant::now();
    let _ = s.execute("paint.bucket", &json!({"x": 45, "y": 45, "color": "#00ff00"}));
    let open = t.elapsed();
    eprintln!(
        "{n} raw shapes; connected set {} shapes, {} anchors, {} handles; click {:.2} ms, list {:.2} ms, handle frame {:.2} ms, merge {:.2} ms, bucket {:.2} ms (open area {:.2} ms)",
        ids(&v, "selected").len(),
        pts.anchors.len(),
        pts.handles.len(),
        click.as_secs_f64() * 1e3,
        list.as_secs_f64() * 1e3,
        frame.as_secs_f64() * 1e3,
        merge.as_secs_f64() * 1e3,
        bucket.as_secs_f64() * 1e3,
        open.as_secs_f64() * 1e3
    );
}

#[test]
fn the_eraser_takes_gradient_fills_too() {
    let mut s = raw_session();
    s.style.fill_paint = Some(linear());
    s.style.lock_fill = true;
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 200, "h": 100, "stroke": null}));
    let before: f32 = fills(&s).iter().map(|e| area(e)).sum();
    run(&mut s, "draw.erase", json!({"points": [[100, -20], [100, 120]], "size": 20}));
    let after: f32 = fills(&s).iter().map(|e| area(e)).sum();
    assert_eq!(fills(&s).len(), 2, "cut in two");
    assert!(before - after > 1500.0, "{before} → {after}");
}
