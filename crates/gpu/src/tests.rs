//! Parity with the CPU oracle.
//!
//! Two layers: the binning is checked everywhere by running the fine shader's arithmetic on the
//! CPU over the binned scene (no GPU needed), and the real GPU path is checked against the
//! oracle on the sample scene and analytic scenes when an adapter exists. Without one those
//! tests print why and pass, so CI machines without a GPU stay green.

use super::*;
use crate::binning::{Binned, PAINT_VEC4S, TILE, piece_cover, solid_of};
use anicraft_doc::{ColorEffect, DisplayItem, ElementId, GradientStop, LayerId, Paint, Rgba, Stroke, sample};
use anicraft_geom::{Affine, FillRule, Path, Vec2};
use anicraft_render::{paint::Premul, render};
use std::sync::OnceLock;

/// Mean-error target from the roadmap (M3): 1/255 per channel.
const MEAN_TARGET: f64 = 1.0;

fn item(path: Path, fill: Paint) -> DisplayItem {
    DisplayItem {
        path: Arc::new(path),
        transform: Affine::IDENTITY,
        fill: Some(fill),
        stroke: None,
        fill_rule: FillRule::NonZero,
        color: ColorEffect::default(),
        layer: LayerId(0),
        element: ElementId(0),
        outline: None,
        clip: None,
        masks: None,
        guide: false,
        group: None,
        image: None,
    }
}

fn list(items: Vec<DisplayItem>) -> DisplayList {
    DisplayList { items, ..Default::default() }
}

fn view(w: u32, h: u32, bg: Option<Rgba>) -> View {
    View { transform: Affine::IDENTITY, width: w, height: h, background: bg }
}

fn stops(colors: &[(f32, Rgba)]) -> Vec<GradientStop> {
    colors.iter().map(|&(offset, color)| GradientStop { offset, color }).collect()
}

/// Analytic scenes: each stresses one part of the coverage or paint model.
fn scenes() -> Vec<(&'static str, DisplayList, View)> {
    let red = Rgba::new(0.9, 0.1, 0.1, 1.0);
    let blue = Rgba::new(0.1, 0.2, 0.9, 0.6);
    let mut out = Vec::new();
    // Fractional rectangle edges across tile boundaries and render bands.
    out.push(("rects", list(vec![item(Path::rect(3.25, 20.5, 40.0, 61.75), Paint::Solid(red)), item(Path::rect(-30.0, 7.3, 64.6, 15.1), Paint::Solid(blue))]), view(96, 100, Some(Rgba::WHITE))));
    // A circle (curves flattened by the oracle), on transparent.
    out.push(("circle", list(vec![item(Path::ellipse(50.0, 50.0, 40.0, 40.0), Paint::Solid(Rgba::BLACK))]), view(100, 100, None)));
    // Even-odd and non-zero holes from the same rings.
    let mut ring = Path::rect(4.0, 4.0, 60.0, 60.0);
    ring.cmds.extend(Path::rect(16.0, 16.0, 36.0, 36.0).cmds);
    let mut eo = item(ring.clone(), Paint::Solid(red));
    eo.fill_rule = FillRule::EvenOdd;
    let mut nz = item(ring, Paint::Solid(blue));
    nz.transform = Affine::translate(60.0, 10.0);
    out.push(("fill rules", list(vec![eo, nz]), view(140, 80, Some(Rgba::WHITE))));
    // A self-intersecting star: winding 2 in the middle.
    let mut star = item(Path::star(40.0, 40.0, 36.0, 14.0, 5), Paint::Solid(Rgba::new(0.2, 0.6, 0.3, 1.0)));
    star.fill_rule = FillRule::EvenOdd;
    out.push(("star", list(vec![star]), view(80, 80, Some(Rgba::WHITE))));
    // Gradients under rotation and scale.
    let mut lin = item(Path::rect(0.0, 0.0, 100.0, 60.0), Paint::Linear { start: Vec2::new(0.0, 0.0), end: Vec2::new(100.0, 0.0), stops: stops(&[(0.0, red), (0.5, Rgba::WHITE), (1.0, blue)]) });
    lin.transform = Affine::translate(70.0, 10.0).mul(&Affine::rotate(0.4)).mul(&Affine::scale(1.2, 0.8));
    let rad = item(Path::ellipse(60.0, 90.0, 50.0, 30.0), Paint::Radial { center: Vec2::new(60.0, 90.0), radius: 50.0, stops: stops(&[(0.0, Rgba::WHITE), (1.0, Rgba::new(0.0, 0.0, 0.0, 0.3))]) });
    out.push(("gradients", list(vec![lin, rad]), view(180, 130, Some(Rgba::new(0.9, 0.9, 0.85, 1.0)))));
    // Strokes: wide, thin (sub-pixel, faded), on open and closed paths.
    let mut wide = item(Path::line(Vec2::new(10.0, 20.0), Vec2::new(150.0, 70.0)), Paint::Solid(red));
    wide.fill = None;
    wide.stroke = Some(Stroke { color: Rgba::BLACK, width: 9.0 });
    let mut thin = item(Path::ellipse(80.0, 50.0, 60.0, 30.0), Paint::Solid(Rgba::new(1.0, 0.9, 0.2, 1.0)));
    thin.stroke = Some(Stroke { color: Rgba::new(0.1, 0.1, 0.4, 1.0), width: 0.4 });
    out.push(("strokes", list(vec![thin, wide]), view(160, 100, Some(Rgba::WHITE))));
    // Larger than the surface on every side; colour effects (onion-skin style tint and alpha).
    let mut huge = item(Path::ellipse(50.0, 40.0, 300.0, 200.0), Paint::Solid(red));
    huge.color = ColorEffect { alpha: 0.5, tint: Rgba::hex(0x4cd07d), tint_amount: 0.7 };
    out.push(("off-surface", list(vec![huge]), view(100, 80, Some(Rgba::WHITE))));
    out
}

