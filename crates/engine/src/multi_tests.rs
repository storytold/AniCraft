//! Edit Multiple Frames, complete (ADR 0051): transforms about one shared pivot, align, delete,
//! copy, cut and paste per keyframe, raw shapes merged again on their keyframes, and all of it
//! (with onion skin) inside a symbol open in place.

use super::*;
use crate::Session;
use crate::onion::OnionSkin;
use anicraft_doc::{ElementKind, Keyframe, LayerId};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

/// One layer, a 20 × 20 drawing object walking right: keyframes on frames 0, 4 and 8 with it at
/// x = 10, 50 and 90 (y = 10). The playhead is on frame 4.
fn walk() -> (Session, LayerId, ElementId) {
    let mut s = Session::default();
    let layer = s.layer;
    let id = ElementId(run(&mut s, "draw.rect", json!({"x": 10, "y": 10, "w": 20, "h": 20, "fill": "#000000", "stroke": null, "object": true}))["element"].as_u64().unwrap());
    for f in [4, 8] {
        run(&mut s, "timeline.goto", json!({"frame": f}));
        run(&mut s, "timeline.insert_keyframe", json!({}));
        run(&mut s, "element.select", json!({"ids": [id.0]}));
        run(&mut s, "element.translate", json!({"dx": 40, "dy": 0}));
    }
    run(&mut s, "timeline.goto", json!({"frame": 4}));
    (s, layer, id)
}

fn key(s: &Session, layer: LayerId, start: u32) -> Keyframe {
    let l = s.timeline().layer(layer).unwrap();
    (**l.keyframes.iter().find(|k| k.start == start).unwrap()).clone()
}

/// The bounds of every element on the keyframe of `layer` starting at `start`.
fn bounds_on(s: &Session, layer: LayerId, start: u32) -> Vec<Rect> {
    key(s, layer, start).elements.iter().map(|e| e.bounds(&s.doc)).collect()
}

fn near(a: Rect, b: [f32; 4]) -> bool {
    [a.min.x, a.min.y, a.max.x, a.max.y].iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-3)
}

#[test]
fn a_transform_about_the_shared_pivot_lands_each_keyframes_artwork_exactly() {
    let (mut s, a, _) = walk();
    let steps = s.history_len();
    // Together the three copies span x 10..110, y 10..30: the pivot is (60, 20).
    let v = run(&mut s, "frames.transform", json!({"first": 0, "last": 8, "scale_x": 2, "scale_y": 0.5}));
    assert_eq!((v["transformed"].as_u64(), v["keyframes"].as_u64()), (Some(3), Some(3)));
    // x' = 60 + 2 (x − 60), y' = 20 + 0.5 (y − 20).
    for (f, x) in [(0, 10.0), (4, 50.0), (8, 90.0)] {
        let b = bounds_on(&s, a, f)[0];
        assert!(near(b, [60.0 + 2.0 * (x - 60.0), 15.0, 60.0 + 2.0 * (x + 20.0 - 60.0), 25.0]), "keyframe {f}: {b:?}");
    }
    assert_eq!(s.history_len(), steps + 1, "one undo step");
    s.undo().unwrap();
    // A quarter turn (clockwise on the y-down stage): (x, y) → (60 − (y − 20), 20 + (x − 60)).
    run(&mut s, "frames.transform", json!({"first": 0, "last": 8, "rotate": 90}));
    for (f, x) in [(0, 10.0), (4, 50.0), (8, 90.0)] {
        let b = bounds_on(&s, a, f)[0];
        let (cx, cy) = (x + 10.0, 20.0);
        let (nx, ny) = (60.0 - (cy - 20.0), 20.0 + (cx - 60.0));
        assert!(near(b, [nx - 10.0, ny - 10.0, nx + 10.0, ny + 10.0]), "keyframe {f}: {b:?}");
    }
    // Its own pivot, and a matrix, as Free Transform sends.
    s.undo().unwrap();
    let on8 = key(&s, a, 8).elements[0].id.0;
    run(&mut s, "frames.transform", json!({"first": 0, "last": 8, "scale_x": 2, "pivot": [10, 10], "items": [[a.0, 8, on8]]}));
    assert!(near(bounds_on(&s, a, 8)[0], [170.0, 10.0, 210.0, 30.0]));
    assert!(near(bounds_on(&s, a, 0)[0], [10.0, 10.0, 30.0, 30.0]), "only the item given");
    run(&mut s, "frames.transform", json!({"first": 0, "last": 4, "a": 1, "b": 0, "c": 0, "d": 1, "e": 5, "f": -5}));
    assert!(near(bounds_on(&s, a, 0)[0], [15.0, 5.0, 35.0, 25.0]));
    // Degenerate or wild transforms are refused, with nothing changed.
    let before = s.doc.clone();
    assert!(s.execute("frames.transform", &json!({"first": 0, "last": 8, "a": 0, "b": 0, "c": 0, "d": 0, "e": 0, "f": 0})).is_err());
    assert!(s.execute("frames.transform", &json!({"first": 0, "last": 8, "scale_x": 0})).is_err());
    assert!(s.execute("frames.transform", &json!({"first": 0, "last": 8, "pivot": [1]})).is_err());
    assert!(s.doc == before);
}

