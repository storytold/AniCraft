# ADR 0019: GPU stage renderer (first slice)

* Status: accepted (2026-11-09); extended by ADR 0029 (no readback, masks and groups on the GPU)
* Relates to: roadmap M3, ADR 0002 (render pipeline), ADR 0007 (tiled stage), AGENTS.md rule 4
  (the CPU renderer is the oracle)

## Context

The stage draws on the CPU (ADR 0002): 15 ms for a 1080p frame of 10k shapes on 32 threads,
about 18 ms for a simple scene at 4K. M3 asks for a wgpu renderer behind a feature flag that
draws the same display list, checked pixel for pixel against the CPU oracle, with the oracle
staying the default until parity is at most 1/255 mean error. egui already brings wgpu 30
(through `egui-wgpu`), so a second graphics stack is out of the question.

Parity is the hard requirement. The oracle computes **exact analytic area coverage** (signed
area accumulation, ADR 0002) in f32, with its own flattening, stroker and 256-entry gradient
tables. Any GPU approach that approximates coverage differently (MSAA, stencil-and-cover,
distance fields) disagrees with it at every anti-aliased edge.

## Options considered

| Option | Parity with the oracle | Work for a first slice | Verdict |
|---|---|---|---|
| Stencil-and-cover with 4×/8× MSAA (triangle fans into a stencil, then a covering quad) | Coverage quantized to 1/4 or 1/8: errors up to ±1/16 at every edge; fine mean, poor max | small | no: the max error never converges, and parity is the point |
| Full GPU pipeline as in Vello (flatten, bin, coarse and fine all in compute) | exact coverage in principle, but our flattening and stroker would be reimplemented in WGSL and drift from the oracle's | large (several shader stages, GPU prefix sums, dynamic memory) | later, stage by stage |
| **CPU prepares and bins, GPU computes coverage and composites** (the "hybrid" split of the public sparse-strips work) | **the oracle's own geometry and paints; the same coverage integral; only f32 rounding differs** | moderate | **chosen** |

## Decision

1. **Same input as the oracle.** `anicraft_render::plain_fills(dl, view)` returns the fills
   exactly as `render` prepares them: culled, flattened at the oracle's tolerance, strokes
   outlined by the oracle's stroker, shaders built (solid colours premultiplied, gradients as
   the oracle's 256-entry tables), pixel bounds clamped. The GPU path never re-derives
   geometry or colour.
2. **Tile binning on the CPU** (`anicraft-gpu::binning`, parallel per chunk of fills, then
   parallel per tile row). The surface is cut into 16 × 16 tiles. Edges are split at tile rows
   and columns; each piece lands in the tile it crosses. What a piece contributes to the tiles
   to its right is a winding change per pixel row (it lies wholly to their left), summed along
   the tile row into a **backdrop** of 16 values, as in the public sparse-strips and Vello
   designs. A fill's tile needs only its own pieces and its backdrop; tiles inside a shape
   carry a backdrop and no pieces; tiles outside get nothing. Each tile lists its entries in
   drawing order.
