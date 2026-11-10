//! GPU stage timings and parity against the CPU oracle (ADR 0019, ADR 0029; numbers in
//! docs/performance.md).
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

use anicraft_doc::{Document, LayerKind, evaluate, sample};
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