#[test]
fn align_lines_every_keyframes_artwork_up() {
    let (mut s, a, _) = walk();
    run(&mut s, "frames.edit_multiple", json!({"first": 0, "last": 8, "dx": 0, "dy": 0}));
    let v = run(&mut s, "frames.align", json!({"first": 0, "last": 8, "edge": "left"}));
    assert_eq!(v["moved"], json!(2), "the one already at the left edge stays");
    for f in [0, 4, 8] {
        assert!(near(bounds_on(&s, a, f)[0], [10.0, 10.0, 30.0, 30.0]), "keyframe {f}");
    }
    s.undo().unwrap();
    run(&mut s, "frames.align", json!({"first": 0, "last": 8, "edge": "bottom", "to_stage": true}));
    let h = s.doc.stage.height as f32;
    assert!([0, 4, 8].iter().all(|&f| (bounds_on(&s, a, f)[0].max.y - h).abs() < 1e-3));
    assert!(s.execute("frames.align", &json!({"first": 0, "last": 8, "edge": "middle"})).is_err());
}

#[test]
fn delete_copy_cut_and_paste_count_per_keyframe() {
    let (mut s, a, id) = walk();
    // A second element on keyframes 0 and 8 only.
    run(&mut s, "timeline.goto", json!({"frame": 0}));
    run(&mut s, "draw.rect", json!({"x": 10, "y": 100, "w": 10, "h": 10, "object": true}));
    run(&mut s, "timeline.goto", json!({"frame": 8}));
    run(&mut s, "draw.rect", json!({"x": 90, "y": 100, "w": 10, "h": 10, "object": true}));
    run(&mut s, "timeline.goto", json!({"frame": 4}));
    let count = |s: &Session| [0, 4, 8].map(|f| key(s, a, f).elements.len());
    assert_eq!(count(&s), [2, 1, 2]);

    // Copy the walker on 0 and 8 (with its keyframes), cut everything else.
    let walker = |f| json!([a.0, f, id.0]);
    let v = run(&mut s, "frames.copy", json!({"first": 0, "last": 8, "items": [walker(0), walker(8)]}));
    assert_eq!((v["copied"].as_u64(), v["keyframes"].as_u64()), (Some(2), Some(2)));
    assert_eq!(s.clipboard.layers.iter().map(|c| c.frame).collect::<Vec<_>>(), vec![Some(0), Some(8)]);
    let steps = s.history_len();
    let v = run(&mut s, "frames.delete", json!({"first": 0, "last": 8, "items": [walker(0), walker(4), walker(8)]}));
    assert_eq!((v["deleted"].as_u64(), v["keyframes"].as_u64()), (Some(3), Some(3)));
    assert_eq!(count(&s), [1, 0, 1]);
    assert_eq!(s.history_len(), steps + 1, "one undo step");

    // Paste in place: back keyframe by keyframe, where each came from.
    let v = run(&mut s, "frames.paste", json!({"first": 0, "last": 8}));
    assert_eq!((v["pasted"].as_u64(), v["keyframes"].as_u64(), v["skipped"].as_u64()), (Some(2), Some(2), Some(0)));
    assert_eq!(count(&s), [2, 0, 2]);
    assert!(near(bounds_on(&s, a, 8)[1], [90.0, 10.0, 110.0, 30.0]), "in place");
    assert_eq!(v["items"].as_array().map(Vec::len), Some(2), "what was pasted is what is selected");
    // Markers that leave out frame 8: only the copy from 0 goes back.
    let v = run(&mut s, "frames.paste", json!({"first": 0, "last": 3}));
    assert_eq!((v["pasted"].as_u64(), v["skipped"].as_u64()), (Some(1), Some(1)));
    assert_eq!(count(&s), [3, 0, 2]);

    // Copied on one frame: onto every keyframe shown.
    run(&mut s, "timeline.goto", json!({"frame": 0}));
    let small = key(&s, a, 0).elements[0].id;
    run(&mut s, "element.select", json!({"ids": [small.0]}));
    run(&mut s, "edit.copy", json!({}));
    let v = run(&mut s, "frames.paste", json!({"first": 0, "last": 8}));
    assert_eq!((v["pasted"].as_u64(), v["keyframes"].as_u64()), (Some(3), Some(3)));
    assert_eq!(count(&s), [4, 1, 3]);

    // Cut: copies, then deletes, as one step.
    let steps = s.history_len();
    let v = run(&mut s, "frames.cut", json!({"first": 0, "last": 8}));
    assert_eq!(v["cut"], json!(8));
    assert_eq!(count(&s), [0, 0, 0]);
    assert_eq!(s.history_len(), steps + 1);
    // A plain Paste puts a multi-frame copy on the current frame, on the active layer.
    run(&mut s, "timeline.goto", json!({"frame": 4}));
    run(&mut s, "edit.paste_in_place", json!({}));
    assert_eq!(count(&s), [0, 8, 0]);

    // Nothing to act on, or nowhere to paste: refused.
    run(&mut s, "layer.toggle_lock", json!({"layer": a.0}));
    assert!(s.execute("frames.paste", &json!({"first": 0, "last": 8})).is_err(), "a locked layer");
}

