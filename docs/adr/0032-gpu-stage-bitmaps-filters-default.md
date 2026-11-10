# ADR 0032: GPU stage, third slice: bitmaps and filters on the GPU, and the GPU as the default stage

* Status: accepted (2026-11-09)
* Relates to: roadmap M3, ADR 0019 (GPU stage, first slice), ADR 0029 (no readback, masks and
  groups), ADR 0025 (filters), ADR 0010 (bitmaps), AGENTS.md rule 4 (the CPU renderer is the
  oracle)

## Context

After ADR 0029 the GPU stage drew everything except **bitmap fills** and **filtered symbols**,
which sent the whole stage tile to the CPU. Documents imported from Animate usually have both,
so they still drew on the CPU. ADR 0029 named three conditions for making the GPU the default:
bitmaps and filters on the GPU, breadth of hardware, then the default itself.

## Decision

### 1. One hand-off: the oracle prepares, and resolves, everything

`render::prepare_scene` no longer refuses filters or bitmaps.

* **Bitmap fills** reach the other renderer as the oracle's `Shader::Bitmap` (pixel → image
  matrix, the pixels, repeat, colour effect).
* **A filtered symbol** reaches it as a `FilterLayer`. The CPU and the GPU both get it from
  `filter_setup`:
  * its items, with the group made neutral and the clips from outside it removed;
  * the view of its buffer: the artwork grown by the filters' reach, within the surface's room;
  * its filters, *resolved*.

  A fill (`Fill::layer`) composites the layer in the group, clipped by the clip around the
  instance.
* **`filters::resolve`** turns a filter at a view scale into the exact steps the oracle runs:
  * box widths `round(blur × scale)` and the pass count;
  * whole-pixel offsets;
  * sanitized or premultiplied colours;
  * the ramp table of 257 premultiplied colours;
  * the colour matrix.

  `filters::apply` now runs those `FilterOp`s too, so the CPU and the GPU take every number
  from the same function. The CPU's arithmetic did not change, and the filter unit tests pass
  unchanged.

### 2. Bitmaps on the GPU

* **Uploaded once per image, cached under a budget.** Each image becomes a straight RGBA8
  texture (`images.rs`), kept until the cache passes 256 MB, least recently used out first.
  * **The key is the image, not the bitmap id.** It is the display list's `Arc<Pixels>`,
    compared by pointer, and the cache holds that `Arc` so the address can't be reused. Two open
    documents can share a bitmap id, and a replaced bitmap keeps its id. Keying by image means
    neither ever shows a stale texture, and every frame of one document hits the cache (the
    document decodes each bitmap once and shares it).
  * Textures never change once uploaded. Evicting one while a submitted frame still samples it
    is safe, because wgpu keeps it alive until then.
  * A failed upload (out of memory) is never cached; it fails that tile, and the CPU draws it.
* **Sampled by hand, exactly as the oracle samples.** The shader does the oracle's own bilinear
  filter: `textureLoad` of four taps, not hardware filtering, which rounds its weights to a few
  bits. It uses the same pixel-centre convention and the same edge clamp (`Pixels::at`). Repeats
  use Rust's `rem_euclid`, then an integer wrap of the taps. Each tap gets the colour effect
  (`ColorEffect::apply`), then `premultiply`'s clamps, in that order; a NaN the oracle would
  sanitize to 0 becomes a channel mask computed on the host.
* **16 texture slots per dispatch** (`MAX_IMAGES`), one per distinct image. Every wgpu limits
  preset has at least 16 sampled textures per shader stage. A stage tile (512 px) with more
  than 16 different images goes to the CPU (`NeedsFull::Bitmaps`), and so does an image larger
  than the device's textures.
  * Binding arrays would lift the limit, but they are an optional feature that eframe's device
    doesn't enable.
  * An atlas would also lift it, but it would need eviction discipline across tiles drawing in
    parallel.
  * Either is the next step if documents show the need.

### 3. Filters on the GPU

Each shown layer is its own job, drawn and filtered before the pass that shows it, in the same
command buffer:

1. The fine pass draws the layer's scene in premultiplied f32 into half 0 of a two-half buffer.
   That scene may have bitmaps and filtered layers of its own (nested symbols, up to 4 deep:
   `MAX_FILTER_DEPTH`; deeper goes to the CPU).
