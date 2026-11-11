//! Adding and deleting raw anchors, smooth handle pairs, Make Smooth / Corner and snapping while
//! drawing (ADR 0045), through the commands.

use crate::Session;
use anicraft_doc::{Element, ElementKind};
use anicraft_geom::raw::{self as rawgeom};
use anicraft_geom::walk::cubic_point;
use anicraft_geom::Vec2;
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

fn is_fill(e: &Element) -> bool {
    matches!(&e.kind, ElementKind::Shape(sh) if sh.fill.is_some())
}

fn all_ids(s: &Session) -> Vec<anicraft_doc::ElementId> {
    raw(s).iter().map(|e| e.id).collect()
}

fn anchors(s: &Session) -> Vec<Vec2> {
    s.raw_points(&all_ids(s)).anchors
}

fn has(points: &[Vec2], p: (f32, f32)) -> bool {
    points.iter().any(|q| q.distance(Vec2::new(p.0, p.1)) < 0.3)
}

fn area(e: &Element) -> f32 {
    let ElementKind::Shape(sh) = &e.kind else { return 0.0 };
    sh.path.flatten(&e.transform, 0.01).iter().map(|pl| pl.points.iter().zip(pl.points.iter().cycle().skip(1)).map(|(a, b)| a.cross(*b)).sum::<f32>() * 0.5).sum::<f32>().abs()
}

/// Every shape's anchors in stage space.
fn shape_anchors(e: &Element) -> Vec<Vec2> {
    match &e.kind {
        ElementKind::Shape(sh) => sh.path.editable_anchors().into_iter().map(|(_, q)| e.transform.apply(q)).collect(),
        _ => Vec::new(),
    }
}

/// Largest distance from points of the curve `c` (stage space) to the shape's outline.
fn off_outline(e: &Element, c: &rawgeom::Cubic) -> f32 {
    let ElementKind::Shape(sh) = &e.kind else { return f32::INFINITY };
    (0..=20).map(|i| sh.path.distance_to_edge(cubic_point(c, i as f32 / 20.0), &e.transform)).fold(0.0, f32::max)
}

/// A raw arc (a fill under it and its outline): `M0 100 C0 0 200 0 200 100 Z`.
fn arc(s: &mut Session) -> rawgeom::Cubic {
    run(s, "draw.path", json!({"d": "M0 100 C0 0 200 0 200 100 Z", "fill": "#ff0000", "stroke": "#000000"}));
    run(s, "edit.deselect", json!({}));
    [Vec2::new(0.0, 100.0), Vec2::new(0.0, 0.0), Vec2::new(200.0, 0.0), Vec2::new(200.0, 100.0)]
}

#[test]
fn an_added_point_is_shared_by_every_shape_on_the_edge_and_the_curve_keeps_its_shape() {
    let mut s = raw_session();
    let c = arc(&mut s);
    let before = anchors(&s).len();
    // How far the outlines are from the drawn curve (flattening): adding may not change it.
    let was: Vec<(anicraft_doc::ElementId, f32)> = raw(&s).iter().map(|e| (e.id, off_outline(e, &c))).collect();
    let top = cubic_point(&c, 0.5);
    let v = run(&mut s, "raw.add_point", json!({"x": top.x, "y": top.y + 1.0, "slop": 4}));
    let at = Vec2::new(v["x"].as_f64().unwrap_or(0.0) as f32, v["y"].as_f64().unwrap_or(0.0) as f32);
    assert!(at.distance(top) < 0.05, "on the curve: {at:?} vs {top:?}");
    assert_eq!(v["changed"].as_array().map(Vec::len), Some(2), "the fill and its outline: {v}");
    assert_eq!(anchors(&s).len(), before + 1, "listed once");
    let changed: Vec<u64> = v["changed"].as_array().into_iter().flatten().filter_map(Value::as_u64).collect();
    for e in raw(&s).into_iter().filter(|e| changed.contains(&e.id.0)) {
        assert!(has(&shape_anchors(&e), (at.x, at.y)), "{} lacks the point", e.id.0);
        // Analytic: the old curve still lies on every shape's outline, as closely as before.
        let w = was.iter().find(|x| x.0 == e.id).map_or(0.0, |x| x.1);
        assert!(off_outline(&e, &c) <= w + 1e-3 && w < 0.25, "the curve moved: {} (was {w})", off_outline(&e, &c));
    }
    // One undo step; a second point at the same place is refused.
    assert!(s.can_undo());
    assert!(s.execute("raw.add_point", &json!({"x": at.x, "y": at.y})).is_err());
    assert!(s.execute("raw.add_point", &json!({"x": 900, "y": 900})).is_err(), "nothing there");
    // Deleting it gives the curve back, in both shapes.
    run(&mut s, "raw.delete_points", json!({"points": [[at.x, at.y]]}));
    assert_eq!(anchors(&s).len(), before);
    let on_curve: Vec<Arc<Element>> = raw(&s).into_iter().filter(|e| off_outline(e, &c) < 0.3).collect();
    assert_eq!(on_curve.len(), 2, "the fill and the arc's line still follow the curve");
}

