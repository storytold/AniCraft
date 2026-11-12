# Performance

Rule: **measure first, then optimize, then record the number here.** Every optimization lands
with a benchmark delta and a test that it did not change output (the CPU renderer is the
oracle; its output must stay bit-identical).

## How to measure

```sh
cargo run --release -p anicraft-cli -- bench                      # 50 layers × 200 shapes × 240 frames
cargo run --release -p anicraft-cli -- bench --layers 200 --shapes 500 --frames 600
```

`bench` builds a deterministic stress document (`doc::sample::stress`) and reports file size,
save/load time, evaluate/render time per frame, zoomed render with culling, snapshot cost,
per-edit and per-undo time, history bytes per step and hit-test time. In the app, the status
bar shows render time, shapes drawn and culled, frame-cache size and hit rate, and history
memory.

## Budgets

The default `bench` scenario: 50 layers × 200 shapes × 240 frames, 200k element instances.

| Scenario | Budget | Measured 2026-11-06 (release, 32 threads) | Enforced by |
|---|---|---|---|
| Edit command on a 200k-instance document | < 1 ms | 0.013 ms ✅ | `bench --check` |
| Undo / redo | < 1 ms | 0.009 ms ✅ | `bench --check` |
| Undo memory per small edit | < 16 KB | 2.7 KB ✅ | test, every PR |
| Document snapshot | < 50 µs, shares everything | 0.04 µs ✅ | test (sharing) and `bench --check` |
| Evaluate one frame (200k instances) | < 4 ms | 1.7 ms ✅ | `bench --check` |
| Render 1080p, ~10k visible shapes | < 16.7 ms (60 fps) | 15.3–15.8 ms ✅ | `bench --check` |
| Render at 400% zoom | < 16.7 ms | 9.0–9.3 ms ✅ | `bench --check` |
| Shapes drawn at 400% zoom | ≤ 1/8 of the frame's shapes | 5.5% ✅ | test, every PR |
| Hit test | < 200 µs | 52 µs ✅ | `bench --check` |
| File size | < 400 bytes per element instance | 342 ✅ | test, every PR |
| Save / load 65 MB file | < 500 ms | 123 / 146 ms ✅ | `bench --check` |
| UI frame while rendering | never blocked | worker thread ✅ | design (ADR 0002) |
| Autosave: time on the editing thread | < 1 ms | 7–15 µs ✅ | `bench --check` |
| Autosave of the 65 MB file, in the background | < 2 s | 183–184 ms ✅ | `bench --check` |
| GPU stage, 1,000 shapes at 4K, stage tiles (M3 exit) | < 16.7 ms | 4.5–4.7 ms ✅ (2026-11-10, RTX 5090) | `gpu_bench --check` |
| GPU stage, 10k shapes at 4K, stage tiles | < 16.7 ms | 10.1–10.4 ms ✅ | `gpu_bench --check` |
| GPU stage, 50k shapes at 1080p, stage tiles | < 33.3 ms (30 fps) | 24.8–26.3 ms ✅ | `gpu_bench --check` |
| GPU stage, 50k shapes at 4K, one texture | < 33.3 ms | 22.5–23.8 ms ✅ | `gpu_bench --check` |
| GPU stage, onion ±5 outlines / tinted at 1080p, stage tiles | ≤ 1.25 | 1.03–1.06 ✅ | `gpu_bench --check` |

**1080p render, back under budget (2026-11-06).** It had drifted to 19.4–20.2 ms (v0.1.0
measured 18.0–18.9 ms on the same machine; the 16.5 ms of 2026-10-28 didn't reproduce).
Timing each step of a frame showed the time was outside the drawing itself:

| Change | 1080p | 400% |
|---|---|---|
| Before | 19.4–20.2 ms | 11.6–11.9 ms |
| Background filled per band, in parallel, instead of one serial 33 MB fill | 16.9–17.5 ms | 9.1–9.6 ms |
| Edges wholly above or below a band skipped before splitting; fill polygons moved, not copied | 17.0–17.2 ms | 8.9–9.1 ms |
| Prepared polygons freed on the pool after the frame, off the critical path | 15.3–15.8 ms | 9.0–9.3 ms |

Output is unchanged: the sample's frames and a bitmap document render byte-identical before
and after.

**Onion skin ±5 at 1080p (2026-11-10, ADR 0047).** Through the stage's tiled path (40 tiles
of 256 px, one scene evaluation per frame, tiles in parallel), release build, 32 threads, best
of frames 12, 30 and 48:
`cargo test --release -p anicraft-engine onion_timings -- --ignored --nocapture`.

