# ADR 0029: GPU stage, second slice: no readback, masks and groups on the GPU

* Status: accepted (2026-11-09)
* Relates to: roadmap M3, ADR 0019 (GPU stage, first slice), ADR 0007 (tiled stage), ADR 0010
  (mask layers), AGENTS.md rule 4 (the CPU renderer is the oracle)

## Context

The first slice (ADR 0019) drew plain fills on a headless wgpu device of its own and read
every stage tile back to the CPU, where the shell uploaded it to egui again. Masks, group
opacity, blend modes, filters and bitmaps sent the whole tile to the CPU. Measured: the 33 MB
readback was most of the GPU's share at 4K, and any document with a mask layer or a faded
symbol (most real ones) never reached the GPU at all.

The roadmap's M3 exit criteria: parity ≤ 1/255 mean error, 4K at 60 fps for 1,000 shapes,
then the GPU as the default stage.

## Decision

### 1. Draw into the window's textures (no readback)

* **Share eframe's device.** `App::new` hands `CreationContext::wgpu_render_state` to the GPU
  stage; `GpuRenderer::shared(adapter, device, queue)` builds the pipelines on the window's
  device and checks it with the startup self-test (a known render read back, and through a
  texture). `render_texture` draws each stage tile into a fresh `Rgba8Unorm` storage texture
  (a second entry point, `main_texture`, writes exactly `k / 255`, so the texture holds the
  same bytes as `Pixmap::to_premul_rgba8`), submits, and returns without waiting: egui's own
  submission comes later on the same queue, so the window always samples a finished tile.
* **The engine stays GPU-free.** A `Rasterizer` returns `Raster::Premul8(bytes)` or
  `Raster::Native { image, width, height }`, where `image` is an opaque
  `Arc<dyn Any + Send + Sync>`; the render service caches either (GPU pictures under their own
  384 MB budget, counted at 4 bytes a pixel) and only the shell, which installed the
  rasterizer, downcasts it to `anicraft_gpu::GpuTexture` and registers it with
  `egui_wgpu::Renderer::register_native_texture`. Evicting a stage tile frees the registration.
* **Fallbacks, in order.** The window's device (direct) → a headless device of our own with
  readback (when the window runs on OpenGL, or its device lacks compute, storage textures or
  the limits) → the CPU oracle (no usable GPU, a device error, or content the GPU doesn't
  draw). Preferences › Stage says which ("straight into the window" or "copied to the window").

**The trade against ADR 0019's isolation.** ADR 0019 owned its device so that a GPU-stage
failure could never touch the window. Sharing gives that up in part, deliberately:

| Risk on the shared device | What we do |
|---|---|
| A validation or internal error reaching the window's uncaptured-error handler (wgpu's default panics) | Every call the renderer makes is inside its own **thread-local** error scopes (validation, out-of-memory, internal) and `catch_unwind`; we install nothing on the window's device. A device error stops the GPU stage for the session (the CPU draws, Preferences says why); out of memory only fails that tile. |
| A long dispatch tripping the driver's watchdog (a TDR resets the whole adapter) | A per-dispatch work bound (`MAX_WORK`, pieces plus entries over tiles, about 30× the 10k-shape 4K frame); beyond it the CPU draws. A TDR resets every device on the adapter anyway, so a private device never protected against this. |
| Running out of graphics memory | The GPU picture cache has its own budget; the stage keeps at most 192 tile textures. |
| Device loss | The window's device lost means the window is gone too; the headless fallback keeps its device-lost callback. |

What isolation bought in ADR 0019 (working when the window is on OpenGL, tests without a
window) is kept by the headless fallback, which is the first slice's path unchanged.

### 2. Masks and groups on the GPU

* **Same input as the oracle.** `render::prepare_scene` (replacing `plain_fills`) returns the
  fills as `render` prepares them, now with each fill's clip and group, the clips (artwork as
  solid fills, cleaned parent links, bounds; clipped fills already cut to their clip's bounds
  and culled, as `render` does) and the groups (chains outermost first, alpha sanitized, blend
  mode). Only filters and bitmaps answer `NeedsFull`.
* **Control entries in the tile streams.** The binning emits, per tile and in drawing order:
  before a clipped fill whose clip isn't the one computed last, `MASK_RESET` and per level of
  the clip chain `LEVEL_BEGIN`, the level's artwork entries in this tile, `LEVEL_END`; before
  a fill, `CLOSE` for each open group not in its chain and `OPEN` for each new one; `CLOSE`
  for the rest at the end. A clipped fill whose clip has a level with no artwork in the tile
  is left out there (its coverage would be 0).
* **The shader mirrors the band compositor per pixel.** Artwork coverage unites into the
  level (`a + k − ak`), levels multiply into the clip coverage, a clipped fill's coverage is
  multiplied by it before the oracle's `> 1e-4` cut-off. `OPEN` pushes the colour on a
  per-pixel stack (8 deep) and continues on transparent; `CLOSE` composites the group into the
  colour below exactly as `close_group` does: src-over at the group's alpha for Normal and
  Layer, otherwise the oracle's `blend` (the W3C separable formulas, plus Alpha and Erase).
  **All 14 blend modes are on the GPU**; none falls back.
