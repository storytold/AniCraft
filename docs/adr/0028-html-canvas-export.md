# ADR 0028: HTML canvas export

* Status: accepted (2026-11-09), first slice
* Relates to: ADR 0024 (frame actions, the action IR and the pure player), ADR 0027 (camera
  layer and layer depth), ADR 0013
  (ActionScript), ADR 0010 (mask layers), ADR 0016 (motion tweens), ADR 0018 (variable-width
  strokes), ADR 0020 (audio), ADR 0002 (the CPU renderer is the oracle), roadmap M6

## Context

Animate's most used publish target today is HTML5 Canvas: a web page that plays the movie,
buttons and timeline scripts included. Our users expect File › Export to give them that, as a
file they can open in a browser, put on a server or send to someone. ADR 0024 gave us the
interactivity model: a typed action IR and a pure, budgeted player in `anicraft-doc`. This
ADR decides what the page contains, how the movie's data is represented, how the browser runs
the player's semantics, and how we keep the browser's behaviour equal to ours.

## Decision

### One self-contained `.html` file

The page holds the movie's data as JSON (in a `<script type="application/json">`), the player
and the page code inline, images as PNG data URIs and sounds as base64. One file, no folder.

* **It opens from a disk.** Browsers refuse `fetch` and `XMLHttpRequest` of local files
  (`file://`), so an `.html` + `.json` + assets layout would play only from a web server: the
  first thing an artist does with an export, double-clicking it, would show nothing. Inline
  data and data URIs have no such limit.
* **It travels as one thing.** Upload it, attach it, drop it in a CMS: nothing to keep together.
* **The cost is small.** Base64 adds a third to images and sounds; vector data dominates most
  movies, and a server's gzip takes back most of base64's overhead. A separate-assets layout
  can come later as an option for large bitmap-heavy movies (see Not yet).

JSON inside `<script>` can only be ended early by `</script`; every `<` in it is inside a
string, so the exporter writes it as `<`, which JSON reads the same. A test checks that
the script sources themselves contain neither `</script` nor `<!--`.

### Baked frames, interactive tree

Two halves with different needs:

* **What a timeline shows at its frame N** depends only on the document: tweens, easing,
  motion tweens' property curves, motion guides, shape morphs with hints. The exporter
  **bakes** it: every timeline the movie uses (each scene's and each reachable symbol's) is
  evaluated frame by frame with the same `evaluate_guided` the renderer uses, into a list of
  layers (bottom first) holding items: shapes (path, paint, stroke, fill rule, transform,
  colour effect) and symbol instances (symbol, element id, name, transform, colour effect,
  blend, event handlers, and for graphics their own baked frame).
* **Which frame each timeline is on** depends on playback: movie clips have their own
  playheads, buttons their pointer state, actions jump around and set properties. That is
  the player's job, at run time, in the browser.

Baking rather than interpolating in JavaScript, because:

* **Fidelity.** The browser shows exactly AniCraft's frames. A JS interpolator would have to
  reimplement easing presets and custom curves, motion-tween property tracks, guide-path
  walking with orientation, and shape morphing with hints (`anicraft-geom::morph` alone is
  ~900 lines): a second implementation to drift from the first, for every feature added later.
* **Size stays reasonable.** Three tables make it compact: paths are stored once (by pointer,
  then by their rounded data), paints once, and identical frames once, as runs per timeline.
  A shape held on a keyframe over 50 frames is one path, one paint and one stored frame. A
  classic tween of a symbol costs a transform per frame (six numbers). Only shape tweens
  store a path per frame, which is what they are. Numbers are rounded to 1/1000 px (1e-5 for
  the linear part of transforms), below anything visible.
* **The player stays tiny.** It never interpolates: it walks the instance tree, picks frames
  and composes transforms.

**The camera** (ADR 0027) is baked the same way: a scene frame seen through its camera carries
the camera's tint (`cc`), and each of its layers the view for its depth (`v`: the camera's
pan, zoom and turn with the layer's parallax), which the player uses in place of the identity
for that layer, in drawing and in hit tests, as `evaluate_player` and `player::tree` do.

Like Lottie export (frame-accurate, not re-keyed), the movie is exact at the cost of not being
"editable" as keyframes; the HTML is a publish target, not an interchange format.

### The player: a port of the Rust player, checked against it