/// The fine shader's arithmetic, on the CPU, over a binned scene.
fn emulate(b: &Binned, v: &View) -> Vec<Premul> {
    let (w, h) = (v.width as usize, v.height as usize);
    let bg = v.background.map_or([0.0; 4], premultiply);
    let mut out = vec![[0.0f32; 4]; w * h];
    for y in 0..h {
        for x in 0..w {
            let tile = (y / TILE as usize) * b.tiles_x as usize + x / TILE as usize;
            let (lx, ly) = (x % TILE as usize, y % TILE as usize);
            let (start, count) = (b.index[2 * tile] as usize, b.index[2 * tile + 1] as usize);
            let mut c = bg;
            for i in 0..count {
                let e = b.entries_at as usize + (start + i) * 4;
                let (op, first, n, bd) = (b.index[e] as usize, b.index[e + 1] as usize, b.index[e + 2] as usize, b.index[e + 3]);
                let mut wnd = if bd == binning::NONE { 0.0 } else { b.geo[bd as usize + ly / 4][ly % 4] };
                for k in 0..n {
                    let s = b.geo[first + k];
                    wnd += piece_cover(s, lx as f32, ly as f32);
                }
                let hd = b.ops_at as usize + op * 4;
                let cov = if b.index[hd + 1] == 0 { wnd.abs().min(1.0) } else { let t = wnd.abs() % 2.0; if t > 1.0 { 2.0 - t } else { t } };
                if cov <= 1e-4 {
                    continue;
                }
                let s = match b.index[hd] {
                    0 => solid_of(b, op),
                    kind => {
                        let base = op * PAINT_VEC4S;
                        let (m, nn, g) = (b.paints[base + 1], b.paints[base + 2], b.paints[base + 3]);
                        let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                        let q = Vec2::new(m[0] * p.x + m[2] * p.y + nn[0], m[1] * p.x + m[3] * p.y + nn[1]);
                        let d = q - Vec2::new(nn[2], nn[3]);
                        let t = if kind == 1 { d.x * g[0] + d.y * g[1] } else { d.length() * g[2] };
                        let i = (t.clamp(0.0, 1.0) * 255.0 + 0.5) as usize & 255;
                        b.paints[b.index[hd + 2] as usize + i]
                    }
                };
                let sa = s[3] * cov;
                for k in 0..4 {
                    c[k] = s[k] * cov + c[k] * (1.0 - sa);
                }
            }
            out[y * w + x] = c;
        }
    }
    out
}

fn f32_diff(a: &[Premul], b: &[Premul]) -> (f64, f32) {
    let mut sum = 0.0f64;
    let mut max = 0.0f32;
    for (p, q) in a.iter().zip(b) {
        for k in 0..4 {
            let d = (p[k] - q[k]).abs();
            sum += f64::from(d);
            max = max.max(d);
        }
    }
    (sum / (a.len().max(1) * 4) as f64, max)
}

