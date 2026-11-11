# ADR 0048: GPU stage, fourth slice: tile binning on the GPU, and hairlines

* Status: accepted (2026-11-10)
* Relates to: roadmap M3, ADR 0019 (GPU stage), ADR 0029 (no readback, masks and groups; binning
  kept on the CPU), ADR 0032 (bitmaps, filters, the default), ADR 0047 (onion skin outlines),
  ADR 0002 (the oracle's coverage), AGENTS.md rule 4 (the CPU renderer is the oracle)

## Context

Two costs were left in the GPU stage:

* **Binning on the CPU.** The CPU split every prepared edge into 16 × 16 tile pieces, summed
  16 floats of backdrop per tile, and uploaded it all. ADR 0029 kept it there because stage tiles
  spread it over every core; it was still the largest term of a heavy frame (10k shapes: 4.0 ms
  of 8.8 at 1080p, 10.6 of 16.7 at 4K; 50k shapes: 17–76 ms) and grew with every shape.
* **Outlines.** Outline mode (an outlined layer, and onion skin's Outlines, ADR 0047) drew each
  item as a stroke of half a pixel: four edges per segment and a hexagon at every bend, all
  overlapping, so an outlined item had 5–10× the edges of its fill. On the 2,000-item stress
  scene with ±5 onion frames, outlines took 34–39 ms on the CPU stage against 16–17 tinted, and
  73 ms against 12.6 on the GPU stage.

## Decision

### 1. One tile format the GPU can compute: integer backdrops and left-edge steps

The per-row float backdrop (the winding of everything left of a tile, per pixel row) needed a
float sum in a fixed order; on the GPU that means float atomics (which WGSL doesn't have) or
serial work. The binning now describes the same winding as **Vello's tiles do** (its public
design notes: a backdrop and "y-edges"):

* **Backdrop:** one integer per tile, the winding number just below its top-left corner. It is
  the sum of the directions of the pieces further left in the tile row that start on the row's
  top edge (exactly the edges crossing it).
* **Left-edge steps:** where the outline passes from one tile column to another inside a row
  (consecutive pieces in different columns), each tile boundary between them gets one more
  piece, a vertical line on that tile's left edge from the crossing down to the row's bottom:
  upwards where the outline runs right (the winding left of the tile drops by one), downwards
  where it runs left. The fine shader treats it like any piece, so its coverage formula is
  unchanged.

Both come **from the split pieces themselves**, so they can never disagree. A first version
computed the crossings geometrically, apart from the split; near a tile corner a rounding
could count one crossing twice, which a stress frame showed (543 pixels off by up to 133/255).
A test on stress frames at awkward scales now catches it.

Integers add up the same in any order, the CPU binning (the reference) and the GPU produce the
same lists, and the CPU's own binning got smaller (no 64-byte backdrops).

### 2. Binning on the GPU

The CPU lays out only what is proportional to the fills and their points (`gpu_binning.rs`):

* the prepared polygons' points, one flat buffer, and each fill's tile range;
* one GPU invocation per **row of tiles of each fill** (an op-row), with room for each tile of
  each fill's bounds (an op-tile);
* per tile row, the fills overlapping it in drawing order (from their bounds), and per tile
  room for every fill whose bounds cover it (a 2D running sum over the bounds' corners);
* an **upper bound of the pieces** per edge (its rows, twice its columns, slack), so every
  buffer is sized before the GPU runs and **nothing is read back**.

`bin.wgsl` then runs, in one compute pass:

1. `count`: each op-row walks its fill's edges, splits the ones in its row (the CPU's
   `split_edge`/`split_row`, line for line), adds the left-edge steps, and counts pieces per
   op-tile; the backdrop deltas are summed along the row. An op-row belongs to one invocation,
   so **no atomics**: counts and backdrops are exact.
2. `scan_blocks`/`scan_add` (`scan.wgsl`): an exclusive prefix sum of the counts (the public
   work-efficient scan, Blelloch 1990: per workgroup, then the workgroups' totals, added back).
3. `scatter`: the same walk, writing each piece at its op-tile's place, in walk order (the
   same every frame).
4. `tiles`: one invocation per tile walks its row's fills in drawing order and keeps those
   with pieces or a backdrop there, as the entries the fine pass reads.

This is the count, prefix-sum and scatter structure of Vello's and Pathfinder's binning stages
(public descriptions; no code or shaders copied), arranged per op-row instead of per segment
so that it needs no atomics and is deterministic. The binding layout uses four storage buffers
per stage, the smallest any device offers.

**Scope.** Fills inside a mask or a group still bin on the CPU: their tile streams carry clip
and group control entries whose number depends on which fills reach each tile, which needs
either a CPU pass over the candidates or a GPU stream pass with counted room. That is the next
step. The CPU binning stays the reference and this fallback.

**When.** `Binning::Auto` (the default) bins on the GPU when the scene can take it, has at
least 32,768 edges and the device runs Vulkan (below, Direct3D 12 measured slower in stage
tiles; other backends aren't measured yet). Below that many edges the CPU bins a stage tile faster than the GPU's passes set up
(measured: forcing the GPU on every 512 px tile made the 1,000-shape 4K stage 1.9× slower, and
the 10k-shape one 1.3× slower). `ANICRAFT_GPU_BINNING=cpu|gpu|auto` overrides it for
comparisons; `render_with`/`render_texture_with` take it per call (tests, the bench). The
start-up self-test also renders its rectangles binned on the GPU; if that comes back wrong,
only the binning stays on the CPU on that device (`gpu_binning_off` says why), and the tests
require it to work wherever the GPU stage runs (lavapipe in CI included).

Limits: at most `MAX_VISITS` (2²⁶) edge visits by the per-row passes (a fill with very many
edges over very many rows bins on the CPU, which splits each edge once), and every buffer
within the device's binding size; otherwise the CPU bins. The work bound (`MAX_WORK`) counts
the room made, an upper bound.

### 3. Hairlines in both renderers

Outline mode draws **hairlines**: the oracle's prepared fill carries the flattened polylines
(closed ones repeat their first point) and `Fill::hairline`. A pixel's coverage is
`1 − d`, clamped, where `d` is the distance from its centre to the nearest segment
(`render::hairline`). That is exactly the area a one-pixel line covers for horizontal and
vertical lines (a line through pixel centres covers them fully, one on a boundary half each
side), a tent-filtered line in other directions (diagonals look up to about 12 % heavier),
with round ends and joins and no darkening where segments overlap. Each segment is one piece
to bin and test, where the stroked hairline was four plus a disc at every bend.

* The oracle computes it per segment over the rows and columns within reach (a direct-coverage
  mode of its accumulation buffer); the GPU's fine shader takes the nearest of the tile's
  segments (with a cheap box reject first) by the same steps (`seg_dist`).
* The binning puts each segment, whole, into every tile with a pixel centre within reach
  (conservatively: two pixels around it); no backdrops.
* This changes the oracle's outline mode slightly (anti-aliasing of 1 px lines); it is an
  editing aid, never exported (vector exports draw outlines as their own strokes), and the
  engine's tests and goldens are unchanged.

Not hairlines (yet): strokes thinner than a pixel. They keep a one-pixel stroke polygon faded
by their width; drawing them as hairlines would change exported pictures and the XFL goldens,
which is its own decision.

## Measured (2026-11-10, release, RTX 5090, 32 threads, a machine shared with other builds)

`cargo run --release -p anicraft-gpu --example gpu_bench`: *direct* is what the calling thread
spends with `render_texture` (prepare + bin + upload and submit), *tiled* is the stage's way
(512 px tiles in parallel until the GPU has finished). Before: binned on the CPU; after: as
`Binning::Auto` bins (the GPU from 32,768 edges, on Vulkan). Best of three runs, Vulkan.

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

## Not in this slice

* GPU binning for fills inside masks and groups (their tile streams, ADR 0029).
* Flattening and stroking on the GPU (preparation is now the largest CPU term of a heavy
  frame: 8–9 ms of 50k shapes).
* Fewer, larger dispatches for the stage's tiles (each 512 px tile is its own submission, so the
  binning passes' set-up repeats per tile).
* Hairlines for strokes thinner than a pixel (above).
* Guide lines and selection outlines: the stage never renders guides, and selection outlines
  are drawn by the shell's painter, not the stage renderer.

## Consequences

* The tile format is one integer backdrop per tile plus left-edge steps, on the CPU and the GPU
  alike; `Binned` carries the op headers apart (`op_heads`) and, when the GPU bins, the plan.
* A test compares the GPU's tile lists with the CPU's, tile by tile, entry by entry and pixel
  by pixel, on stress frames, outlined onion frames and the analytic scenes; GPU-binned frames
  are checked against the oracle read back and through a texture; on Vulkan and Direct3D 12.
* `gpu_bench --check` enforces the GPU stage's timing budgets (docs/performance.md).