/// Raw (merge-drawing) rectangles, the same colour, on keyframes 0 and 5: one at x 0..40 and one
/// at x 100..140 (y 0..40). Their ids on each keyframe.
fn raw_pair() -> (Session, LayerId) {
    let mut s = Session::default();
    s.style.object_drawing = false;
    let layer = s.layer;
    for x in [0, 100] {
        run(&mut s, "draw.rect", json!({"x": x, "y": 0, "w": 40, "h": 40, "fill": "#3366ff", "stroke": null}));
    }
    run(&mut s, "timeline.goto", json!({"frame": 5}));
    run(&mut s, "timeline.insert_keyframe", json!({}));
    run(&mut s, "timeline.goto", json!({"frame": 0}));
    (s, layer)
}

fn raw_fills(s: &Session, layer: LayerId, start: u32) -> Vec<Rect> {
    key(s, layer, start).elements.iter().filter(|e| matches!(&e.kind, ElementKind::Shape(sh) if sh.merge && sh.fill.is_some())).map(|e| e.bounds(&s.doc)).collect()
}

#[test]
fn raw_shapes_merge_again_on_each_keyframe_after_a_move_or_transform() {
    let (mut s, a) = raw_pair();
    assert_eq!(raw_fills(&s, a, 0).len(), 2);
    let right = |s: &Session, f: u32| key(s, a, f).elements.iter().find(|e| e.bounds(&s.doc).min.x > 50.0).map(|e| json!([a.0, f, e.id.0])).unwrap();
    let items = json!([right(&s, 0), right(&s, 5)]);
    // Moved left by 80 it overlaps the other (x 20..60): one fill region on each keyframe.
    let v = run(&mut s, "frames.edit_multiple", json!({"first": 0, "last": 5, "dx": -80, "dy": 0, "items": items}));
    for f in [0, 5] {
        let fills = raw_fills(&s, a, f);
        assert_eq!(fills.len(), 1, "keyframe {f}: joined into one fill: {fills:?}");
        assert!(near(fills[0], [0.0, 0.0, 60.0, 40.0]), "keyframe {f}: {:?}", fills[0]);
    }
    // The selection is the merged pieces, with their new ids.
    let after: Vec<Value> = v["items"].as_array().unwrap().clone();
    assert_eq!(after.len(), 2);
    for it in &after {
        let f = it[1].as_u64().unwrap() as u32;
        assert!(key(&s, a, f).elements.iter().any(|e| json!(e.id.0) == it[2]), "{it} is on its keyframe");
    }
    s.undo().unwrap();
    assert_eq!(raw_fills(&s, a, 5).len(), 2, "one undo takes the move and the merge back");

    // A drag holds the merge back (merge: false) and merges when it ends (frames.merge).
    let items = json!([right(&s, 0), right(&s, 5)]);
    s.begin_gesture("Move");
    for _ in 0..4 {
        run(&mut s, "frames.edit_multiple", json!({"first": 0, "last": 5, "dx": -20, "dy": 0, "items": items, "merge": false}));
    }
    assert_eq!(raw_fills(&s, a, 0).len(), 2, "not merged mid-drag");
    let v = run(&mut s, "frames.merge", json!({"first": 0, "last": 5, "items": items}));
    s.end_gesture();
    assert_eq!([0, 5].map(|f| raw_fills(&s, a, f).len()), [1, 1]);
    assert_eq!(v["items"].as_array().map(Vec::len), Some(2));
    assert!(s.execute("frames.merge", &json!({"first": 0, "last": 5})).is_err(), "items are required");

    // A transform merges too: the right square scaled 3× about its right edge reaches over.
    s.undo().unwrap();
    let items = json!([right(&s, 0), right(&s, 5)]);
    run(&mut s, "frames.transform", json!({"first": 0, "last": 5, "items": items, "scale_x": 3, "pivot": [140, 0]}));
    for f in [0, 5] {
        let fills = raw_fills(&s, a, f);
        assert_eq!(fills.len(), 1, "keyframe {f}: {fills:?}");
        assert!(near(fills[0], [0.0, 0.0, 140.0, 40.0]), "keyframe {f}: {:?}", fills[0]);
    }
}

