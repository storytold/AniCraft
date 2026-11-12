//! GPU stage timings and parity against the CPU oracle (ADR 0019, 0029, 0032, 0048; numbers
//! in docs/performance.md).
//!
//! ```sh
//! cargo run --release -p anicraft-gpu --example gpu_bench
//! cargo run --release -p anicraft-gpu --example gpu_bench -- --layers 50 --shapes 200
//! cargo run --release -p anicraft-gpu --example gpu_bench -- --only onion
//! cargo run --release -p anicraft-gpu --example gpu_bench -- --check   # fails past a budget
//! ```
//! Prints the adapter, then for each scene and size: CPU ms; then, with the tiles binned on
//! the CPU and then on the GPU (ADR 0048), the direct path the window uses (into a texture, no
//! readback): what the calling thread spends, and until the GPU has finished; the stage's way
//! (512 px tiles drawn in parallel, as the render service does, until finished); and how far
//! the GPU-binned picture is from the oracle's.

use anicraft_doc::{Bitmap, BitmapId, ColorEffect, ColorTransform, DisplayList, Document, Element, ElementId, ElementKind, Filter, FilterSide, Keyframe, Layer, LayerId, LayerKind, Paint, Pixels, Rgba, Shape, evaluate, sample};
use anicraft_geom::{FillRule, Path};
use anicraft_gpu::{Binning, GpuRenderer, compare_premul8};
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
                        Arc::make_mut(e).color = ColorEffect::Alpha(0.6);
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
        let shape = Shape { merge: false, path: Arc::new(Path::ellipse(x + 40.0, y + 40.0, 60.0, 45.0)), fill: Some(fill), stroke: None, fill_rule: FillRule::NonZero };
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
        let shape = Shape { merge: false, path: Arc::new(Path::rect(0.0, 0.0, 1280.0, 300.0)), fill: Some(fill), stroke: None, fill_rule: FillRule::NonZero };
        if let Some(k) = l.keyframes.first_mut() {
            Arc::make_mut(k).elements.push(Arc::new(Element { id: ElementId(91_001), name: "sky".into(), transform: Affine::IDENTITY, color: ColorEffect::default(), kind: ElementKind::Shape(shape) }));
        }
        doc.bitmaps.push(sky);
        doc.timeline_mut(0).layers.push(layer);
    }
    doc
}

/// Onion skin as the stage composes it (ADR 0047), for the bench: no onion skin, ±5 frames
/// tinted and fading, or ±5 frames as outlines.
#[derive(Clone, Copy, PartialEq)]
enum Onion {
    Off,
    Tinted,
    Outlines,
}

/// Frame `frame` with ±5 onion frames, as `anicraft_engine::onion::compose` draws them with
/// the default colours, tint and fade: each frame's items under the playhead's on their own
/// layer (nearest on top), at its alpha and tint, or as hairline outlines in its colour. (The
/// engine sits beside this crate, so the bench builds the same list itself.)
fn onion_list(doc: &Document, frame: u32, mode: Onion) -> DisplayList {
    let now = evaluate(doc, frame);
    if mode == Onion::Off {
        return now;
    }
    let frames = doc.frame_count().max(1);
    let (past, future) = (Rgba::hex(0x4f8dff), Rgba::hex(0x4cd07d));
    let rank: std::collections::HashMap<LayerId, usize> = doc.timeline(0).layers.iter().enumerate().map(|(i, l)| (l.id, i)).collect();
    let mut dl = DisplayList::default();
    let mut part_of: Vec<usize> = Vec::new();
    for d in (1..=5u32).rev() {
        for (is_past, f) in [(true, frame.checked_sub(d)), (false, Some(frame + d).filter(|f| *f < frames))] {
            let Some(f) = f else { continue };
            // Fading from 0.5 next to the playhead to 0.12 at the marker; tinted at 0.8.
            let alpha = 0.5 + (0.12 - 0.5) * (d - 1) as f32 / 4.0;
            let color = if is_past { past } else { future };
            let mut part = evaluate(doc, f);
            for item in &mut part.items {
                item.color = item.color.concat(&ColorTransform::alpha(alpha).concat(&ColorTransform::tint(color, 0.8)));
                if mode == Onion::Outlines && item.masks.is_none() {
                    item.outline = Some(color.with_alpha(alpha));
                }
            }
            part_of.extend(std::iter::repeat_n(part_of.last().map_or(0, |p| p + 1), part.items.len()));
            dl.append(part);
        }
    }
    let k = part_of.last().map_or(0, |p| p + 1);
    part_of.extend(std::iter::repeat_n(k, now.items.len()));
    dl.append(now);
    // Layer by layer, bottom first; within a layer, the parts in order (a stable sort).
    let mut order: Vec<usize> = (0..dl.items.len()).collect();
    order.sort_by_key(|&i| (std::cmp::Reverse(dl.items.get(i).and_then(|it| rank.get(&it.layer)).copied().unwrap_or(0)), part_of.get(i).copied().unwrap_or(0)));
    let mut items: Vec<Option<anicraft_doc::DisplayItem>> = std::mem::take(&mut dl.items).into_iter().map(Some).collect();
    dl.items = order.into_iter().filter_map(|i| items.get_mut(i).and_then(Option::take)).collect();
    dl
}