| Scene | Off | ±5 tinted, fading | ±5 outlines | Keyframes only ±5 | Edit Multiple Frames ±5 |
|---|---|---|---|---|---|
| Sample (7 items a frame; 67 with ±5) | 5.9–6.1 ms | 6.0–6.5 ms | 6.5–7.0 ms | 5.8–6.2 ms | 6.0–6.1 ms |
| Stress 20 × 100 (2,000 items a frame; 22,000 with ±5) | 7.2–7.9 ms | 16.2–17.2 ms | 34–39 ms | 7.5–7.6 ms | 6.6–7.3 ms |

Composing the stress scene with ±5 (ten extra evaluations, then each layer's onion frames
merged under its own artwork) took 8.3 ms at first: 3.9 ms of it was putting the items in layer
order (an append, then a reorder). One k-way merge by layer, each item moved once, brought that
to 0.85 ms, and evaluating the extra frames in parallel brought the whole to 3.0–3.3 ms. Output
is unchanged (the onion tests compare items and pixels). The settings are part of every tile's
key, so the frame cache keeps working: looking at the same frame again is 40 hits in 0.02 ms,
and other settings are other pictures.

**Outlines as hairlines (2026-11-10, ADR 0048).** Outline mode drew half-pixel strokes (four
edges per segment and a disc at every bend); it now draws hairlines, one edge per segment,
covered by distance. Same test, same machine:

| Scene | Off | ±5 tinted, fading | ±5 outlines | Keyframes only ±5 | Edit Multiple Frames ±5 |
|---|---|---|---|---|---|
| Sample | 5.7 ms | 5.9 ms | 5.5 ms | 5.9 ms | 5.7 ms |
| Stress 20 × 100 | 7.6 ms | 14.8 ms | **17.1 ms** (was 34–39) | 7.3 ms | 7.2 ms |

**Crash-recovery autosave (2026-11-09, ADR 0015).** `bench` now runs one autosave the way the
app does: an `Autosaver` in a scratch recovery folder. The editing thread only hands over an
`Arc` snapshot of the document; the worker serializes and writes atomically (temp file,
flushed to disk, renamed).

| Document | Editing thread | Serialize | Write + flush + rename | Total, in the background |
|---|---|---|---|---|
| 200k instances, 65 MB | 6.6–15.4 µs | 124–128 ms | 52–55 ms | 183–184 ms |
| 800k instances (100 × 400 × 240), 262 MB | 9.9 µs | 501 ms | 210 ms | 725 ms |

With the default "every minute or 25 changes", even the 262 MB document keeps the worker busy
for about 1% of the time, and the UI never waits for it.

**Opening a 50 MB `.fla` (2026-11-11, ADR 0054; release, 32 threads, one used).** A generated
archive (`measure_importing_a_50_mb_fla` in `crates/xfl/tests/fla.rs`: 400 symbols of 600
edges each, deflated, and 24 noisy 1024 × 512 PNGs, stored; 54.6 MB, 425 files, 65.8 MB
inflated), five runs each:

```sh
cargo test --release -p anicraft-xfl --test fla -- --ignored --nocapture
```

| Step | CRC-32 byte by byte | CRC-32 by slicing by 8 |
|---|---|---|
| Open (end record, central directory, every local header, overlap check) | 0.19–0.25 ms | 0.18–0.24 ms |
| Read and check every entry (inflate, size, CRC) | 128–140 ms (471–516 MB/s) | 53–54 ms (1.22–1.24 GB/s) |
| Whole import (read, XML, edges, library, timeline) | 388–458 ms | **311–328 ms** |

The checks on every entry (exact size, checksum) now cost about 30 ms of the import; the rest
is parsing and building the document.

### GPU stage (M3, ADR 0019)

`cargo run --release -p anicraft-gpu --example gpu_bench` (2026-11-09, RTX 5090 over Vulkan,
32 threads). GPU ms is the whole call: preparing the fills (shared with the oracle), binning,
upload, the compute pass and the readback. Parity is mean / max difference from the oracle per
8-bit channel.

