# Plug-ins: the WebAssembly plug-in API (ABI v1)

AniCraft plug-ins are **WebAssembly modules** run in a sandbox, the same host model as the other
Craft apps (PhotoCraft's filter plug-ins). One `.wasm` file works on Windows, macOS, Linux and,
later, the web build. A plug-in adds a command: it reads the open document and changes it
through AniCraft's own command registry, so everything it does is checked like any other edit
and can be undone in one step. The decision and the alternatives are in
[ADR 0017](adr/0017-wasm-plugins.md).

This page is the contract for plug-in authors and the reference for the host
(`crates/plugins`, the `plugin.*` commands in `crates/engine/src/cmds_plugin.rs`).

## Using plug-ins

In the app: **Commands › Plug-ins** lists the installed plug-ins. One with settings opens a
dialog generated from its parameters, with a live preview on the stage; **OK** (Enter) keeps
the result as one undo step named after the plug-in, **Cancel** (Esc) leaves the document as it
was. **Commands › Install Plug-in…** picks a `.wasm` file, **Manage Plug-ins…** shows what is
installed (with Run, Remove, the plug-ins folder and Reload), and **Reload Plug-ins** re-reads
the folder. The command palette (Ctrl+K) finds plug-ins by name too. A plug-in that works on
the selection is greyed out until something is selected.

| Command | Params | What it does |
|---|---|---|
| `plugin.install` | `{"path": "x.wasm", "confirm"?: true}` | Validates the module and installs it. With a plug-ins folder, the module is copied there as `<id>.wasm` (atomically). Replacing an installed plug-in's file is irreversible, so it needs `"confirm": true` (ADR 0004). |
| `plugin.list` | `{}` | `{"plugins": [{id, name, version, author, description, kind, entries, selection, params, notation, size, source, in_folder}], "folder", "abi"}`. In the app: the Plug-ins window. |
| `plugin.run` | `{"id": "…", "params"?: {…}}` | Runs a plug-in. Missing params take their defaults; numbers are clamped to their range; a wrong type is an error. One undo step; on any failure the document is left exactly as it was. Returns the run's log, request count and instructions used. |
| `plugin.remove` | `{"id": "…", "confirm"?: true}` | Uninstalls. When its file is in the plug-ins folder it is deleted, which needs `"confirm": true`. |
| `plugin.reload` | `{"folder"?: "…"}` | Re-reads every `*.wasm` in the plug-ins folder (first making `folder` the plug-ins folder when given). A bad module is reported and skipped. |

**Where plug-ins live.** The app uses a `plugins` folder in its settings folder:
`%APPDATA%\AniCraft\plugins` on Windows, `~/Library/Application Support/AniCraft/plugins` on
macOS, `$XDG_CONFIG_HOME/anicraft/plugins` (default `~/.config/anicraft/plugins`) on Linux. Every
`*.wasm` in it loads at start. The `ANICRAFT_PLUGINS` environment variable points somewhere
else instead (handy while writing a plug-in). Plug-ins belong to the session, not the
document: they stay installed across New and Open, and documents never contain plug-ins.

**Headless.** `anicraft-cli` loads plug-ins only when asked, so scripted runs are repeatable:
`--plugins <folder>` (or `ANICRAFT_PLUGINS`) on any subcommand, or `plugin.reload` with a folder.
Over MCP the same commands are available through `anicraft_run`.

```sh
anicraft-cli exec --sample plugin.run '{"id":"org.anicraft.star-field","params":{"count":200}}' \
    --plugins examples/plugins/dist --save stars.anicraft
```

## The sandbox

The host is [`wasmi`](https://github.com/wasmi-labs/wasmi), a pure-Rust WebAssembly interpreter
(MIT OR Apache-2.0), built with full validation, its strict module limits and the portable
dispatcher (the default tail-call dispatcher can overflow the host stack on a long-running
module, which would abort the process). Limits (`anicraft_plugins::Limits`, defaults):

| Limit | Default | On violation |
|---|---|---|
| Host imports | only `anicraft.call` and `anicraft.response` (no WASI, files, clock, randomness or network) | module rejected at install |
| Module size | 16 MiB | rejected |
| Linear memory | 256 MiB per instance | `memory.grow` returns −1; an oversized initial memory fails to load |
| Instructions (fuel) | 1,000,000,000 per run (a tenth for loading) | the run stops: "exceeded its instruction budget" |
| Wall clock | 5 s per run, host calls included (native builds; checked every 10 M instructions and at every request) | "exceeded its time budget" |
| Requests | 100,000 per run | "exceeded its host-call budget" |
| Request / response size | 1 MiB / 8 MiB | an oversized request traps; an oversized response becomes an error answer |
| All requests of a run | 64 MiB | "exceeded its request budget" (bounds what one run can add to a document) |
| Params handed to `ac_run` | 64 KiB | error |
| Log | 200 lines of ≤ 500 characters (more are counted, not kept) | dropped |
| Call depth | 1,024 frames | trap |

Every run gets a **fresh instance**, so nothing survives from one run to the next. Traps,
exhausted budgets, malformed modules, bad manifests, out-of-range pointers and non-zero return
codes all become errors, and the document goes back to how it was before the run. The CPU
interpreter runs about 470 M simple instructions per second on the development machine, so the
default fuel is roughly two seconds of computing.

**What a plug-in may do.** Requests reach the same commands the menus, the CLI and MCP use,
with the same parameter checks, except:

* `file.*`: no opening, saving, importing or exporting files;
* `plugin.*`: no installing, running or removing plug-ins;
* `edit.undo`, `edit.redo`: the whole run is one undo step;
* cut, copy and paste (artwork and frames): your clipboard is yours.

Commands in a namespace the host doesn't know are refused, and a test fails when a new
namespace appears without a decision. Commands that would need an irreversible confirmation
are refused too.

## ABI v1

A plug-in is a core WebAssembly module (MVP plus the usual post-MVP features: bulk memory, sign
extension, multi-value, saturating conversions; no SIMD, no memory64). All integers are `i32`
unless noted; pointers are offsets into the module's memory.

### Exports

| Export | Signature | Meaning |
|---|---|---|
| `memory` | memory | The module's linear memory. |
| `ac_abi_version` | `() -> i32` | Must return `1`. |
| `ac_manifest` | `() -> i64` | `(len << 32) \| ptr` of the UTF-8 manifest JSON (≤ 64 KiB). |
| `ac_alloc` | `(size) -> ptr` | A block of `size` bytes the host may write, or `0` on failure. Never freed: each run gets a fresh instance. |
| `ac_run` | `(params_ptr, params_len) -> i32` | Runs the command with the validated parameters (UTF-8 JSON object). `0` = success; anything else fails the run (and undoes it), with the last log line as the message. |

### Imports (module `anicraft`, both optional)

| Import | Signature | Meaning |
|---|---|---|
| `call` | `(req_ptr, req_len) -> i32` | Sends one JSON request; returns the length of the response. |
| `response` | `(dst_ptr, dst_cap) -> i32` | Copies up to `dst_cap` bytes of the last response to `dst_ptr`; returns the bytes copied. |

Every response is JSON: `{"ok":true,"result":…}` or `{"ok":false,"error":"…"}` (`ok` always
first). A refused or failed request is an answer, not a trap: the plug-in decides what to do.

### Requests

| Request | Result |
|---|---|
| `{"op":"command","id":"draw.star","params":{…}}` | The command's result (see `anicraft-cli commands` for ids and params). |
| `{"op":"document"}` | `{abi, stage: {width, height, fps, background}, scene, scenes, frame, frames, layer, layers: [{id, name, kind, visible, locked, parent}], selection: [ids]}` (the selection on every layer, ADR 0044) |
| `{"op":"elements","selected"?:bool,"paths"?:bool}` | The elements on the active layer at the playhead (tweens applied): `{layer, frame, elements: [{id, kind: shape\|symbol\|text, name, selected, transform: [a,b,c,d,e,f], bounds: [x,y,w,h] (stage), alpha, fill, stroke, path (SVG path data, local space, with "paths")}]}` (at most 10,000). |
| `{"op":"log","message":"…"}` | `null`; the line goes into the run's report, and the last one explains a failure. |

Unknown ops answer `{"ok":false,"error":"unknown op '…'"}`, so a plug-in can feature-detect.
New ops and plug-in kinds are added without changing version 1; the version changes only
when an existing contract changes.

Two more kinds are below: frame scripts, and exporters and eases
([ADR 0088](adr/0088-exporter-and-easing-plug-ins.md)).

## Frame scripts (`kind: "frame-script"`)

A frame script runs **while the movie plays** (Test Movie, `control.test_movie`), when the
playhead enters a keyframe that names one of its entry points, and steers playback with the
same timeline actions the Actions panel edits ([ADR 0024](adr/0024-frame-scripts-and-test-movie.md)).
It never changes the document, so the stage and every export stay as authored.

```json
{"id": "org.example.menu", "name": "Menu Logic", "kind": "frame-script", "entries": ["main", "intro"]}
```

* `entries`: 1-32 entry points (`[A-Za-z0-9_]{1,32}`), `["main"]` when left out. A keyframe
  picks one: **Window › Actions › Frame › Runs: Frame script**, or
  `actions.set {"layer": …, "frame": …, "plugin": "org.example.menu", "entry": "intro"}`.
* Frame scripts are not commands: they are not in Commands › Plug-ins and `plugin.run`
  refuses them.
* Each run is a **fresh instance**; `ac_run` gets `{"entry", "frame", "label", "timeline"}`
  (the keyframe's frame, 0-based, the label there, and the dot path of the timeline it is on,
  `""` for the main timeline).

| Request | Result |
|---|---|
| `{"op":"player"}` | The player state: `{scene, scene_name, scenes, frame, frames, playing, clips: [{path, name, frame, frames, playing}], pointer: {x, y, down} \| null, hover, tick, timeline: {path, frame, frames, label, playing}}` (`timeline` is the script's own). |
| `{"op":"action","action":{…}}` | Queues one action: `{"queued": 1}`. A malformed action is an error answer. |
| `{"op":"actions","actions":[…]}` | Queues several (at most 256 per run). |
| `{"op":"log","message":"…"}` | A line in the Output panel. |
| `command`, `document`, `elements` | Refused: a frame script can't read or change the document. |

Actions are JSON objects with an `op` (the same as `actions.set` takes; frames are 0-based):

| Action | Fields |
|---|---|
| `{"op":"stop"}`, `{"op":"play"}` | `target?` |
| `{"op":"goto_and_play"}`, `{"op":"goto_and_stop"}` | `frame`: a number or a label; `scene?` (main timeline); `target?` |
| `{"op":"next_frame"}`, `{"op":"prev_frame"}` | `target?` |
| `{"op":"next_scene"}`, `{"op":"prev_scene"}` | — |
| `{"op":"trace","message":"…"}` | — |
| `{"op":"set_property","property":"x\|y\|alpha\|visible\|rotation\|scale_x\|scale_y","value":n}` | `target?` |

`target` is a dot path of instance names from the script's timeline: `""` (itself),
`hero`, `hero.arm`, `parent`, `root`. The queued actions run after the script returns, in
order; if the script fails (a trap, a non-zero return, a budget), none of them run.

**Limits** (`Limits::frame_script`): **20,000,000 instructions per frame, shared** by all the
frame scripts of that frame; 100 ms wall clock per run; 1,000 requests; 64 KiB per request,
1 MiB per response, 1 MiB in all; 50 log lines. A script that runs out writes one Output line
(“exceeded its instruction budget”), the rest of the frame's scripts don't run, and the movie
plays on. A script that reads the player and sends an action takes about 50 µs (debug build).

```rust
// The guest SDK's request helper sends one JSON request and returns the answer.
fn run(p: &Value) -> Result<(), String> {
    let st = ac::request(&json!({"op": "player"}))?;
    if st["frame"].as_u64() == Some(0) && st["pointer"].is_null() {
        ac::request(&json!({"op": "action", "action": {"op": "stop"}}))?;
        ac::request(&json!({"op": "log", "message": format!("waiting on {}", p["entry"])}))?;
    }
    Ok(())
}
```

## Exporters (`kind: "exporter"`)

An exporter adds an export format. File › Export lists it below the built-in formats; it opens
an Export window with the plug-in's options, the frames and the file, then shows a progress bar
and Cancel while the export runs (on a worker thread, with a snapshot of the document).
Headless: `file.export_plugin` and `anicraft-cli export --format <id>`.

```json
{"id": "org.example.text", "name": "Plain Text Frames", "kind": "exporter",
 "format": {"name": "Plain Text Frames", "extension": "txt"},
 "params": {"detail": {"type": "choice", "options": ["Summary", "Shapes"], "default": "Shapes"}}}
```

* `format`: `name` (1-64 printable characters; the menu item and the window's title) and
  `extension` (1-16 characters from a-z 0-9, without the dot; the file dialog's filter).
* `params` are the export's **options**, declared and checked exactly as a command's parameters.
* Exports: `memory`, `ac_abi_version`, `ac_manifest`, `ac_alloc` and
  **`ac_export(options_ptr, options_len) -> i64`**: `(len << 32) | ptr` of the file's bytes in
  the module's memory, or a negative number when it fails (the last log line says why).
  AniCraft writes the file; an exporter never touches files.
* An exporter **never sees the document**, only evaluated frames:

| Request | Result |
|---|---|
| `{"op":"movie"}` | `{abi, name (the file's stem), width, height, fps, background: "#rrggbbaa", from, to (movie frames, 0-based), frames (in the export), total_frames, scenes: [{name, start, frames}]}` |
| `{"op":"frame","index":i}` | Frame `from + i`, evaluated as the stage shows it at that frame (guides left out): `{index, frame, scene, scene_frame, items, clips, groups}` |
| `{"op":"log","message":"…"}` | `null` |
| `command`, `document`, `elements` | Refused |

Each item of a frame is one filled or stroked path, in drawing order:
`{layer, element, path (SVG path data, local space), transform: [a,b,c,d,e,f] (local → stage),
fill, stroke, fill_rule: "nonzero"|"evenodd", alpha, color?: {mul: [r,g,b,a], add: [r,g,b,a]}
(the colour effect, when there is one), clip?, masks?, group?}`.
`fill` is `null` or `{"type": "solid", "color": "#rrggbbaa"}`, `{"type": "linear", start, end,
stops: [[offset, colour]], spread, linear_rgb}`, `{"type": "radial", center, radius, focal,
matrix, stops, spread, linear_rgb}` or `{"type": "bitmap", bitmap (library id), repeat,
matrix}`; `stroke` is `null` or `{color, width, cap, join, miter_limit, dash?, variable_width?}`.
An item with `masks: n` is mask artwork that forms clip `n` (it isn't drawn); an item with
`clip: n` is drawn only inside clip `n`; `clips[n]` is the clip it is intersected with.
`groups[n]` (`{parent, alpha, blend, clip, filters}`) fades its items as one. Text arrives as
outlines.

**Limits** (`Limits::exporter`): 50,000,000,000 instructions (about 100 s of computing), 10
minutes, 1,000,000 requests, 32 MiB per answer, **64 MiB of output** (checked before a byte is
copied). Progress is the frames read (each counted once); Cancel stops the export at its next
request or within one slice of fuel, and nothing is written.

## Eases (`kind: "ease"`)

An ease plug-in adds eases to every ease menu (the timeline's, Properties › Frame, the Motion
Editor), under **Plug-in eases**.

```json
{"id": "org.example.steps", "name": "Stepped Eases", "kind": "ease",
 "eases": [{"id": "steps4", "name": "Stepped 4"}, {"id": "smooth4", "name": "Smooth Steps 4"}]}
```

* `eases`: 1-32 of `{id ([A-Za-z0-9_-]{1,32}), name (1-48 printable characters)}`. No `params`.
* Exports: `memory`, `ac_abi_version`, `ac_manifest` and
  **`ac_ease(index: i32, t: f64) -> f64`**: ease number `index`'s eased progress at time `t`.
* AniCraft calls it **once, when the plug-in loads**, at 257 times (`t = i / 256`), with no host
  calls allowed, and keeps the table. The values must be finite, within −10..10, 0 at t = 0
  and 1 at t = 1 (within 0.001); otherwise the plug-in doesn't load and the message names the
  ease, the time and the value.
* A tween given one **keeps the table** (`Easing::Sampled`, read with linear interpolation), so
  the document opens and plays the same without the plug-in, and a newer version of it never
  changes a finished movie. Pick the ease again to take a newer curve. Editing its points in
  the Custom Ease window makes it a custom ease through 48 of its samples.
* Easing params take `{"plugin": "org.example.steps", "ease": "steps4"}` (its id or name) and
  `{"sampled": {…}}` (what `ease.info` returns; works without the plug-in). `ease.list` lists the
  installed plug-ins' eases.

### Manifest

```json
{
  "id": "org.example.stars",
  "name": "Star Field",
  "version": "1.0.0",
  "kind": "command",
  "author": "Example",
  "description": "What it does, one sentence.",
  "selection": "optional",
  "params": {
    "count":  {"type": "int", "min": 1, "max": 1000, "default": 120, "label": "Stars"},
    "size":   {"type": "number", "min": 1, "max": 50, "default": 4},
    "twinkle":{"type": "bool", "default": true},
    "shape":  {"type": "choice", "options": ["star", "dot"], "default": "star"},
    "color":  {"type": "color", "default": "#fff4c2"}
  }
}
```

* `id`: 1–64 characters from `A-Z a-z 0-9 . _ -` (reverse-DNS recommended). Installing a
  module with the same id replaces the old one.
* `name`: the menu label, 1–64 printable characters (control characters and invisible
  format characters such as bidi overrides are rejected). The app adds "…" when there are
  parameters.
* `kind`: `"command"`, `"frame-script"` (see Frame scripts above, with its `entries`),
  `"exporter"` (with its `format`) or `"ease"` (with its `eases`); see above.
* `selection`: `"optional"` (default; generators) or `"required"` (selection transforms: the
  menu item is disabled without a selection and `plugin.run` refuses).
* `params`: in dialog order, at most 32. Names are `[A-Za-z0-9_]`, 1–32 characters, not
  starting with `_`, not `id`. `label` (optional) is the dialog label; by default the name with
  underscores as spaces. Types: `number` and `int` (`min`, `max`, `default`; a slider),
  `bool` (a checkbox), `choice` (`options`, `default`; a dropdown), `color` (`"#rrggbb"`; a
  swatch).

## Writing a plug-in in Rust

[`examples/plugins`](../examples/plugins) has a small guest SDK and two complete plug-ins:

* **Star Field** (`star-field/`): a generator. Scatters stars of varied size, angle and
  brightness over the stage on a new layer (`layer.new`, `draw.star`,
  `element.set_transform`, `element.set_alpha`).
* **Jitter Selection** (`jitter/`): a selection transform. Moves, turns and scales each selected
  element about its own centre (`elements`, `element.select`, `modify.transform`).
* **Plain Text Frames** (`text-export/`): an exporter. The movie as readable text, one block per
  frame, each shape with its position and paint (`exporter!`, `movie`, `frame`).
* **Stepped Eases** (`stepped-ease/`): an ease plug-in. Stepped 2, 4 and 8, and Smooth Steps 4
  (`ease!`).

```rust
// An exporter: options in, the file's bytes out.
ac::exporter!(MANIFEST, export);
fn export(options: &Value) -> Result<Vec<u8>, String> {
    let movie = ac::movie()?;
    let mut out = String::new();
    for i in 0..movie["frames"].as_u64().unwrap_or(0) {
        let f = ac::frame(i)?;
        out += &format!("frame {}: {} shapes\n", f["frame"], f["items"].as_array().map_or(0, Vec::len));
    }
    Ok(out.into_bytes())
}

// An ease plug-in: eased progress of ease `index` at time `t`.
ac::ease!(MANIFEST, ease);
fn ease(index: usize, t: f64) -> f64 {
    let n = [2.0, 4.0][index.min(1)];
    if t >= 1.0 { 1.0 } else { (t * n).floor() / n }
}
```

Built for anything but WebAssembly, the SDK's host imports are stand-ins that answer every
request with an error, so `cargo test` runs a plug-in's pure parts natively (the samples do).

```rust
use anicraft_plugin_sdk::{self as ac, Rng};
use serde_json::{Value, json};

const MANIFEST: &str = r#"{"id":"org.example.dots","name":"Dots","kind":"command",
  "params":{"count":{"type":"int","min":1,"max":100,"default":10}}}"#;

ac::plugin!(MANIFEST, run);

fn run(p: &Value) -> Result<(), String> {
    let stage = &ac::document()?["stage"];
    let (w, h) = (stage["width"].as_f64().unwrap_or(550.0), stage["height"].as_f64().unwrap_or(400.0));
    let mut rng = Rng::new(1);
    for _ in 0..p["count"].as_u64().unwrap_or(10) {
        let (x, y) = (rng.unit() * w, rng.unit() * h);
        ac::command("draw.ellipse", json!({"x": x, "y": y, "w": 8, "h": 8, "stroke": null}))?;
    }
    Ok(())
}
```

Build them with `examples/plugins/build.sh` (needs `rustup target add wasm32-unknown-unknown`);
the modules land in `examples/plugins/dist/`. `std` and `serde_json` are fine as long as nothing
touches WASI. Plug-ins get no randomness: use a seed parameter (the SDK's `Rng`), which also
makes every preview match the result. The only `unsafe` code a Rust plug-in needs is the two
imports and the exports' raw pointers, all inside the SDK. Any language that compiles to core
WebAssembly works (C, Zig, AssemblyScript, hand-written WAT; the host's tests are written in
WAT).

## Performance

Measured on the development machine (Windows, debug build of AniCraft with optimised
dependencies, `anicraft-cli run --sample … --plugins examples/plugins/dist`):

| | |
|---|---|
| Load both sample modules (72 KB and 80 KB) | under 5 ms of the CLI's ~45 ms start-up |
| Star Field, 120 stars (361 requests, 60 M instructions for 200) | ~27 ms, fast enough for the live preview |
| Star Field, 1,000 stars (3,001 requests) | ~230 ms |
| Jitter Selection on one element | ~1 ms (0.8 M instructions) |
| Plain Text Frames, the 48-frame demo with paths (70 KB of text) | ~77 ms (92 M instructions) |
| Stepped Eases: load and sample four eases (1,028 calls) | ~1.8 ms |
| A busy loop in the interpreter | ~470 M instructions/s (20 M in 43 ms) |

A request costs about 75 µs here: most of it is the command itself plus the plug-in parsing
and writing JSON. Generators that create thousands of elements should expect a fraction of a
second per run.

## Not in v1

* Panels and other UI plug-ins; exporter plug-ins in Publish Settings; bitmaps' pixels and
  sounds handed to exporters; ease plug-ins with parameters.
* State that survives between frame-script runs (each run is a fresh instance).
* Command plug-ins on a background thread with progress and cancel: a run blocks the app for at
  most its time budget (5 s). (Exports already run on one.)
* A compact binary view of large documents (v1 answers JSON only), and requests that edit a
  path's points directly by data (today: `path.*` commands, or redraw with `draw.path`).
* Signing, a plug-in catalogue, and per-plug-in permissions beyond the fixed policy above.
