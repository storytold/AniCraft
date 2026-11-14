use super::*;
use crate::{ColorEffect, Easing, Keyframe, Symbol, SymbolId, SymbolKind, Tween, TweenKind};

fn shape_el(id: u64, x: f32) -> Element {
    Element {
        id: ElementId(id),
        name: String::new(),
        transform: Affine::translate(x, 0.0),
        color: ColorEffect::default(),
        kind: ElementKind::Shape(Shape::filled(Path::rect(0.0, 0.0, 10.0, 10.0), Paint::Solid(Rgba::BLACK))),
    }
}

fn layer0(d: &mut Document) -> &mut Layer {
    Arc::make_mut(&mut d.timeline_mut(0).layers[0])
}

fn push(layer: &mut Layer, kf: usize, e: Element) {
    layer.keyframe_mut(kf).unwrap().elements.push(Arc::new(e));
}

fn tween_doc(easing: Easing) -> Document {
    let mut d = Document::default();
    let mut k0 = Keyframe::new(0, 10);
    k0.elements.push(Arc::new(shape_el(10, 0.0)));
    k0.tween = Some(Tween { easing, ..Default::default() });
    let mut k1 = Keyframe::new(10, 1);
    k1.elements.push(Arc::new(shape_el(10, 100.0)));
    layer0(&mut d).keyframes = vec![Arc::new(k0), Arc::new(k1)];
    d
}

#[test]
fn linear_tween_interpolates_translation() {
    let d = tween_doc(Easing::Linear);
    let x = |f| evaluate(&d, f).items[0].transform.translation().x;
    assert_eq!(x(0), 0.0);
    assert!((x(5) - 50.0).abs() < 1e-3);
    assert_eq!(x(10), 100.0);
}

#[test]
fn eased_tween_differs_from_linear() {
    let d = tween_doc(Easing::QuadIn);
    let x5 = evaluate(&d, 5).items[0].transform.translation().x;
    assert!((x5 - 25.0).abs() < 1e-3);
}

#[test]
fn static_frames_share_elements_with_the_document() {
    let d = tween_doc(Easing::Linear);
    let (_, els) = evaluate_layer_elements(&d.timeline(0).layers[0], 10).unwrap();
    assert!(matches!(els, Cow::Borrowed(_)), "a non-tweened frame must not copy elements");
}

#[test]
fn hidden_layers_are_skipped_and_guide_artwork_is_marked() {
    let mut d = tween_doc(Easing::Linear);
    layer0(&mut d).visible = false;
    assert!(evaluate(&d, 0).items.is_empty());
    layer0(&mut d).visible = true;
    layer0(&mut d).kind = LayerKind::Guide;
    // Guide artwork is listed for editing, but marked so nothing draws or exports it.
    let dl = evaluate(&d, 0);
    assert!(!dl.items.is_empty() && dl.items.iter().all(|i| i.guide));
}

#[test]
fn nested_symbols_compose_transforms_and_cycles_terminate() {
    let mut d = Document::default();
    let sid = SymbolId(99);
    // A symbol that contains a shape and an instance of itself: a cycle.
    let mut tl = Timeline::default();
    let mut layer = Layer::new(LayerId(50), "s");
    push(&mut layer, 0, shape_el(51, 0.0));
    push(
        &mut layer,
        0,
        Element {
            id: ElementId(52),
            name: String::new(),
            transform: Affine::translate(1.0, 0.0),
            color: ColorEffect::default(),
            kind: ElementKind::Symbol { symbol: sid, loop_mode: Default::default(), first_frame: 0, blend: Default::default(), filters: Vec::new(), events: Vec::new() },
        },
    );
    tl.layers.push(Arc::new(layer));
    d.library.push(Arc::new(Symbol { id: sid, name: "loop".into(), kind: SymbolKind::Graphic, linkage: None, scale_grid_kept: None, scale_grid: None, timeline: tl }));
    push(
        layer0(&mut d),
        0,
        Element {
            id: ElementId(60),
            name: String::new(),
            transform: Affine::translate(100.0, 0.0),
            color: ColorEffect::Alpha(0.5),
            kind: ElementKind::Symbol { symbol: sid, loop_mode: Default::default(), first_frame: 0, blend: Default::default(), filters: Vec::new(), events: Vec::new() },
        },
    );
    let dl = evaluate(&d, 0);
    assert_eq!(dl.items.len(), MAX_SYMBOL_DEPTH);
    assert_eq!(dl.items[0].transform.translation().x, 100.0);
    // The instance's opacity is its group's: it fades as one.
    assert_eq!((dl.items[0].color.mul[3], dl.effective_alpha(&dl.items[0])), (1.0, 0.5));
    assert_eq!(dl.groups.len(), 1);
    assert!(dl.items.iter().all(|i| i.element == ElementId(60)));
}