/// Timing budgets for the stage on the GPU (docs/performance.md › Budgets), checked with
/// `--check` on a known machine: `(scene, width, what, limit in ms)`, `what` being the Auto
/// column's `direct` or `tiled` (the stage's way).
const BUDGETS: &[(&str, u32, &str, f64)] = &[
    ("1,000 shapes", 3840, "tiled", 16.7),
    ("stress 50×200", 3840, "tiled", 16.7),
    ("50,000 shapes", 1920, "tiled", 33.3),
    ("50,000 shapes", 3840, "direct", 33.3),
];
/// Outline mode against tinted onion skin (ADR 0048: no slower), at 1080p in stage tiles.
const OUTLINE_RATIO: f64 = 1.25;

fn main() {
    let check = std::env::args().any(|a| a == "--check");
    let mut measured: Vec<(String, u32, &str, f64)> = Vec::new();
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
    let only: Option<String> = std::env::args().skip_while(|a| a != "--only").nth(1);
    let onion_doc = sample::stress(20, 100, 48);
    let plain = |name: String, doc: Document| (name, doc, Onion::Off);
    let scenes: Vec<(String, Document, Onion)> = vec![
        plain(format!("stress {layers}×{shapes}"), sample::stress(layers, shapes, 240)),
        plain("1,000 shapes".into(), sample::stress(5, 200, 240)),
        plain("50,000 shapes".into(), sample::stress(250, 200, 240)),
        plain(format!("masked {layers}×{shapes}"), masked(sample::stress(layers, shapes, 240))),
        plain("sample".into(), sample::demo()),
        plain("composited sample".into(), composited()),
        plain("filtered sample".into(), filtered()),
        plain("200 bitmaps".into(), bitmaps()),
        ("onion off 20×100".into(), onion_doc.clone(), Onion::Off),
        ("onion ±5 tinted".into(), onion_doc.clone(), Onion::Tinted),
        ("onion ±5 outlines".into(), onion_doc, Onion::Outlines),
    ];
    println!("Per scene and size: the CPU oracle, then the GPU stage with the tiles binned on the CPU, then as Binning::Auto bins them (on the GPU with --forced):");
    println!("direct = the calling thread's time with render_texture (prepare + bin + upload/submit), waited = until the GPU has finished,");
    println!("tiled = 512 px stage tiles in parallel until finished. Parity: the GPU-binned picture against the oracle (mean / max / px).");
    println!(
        "{:<20} {:>10} {:>7} | {:>7} {:>7} {:>7} {:>19} | {:>7} {:>7} {:>7} {:>19} | parity",
        "scene", "size", "CPU", "direct", "waited", "tiled", "prep + bin + gpu", "direct", "waited", "tiled", "prep + bin + gpu"
    );
    for (name, doc, onion) in scenes.iter().filter(|(n, ..)| only.as_ref().is_none_or(|o| n.contains(o.as_str()))) {
        for scale in [1.0f32, 2.0] {
            let v = View::stage(doc, scale * 1920.0 / doc.stage.width as f32);
            let frames = doc.frame_count().max(1);
            let evaluate = |doc: &Document, f: u32| onion_list(doc, f, *onion);
            let dl0 = evaluate(doc, if *onion == Onion::Off { 0 } else { 30 });
            let lists: Vec<_> = (0..runs).map(|i| evaluate(doc, (i * 20) % frames)).collect();
            let per = |t: Instant| t.elapsed().as_secs_f64() * 1000.0 / f64::from(runs);
            let _ = render(&dl0, &v);
            let t = Instant::now();
            for dl in &lists {
                let _ = render(dl, &v);
            }
            let cpu_ms = per(t);
            // The stage's tiles: 512 px of the same frame, drawn in parallel.
            let tiles: Vec<View> = (0..v.height.div_ceil(512))
                .flat_map(|ty| (0..v.width.div_ceil(512)).map(move |tx| (tx, ty)))
                .map(|(tx, ty)| View { transform: Affine::translate(-((tx * 512) as f32), -((ty * 512) as f32)).mul(&v.transform), width: 512.min(v.width - tx * 512), height: 512.min(v.height - ty * 512), background: v.background })
                .collect();
            let mut cols = Vec::new();
            let mut binned_on_gpu = false;
            let second = if std::env::args().any(|a| a == "--forced") { Binning::Gpu } else { Binning::Auto };
            for binning in [Binning::Cpu, second] {
                // Warm-up (pipeline caches, allocations).
                let _ = gpu.render_texture_with(&dl0, &v, binning);
                let _ = gpu.wait_idle();
                let mut sum = [0.0f32; 3];
                let t = Instant::now();
                let mut keep = Vec::with_capacity(lists.len());
                for dl in &lists {
                    if let Ok(f) = gpu.render_texture_with(dl, &v, binning) {
                        sum[0] += f.timings.prepare;
                        sum[1] += f.timings.bin;
                        sum[2] += f.timings.gpu;
                        binned_on_gpu |= f.gpu_binned;
                        keep.push(f);
                    }
                }
                let direct = per(t);
                let _ = gpu.wait_idle();
                drop(keep);
                let t = Instant::now();
                for dl in &lists {
                    let _ = gpu.render_texture_with(dl, &v, binning);
                    let _ = gpu.wait_idle();
                }
                let waited = per(t);
                let t = Instant::now();
                for dl in &lists {
                    let _: Vec<_> = tiles.par_iter().map(|tv| gpu.render_texture_with(dl, tv, binning).ok()).collect();
                    let _ = gpu.wait_idle();
                }
                let tiled = per(t);
                let n = runs as f32;
                if binning == Binning::Auto {
                    measured.push((name.clone(), v.width, "direct", direct));
                    measured.push((name.clone(), v.width, "tiled", tiled));
                }
                cols.push(format!("{direct:>7.2} {waited:>7.2} {tiled:>7.2} {:>19}", format!("{:.2} + {:.2} + {:.2}", sum[0] / n, sum[1] / n, sum[2] / n)));
            }
            let parity = match (gpu.render_texture_with(&dl0, &v, Binning::Gpu).and_then(|t| gpu.read_texture(&t)), render(&dl0, &v)) {
                (Ok(g), Ok((c, _))) => compare_premul8(&g, &c.to_premul_rgba8()).map_or("size mismatch".into(), |d| format!("{:.4} / {} / {}", d.mean, d.max, d.differing)),
                (Err(e), _) => format!("GPU: {e}"),
                (_, Err(e)) => format!("CPU: {e}"),
            };
            let note = if binned_on_gpu { "" } else { " (binned on the CPU: masks, groups or few edges)" };
            println!("{:<20} {:>10} {:>7.2} | {} | {} | {parity}{note}", name, format!("{}×{}", v.width, v.height), cpu_ms, cols[0], cols[1]);
        }
    }
    if !check {
        return;
    }
    let get = |scene: &str, w: u32, what: &str| measured.iter().find(|m| m.0 == scene && m.1 == w && m.2 == what).map(|m| m.3);
    let mut missed = 0;
    println!();
    println!("budgets (the means above; run on a quiet machine)");
    for &(scene, w, what, limit) in BUDGETS {
        let got = get(scene, w, what);
        let ok = got.is_some_and(|g| g <= limit);
        missed += usize::from(!ok);
        println!("  {} {scene} at {w} px, {what}: {} (budget {limit} ms)", if ok { "ok  " } else { "MISS" }, got.map_or("not measured".into(), |g| format!("{g:.2} ms")));
    }
    let ratio = get("onion ±5 outlines", 1920, "tiled").zip(get("onion ±5 tinted", 1920, "tiled")).map(|(o, t)| o / t);
    let ok = ratio.is_some_and(|r| r <= OUTLINE_RATIO);
    missed += usize::from(!ok);
    println!("  {} onion outlines / tinted at 1920 px, tiled: {} (budget {OUTLINE_RATIO})", if ok { "ok  " } else { "MISS" }, ratio.map_or("not measured".into(), |r| format!("{r:.2}")));
    if missed > 0 {
        println!("{missed} budget(s) missed");
        std::process::exit(1);
    }
}
