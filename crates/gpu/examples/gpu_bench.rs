//! GPU stage timings and parity against the CPU oracle (ADR 0019, ADR 0029, ADR 0031; numbers
//! in docs/performance.md).
//!
//! ```sh
//! cargo run --release -p anicraft-gpu --example gpu_bench
//! cargo run --release -p anicraft-gpu --example gpu_bench -- --layers 50 --shapes 200
//! ```
//! Prints the adapter, then for each scene and size: CPU ms; the first slice's read-back path
//! (prepare + bin + upload, compute, copy back); the direct path the window uses (into a
//! texture, no readback): what the calling thread spends, and until the GPU has finished; the
//! stage's way (512 px tiles drawn in parallel, as the render service does, until finished);
//! the work the fine pass did; and how far the GPU's picture is from the oracle's.

use anicraft_doc::{Bitmap, BitmapId, ColorEffect, Document, Element, ElementId, ElementKind, Filter, FilterSide, Keyframe, Layer, LayerId, LayerKind, Paint, Pixels, Rgba, Shape, evaluate, sample};
use anicraft_geom::{FillRule, Path};
use anicraft_gpu::{GpuRenderer, Output, compare_premul8};
use anicraft_geom::Affine;
use anicraft_render::{View, render};
use rayon::prelude::*;
use std::sync::Arc;
use std::time::Instant;

fn arg(name: &str, default: u32) -> u32 {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(default)
}

/// The stress document with every other layer a mask over the layer below it (half the shapes
/// are mask artwork, half are drawn inside a mask), and every shape at 80 % in a group.
fn masked(mut doc: Document) -> Document {
    let tl = doc.timeline_mut(0);
    let ids: Vec<_> = tl.layers.iter().map(|l| l.id).collect();
    for (i, layer) in tl.layers.iter_mut().enumerate() {
        let l = Arc::make_mut(layer);
        if i % 2 == 0 {
            l.kind = LayerKind::Mask;
        } else {
            l.parent = ids.get(i - 1).copied();
        }
    }
    doc
}

/// The demo with the ball and star symbols faded (groups) and the hills a mask over the sky.
fn composited() -> Document {
    let mut doc = sample::demo();
    let tl = doc.timeline_mut(0);
    let hills = tl.layers.get(3).map(|l| l.id);
    for (i, layer) in tl.layers.iter_mut().enumerate() {
        let l = Arc::make_mut(layer);
        match i {
            0 | 1 => {
                for k in &mut l.keyframes {
                    for e in &mut Arc::make_mut(k).elements {
                        Arc::make_mut(e).color.alpha = 0.6;
                    }
                }
            }
            3 => l.kind = LayerKind::Mask,
            4 => l.parent = hills,
            _ => {}
        }
    }
    doc
}

/// A generated `w × h` image as a document bitmap.
fn bitmap(id: u64, w: u32, h: u32) -> Option<Arc<Bitmap>> {
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let v = (x * 7 + y * 13 + id as u32 * 29) % 256;
            rgba.extend_from_slice(&[v as u8, (255 - v) as u8, ((x * 255) / w) as u8, 255 - ((y * 120) / h) as u8]);
        }
    }
    let px = Pixels { width: w, height: h, rgba };
    let png = anicraft_render::encode_png(px.width, px.height, &px.rgba).ok()?;
    Bitmap::from_png(BitmapId(id), format!("b{id}"), png).map(Arc::new)
}

/// A layer of `n` shapes filled with `bitmaps` (in turn), each its own element.
fn bitmap_layer(id: u64, n: u32, bitmaps: &[Arc<Bitmap>], frames: u32) -> Arc<Layer> {
    let mut k = Keyframe::new(0, frames);
    for i in 0..n {
        let b = &bitmaps[i as usize % bitmaps.len()];
        let (x, y) = ((i % 20) as f32 * 64.0, (i / 20) as f32 * 72.0);
        let fill = Paint::Bitmap { bitmap: b.id, repeat: i % 3 == 0, matrix: Affine::translate(x, y).mul(&Affine::rotate(i as f32 * 0.1)).mul(&Affine::scale(0.4, 0.4)) };
        let shape = Shape { path: Arc::new(Path::ellipse(x + 40.0, y + 40.0, 60.0, 45.0)), fill: Some(fill), stroke: None, fill_rule: FillRule::NonZero };
        k.elements.push(Arc::new(Element { id: ElementId(id * 10_000 + u64::from(i)), name: String::new(), transform: Affine::IDENTITY, color: ColorEffect::default(), kind: ElementKind::Shape(shape) }));
    }
    let mut l = Layer::new(LayerId(id), "Bitmaps");
    l.keyframes = vec![Arc::new(k)];
    Arc::new(l)
}

