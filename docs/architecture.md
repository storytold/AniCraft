# AniCraft architecture

## 1. Philosophy

1. **The document is data; time is a function.** A document is an immutable-by-convention tree
   of plain structs (stage, timelines, layers, keyframes, elements, symbols). Nothing animates by
   mutating state: the picture at frame *f* is `evaluate(document, f) -> DisplayList`, a pure
   function. Playback, scrubbing, export, onion skin and tests are all calls to that function.
2. **One oracle renderer.** The CPU renderer is deterministic and is the reference. Faster
   renderers (wgpu, M3) must match it within a tolerance in golden tests. Agents and CI trust
   CPU pixels.
3. **Everything is a command.** Every user-visible action is a `CommandSpec` in the engine
   (id, label, menu path, shortcut, params doc, `run`). The egui shell, the CLI, the JSON-lines
   channel and (M2) the MCP server dispatch the same ids, so headless parity holds by
   construction. Menus are generated from the registry.
4. **Thin UI.** The egui layer holds only view state (zoom, pan, active tool, panel sizes,
   hover). It reads the session and emits commands. No document logic lives in the UI.
5. **Layered crates, enforced.** A crate depends only on lower layers. Nothing below the shell
   knows about egui, winit or the file dialog.
6. **Never crash.** Errors are values. Commands validate params; recursion (nested symbols) is
   depth-limited; allocations sized by input are capped.
7. **Pay for what changes.** Cache by revision: the stage texture is re-rendered only when the
   document revision, frame, zoom or viewport changes. Rasterize only shape bounding boxes, in
   parallel rows.

## 2. Crate map

```text
app/                         workspace root; the root package is the desktop app (`anicraft`)
  crates/
    geom      L0  Vec2, Affine (+ decompose/compose for tweening), Rect, Path, flattening, hit test
    plugins   L0  WebAssembly plug-in sandbox (wasmi): ABI v1, limits, manifest, registry (ADR 0017)
    audio     L0  Sound decoding (WAV, Ogg Vorbis, MP3), pure mixing/resampling, WAV, waveform peaks
    doc       L1  Document model + timeline evaluation (keyframes, tweens, easing, symbols)
    render    L2  CPU oracle rasterizer, paints/gradients, compositor, PNG encode
    engine    L3  Session (document + history + selection + playhead), command registry, file format
    gpu       L3  wgpu stage renderer over the oracle's prepared fills (ADR 0019; shell feature `gpu-stage`)
    cli       L4  `anicraft-cli`: info / render / render-seq / commands / run / serve (JSON-lines)
  src/        L5  egui shell: menus (from registry), tools, stage, timeline, properties, library
  docs/           requirements, architecture, roadmap
```

| Layer | Crate | May depend on |
|---|---|---|
| L0 | `anicraft-geom` | std, serde |
| L0 | `anicraft-plugins` | wasmi, serde_json (knows nothing of documents; the engine is its host) |
| L0 | `anicraft-audio` | std, lewton, nanomp3 |
| L1 | `anicraft-doc` | geom, audio |
| L2 | `anicraft-render` | geom, doc, rayon, png |
| L3 | `anicraft-engine` | geom, doc, render, serde_json |
| L3 | `anicraft-gpu` | geom, doc, render, wgpu (the version egui brings), pollster, bytemuck |
| L4 | `anicraft-cli` | engine (+ lower) |
| L5 | `anicraft` (shell) | engine (+ lower), eframe/egui |

Planned crates: `io` (L3, XFL/SVG/Lottie/GIF),
`automation` (L4, MCP), `testkit`.

## 3. Document model (`anicraft-doc`)

```text
Document
├── stage: width, height, fps, background
├── timeline: Timeline                 (main scene; scenes list arrives in M2)
│   └── layers: [Layer]                (index 0 = top of the stack, as in the timeline panel)
│       ├── name, visible, locked, outline, kind (Normal | Guide | Mask | Folder)
│       └── keyframes: [Keyframe]      (sorted by start, spans do not overlap)
│           ├── start, duration        (frames covered by this keyframe)
│           ├── elements: [Element]
│           │   ├── Shape { path, fill: Paint?, stroke: Stroke?, transform }
│           │   └── Symbol instance { symbol id, transform, color effect, loop mode, first frame }
│           ├── tween: Tween? { kind: Classic | Shape, easing: Easing }
│           └── sound: FrameSound? { sound id, sync: Event | Start | Stop | Stream, repeat, loop }
├── library: [Symbol { id, name, kind: Graphic | MovieClip | Button, timeline }]
├── bitmaps: [Bitmap { id, name, PNG data }]
└── sounds: [Sound { id, name, format, rate, channels, frames, data as imported }]
```