| Scene | Size | CPU oracle | GPU stage (prepare + bin + GPU) | Parity |
|---|---|---|---|---|
| Stress, 50 × 200 (≈10k drawn) | 1920×1080 | 11.3 ms | 13.6–14.3 ms (3.1 + 5.0 + 4.6) | < 0.0001 / 1 |
| Stress, 50 × 200 | 3840×2160 | 26.6–27.3 ms | 22.1–22.4 ms (2.8 + 8.0 + 8.4) | < 0.0001 / 1 |
| 1,000 shapes | 1920×1080 | 4.4–4.6 ms | 2.8–2.9 ms | < 0.0001 / 1 |
| 1,000 shapes (M3 target: 4K at 60 fps) | 3840×2160 | 17.4–17.8 ms | 8.8–9.0 ms ✅ | < 0.0001 / 1 |
| Sample scene | 3840×2160 | 18.2 ms | 8.8 ms | < 0.0001 / 1 |

Binning went from 16 ms to 5 ms on the stress frame by binning fills in chunks with reused
scratch (no allocation per edge piece), freeing the per-chunk buffers on the pool, and
scattering the tile lists in parallel per tile row. What is left: CPU binning on very large
scenes (next: GPU binning) and the 33 MB readback at 4K (next: render into the texture egui
shows).

### GPU stage, second slice: no readback, masks and groups (M3, ADR 0029)

