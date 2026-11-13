use super::*;
use crate::{Easing, Keyframe, Symbol, SymbolId, SymbolKind, Tween, TweenKind};

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
    Arc::make_mut(&mut d.timeline.layers[0])
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
    let (_, els) = evaluate_layer_elements(&d.timeline.layers[0], 10).unwrap();
    assert!(matches!(els, Cow::Borrowed(_)), "a non-tweened frame must not copy elements");
}

#[test]
fn hidden_and_guide_layers_are_skipped() {
    let mut d = tween_doc(Easing::Linear);
    layer0(&mut d).visible = false;
    assert!(evaluate(&d, 0).items.is_empty());
    layer0(&mut d).visible = true;
    layer0(&mut d).kind = LayerKind::Guide;
    assert!(evaluate(&d, 0).items.is_empty());
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
            kind: ElementKind::Symbol { symbol: sid, loop_mode: Default::default(), first_frame: 0 },
        },
    );
    tl.layers.push(Arc::new(layer));
    d.library.push(Arc::new(Symbol { id: sid, name: "loop".into(), kind: SymbolKind::Graphic, timeline: tl }));
    push(
        layer0(&mut d),
        0,
        Element {
            id: ElementId(60),
            name: String::new(),
            transform: Affine::translate(100.0, 0.0),
            color: ColorEffect { alpha: 0.5, ..Default::default() },
            kind: ElementKind::Symbol { symbol: sid, loop_mode: Default::default(), first_frame: 0 },
        },
    );
    let dl = evaluate(&d, 0);
    assert_eq!(dl.items.len(), MAX_SYMBOL_DEPTH);
    assert_eq!(dl.items[0].transform.translation().x, 100.0);
    assert_eq!(dl.items[0].color.alpha, 0.5);
    assert!(dl.items.iter().all(|i| i.element == ElementId(60)));
}

