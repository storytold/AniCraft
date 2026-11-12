# ADR 0072: Filters, Subtract and Invert in HTML canvas export

* Status: accepted (2026-11-12), first slice
* Relates to: ADR 0028 (HTML canvas export), ADR 0025 (bevel and gradient filters), ADR 0002
  (the CPU renderer is the oracle), ADR 0032 (filters on the GPU stage, from the oracle's
  resolved steps), ADR 0055 (9-slice scaling), roadmap M6

## Context

ADR 0028's page drew symbols with filters as if they had none, and Subtract and Invert
blending as Normal, and said so in the export report. Filters are among the most used effects
in Animate (drop shadows under characters, glows on buttons, bevels on titles), so a page
without them looks wrong, not merely approximate. Canvas 2D offers two shortcuts, neither
acceptable:

* **CSS `filter` on the context** (`ctx.filter = "blur(4px) drop-shadow(…)"`): a Gaussian, not
  our box passes; no inner shadow or glow, knockout, hide object, bevel, gradient ramps or
  colour matrix of ours; support and results vary between browsers. The page would differ
  from AniCraft and from browser to browser.
* **`shadowBlur`**: one shadow, Gaussian-ish, outer only, again browser-defined.

Subtract and Invert have no `globalCompositeOperation` at all.

## Decision

### The renderer's filters, ported, run over the group's pixels

`crates/render/src/html/filters.js` is a port of `crates/render/src/filters.rs`, as
`player.js` is of the Rust player: the same box-blur passes (an odd width covers n/2 pixels
each side, an even one a pixel more at half weight, outside the buffer transparent), Drop
Shadow and Glow from the blurred, offset opacity (inner, knockout, hide object), Bevel and
Gradient Bevel from the difference of the blurred opacity towards and away from the light,
Gradient Glow from the glow's intensity, both through the same 257-entry premultiplied ramp
table, inner/outer/full and knockout, and Adjust Color as the renderer's 4 × 5 matrix on
unpremultiplied colour. It works on premultiplied floats (`Float32Array`), as the oracle does.

What is the same number in both places is written once, by Rust:

* **Each filter is exported sanitized**, values as exact `f32` (`fx` on the instance), so the
  page never clamps or defaults differently. Sizes stay in stage pixels and the page scales
  them by its view (the canvas's device pixels per stage pixel), as the renderer does.
* **Angles go as the renderer's own cosine and sine.** Offsets are whole pixels, rounded from
  `distance × scale × cos`; at exact halves (sin 210° = −0.5) an `f32` sine and JavaScript's
  `f64` one round differently. With the oracle's two numbers the products are IEEE `f32`
  products on both sides (the page uses `Math.fround`) and round the same.
* **Adjust Color goes as its matrix**, computed by `filters::color_matrix`; ramps go as their
  sanitized stops (the page builds the same table with the same `f32` interpolation).
* **The group's reach** (`fm`, the sum of the filters' margins) is exported; the page uses the
  renderer's `ceil(margin × scale) + 2`.

### Where the filters run: a layer past the canvas, cut to the artwork plus the reach

A filtered group draws on its own layer, like any group, but that layer reaches past the layer
below it by the filters' reach on every side (at most what keeps it within 4096 × 4096 pixels:
the renderer's limit, and what Safari on iOS allows a canvas), so a shadow cast from off-stage comes back on
stage. After its artwork is drawn the page reads it back, finds the box of pixels with any
opacity, grows it by the reach, and runs the filters over that cut only.

The renderer's buffer is the artwork's path bounds grown by the same reach; the page's is the
drawn pixels' bounds grown by it, which is inside. Both are transparent outside the artwork,
every filter reads transparency (or, inner, full transparency) there, and no filter spreads
further than its margin, so the pixels are the same where either shows any. A test checks
exactly this on the oracle's own layers.

The result is composited into the layer below at the group's alpha and in its blend mode, so
clips from outside the symbol apply after filtering and clips inside it before, as in the
renderer.

### With soft masks (ADR 0070)

A mask's layer always has the place and size of the layer it masks, so inside a filtered
group (which reaches past the canvas) a mask, hard or soft, lines up with what it masks. A
soft mask over a filtered symbol cuts what the filters made: the filtered group is composited
into the clip's layer, and the mask (its artwork drawn as it looks, opacity or luminance
becoming opacity) is applied when the clip ends. A soft mask inside a filtered symbol shapes
what the filters see: its clip's layer is inside the filtered layer. Both are the renderer's
order (clips from outside the symbol after filtering, its own before).

### Subtract and Invert, per pixel

The page reads the group and what is under it where the group drew, applies the renderer's
`blend` (the W3C compositing formula with `max(cb − cs, 0)` or `1 − cb`) and writes the result
back. Those group layers are created for reading (`willReadFrequently`), so reading them is
cheap; the stage canvas below is read where the group covers it.

