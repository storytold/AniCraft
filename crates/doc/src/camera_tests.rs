//! Evaluation through the camera layer (ADR 0027).

use crate::camera::{Camera, DepthKey, FOCAL, MAX_DEPTH};
use crate::{ColorEffect, Document, Element, ElementId, ElementKind, Keyframe, Layer, LayerId, LayerKind, MotionTween, Paint, Property, Rgba, Shape, Symbol, SymbolId, SymbolKind, Timeline, Tween, evaluate, evaluate_scene};
use anicraft_geom::{Affine, Path, Vec2};
use std::sync::Arc;

fn square(id: u64, x: f32) -> Arc<Element> {
    Arc::new(Element { id: ElementId(id), name: String::new(), transform: Affine::translate(x, 0.0), color: ColorEffect::default(), kind: ElementKind::Shape(Shape::filled(Path::rect(0.0, 0.0, 10.0, 10.0), Paint::Solid(Rgba::BLACK))) })
}

fn art_layer(id: u64, el: Arc<Element>) -> Layer {
    let mut l = Layer::new(LayerId(id), format!("L{id}"));
    let mut k = Keyframe::new(0, 20);
    k.elements.push(el);
    l.keyframes = vec![Arc::new(k)];
    l
}

/// A 200 × 100 stage with a 10 × 10 square at (95, 0).
fn one_square() -> Document {
    let mut d = Document::default();
    d.stage.width = 200;
    d.stage.height = 100;
    d.timeline_mut(0).layers = vec![Arc::new(art_layer(1, square(10, 95.0)))];
    d
}

/// A camera layer on top of `d` holding `keys` (start, duration, camera, classic tween?).
fn with_camera(d: &mut Document, keys: &[(u32, u32, Camera, bool)]) {
    let mut cl = Layer::new(LayerId(900), "Camera");
    cl.kind = LayerKind::Camera;
    cl.keyframes = keys
        .iter()
        .map(|&(start, dur, cam, tween)| {
            let mut k = Keyframe::new(start, dur);
            k.elements.push(Arc::new(cam.to_element(ElementId(901))));
            k.tween = tween.then(Tween::default);
            Arc::new(k)
        })
        .collect();
    d.timeline_mut(0).layers.insert(0, Arc::new(cl));
}

#[test]
fn a_camera_zoom_doubles_sizes_about_its_centre_and_editing_keeps_stage_space() {
    let mut d = one_square();
    let cam = Camera { center: Vec2::new(100.0, 5.0), zoom: 2.0, ..Camera::rest(&d.stage) };
    with_camera(&mut d, &[(0, 20, cam, false)]);
    let m = evaluate(&d, 0).items[0].transform;
    // The square's corner (95, 0) is 5 px left of and above the camera centre: 10 and 10 from
    // the output's middle (100, 50).
    assert!(m.apply(Vec2::new(0.0, 0.0)).distance(Vec2::new(90.0, 40.0)) < 1e-3, "{m:?}");
    assert!((m.mean_scale() - 2.0).abs() < 1e-5);
    // While editing, the stage shows stage space; the camera draws nothing.
    let edit = evaluate_scene(&d, 0, 0);
    assert_eq!(edit.items.len(), 1);
    assert_eq!(edit.items[0].transform, Affine::translate(95.0, 0.0));
    // A hidden camera layer is a camera turned off.
    Arc::make_mut(&mut d.timeline_mut(0).layers[0]).visible = false;
    assert_eq!(evaluate(&d, 0).items[0].transform, Affine::translate(95.0, 0.0));
}

#[test]
fn camera_rotation_and_tint_apply_to_the_whole_view() {
    let mut d = one_square();
    let cam = Camera { rotation: std::f32::consts::FRAC_PI_2, color: ColorEffect::Tint { color: Rgba::hex(0xff0000), amount: 0.5 }, ..Camera::rest(&d.stage) };
    with_camera(&mut d, &[(0, 20, cam, false)]);
    let i = &evaluate(&d, 0).items[0];
    // Stage point (105, 50) is right of the centre; the camera turned clockwise, so it shows
    // above the middle. In the square's space that point is (10, 50).
    assert!(i.transform.apply(Vec2::new(10.0, 50.0)).distance(Vec2::new(100.0, 45.0)) < 1e-3);
    assert_eq!(i.color, crate::ColorTransform::tint(Rgba::hex(0xff0000), 0.5));
}

