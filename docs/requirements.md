# AniCraft: technology and requirements

AniCraft is an open-source, native, clean-room 2D vector animation studio in the class of
Adobe Animate, written in **Rust only**. It is a sibling of PhotoCraft, LightCraft, WordCraft and
the other `{Function}Craft` apps and follows the same house rules (Rust only, no webview shells,
everything is a command, never crash, tests are the gate).

This document fixes *what* we build and *with what*. `architecture.md` says *how the pieces fit*,
`roadmap.md` says *in which order*.

## 1. Product scope

### 1.1 Users and jobs

| User | Job to be done |
|---|---|
| Motion designer / animator | Draw vector art, animate it on a timeline with keyframes, tweens and symbols, preview at the target frame rate, export video/GIF/web. |
| Game / web developer | Author sprite animations and UI motion, export sprite sheets, Lottie or HTML canvas, script interactivity. |
| Educator / hobbyist | A free, fast, cross-platform replacement for an abandoned or subscription-only tool. |
| Agent / pipeline | Create, edit, render and validate documents with no window (CLI, JSON-lines, MCP), in CI or a render farm. |

### 1.2 Functional requirements (feature areas)

Priority: **P0** = M0–M2 skeleton must have, **P1** = parity core, **P2** = long tail.

| Area | Requirement | Pri |
|---|---|---|
| Document | Stage size, frame rate, background colour; multiple scenes; library of symbols and assets. | P0 |
| Timeline | Layers (normal, guide, mask, folder), frames, keyframes, blank keyframes, frame spans, labels, playhead, scrubbing, looping playback at document fps. | P0 |
| Tweens | Classic tween with easing presets and custom cubic-Bézier ease; shape tween; motion tween with property curves (Motion Editor); motion guides. | P0 / P1 |
| Symbols | Graphic (timeline-synced), Movie Clip (independent timeline), Button (Up/Over/Down/Hit); instances with transform, colour effect (alpha, tint, brightness, advanced), loop modes and first frame. | P0 / P1 |
| Drawing | Selection, Subselection, Free transform, Rectangle, Oval, PolyStar, Line, Pen, Pencil, Brush (variable width), Paint bucket, Ink bottle, Eraser, Eyedropper, Hand, Zoom; object drawing and merge drawing models. | P0 / P1 |
| Fills and strokes | Solid, linear and radial gradients, bitmap fills; stroke width, caps, joins, miter limit, variable width profiles, dashes. | P0 / P1 |
| Text | Static, dynamic and input text with font embedding policy; text on stage animates like any element. | P1 |
| Onion skin | Before/after range, tint, outlines. | P1 |
| Camera | Layer-depth camera with pan, zoom, rotate. | P2 |
| Bones / IK | Bone tool, armature poses. | P2 |
| Audio | Import WAV/OGG/MP3, stream vs event sync, waveform on timeline, scrub audio. | P1 |
| Filters / blend | Blend modes, drop shadow, blur, glow, bevel, colour adjust on Movie Clips. | P1 |
| Scripting | Frame scripts and automation (see open question U2 below). ActionScript compatibility for imported Animate projects: keep scripts, then run the ActionScript 3 timeline subset (ADR 0013). | P2 |
| Import | SVG, PNG/JPEG sequences, **XFL (uncompressed FLA XML)**, then binary FLA as a stretch goal. | P1 |
| Export | PNG / PNG sequence, animated GIF, sprite sheet + JSON, SVG (static frame), **Lottie JSON**, HTML canvas, MP4/WebM via a system encoder (ffmpeg pipe), optionally SWF. | P0 (PNG) / P1 |
| History | Unlimited undo/redo, named history entries, history is per document. | P0 |
| Headless | Every action available from the CLI, a JSON-lines control channel and an MCP server; deterministic renders. | P0 |

### 1.3 Non-functional requirements

| Quality | Requirement (measurable where possible) |
|---|---|
| **Never crash** | No panics in non-test code. Malformed files, bad command params and odd input return actionable errors. Every command is fuzzed with adversarial params (`panic_hunt` test). |
| **Determinism** | The same document + frame renders byte-identical PNGs on every platform with the CPU renderer (fixed-point-free f32 path, no time or thread-order dependence). This is the oracle for tests and agents. |
| **Performance** | Stage playback at document fps (24/30/60) for 1920×1080 with ≥1,000 visible shapes; scrub latency < 16 ms; open a 50 MB document < 1 s; render a 1080p frame headless < 20 ms on 8 cores. Budgets live in `perf/budgets.toml` from M2. |
| **Memory** | Documents are structurally shared (copy-on-write); an undo step costs proportional to what changed, not the document size. |
| **Fidelity** | Analytic anti-aliasing (exact area coverage), correct non-zero and even-odd fill rules, gradient spread modes, premultiplied-alpha compositing. |
| **Portability** | Windows, macOS, Linux desktop; the engine layers (L0–L5) also build for `wasm32-unknown-unknown`. |
| **Accessibility** | Keyboard reachable commands, scalable UI, high-contrast theme, screen-reader labels via AccessKit. |
| **Localisation** | Every interface string translatable, keyed by its English text (ADR 0040); catalogs from the Craft family's craft-i18n; a missing translation shows English, never an error. The command registry, CLI, MCP and file format stay English. |
| **Clean room** | Behaviour and look learned only from public documentation, public specs and observation. No proprietary code, shaders, presets or assets. |
| **Licensing** | MIT OR Apache-2.0. Every third-party asset is permissively licensed and listed in `ATTRIBUTION.md`. |