#[test]
fn a_motion_tweens_object_moves_but_does_not_scale_on_several_frames() {
    let mut s = Session::default();
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 20, "h": 20, "object": true}));
    run(&mut s, "timeline.insert_frame", json!({"count": 9}));
    run(&mut s, "timeline.create_motion_tween", json!({}));
    let before = s.doc.clone();
    let e = s.execute("frames.transform", &json!({"first": 0, "last": 9, "rotate": 30}));
    assert!(e.is_err_and(|e| e.to_string().contains("motion tween")));
    assert!(s.doc == before);
    run(&mut s, "frames.transform", json!({"first": 0, "last": 9, "a": 1, "b": 0, "c": 0, "d": 1, "e": 10, "f": 0}));
}

/// A symbol open in place (ADR 0045) whose own timeline has keyframes on 0 and 4, the playhead on 4.
fn in_symbol() -> (Session, LayerId, ElementId) {
    let mut s = Session::default();
    let id = ElementId(run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 20, "h": 20, "fill": "#000000", "stroke": null, "object": true}))["element"].as_u64().unwrap());
    run(&mut s, "modify.convert_to_symbol", json!({"name": "Walker"}));
    run(&mut s, "element.translate", json!({"dx": 100, "dy": 50}));
    run(&mut s, "symbol.edit", json!({}));
    let inner = s.layer;
    run(&mut s, "timeline.goto", json!({"frame": 4}));
    run(&mut s, "timeline.insert_keyframe", json!({}));
    run(&mut s, "element.select", json!({"ids": [id.0]}));
    run(&mut s, "element.translate", json!({"dx": 40, "dy": 0}));
    assert_eq!(s.editing.len(), 1);
    (s, inner, id)
}