/// Layers top to bottom: mask (id 1), masked (2), masked (3), plain (4).
fn mask_doc() -> Document {
    let mut d = Document { scenes: vec![Arc::new(crate::Scene::new("Scene 1"))], ..Document::default() };
    for (id, kind, parent) in [(1, LayerKind::Mask, None), (2, LayerKind::Normal, Some(1)), (3, LayerKind::Normal, Some(1)), (4, LayerKind::Normal, None)] {
        let mut l = Layer::new(LayerId(id), format!("L{id}"));
        l.kind = kind;
        l.parent = parent.map(LayerId);
        push(&mut l, 0, shape_el(10 + id, 0.0));
        d.timeline_mut(0).layers.push(Arc::new(l));
    }
    d
}

#[test]
fn mask_layers_clip_the_layers_they_mask() {
    let d = mask_doc();
    let dl = evaluate(&d, 0);
    assert_eq!(dl.clip_parents, vec![None]);
    let by = |e: u64| dl.items.iter().find(|i| i.element == ElementId(e)).unwrap();
    assert_eq!((by(14).clip, by(14).masks), (None, None), "unmasked layer");
    assert_eq!((by(13).clip, by(12).clip), (Some(0), Some(0)), "masked layers");
    assert_eq!(by(11).masks, Some(0), "the mask layer's artwork forms the clip");
    // A hidden mask masks nothing; its layers draw normally.
    let mut h = d.clone();
    Arc::make_mut(&mut h.timeline_mut(0).layers[0]).visible = false;
    let dl = evaluate(&h, 0);
    assert!(dl.items.iter().all(|i| i.clip.is_none() && i.masks.is_none()) && dl.items.len() == 3);
}

#[test]
fn masks_inside_symbols_nest_in_the_instance_clip() {
    // The main timeline's masked layer holds an instance of a symbol that has its own mask.
    let mut d = mask_doc();
    let sid = SymbolId(99);
    d.library.push(Arc::new(Symbol { id: sid, name: "masked inside".into(), kind: SymbolKind::Graphic, linkage: None, scale_grid_kept: None, scale_grid: None, timeline: mask_doc().timeline(0).clone() }));
    let inst = Element {
        id: ElementId(60),
        name: String::new(),
        transform: Affine::IDENTITY,
        color: ColorEffect::default(),
        kind: ElementKind::Symbol { symbol: sid, loop_mode: Default::default(), first_frame: 0, blend: Default::default(), filters: Vec::new(), events: Vec::new() },
    };
    push(Arc::make_mut(&mut d.timeline_mut(0).layers[1]), 0, inst);
    let dl = evaluate(&d, 0);
    assert_eq!(dl.clip_parents, vec![None, Some(0)]);
    let inner: Vec<_> = dl.items.iter().filter(|i| i.element == ElementId(60)).map(|i| (i.clip, i.masks)).collect();
    // inside: unmasked layer keeps the outer clip; masked layers use the nested clip.
    assert_eq!(inner, vec![(Some(0), None), (Some(1), None), (Some(1), None), (Some(0), Some(1))]);
}

#[test]
fn a_soft_mask_layer_gives_its_clip_its_mode() {
    let mut d = mask_doc();
    assert_eq!((evaluate(&d, 0).clip_mode(0), evaluate(&d, 0).clip_modes.len()), (crate::MaskMode::Coverage, 0), "coverage stays implicit");
    Arc::make_mut(&mut d.timeline_mut(0).layers[0]).mask_mode = crate::MaskMode::Luminance;
    let dl = evaluate(&d, 0);
    assert_eq!(dl.clip_mode(0), crate::MaskMode::Luminance);
    // Appended lists keep their modes on their renumbered clips.
    let mut a = evaluate(&mask_doc(), 0);
    a.append(dl);
    assert_eq!((a.clip_mode(0), a.clip_mode(1)), (crate::MaskMode::Coverage, crate::MaskMode::Luminance));
    // The mode is saved only when it isn't coverage, and older files (without it) open as coverage.
    let json = serde_json::to_string(&*d.timeline(0).layers[0]).unwrap();
    assert!(json.contains("\"mask_mode\":\"luminance\""), "{json}");
    let plain = serde_json::to_string(&*mask_doc().timeline(0).layers[0]).unwrap();
    assert!(!plain.contains("mask_mode"));
    let back: Layer = serde_json::from_str(&json).unwrap();
    assert_eq!(back.mask_mode, crate::MaskMode::Luminance);
    assert_eq!(serde_json::from_str::<Layer>(&plain).unwrap().mask_mode, crate::MaskMode::Coverage);
}

