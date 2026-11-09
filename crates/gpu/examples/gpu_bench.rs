//! GPU stage timings and parity against the CPU oracle (ADR 0014; numbers in
//! docs/performance.md).
//!
//! ```sh
//! cargo run --release -p anicraft-gpu --example gpu_bench
//! cargo run --release -p anicraft-gpu --example gpu_bench -- --layers 50 --shapes 200
//! ```
//! Prints the adapter, then for each scene and size: CPU ms, GPU ms (prepare + bin + GPU),
//! the work the fine pass did, and how far the two pictures are apart.

use anicraft_doc::{Document, evaluate, sample};
use anicraft_gpu::{GpuRenderer, Output, compare_premul8};
use anicraft_render::{View, render};
use std::time::Instant;

fn arg(name: &str, default: u32) -> u32 {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(default)
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
        ("sample".into(), sample::demo()),
    ];
    println!("{:<18} {:>10} {:>9} {:>9} {:>30} {:>9} {:>9}  parity (mean / max / differing px)", "scene", "size", "CPU ms", "GPU ms", "prepare + bin + gpu", "entries", "pieces");
    for (name, doc) in &scenes {
        for scale in [1.0f32, 2.0] {
            let v = View::stage(doc, scale * 1920.0 / doc.stage.width as f32);
            let frames = doc.frame_count().max(1);
            let dl0 = evaluate(doc, 0);
            // Warm-up (pipeline caches, allocations).
            let _ = gpu.render(&dl0, &v, Output::Premul8);
            let _ = render(&dl0, &v);
            let lists: Vec<_> = (0..runs).map(|i| evaluate(doc, (i * 20) % frames)).collect();
            let t = Instant::now();
            for dl in &lists {
                let _ = render(dl, &v);
            }
            let cpu_ms = t.elapsed().as_secs_f64() * 1000.0 / f64::from(runs);
            let mut sum = [0.0f32; 3];
            let mut work = (0, 0);
            let t = Instant::now();
            for dl in &lists {
                if let Ok(f) = gpu.render(dl, &v, Output::Premul8) {
                    sum[0] += f.timings.prepare;
                    sum[1] += f.timings.bin;
                    sum[2] += f.timings.gpu;
                    work = (f.entries, f.pieces);
                }
            }
            let gpu_ms = t.elapsed().as_secs_f64() * 1000.0 / f64::from(runs);
            let n = runs as f32;
            let parity = match (gpu.render(&dl0, &v, Output::Premul8), render(&dl0, &v)) {
                (Ok(g), Ok((c, _))) => compare_premul8(&g.premul8, &c.to_premul_rgba8()).map_or("size mismatch".into(), |d| format!("{:.4} / {} / {}", d.mean, d.max, d.differing)),
                (Err(e), _) => format!("GPU: {e}"),
                (_, Err(e)) => format!("CPU: {e}"),
            };
            println!(
                "{:<18} {:>10} {:>9.2} {:>9.2} {:>30} {:>9} {:>9}  {parity}",
                name,
                format!("{}×{}", v.width, v.height),
                cpu_ms,
                gpu_ms,
                format!("{:.2} + {:.2} + {:.2}", sum[0] / n, sum[1] / n, sum[2] / n),
                work.0,
                work.1
            );
        }
    }
}
