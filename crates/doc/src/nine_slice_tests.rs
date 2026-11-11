//! 9-slice scaling (ADR 0055), analytically: what evaluation draws for a scaled instance.

use crate::{ColorEffect, Document, Element, ElementId, ElementKind, Keyframe, Layer, LayerId, Paint, Rgba, ScaleGrid, Shape, Symbol, SymbolId, SymbolKind, Timeline, evaluate};
use anicraft_geom::{Affine, Path, Vec2};
use std::sync::Arc;

const SYM: SymbolId = SymbolId(900);

fn el(id: u64, transform: Affine, kind: ElementKind) -> Element {
    Element { id: ElementId(id), name: String::new(), transform, color: ColorEffect::default(), kind }
}

fn instance(of: SymbolId, transform: Affine) -> Element {
    el(1, transform, ElementKind::Symbol { symbol: of, loop_mode: Default::default(), first_frame: 0, blend: Default::default(), filters: Vec::new(), events: Vec::new() })
}

/// A 100×40 rounded rectangle (corner radius 10) at the symbol's origin, guides 10 in from each
/// edge, with `extra` inside it, and one instance of it on the stage placed by `xf`.
fn doc(xf: Affine, grid: Option<ScaleGrid>, extra: Vec<Element>) -> Document {
    let mut d = Document::default();
    let mut shape = Shape::filled(Path::rounded_rect(0.0, 0.0, 100.0, 40.0, 10.0), Paint::Solid(Rgba::BLACK));
    shape.stroke = Some(crate::Stroke::new(Rgba::WHITE, 2.0));
    let mut k = Keyframe::new(0, 1);
    k.elements.push(Arc::new(el(2, Affine::IDENTITY, ElementKind::Shape(shape))));
    k.elements.extend(extra.into_iter().map(Arc::new));
    let mut layer = Layer::new(LayerId(901), "art");
    layer.keyframes = vec![Arc::new(k)];
    d.library.push(Arc::new(Symbol { id: SYM, name: "Panel".into(), kind: SymbolKind::MovieClip, linkage: None, scale_grid_kept: None, scale_grid: grid, timeline: Timeline { layers: vec![Arc::new(layer)] } }));
    let mut k = Keyframe::new(0, 1);
    k.elements.push(Arc::new(instance(SYM, xf)));
    Arc::make_mut(&mut d.timeline_mut(0).layers[0]).keyframes = vec![Arc::new(k)];
    d
}

const GRID: ScaleGrid = ScaleGrid { left: 10.0, right: 90.0, top: 10.0, bottom: 30.0 };

/// The stage-space outline of the first item, finely flattened.
fn outline(d: &Document) -> Vec<Vec2> {
    let dl = evaluate(d, 0);
    let it = &dl.items[0];
    it.path.flatten(&it.transform, 0.005).into_iter().flat_map(|l| l.points).collect()
}

/// How far `p` is from the polyline `pts` (closed).
fn dist(pts: &[Vec2], p: Vec2) -> f32 {
    let n = pts.len();
    (0..n).map(|i| anicraft_geom::segment_distance(p, pts[i], pts[(i + 1) % n])).fold(f32::INFINITY, f32::min)
}

/// A point on the source corner arc of the rounded rectangle, `a` radians round the corner
/// centred at `c`.
fn arc(c: Vec2, a: f32) -> Vec2 {
    c + Vec2::new(a.cos(), a.sin()) * 10.0
}

