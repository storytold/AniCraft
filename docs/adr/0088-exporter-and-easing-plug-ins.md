# ADR 0088: Exporter and easing plug-ins

* Status: accepted (2026-11-13)
* Relates to: roadmap M6, ADR 0017 (WebAssembly plug-ins), ADR 0081 (ease presets), ADR 0060
  (publish: progress and cancel), ADR 0004 (destructive actions), ADR 0036 (web build);
  `docs/plugins.md` is the contract

## Context

ADR 0017 reserved two more plug-in kinds on the `wasmi` host: exporters (frames in, bytes out)
and easing presets. Studios want their own formats (a game engine's animation data, a text dump
for review tools) without forking AniCraft, and artists want eases the presets don't have
(stepped motion for stop-motion looks, house curves). Both must keep the guarantees of the
sandbox: a hostile or buggy module ends in a clear error, never a crash or a hang; and both
must keep the project rules: everything is a command (CLI and MCP too), time is a function
(`evaluate` stays pure), and a document opens the same everywhere.

## Options considered

| Question | Options | Choice |
|---|---|---|
| What an exporter sees | the live document; the `.anicraft` JSON; **the evaluated display list per frame** | The display list: the flattened data AniCraft's own exporters draw (paths in local space, stage transform, paints, strokes, colour effects, clips, groups), one frame per request. The plug-in never sees the document model, which can change between versions, nor the session. |
| Frames: pushed or pulled | AniCraft pushes every frame into memory; **the plug-in asks (`frame`)** | Pulled: memory stays bounded by one frame, a plug-in may skip frames (every nth), and progress and Cancel are checked at each request. |
| How bytes come back | JSON chunks in requests (base64 for binary); **`ac_export` returns `(len << 32) \| ptr`** | Returned from memory: binary formats cost nothing extra; the length is checked against a cap before a byte is copied. AniCraft writes the file (the plug-in can't touch files). |
| When an ease runs | at every evaluation (a plug-in call per frame per tween); **once, at load, into a table** | At load. Evaluation stays pure, deterministic, fast and runnable without the plug-in, in the web build, in the HTML page's baked frames and in exports. |
| What a document stores | a reference (plug-in id + ease id); **the sampled table** | The table, as ADR 0081 copies a saved ease's curve into a tween: the document opens and plays the same on a computer without the plug-in, and a newer version of the plug-in never changes an old movie behind the artist's back. The id is kept beside it, for the menu. |
| Table form | a fitted Bézier `CustomEase`; **uniform samples, linear in between** | Samples: a step is a step (a Bézier ease can't jump), any function is drawn faithfully, and evaluation is one multiply and one lerp. |

## Decision

1. **Two new kinds, ABI v1 unchanged.** `kind: "exporter"` declares `format: {name, extension}`
   and an options schema (`params`, as a command's); it exports `ac_alloc` and
   `ac_export(options_ptr, len) -> i64` (`(len << 32) | ptr`, negative = failure, the last log
   line says why). `kind: "ease"` declares 1-32 `eases: [{id, name}]` and exports
   `ac_ease(index: i32, t: f64) -> f64`; it has no `params` and needs no `ac_alloc`/`ac_run`.
   Everything is additive: existing plug-ins and hosts are untouched.
2. **Exporters get frames, not the document.** The host holds a snapshot (`Document`, cheap to
   clone) and answers `movie` (name, size, fps, background, the range, the scenes) and `frame`
   (`index` within the range → that movie frame evaluated, guide artwork left out); `command`,
   `document` and `elements` are refused. `file.export_plugin {format, path, from?, to?,
   options?, confirm?}` runs it and writes the bytes; replacing a file needs `"confirm": true`
   (ADR 0004). `file.export_formats` lists the built-in formats with their commands and the
   exporter plug-ins with their extensions and options.
3. **Exporter limits** (`Limits::exporter`): 5·10¹⁰ instructions (about 100 s of computing),
   10 minutes, 10⁶ requests, 32 MiB answers (a busy frame), 64 MiB output (`max_output_bytes`,
   checked before copying). Progress is the frames read (each once) of the range; **Cancel** is
   a publish `Control` checked by the host at every request and every 10 M-instruction fuel
   slice (`Host::cancelled`), so even a busy loop stops within milliseconds and nothing is
   written.
4. **Eases are sampled at load**: 257 samples at `t = i/256`, through `ac_ease`, on the loading
   budget (a tenth of a run's fuel), with no host calls allowed (so the function sees nothing
   but `t`). A value that isn't finite, is beyond ±10, or misses 0 at t = 0 or 1 at t = 1 by
   more than 10⁻³ fails the load, naming the ease, the time and the value; the ends are then
   pinned exactly.
5. **`Easing::Sampled(Arc<SampledEase>)`** in `anicraft-doc`: name, plug-in id, ease id and
   2-1025 values, made valid however it arrives (file, command, plug-in): thinned, non-finite
   values replaced by the straight line's, clamped to ±10, ends pinned. `apply` interpolates
   linearly. Editing its points makes a custom ease through 48 of its samples (room left to add
   points). The file format stays version 3: the variant is additive; an older build refuses a
   file that uses one, as with ADR 0081's new presets.
6. **Commands.** Easing params take `{"plugin": id, "ease": id or name}` (resolved against the
   installed plug-ins into the sampled curve) and `{"sampled": {…}}` (what `ease.info` returns,
   so the curve copies between tweens without the plug-in); `ease.list` lists the plug-in eases.
   `plugin.run` refuses exporters and eases with where to find them. `file.*` stays refused to
   plug-ins, so a command plug-in can't export.
7. **Interface.** File › Export lists the exporter plug-ins below the built-in formats; one opens
   an **Export** window with its options (generated from the manifest, as a command plug-in's
   dialog), the frame range, the file (Browse… with the format's extension), then a progress
   bar and Cancel while it runs on a worker thread with a snapshot (in the browser, in the
   frame, the file becoming a download). Replacing a file asks in the window. The ease menu
   (timeline, Properties, Motion Editor) gains **Plug-in eases**: each plug-in's eases as curve
   thumbnails under its name; the menu then names the tween's ease by its plug-in name. The
   Plug-ins window gives an exporter an Export… button and says how many eases an ease plug-in
   adds.
8. **Headless.** `anicraft-cli export <file> --format <id> --out <path> [--from --to --frame
   --options '<json>' --overwrite yes]` takes the built-in ids (gif, apng, webp, lottie, html,
   video, svg, png, png-sequence, sprite-sheet) and exporter plug-in ids (with `--plugins`);
   `anicraft-cli formats` lists them. MCP reaches the same commands through `anicraft_run`.
9. **Samples** (`examples/plugins`): **Plain Text Frames** (an exporter: each frame's shapes,
   positions and paints, with Detail and Every nth frame options) and **Stepped Eases** (Stepped
   2, 4, 8 and Smooth Steps 4). The guest SDK gains `exporter!`, `ease!`, `movie()` and
   `frame()`, and native stand-ins for the host imports so a plug-in's pure parts unit-test with
   `cargo test`. CI's web job builds them to wasm, runs their tests and runs the built modules
   through the engine (`crates/engine/tests/example_plugins.rs`).

## Consequences

* Studios can add formats and eases with no AniCraft release, and hostile modules end in an
  error: endless loops by fuel or time (or Cancel), huge outputs at the cap before copying,
  misplaced outputs as ABI errors, NaN or unbounded eases and eases that miss their ends at
  load (tests in `crates/plugins/tests/kinds.rs` and `crates/engine/src/plugin_kinds_tests.rs`).
* A movie using a plug-in ease plays the same everywhere; the cost is about 2.5 KB of JSON per
  tween that uses one, and that a later version of the plug-in doesn't update old tweens (pick
  the ease again).
* Measured (debug build, this machine): the sample exporter writes the 48-frame demo with paths
  (70 KB) in 77 ms and 92 M instructions; the sample ease plug-in loads and samples its four
  eases in 1.8 ms.
* Not yet: exporter plug-ins in Publish Settings; bitmaps' pixels and sounds handed to
  exporters (a bitmap fill names its library bitmap only); text as text (it arrives as
  outlines); ease plug-ins with parameters; an "update tweens to the plug-in's newer curve"
  action; command plug-ins on the worker thread.