#[test]
fn deleting_a_corner_rejoins_the_fill_and_its_outline() {
    let mut s = raw_session();
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 100, "h": 80, "fill": "#ff0000", "stroke": "#000000"}));
    run(&mut s, "edit.deselect", json!({}));
    let steps = s.history_len();
    let v = run(&mut s, "raw.delete_points", json!({"points": [[0, 0]]}));
    assert_eq!(v["deleted"], json!(1), "{v}");
    assert_eq!(s.history_len(), steps + 1, "one undo step");
    // A triangle of half the area, outlined all round.
    let fill: Vec<Arc<Element>> = raw(&s).into_iter().filter(|e| is_fill(e)).collect();
    assert_eq!(fill.len(), 1);
    assert!((area(&fill[0]) - 4000.0).abs() < 1.0, "{}", area(&fill[0]));
    let pts = anchors(&s);
    assert_eq!(pts.len(), 3, "{pts:?}");
    assert!(!has(&pts, (0.0, 0.0)));
    // Connectivity: every shape's anchors are shared with another shape.
    let all = raw(&s);
    for e in &all {
        for p in shape_anchors(e) {
            let n = all.iter().filter(|o| o.id != e.id && has(&shape_anchors(o), (p.x, p.y))).count();
            assert!(n >= 1, "{p:?} of {} is on no other shape", e.id.0);
        }
    }
    // The diagonal is stroked: some outline runs from (0, 80) to (100, 0).
    let diag = rawgeom::Seg::line(Vec2::new(0.0, 80.0), Vec2::new(100.0, 0.0)).c;
    assert!(all.iter().filter(|e| !is_fill(e)).any(|e| off_outline(e, &diag) < 0.05), "no line on the new edge");
    s.undo().unwrap_or_default();
    assert_eq!(anchors(&s).len(), 4);
}

#[test]
fn deleting_the_last_points_of_a_line_removes_it_and_junctions_stay() {
    let mut s = raw_session();
    run(&mut s, "draw.line", json!({"x0": 0, "y0": 0, "x1": 100, "y1": 0, "stroke": "#000000"}));
    let v = run(&mut s, "raw.delete_points", json!({"points": [[100, 0]]}));
    assert_eq!(v["removed"].as_array().map(Vec::len), Some(1), "a one-segment line goes: {v}");
    assert!(raw(&s).is_empty());
    assert!(s.selection.is_empty(), "nothing selected that is gone");
    // A T: the point where the stem stands on the bar joins three edges.
    run(&mut s, "draw.line", json!({"x0": 0, "y0": 0, "x1": 100, "y1": 0, "stroke": "#000000"}));
    run(&mut s, "draw.line", json!({"x0": 50, "y0": 0, "x1": 50, "y1": 60, "stroke": "#000000"}));
    let n = raw(&s).len();
    let e = s.execute("raw.delete_points", &json!({"points": [[50, 0]]}));
    assert!(e.is_err_and(|e| e.to_string().contains("three or more")));
    assert_eq!(raw(&s).len(), n);
    // The stem's free end shortens nothing else.
    run(&mut s, "raw.delete_points", json!({"points": [[50, 60]]}));
    assert!(!has(&anchors(&s), (50.0, 60.0)));
    assert!(has(&anchors(&s), (0.0, 0.0)) && has(&anchors(&s), (100.0, 0.0)));
    assert!(s.execute("raw.delete_points", &json!({"points": []})).is_err());
    assert!(s.execute("raw.delete_points", &json!({"points": [[500, 500]]})).is_err());
}

