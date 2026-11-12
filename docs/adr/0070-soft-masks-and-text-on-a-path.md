# ADR 0070: Soft masks and text on a path

* Status: accepted (2026-11-12)
* Relates to: ADR 0010 (mask layers), ADR 0012 (text), ADR 0021 (SVG clips, masks and text),
  ADR 0019 / 0029 (GPU stage and its masks), ADR 0046 (stroke styles), roadmap M4

## Context

SVG import (ADR 0021) brought clip paths and opaque masks onto mask layers, but drew soft
(luminance or faded) masks unmasked and reported `textPath` and per-character positions. Both are
common in design-tool exports. They are also features in their own right: Animate users fade
artwork out with alpha-gradient masks (in Animate through a masking movie clip with cache-as-
bitmap and scripting), and type on a curve is a basic layout task Animate itself lacks.

## Decision

### Soft masks

1. **A mode on the mask layer, not a new layer kind.** `Layer::mask_mode` is `coverage` (today's
   masks: the fills' area, colour, opacity and strokes ignored), `alpha` (the artwork as drawn:
   its opacity reveals) or `luminance` (its brightness times its opacity reveals: white shows,
   black hides). It is saved only when not `coverage` (`#[serde(default)]`), so older files open
   unchanged and files without soft masks are byte-for-byte as before. Every existing mask rule
   (parents, folders, the stage showing the mask's edge, picking) stays as it is.
2. **Luminance is SVG's**: `0.2125 R + 0.7154 G + 0.0721 B` on premultiplied sRGB values
   (`luminanceToAlpha`), which is what browsers and the SVG mask default compute
   (`MaskMode::reveal`).
3. **The display list carries the mode per clip** (`DisplayList::clip_modes`, sparse: a clip past
   its end is coverage, so the many lists built by hand stay as they are). Mask artwork items are
   the same items as before; for a soft clip the renderers draw them with their paint.
4. **The CPU oracle** composites a soft mask's artwork src-over (fills, strokes, gradients,
   bitmaps, colour effects) onto a transparent band buffer and takes each pixel's opacity or
   luminance as the clip's coverage, then multiplies by the parent clip as before. Overlapping
   soft artwork composites rather than unites (two 50 % shapes give 75 %). Analytic tests pin
   opacity, luminance, linear gradient ramps, colour effects, strokes and nesting.
5. **The GPU stage** gets the mode in the `LEVEL_BEGIN` control word; a soft level shades its
   artwork with the same paint code and composites it per pixel, and `LEVEL_END` takes the
   opacity or luminance. Measured parity on the analytic soft-mask scene: mean error 0.00006 of
   a step, worst 1 step of 255 (RTX 5090, Vulkan).
6. **Exports**: SVG writes a soft clip as `<mask maskUnits="userSpaceOnUse">` with
   `mask-type="alpha"` (luminance is the default), its artwork drawn as it looks, wrapped in a
   group carrying its parent clip; a `clipPath` can only be clipped by a clip path, so a coverage
   clip inside a soft one is applied as `clip-path` plus `mask` where it is used. Lottie writes a
   track matte (`td` layer above, `tt` 1 alpha / 3 luma) with the mask's artwork for every layer
   it shows through. The HTML canvas draws the artwork into the mask's offscreen layer and, for
   luminance, turns each pixel's luminance into its opacity before `destination-in`.
7. **SVG import** keeps opaque masks as coverage masks (an Animate-style mask stays the simplest
   thing), and imports any other `mask` as a soft mask layer: luminance by default, alpha with
   `mask-type: alpha`, the content as drawn (gradients, strokes, opacity). The mask's region cuts
   unstroked artwork exactly in its own space; artwork that crosses it otherwise is kept whole
   and reported. A soft mask is preferred over a curved clip when both apply.
8. **Where artists find it**: Layer Properties shows *Mask type: Shape · Alpha · Luminance* on
   mask layers (Shape is the default and means what Animate's masks do); `layer.set_mask` and
   `layer.set_properties {mask}` do it headless.

### Text on a path

9. **The text block holds a copy of the path** (`TextBlock::on_path`: the path in the block's
   own space, the offset along it and whether it is flipped). Attaching copies the shape's first
   contour; the shape is left as it is (nothing is deleted; artists delete or hide it if they
   want). We chose a copy over a live link to the shape: a link would make text depend on
   another element's lifetime, order and keyframes, and every edit to either would need to
   relayout the other; a copy keeps "time is a function" and rendering untouched.
10. **Layout by arc length, outlines stored as today.** The text crate lays glyphs out on one
    line (line breaks become spaces) and sets each glyph's middle at its distance along the path
    (`CurveWalk`, on the Bézier curve itself), turned to the path's direction there, its baseline
    on the path, standing on the path's left (above a path drawn left to right). The block's
    alignment decides the anchor (left: starts at the offset, centre: centred on it, right: ends
    there), as SVG's `text-anchor` with `startOffset`. On a closed path glyphs wrap round; on an
    open one glyphs whose middle falls past the ends are left out, as SVG does. *Flip* walks the
    path backwards (SVG 2's `side="right"`). The outline is stored with the block, so rendering,
    every export and Break Apart never need the font.
11. **The box is the path and the glyphs**: the block's space is moved so both start at the
    origin (text boxes always start there), and the element's transform moves the other way, so
    nothing moves on the stage when the text or its size changes.
12. **Commands**: `text.attach_path` (the selected text and a selected shape, a shape id, or the
    shape at a stage point), `text.path` (offset, flip, release) and `text.release_path`. Text
    edits, alignment, Find and Replace keep text on its path. Text on a path is static text
    (fields can't be on a path). The shell: Properties › Path › *Attach to Path* waits for a
    click on a shape (Esc cancels); then Offset, Flip and Release; the selected text shows its
    path.
13. **SVG import**: `textPath` (`href` to a path or basic shape, with that element's transform)
    becomes text on a path with its `startOffset` (length or percentage), `side` and
    `text-anchor`. Per-character `x`, `y`, `dx`, `dy` and `rotate` are kept where simple: a
    character with its own position, shift or angle starts a block of its own (rotated about its
    baseline origin), the next continuing after the text before it. Values running into nested
    `tspan`s and shifts in centred or right-anchored text are approximate and reported.

## Consequences

* Soft edges, vignettes and fades are one gradient on a mask layer away, on the stage, in Test
  Movie and in every export; SVG files with soft masks import editable.
* Soft masks cost a band buffer per clip per band where used (as clips already did) and shade
  their artwork once per band; coverage masks cost exactly what they did.
* Text on a path is editable text everywhere; the path it follows is its own copy.
* Not yet: soft masks inside soft mask artwork composited as groups (a symbol fading inside a
  soft mask fades shape by shape), mask modes in XFL import/export (Animate has no equivalent
  field), editing the copied path in place (release, reshape the shape, attach again), several
  lines on a path, vertical text and per-character styles on a path; per-character positions
  stay positions on import (editing the text lays it out again normally).
