//! Onion skin and Edit Multiple Frames (ADR 0047): markers, tint and fade, outlines, keyframes
//! only, the composed display list and its render, and moves on several keyframes.

use super::*;
use crate::Session;
use anicraft_doc::{DisplayItem, ElementId, LayerId, Rgba, evaluate_scene, sample};
use anicraft_geom::Affine;
use anicraft_render::{View, render};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

/// One layer, a 20 × 20 drawing object walking right: keyframes on frames 0, 4 and 8 (the last
/// one frame long) with it at x = 10, 50 and 90 (y = 10). The playhead is on frame 4.
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
    assert_eq!(s.frame_count(), 9);
    (s, layer, id)
}

/// The left edge of `id` on the keyframe of `layer` starting at `start`.
fn x_at(s: &Session, layer: LayerId, start: u32, id: ElementId) -> f32 {
    let l = s.doc.timeline(s.scene).layer(layer).unwrap();
    let k = l.keyframes.iter().find(|k| k.start == start).unwrap();
    let e = k.elements.iter().find(|e| e.id == id).unwrap();
    e.bounds(&s.doc).min.x
}

fn left(i: &DisplayItem) -> f32 {
    i.path.bounds().transformed(&i.transform).min.x
}

#[test]
fn markers_stay_inside_the_timeline() {
    let mut o = OnionSkin::default();
    assert_eq!(o.range(0, 10), (0, 2), "nothing before frame 1");
    assert_eq!(o.range(9, 10), (7, 9), "nothing after the last frame");
    assert_eq!(o.range(50, 10), (7, 9), "a playhead past the end counts as the last frame");
    assert_eq!(o.range(0, 0), (0, 0));
    o.markers = Markers::Relative { before: u32::MAX, after: u32::MAX };
    assert_eq!(o.range(5, 10), (0, 9));
    o.markers = Markers::Anchored { first: 8, last: 3 };
    assert_eq!(o.range(0, 10), (3, 8), "anchored markers in order, whatever the playhead");
    o.markers = Markers::Anchored { first: 400, last: 900 };
    assert_eq!(o.range(0, 10), (9, 9));
    o.all();
    assert!(o.is_all() && o.anchored());
    assert_eq!(o.range(4, 10), (0, 9));
    assert_eq!(o.range(4, 500), (0, 499), "Onion All grows with the timeline");
    o.preset(5);
    assert!(!o.anchored() && !o.is_all());
    assert_eq!(o.range(10, 100), (5, 15));
}

#[test]
fn markers_drag_and_anchor() {
    let mut o = OnionSkin::default();
    o.drag_marker(true, 1, 6, 20);
    assert_eq!(o.range(6, 20), (1, 8));
    o.drag_marker(true, 9, 6, 20);
    assert_eq!(o.range(6, 20), (6, 8), "the start marker stops at the playhead");
    o.drag_marker(false, 50, 6, 20);
    assert_eq!(o.range(6, 20), (6, 19), "the end marker stops at the last frame");
    o.drag_marker(false, 2, 6, 20);
    assert_eq!(o.range(6, 20), (6, 6));
    // Anchoring keeps the frames; the playhead then moves without them.
    o.preset(2);
    o.set_anchored(true, 6, 20);
    assert_eq!((o.range(6, 20), o.range(15, 20)), ((4, 8), (4, 8)));
    o.drag_marker(true, 12, 15, 20);
    assert_eq!(o.range(15, 20), (8, 8), "never past the other marker");
    o.drag_marker(false, 3, 15, 20);
    assert_eq!(o.range(15, 20), (8, 8));
    o.drag_marker(false, 12, 15, 20);
    assert_eq!(o.range(15, 20), (8, 12));
    // Following the playhead again: the same distances from it...
    o.set_anchored(false, 10, 20);
    assert_eq!(o.markers, Markers::Relative { before: 2, after: 2 });
    // ...and a marker the playhead has passed lands on it.
    o.set_anchored(true, 10, 20);
    o.set_anchored(false, 15, 20);
    assert_eq!(o.range(15, 20), (8, 15));
}