/// A raw S-curve stroke through (100, 0) with a smooth anchor there.
fn s_curve(s: &mut Session) {
    run(s, "draw.path", json!({"d": "M0 0 C0 -50 60 -50 100 0 C140 50 200 50 200 0", "stroke": "#000000", "fill": null}));
    run(s, "edit.deselect", json!({}));
}

/// The handles at `a` listed by `raw.points`.
fn handles_at(s: &Session, a: Vec2) -> Vec<crate::cmds_rawsub::Handle> {
    s.raw_points(&all_ids(s)).handles.into_iter().filter(|h| h.anchor.distance(a) < 0.3).collect()
}

#[test]
fn dragging_a_smooth_anchors_handle_keeps_the_other_opposite_and_alt_breaks_it() {
    let mut s = raw_session();
    s_curve(&mut s);
    let a = Vec2::new(100.0, 0.0);
    let hs = handles_at(&s, a);
    assert_eq!(hs.len(), 2, "{hs:?}");
    let (h, k) = (hs[0], hs[1]);
    let len_k = k.at.distance(a);
    run(&mut s, "raw.move_handle", json!({"id": h.id.0, "cmd": h.cmd, "slot": h.slot, "dx": 0, "dy": 30}));
    let hs = handles_at(&s, a);
    assert_eq!(hs.len(), 2);
    let (moved, other) = if hs[0].at.distance(h.at + Vec2::new(0.0, 30.0)) < 0.5 { (hs[0], hs[1]) } else { (hs[1], hs[0]) };
    assert!(moved.at.distance(h.at + Vec2::new(0.0, 30.0)) < 0.5, "{hs:?}");
    // Colinear: on one line through the anchor, opposite, the other's length kept.
    assert!(rawgeom::smooth_at(a, moved.at, other.at), "{moved:?} {other:?}");
    assert!((moved.at - a).normalized().cross((other.at - a).normalized()).abs() < 1e-3);
    assert!((other.at.distance(a) - len_k).abs() < 0.3);
    // Alt (smooth: false) moves the handle alone: a corner.
    let before = other.at;
    run(&mut s, "raw.move_handle", json!({"id": moved.id.0, "cmd": moved.cmd, "slot": moved.slot, "dx": 30, "dy": 0, "smooth": false}));
    let hs = handles_at(&s, a);
    assert!(hs.iter().any(|x| x.at.distance(before) < 0.3), "the other stays: {hs:?}");
    assert!(!rawgeom::smooth_at(a, hs[0].at, hs[1].at));
}

#[test]
fn make_corner_and_make_smooth_on_raw_anchors() {
    let mut s = raw_session();
    s_curve(&mut s);
    let a = Vec2::new(100.0, 0.0);
    let v = run(&mut s, "raw.convert_points", json!({"points": [[a.x, a.y]], "smooth": false}));
    assert_eq!(v["converted"], json!(1), "{v}");
    assert!(handles_at(&s, a).is_empty(), "the handles are drawn in");
    let v = run(&mut s, "raw.convert_points", json!({"points": [[a.x, a.y]], "smooth": true}));
    assert_eq!(v["converted"], json!(1), "{v}");
    let hs = handles_at(&s, a);
    assert_eq!(hs.len(), 2, "{hs:?}");
    assert!(rawgeom::smooth_at(a, hs[0].at, hs[1].at));
    // Smooth on a fill's corner: the fill and both sides it has there follow.
    run(&mut s, "draw.rect", json!({"x": 300, "y": 0, "w": 100, "h": 80, "fill": "#ff0000", "stroke": "#000000"}));
    let v = run(&mut s, "raw.convert_points", json!({"points": [[300, 0]], "smooth": true}));
    assert_eq!(v["converted"], json!(1));
    let fill = raw(&s).into_iter().find(|e| is_fill(e)).unwrap_or_else(|| panic!("a fill"));
    assert!(area(&fill) > 8000.0, "the corner rounds out: {}", area(&fill));
    for e in raw(&s).iter().filter(|e| !is_fill(e) && e.transform.apply(Vec2::ZERO).x > 250.0) {
        let ElementKind::Shape(sh) = &e.kind else { continue };
        let ElementKind::Shape(fs) = &fill.kind else { continue };
        for pl in sh.path.flatten(&e.transform, 0.05) {
            for p in pl.points {
                assert!(fs.path.distance_to_edge(p, &fill.transform) < 0.3, "{p:?} off the fill's edge");
            }
        }
    }
}