3. **One compute pass** (`fine.wgsl`): a workgroup per tile, an invocation per pixel. For each
   entry, coverage = backdrop + Σ over the tile's pieces of the exact signed area of the pixel
   to the right of the piece: the integral the oracle's accumulation buffer computes, written
   as the mean of `clamp(u, 0, 1)` over the piece so it has no cancellation. Then the oracle's
   fill rule fold, its `coverage > 1e-4` cut-off, the paint (gradient index rounded as the
   oracle's table lookup) and src-over in premultiplied f32. Output is premultiplied RGBA8
   rounded like `Pixmap::to_premul_rgba8` (the stage) or f32 (tests, exports later).
4. **What the plain path doesn't draw yet goes to the CPU, per frame (per tile).** Masks
   (clips), group opacity, blend modes, filters and bitmap fills make `plain_fills` answer
   `NeedsFull`; the render service then draws that tile with the oracle and records why.
   The picture is always complete and always the oracle's when the GPU steps back.
5. **A headless device of our own**, high-performance adapter first, compute shaders and
   downlevel limits required, with a startup **self-test** (a known 24 × 20 render must come
   back exactly) before it is trusted. Errors are captured in thread-local error scopes per
   render; uncaptured errors and device loss mark the renderer broken, and from then on the CPU
   draws. Every call is behind `catch_unwind`. Owning the device (rather than sharing egui's)
   keeps a GPU-stage failure from ever touching the window's device, works when the window
   itself fell back to OpenGL, and lets tests run without a window.
6. **Layering.** `crates/gpu` is **L3**, beside the engine: it builds on the oracle's
   preparation (L2), and the shell (L5) wires it in. The engine stays GPU-free: its
   `RenderService` takes any `Rasterizer` (`Fn(&DisplayList, &View) -> Result<RGBA8, why>`),
   and jobs are keyed by which rasterizer drew them, so switching renderers re-renders while
   the old tiles stand in, and switching back finds the CPU's pictures still cached.
7. **Behind a feature, off by default.** `cargo run --features gpu-stage` adds Preferences ›
   Stage › Renderer (and View › Stage Rendering): *CPU (reference)* or *GPU (preview)*. The
   status bar says who drew the visible tiles and why the CPU stepped in ("CPU for masks").
   `ANICRAFT_STAGE_RENDERER=gpu|cpu` overrides the preference (also in `--screenshot` runs).
   **Exports always use the CPU oracle.**
8. **Parity tests** (`crates/gpu/src/tests.rs`): the sample scene at five frames and zooms, a
   10k-shape stress frame, an offset zoomed 512 px stage tile, and analytic scenes (fractional
   rectangles across tiles, a circle, even-odd and non-zero rings, a self-intersecting star,
   rotated linear and radial gradients, wide and sub-pixel strokes, shapes far larger than the
   surface with a colour effect). Bounds: mean ≤ 1/16 of the roadmap's 1/255 target and max ≤
   2/255 on RGBA8; mean < 1e-4 and max < 4e-3 on f32. A second layer runs the shader's
   arithmetic on the CPU over the binned scene, so the binning is checked on every machine,
   GPU or not. Without an adapter the GPU tests print why and pass.

## Measured (2026-11-09, release, RTX 5090 over Vulkan, 32 threads)

`cargo run --release -p anicraft-gpu --example gpu_bench`. GPU ms is the whole call
(prepare + bin + upload, compute, readback). Parity is against the oracle on the same frame.

| Scene | Size | CPU oracle | GPU stage (prepare + bin + GPU) | Parity (mean / max) |
|---|---|---|---|---|
| Stress, 50 layers × 200 shapes (≈10k drawn) | 1920×1080 | 11.3 ms | 13.6–14.3 ms (3.1 + 5.0 + 4.6) | < 0.0001 / 1 (of 255) |
| Stress, 50 × 200 | 3840×2160 | 26.6–27.3 ms | 22.1–22.4 ms (2.8 + 8.0 + 8.4) | < 0.0001 / 1 |
| 1,000 shapes | 1920×1080 | 4.4–4.6 ms | 2.8–2.9 ms | < 0.0001 / 1 |
| 1,000 shapes | 3840×2160 | 17.4–17.8 ms | **8.8–9.0 ms** | < 0.0001 / 1 |
| Sample scene | 3840×2160 | 18.2 ms | 8.8 ms | < 0.0001 / 1 |

At most 0.06 % of pixels differ at all, each by one 8-bit step. The M3 target of 4K at 60 fps
for 1,000 shapes (16.7 ms) is met with room; the 10k-shape frame is CPU-bound in preparation
and binning. The GPU share at 4K is mostly the 33 MB readback.

## Consequences and next steps

* **Parity holds by construction** for everything the plain path draws: same geometry, same
  paints, same coverage integral. A renderer change on either side shows up in the tests.
* The GPU stays opt-in until the steps below make it the faster choice everywhere; then it
  becomes the stage default (the oracle stays the reference and the export path).
* Not in this slice, in order:
  1. **No readback**: render into a texture egui shows directly (share egui's device through
     `eframe`'s render state, or copy across devices), removing the largest fixed cost.
  2. **Masks, group opacity and blend modes** on the GPU: clip coverage and group layers as
     extra entry kinds in the same pass (the oracle's band algorithm maps onto tiles).
  3. **Binning on the GPU** (Vello-style coarse rasterization with prefix sums) for scenes of
     tens of thousands of shapes; buffer reuse across frames.
  4. Bitmaps (textures and the oracle's bilinear sampling), filters (compute blurs), and GPU
     exports once parity covers them.
* Shared-`target/` builds note: the GPU crate builds with the wgpu the shell already builds
  (same version, default features), so it adds no second wgpu.