#[test]
fn onion_frames_fade_by_distance_in_their_colours() {
    let o = OnionSkin::default();
    assert_eq!(o.alpha(1, 5), NEAR_ALPHA);
    assert_eq!(o.alpha(5, 5), FAR_ALPHA);
    assert!((o.alpha(3, 5) - (NEAR_ALPHA + FAR_ALPHA) / 2.0).abs() < 1e-6);
    assert!((1..5).all(|d| o.alpha(d, 5) > o.alpha(d + 1, 5)), "fainter with each frame");
    assert_eq!(o.alpha(1, 1), NEAR_ALPHA);
    assert_eq!(o.alpha(9, 5), FAR_ALPHA, "never fainter than the farthest");
    let flat = OnionSkin { fade: false, ..o };
    assert!((1..=5).all(|d| flat.alpha(d, 5) == NEAR_ALPHA));

    let (past, line) = o.look(true, 1, 2);
    let look = |c, a| anicraft_doc::ColorTransform::alpha(a).concat(&anicraft_doc::ColorTransform::tint(c, TINT_AMOUNT));
    assert_eq!((past, line), (look(o.past, NEAR_ALPHA), None));
    let (future, _) = o.look(false, 2, 2);
    assert_eq!(future, look(o.future, FAR_ALPHA));
    let (own, _) = OnionSkin { tint: false, ..o }.look(true, 1, 2);
    assert!(!own.changes_colour(), "untinted frames keep their own colours");
    let (_, line) = OnionSkin { outlines: true, ..o }.look(false, 2, 2);
    assert_eq!(line, Some(o.future.with_alpha(FAR_ALPHA)));
}

#[test]
fn every_setting_is_in_the_key() {
    let o = OnionSkin::default();
    let variants = [
        OnionSkin { ghosts: false, ..o },
        OnionSkin { edit_multiple: true, ..o },
        OnionSkin { markers: Markers::Relative { before: 2, after: 3 }, ..o },
        OnionSkin { markers: Markers::Anchored { first: 0, last: 2 }, ..o },
        OnionSkin { tint: false, ..o },
        OnionSkin { past: Rgba::hex(0x123456), ..o },
        OnionSkin { future: Rgba::hex(0x123456), ..o },
        OnionSkin { fade: false, ..o },
        OnionSkin { outlines: true, ..o },
        OnionSkin { keyframes_only: true, ..o },
    ];
    let mut keys: Vec<u64> = variants.iter().map(OnionSkin::key).collect();
    keys.push(o.key());
    let n = keys.len();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), n);
    assert_eq!(o.key(), OnionSkin::default().key());
}

#[test]
fn onion_frames_come_farthest_first_and_skip_locked_layers() {
    let (mut s, a, _) = walk();
    let b = LayerId(run(&mut s, "layer.new", json!({"name": "Locked"}))["layer"].as_u64().unwrap());
    run(&mut s, "draw.rect", json!({"x": 0, "y": 100, "w": 9, "h": 9, "object": true}));
    run(&mut s, "timeline.insert_frame", json!({"count": 8}));
    run(&mut s, "layer.toggle_lock", json!({"layer": b.0}));
    let tl = s.doc.timeline(s.scene);
    let got: Vec<(u32, Option<bool>, u32)> = onion_frames(tl, 4, &OnionSkin::default()).iter().map(|e| (e.frame, e.past, e.distance)).collect();
    assert_eq!(got, vec![(2, Some(true), 2), (6, Some(false), 2), (3, Some(true), 1), (5, Some(false), 1)]);
    assert!(onion_frames(tl, 4, &OnionSkin::default()).iter().all(|e| e.layers == vec![a]), "locked layers are not onion-skinned");
    // At the first frame there is no past.
    assert!(onion_frames(tl, 0, &OnionSkin::default()).iter().all(|e| e.past == Some(false)));
}

