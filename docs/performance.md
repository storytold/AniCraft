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
  table in step.

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