#[test]
fn camera_keyframes_tween_like_any_instance() {
    let mut d = one_square();
    let a = Camera::rest(&d.stage);
    let b = Camera { center: Vec2::new(150.0, 50.0), zoom: 3.0, ..a };
    with_camera(&mut d, &[(0, 10, a, true), (10, 10, b, false)]);
    let cam = |d: &Document, f| d.timeline(0).camera_state_at(f, &d.stage).unwrap();
    assert_eq!(cam(&d, 0).zoom, 1.0);
    let half = cam(&d, 5);
    assert!((half.zoom - 2.0).abs() < 1e-5 && (half.center.x - 125.0).abs() < 1e-3, "{half:?}");
    assert_eq!(cam(&d, 15).zoom, 3.0);
    // The square follows: at frame 5 it is drawn 2× about (125, 50).
    let m = evaluate(&d, 5).items[0].transform;
    assert!(m.apply(Vec2::ZERO).distance(Vec2::new(100.0 + (95.0 - 125.0) * 2.0, 50.0 - 50.0 * 2.0)) < 1e-2, "{m:?}");
    // A motion tween on the camera works too: Scale X (the zoom) keyed from 100 % to 300 %.
    let mut mt = MotionTween::new(ElementId(901));
    mt.set_key(Property::ScaleX, 10, 300.0, 100.0);
    let cl = Arc::make_mut(&mut d.timeline_mut(0).layers[0]);
    cl.keyframes.truncate(1);
    let k = cl.keyframe_mut(0).unwrap();
    k.tween = None;
    k.motion = Some(mt);
    k.duration = 11;
    let z = cam(&d, 5).zoom;
    assert!(z > 1.0 && z < 3.0, "{z}");
    assert!((cam(&d, 10).zoom - 3.0).abs() < 1e-5);
}

#[test]
fn layer_depth_gives_parallax_and_guides_never_show() {
    let mut d = one_square();
    let mut back = art_layer(50, square(11, 95.0));
    back.depth = FOCAL;
    let mut guide = art_layer(60, square(12, 0.0));
    guide.kind = LayerKind::Guide;
    d.timeline_mut(0).layers.extend([Arc::new(back), Arc::new(guide)]);
    let cam = Camera { center: Vec2::new(140.0, 50.0), ..Camera::rest(&d.stage) };
    with_camera(&mut d, &[(0, 20, cam, false)]);
    let dl = evaluate(&d, 0);
    let x = |dl: &crate::DisplayList, id| dl.items.iter().find(|i| i.element == ElementId(id)).unwrap().transform.translation().x;
    assert_eq!(x(&dl, 10), 55.0, "the stage layer moved the full 40 px");
    assert_eq!(x(&dl, 11), 75.0, "the far layer moved half as far");
    assert_eq!(dl.items.iter().filter(|i| !i.guide).count(), 2, "the camera draws nothing, guides stay guides");
    // Without a camera, depth changes nothing.
    Arc::make_mut(&mut d.timeline_mut(0).layers[0]).keyframes = vec![Arc::new(Keyframe::new(0, 20))];
    assert_eq!(x(&evaluate(&d, 0), 11), 95.0);
}