#[test]
fn keyframes_only_shows_each_layers_own_keyframes() {
    let (mut s, a, _) = walk();
    let b = LayerId(run(&mut s, "layer.new", json!({"name": "B"}))["layer"].as_u64().unwrap());
    run(&mut s, "timeline.goto", json!({"frame": 0}));
    run(&mut s, "draw.rect", json!({"x": 0, "y": 100, "w": 9, "h": 9, "object": true}));
    run(&mut s, "timeline.goto", json!({"frame": 2}));
    run(&mut s, "timeline.insert_keyframe", json!({}));
    run(&mut s, "timeline.insert_frame", json!({"count": 6}));
    let tl = s.doc.timeline(s.scene);
    let skin = OnionSkin { keyframes_only: true, markers: Markers::Relative { before: 5, after: 5 }, ..OnionSkin::default() };
    let mut got: Vec<(u32, Vec<LayerId>)> = onion_frames(tl, 4, &skin).into_iter().map(|e| (e.frame, e.layers)).collect();
    got.sort_by_key(|(f, _)| *f);
    // Frame 0 is a keyframe on both layers, frame 8 on A. B's keyframe on 2 is the one under the
    // playhead and doesn't tween: it would only repeat what the playhead shows, so it is left out.
    assert_eq!(got, vec![(0, vec![b, a]), (8, vec![a])]);
    // With all frames, every frame in range is there.
    let all = onion_frames(tl, 4, &OnionSkin { keyframes_only: false, ..skin }).len();
    let (f0, f1) = skin.range(4, tl.frame_count());
    assert_eq!(all as u32, f1 - f0);
}

#[test]
fn composed_onion_frames_are_tinted_faded_and_under_the_playhead() {
    let (s, _, _) = walk();
    let skin = OnionSkin::default();
    let dl = compose(&s.doc, 0, 4, Some(&skin), |sc, f| evaluate_scene(&s.doc, sc, f));
    // Frames 2, 6, 3, 5 under frame 4: the keyframes there hold the rectangle at 10, 50, 10, 50.
    let got: Vec<(f32, f32, f32)> = dl.items.iter().map(|i| (left(i), i.color.mul[3], 1.0 - i.color.mul[0])).collect();
    assert_eq!(got, vec![(10.0, FAR_ALPHA, TINT_AMOUNT), (50.0, FAR_ALPHA, TINT_AMOUNT), (10.0, NEAR_ALPHA, TINT_AMOUNT), (50.0, NEAR_ALPHA, TINT_AMOUNT), (50.0, 1.0, 0.0)]);
    let tint = |c| anicraft_doc::ColorTransform::tint(c, TINT_AMOUNT).add;
    assert_eq!(dl.items.first().map(|i| i.color.add), Some(tint(skin.past)));
    assert_eq!(dl.items.get(1).map(|i| i.color.add), Some(tint(skin.future)));
    // Off, or None: the frame alone.
    let off = OnionSkin { ghosts: false, ..skin };
    assert_eq!(compose(&s.doc, 0, 4, Some(&off), |sc, f| evaluate_scene(&s.doc, sc, f)).items.len(), 1);
    assert_eq!(compose(&s.doc, 0, 4, None, |sc, f| evaluate_scene(&s.doc, sc, f)).items.len(), 1);
}