* **Limits.** Groups nested deeper than 8 or clip chains longer than 16 go to the CPU (as
  `NeedsFull::Groups` / `NeedsFull::Clips`). The oracle decides group runs per 32-row band
  and the GPU per 16 × 16 tile; they agree whenever a group's items are contiguous in the
  display list, which is how `evaluate` emits every instance (an interleaved list, which
  nothing produces, could split a group differently on the two).

### 3. Binning stays on the CPU, for now

Measured (below), binning is the largest single term of a whole 10k-shape frame (4.7 ms at
1080p, 7.5–9.6 ms at 4K), but the stage never draws whole frames: it draws 512 px tiles in
parallel, which spreads preparation and binning over every core, and at 4K the 10k-shape
frame already takes 10.2 ms that way, inside a 60 fps frame. A GPU binner (Vello-style coarse
rasterization with prefix sums) only pays off together with flattening and stroking on the GPU
(preparation is the other 3 ms), and those would have to match the oracle's flattening and
stroker to keep parity. Not yet; revisit at 50k+ shapes or when 8K stages matter.

## Measured (2026-11-09, release, RTX 5090 over Vulkan, 32 threads)

`cargo run --release -p anicraft-gpu --example gpu_bench`. *Read-back* is the first slice's
whole call; *direct* is what the calling thread spends with `render_texture` (prepare + bin +
upload and submit); *tiled* is the stage's way, 512 px tiles in parallel, until the GPU has
finished. Parity is the direct path's texture against the oracle.

| Scene | Size | CPU oracle | Read-back | Direct | Tiled (stage) | Parity (mean / max, 1/255) |
|---|---|---|---|---|---|---|
| Stress, 50 × 200 (≈10k shapes) | 1920×1080 | 10.6 ms | 13.4 ms | 9.4–10.4 ms | 8.7 ms | < 0.0001 / 1 |
| Stress, 50 × 200 | 3840×2160 | 25.8–28.1 ms | 23.2 ms | 16.8–18.7 ms | **10.2 ms** | < 0.0001 / 1 |
| 1,000 shapes | 1920×1080 | 4.4–5.0 ms | 2.9–3.0 ms | 1.1–1.2 ms | 1.8 ms | < 0.0001 / 1 |
| 1,000 shapes | 3840×2160 | 16.2–19.6 ms | 8.4–8.5 ms | **2.1–2.4 ms** | 4.8 ms | < 0.0001 / 1 |
| Masked stress (half the shapes masks) | 1920×1080 | 25.4–26.2 ms | 10.7 ms | 8.7–9.4 ms | 6.6 ms | < 0.0001 / 1 |
| Masked stress | 3840×2160 | 74.8–80.0 ms | 20.5–21.8 ms | 14.6–15.3 ms | 10.9 ms | < 0.0001 / 1 |
| Sample with masks and faded symbols | 3840×2160 | 19.1–19.9 ms | 8.3–8.9 ms | 2.1–2.6 ms | 4.6 ms | < 0.0001 / 1 |

Parity tests (`crates/gpu/src/tests.rs`, every scene read back and through a texture, both
byte-identical): masks, nested masks with union and even-odd artwork, group alpha (nested,
across tiles and bands), all 14 blend modes at full and part alpha over a translucent
backdrop, masks inside groups and groups inside masks, malformed links, and the sample built
the way artists do it (a mask layer, faded and multiplied symbols): max 1/255, mean ≤ 0.0011/255;
six of the ten new scenes and frames are byte-identical to the oracle.

## Recommendation: GPU as the default stage

**Not yet; next.** Parity (max 1/255) and speed (4K at 60 fps for 1,000 and for 10,000
shapes, 2–8× the CPU at 4K, 7× on masked scenes) now meet the exit criteria. What is still
missing before it should be the default:

1. **Bitmaps and filters** on the GPU (textures with the oracle's bilinear sampling; compute
   blurs): today any frame with a bitmap fill or a filtered symbol draws on the CPU, and
   imported Animate documents often have both.
2. **Breadth of hardware**: verified on one NVIDIA card over Vulkan. It needs DX12, Metal and
   an integrated GPU, and a GPU runner (or a software adapter) in CI, since CI skips the GPU
   tests today.
3. Then flip the preference's default (keeping the `gpu-stage` build feature until it is
   compiled in by default) and keep the oracle for exports and as the reference.

## Consequences

* `render::plain_fills` is gone; `prepare_scene` is the one hand-off to other renderers.
* `Raster` and `NativeImage` in the render service: a future renderer (or a GPU export) plugs
  in the same way.
* Smaller scenes lose a little in the tiled stage against one whole-frame call (per-tile
  set-up: 1.8 vs 1.1 ms at 1080p); buffer reuse across tiles is the next optimization.