2. Each `FilterOp` runs as compute passes (`filters.wgsl`, `layers.rs`):
   * **Box passes:** one invocation per row or column, running the oracle's running sum in the
     oracle's order, including its even-width half weights at both ends. Passes ping-pong
     between the halves, or between a coverage slot and a spare one, and end where they began.
     A width of 1 copies.
   * **Shadow and glow:** the opacity (or, inside, the transparency) moved by `(dx, dy)` into a
     coverage slot, blurred there, then composed with the layer per pixel.
   * **Bevel and Gradient Bevel:** the bevel field from the blurred opacity, then colours or the
     ramp.
   * **Gradient Glow:** inner and outer in two slots.
   * **Adjust Color:** the colour matrix, its rows summed in the oracle's order.
3. The finished layer is copied into the paint buffer of the scene that shows it, after the
   gradient tables. Its fill reads it pixel for pixel, as `Shader::Image` does.

Limits per call:
* at most 8 M layer pixels (44 bytes each while the frame draws);
* every buffer within the device's binding size;
* the filter passes count against `MAX_WORK`.

Past these limits, the CPU draws the tile.

### 4. Hardware breadth

* **Direct3D 12 works now.** wgpu compiles HLSL with FXC unless DXC is installed, and FXC
  couldn't compile the fine shader of ADR 0019 and ADR 0029 at all: X3511, "forced to unroll
  loop", because `blend` looped over vector components. On any window running on DX12 the GPU
  stage never started, and the CPU drew instead. `blend` and `adjust` are vector code now. The
  whole parity suite passes on DX12 and on Vulkan (RTX 5090), with the same numbers.