/// The pixel at (x, y), premultiplied RGBA8.
fn px(rgba: &[u8], w: u32, x: u32, y: u32) -> [u8; 4] {
    let i = ((y * w + x) * 4) as usize;
    [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
}

#[test]
fn outline_mode_draws_onion_frames_as_edges_only() {
    let (s, _, _) = walk();
    let view = View { transform: Affine::IDENTITY, width: 120, height: 40, background: Some(Rgba::WHITE) };
    let draw = |skin: &OnionSkin| {
        let dl = compose(&s.doc, 0, 4, Some(skin), |sc, f| evaluate_scene(&s.doc, sc, f));
        render(&dl, &view).unwrap().0.to_premul_rgba8()
    };
    let filled = draw(&OnionSkin { markers: Markers::Relative { before: 4, after: 0 }, ..OnionSkin::default() });
    let lines = draw(&OnionSkin { markers: Markers::Relative { before: 4, after: 0 }, outlines: true, ..OnionSkin::default() });
    const WHITE: [u8; 4] = [255; 4];
    // Inside the onion rectangle (frames 0–3 hold it at 10..30): tinted when filled, the stage
    // when outlined; on its left edge (x = 10), the outline shows.
    assert_ne!(px(&filled, 120, 20, 20), WHITE);
    assert_eq!(px(&lines, 120, 20, 20), WHITE);
    assert_ne!(px(&lines, 120, 10, 20), WHITE);
    let edge = px(&lines, 120, 10, 20);
    assert!(edge[2] > edge[0], "in the past colour (blue): {edge:?}");
    // The playhead's frame (x = 50..70) is drawn as it is, either way.
    assert_eq!(px(&lines, 120, 60, 20), [0, 0, 0, 255]);
    assert_eq!(px(&filled, 120, 60, 20), [0, 0, 0, 255]);
}

#[test]
fn edit_multiple_frames_shows_keyframes_within_the_markers_at_full_strength() {
    let (mut s, a, id) = walk();
    let tl = s.doc.timeline(s.scene);
    let near = OnionSkin { edit_multiple: true, ..OnionSkin::default() };
    // ±2 around 4 is frames 2..6: the keyframe on 0 reaches into it (to frame 3), the one on 8
    // doesn't; the one on 4 is the playhead's.
    assert_eq!(multi_frames(tl, 4, 2, 6).iter().map(|e| e.frame).collect::<Vec<_>>(), vec![0]);
    let dl = compose(&s.doc, 0, 4, Some(&near), |sc, f| evaluate_scene(&s.doc, sc, f));
    assert_eq!(dl.items.iter().map(|i| (left(i), i.color.mul[3], 1.0 - i.color.mul[0])).collect::<Vec<_>>(), vec![(10.0, 1.0, 0.0), (50.0, 1.0, 0.0)]);
    let wide = OnionSkin { markers: Markers::Relative { before: 5, after: 5 }, ..near };
    let dl = compose(&s.doc, 0, 4, Some(&wide), |sc, f| evaluate_scene(&s.doc, sc, f));
    assert_eq!(dl.items.iter().map(left).collect::<Vec<_>>(), vec![10.0, 90.0, 50.0], "other keyframes in frame order, the playhead's on top");
    // What it shows is what it lists, each with its keyframe's first frame.
    let items: Vec<(FrameItem, f32)> = s.multi_frame_items(0, 8).into_iter().map(|(it, b)| (it, b.min.x)).collect();
    let item = |frame| FrameItem { layer: a, frame, element: id };
    assert_eq!(items, vec![(item(0), 10.0), (item(8), 90.0), (item(4), 50.0)]);
    let v = run(&mut s, "frames.artwork", json!({"first": 8, "last": 0}));
    assert_eq!(v["items"].as_array().map(Vec::len), Some(3));
    assert_eq!(v["items"][1], json!({"layer": a.0, "frame": 8, "id": id.0, "bounds": [90.0, 10.0, 110.0, 30.0]}));
}

#[test]
fn a_move_on_multiple_frames_moves_each_keyframe_and_undoes_in_one_step() {
    let (mut s, a, id) = walk();
    let steps = s.history_len();
    let v = run(&mut s, "frames.edit_multiple", json!({"first": 0, "last": 8, "dx": 5, "dy": 7}));
    assert_eq!((v["moved"].as_u64(), v["keyframes"].as_u64()), (Some(3), Some(3)));
    assert_eq!([0, 4, 8].map(|f| x_at(&s, a, f, id)), [15.0, 55.0, 95.0]);
    assert_eq!(s.history_len(), steps + 1);
    s.undo().unwrap();
    assert_eq!([0, 4, 8].map(|f| x_at(&s, a, f, id)), [10.0, 50.0, 90.0], "one undo takes back every keyframe");
    // Only the items given.
    run(&mut s, "frames.edit_multiple", json!({"first": 0, "last": 8, "dx": -10, "dy": 0, "items": [[a.0, 8, id.0]]}));
    assert_eq!([0, 4, 8].map(|f| x_at(&s, a, f, id)), [10.0, 50.0, 80.0]);
    // A drag: many small moves in one gesture are one step.
    let steps = s.history_len();
    s.begin_gesture("Move");
    for _ in 0..10 {
        run(&mut s, "frames.edit_multiple", json!({"first": 0, "last": 4, "dx": 1, "dy": 0}));
    }
    s.end_gesture();
    assert_eq!(s.history_len(), steps + 1);
    assert_eq!([0, 4, 8].map(|f| x_at(&s, a, f, id)), [20.0, 60.0, 80.0], "the keyframe on 8 is outside frames 0..4");
}

#[test]
fn moves_refuse_what_edit_multiple_frames_does_not_show() {
    let (mut s, a, id) = walk();
    let before = s.doc.clone();
    let e = s.execute("frames.edit_multiple", &json!({"first": 0, "last": 3, "dx": 5, "dy": 0, "items": [[a.0, 8, id.0]]}));
    assert!(e.is_err(), "the keyframe on 8 is outside the markers");
    assert!(s.execute("frames.edit_multiple", &json!({"first": 0, "last": 8, "dx": 5, "dy": 0, "items": [[a.0, 8, 999]]})).is_err());
    assert!(s.execute("frames.edit_multiple", &json!({"first": 0, "last": 8, "dx": 5, "dy": 0, "items": [[a.0, 8]]})).is_err());
    assert!(s.execute("frames.edit_multiple", &json!({"first": 0, "dx": 5, "dy": 0})).is_err());
    assert!(s.doc == before);
    run(&mut s, "layer.toggle_lock", json!({"layer": a.0}));
    assert!(s.multi_frame_items(0, 8).is_empty(), "nothing on a locked layer");
    assert_eq!(run(&mut s, "frames.edit_multiple", json!({"first": 0, "last": 8, "dx": 5, "dy": 0}))["moved"], json!(0));
    assert!(s.execute("frames.edit_multiple", &json!({"first": 0, "last": 8, "dx": 5, "dy": 0, "items": [[a.0, 0, id.0]]})).is_err());
}

#[test]
fn picking_finds_artwork_on_every_keyframe_shown() {
    let (s, a, id) = walk();
    let at = |x: f32| s.multi_frame_pick(0, 8, Vec2::new(x, 20.0), 1.0);
    assert_eq!(at(100.0), Some(FrameItem { layer: a, frame: 8, element: id }));
    assert_eq!(at(60.0), Some(FrameItem { layer: a, frame: 4, element: id }));
    assert_eq!(at(20.0), Some(FrameItem { layer: a, frame: 0, element: id }));
    assert_eq!(at(150.0), None);
    assert_eq!(s.multi_frame_pick(2, 6, Vec2::new(100.0, 20.0), 1.0), None, "outside the markers");
}

#[test]
fn a_motion_tweens_path_and_keys_move_with_its_object() {
    let mut s = Session::default();
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 20, "h": 20, "object": true}));
    run(&mut s, "timeline.insert_frame", json!({"count": 9}));
    run(&mut s, "timeline.create_motion_tween", json!({}));
    run(&mut s, "timeline.goto", json!({"frame": 9}));
    run(&mut s, "edit.select_all", json!({}));
    run(&mut s, "element.translate", json!({"dx": 100, "dy": 0}));
    let x_on = |s: &Session, f: u32| evaluate_scene(&s.doc, 0, f).items.first().map(left).unwrap_or(f32::NAN);
    let before: Vec<f32> = (0..10).map(|f| x_on(&s, f)).collect();
    run(&mut s, "timeline.goto", json!({"frame": 0}));
    let v = run(&mut s, "frames.edit_multiple", json!({"first": 0, "last": 9, "dx": 10, "dy": 0}));
    assert_eq!(v["moved"], json!(1));
    let after: Vec<f32> = (0..10).map(|f| x_on(&s, f)).collect();
    for (b, a) in before.iter().zip(&after) {
        assert!((a - b - 10.0).abs() < 1e-3, "{before:?} → {after:?}");
    }
}