#[test]
fn binned_coverage_matches_the_oracle_on_the_cpu() {
    let mut all = scenes();
    let doc = sample::demo();
    for (f, s) in [(0u32, 0.5f32), (13, 0.37), (40, 1.0)] {
        all.push(("sample", anicraft_doc::evaluate(&doc, f), View::stage(&doc, s)));
    }
    for (name, dl, v) in all {
        let (fills, _) = plain_fills(&dl, &v).unwrap();
        let b = binning::bin(&fills, v.width, v.height);
        let (oracle, _) = render(&dl, &v).unwrap();
        let (mean, max) = f32_diff(&emulate(&b, &v), &oracle.data);
        assert!(mean < 1e-4 && max < 2e-3, "{name}: mean {mean:e}, max {max:e}");
    }
}

#[test]
fn binning_shapes_the_scene_into_tiles() {
    // One rectangle exactly covering tiles 1..3 × 1..2 of a 64 × 48 surface: its edges sit on
    // tile boundaries, so the interior tiles carry a backdrop and no pieces.
    let dl = list(vec![item(Path::rect(16.0, 16.0, 32.0, 16.0), Paint::Solid(Rgba::BLACK))]);
    let v = view(64, 48, None);
    let (fills, _) = plain_fills(&dl, &v).unwrap();
    let b = binning::bin(&fills, 64, 48);
    assert_eq!((b.tiles_x, b.tiles_y), (4, 3));
    let count = |tx: usize, ty: usize| b.index[2 * (ty * 4 + tx) + 1];
    assert_eq!((count(0, 1), count(1, 1), count(2, 1), count(3, 1)), (0, 1, 1, 0));
    assert_eq!(count(1, 0) + count(1, 2), 0, "nothing above or below");
    // Odd sizes: partial tiles at the right and bottom still bin.
    let dl = list(vec![item(Path::ellipse(20.0, 10.0, 30.0, 30.0), Paint::Solid(Rgba::BLACK))]);
    let v = view(37, 21, None);
    let (fills, _) = plain_fills(&dl, &v).unwrap();
    let b = binning::bin(&fills, 37, 21);
    assert_eq!((b.tiles_x, b.tiles_y), (3, 2));
    let (oracle, _) = render(&dl, &v).unwrap();
    assert!(f32_diff(&emulate(&b, &v), &oracle.data).1 < 2e-3);
    // Nothing to draw: an empty scene bins to empty tile lists.
    let b = binning::bin(&[], 40, 40);
    assert_eq!((b.entries, b.pieces), (0, 0));
    assert!(b.index.iter().all(|&v| v == 0));
}

#[test]
fn degenerate_geometry_bins_without_trouble() {
    // NaN and infinite points, zero-area polygons, edges far outside the surface.
    let fills = vec![anicraft_render::Fill {
        polys: vec![
            vec![Vec2::new(f32::NAN, 0.0), Vec2::new(5.0, 5.0), Vec2::new(1.0, f32::INFINITY)],
            vec![Vec2::new(3.0, 3.0), Vec2::new(3.0, 3.0), Vec2::new(3.0, 3.0)],
            vec![Vec2::new(-1e9, -1e9), Vec2::new(1e9, 5.0), Vec2::new(-1e9, 1e9)],
            vec![Vec2::new(1.0, 1.0)],
        ],
        rule: FillRule::NonZero,
        shader: anicraft_render::paint::Shader::Solid([1.0; 4]),
        bounds: anicraft_render::raster::IRect { x0: 0, y0: 0, x1: 32, y1: 32 },
    }];
    let b = binning::bin(&fills, 32, 32);
    assert!(b.geo.iter().flatten().all(|v| v.is_finite()));
}

// ----- the real GPU ----------------------------------------------------------------------------

/// One device for all tests; `None` (with the reason printed) when this machine has no usable
/// adapter.
fn gpu() -> Option<&'static GpuRenderer> {
    static GPU: OnceLock<Option<GpuRenderer>> = OnceLock::new();
    GPU.get_or_init(|| match GpuRenderer::new() {
        Ok(g) => {
            eprintln!("GPU parity tests on {}", g.adapter);
            Some(g)
        }
        Err(e) => {
            eprintln!("skipping GPU parity tests: {e}");
            None
        }
    })
    .as_ref()
}

