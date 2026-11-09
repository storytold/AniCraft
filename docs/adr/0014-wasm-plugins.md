# ADR 0014: WebAssembly plug-ins over the command registry

* Status: accepted (2026-11-09)
* Relates to: requirements U2, roadmap M6, ADR 0001 (undo), ADR 0004 (destructive actions),
  ADR 0006 (platforms), ADR 0013 (ActionScript); `docs/plugins.md` is the contract

## Context

U2 settled AniCraft's scripting surface: the Craft-family plug-in system, as in PhotoCraft
(sandboxed WebAssembly run by `wasmi`, `plugin.*` commands, fuel, memory and time limits, one
undo step per run). PhotoCraft's filters are pure functions: pixels in, pixels out, and no
imports at all. AniCraft's plug-ins are different work: generate shapes, transform the
selection, later drive the timeline from frame scripts. They must read the document and edit
it, and the project rules say every edit is a command. Hostile or buggy plug-ins must never
crash or hang the app, touch files, or leave half an edit behind.

## Options considered

| Option | Verdict |
|---|---|
| **A. No imports** (PhotoCraft's model): document in, document out as one JSON blob | Every plug-in would re-implement edits and the host would have to diff and validate a whole document. Bypasses the command registry, so no shared checks; large documents cost a full copy both ways. Rejected. |
| **B. WASI** (a standard system interface) | Brings files, clocks, randomness and environment into reach; we would spend the design disabling it. Rejected. |
| **C. Wide typed ABI** (one import per operation: `draw_rect(x, y, w, h…)`) | Fast, but every new command needs a new import and a new ABI version; the plug-in surface drifts from the registry. Rejected. |
| **D. Component model / WIT** | Not supported by `wasmi`'s stable feature set; heavy toolchain for authors. Revisit later. |
| **E. `wasmtime`** instead of `wasmi` | A JIT: faster, but large, uses `unsafe` code generation, and doesn't run in the web build. The Craft family standardised on `wasmi`. Rejected. |
| **F. Two imports, JSON requests routed to the command registry** | Chosen. |

## Decision

1. **Crate `anicraft-plugins` (L0)**: the sandbox. It knows WebAssembly, the manifest and the
   limits, not the document. The engine implements its `Host` trait.
2. **ABI v1** (`docs/plugins.md`): exports `ac_abi_version`, `ac_manifest`, `ac_alloc`,
   `ac_run`, `memory`; the only imports allowed are `anicraft.call(ptr, len) -> len` and
   `anicraft.response(ptr, cap) -> n`. A request is `{"op": …}`: `command` (any registry
   command by id with JSON params), `document` (a compact view), `elements` (the active
   layer's elements, optionally with paths) and `log`. Answers are `{"ok", "result"|"error"}`.
   New ops are additive; the version changes only when an existing contract does.
3. **Host calls are resumable traps.** `anicraft.call` copies the request out and traps with a
   marker; the run loop answers it through the `Host` (which borrows the session mutably, so it
   can't live inside wasmi's `'static` store) and resumes. Fuel runs out the same way, in
   10 M-instruction slices, which is where the wall-clock deadline is checked.
4. **Limits**: 16 MiB modules, 256 MiB memory, 10⁹ instructions and 5 s per run, 100,000
   requests, 1 MiB requests and 8 MiB responses, 64 KiB params, 200 log lines, call depth
   1,024, wasmi's strict module limits, a fresh instance per run. Every violation is an error.
5. **Policy**: plug-ins may call document commands only. `file.*`, `plugin.*`, undo/redo and
   the clipboard commands are refused, and so is any namespace the policy doesn't know (a test
   fails when a new namespace appears, forcing a decision). Irreversible confirmations are
   refused inside a run.
6. **One undo step, atomic.** `plugin.run` is one mutating command. The commands a plug-in
   makes run *nested* (`Session::execute_nested`): checked and atomic each, no history of their
   own, state ids refreshed so later requests see earlier edits. Any failure restores the
   document as it was before the run. The step is named after the plug-in ("Undo Star Field").
   The app's live preview runs inside a gesture and puts the document back before each re-run
   (`Session::revert_gesture`), so OK records one step and Cancel none.
7. **Registry per session, folder per OS.** Installed plug-ins live on the `Session` (not a
   process global, not the document), survive New/Open, and are never saved in documents. The
   app's folder is `<settings folder>/plugins` (from `platform.rs`, ADR 0006), or
   `ANICRAFT_PLUGINS`; install copies the module there atomically as `<id>.wasm`. The CLI loads
   plug-ins only with `--plugins` or `ANICRAFT_PLUGINS`, so headless runs stay repeatable.
8. **Confirmations (ADR 0004)**: replacing an installed plug-in's file and removing a plug-in
   whose file is in the folder are irreversible, so they need `"confirm": true`.
9. **UI**: a **Commands** menu between Modify and Control, as in Animate, with Plug-ins
   (installed plug-ins, disabled with a reason when they need a selection), Install Plug-in…,
   Manage Plug-ins… and Reload Plug-ins; plug-ins in the command palette; a dialog generated
   from the manifest's parameters with a live preview beside the stage.
10. **Samples** in Rust (`examples/plugins`: Star Field, Jitter Selection, a small SDK), built
    by a script and never committed as binaries; the host's tests use WAT fixtures compiled
    by the `wat` crate (dev-dependency). The guest SDK has the only `unsafe` a Rust plug-in
    needs (two imports, raw pointers of the exports). It is guest code inside the sandbox, outside
    the AniCraft workspace and its `unsafe_code = "forbid"`.

## Building on it

Frame scripts and ActionScript tier 2 (ADR 0013) reuse this host: a `frame-script` kind whose
`ac_run` answers playback events with ops from the timeline action IR (`stop`, `play`,
`goto`, …), run by the pure player, never by mutating the document. Exporters and easing
presets are further kinds. None of them changes v1.

## Consequences

* Plug-ins get the whole command registry, with the same validation as the UI, the CLI and
  MCP, and every run is one undoable, atomic step.
* Hostile modules end in an error: endless loops by fuel or time, memory bombs at the cap,
  request floods at the request budget, bad pointers and traps as errors (tests in
  `crates/plugins/tests/abi.rs` and `crates/engine/src/plugin_tests.rs`).
* JSON costs about 75 µs per request in a debug build: fine for hundreds or a few thousand
  edits per run, not for per-pixel work. A compact binary view can be added as new ops.
* Runs block the UI thread for at most their time budget; background runs with progress and
  cancel come later.