/// Layers top to bottom: mask (id 1), masked (2), masked (3), plain (4).
fn mask_doc() -> Document {
    let mut d = Document { timeline: Timeline::default(), ..Document::default() };
    for (id, kind, parent) in [(1, LayerKind::Mask, None), (2, LayerKind::Normal, Some(1)), (3, LayerKind::Normal, Some(1)), (4, LayerKind::Normal, None)] {
        let mut l = Layer::new(LayerId(id), format!("L{id}"));
        l.kind = kind;
        l.parent = parent.map(LayerId);
        push(&mut l, 0, shape_el(10 + id, 0.0));
        d.timeline.layers.push(Arc::new(l));
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
    Arc::make_mut(&mut h.timeline.layers[0]).visible = false;
    let dl = evaluate(&h, 0);
    assert!(dl.items.iter().all(|i| i.clip.is_none() && i.masks.is_none()) && dl.items.len() == 3);
}

#[test]
fn masks_inside_symbols_nest_in_the_instance_clip() {
    // The main timeline's masked layer holds an instance of a symbol that has its own mask.
    let mut d = mask_doc();
    let sid = SymbolId(99);
    d.library.push(Arc::new(Symbol { id: sid, name: "masked inside".into(), kind: SymbolKind::Graphic, timeline: mask_doc().timeline }));
    let inst = Element {
        id: ElementId(60),
        name: String::new(),
        transform: Affine::IDENTITY,
        color: ColorEffect::default(),
        kind: ElementKind::Symbol { symbol: sid, loop_mode: Default::default(), first_frame: 0 },
    };
    push(Arc::make_mut(&mut d.timeline.layers[1]), 0, inst);
    let dl = evaluate(&d, 0);
    assert_eq!(dl.clip_parents, vec![None, Some(0)]);
    let inner: Vec<_> = dl.items.iter().filter(|i| i.element == ElementId(60)).map(|i| (i.clip, i.masks)).collect();
    // inside: unmasked layer keeps the outer clip; masked layers use the nested clip.
    assert_eq!(inner, vec![(Some(0), None), (Some(1), None), (Some(1), None), (Some(0), Some(1))]);
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
    d.timeline.repair_masks();
    assert_eq!(d.timeline.layers.iter().map(|l| l.parent).collect::<Vec<_>>(), vec![None, Some(LayerId(1)), Some(LayerId(1)), None]);
    assert_eq!(d.timeline.group_range(0), 0..3);
    assert_eq!(d.timeline.mask_above(3), Some(LayerId(1)), "a layer under the group would join it");
    // The plain layer moved between the masked ones breaks the run: the lower one lets go.
    d.timeline.layers.swap(2, 3);
    d.timeline.repair_masks();
    assert_eq!(d.timeline.layers[3].parent, None);
    assert_eq!(d.timeline.layers[1].parent, Some(LayerId(1)));
    // A masked layer above its mask, or a mask that is no longer a mask, lets go too.
    let mut e = mask_doc();
    e.timeline.layers.swap(0, 1);
    e.timeline.repair_masks();
    assert_eq!(e.timeline.layers[0].parent, None);
    assert_eq!(e.timeline.layers[2].parent, Some(LayerId(1)), "still directly under the mask");
    let mut f = mask_doc();
    Arc::make_mut(&mut f.timeline.layers[0]).kind = LayerKind::Normal;
    f.sanitize();
    assert!(f.timeline.layers.iter().all(|l| l.parent.is_none()));
}

#[test]
fn layer_order_is_back_to_front() {
    let mut d = Document::default();
    push(layer0(&mut d), 0, shape_el(1, 0.0));
    let mut bottom = Layer::new(LayerId(7), "bottom");
    push(&mut bottom, 0, shape_el(2, 0.0));
    d.timeline.layers.push(Arc::new(bottom));
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
    layer0(&mut d).keyframe_mut(0).unwrap().tween = Some(Tween { easing: Easing::Linear, kind: TweenKind::Classic });
    assert_eq!(*path_at(&d, 5), Path::rect(0.0, 0.0, 100.0, 100.0));
    // Shape tween: halfway, the shape sits between the two and is neither of them.
    layer0(&mut d).keyframe_mut(0).unwrap().tween = Some(Tween { easing: Easing::Linear, kind: TweenKind::Shape });
    let mid = path_at(&d, 5);
    let c = mid.bounds().center();
    assert!((c.x - 150.0).abs() < 1.0 && (c.y - 50.0).abs() < 1.0, "{c:?}");
    assert_ne!(*mid, Path::rect(0.0, 0.0, 100.0, 100.0));
    // Keyframes themselves show exactly what was drawn.
    assert_eq!(*path_at(&d, 0), Path::rect(0.0, 0.0, 100.0, 100.0));
    assert_eq!(*path_at(&d, 10), Path::ellipse(250.0, 50.0, 50.0, 50.0));
    // The kind survives a save (and older files without it read as classic).
    let t: Tween = serde_json::from_str(&serde_json::to_string(&Tween { easing: Easing::Linear, kind: TweenKind::Shape }).unwrap()).unwrap();
    assert_eq!(t.kind, TweenKind::Shape);
    assert_eq!(serde_json::from_str::<Tween>(r#"{"easing":"Linear"}"#).unwrap().kind, TweenKind::Classic);
}

#[test]
fn symbols_that_branch_into_themselves_stay_bounded() {
    // 50 shapes and two instances of itself: the depth cap alone would expand it 2^32 times.
    let mut d = Document::default();
    let sid = SymbolId(99);
    let inst = |id| Element {
        id: ElementId(id),
        name: String::new(),
        transform: Affine::IDENTITY,
        color: ColorEffect::default(),
        kind: ElementKind::Symbol { symbol: sid, loop_mode: Default::default(), first_frame: 0 },
    };
    let mut layer = Layer::new(LayerId(50), "s");
    for i in 0..50 {
        push(&mut layer, 0, shape_el(100 + i, 0.0));
    }
    push(&mut layer, 0, inst(52));
    push(&mut layer, 0, inst(53));
    let tl = Timeline { layers: vec![Arc::new(layer)] };
    d.library.push(Arc::new(Symbol { id: sid, name: "fork".into(), kind: SymbolKind::Graphic, timeline: tl }));
    push(layer0(&mut d), 0, inst(60));
    let mut budget = EvaluationBudget::new(100);
    let n = evaluate_with_budget(&d, 0, &mut budget).items.len();
    assert_eq!(n, 15);
    assert!(budget.truncated());
    // Selection bounds walk the same tree.
    let mut budget = EvaluationBudget::new(100);
    assert!(!d.timeline.layers[0].keyframes[0].elements[0].bounds_with_budget(&d, &mut budget).is_empty());
    assert!(budget.truncated());
}

#[test]
fn shape_budget_boundaries_and_fresh_calls() {
    let mut d = Document::default();
    push(layer0(&mut d), 0, shape_el(10, 0.0));
    push(layer0(&mut d), 0, shape_el(11, 20.0));
    // Timeline + two layer visits + two (element + five path-command) charges.
    for (units, count, truncated) in [(15, 2, false), (14, 1, true), (0, 0, true)] {
        for _ in 0..2 {
            let mut budget = EvaluationBudget::new(units);
            assert_eq!(evaluate_with_budget(&d, 0, &mut budget).items.len(), count);
            assert_eq!(budget.truncated(), truncated);
        }
    }
    let el = shape_el(12, 20.0);
    let mut budget = EvaluationBudget::new(12);
    assert_eq!(el.local_bounds_with_budget(&d, &mut budget), anicraft_geom::Rect::new(0.0, 0.0, 10.0, 10.0));
    assert_eq!(el.bounds_with_budget(&d, &mut budget), anicraft_geom::Rect::new(20.0, 0.0, 30.0, 10.0));
    assert!(!budget.truncated());
    assert!(el.bounds_with_budget(&d, &mut budget).is_empty());
    assert!(el.local_bounds_with_budget(&d, &mut budget).is_empty());
    assert!(budget.truncated());
}

#[test]
fn layers_and_top_level_instances_share_work() {
    let mut d = Document::default();
    push(layer0(&mut d), 0, shape_el(10, 0.0));
    let mut second = Layer::new(LayerId(2), "second");
    push(&mut second, 0, shape_el(11, 20.0));
    d.timeline.layers.push(Arc::new(second));
    let mut budget = EvaluationBudget::new(10);
    let list = evaluate_with_budget(&d, 0, &mut budget);
    assert_eq!(list.items.len(), 1);
    assert_eq!(list.items[0].element, ElementId(11));
    assert!(budget.truncated());
    d.timeline.layers = (0..100).map(|id| Arc::new(Layer::new(LayerId(id), "empty"))).collect();
    let mut budget = EvaluationBudget::new(10);
    assert!(evaluate_with_budget(&d, 0, &mut budget).items.is_empty());
    assert!(budget.truncated());
}

#[test]
fn sibling_instances_share_the_budget_and_large_paths_are_charged() {
    let mut d = Document::default();
    let mut layer = Layer::new(LayerId(50), "symbol");
    push(&mut layer, 0, shape_el(50, 0.0));
    d.library.push(Arc::new(Symbol {
        id: SymbolId(99),
        name: "one shape".into(),
        kind: SymbolKind::Graphic,
        timeline: Timeline { layers: vec![Arc::new(layer)] },
    }));
    for id in [10, 11] {
        push(
            layer0(&mut d),
            0,
            Element {
                id: ElementId(id),
                name: String::new(),
                transform: Affine::IDENTITY,
                color: ColorEffect::default(),
                kind: ElementKind::Symbol { symbol: SymbolId(99), loop_mode: Default::default(), first_frame: 0 },
            },
        );
    }
    for (units, count, truncated) in [(15, 1, true), (27, 2, false)] {
        let mut budget = EvaluationBudget::new(units);
        assert_eq!(evaluate_with_budget(&d, 0, &mut budget).items.len(), count);
        assert_eq!(budget.truncated(), truncated);
    }
    assert_eq!(evaluate(&d, 0).items.len(), 2);
    assert_eq!(evaluate(&d, 0).items.len(), 2);
    let mut large = shape_el(20, 0.0);
    if let ElementKind::Shape(shape) = &mut large.kind {
        Arc::make_mut(&mut shape.path).cmds = vec![anicraft_geom::PathCmd::LineTo(anicraft_geom::Vec2::ZERO); 100];
    }
    d.timeline.layers = vec![Arc::new(Layer::new(LayerId(1), "large"))];
    push(layer0(&mut d), 0, large.clone());
    let mut budget = EvaluationBudget::new(20);
    assert!(evaluate_with_budget(&d, 0, &mut budget).items.is_empty());
    assert!(budget.truncated());
    let mut budget = EvaluationBudget::new(20);
    assert!(large.bounds_with_budget(&d, &mut budget).is_empty());
    assert!(budget.truncated());
}

#[test]
fn bounds_charge_wide_children_at_the_depth_limit() {
    let mut d = Document::default();
    let instance = Element {
        id: ElementId(10),
        name: String::new(),
        transform: Affine::IDENTITY,
        color: ColorEffect::default(),
        kind: ElementKind::Symbol { symbol: SymbolId(99), loop_mode: Default::default(), first_frame: 0 },
    };
    let mut layer = Layer::new(LayerId(50), "wide");
    for _ in 0..1000 {
        push(&mut layer, 0, instance.clone());
    }
    d.library.push(Arc::new(Symbol { id: SymbolId(99), name: "wide".into(), kind: SymbolKind::Graphic, timeline: Timeline { layers: vec![Arc::new(layer)] } }));
    let mut budget = EvaluationBudget::new(10);
    assert!(crate::element_bounds(&d, &instance, &Affine::IDENTITY, MAX_SYMBOL_DEPTH, &mut budget).is_empty());
    assert_eq!(budget.remaining, 0);
    assert!(budget.truncated());
    let mut budget = EvaluationBudget::new(1);
    assert!(crate::element_bounds(&d, &instance, &Affine::IDENTITY, MAX_SYMBOL_DEPTH + 1, &mut budget).is_empty());
    assert_eq!(budget.remaining, 0, "even a depth-rejected visit consumes work");
}