`crates/render/src/html/player.js` is a line-by-line port of `crates/doc/src/player.rs`
(state, `start`/`step`/`input`, `refresh`, `advance`, `settle`, `enter`, `exec`, `resolve`,
`goto_target`, pointer routing and `hit_in`) and of `eval_timeline` (clip and group numbering,
colour-effect concatenation, script-set property overrides), over the baked data. The same
budgets (actions, entered frames, nodes, output lines, events per step) bound it, and the same
iteration orders (Rust's `BTreeMap` order of instance paths, top layer first for scripts) keep
the order in which actions run identical. Messages are the Rust player's, word for word; the
exporter writes each action's description so they can be.

Targets are resolved to steps at export time (`^` the main timeline, `..` up, a name), so the
JS doesn't parse paths; a malformed target carries its error message instead.

**Test strategy.** A permissive pure-Rust JavaScript engine exists: **Boa** (`boa_engine`,
Unlicense OR MIT, crates.io, default features off). It is a **dev-dependency of
`anicraft-render` only**, never in the app or CLI: 66 crates in `Cargo.lock`, all under
licences `deny.toml` already allows, about 35 s added once to a clean test build. Tests run
`player.js` in Boa and drive both players through the same scripts (frames, pointer moves,
presses and releases, between frames and within steps), comparing at every step:

* the state (scene, frame, playing, every movie clip's playhead, hover and pressed instance,
  script-set properties),
* the output lines (traces and error messages),
* and the **display list**: the JS player's evaluation against `evaluate_player` at the same
  state, item by item (path, stage transform, paint, colour effect, clip, mask and group) and
  the clip and group tables. This is how the export is "verified visually" without a
  browser: what the canvas code draws is checked against what the CPU renderer draws.

Fixtures: a self-authored interactive movie (a door movie clip with labels and stops, a
four-state button with click/press/release/over/out handlers, a nested clip addressed as
`panel.door` that talks to `parent`, a looping graphic under a mask, a faded and tinted group,
a gradient with a stroke, a target that doesn't exist, two scenes that jump between each other
by label and scene name), the interactive sample, the demo movie and a go-to loop that runs
into the budget. Mutating the JS player (button states, click detection, frame clamping, tint
order, name lookup) makes these tests fail. Hit tests use a host interface (`inFill`,
`nearStroke`): the browser answers with the canvas's `isPointInPath`/`isPointInStroke`, the
tests with a polygon test over the same flattened paths `Path::contains` uses.

`canvas.js` (drawing, timing, pointer, sound) needs a DOM, so the tests only compile it.

### Drawing on canvas 2D (`canvas.js`)

It follows the CPU renderer, the oracle:

* fills by their rule (`nonzero`/`evenodd`) under the item's transform; gradients are canvas
  gradients in the shape's local space (the transform applies to them as in the renderer),
  stop colours with the colour effect applied; bitmaps are patterns (repeat or once) with
  their matrix;
* strokes in stage space with the stage-space width (`width × mean scale`), round joins and
  caps, as the renderer and the SVG export do; variable-width strokes are exported as the
  filled outline the renderer fills (in the element's parent space);
* a symbol that fades, blends or has filters draws into its own offscreen layer, which is
  composited at its alpha with the blend mode (`multiply`, `screen`, `overlay`, `darken`,
  `lighten`, `hard-light`, `lighter` for Add, `difference`, `destination-in` for Alpha,
  `destination-out` for Erase), so overlaps don't show through;
* a mask layer's masked layers draw into an offscreen layer; the mask's fills (opaque, strokes
  and opacity ignored) draw into another, which cuts the first with `destination-in`. Nesting
  follows the timelines. A clip that never meets its mask shows nothing, like an empty mask on
  the stage;
* the canvas fills the window keeping the stage's aspect ratio (option `fit`, default on) at
  the screen's pixel density; the page background is the stage colour.

Timing: a `requestAnimationFrame` loop steps the player at the document frame rate (fixed
step, at most 250 ms caught up at once). Pointer events go to `input()` at once, between
frames, as in Test Movie. Traces go to the browser console.

### Sound (approximation, optional)

Sounds on keyframes the player enters are handed to the page with each step (the player stays
pure). Event sounds play in full (Repeat ×N, Loop), Start skips a sound already playing, Stop
stops it, Stream plays over its keyframe's span (it does not follow go-tos). Browsers start
audio only after a user gesture, so sound starts with the first click or key. Formats are
those the browser decodes (WAV and MP3 everywhere, Ogg Vorbis not in Safari). `sound: false`
leaves sounds out.

### Command and UI

`file.export_html {path, title?, sound?, fit?, confirm?}` in File › Export › Export HTML
Canvas…; replacing a file asks first (ADR 0004 tier 1, like every export). It reports the
size, the frames stored and what was approximated. Headless: `anicraft-cli exec … file.export_html`.

### Not carried across (reported)

Tints on bitmap fills and **plug-in frame scripts** (they need the WebAssembly plug-in host;
the page logs that they didn't run, as Test Movie does without plug-ins). Outline-mode layers
are drawn in full: outline mode is an authoring view, as in Animate's publish.

**Update (ADR 0072):** filters and Subtract and Invert blending are now drawn in the page as
the renderer draws them (`filters.js`, a port of `filters.rs`, over the group's pixels; Subtract
and Invert per pixel), tested against the oracle's pixels, and no longer reported.

## Not yet

A folder layout (`.html` + `.js` + assets) for bitmap-heavy movies;
keyboard focus and Enter/Space on buttons; a loading poster; hit tests that respect masks
(the Rust player doesn't either); stream sounds that follow the playhead; frame-script
plug-ins in the browser (wasm in the page through the same ABI); ActionScript (ADR 0013 tiers).

## Consequences

* The browser's behaviour has a reference: a change to `player.rs` that isn't mirrored in
  `player.js` fails the comparison tests.
* `instance_start` and `guide_walk` become public in `anicraft-doc`, so the exporter evaluates
  exactly as the renderer does.
* `FORMAT_VERSION` (the `v` field) versions the embedded data; a page always carries the
  player that reads it, so old pages keep playing.