#[test]
fn drawing_snaps_to_anchors_then_lines_while_snap_to_objects_is_on() {
    let mut s = raw_session();
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 100, "h": 80, "fill": "#ff0000", "stroke": "#000000"}));
    let v = run(&mut s, "draw.snap", json!({"x": 104, "y": 3, "snap": 10}));
    assert_eq!(v["snapped"]["to"], json!("point"), "{v}");
    assert_eq!((v["x"].as_f64(), v["y"].as_f64()), (Some(100.0), Some(0.0)));
    let v = run(&mut s, "draw.snap", json!({"x": 50, "y": 84, "snap": 10}));
    assert_eq!(v["snapped"]["to"], json!("line"), "{v}");
    assert!((v["y"].as_f64().unwrap_or(0.0) - 80.0).abs() < 1e-3 && (v["x"].as_f64().unwrap_or(0.0) - 50.0).abs() < 1e-3);
    let v = run(&mut s, "draw.snap", json!({"x": 300, "y": 300, "snap": 10}));
    assert!(v["snapped"].is_null());
    // Off, nothing snaps.
    run(&mut s, "draw.snap_to_objects", json!({"on": false}));
    let v = run(&mut s, "draw.snap", json!({"x": 104, "y": 3, "snap": 10}));
    assert!(v["snapped"].is_null() && v["x"] == json!(104.0));
    // A line drawn from a snapped point shares the rectangle's corner.
    run(&mut s, "draw.snap_to_objects", json!({"on": true}));
    let a = run(&mut s, "draw.snap", json!({"x": 98, "y": 78, "snap": 10}));
    run(&mut s, "draw.line", json!({"x0": a["x"], "y0": a["y"], "x1": 200, "y1": 200, "stroke": "#000000"}));
    assert!(has(&anchors(&s), (100.0, 80.0)) && anchors(&s).len() == 5, "{:?}", anchors(&s));
}

#[test]
fn an_object_paths_smooth_anchor_keeps_its_handles_opposite() {
    let mut s = Session::new(anicraft_doc::Document::default());
    let v = run(&mut s, "draw.path", json!({"d": "M0 0 C0 -50 60 -50 100 0 C140 50 200 50 200 0", "stroke": "#000000", "fill": null}));
    let id = v["element"].as_u64().unwrap_or(0);
    let el = s.visible_elements().into_iter().find(|e| e.id.0 == id).unwrap_or_else(|| panic!("the path"));
    let ElementKind::Shape(sh) = &el.kind else { panic!() };
    let p = sh.path.clone();
    // The in-handle of anchor 1 (C2 of command 1) moves; command 2's C1 turns with it.
    run(&mut s, "path.move_points", json!({"id": id, "points": [[1, 2]], "dx": 0, "dy": 25}));
    let el = s.visible_elements().into_iter().find(|e| e.id.0 == id).unwrap_or_else(|| panic!("the path"));
    let ElementKind::Shape(sh) = &el.kind else { panic!() };
    let (a, h, k) = (sh.path.point(1, 0), sh.path.point(1, 2), sh.path.point(2, 1));
    let (Some(a), Some(h), Some(k)) = (a, h, k) else { panic!("points") };
    assert!(rawgeom::smooth_at(a, h, k), "{a:?} {h:?} {k:?}");
    assert!((k.distance(a) - p.point(2, 1).map_or(0.0, |q| q.distance(a))).abs() < 1e-3, "length kept");
    run(&mut s, "path.move_points", json!({"id": id, "points": [[1, 2]], "dx": 30, "dy": 0, "smooth": false}));
    let el = s.visible_elements().into_iter().find(|e| e.id.0 == id).unwrap_or_else(|| panic!("the path"));
    let ElementKind::Shape(sh) = &el.kind else { panic!() };
    assert_eq!(sh.path.point(2, 1), Some(k), "Alt moves the handle alone");
}