#[test]
fn inside_a_symbol_the_commands_edit_its_own_keyframes() {
    let (mut s, inner, id) = in_symbol();
    let shown = s.multi_frame_items(0, 4);
    assert_eq!(shown.iter().map(|(it, _)| it.frame).collect::<Vec<_>>(), vec![0, 4], "the symbol's keyframes");
    run(&mut s, "frames.transform", json!({"first": 0, "last": 4, "scale_x": 2, "scale_y": 2}));
    // In the symbol's space (its registration point at the square's centre) the two copies
    // span x −10..50, y −10..10 together: the pivot is (20, 0).
    assert!(near(bounds_on(&s, inner, 0)[0], [-40.0, -20.0, 0.0, 20.0]));
    assert!(near(bounds_on(&s, inner, 4)[0], [40.0, -20.0, 80.0, 20.0]));
    run(&mut s, "frames.copy", json!({"first": 0, "last": 4, "items": [[inner.0, 0, id.0]]}));
    run(&mut s, "frames.paste", json!({"first": 0, "last": 4}));
    assert_eq!(key(&s, inner, 0).elements.len(), 2);
    run(&mut s, "frames.delete", json!({"first": 0, "last": 4}));
    assert!([0, 4].iter().all(|&f| key(&s, inner, f).elements.is_empty()));
    // The scene's own timeline never changed shape: one layer, one instance.
    assert_eq!(s.doc.timeline(s.scene).layers.len(), 1);
}

#[test]
fn onion_skin_inside_a_symbol_composes_its_own_frames_where_it_stands() {
    let (s, _, _) = in_symbol();
    let pic = s.stage_picture();
    let front = pic.front.clone().expect("the open symbol is in front");
    assert_eq!(front.frame, 4);
    let plain = crate::render_service::display_list_with_onion(&pic.doc, pic.frame, None);
    let ghosts = OnionSkin { markers: crate::onion::Markers::Relative { before: 4, after: 0 }, keyframes_only: true, ..OnionSkin::default() };
    let dl = crate::render_service::display_list_inside(&pic.doc, pic.frame, &front, &ghosts);
    assert_eq!(dl.items.len(), plain.items.len() + 1, "frame 0 of the symbol under its frame 4");
    // The symbol's origin stands at x 110: its frame 0 (x −10..10) shows at x 100..120, tinted
    // and faded; its frame 4 (x 30..50) at 140..160 at full strength, on top.
    let left = |i: &anicraft_doc::DisplayItem| i.path.bounds().transformed(&i.transform).min.x;
    let n = dl.items.len();
    let (ghost, now) = (&dl.items[n - 2], &dl.items[n - 1]);
    assert_eq!((left(ghost), left(now)), (100.0, 140.0));
    assert!(ghost.color.mul[3] < 1.0 && ghost.color.changes_colour() && now.color.is_identity());
    // The job does the same, keyed apart from the plain picture.
    let view = anicraft_render::View { transform: Affine::IDENTITY, width: 64, height: 64, background: None };
    let plain_job = crate::render_service::make_job(pic.state, &pic.doc, pic.frame, view, Some(&ghosts));
    let job = plain_job.clone().inside(pic.front.clone());
    assert_ne!(job.key, plain_job.key);
    assert_eq!(job.display_list().items.len(), dl.items.len());
    // Edit Multiple Frames shows both keyframes at full strength.
    let multi = OnionSkin { edit_multiple: true, markers: crate::onion::Markers::Relative { before: 4, after: 0 }, ..OnionSkin::default() };
    let dl = crate::render_service::display_list_inside(&pic.doc, pic.frame, &front, &multi);
    assert!(dl.items.iter().rev().take(2).all(|i| i.color.is_identity()));
    // With nothing to add, the job draws the picture as it is.
    let off = make_off(&pic);
    assert_eq!(off.display_list().items.len(), plain.items.len());
}