### Kept, not redone: filtered pixels by what was drawn

Filtering costs milliseconds; most filtered symbols don't change from frame to frame. While a
filtered group draws, the page writes down what it draws (paths, transforms, paints, colour
effects, strokes, text, nested groups, clips and masks) as text; with the view scale and the
filters that text is the key the filtered result is kept by. A group drawn the same way again,
in the same frame (several instances) or later, skips the read-back and the filters. At most 64
results and 32 M pixels (about 128 MB) are kept, least recently used first; a resize clears
them. Drawing the artwork itself is the canvas's (fast) work and is not skipped.

### Measured

`filters.js` in node 24 (V8), on this machine:

| Work | Pixels | Time |
|---|---|---|
| The demo with the GPU bench's filters (ADR 0032) at 1080p: the star's glow | 236 × 236 | 1.8 ms |
| … the ball's drop shadow and bevel | 170 × 164 | 1.6 ms |
| … a frame where both changed | | 3.4 ms (0 when kept) |
| A filtered group covering the whole 1080p frame: Glow (blur 12, quality 2) | 1920 × 1080 | 51 ms |
| … Drop Shadow and Bevel | 1920 × 1080 | 92 ms |
| … Blur 20, quality High | 1920 × 1080 | 146 ms |
| Subtract over the whole 1080p frame | 1920 × 1080 | 6 ms |

So filtered symbols of ordinary size cost a few milliseconds a frame while they change and
nothing while they don't. A filtered group the size of the whole frame, changing every frame,
drops a 1080p page to about 7 to 20 frames per second: the same advice as in Animate (keep
large animated filters small, or bake them into a bitmap) applies.

The first version walked each line through a temporary copy with an inner loop per channel;
blurring along rows and columns in memory order over a copy of the buffer, with the edge tests
taken out of the inner loops, made it about twice as fast (a 1080p four-channel blur pass
from 87 to 40 ms).

### Tested

* **Pixels against the oracle**, in Boa (ADR 0028): the oracle's own unfiltered layer for a
  movie clip with a gradient, a translucent shape and a stroke (soft edges everywhere) is
  placed in a larger buffer, as the page's layer is; the page's `region` cuts it and `apply`
  filters it; the result equals the oracle's filtered layer everywhere (outside it, zero).
  35 cases (every filter; every side, knockout, inner and hide object; even and odd box
  widths; stacks of three) at view scales 1 and 1.5 or 2: largest difference 9.3 × 10⁻⁴,
  under a quarter of an 8-bit step. Breaking the even-width column pass fails it at once.
* **With soft masks**: a filtered movie clip under an alpha or a luminance mask, and one whose
  symbol holds such a mask: both players' display lists agree (clip modes, filters, groups),
  and the page's frame, composed in canvas.js's order from `filters.js` and the mask artwork
  read as the page reads it, equals the oracle's frame at view scales 1 and 1.5 (largest
  difference 4 × 10⁻⁷). Ignoring the mask, or masking before filtering, fails it.
* **Resolution**: box widths, passes and offsets equal the renderer's for 4,160 filters (13
  angles, 8 distances, 4 blurs, 5 scales), including halves.
* **Subtract and Invert**: `blendPixel` equals the renderer's `blend` to 10⁻⁵ over a grid of
  colours and opacities; on 8-bit pixels at a group alpha within one step.
* **Broken data** (unknown kinds, missing fields, NaN) is skipped, never thrown on.
* **In a browser** (headless Chrome, by hand, not in CI): a page with every filter kind,
  Subtract, Invert and a glow cast from off-stage, against the oracle's PNG: 791 of 105,000
  pixels differ by more than 8 levels, all on edges; the same page without filters differs on
  944 (canvas anti-aliasing against the oracle's analytic coverage). The filters add no
  visible error of their own. This check found that the first version of the page lacked
  `filters.js` entirely; a test now checks the page's script holds it.

### Format

`fx` changed from a flag to the filter list and `fm` was added, so `FORMAT_VERSION` is 2. A
page always carries the player that reads it, so old pages keep playing.

## Not yet

* **9-slice scaling** in the page (ADR 0055): a sliced instance's shapes depend on its scale at
  each frame and on scripts that set it, so the page would need the slicing itself (a port of
  `anicraft_geom::slice` with the symbol's box and guides), not baked data. Still reported.
* Keeping a filtered group's pixels when it only moves (a key without its translation, and the
  result drawn where it now is): moving filtered symbols are filtered every frame.
* Filtering in a worker or with WebGL for very large filtered groups.
* Hit tests through filters (the Rust player doesn't either: a glow isn't clickable).

## Consequences

* A change to `filters.rs` that isn't mirrored in `filters.js` fails the pixel tests.
* The export report no longer lists filters or Subtract and Invert blending.
* Every page carries about 17 KB more script (unminified); only filtered and Subtract/Invert
  groups read pixels back.