#[test]
fn every_setting_composes_and_renders_on_the_sample() {
    let doc = sample::demo();
    let n = doc.timeline(0).frame_count();
    let view = View { transform: Affine::scale(0.1, 0.1), width: 128, height: 72, background: Some(doc.stage.background) };
    let mut count = 0;
    for bits in 0u32..64 {
        let mut o = OnionSkin { ghosts: bits & 1 != 0, edit_multiple: bits & 2 != 0, tint: bits & 4 != 0, fade: bits & 8 != 0, outlines: bits & 16 != 0, keyframes_only: bits & 32 != 0, ..OnionSkin::default() };
        for (i, f) in [0, n / 2, n - 1, n + 40].into_iter().enumerate() {
            match i {
                1 => o.all(),
                2 => o.markers = Markers::Anchored { first: n + 5, last: 1 },
                3 => o.preset(5),
                _ => {}
            }
            let dl = compose(&doc, 0, f.min(n - 1), Some(&o), |sc, fr| evaluate_scene(&doc, sc, fr));
            count += dl.items.len();
            if bits % 7 == 0 {
                assert!(render(&dl, &view).is_ok());
            }
        }
    }
    assert!(count > 0);
}

/// Measures the stage's cost with onion skin ±5 at 1080p, through the tiled path the stage uses
/// (one scene evaluation per frame, tiles in parallel), and that a second look is a cache hit:
/// `cargo test --release -p anicraft-engine onion_timings -- --ignored --nocapture`.
#[test]
#[ignore]
fn onion_timings() {
    use crate::render_service::{RenderService, make_job};
    for doc in [sample::demo(), sample::stress(20, 100, 48)] {
        println!("-- {} items on a frame", evaluate_scene(&doc, 0, 30).items.len());
        timings_of(&doc);
    }
    // The cache: the same frame and settings again are hits, other settings keyed apart.
    let doc = sample::demo();
    let five = OnionSkin { markers: Markers::Relative { before: 5, after: 5 }, ..OnionSkin::default() };
    let view = View { transform: Affine::scale(1.5, 1.5), width: 256, height: 256, background: Some(doc.stage.background) };
    let jobs = |o: &OnionSkin| (0..40).map(|i| make_job(1, &doc, 30, View { transform: Affine::translate(-(i as f32) * 10.0, 0.0).mul(&view.transform), ..view }, Some(o))).collect::<Vec<_>>();
    let svc = RenderService::new(1 << 30, None);
    let first = svc.render_now_batch(jobs(&five));
    let t = std::time::Instant::now();
    let again = svc.request_batch(jobs(&five));
    let hit = t.elapsed().as_secs_f32() * 1000.0;
    assert!(again.iter().all(Option::is_some) && first.len() == again.len());
    let outlined = svc.request_batch(jobs(&OnionSkin { outlines: true, ..five }));
    assert!(outlined.iter().all(Option::is_none), "other settings are other pictures");
    println!("cache: {} tiles, second look {hit:.3} ms, all hits", again.len());
}