fn make_off(pic: &crate::editing::StagePicture) -> crate::render_service::FrameJob {
    let view = anicraft_render::View { transform: Affine::IDENTITY, width: 64, height: 64, background: None };
    crate::render_service::make_job(pic.state, &pic.doc, pic.frame, view, None).inside(pic.front.clone())
}


/// Measured numbers for ADR 0051: `cargo test -p anicraft-engine --lib multi_timings -- --ignored --nocapture`.
#[test]
#[ignore]
fn multi_timings() {
    use std::time::Instant;
    // 24 keyframes, each with 12 × 12 raw squares and 50 objects.
    let mut s = Session::default();
    s.style.object_drawing = false;
    for i in 0..144 {
        let (x, y) = ((i % 12) as f32 * 50.0, (i / 12) as f32 * 50.0);
        run(&mut s, "draw.rect", json!({"x": x, "y": y, "w": 30, "h": 30, "fill": "#3366ff", "stroke": null}));
    }
    for i in 0..50 {
        run(&mut s, "draw.rect", json!({"x": 700 + i, "y": i * 10, "w": 20, "h": 20, "object": true}));
    }
    for f in 1..24 {
        run(&mut s, "timeline.goto", json!({"frame": f * 2}));
        run(&mut s, "timeline.insert_keyframe", json!({}));
    }
    let last = 47;
    let t = |label: &str, f: &mut dyn FnMut(&mut Session)| {
        let mut s2 = Session::new(s.doc.clone());
        let t0 = Instant::now();
        f(&mut s2);
        eprintln!("{label}: {:.2} ms", t0.elapsed().as_secs_f64() * 1000.0);
    };
    let objects = |s: &Session| -> Vec<Value> { s.multi_frame_items(0, last).into_iter().filter(|(it, _)| key(s, it.layer, it.frame).elements.iter().any(|e| e.id == it.element && !matches!(&e.kind, ElementKind::Shape(sh) if sh.merge))).map(|(it, _)| it.to_json()).collect() };
    let raws = |s: &Session| -> Vec<Value> { s.multi_frame_items(0, last).into_iter().filter(|(it, _)| key(s, it.layer, it.frame).elements.iter().any(|e| e.id == it.element && matches!(&e.kind, ElementKind::Shape(sh) if sh.merge))).map(|(it, _)| it.to_json()).take(24 * 12).collect() };
    let (objects, raws) = (objects(&s), raws(&s));
    eprintln!("items shown: {}, objects: {}, raw: {}", s.multi_frame_items(0, last).len(), objects.len(), raws.len());
    t("list what is shown (24 keyframes)", &mut |s| drop(s.multi_frame_items(0, last)));
    t("move 1,200 objects, a drag step", &mut |s| drop(run(s, "frames.edit_multiple", json!({"first": 0, "last": last, "dx": 1, "dy": 0, "items": objects, "merge": false}))));
    t("rotate 1,200 objects", &mut |s| drop(run(s, "frames.transform", json!({"first": 0, "last": last, "rotate": 15, "items": objects}))));
    t("move and merge 288 raw squares (24 keyframes)", &mut |s| drop(run(s, "frames.edit_multiple", json!({"first": 0, "last": last, "dx": 25, "dy": 0, "items": raws}))));
    t("copy 1,200 objects, paste in place", &mut |s| {
        run(s, "frames.copy", json!({"first": 0, "last": last, "items": objects}));
        run(s, "frames.paste", json!({"first": 0, "last": last}));
    });
    t("delete everything shown", &mut |s| drop(run(s, "frames.delete", json!({"first": 0, "last": last}))));
}