### Evaluation

`evaluate(doc, frame) -> DisplayList` walks layers bottom to top. For each visible layer it finds
the keyframe covering the frame. If that keyframe has a tween and a following keyframe exists, it
computes `t = ease((frame - start) / duration)` and interpolates element *i* of both keyframes
when they are compatible:

* **Transforms** are decomposed into translation, scale, rotation and skew and interpolated
  component-wise (rotation along the shortest arc), so a rotating tween does not shear like a
  naive matrix lerp would.
* **Colour effects** (alpha multiplier, tint) and **paints** interpolate linearly in
  straight-alpha sRGB, like the reference tools.
* **Shape tweens** interpolate path points when both paths have the same command structure
  (M2 adds correspondence via shape hints).

Symbol instances recurse into the symbol's timeline at the local frame given by the loop mode
(`Loop`, `PlayOnce`, `SingleFrame`) and the first frame. Recursion is limited to depth 32, so a
cyclic library in a malformed file renders what it can instead of overflowing the stack.

### Easing

`Easing` is `Linear`, `QuadIn/Out/InOut`, `CubicIn/Out/InOut`, `Classic(strength −100…100)`
(the traditional single-slider ease) or `CubicBezier(x1, y1, x2, y2)` solved with Newton
iterations and a bisection fallback, the same model CSS uses.

### Sound

Sound is time as a function too (ADR 0020): `doc::sound::voices(doc, rate)` lays every
keyframe's sound out in output frames by playing the timeline from the start (Event, Start,
Stop and Stream resolved, repeats and loops included), and `audio::mix(voices, rate, origin,
start, out)` turns any stretch of it into samples, exactly when the rates match. Export Audio,
the video soundtrack and stage playback all use these two pure functions; the shell's audio
callback only calls `Player::fill`, and the playhead follows the player's clock while sound
plays.

## 4. Rendering (`anicraft-render`)

```text
DisplayList ──► for each item:
                  flatten Bézier path under (view × item) transform, tolerance 0.1 px
                  ├─ fill:   lines ──► signed-area coverage accumulator (bbox-sized)
                  └─ stroke: polylines ──► stroker (segment quads + round joins/caps) ──► accumulator
                coverage ──► fill rule (non-zero / even-odd) ──► paint shader ──► src-over blend
                           (rows of the bbox in parallel with rayon)
```

* **Coverage accumulation.** Each line adds signed area to a per-pixel accumulation buffer;
  a prefix sum along each row gives exact analytic coverage, with no supersampling. Lines are
  clipped against the buffer's x range: the part left of it collapses onto the left edge, which
  keeps winding correct. Cost is proportional to edge length plus bbox area.
* **Fill rules.** Non-zero is `min(1, |w|)`; even-odd folds the accumulated winding with a
  triangle wave. Both are exact for non-overlapping edges inside a pixel.
* **Paints.** Solid, linear and radial gradients with pad spread. They are evaluated in the
  shape's local space through the inverse transform, so gradients follow the shape's
  transforms and tweens.
* **Compositing.** f32 premultiplied RGBA; src-over; conversion to 8-bit happens only at
  output. Group alpha for symbols with colour effects comes in M2 (offscreen layer).
* **Determinism.** No dependence on thread scheduling: each row is written by exactly one task,
  and shapes are composited in order.
* **Parallelism.** Phase 1 prepares items in parallel (cull, flatten, stroke, shader). Phase 2
  rasterizes 32-row bands in parallel, each compositing all ops that touch it, in order.
  Output is bit-identical to serial. Numbers are in [performance.md](performance.md).