Same bench and machine. *Read-back* is the call above; *direct* is the calling thread's time
with `render_texture` (prepare + bin + upload and submit, the window's path); *tiled* is the
stage's way, 512 px tiles drawn in parallel, until the GPU has finished.

| Scene | Size | CPU oracle | Read-back | Direct | Tiled (stage) | Parity |
|---|---|---|---|---|---|---|
| Stress, 50 × 200 (≈10k shapes) | 1920×1080 | 10.6 ms | 13.4 ms | 9.4–10.4 ms (3.0 + 4.7 + 1.7) | 8.7 ms | < 0.0001 / 1 |
| Stress, 50 × 200 | 3840×2160 | 25.8–28.1 ms | 23.2 ms | 16.8–18.7 ms (2.7 + 7.5 + 4.8) | **10.2 ms** ✅ 60 fps | < 0.0001 / 1 |
| 1,000 shapes | 1920×1080 | 4.4–5.0 ms | 2.9–3.0 ms | 1.1–1.2 ms | 1.8 ms | < 0.0001 / 1 |
| 1,000 shapes (M3 target: 4K at 60 fps) | 3840×2160 | 16.2–19.6 ms | 8.4–8.5 ms | **2.1–2.4 ms** ✅ | 4.8 ms | < 0.0001 / 1 |
| Masked stress (every other layer a mask) | 1920×1080 | 25.4–26.2 ms | 10.7 ms | 8.7–9.4 ms | 6.6 ms | < 0.0001 / 1 |
| Masked stress | 3840×2160 | 74.8–80.0 ms | 20.5–21.8 ms | 14.6–15.3 ms | 10.9 ms | < 0.0001 / 1 |
| Sample with a mask and faded symbols | 3840×2160 | 19.1–19.9 ms | 8.3–8.9 ms | 2.1–2.6 ms | 4.6 ms | < 0.0001 / 1 |

Masked scenes are where the GPU gains most (7× at 4K): the oracle computes each clip's
coverage per band into a buffer, the GPU per pixel from the artwork's tile entries. Binning is
the largest term of a whole 10k-shape frame, but tiles spread it over every core; it stays on
the CPU until scenes of 50k+ shapes or 8K stages need more (ADR 0029). Small scenes pay a
little per-tile set-up in the tiled stage (1.8 vs 1.1 ms at 1080p); reusing buffers across
tiles is next.

### GPU stage, third slice: bitmaps and filters, the default stage (M3, ADR 0032)

Same bench and machine, on Vulkan and on Direct3D 12. The CPU column is also what these
frames cost the stage before this slice, when bitmaps and filters always fell back to the CPU.

| Scene | Size | CPU oracle | Direct (Vulkan) | Tiled (Vulkan) | Direct (DX12) | Tiled (DX12) | Parity |
|---|---|---|---|---|---|---|---|
| Filtered sample (drop shadow + bevel, glow, a bitmap sky) | 1920×1080 | 9.8 ms | 1.6 ms | 3.7 ms | 1.5 ms | 5.4 ms | < 0.0001 / 1 |
| Filtered sample | 3840×2160 | 31.5 ms | **3.9 ms** | 9.5 ms ✅ 60 fps | 4.2 ms | 14.0 ms | < 0.0001 / 1 |
| 200 bitmap fills (8 images) | 1920×1080 | 10.9 ms | 1.5 ms | 2.0 ms | 1.2 ms | 2.9 ms | 0.0002 / 172 (1 px) |
| 200 bitmap fills | 3840×2160 | 37.2 ms | **4.4 ms** | 5.7 ms | 4.3 ms | 8.7 ms | 0.0002 / 1 |
| 1,000 shapes | 3840×2160 | 17.1 ms | 2.0 ms | 4.7 ms | 2.9 ms | 8.7 ms | < 0.0001 / 1 |

Each stage tile a filtered symbol touches draws and filters its own copy of the layer (with
the filters' reach around the tile), so tiling costs more than on plain scenes; sharing the
layer across a frame's tiles is next. The one large difference is a single pixel whose centre
falls within float rounding of a non-repeating bitmap's edge (ADR 0032). Bitmaps are uploaded
once per image and cached (256 MB budget), so after the first frame they cost no transfer.

### GPU stage, fourth slice: binning on the GPU, hairlines (M3, ADR 0048)

`cargo run --release -p anicraft-gpu --example gpu_bench` (2026-11-10, RTX 5090 over Vulkan,
32 threads, a machine shared with other builds: best of three runs). *Direct* is the calling
thread's time with `render_texture`, *tiled* the stage's way (512 px tiles in parallel until
the GPU has finished).

| Scene | Size | CPU oracle | Before: direct / tiled | After, binned on the CPU: direct / tiled | After, Auto: direct / tiled | Parity (mean / max, 1/255) |
|---|---|---|---|---|---|---|
| Stress, 50 × 200 (≈10k shapes) | 1920×1080 | 9.8 ms | 9.0 / 7.4 ms | 8.6 / 5.5 ms | **3.9** / 5.5 ms | < 0.0001 / 1 |
| Stress, 50 × 200 | 3840×2160 | 25.9 ms | 21.7 / 10.9 ms | 16.4 / 10.1 ms | **4.7** / 10.1 ms | < 0.0001 / 1 |
| 1,000 shapes (binned on the CPU: few edges) | 3840×2160 | 16.9 ms | 2.2 / 4.5 ms | 1.6 / 4.2 ms | 1.7 / 4.5 ms | < 0.0001 / 1 |
| 50,000 shapes (250 × 200) | 1920×1080 | 31.7 ms | 52.2 / 32.4 ms | 33.5 / 26.8 ms | **17.7** / 24.8 ms | < 0.0001 / 1 |
| 50,000 shapes | 3840×2160 | 73.1 ms | 122.1 / 75.3 ms | 103.9 / 61.5 ms | **22.5** / **36.7** ms | < 0.0001 / 1 |
| Masked stress (binned on the CPU: masks) | 3840×2160 | 72.4 ms | 13.8 / 12.3 ms | 12.0 / 10.7 ms | 11.3 / 10.5 ms | < 0.0001 / 1 |
| Onion skin off (stress 20 × 100) | 1920×1080 | 5.0 ms | 2.1 / 2.7 ms | 1.8 / 2.2 ms | 1.1 / 2.3 ms | < 0.0001 / 1 |
| Onion ±5, tinted (22,000 items) | 1920×1080 | 15.1 ms | 15.7 / 12.6 ms | 12.8 / 10.5 ms | 7.1 / 9.8 ms | < 0.0001 / 1 |
| Onion ±5, tinted | 3840×2160 | 37.3 ms | 32.1 / 25.1 ms | 20.3 / 22.0 ms | 8.5 / 16.5 ms | 0.0001 / 1 |
| Onion ±5, **outlines** | 1920×1080 | 20.0 ms (was 36.0) | 74.7 / 73.0 ms | 12.9 / 10.4 ms | 7.3 / **10.1** ms | < 0.0001 / 1 |
| Onion ±5, **outlines** | 3840×2160 | 37.9 ms (was 60.2) | 135.4 / 95.4 ms | 20.3 / 18.1 ms | 8.6 / **15.3** ms | < 0.0001 / 1 |

*Before* is `main` before this slice (CPU binning with float backdrops, stroked hairlines),
measured the same day on the same machine. In stage tiles the GPU binning wins once frames are
heavy (50k shapes: 61.5 → 36.7 ms at 4K) and never loses below `AUTO_MIN_EDGES`; a whole frame
in one texture gains most (10k shapes at 4K: 16.4 → 4.7 ms). **Outline mode is as fast as
tinted onion skin** on the GPU stage: 10.1 vs 9.8 ms at 1080p in tiles, 15.3 vs 16.5 ms at 4K,
7.3 vs 7.1 ms in one texture (it was 73 vs 12.6 ms in tiles).

On **Direct3D 12** (same card, `WGPU_BACKEND=dx12`, two runs) the GPU binning still wins in
one texture (10k shapes at 4K: 17.5 → 6.3 ms; 50k: 154 → 30 ms) but loses in stage tiles
(onion ±5 at 1080p: 9.7 → 14.4 ms tinted, 10.0 → 15.4 ms outlines; the passes cost more per
dispatch there, as ADR 0032 saw for the fine pass), so `Binning::Auto` bins on the GPU on
Vulkan only. With CPU binning, DX12's onion outlines take 10.1 ms in tiles against 10.0 tinted.

The CPU stage (the oracle through the render service's 256 px tiles, `onion_timings`, best of
frames 12, 30 and 48) with hairlines: onion ±5 outlines **17.1 ms** against 14.8 tinted at
1080p (was 34–39 against 16.2–17.2).

### How budgets are enforced

* **Budgets that don't depend on the machine** (undo memory, snapshot sharing, file size,
  culling) are a test (`machine_independent_budgets` in `crates/cli`), so the CI gate fails
  any PR that breaks them.
* **Timing budgets** are `cargo run --release -p anicraft-cli -- bench --check yes`. It
  prints every measurement next to its budget and fails when one is missed. Shared CI
  runners vary too much in speed and core count to gate on time, so this runs on a known
  machine before a release and after performance work. A dedicated benchmark runner would
  let CI run it too.
* The limits live in `TIME_BUDGETS` in `crates/cli/src/lib.rs`; keep that list and this
  table in step. The GPU stage's are `BUDGETS` in `crates/gpu/examples/gpu_bench.rs`
  (`--check`; the CLI has no GPU).

## Animated PNG and WebP (2026-11-11, ADR 0063)

The sample, all 48 frames, release build, Windows 10, 32 threads
(`cargo test --release -p anicraft-engine anim::tests::sizes -- --ignored --nocapture`).
Times include rendering every frame.

| Export | 1280 × 720 | Time | 640 × 360 | Time |
|---|---|---|---|---|
| Animated GIF (256 colours, 1-bit alpha) | 1464 KB | 421 ms | 660 KB | 110 ms |
| Animated PNG | 2019 KB | 183 ms | 741 KB | 52 ms |
| Animated WebP, lossless | 1829 KB | 130 ms | 666 KB | 36 ms |

The sample moves something in every frame, so all 48 frames are stored. Its sky is a full-stage
shape, so the transparent and stage-colour exports come out the same size. Compressing each
APNG frame at the `png` crate's best level instead of its default brought the file from 3387 KB
to 2019 KB for 25 % more time. The frames compress in parallel, which is why both formats are
faster than GIF, whose LZW coding runs on one thread.

## Sound (2026-11-09, ADR 0020)

Release build, Windows 10, one 120-second 48 kHz stereo sine, through `anicraft-cli run`
(times include starting the CLI and loading the sample):

| Step | Time |
|---|---|
| Import + decode WAV (23 MB) | 129 ms |
| Import + decode Ogg Vorbis (420 KB) | 147 ms |
| Import + decode MP3 (2.9 MB, 192 kb/s) | 99 ms |
| `file.export_audio`, 120 s at 48 kHz (exact copy path) | 95 ms |
| `file.export_audio`, 120 s at 44.1 kHz (Catmull-Rom resampling) | 128 ms |

Mixing runs about 1,000× faster than real time, so the audio callback's share of a 10 ms device
buffer is tiny; the stage never waits on sound.

## Decisions

* [ADR 0001](adr/0001-undo-persistent-snapshots.md): persistent document and snapshot undo
* [ADR 0002](adr/0002-render-pipeline.md): render worker, frame cache, band parallelism, culling
* [ADR 0003](adr/0003-file-format-v2.md): compact file format with shared paths
* [ADR 0015](adr/0015-crash-recovery-autosave.md): crash-recovery autosave on a worker thread
* [ADR 0019](adr/0019-gpu-stage.md): GPU stage renderer (first slice), parity with the oracle
* [ADR 0029](adr/0029-gpu-stage-direct-masks-groups.md): GPU stage without readback, masks and
  groups on the GPU, binning kept on the CPU
* [ADR 0032](adr/0032-gpu-stage-bitmaps-filters-default.md): bitmaps and filters on the GPU,
  Direct3D 12, and the GPU as the default stage renderer
* [ADR 0048](adr/0048-gpu-binning-and-hairlines.md): tile binning on the GPU, one tile format
  for both binnings, and hairlines for outline mode