#[test]
fn a_rounded_rect_scaled_3x_keeps_its_corner_radii_exact() {
    let d = doc(Affine::scale(3.0, 3.0), Some(GRID), Vec::new());
    let pts = outline(&d);
    // The box lands where plain scaling puts it: 0..300 by 0..120.
    let (lo, hi) = pts.iter().fold((Vec2::new(f32::MAX, f32::MAX), Vec2::new(f32::MIN, f32::MIN)), |(lo, hi), p| (Vec2::new(lo.x.min(p.x), lo.y.min(p.y)), Vec2::new(hi.x.max(p.x), hi.y.max(p.y))));
    assert!((lo.x).abs() < 1e-3 && (lo.y).abs() < 1e-3 && (hi.x - 300.0).abs() < 1e-3 && (hi.y - 120.0).abs() < 1e-3, "{lo:?} {hi:?}");
    // Each corner is the same radius-10 arc, moved to its corner of the bigger box.
    let corners = [(Vec2::new(10.0, 10.0), Vec2::ZERO, 180.0f32), (Vec2::new(90.0, 10.0), Vec2::new(200.0, 0.0), 270.0), (Vec2::new(90.0, 30.0), Vec2::new(200.0, 80.0), 0.0), (Vec2::new(10.0, 30.0), Vec2::new(0.0, 80.0), 90.0)];
    for (c, shift, from) in corners {
        for i in 0..=8 {
            let a = (from + i as f32 * 90.0 / 8.0).to_radians();
            let want = arc(c, a) + shift;
            let d = dist(&pts, want);
            assert!(d < 0.02, "corner at {c:?}, {i}/8: {d}");
            // And the arc's centre is still 10 from it: the radius is exact, not stretched.
            assert!(((want - (c + shift)).length() - 10.0).abs() < 1e-3);
        }
    }
    // Without guides, the same instance is a plain 3× scale (corners radius 30).
    let plain = evaluate(&doc(Affine::scale(3.0, 3.0), None, Vec::new()), 0);
    assert_eq!(plain.items[0].transform, Affine::scale(3.0, 3.0));
}

#[test]
fn the_centre_scales_by_the_remainder_and_edges_one_way() {
    // 2× wide, 1.5× high: 200 × 60. The centre is 200 − 20 = 180 wide (80 → 180) and
    // 60 − 20 = 40 high (20 → 40).
    let d = doc(Affine::scale(2.0, 1.5), Some(GRID), Vec::new());
    let it = &evaluate(&d, 0).items[0];
    let map = |p: Vec2| it.transform.apply(p);
    let pts = outline(&d);
    // The middle of the top edge (50, 0) lands at (100, 0); of the left edge (0, 20) at (0, 30).
    assert!(dist(&pts, Vec2::new(100.0, 0.0)) < 1e-3);
    assert!(dist(&pts, Vec2::new(0.0, 30.0)) < 1e-3);
    // The straight top edge runs from x = 10 to x = 190 (the corners are 10 each).
    assert!(dist(&pts, Vec2::new(10.0, 0.0)) < 1e-3 && dist(&pts, Vec2::new(190.0, 0.0)) < 1e-3);
    // The stroke keeps its width (no scale is left in the item's transform).
    assert_eq!(map(Vec2::new(1.0, 0.0)) - map(Vec2::ZERO), Vec2::new(1.0, 0.0));
    assert_eq!(it.stroke.as_ref().map(|s| s.width), Some(2.0));
}

#[test]
fn no_spare_space_clamps_the_corners_and_closes_the_centre() {
    // 0.1× wide: 10 across, less than the corners' 20. They share it (5 each) and the centre is
    // a line; nothing turns inside out.
    let d = doc(Affine::scale(0.1, 1.0), Some(GRID), Vec::new());
    let pts = outline(&d);
    assert!(pts.iter().all(|p| p.x >= -1e-3 && p.x <= 10.0 + 1e-3), "{pts:?}");
    assert!(dist(&pts, Vec2::new(5.0, 0.0)) < 1e-3);
    // Exactly no spare space (0.2× → 20 across): corners full size, the centre a line.
    let pts = outline(&doc(Affine::scale(0.2, 1.0), Some(GRID), Vec::new()));
    assert!(dist(&pts, arc(Vec2::new(10.0, 10.0), 225f32.to_radians())) < 0.02);
    assert!(pts.iter().all(|p| p.x <= 20.0 + 1e-3));
    // Zero scale: everything on one line, still finite.
    let pts = outline(&doc(Affine::scale(0.0, 1.0), Some(GRID), Vec::new()));
    assert!(pts.iter().all(|p| p.x.abs() < 1e-3 && p.y.is_finite()));
}