**GPU stage** ([ADR 0019](adr/0019-gpu-stage.md), behind the shell's `gpu-stage` feature):
`render::plain_fills` hands the oracle's own prepared fills to `anicraft-gpu`, which bins edge
pieces into 16 × 16 tiles with per-row backdrops (CPU, parallel) and computes each pixel's
exact area coverage and src-over in one compute pass. Display lists with masks, groups or
bitmaps are drawn by the oracle instead. The engine's `RenderService` takes any `Rasterizer`
and falls back to the oracle per job, so the engine itself never depends on the GPU.

## 5. Engine (`anicraft-engine`)

* `Session` owns the `Document` (a **persistent, structurally shared tree**: layers,
  keyframes, elements, paths and symbols live behind `Arc`, mutated only via `Arc::make_mut`),
  the content `state` id, the save point, the playhead, the active layer, the selection, a
  per-`(state, frame)` display-list cache, and the `History`.
* `History` stores document snapshots (O(layers) to take) with an estimate of each step's
  unique bytes, bounded by a memory budget. Drags coalesce into one step. Undoing to the saved
  state makes the document clean. See [ADR 0001](adr/0001-undo-persistent-snapshots.md).
* `RenderService` renders frames on a worker thread with an LRU frame cache keyed by
  `(state, frame, view)`, and prefetches during playback. See
  [ADR 0002](adr/0002-render-pipeline.md).
* `CommandSpec { id, label, menu, shortcut, params, run }`. `run(&mut Session, &Value) ->
  Result<Value, EngineError>`. The registry (`commands()`) is the single list the shell, CLI and
  agents read. `Session::execute(id, params)` handles undo snapshots for mutating commands.
* **File format** `.anicraft` v2: JSON `{ "format", "version": 2, "paths": [svg…], "document" }`
  with paths as SVG path strings, shared paths written once and streamed back as shared `Arc`s.
  v1 files still load; unknown fields are ignored and missing fields default. See
  [ADR 0003](adr/0003-file-format-v2.md).

## 6. Headless and agents (`anicraft-cli`)

```text
anicraft-cli info <file|--sample>
anicraft-cli render <file|--sample> --frame N --out frame.png [--scale S]
anicraft-cli render-seq <file|--sample> --out-dir dir [--from A --to B]
anicraft-cli commands                       # registry as JSON (id, label, menu, params)
anicraft-cli run <file|--sample> <script.json> [--save out.anicraft]
anicraft-cli serve [file]                   # JSON-lines: {"id":1,"cmd":"draw.rect","params":{…}}
```

`serve` keeps one session alive and answers each request line with
`{"id":…, "ok":true, "result":…}` or `{"id":…, "ok":false, "error":"…"}`. The MCP server (M2)
is a thin adapter over the same loop.

## 7. Desktop shell (`anicraft`)

Panels follow the familiar animation-tool layout. The menu bar sits on top, with an edit bar
under it. Tools are on the left, the stage in the centre over a pasteboard, and Properties and
Library on the right. The timeline at the bottom has layer rows (eye and lock toggles), a frame
grid with keyframe dots, tween spans and a playhead, plus transport controls and
fps/time readouts. Theme tokens live in `theme.rs`.

The stage texture comes from the CPU renderer and is cached per
`(revision, frame, zoom, viewport)`. Overlays (selection boxes, drag previews, the stage frame)
are drawn with the egui painter in screen space.

`--screenshot <file.png>` runs the normal UI for about 120 frames, then saves the window picture
and exits. CI and Craft Anything use it for visual checks.

## 8. Testing strategy

| Kind | Where | What |
|---|---|---|
| Unit | every crate | affine algebra, flattening tolerance, easing monotonicity and endpoints, keyframe lookup |
| Golden / analytic | render | coverage of axis-aligned rects is exact; circle area within 0.5 %; gradients hit endpoints |
| Property | geom, doc | decompose∘compose ≈ identity; tween at t=0/1 equals the keyframes |
| Robustness | engine | every command with null / wrong-type / huge params returns `Err` or `Ok`, never panics; cyclic symbols render |
| Round trip | engine | save → load → identical document; render identical before and after |
| CLI | cli | render produces a PNG of stage size with non-background pixels |
| Visual | shell | `--screenshot` produces a non-blank image |
| Perf (M2) | benches | criterion benches for flatten, rasterize, evaluate at 1080p with 1,000 shapes |