## 2. Technology choices

| Concern | Choice | Why |
|---|---|---|
| Language | Rust 2024 edition, stable toolchain (MSRV 1.95, ADR 0062: the macOS Apple Event crate needs it; CI builds with current stable) | Memory safety, fearless parallelism, one language for engine, UI, CLI and web. |
| UI | **egui / eframe 0.36** (wgpu backend) | Immediate mode keeps the UI a thin function of state, matches PhotoCraft, runs native and on the web. |
| Rendering (oracle) | In-house CPU rasterizer: Bézier flattening + signed-area coverage accumulation, rayon row parallelism | Deterministic, testable, headless; the reference every other renderer is compared against. |
| Rendering (interactive, M3) | wgpu compute/fragment renderer (sparse strips or tile binning in the style of public Vello/Pathfinder papers), compared per-pixel against the CPU oracle | Fast stage at high zoom and 4K; GPU and CPU must agree within a tolerance in tests. |
| Geometry | In-house `geom` crate (affine, Bézier, flattening, hit test); `kurbo`-compatible API shape | Small, no_std-friendly, full control over tolerance and determinism. |
| Serialization | serde + serde_json for `.anicraft` (JSON manifest), zip container from M2 for embedded media | Human-diffable, agent-friendly, versioned with `#[serde(default)]`. |
| Images | `png` crate for encode/decode; GIF encoder from M1 | Minimal dependencies. |
| Parallelism | rayon | Row and tile parallelism in the renderer, batch export. |
| Errors | thiserror per crate | Typed, propagatable, no panics. |
| Audio (M5) | cpal for output; own WAV reader, `lewton` (Ogg Vorbis) and `nanomp3` (MP3) for decoding (ADR 0020) | Pure Rust, cross-platform, permissive (symphonia is MPL-2.0, not allowed). |
| Video export (M4) | ffmpeg as an external process (pipe raw frames) | No codec licensing in the binary. |
| Testing | `cargo test`, golden-image tests, property tests (proptest), adversarial command fuzz, criterion benches | Tests are the gate. |
| Automation | CLI (`anicraft-cli`), JSON-lines `serve` mode, MCP server (M2) over the same command registry | Headless parity with the UI by construction. |

### Rejected options

* **Tauri / Electron / any webview UI**: violates the Rust-only rule of the Craft family.
* **Skia / tiny-skia as the renderer of record**: good libraries, but owning the rasterizer is
  central to the project's rendering and performance goals and to cross-renderer determinism.
  tiny-skia stays acceptable as a *test-only* cross-check.
* **A retained-mode UI toolkit**: more state to keep in sync with the document; egui's immediate
  mode plus a command engine gives one source of truth.

## 3. Constraints and rules (inherited from the Craft family)

1. Rust only; no unsafe (`unsafe_code = "forbid"` workspace-wide).
2. Everything a user can do is a **command** with an id, label, menu path, shortcut, params schema
   and tests. UI, CLI, JSON-lines and MCP all dispatch by id.
3. The UI is thin: no document logic in `ui`.
4. Input-derived numbers are hostile: checked arithmetic, bounded allocation, bounded recursion
   (symbols can nest and can be cyclic in malformed files).
5. Layering is enforced; lower layers never depend on egui/eframe/winit.
6. Every change comes with tests; renderer changes come with golden images.

## 4. Open questions

| Id | Question | Current decision (until settled) |
|---|---|---|
| U1 | Which interchange format first? | Own `.anicraft` + PNG/GIF first (headless verification needs it), then **XFL import** (migration path for existing projects), then Lottie/SVG export. Chosen by the builder in auto mode. |
| U2 | Scripting surface replacing ActionScript/JSFL? | **Settled by the project lead: use the sibling apps' plug-in system.** As in PhotoCraft (`docs/plugins.md` there): sandboxed WebAssembly plug-ins run by `wasmi` (pure Rust, MIT OR Apache-2.0, works in the web build), with no host imports, fuel/memory/time limits, `plugin.install/list/run/remove/reload` commands, and one undo step per run. AniCraft's ABI targets vector/timeline work (shape generators, selection transforms, easing presets, exporters) and calls the same command registry. Commands-only scripting (CLI/JSON-lines) stays available. **ActionScript** (asked for by the project lead, low priority) is a compatibility front end on top, not a second engine: ADR 0013. **Built (first slice, ADR 0017, `docs/plugins.md`)**: command plug-ins that call the registry through a two-import JSON ABI. |
| U3 | When does the GPU renderer become primary on stage? | CPU oracle drives the stage until the GPU renderer passes golden parity tests (M3). |