#[test]
fn the_camera_stays_on_top_alone_and_old_files_read_without_one() {
    let mut d = one_square();
    let mut a = Layer::new(LayerId(70), "Cam A");
    a.kind = LayerKind::Camera;
    let mut b = a.clone();
    b.id = LayerId(71);
    a.parent = Some(LayerId(1));
    // Two camera layers, below the artwork.
    d.timeline_mut(0).layers.extend([Arc::new(a), Arc::new(b)]);
    d.sanitize();
    let tl = d.timeline(0);
    assert_eq!(tl.layers[0].id, LayerId(70));
    assert_eq!(tl.layers[0].parent, None);
    assert!(tl.camera_layer().is_some());
    assert_eq!(tl.layers.iter().filter(|l| l.kind == LayerKind::Camera).count(), 1, "the extra camera became a normal layer");
    // A symbol's camera layer is a normal layer.
    let mut sl = Layer::new(LayerId(80), "in symbol");
    sl.kind = LayerKind::Camera;
    d.library.push(Arc::new(Symbol { id: SymbolId(81), name: "S".into(), kind: SymbolKind::Graphic, linkage: None, scale_grid_kept: None, scale_grid: None, timeline: Timeline { layers: vec![Arc::new(sl)] } }));
    d.sanitize();
    assert_eq!(d.library[0].timeline.layers[0].kind, LayerKind::Normal);
    // Depth round-trips, is left out when 0, and an old file reads at 0; hostile depths clamp.
    let mut l = Layer::new(LayerId(5), "x");
    assert!(!serde_json::to_string(&l).unwrap().contains("depth"));
    l.depth = 250.0;
    let back: Layer = serde_json::from_str(&serde_json::to_string(&l).unwrap()).unwrap();
    assert_eq!(back, l);
    let old: Layer = serde_json::from_str(r#"{"id":5,"name":"x"}"#).unwrap();
    assert_eq!(old.depth, 0.0);
    let mut d2 = Document::default();
    Arc::make_mut(&mut d2.timeline_mut(0).layers[0]).depth = 1e12;
    d2.sanitize();
    assert_eq!(d2.timeline(0).layers[0].depth, MAX_DEPTH);
    // The camera element survives a file.
    let el = Camera::rest(&d.stage).to_element(ElementId(3));
    let back: Element = serde_json::from_str(&serde_json::to_string(&el).unwrap()).unwrap();
    assert_eq!(back, el);
}

#[test]
fn a_keyframe_without_a_camera_is_the_camera_at_rest() {
    let mut d = one_square();
    let cam = Camera { zoom: 4.0, ..Camera::rest(&d.stage) };
    with_camera(&mut d, &[(0, 20, cam, false)]);
    let cl = Arc::make_mut(&mut d.timeline_mut(0).layers[0]);
    cl.keyframe_mut(0).unwrap().elements.clear();
    assert!(d.timeline(0).camera_at(0, &d.stage).is_none());
    assert_eq!(evaluate(&d, 0).items[0].transform, Affine::translate(95.0, 0.0));
}

/// Each item's x translation by element id.
fn xs(dl: &crate::DisplayList, id: u64) -> f32 {
    dl.items.iter().find(|i| i.element == ElementId(id)).map_or(f32::NAN, |i| i.transform.translation().x)
}

#[test]
fn layers_at_depth_zero_are_bit_exact_through_any_camera() {
    // The same scene with and without a deep layer elsewhere: the stage-plane layer's item is
    // identical, bit for bit, to what the camera's stage view gives it.
    let mut d = one_square();
    let cam = Camera { center: Vec2::new(123.4, 56.7), zoom: 1.7, rotation: 0.3, ..Camera::rest(&d.stage) };
    with_camera(&mut d, &[(0, 20, cam, false)]);
    let before = evaluate(&d, 0).items[0].transform;
    let want = cam.stage_view(&d.stage).mul(&Affine::translate(95.0, 0.0));
    assert!(before.m.iter().zip(want.m).all(|(a, b)| (a - b).abs() < 1e-3), "the stage view: {before:?}");
    let mut back = art_layer(50, square(11, 0.0));
    back.depth = 400.0;
    d.timeline_mut(0).layers.push(Arc::new(back));
    let after = evaluate(&d, 0);
    let it = after.items.iter().find(|i| i.element == ElementId(10)).unwrap();
    assert_eq!(it.transform.m, before.m);
}

#[test]
fn a_pan_moves_layers_by_their_parallax_and_attached_layers_stay() {
    let mut d = one_square();
    let mut mid = art_layer(40, square(11, 95.0));
    mid.depth = 250.0;
    let mut far = art_layer(50, square(12, 95.0));
    far.depth = 3000.0;
    let mut hud = art_layer(60, square(13, 95.0));
    hud.attached = true;
    hud.depth = 900.0;
    d.timeline_mut(0).layers.extend([Arc::new(mid), Arc::new(far)]);
    d.timeline_mut(0).layers.insert(0, Arc::new(hud));
    let cam = Camera { center: Vec2::new(150.0, 50.0), ..Camera::rest(&d.stage) };
    with_camera(&mut d, &[(0, 20, cam, false)]);
    let dl = evaluate(&d, 0);
    // A pan of 50 px moves a layer at depth d by 50 · F / (F + d).
    for (id, depth) in [(10, 0.0), (11, 250.0), (12, 3000.0)] {
        let moved = 95.0 - xs(&dl, id);
        let want = 50.0 * FOCAL / (FOCAL + depth);
        assert!((moved - want).abs() < 1e-3, "{id}: moved {moved}, want {want}");
    }
    assert_eq!(xs(&dl, 13), 95.0, "the attached layer stays where it is on screen");
    // While editing, every layer is in stage space.
    let edit = evaluate_scene(&d, 0, 0);
    assert!([10, 11, 12, 13].iter().all(|&id| xs(&edit, id) == 95.0));
}

#[test]
fn deeper_layers_draw_first_ties_in_timeline_order() {
    let mut d = one_square();
    // Top to bottom: 10 (depth 0), 11 (depth 500), 12 (depth 0), 13 (depth 500).
    let deep = |id, el| {
        let mut l = art_layer(id, el);
        l.depth = 500.0;
        Arc::new(l)
    };
    d.timeline_mut(0).layers.extend([deep(40, square(11, 0.0)), Arc::new(art_layer(50, square(12, 0.0))), deep(60, square(13, 0.0))]);
    let ids = |dl: &crate::DisplayList| dl.items.iter().map(|i| i.element.0).collect::<Vec<_>>();
    assert_eq!(ids(&evaluate_scene(&d, 0, 0)), [13, 11, 12, 10], "the deep pair first, each pair bottom-up");
    assert_eq!(ids(&evaluate(&d, 0)), [13, 11, 12, 10], "the movie stacks as the stage does");
    // All at depth 0: the timeline order, as always.
    for l in &mut d.timeline_mut(0).layers {
        Arc::make_mut(l).depth = 0.0;
    }
    assert_eq!(ids(&evaluate_scene(&d, 0, 0)), [13, 12, 11, 10]);
}

#[test]
fn a_depth_tween_is_halfway_at_its_midpoint() {
    let mut d = one_square();
    let l = Arc::make_mut(&mut d.timeline_mut(0).layers[0]);
    l.depth_keys = vec![DepthKey { frame: 0, depth: 0.0 }, DepthKey { frame: 10, depth: 2000.0 }];
    let cam = Camera { center: Vec2::new(140.0, 50.0), ..Camera::rest(&d.stage) };
    with_camera(&mut d, &[(0, 20, cam, false)]);
    assert_eq!(d.timeline(0).layers[1].depth_at(5), 1000.0);
    // At frame 5 the layer is 1,000 px back: the 40 px pan moves it 20.
    assert!((xs(&evaluate(&d, 5), 10) - 75.0).abs() < 1e-3);
    assert!((xs(&evaluate(&d, 0), 10) - 55.0).abs() < 1e-3);
    // Keys and attachment round-trip; older files read without them, and they are left out
    // when unused.
    let mut l = Layer::new(LayerId(5), "x");
    let plain = serde_json::to_string(&l).unwrap();
    assert!(!plain.contains("depth_keys") && !plain.contains("attached"));
    l.depth_keys = vec![DepthKey { frame: 3, depth: 120.5 }];
    l.attached = true;
    let back: Layer = serde_json::from_str(&serde_json::to_string(&l).unwrap()).unwrap();
    assert_eq!(back, l);
    let old: Layer = serde_json::from_str(r#"{"id":5,"name":"x","depth":40}"#).unwrap();
    assert!(old.depth_keys.is_empty() && !old.attached && old.depth_at(9) == 40.0);
    // Hostile keys on load are sorted and clamped.
    let mut d2 = Document::default();
    Arc::make_mut(&mut d2.timeline_mut(0).layers[0]).depth_keys = vec![DepthKey { frame: 9, depth: f32::NAN }, DepthKey { frame: 2, depth: 1e20 }];
    d2.sanitize();
    assert_eq!(d2.timeline(0).layers[0].depth_keys, vec![DepthKey { frame: 2, depth: MAX_DEPTH }, DepthKey { frame: 9, depth: 0.0 }]);
}

#[test]
fn a_camera_brightness_colours_the_whole_view() {
    let mut d = one_square();
    let cam = Camera { color: ColorEffect::Brightness(-0.5), ..Camera::rest(&d.stage) };
    with_camera(&mut d, &[(0, 20, cam, false)]);
    assert!(d.timeline(0).camera_at(0, &d.stage).is_some(), "a colour alone is a camera that does something");
    let i = &evaluate(&d, 0).items[0];
    assert_eq!(i.transform, Affine::translate(95.0, 0.0), "nothing moved");
    assert_eq!(i.color, crate::ColorTransform::brightness(-0.5));
}