/// 200 bitmap-filled shapes from eight 256 × 256 images, over the demo.
fn bitmaps() -> Document {
    let mut doc = sample::demo();
    let bms: Vec<Arc<Bitmap>> = (1..=8).filter_map(|i| bitmap(i, 256, 256)).collect();
    doc.bitmaps.extend(bms.iter().cloned());
    let frames = doc.frame_count();
    doc.timeline_mut(0).layers.insert(0, bitmap_layer(90_000, 200, &bms, frames));
    doc
}

/// The composited demo as artists dress it: the ball casts a drop shadow and is bevelled, the
/// star glows, and a bitmap fills the sky.
fn filtered() -> Document {
    let mut doc = composited();
    let tl = doc.timeline_mut(0);
    for (i, layer) in tl.layers.iter_mut().enumerate() {
        let l = Arc::make_mut(layer);
        for k in &mut l.keyframes {
            for e in &mut Arc::make_mut(k).elements {
                if let ElementKind::Symbol { filters, .. } = &mut Arc::make_mut(e).kind {
                    *filters = match i {
                        0 => vec![
                            Filter::DropShadow { blur_x: 8.0, blur_y: 8.0, strength: 0.8, quality: 2, angle: 45.0, distance: 10.0, color: Rgba::BLACK, inner: false, knockout: false, hide_object: false },
                            Filter::Bevel { blur_x: 5.0, blur_y: 5.0, strength: 1.0, quality: 1, angle: 45.0, distance: 4.0, shadow_color: Rgba::BLACK, highlight_color: Rgba::WHITE, side: FilterSide::Inner, knockout: false },
                        ],
                        _ => vec![Filter::Glow { blur_x: 12.0, blur_y: 12.0, strength: 1.4, quality: 2, color: Rgba::new(1.0, 0.85, 0.2, 1.0), inner: false, knockout: false }],
                    };
                }
            }
        }
    }
    if let Some(sky) = bitmap(77, 48, 32) {
        let frames = doc.frame_count();
        let mut layer = bitmap_layer(91_000, 0, std::slice::from_ref(&sky), frames);
        let l = Arc::make_mut(&mut layer);
        let fill = Paint::Bitmap { bitmap: sky.id, repeat: true, matrix: Affine::scale(5.0, 5.0) };
        let shape = Shape { path: Arc::new(Path::rect(0.0, 0.0, 1280.0, 300.0)), fill: Some(fill), stroke: None, fill_rule: FillRule::NonZero };
        if let Some(k) = l.keyframes.first_mut() {
            Arc::make_mut(k).elements.push(Arc::new(Element { id: ElementId(91_001), name: "sky".into(), transform: Affine::IDENTITY, color: ColorEffect::default(), kind: ElementKind::Shape(shape) }));
        }
        doc.bitmaps.push(sky);
        doc.timeline_mut(0).layers.push(layer);
    }
    doc
}