#[test]
fn appending_lists_renumbers_clips() {
    let d = mask_doc();
    let mut a = evaluate(&d, 0);
    a.append(evaluate(&d, 0));
    assert_eq!(a.clip_parents, vec![None, None]);
    let second = &a.items[4..];
    assert!(second.iter().all(|i| i.clip.is_none_or(|c| c == 1) && i.masks.is_none_or(|c| c == 1)));
    assert!(second.iter().any(|i| i.masks == Some(1)));
}

#[test]
fn mask_links_must_stay_directly_under_their_mask() {
    let mut d = mask_doc();
    d.timeline_mut(0).repair_masks();
    assert_eq!(d.timeline(0).layers.iter().map(|l| l.parent).collect::<Vec<_>>(), vec![None, Some(LayerId(1)), Some(LayerId(1)), None]);
    assert_eq!(d.timeline(0).group_range(0), 0..3);
    assert_eq!(d.timeline(0).mask_above(3), Some(LayerId(1)), "a layer under the group would join it");
    // The plain layer moved between the masked ones breaks the run: the lower one lets go.
    d.timeline_mut(0).layers.swap(2, 3);
    d.timeline_mut(0).repair_masks();
    assert_eq!(d.timeline(0).layers[3].parent, None);
    assert_eq!(d.timeline(0).layers[1].parent, Some(LayerId(1)));
    // A masked layer above its mask, or a mask that is no longer a mask, lets go too.
    let mut e = mask_doc();
    e.timeline_mut(0).layers.swap(0, 1);
    e.timeline_mut(0).repair_masks();
    assert_eq!(e.timeline(0).layers[0].parent, None);
    assert_eq!(e.timeline(0).layers[2].parent, Some(LayerId(1)), "still directly under the mask");
    let mut f = mask_doc();
    Arc::make_mut(&mut f.timeline_mut(0).layers[0]).kind = LayerKind::Normal;
    f.sanitize();
    assert!(f.timeline(0).layers.iter().all(|l| l.parent.is_none()));
}

#[test]
fn layer_order_is_back_to_front() {
    let mut d = Document::default();
    push(layer0(&mut d), 0, shape_el(1, 0.0));
    let mut bottom = Layer::new(LayerId(7), "bottom");
    push(&mut bottom, 0, shape_el(2, 0.0));
    d.timeline_mut(0).layers.push(Arc::new(bottom));
    let dl = evaluate(&d, 0);
    assert_eq!(dl.items[0].element, ElementId(2));
    assert_eq!(dl.items[1].element, ElementId(1));
}

