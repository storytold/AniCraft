//! Warped bitmaps in the oracle (ADR 0079): analytic checks against the same image moved by a
//! plain transform.

use super::*;
use anicraft_doc::warp::{Pin, PinId, Warp};
use anicraft_doc::{BitmapId, ElementId, LayerId, Pixels, Shape};
use anicraft_geom::Path;
use std::sync::Arc;

/// A 32 × 16 image: a horizontal red ramp, green rows, blue on every fourth column.
fn image() -> Arc<Pixels> {
    let (w, h) = (32u32, 16u32);
    let mut rgba = Vec::new();
    for y in 0..h {
        for x in 0..w {
            rgba.extend_from_slice(&[(x * 8) as u8, (y * 16) as u8, if x % 4 == 0 { 255 } else { 0 }, 255]);
        }
    }
    Arc::new(Pixels { width: w, height: h, rgba })
}

fn bitmap_shape() -> Shape {
    Shape::filled(Path::rect(0.0, 0.0, 32.0, 16.0), Paint::Bitmap { bitmap: BitmapId(1), repeat: false, matrix: Affine::IDENTITY })
}

/// The display item of `shape` (its warp's mesh when it bends it), placed by `transform`.
fn item(shape: &Shape, transform: Affine, smooth: bool) -> DisplayItem {
    DisplayItem {
        path: shape.path.clone(),
        transform,
        fill: shape.fill.clone(),
        stroke: None,
        fill_rule: FillRule::NonZero,
        color: ColorTransform::IDENTITY,
        layer: LayerId(0),
        element: ElementId(0),
        outline: None,
        clip: None,
        masks: None,
        guide: false,
        group: None,
        image: Some(image()),
        smooth,
        warp: shape.live_warp().filter(|w| !w.at_rest()).and_then(Warp::mesh).map(Arc::new),
    }
}

fn draw(items: Vec<DisplayItem>, w: u32, h: u32) -> Pixmap {
    let dl = DisplayList { items, ..Default::default() };
    let view = View { transform: Affine::IDENTITY, width: w, height: h, background: None };
    render(&dl, &view).map(|(p, _)| p).unwrap_or_else(|_| Pixmap { width: 0, height: 0, data: Vec::new() })
}

fn pin(id: u64, rest: (f32, f32), at: (f32, f32)) -> Pin {
    Pin { id: PinId(id), rest: anicraft_geom::Vec2::new(rest.0, rest.1), at: anicraft_geom::Vec2::new(at.0, at.1) }
}

fn warped(pins: Vec<Pin>, resolution: u32) -> Shape {
    let mut s = bitmap_shape();
    let mut w = Warp::new(s.path.clone(), resolution);
    w.pins = pins;
    s.set_warp(w);
    s
}

/// Largest channel difference over pixels both images cover fully (alpha 1 in `a`).
fn worst(a: &Pixmap, b: &Pixmap) -> f32 {
    a.data.iter().zip(&b.data).filter(|(p, _)| p[3] > 0.999).map(|(p, q)| (0..4).map(|k| (p[k] - q[k]).abs()).fold(0.0, f32::max)).fold(0.0, f32::max)
}

#[test]
fn a_warp_that_only_moves_draws_the_image_moved() {
    // One pin moved: the whole image moves with it, exactly as a translated bitmap draws.
    let s = warped(vec![pin(1, (16.0, 8.0), (26.0, 13.0))], 8);
    for smooth in [true, false] {
        let got = draw(vec![item(&s, Affine::IDENTITY, smooth)], 64, 40);
        let want = draw(vec![item(&bitmap_shape(), Affine::translate(10.0, 5.0), smooth)], 64, 40);
        let covered = got.data.iter().filter(|p| p[3] > 0.999).count();
        assert_eq!(covered, 32 * 16, "the image's pixels, no more and no fewer");
        assert!(worst(&got, &want) <= 1.0 / 255.0 + 1e-6, "smooth {smooth}: {}", worst(&got, &want));
    }
}

#[test]
fn a_warp_that_turns_draws_the_image_turned() {
    // Two pins turned a quarter turn about the image's centre: every triangle turns with them.
    let c = (16.0, 8.0);
    let turn = |(x, y): (f32, f32)| (c.0 - (y - c.1) + 24.0, c.1 + (x - c.0) + 16.0);
    let s = warped(vec![pin(1, (4.0, 8.0), turn((4.0, 8.0))), pin(2, (28.0, 8.0), turn((28.0, 8.0)))], 8);
    let xf = Affine::translate(c.0 + 24.0, c.1 + 16.0).mul(&Affine::rotate(std::f32::consts::FRAC_PI_2)).mul(&Affine::translate(-c.0, -c.1));
    let got = draw(vec![item(&s, Affine::IDENTITY, false)], 80, 64);
    let want = draw(vec![item(&bitmap_shape(), xf, false)], 80, 64);
    assert!(got.data.iter().filter(|p| p[3] > 0.999).count() >= 32 * 16 - 64);
    assert!(worst(&got, &want) <= 1.0 / 255.0 + 1e-6, "{}", worst(&got, &want));
}