fn main() {
    let gpu = match GpuRenderer::new() {
        Ok(g) => g,
        Err(e) => {
            println!("no GPU: {e}");
            return;
        }
    };
    println!("adapter: {}", gpu.adapter);
    let (layers, shapes) = (arg("--layers", 50), arg("--shapes", 200));
    let runs = arg("--runs", 12);
    let scenes: Vec<(String, Document)> = vec![
        (format!("stress {layers}×{shapes}"), sample::stress(layers, shapes, 240)),
        ("1,000 shapes".into(), sample::stress(5, 200, 240)),
        (format!("masked {layers}×{shapes}"), masked(sample::stress(layers, shapes, 240))),
        ("sample".into(), sample::demo()),
        ("composited sample".into(), composited()),
        ("filtered sample".into(), filtered()),
        ("200 bitmaps".into(), bitmaps()),
    ];
    println!(
        "{:<20} {:>10} {:>8} {:>9} {:>9} {:>9} {:>7} {:>22} {:>8} {:>8}  parity (mean / max / differing px)",
        "scene", "size", "CPU ms", "readback", "direct", "direct+", "tiled", "prepare + bin + submit", "entries", "pieces"
    );
    for (name, doc) in &scenes {
        for scale in [1.0f32, 2.0] {
            let v = View::stage(doc, scale * 1920.0 / doc.stage.width as f32);
            let frames = doc.frame_count().max(1);
            let dl0 = evaluate(doc, 0);
            // Warm-up (pipeline caches, allocations).
            let _ = gpu.render(&dl0, &v, Output::Premul8);
            let _ = gpu.render_texture(&dl0, &v);
            let _ = gpu.wait_idle();
            let _ = render(&dl0, &v);
            let lists: Vec<_> = (0..runs).map(|i| evaluate(doc, (i * 20) % frames)).collect();
            let per = |t: Instant| t.elapsed().as_secs_f64() * 1000.0 / f64::from(runs);
            let t = Instant::now();
            for dl in &lists {
                let _ = render(dl, &v);
            }
            let cpu_ms = per(t);
            let t = Instant::now();
            for dl in &lists {
                let _ = gpu.render(dl, &v, Output::Premul8);
            }
            let readback_ms = per(t);
            // Direct: what the calling thread spends (the stage's cost per tile), then each frame
            // waited for to the end (the latency until the window can show it).
            let mut sum = [0.0f32; 3];
            let mut work = (0, 0);
            let t = Instant::now();
            let mut keep = Vec::with_capacity(lists.len());
            for dl in &lists {
                if let Ok(f) = gpu.render_texture(dl, &v) {
                    sum[0] += f.timings.prepare;
                    sum[1] += f.timings.bin;
                    sum[2] += f.timings.gpu;
                    work = (f.entries, f.pieces);
                    keep.push(f);
                }
            }
            let direct_ms = per(t);
            let _ = gpu.wait_idle();
            drop(keep);
            let t = Instant::now();
            for dl in &lists {
                let _ = gpu.render_texture(dl, &v);
                let _ = gpu.wait_idle();
            }
            let done_ms = per(t);
            // The stage: 512 px tiles of the same frame, in parallel, then waited for.
            let tiles: Vec<View> = (0..v.height.div_ceil(512))
                .flat_map(|ty| (0..v.width.div_ceil(512)).map(move |tx| (tx, ty)))
                .map(|(tx, ty)| View { transform: Affine::translate(-((tx * 512) as f32), -((ty * 512) as f32)).mul(&v.transform), width: 512.min(v.width - tx * 512), height: 512.min(v.height - ty * 512), background: v.background })
                .collect();
            let t = Instant::now();
            for dl in &lists {
                let _: Vec<_> = tiles.par_iter().map(|tv| gpu.render_texture(dl, tv).ok()).collect();
                let _ = gpu.wait_idle();
            }
            let tiled_ms = per(t);
            let n = runs as f32;
            let parity = match (gpu.render_texture(&dl0, &v).and_then(|t| gpu.read_texture(&t)), render(&dl0, &v)) {
                (Ok(g), Ok((c, _))) => compare_premul8(&g, &c.to_premul_rgba8()).map_or("size mismatch".into(), |d| format!("{:.4} / {} / {}", d.mean, d.max, d.differing)),
                (Err(e), _) => format!("GPU: {e}"),
                (_, Err(e)) => format!("CPU: {e}"),
            };
            println!(
                "{:<20} {:>10} {:>8.2} {:>9.2} {:>9.2} {:>9.2} {:>7.2} {:>22} {:>8} {:>8}  {parity}",
                name,
                format!("{}×{}", v.width, v.height),
                cpu_ms,
                readback_ms,
                direct_ms,
                done_ms,
                tiled_ms,
                format!("{:.2} + {:.2} + {:.2}", sum[0] / n, sum[1] / n, sum[2] / n),
                work.0,
                work.1
            );
        }
    }
}