#[test]
fn raw_points_timings() {
    // ADR 0042's drawing: 12 × 12 outlined, rounded squares and a line across every row.
    let mut s = raw_session();
    for i in 0..12 {
        for j in 0..12 {
            run(&mut s, "draw.rect", json!({"x": i * 50, "y": j * 50, "w": 40, "h": 40, "fill": "#ff0000", "stroke": "#000000", "radius": 6}));
        }
        run(&mut s, "draw.line", json!({"x0": -10, "y0": i * 50 + 20, "x1": 610, "y1": i * 50 + 20, "stroke": "#0000ff"}));
    }
    run(&mut s, "edit.deselect", json!({}));
    let n = s.visible_elements().len();
    let ms = |t: std::time::Instant| t.elapsed().as_secs_f64() * 1e3;
    let t = std::time::Instant::now();
    let v = run(&mut s, "raw.add_point", json!({"x": 320, "y": 300}));
    let add = ms(t);
    let at = [v["x"].as_f64().unwrap_or(0.0), v["y"].as_f64().unwrap_or(0.0)];
    let t = std::time::Instant::now();
    run(&mut s, "raw.delete_points", json!({"points": [at]}));
    let delete = ms(t);
    let t = std::time::Instant::now();
    run(&mut s, "raw.convert_points", json!({"points": [[306, 300]], "smooth": true}));
    let smooth = ms(t);
    let h = handles_at(&s, Vec2::new(306.0, 300.0)).first().copied().unwrap_or_else(|| panic!("handles after Make Smooth"));
    s.begin_gesture("Move Handle");
    let t = std::time::Instant::now();
    run(&mut s, "raw.move_handle", json!({"id": h.id.0, "cmd": h.cmd, "slot": h.slot, "dx": 1, "dy": 1, "merge": false}));
    let pair = ms(t);
    s.end_gesture();
    let t = std::time::Instant::now();
    for k in 0..100 {
        let _ = s.execute("draw.snap", &json!({"x": 3 + k * 6, "y": 303, "snap": 10}));
    }
    let snap = ms(t) / 100.0;
    // Inside a symbol: the whole drawing converted, opened in place, and its stage picture.
    run(&mut s, "edit.select_all", json!({}));
    run(&mut s, "modify.convert_to_symbol", json!({"name": "Grid"}));
    let t = std::time::Instant::now();
    run(&mut s, "symbol.edit", json!({}));
    let open = ms(t);
    let t = std::time::Instant::now();
    let pic = s.stage_picture();
    let picture = ms(t);
    let a = anchors(&s).first().copied().unwrap_or_else(|| panic!("anchors inside"));
    let t = std::time::Instant::now();
    run(&mut s, "raw.move_point", json!({"x": a.x, "y": a.y, "to_x": a.x - 5.0, "to_y": a.y}));
    let inside = ms(t);
    eprintln!("{n} raw shapes; add point {add:.2} ms, delete point {delete:.2} ms, make smooth {smooth:.2} ms, smooth handle frame {pair:.2} ms, draw snap {snap:.3} ms; inside a symbol: open {open:.2} ms, stage picture {picture:.2} ms (frame {}), point move {inside:.2} ms", pic.frame);
}

#[test]
fn point_edits_keep_a_dashed_lines_style() {
    let mut s = raw_session();
    // A dashed, butt-capped, mitred zigzag (ADR 0046).
    run(&mut s, "draw.path", json!({"d": "M0 0 L50 40 L100 0 L150 40", "stroke": "#000000", "fill": null, "stroke_width": 2, "cap": "none", "join": "miter", "dash": [6, 3]}));
    let styled = |s: &Session| {
        raw(s).iter().all(|e| match &e.kind {
            ElementKind::Shape(sh) => sh.stroke.as_ref().is_some_and(|st| st.style.dash == anicraft_doc::Dash::dashed(6.0, 3.0) && st.style.cap == anicraft_doc::Cap::Butt && st.style.join == anicraft_doc::Join::Miter),
            _ => false,
        })
    };
    run(&mut s, "raw.add_point", json!({"x": 25, "y": 20}));
    run(&mut s, "raw.delete_points", json!({"points": [[50, 40]]}));
    run(&mut s, "raw.convert_points", json!({"points": [[100, 0]], "smooth": true}));
    assert!(!raw(&s).is_empty() && styled(&s), "every piece keeps the line's style");
}