fn timings_of(doc: &anicraft_doc::Document) {
    use crate::render_service::{make_job, render_batch};
    let (w, h) = (1920u32, 1080u32);
    let z = w as f32 / doc.stage.width as f32;
    let tile = 256u32;
    let jobs = |onion: Option<&OnionSkin>, frame: u32| -> Vec<crate::render_service::FrameJob> {
        let mut v = Vec::new();
        for ty in 0..h.div_ceil(tile) {
            for tx in 0..w.div_ceil(tile) {
                let view = View { transform: Affine::translate(-((tx * tile) as f32), -((ty * tile) as f32)).mul(&Affine::scale(z, z)), width: tile.min(w - tx * tile), height: tile.min(h - ty * tile), background: Some(doc.stage.background) };
                v.push(make_job(1, doc, frame, view, onion));
            }
        }
        v
    };
    let five = OnionSkin { markers: Markers::Relative { before: 5, after: 5 }, ..OnionSkin::default() };
    let cases: [(&str, Option<OnionSkin>); 5] = [
        ("off", None),
        ("±5, tinted, fading", Some(five)),
        ("±5, outlines", Some(OnionSkin { outlines: true, ..five })),
        ("±5, keyframes only", Some(OnionSkin { keyframes_only: true, ..five })),
        ("Edit Multiple Frames ±5", Some(OnionSkin { edit_multiple: true, ..five })),
    ];
    for (name, o) in cases {
        let mut best = f32::MAX;
        for frame in [12u32, 30, 48] {
            let t = std::time::Instant::now();
            let _ = render_batch(&jobs(o.as_ref(), frame));
            best = best.min(t.elapsed().as_secs_f32() * 1000.0);
        }
        let t = std::time::Instant::now();
        let items = crate::render_service::display_list_with_onion(doc, 30, o.as_ref()).items.len();
        let eval = t.elapsed().as_secs_f32() * 1000.0;
        println!("{name}: 1080p {best:.1} ms (best of frames 12, 30, 48), scene {eval:.2} ms, {items} items on frame 30");
    }
}

