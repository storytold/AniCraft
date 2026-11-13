//! 9-slice scaling (ADR 0055) in the renderer and the exports that draw evaluated frames: the
//! CPU oracle, SVG and Lottie all see the sliced outline, baked, with strokes at their width.

use anicraft_doc::{ColorEffect, Document, Element, ElementId, ElementKind, Keyframe, Layer, LayerId, Paint, Rgba, ScaleGrid, Shape, Stage, Stroke, Symbol, SymbolId, SymbolKind, Timeline, evaluate};
use anicraft_geom::{Affine, Path};
use std::sync::Arc;

/// A 100 × 40 rounded panel (radius 10, a 2-wide outline) with guides 10 in, one instance at
/// (20, 20) scaled by `sx`, `sy`, on a 400 × 200 stage.
fn panel(sx: f32, sy: f32, guides: bool) -> Document {
    let mut d = Document { stage: Stage { width: 400, height: 200, ..Stage::default() }, ..Document::default() };
    let mut shape = Shape::filled(Path::rounded_rect(0.0, 0.0, 100.0, 40.0, 10.0), Paint::Solid(Rgba::hex(0x2A9D8F)));
    shape.stroke = Some(Stroke::new(Rgba::hex(0x264653), 2.0));
    let mut k = Keyframe::new(0, 1);
    k.elements.push(Arc::new(Element { id: ElementId(2), name: String::new(), transform: Affine::IDENTITY, color: ColorEffect::default(), kind: ElementKind::Shape(shape) }));
    let mut l = Layer::new(LayerId(3), "art");
    l.keyframes = vec![Arc::new(k)];
    let grid = guides.then_some(ScaleGrid { left: 10.0, right: 90.0, top: 10.0, bottom: 30.0 });
    d.library.push(Arc::new(Symbol { id: SymbolId(4), name: "Panel".into(), kind: SymbolKind::MovieClip, linkage: None, scale_grid_kept: None, scale_grid: grid, timeline: Timeline { layers: vec![Arc::new(l)] } }));
    let inst = Element {
        id: ElementId(5),
        name: String::new(),
        transform: Affine::translate(20.0, 20.0).mul(&Affine::scale(sx, sy)),
        color: ColorEffect::default(),
        kind: ElementKind::Symbol { symbol: SymbolId(4), loop_mode: Default::default(), first_frame: 0, blend: Default::default(), filters: Vec::new(), events: Vec::new() },
    };
    let mut k = Keyframe::new(0, 1);
    k.elements.push(Arc::new(inst));
    Arc::make_mut(&mut d.timeline_mut(0).layers[0]).keyframes = vec![Arc::new(k)];
    d
}

/// The `n` × `n` block of pixels from (x0, y0).
fn pixels(d: &Document, x0: u32, y0: u32, n: u32) -> Vec<crate::Premul> {
    let pm = crate::render_frame(d, 0, 1.0).unwrap();
    (0..n).flat_map(|y| (0..n).map(move |x| (x, y))).filter_map(|(x, y)| pm.pixel(x0 + x, y0 + y)).collect()
}

fn same(a: &[crate::Premul], b: &[crate::Premul]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(p, q)| p.iter().zip(q).all(|(x, y)| (x - y).abs() < 1e-3))
}

#[test]
fn the_oracle_draws_every_corner_as_the_unscaled_one() {
    // Unscaled the panel covers (20, 20)–(120, 60); scaled 3.5 × 3, (20, 20)–(370, 140). Each
    // 12 × 12 corner block (its outline included) matches the unscaled panel's own corner.
    let (one, big, plain) = (panel(1.0, 1.0, true), panel(3.5, 3.0, true), panel(3.5, 3.0, false));
    for ((ux, uy), (bx, by)) in [((19, 19), (19, 19)), ((109, 19), (359, 19)), ((19, 49), (19, 129)), ((109, 49), (359, 129))] {
        let want = pixels(&one, ux, uy, 12);
        assert!(same(&pixels(&big, bx, by, 12), &want), "corner at ({bx}, {by})");
        assert!(!same(&pixels(&plain, bx, by, 12), &want), "plain scaling changes the corner at ({bx}, {by})");
    }
}

#[test]
fn svg_writes_the_sliced_outline_with_the_stroke_at_its_width() {
    let svg = crate::svg::to_svg(&evaluate(&panel(3.0, 3.0, true), 0), &Stage { width: 400, height: 200, ..Stage::default() });
    // The right edge lands at 20 + 300 and the stroke stays 2 wide (plain scaling: 6).
    assert!(svg.contains("320"), "{svg}");
    assert!(svg.contains("stroke-width=\"2\""), "{svg}");
    let plain = crate::svg::to_svg(&evaluate(&panel(3.0, 3.0, false), 0), &Stage { width: 400, height: 200, ..Stage::default() });
    assert!(plain.contains("stroke-width=\"6\""), "{plain}");
}

#[test]
fn lottie_bakes_the_sliced_outline() {
    let x = crate::lottie::to_lottie(&panel(3.0, 3.0, true), "panel", 0, 1, false);
    let v: serde_json::Value = serde_json::from_str(&x.json).unwrap();
    // Every vertex of every shape layer, in stage space.
    let mut xs = Vec::new();
    fn walk(v: &serde_json::Value, xs: &mut Vec<f64>) {
        match v {
            serde_json::Value::Object(o) => {
                if let Some(serde_json::Value::Array(vs)) = o.get("v") {
                    xs.extend(vs.iter().filter_map(|p| p.get(0).and_then(serde_json::Value::as_f64)));
                }
                o.values().for_each(|c| walk(c, xs));
            }
            serde_json::Value::Array(a) => a.iter().for_each(|c| walk(c, xs)),
            _ => {}
        }
    }
    walk(&v, &mut xs);
    let max = xs.iter().copied().fold(f64::MIN, f64::max);
    assert!((max - 320.0).abs() < 0.01, "{max}");
    // The top-right corner arc starts 10 in from the right edge: a vertex at x = 310.
    assert!(xs.iter().any(|x| (x - 310.0).abs() < 0.01), "{xs:?}");
    assert!(x.report.is_empty(), "{:?}", x.report);
}