* **Software adapters are not used for the stage.** WARP, llvmpipe and SwiftShader emulate a GPU
  on the CPU, so there the CPU oracle is faster.
  * `GpuRenderer::new` and `shared` refuse a `DeviceType::Cpu` adapter.
  * `GpuRenderer::software()` opens one for tests: the parity tests use it when there is no
    graphics card, so a CI runner with lavapipe (Mesa's Vulkan) runs them.
  * WARP loses its device on the fine pass (seen on Windows 10). It isn't investigated: the
    self-test catches it, and the stage never uses WARP.
* **The self-test compares with the oracle.** At start-up a 24 × 20 render must match the CPU
  within the parity target: two rectangles exactly, plus a repeating bitmap and a blurred,
  colour-adjusted symbol, read back and through a texture. If not, the GPU stage doesn't start,
  and Preferences shows which pixel was off and by how much.

### 5. The GPU becomes the default stage renderer

**Yes.** ADR 0029's conditions are met:
* Everything the stage draws is drawn on the GPU (within the limits above). The picture is the
  oracle's within 1/255.
* It is 3–9× faster than the CPU at 4K, including on the filtered and bitmap scenes.
* It runs on Vulkan and DX12.
* Every failure falls back to the CPU on its own: no adapter, a software adapter, a failed
  self-test, a device error, out of memory, or a tile past the limits.

What changes:
* **The app's `gpu-stage` feature is on by default** (`default = ["sound-output",
  "gpu-stage"]`). The engine crates don't depend on `anicraft-gpu`, so headless builds (CLI,
  MCP, exports) are unchanged. A CPU-only shell builds with `--no-default-features --features
  sound-output`.
* **The preference defaults to the GPU** (Preferences › Stage › Renderer: *Graphics card
  (GPU)* or *CPU (reference)*).
  * Every earlier build wrote `"stage_renderer": "Cpu"` into the preferences file without anyone
    choosing it: no released build had the choice, because the feature was off.
  * So the setting is now stored under a new key, `stage`, and the old key isn't read. Everyone
    starts on the GPU, and a later choice of the CPU is kept.
* **The CPU stays the oracle**, the export path (PNG, sequences, video, HTML, Lottie, SVG) and
  the fallback. The status bar says which renderer drew the stage, and why the CPU did when it
  did. `ANICRAFT_STAGE_RENDERER=cpu|gpu` still overrides.
* **CI's default build has no GPU.** The shell builds and tests with the feature on. The GPU
  tests try the software adapter, and skip, saying why, without one. The app on such a machine
  falls back to the CPU.

## Measured (2026-11-09, release, RTX 5090, 32 threads)

`cargo run --release -p anicraft-gpu --example gpu_bench`, columns as in ADR 0029:
* *Direct* is what the calling thread spends with `render_texture`.
* *Tiled* is the stage's way: 512 px tiles in parallel, until the GPU has finished.
* *CPU oracle* is what these frames cost before this slice: they always fell back to the CPU.

| Scene | Size | CPU oracle | Direct (Vulkan) | Tiled (Vulkan) | Direct (DX12) | Tiled (DX12) | Parity (mean / max, 1/255) |
|---|---|---|---|---|---|---|---|
| Filtered sample (drop shadow + bevel on the ball, glow on the star, a bitmap sky) | 1920×1080 | 9.8 ms | 1.6 ms | 3.7 ms | 1.5 ms | 5.4 ms | < 0.0001 / 1 |
| Filtered sample | 3840×2160 | 31.5 ms | **3.9 ms** | 9.5 ms | 4.2 ms | 14.0 ms | < 0.0001 / 1 |
| 200 bitmap fills (8 images, 256 × 256) | 1920×1080 | 10.9 ms | 1.5 ms | 2.0 ms | 1.2 ms | 2.9 ms | 0.0002 / 172 (one pixel, below) |
| 200 bitmap fills | 3840×2160 | 37.2 ms | **4.4 ms** | 5.7 ms | 4.3 ms | 8.7 ms | 0.0002 / 1 |
| 1,000 shapes (ADR 0029's scene, for comparison) | 3840×2160 | 17.1 ms | 2.0 ms | 4.7 ms | 2.9 ms | 8.7 ms | < 0.0001 / 1 |

* A filtered symbol costs each tile it touches a layer drawn and filtered with the filters'
  reach around the tile, so tiling costs more here than on plain scenes (9.5 vs 3.9 ms at 4K).
  Sharing one layer between the tiles of a frame is the next optimization.
* DX12 pays more per dispatch than Vulkan in the tiled stage. It is still 2–4× the CPU.
* **The one large difference:** a pixel centre within one float rounding of a non-repeating
  bitmap's own edge can land on the other side of it. GPU compilers may fuse `a·x + c·y + e`
  into a fused multiply-add, which WGSL can't forbid; the oracle never fuses. That pixel then
  shows the bitmap's edge texel or what is under it: 1 pixel of 2 M at 1080p in this scene.
  The mean stays far inside the target, and the parity tests' scenes are all byte-identical.

**Parity tests** (`crates/gpu/src/tests.rs`, each read back and through a texture):
* bitmaps placed, turned, mirrored, repeated, one pixel, malformed, with colour effects (a NaN
  tint included), masked and multiplied: all **byte-identical** to the oracle;
* 16 bitmaps in one picture, with the 17th going to the CPU, and the cache not growing on
  redraw;
* every filter with its sides and switches (16 variants) at 1:1, zoomed and panned past the
  surface: max 1/255, mean ≤ 0.0051/255;
* filters in a row, faded and multiplied; nested filtered symbols; masks outside and inside the
  symbol; a filtered bitmap: max 1/255, mean ≤ 0.0066/255;
* the sample dressed with filters and a bitmap sky, whole frames and a zoomed stage tile: max
  1/255, 2 pixels of 2 M differ;
* filters nested as deep as the GPU draws, with one deeper going to the CPU.

## Not in this slice

* Sharing a filtered layer between the stage tiles of a frame.
* More than 16 images per tile (an atlas, or binding arrays).
* A GPU runner in CI (the software adapter covers parity where a runner has one).
* Metal and integrated GPUs verified by hand: the paths are the same wgpu code, and the
  self-test guards them.
* GPU exports.
* GPU binning (ADR 0029).

## Consequences

* `prepare_scene` is the one complete hand-off: whatever `render` draws, another renderer gets
  in prepared form. `NeedsFull` now only means "past this renderer's limits".
* `filters::resolve` is the filters' single source of numbers. A new filter adds a `FilterOp`,
  its CPU steps, and its GPU passes, and the parity test that lists every filter grows by one.
* The default build compiles the GPU stage, which adds no new dependency (wgpu was already in
  the tree through eframe).