#[test]
fn gpu_matches_the_oracle_on_analytic_scenes() {
    let Some(g) = gpu() else { return };
    for (name, dl, v) in scenes() {
        let f = g.render(&dl, &v, Output::Premul8).unwrap();
        let (oracle, _) = render(&dl, &v).unwrap();
        let d = compare_premul8(&f.premul8, &oracle.to_premul_rgba8()).unwrap();
        assert!(d.mean <= MEAN_TARGET / 16.0 && d.max <= 2, "{name}: {d:?}");
        // Float output agrees closely too.
        let ff = g.render(&dl, &v, Output::Float).unwrap();
        let (mean, max) = f32_diff(&ff.pixmap.unwrap().data, &oracle.data);
        assert!(mean < 1e-4 && max < 4e-3, "{name} (f32): mean {mean:e}, max {max:e}");
    }
}

#[test]
fn gpu_matches_the_oracle_on_the_sample_scene() {
    let Some(g) = gpu() else { return };
    let doc = sample::demo();
    let frames = doc.frame_count();
    for (f, s) in [(0u32, 1.0f32), (7, 0.5), (13, 0.37), (frames / 2, 1.5), (frames.saturating_sub(1), 0.25)] {
        let dl = anicraft_doc::evaluate(&doc, f);
        let v = View::stage(&doc, s);
        let gpu = g.render(&dl, &v, Output::Premul8).unwrap();
        let (oracle, st) = render(&dl, &v).unwrap();
        let d = compare_premul8(&gpu.premul8, &oracle.to_premul_rgba8()).unwrap();
        assert!(d.mean <= MEAN_TARGET / 16.0 && d.max <= 2, "frame {f} at {s}: {d:?}");
        assert_eq!((gpu.stats.drawn, gpu.stats.culled), (st.drawn, st.culled));
    }
}

#[test]
fn gpu_matches_the_oracle_on_many_shapes_and_tiles() {
    let Some(g) = gpu() else { return };
    let doc = sample::stress(6, 300, 24);
    let dl = anicraft_doc::evaluate(&doc, 5);
    // A whole frame and an offset, zoomed tile like the stage asks for.
    for v in [View::stage(&doc, 0.5), View { transform: Affine::translate(-512.0, -256.0).mul(&Affine::scale(1.7, 1.7)), width: 512, height: 512, background: Some(doc.stage.background) }] {
        let gpu = g.render(&dl, &v, Output::Premul8).unwrap();
        let (oracle, _) = render(&dl, &v).unwrap();
        let d = compare_premul8(&gpu.premul8, &oracle.to_premul_rgba8()).unwrap();
        assert!(d.mean <= MEAN_TARGET / 16.0 && d.max <= 2, "{d:?}");
    }
}

#[test]
fn gpu_hands_back_what_it_does_not_draw_and_stays_usable() {
    let Some(g) = gpu() else { return };
    let mut clipped = item(Path::rect(0.0, 0.0, 8.0, 8.0), Paint::Solid(Rgba::BLACK));
    clipped.clip = Some(0);
    let dl = DisplayList { items: vec![clipped], clip_parents: vec![None], groups: Vec::new() };
    assert!(matches!(g.render(&dl, &view(8, 8, None), Output::Premul8), Err(GpuError::NeedsFull(NeedsFull::Clips))));
    assert!(matches!(g.render(&list(vec![]), &view(0, 8, None), Output::Premul8), Err(GpuError::TooLarge(_))));
    assert!(matches!(g.render(&list(vec![]), &view(100_000, 100_000, None), Output::Premul8), Err(GpuError::TooLarge(_))));
    // An empty list is just the background; and the device is still fine afterwards.
    let f = g.render(&list(vec![]), &view(20, 3, Some(Rgba::new(0.0, 1.0, 0.0, 1.0))), Output::Premul8).unwrap();
    assert!(f.premul8.as_chunks::<4>().0.iter().all(|p| *p == [0, 255, 0, 255]));
    assert!(g.broken().is_none());
}

#[test]
fn gpu_renders_from_many_threads_at_once() {
    let Some(g) = gpu() else { return };
    let doc = sample::demo();
    let dl = anicraft_doc::evaluate(&doc, 9);
    let v = View::stage(&doc, 0.25);
    let first = g.render(&dl, &v, Output::Premul8).unwrap().premul8;
    use rayon::prelude::*;
    let all: Vec<Vec<u8>> = (0..8).into_par_iter().map(|_| g.render(&dl, &v, Output::Premul8).unwrap().premul8).collect();
    assert!(all.iter().all(|a| *a == first), "deterministic under concurrency");
}