#[test]
fn extra_frames_sit_on_their_own_layer_over_an_opaque_background() {
    // The sample's sky (locked) and hills cover the stage: frames before or after the playhead
    // must show over them, under the playhead's artwork on their own layer only.
    let doc = sample::demo();
    let tl = doc.timeline(0);
    let rank = |l: LayerId| tl.layers.iter().position(|x| x.id == l).unwrap();
    for o in [OnionSkin { markers: Markers::Relative { before: 5, after: 5 }, ..OnionSkin::default() }, OnionSkin { edit_multiple: true, markers: Markers::Relative { before: 5, after: 5 }, ..OnionSkin::default() }] {
        let dl = compose(&doc, 0, 12, Some(&o), |sc, f| evaluate_scene(&doc, sc, f));
        let plain = evaluate_scene(&doc, 0, 12);
        assert!(dl.items.len() > plain.items.len());
        // Back to front: layer by layer, bottom first.
        assert!(dl.items.windows(2).all(|w| rank(w[0].layer) >= rank(w[1].layer)), "items are grouped by layer, bottom up");
        // On each layer, its own extra frames come before (under) the playhead's artwork.
        let ball = plain.items.last().unwrap().layer;
        let on_ball: Vec<&DisplayItem> = dl.items.iter().filter(|i| i.layer == ball).collect();
        assert!(on_ball.len() > 1);
        assert_eq!(on_ball.last().map(|i| i.color.is_identity()), Some(true), "the playhead's ball is on top");
    }
    // Rendered: the ball on frame 1 (top left of its path) shows over the sky, in Edit Multiple
    // Frames at full strength.
    let view = View { transform: Affine::IDENTITY, width: 1280, height: 720, background: Some(Rgba::WHITE) };
    let multi = OnionSkin { edit_multiple: true, markers: Markers::Relative { before: 12, after: 0 }, ..OnionSkin::default() };
    let with = render(&compose(&doc, 0, 12, Some(&multi), |sc, f| evaluate_scene(&doc, sc, f)), &view).unwrap().0.to_premul_rgba8();
    let without = render(&evaluate_scene(&doc, 0, 12), &view).unwrap().0.to_premul_rgba8();
    assert_ne!(px(&with, 1280, 200, 260), px(&without, 1280, 200, 260), "the ball on frame 1 is visible");
}