#[test]
fn a_bent_image_has_no_cracks_and_is_deterministic() {
    let s = warped(vec![pin(1, (2.0, 8.0), (2.0, 8.0)), pin(2, (16.0, 8.0), (16.0, 8.0)), pin(3, (30.0, 8.0), (34.0, 20.0))], 12);
    let a = draw(vec![item(&s, Affine::scale(3.0, 3.0), true)], 140, 110);
    let b = draw(vec![item(&s, Affine::scale(3.0, 3.0), true)], 140, 110);
    assert_eq!(a.data, b.data);
    // Inside the bent outline every pixel is opaque: the triangles leave no seam.
    let outline = s.path.clone();
    let m = Affine::scale(3.0, 3.0);
    let mut inside = 0;
    for y in 0..110 {
        for x in 0..140 {
            let p = anicraft_geom::Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
            if outline.contains(p, &m, FillRule::NonZero) && outline.distance_to_edge(p, &m) > 1.0 {
                inside += 1;
                let px = a.data[y * 140 + x];
                assert!(px[3] > 0.999, "a crack at ({x}, {y}): {px:?}");
            }
        }
    }
    assert!(inside > 3000);
}

#[test]
fn the_gpu_stage_gets_the_same_prepared_fill() {
    // `prepare_scene` hands the warped bitmap over as a plain image shader: what the GPU draws.
    let s = warped(vec![pin(1, (16.0, 8.0), (20.0, 8.0)), pin(2, (2.0, 2.0), (2.0, 2.0))], 6);
    let dl = DisplayList { items: vec![item(&s, Affine::IDENTITY, true)], ..Default::default() };
    let view = View { transform: Affine::IDENTITY, width: 48, height: 24, background: None };
    let (scene, _) = prepare_scene(&dl, &view).unwrap_or_else(|_| (Scene { fills: Vec::new(), clips: Vec::new(), groups: Vec::new(), layers: Vec::new() }, RenderStats::default()));
    assert_eq!(scene.fills.len(), 1);
    assert!(matches!(&scene.fills[0].shader, paint::Shader::Bitmap { smooth: false, repeat: false, .. }));
}

#[test]
fn svg_bakes_a_warped_bitmap_as_the_image_drawn() {
    let s = warped(vec![pin(1, (2.0, 8.0), (2.0, 8.0)), pin(2, (30.0, 8.0), (36.0, 18.0))], 8);
    let dl = DisplayList { items: vec![item(&s, Affine::translate(5.0, 5.0), true)], ..Default::default() };
    let stage = anicraft_doc::Stage { width: 64, height: 48, ..Default::default() };
    let svg = crate::svg::to_svg(&dl, &stage);
    assert!(svg.contains("<image x=") && svg.contains("data:image/png;base64,"), "{svg}");
    assert!(!svg.contains("<pattern"), "the unbent pattern isn't written");
    // Unwarped, the same bitmap is a pattern fill as before.
    let flat = DisplayList { items: vec![item(&bitmap_shape(), Affine::IDENTITY, true)], ..Default::default() };
    assert!(crate::svg::to_svg(&flat, &stage).contains("<pattern"));
}

#[test]
#[ignore = "a measurement: cargo test -p anicraft-render warp_measured -- --ignored --nocapture"]
fn warp_measured() {
    // The 32 × 16 image bent by three pins and drawn 30× (960 × 480 px), on a 1,000 × 700 view.
    let s = warped(vec![pin(1, (2.0, 8.0), (2.0, 8.0)), pin(2, (16.0, 8.0), (16.0, 6.0)), pin(3, (30.0, 8.0), (31.0, 14.0))], 48);
    let one = |w: bool| {
        let shape = if w { s.clone() } else { bitmap_shape() };
        let t = std::time::Instant::now();
        for _ in 0..10 {
            draw(vec![item(&shape, Affine::scale(30.0, 30.0), true)], 1000, 700);
        }
        t.elapsed().as_secs_f64() * 100.0
    };
    eprintln!("warp: 960×480 px bitmap, 48-cell mesh: {:.2} ms warped, {:.2} ms flat", one(true), one(false));
}