#[test]
fn shape_tweens_morph_between_different_shapes_and_classic_ones_hold() {
    // A square at frame 0 becomes a circle at frame 10.
    let mut d = Document::default();
    let shape = |id, path: Path| Element {
        id: ElementId(id),
        name: String::new(),
        transform: Affine::IDENTITY,
        color: ColorEffect::default(),
        kind: ElementKind::Shape(Shape::filled(path, Paint::Solid(Rgba::BLACK))),
    };
    let mut k0 = Keyframe::new(0, 10);
    k0.elements.push(Arc::new(shape(1, Path::rect(0.0, 0.0, 100.0, 100.0))));
    let mut k1 = Keyframe::new(10, 1);
    k1.elements.push(Arc::new(shape(1, Path::ellipse(250.0, 50.0, 50.0, 50.0))));
    layer0(&mut d).keyframes = vec![Arc::new(k0), Arc::new(k1)];
    let path_at = |d: &Document, f| evaluate(d, f).items[0].path.clone();

    // Classic tween: incompatible shapes hold the first one until the next keyframe.
    layer0(&mut d).keyframe_mut(0).unwrap().tween = Some(Tween { easing: Easing::Linear, kind: TweenKind::Classic, ..Default::default() });
    assert_eq!(*path_at(&d, 5), Path::rect(0.0, 0.0, 100.0, 100.0));
    // Shape tween: halfway, the shape sits between the two and is neither of them.
    layer0(&mut d).keyframe_mut(0).unwrap().tween = Some(Tween { easing: Easing::Linear, kind: TweenKind::Shape, ..Default::default() });
    let mid = path_at(&d, 5);
    let c = mid.bounds().center();
    assert!((c.x - 150.0).abs() < 1.0 && (c.y - 50.0).abs() < 1.0, "{c:?}");
    assert_ne!(*mid, Path::rect(0.0, 0.0, 100.0, 100.0));
    // Keyframes themselves show exactly what was drawn.
    assert_eq!(*path_at(&d, 0), Path::rect(0.0, 0.0, 100.0, 100.0));
    assert_eq!(*path_at(&d, 10), Path::ellipse(250.0, 50.0, 50.0, 50.0));
    // The kind survives a save (and older files without it read as classic).
    let t: Tween = serde_json::from_str(&serde_json::to_string(&Tween { easing: Easing::Linear, kind: TweenKind::Shape, ..Default::default() }).unwrap()).unwrap();
    assert_eq!(t.kind, TweenKind::Shape);
    assert_eq!(serde_json::from_str::<Tween>(r#"{"easing":"Linear"}"#).unwrap().kind, TweenKind::Classic);
}

#[test]
fn motion_guides_steer_tweens_and_their_artwork_is_marked() {
    // Guide (id 1): an L-shaped line, right 100 then down 100. Guided layer (id 2): a box
    // tweened from (0,0) to (100,100) — a straight diagonal without the guide.
    let mut d = Document { scenes: vec![Arc::new(crate::Scene::new("Scene 1"))], ..Document::default() };
    let mut g = Layer::new(LayerId(1), "Guide");
    g.kind = LayerKind::Guide;
    let mut l = Path::new();
    l.move_to(anicraft_geom::Vec2::new(0.0, 0.0)).line_to(anicraft_geom::Vec2::new(100.0, 0.0)).line_to(anicraft_geom::Vec2::new(100.0, 100.0));
    let mut line = Shape::filled(l, Paint::Solid(Rgba::BLACK)).with_stroke(Rgba::BLACK, 1.0);
    line.fill = None;
    g.keyframe_mut(0).unwrap().duration = 11;
    push(&mut g, 0, Element { id: ElementId(5), name: String::new(), transform: Affine::IDENTITY, color: ColorEffect::default(), kind: ElementKind::Shape(line) });
    let mut moving = Layer::new(LayerId(2), "Box");
    moving.parent = Some(LayerId(1));
    let mut k0 = Keyframe::new(0, 10);
    k0.elements.push(Arc::new(shape_el(10, 0.0)));
    k0.tween = Some(Tween { easing: Easing::Linear, ..Default::default() });
    let mut k1 = Keyframe::new(10, 1);
    k1.elements.push(Arc::new(Element { transform: Affine::translate(100.0, 100.0), ..shape_el(10, 0.0) }));
    moving.keyframes = vec![Arc::new(k0), Arc::new(k1)];
    d.timeline_mut(0).layers = vec![Arc::new(g), Arc::new(moving)];
    d.timeline_mut(0).repair_masks();
    assert_eq!(d.timeline(0).layers[1].parent, Some(LayerId(1)), "a guided layer under its guide holds");

    let at = |d: &Document, f| evaluate(d, f).items.iter().find(|i| i.element == ElementId(10)).unwrap().transform;
    // Halfway along the 200-long path is the corner (100, 0), not the diagonal's (50, 50).
    let mid = at(&d, 5).translation();
    assert!((mid.x - 100.0).abs() < 1e-3 && mid.y.abs() < 1e-3, "{mid:?}");
    let q = at(&d, 2).translation();
    assert!((q.x - 40.0).abs() < 1e-3 && q.y.abs() < 1e-3, "{q:?}");
    // Keyframes stay where they were drawn.
    assert_eq!(at(&d, 0).translation(), anicraft_geom::Vec2::new(0.0, 0.0));
    // Orient to path: on the downward leg the box has turned a quarter turn.
    Arc::make_mut(&mut Arc::make_mut(&mut d.timeline_mut(0).layers[1]).keyframes[0]).tween.as_mut().unwrap().orient = true;
    let m = at(&d, 8);
    assert!((m.m[0]).abs() < 1e-3 && (m.m[1] - 1.0).abs() < 1e-3, "rotated 90°: {:?}", m.m);
    // The guide's line is in the list, marked, so it can be edited but is never drawn.
    let dl = evaluate(&d, 0);
    assert!(dl.items.iter().any(|i| i.element == ElementId(5) && i.guide));
    assert!(dl.items.iter().filter(|i| i.element != ElementId(5)).all(|i| !i.guide));
}

#[test]
fn movie_clips_play_their_own_timeline_graphics_keep_step_buttons_hold_up() {
    // A 4-frame symbol: a box at x = 10·frame.
    let sid = SymbolId(99);
    let mut st = Layer::new(LayerId(50), "anim");
    st.keyframes = (0..4u32)
        .map(|f| {
            let mut k = Keyframe::new(f, 1);
            k.elements.push(Arc::new(shape_el(51, f as f32 * 10.0)));
            Arc::new(k)
        })
        .collect();
    let mut d = Document::default();
    d.library.push(Arc::new(Symbol { id: sid, name: "Clip".into(), kind: SymbolKind::MovieClip, linkage: None, scale_grid_kept: None, scale_grid: None, timeline: Timeline { layers: vec![Arc::new(st)] } }));
    // The same instance on two keyframes back to back (frames 0–4 and 5–11), moved on the second.
    let inst = |x: f32| Element {
        id: ElementId(60),
        name: String::new(),
        transform: Affine::translate(x, 0.0),
        color: ColorEffect::default(),
        kind: ElementKind::Symbol { symbol: sid, loop_mode: crate::LoopMode::Loop, first_frame: 0, blend: Default::default(), filters: Vec::new(), events: Vec::new() },
    };
    let (mut k0, mut k1) = (Keyframe::new(0, 5), Keyframe::new(5, 7));
    k0.elements.push(Arc::new(inst(0.0)));
    k1.elements.push(Arc::new(inst(1000.0)));
    layer0(&mut d).keyframes = vec![Arc::new(k0), Arc::new(k1)];
    let x = |d: &Document, f| evaluate(d, f).items[0].transform.translation().x;
    // Movie clip: frame 7 of the parent is frame 7 of its run (7 mod 4 = 3), across the keyframe.
    assert_eq!(x(&d, 7), 1030.0);
    assert_eq!(x(&d, 2), 20.0);
    // Graphic: in step with the keyframe it is on (7 − 5 = 2).
    let kind = |d: &mut Document, k| Arc::make_mut(&mut d.library[0]).kind = k;
    kind(&mut d, SymbolKind::Graphic);
    assert_eq!(x(&d, 7), 1020.0);
    // Button: always its first (Up) frame.
    kind(&mut d, SymbolKind::Button);
    assert_eq!((x(&d, 2), x(&d, 7)), (0.0, 1000.0));
    // A different instance on the next keyframe starts its movie clip over.
    kind(&mut d, SymbolKind::MovieClip);
    let mut k1b = Keyframe::new(5, 7);
    k1b.elements.push(Arc::new(Element { id: ElementId(61), ..inst(1000.0) }));
    layer0(&mut d).keyframes[1] = Arc::new(k1b);
    assert_eq!(x(&d, 7), 1020.0);
}

/// A 24-frame motion tween on one shape: X keyed at 0, 12 (ease in) and 23; alpha keyed at 23.
fn motion_doc() -> Document {
    use crate::{MotionTween, Property};
    let mut d = Document::default();
    let mut k = Keyframe::new(0, 24);
    k.elements.push(Arc::new(shape_el(10, 0.0)));
    let mut m = MotionTween::new(ElementId(10));
    m.set_key(Property::X, 12, 120.0, 0.0);
    m.set_key(Property::X, 23, 120.0, 0.0);
    if let Some(t) = m.track_mut(Property::X) {
        t.keys[0].ease = Easing::QuadIn;
    }
    m.set_key(Property::Alpha, 23, 0.0, 100.0);
    k.motion = Some(m);
    // A classic tween on the same span is ignored: the motion tween wins.
    k.tween = Some(Tween::default());
    layer0(&mut d).keyframes = vec![Arc::new(k)];
    d
}

#[test]
fn a_motion_tween_drives_each_property_on_its_own_curve() {
    let d = motion_doc();
    let at = |f| {
        let dl = evaluate(&d, f);
        (dl.items[0].transform.translation().x, dl.items[0].color.mul[3])
    };
    assert_eq!(at(0), (0.0, 1.0));
    // X eases in over 0..12 (a quarter of the way at half time); alpha runs linearly over 0..23.
    let (x6, a6) = at(6);
    assert!((x6 - 30.0).abs() < 1e-3, "{x6}");
    assert!((a6 - (1.0 - 6.0 / 23.0)).abs() < 1e-5, "{a6}");
    // X holds on 12..23 while alpha keeps fading.
    let (x18, a18) = at(18);
    assert!((x18 - 120.0).abs() < 1e-3 && a18 < a6);
    assert_eq!(at(23).1, 0.0);
    // Y, scale and rotation were never keyed: they stay the element's own.
    let item = &evaluate(&d, 18).items[0];
    let dc = item.transform.decompose();
    assert!(dc.ty.abs() < 1e-4 && (dc.sx - 1.0).abs() < 1e-5 && dc.rotation.abs() < 1e-5);
}

#[test]
fn motion_tweens_survive_a_file_round_trip_and_old_files_have_none() {
    let d = motion_doc();
    let json = serde_json::to_string(&d).unwrap();
    let back: Document = serde_json::from_str(&json).unwrap();
    assert_eq!(back, d);
    assert!(back.timeline(0).layers[0].keyframes[0].motion.is_some());
    // A keyframe written before motion tweens existed loads with none.
    let old: Keyframe = serde_json::from_str(r#"{"start": 0, "duration": 3}"#).unwrap();
    assert!(old.motion.is_none());
    // Loading repairs a hostile tween and drops a classic tween beside it.
    let mut d2 = d.clone();
    let k = layer0(&mut d2).keyframe_mut(0).unwrap();
    if let Some(m) = k.motion.as_mut() {
        m.tracks[0].keys.reverse();
        m.tracks[0].keys[0].value = f32::INFINITY;
    }
    d2.sanitize();
    let k = &d2.timeline(0).layers[0].keyframes[0];
    assert!(k.tween.is_none() && k.motion.as_ref().is_some_and(crate::MotionTween::is_sane));
    for f in 0..30 {
        assert!(evaluate(&d2, f).items.iter().all(|i| i.transform.is_finite()));
    }
}

/// A square on frame 0 shape-tweening to a circle on frame 10 (the square moved 200 right,
/// so element transforms count too).
fn square_to_circle(hints: Vec<crate::ShapeHint>) -> Document {
    let mut d = Document::default();
    let el = |id, x, path| Element { id: ElementId(id), name: String::new(), transform: Affine::translate(x, 0.0), color: ColorEffect::default(), kind: ElementKind::Shape(Shape::filled(path, Paint::Solid(Rgba::BLACK))) };
    let mut k0 = Keyframe::new(0, 10);
    k0.elements.push(Arc::new(el(1, 0.0, Path::rect(0.0, 0.0, 100.0, 100.0))));
    k0.tween = Some(Tween { easing: Easing::Linear, kind: TweenKind::Shape, ..Default::default() });
    k0.hints = hints;
    let mut k1 = Keyframe::new(10, 1);
    k1.elements.push(Arc::new(el(1, 200.0, Path::ellipse(50.0, 50.0, 50.0, 50.0))));
    layer0(&mut d).keyframes = vec![Arc::new(k0), Arc::new(k1)];
    d
}

fn has_anchor(p: &Path, m: &Affine, q: anicraft_geom::Vec2) -> bool {
    p.cmds.iter().any(|c| match *c {
        anicraft_geom::PathCmd::MoveTo(a) | anicraft_geom::PathCmd::CubicTo(_, _, a) => m.apply(a).distance(q) < 1e-2,
        _ => false,
    })
}

#[test]
fn shape_hints_carry_the_hinted_corner_where_asked() {
    use anicraft_geom::Vec2;
    let (corner, bottom) = (Vec2::new(0.0, 0.0), Vec2::new(250.0, 100.0));
    let hinted = square_to_circle(vec![crate::ShapeHint { start: corner, end: bottom }]);
    let at = |d: &Document, f| {
        let dl = evaluate(d, f);
        let i = &dl.items[0];
        ((*i.path).clone(), i.transform)
    };
    // Halfway the corner is exactly halfway along its way (stage space).
    let (p, m) = at(&hinted, 5);
    assert!(has_anchor(&p, &m, corner.lerp(bottom, 0.5)), "{p:?}");
    // Without the hint it goes elsewhere.
    let (p, m) = at(&square_to_circle(Vec::new()), 5);
    assert!(!has_anchor(&p, &m, corner.lerp(bottom, 0.5)));
    // An unplaced hint (off the outline, as when just added) changes nothing.
    let off = square_to_circle(vec![crate::ShapeHint { start: Vec2::new(50.0, 50.0), end: bottom }]);
    let plain = square_to_circle(Vec::new());
    assert_eq!(*evaluate(&off, 5).items[0].path, *evaluate(&plain, 5).items[0].path);
    // The keyframes themselves are untouched by hints.
    assert_eq!(*evaluate(&hinted, 0).items[0].path, Path::rect(0.0, 0.0, 100.0, 100.0));
    // Placement and the hinted tween lookup.
    let l = &hinted.timeline(0).layers[0];
    assert_eq!(l.hint_tween_at(0), Some((0, crate::HintSide::Start)));
    assert_eq!(l.hint_tween_at(9), Some((0, crate::HintSide::Start)));
    assert_eq!(l.hint_tween_at(10), Some((0, crate::HintSide::End)));
    assert_eq!(crate::placed_on(&l.keyframes[0].elements, corner), Some(0));
    assert_eq!(crate::placed_on(&l.keyframes[1].elements, bottom), Some(0));
    assert_eq!(crate::placed_on(&l.keyframes[0].elements, Vec2::new(50.0, 50.0)), None);
    assert!(crate::shapes_centre(&l.keyframes[1].elements).unwrap().distance(Vec2::new(250.0, 50.0)) < 1e-3);
}

#[test]
fn shape_hints_survive_a_round_trip_and_old_files_have_none() {
    let d = square_to_circle(vec![crate::ShapeHint { start: anicraft_geom::Vec2::new(0.0, 0.0), end: anicraft_geom::Vec2::new(250.0, 100.0) }]);
    let k = &d.timeline(0).layers[0].keyframes[0];
    let json = serde_json::to_string(&**k).unwrap();
    assert!(json.contains("\"hints\""));
    let back: Keyframe = serde_json::from_str(&json).unwrap();
    assert_eq!(back, **k);
    // A keyframe without hints writes none, and a file from before hints reads with none.
    let plain = serde_json::to_string(&Keyframe::new(0, 1)).unwrap();
    assert!(!plain.contains("hints"));
    let old: Keyframe = serde_json::from_str(r#"{"start":0,"duration":3}"#).unwrap();
    assert!(old.hints.is_empty());
}

/// A tween from (0, 0), unturned, full size and opaque to (100, 0), half a turn, double size
/// and transparent, over 10 frames.
fn group_doc(tween: Tween) -> Document {
    let mut d = Document::default();
    let mut a = shape_el(10, 0.0);
    a.color = ColorEffect::Alpha(1.0);
    let mut b = shape_el(10, 100.0);
    b.transform = Affine::translate(100.0, 0.0).mul(&Affine::rotate(std::f32::consts::FRAC_PI_2)).mul(&Affine::scale(2.0, 2.0));
    b.color = ColorEffect::Alpha(0.0);
    let mut k0 = Keyframe::new(0, 10);
    k0.elements.push(Arc::new(a));
    k0.tween = Some(tween);
    let mut k1 = Keyframe::new(10, 1);
    k1.elements.push(Arc::new(b));
    layer0(&mut d).keyframes = vec![Arc::new(k0), Arc::new(k1)];
    d
}

#[test]
fn classic_tweens_ease_each_property_group_its_own_way() {
    use crate::EaseGroup;
    let mut tw = Tween::new(Easing::Linear, TweenKind::Classic);
    tw.set_ease(EaseGroup::Position, Some(Easing::BounceOut));
    tw.set_ease(EaseGroup::Scale, Some(Easing::QuadIn));
    tw.set_ease(EaseGroup::Color, Some(Easing::CubicOut));
    let d = group_doc(tw.clone());
    let it = |f| {
        let dl = evaluate(&d, f);
        let i = &dl.items[0];
        (i.transform.decompose(), i.color.mul[3])
    };
    let (m, alpha) = it(5);
    assert!((m.tx - 100.0 * Easing::BounceOut.apply(0.5)).abs() < 1e-2, "position: {}", m.tx);
    assert!((m.sx - (1.0 + Easing::QuadIn.apply(0.5))).abs() < 1e-3, "scale: {}", m.sx);
    // Rotation has no ease of its own: the tween's (linear).
    assert!((m.rotation - std::f32::consts::FRAC_PI_4).abs() < 1e-3, "rotation: {}", m.rotation);
    assert!((alpha - (1.0 - Easing::CubicOut.apply(0.5))).abs() < 1e-3, "colour: {alpha}");
    // Groups stay in order, once each; clearing one gives it the tween's ease again.
    tw.set_ease(EaseGroup::Position, Some(Easing::SineIn));
    assert_eq!(tw.groups.iter().map(|g| g.0).collect::<Vec<_>>(), [EaseGroup::Position, EaseGroup::Scale, EaseGroup::Color]);
    tw.set_ease(EaseGroup::Scale, None);
    assert_eq!(tw.ease(EaseGroup::Scale), &Easing::Linear);
    // With one ease for everything, every group follows it exactly as before.
    let one = group_doc(Tween::new(Easing::QuadOut, TweenKind::Classic));
    let mut same = Tween::new(Easing::QuadOut, TweenKind::Classic);
    for g in EaseGroup::ALL {
        same.set_ease(g, Some(Easing::QuadOut));
    }
    let both = group_doc(same);
    for f in 0..=10 {
        assert_eq!(evaluate(&one, f).items[0].transform, evaluate(&both, f).items[0].transform);
    }
    // Older files' tweens (no groups) still read.
    let t: Tween = serde_json::from_str(r#"{"easing": "QuadIn"}"#).unwrap();
    assert!(t.groups.is_empty());
}

#[test]
fn a_custom_ease_drives_a_classic_tween() {
    let c = crate::CustomEase::from_flat(&[(0.0, 0.0), (0.2, 1.5), (0.4, 1.5), (0.5, 1.2), (0.6, 0.9), (0.8, 1.0), (1.0, 1.0)].map(|(x, y)| anicraft_geom::Vec2::new(x, y))).unwrap();
    let e = Easing::Custom(Arc::new(c.clone()));
    let d = tween_doc(e);
    for f in 0..10 {
        let x = evaluate(&d, f).items[0].transform.translation().x;
        assert!((x - 100.0 * c.apply(f as f32 / 10.0)).abs() < 1e-2, "frame {f}: {x}");
    }
    // It overshoots past the second keyframe and comes back.
    assert!(evaluate(&d, 5).items[0].transform.translation().x > 100.0);
}

#[test]
fn instance_filters_tween_when_they_pair_up() {
    use crate::Filter;
    let mut d = Document::default();
    let sid = SymbolId(90);
    d.library.push(Arc::new(Symbol { id: sid, name: "Clip".into(), kind: SymbolKind::MovieClip, linkage: None, scale_grid_kept: None, scale_grid: None, timeline: Timeline { layers: vec![Arc::new(Layer::new(LayerId(91), "x"))] } }));
    let inst = |filters: Vec<Filter>| Element { id: ElementId(92), name: String::new(), transform: Affine::IDENTITY, color: ColorEffect::default(), kind: ElementKind::Symbol { symbol: sid, loop_mode: Default::default(), first_frame: 0, blend: Default::default(), filters: filters.into_iter().map(Into::into).collect(), events: Vec::new() } };
    let blur = |b: f32| Filter::Blur { blur_x: b, blur_y: b, quality: 1 };
    let mut tw = Tween::new(Easing::Linear, TweenKind::Classic);
    tw.set_ease(crate::EaseGroup::Filters, Some(Easing::QuadIn));
    let mut k0 = Keyframe::new(0, 10);
    k0.elements.push(Arc::new(inst(vec![blur(0.0), Filter::new("glow").unwrap()])));
    k0.tween = Some(tw);
    let mut k1 = Keyframe::new(10, 1);
    k1.elements.push(Arc::new(inst(vec![blur(40.0), Filter::new("glow").unwrap()])));
    layer0(&mut d).keyframes = vec![Arc::new(k0), Arc::new(k1)];
    let filters = |f| match &crate::evaluate_layer_elements(&d.timeline(0).layers[0], f).unwrap().1[0].kind {
        ElementKind::Symbol { filters, .. } => filters.clone(),
        _ => Vec::new(),
    };
    assert_eq!(filters(5)[0], blur(40.0 * 0.25).into(), "eased by the filters group's ease");
    // Lists that don't pair up hold the first keyframe's.
    let e = |f: Filter| crate::FilterEntry::from(f);
    assert_eq!(crate::filter::lerp_filters(&[e(blur(1.0))], &[e(Filter::new("glow").unwrap())], 0.5), None);
    assert_eq!(crate::filter::lerp_filters(&[e(blur(1.0))], &[e(blur(1.0)), e(blur(2.0))], 0.5), None);
    // A switched-off filter still pairs up (and stays off, as the first keyframe has it).
    let off = crate::FilterEntry { filter: blur(0.0), enabled: false };
    let got = crate::filter::lerp_filters(std::slice::from_ref(&off), &[e(blur(10.0))], 0.5).unwrap();
    assert_eq!(got, vec![crate::FilterEntry { filter: blur(5.0), enabled: false }]);
}