#[test]
fn rotation_skew_and_flips_apply_after_slicing() {
    let rot = 30f32.to_radians();
    let xf = Affine::translate(400.0, 300.0).mul(&Affine::rotate(rot)).mul(&Affine::scale(3.0, 3.0));
    let d = doc(xf, Some(GRID), Vec::new());
    let it = &evaluate(&d, 0).items[0];
    // The item's transform is the instance's translation and rotation, with no scale.
    let expect = Affine::translate(400.0, 300.0).mul(&Affine::rotate(rot));
    assert!(it.transform.m.iter().zip(expect.m).all(|(a, b)| (a - b).abs() < 1e-4), "{:?}", it.transform);
    // The sliced corner (the top-right arc, moved by 200) turns with the instance.
    let pts = outline(&d);
    let want = expect.apply(arc(Vec2::new(90.0, 10.0), 315f32.to_radians()) + Vec2::new(200.0, 0.0));
    assert!(dist(&pts, want) < 0.02);
    // A horizontal flip slices by the magnitude, then mirrors.
    let flip = doc(Affine::scale(3.0, -3.0), Some(GRID), Vec::new());
    let it = &evaluate(&flip, 0).items[0];
    assert!((it.transform.determinant() + 1.0).abs() < 1e-4);
    let pts = outline(&flip);
    assert!(dist(&pts, Vec2::new(300.0, -120.0)).is_finite() && pts.iter().all(|p| p.y <= 1e-3 && p.y >= -120.0 - 1e-3));
}

#[test]
fn unscaled_instances_and_nested_symbols_draw_as_they_are() {
    // At scale 1 nothing is cut: the symbol's own path is drawn, shared.
    let d = doc(Affine::translate(5.0, 5.0), Some(GRID), Vec::new());
    let dl = evaluate(&d, 0);
    let ElementKind::Shape(s) = &d.symbol(SYM).unwrap().timeline.layers[0].keyframes[0].elements[0].kind else { panic!("the panel is a shape") };
    let sym_path = s.path.clone();
    assert!(Arc::ptr_eq(&dl.items[0].path, &sym_path));
    // A nested symbol inside scales normally: its item carries the whole 3× scale.
    let mut d = doc(Affine::scale(3.0, 3.0), Some(GRID), vec![instance(SymbolId(950), Affine::translate(50.0, 20.0))]);
    let mut k = Keyframe::new(0, 1);
    k.elements.push(Arc::new(el(3, Affine::IDENTITY, ElementKind::Shape(Shape::filled(Path::rect(0.0, 0.0, 4.0, 4.0), Paint::Solid(Rgba::WHITE))))));
    let mut l = Layer::new(LayerId(951), "dot");
    l.keyframes = vec![Arc::new(k)];
    d.library.push(Arc::new(Symbol { id: SymbolId(950), name: "Dot".into(), kind: SymbolKind::Graphic, linkage: None, scale_grid_kept: None, scale_grid: None, timeline: Timeline { layers: vec![Arc::new(l)] } }));
    let dl = evaluate(&d, 0);
    let dot = dl.items.iter().find(|i| i.path.bounds().width() == 4.0).unwrap();
    assert_eq!(dot.transform, Affine::scale(3.0, 3.0).mul(&Affine::translate(50.0, 20.0)));
}

#[test]
fn hostile_guides_and_scales_never_break_evaluation() {
    for grid in [
        ScaleGrid { left: 90.0, right: 10.0, top: 10.0, bottom: 30.0 },
        ScaleGrid { left: f32::NAN, right: 10.0, top: 10.0, bottom: 30.0 },
        ScaleGrid { left: -1e9, right: 1e9, top: -5.0, bottom: 500.0 },
        ScaleGrid { left: 50.0, right: 50.0, top: 20.0, bottom: 20.0 },
    ] {
        for xf in [Affine::scale(3.0, 3.0), Affine::scale(1e-30, 1e30), Affine::scale(f32::INFINITY, 1.0), Affine { m: [0.0, 0.0, 0.0, 0.0, 1.0, 1.0] }] {
            let dl = evaluate(&doc(xf, Some(grid), Vec::new()), 0);
            for it in &dl.items {
                let _ = it.path.flatten(&it.transform, 0.25);
            }
        }
    }
}
