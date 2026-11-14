# ADR 0034: Eraser and Eyedropper

* Status: accepted (2026-11-09)
* Relates to: ADR 0033 (Brush tool), ADR 0031 (merge drawing), ADR 0018 (variable-width
  strokes), ADR 0004 (destructive actions)

## Context

Animate's Brush has two companions that artists reach for constantly:

* **Eraser (E).** It sweeps a tip over the drawing and takes away what it covers, on the active
  layer only. Its modes choose what it takes: *Erase Normal* (lines and fills), *Erase Fills*,
  *Erase Lines*, *Erase Selected Fills* and *Erase Inside* (only the fill where the stroke
  starts). The *Faucet* option deletes a whole fill or line with one click. Drawing objects and
  symbols are not erased. Double-clicking the tool erases everything on the stage.
* **Eyedropper (I).** A click on a fill picks its colour, gradient or bitmap; a click on a line
  picks its colour, width and style. The tool then becomes the Paint Bucket or the Ink Bottle,
  and with shapes selected the picked style goes onto them.

We learned the behaviour from Adobe's public help pages and by watching the program. Nothing
was copied.

## Decision

1. **The eraser is a fill in the merge that comes out empty.** The eraser's outline is the
   Brush's swept tip (`anicraft_geom::brush::outline`, the same tip model: size, round or
   square, angle, roundness, pressure), with almost no smoothing (a third of a screen point).
   It goes into the planar merge (ADR 0031) as one more item with a reserved fill style. The
   merge already gives each face the topmost fill over it and hides a line that a later fill
   covers on both sides, so whatever sits under the eraser is replaced by "erased", which is
   then dropped. The rest comes back as ordinary raw shapes: one element per fill region and
   per stroke run, lines rejoined through the cut points. There is no new geometry and no
   format change.

   The mode is only **where the eraser goes in the stack**, among the raw shapes its bounds
   touch:

   | Mode | Bottom to top |
   | --- | --- |
   | Erase Normal | everything, eraser |
   | Erase Fills | fills, eraser, lines |
   | Erase Lines | lines, eraser (the fills stay out of the merge, exactly as they are) |
   | Erase Selected Fills | selected fills, eraser, other fills, lines |
   | Erase Inside | the fill under the stroke's first point, eraser, other fills, lines |

   Fills don't overlap in a merged drawing, so a fill above the eraser keeps its whole area.
   Lines above it are only cut at the eraser's edge and joined again. In Erase Lines the fills
   are left out of the merge, because a fill above a line that ends inside it would hide that
   line.

   *Rejected:* a general boolean subtraction on paths. The merge already is that operation for
   raw shapes, with snapping, exact sub-curves and the selection model. A second engine would
   disagree with it at the edges.

2. **Only raw shapes are erased.** Drawing objects, gradient or bitmap fills and width-profiled
   strokes (which the merge keeps whole, ADR 0031), symbols and text are left alone, as in
   Animate. AniCraft has no "edit inside a symbol" mode yet, so symbols are never erased.

3. **Nothing erased, nothing changed.** A stroke that only comes near the artwork must not
   create a keyframe or an undo step. The command compares the total fill area and line length
   of the shapes it took in with what came out. Areas are exact (Green's theorem with
   five-point Gauss–Legendre nodes, exact for the degree-5 integrand of a cubic) and lengths use
   the same nodes. A shape that the merge returns unchanged comes back as the same sub-curves,
   so it measures the same. If nothing changed, the document is put back as it was.

4. **Commands** (everything is a command, so it works over the CLI and MCP):
   * `draw.erase {points, size?, shape?, angle?, roundness?, mode?, unit?}`: one stroke is one
     undo step. It returns `erased: false` and a reason when there was nothing to erase.
   * `draw.faucet {x, y, slop?}` deletes the raw fill or line under the point on the active
     layer. It refuses objects, symbols and text with a message saying how to delete them.
   * `draw.erase_all {}` clears every visible, unlocked layer's keyframe at the playhead
     (raw shapes, objects, symbols, text). Tweens on those keyframes are removed, since nothing
     is left to tween. Locked and hidden layers are kept. ADR 0004: this is a large undoable
     deletion, so `confirm.rs` asks first, with the counts. The engine doesn't require
     `"confirm": true` (undo brings it back). The shell runs it on a double-click of the Eraser
     tool button and from "Erase Everything…" in the Eraser's options.
   * `paint.pick {x, y, other?, slop?}` (non-mutating, like `draw.object_drawing`) picks from
     the topmost item drawn under the point on any visible layer, including locked ones.
     Picking reads; it changes nothing. On a line it picks the colour, the width as drawn
     (times the transform's scale) and the width profile (a preset when it matches one,
     otherwise kept as a custom profile). In a fill it picks the paint. `other: true`
     (Shift-click) puts the picked colour into the other swatch only.
   * `paint.apply_style {fill?, stroke?}` gives the selected shapes the fill swatch (shapes
     with a fill) and the stroke settings (shapes with a line, and drawing objects without
     one). Raw fills don't gain strokes, because their lines are elements of their own.

   All of these are in the existing `draw` and `paint` namespaces, which plug-ins may call.

5. **A picked gradient or bitmap stays where it is on the stage.** `ToolStyle` gains
   `fill_paint: Option<Paint>`, in stage space, with `fill` kept as its first colour for the
   swatch. It also gains `custom_profile` for a picked profile that is none of the presets.
   Whatever uses the fill swatch maps the paint into the shape's local space
   (`Paint::transformed`): the Paint Bucket, `paint.apply_style`, and the shape tools. So the
   gradient carries on across the shapes it is poured into, which is Animate's *Lock Fill*
   after the Eyedropper. Choosing a colour in the swatch drops it. Linear gradient ends and
   radial centres are mapped exactly; a radial radius scales by the mean scale, so a skewed or
   unevenly scaled source picks a near match.

6. **The tools** are view state over those commands.
   * **Eraser (E, after the Brush).** A drag collects samples like the Brush and shows the
     sweep in the stage colour as it goes. Letting go sends one command; a click erases a dab.
     `[` and `]` step the size.
   * **Eraser options:** Faucet, Size, Shape, Mode and Erase Everything….
   * **Eyedropper (I, after the Ink Bottle).** A click picks. With shapes selected, it also
     applies the picked style. It then becomes the Paint Bucket (after a fill) or the Ink
     Bottle (after a line). Shift-click picks into the other swatch and stays.
   * The Fill row in the tool options says when the swatch holds a gradient or a bitmap.
   * The icons are original, drawn on Lucide's grid.

## Consequences

* Measured in release, 32 threads, median of 7. The eraser's outline is computed once per
  stroke, when it ends, and the merge only takes in what its bounds touch.

  | Stroke | Raw shapes | Median |
  | --- | --- | --- |
  | 1000 px wave, size 20, through a grid of 40 outlined rectangles | 200 | 6.4 ms |
  | Same, Erase Lines | 200 | 6.0 ms |
  | Same, size 80 | 200 | 4.0 ms |
  | Same wave through 30 brushed waves | 30 | 22 ms |

* Tested:
  * A 20 wide band through a 200 × 100 rectangle leaves 18 000 units², within 40.
  * Each mode's area and line length are analytic.
  * The faucet; objects and symbols untouched, with no undo step.
  * Erase Everything asks and keeps locked layers.
  * Solid, gradient and line picks; picking into the other swatch; applying to the selection.
  * The every-command fuzz and the file round trip.
* The eraser cuts lines by their centre line. A thin eraser grazing the edge of a thick line
  without crossing its centre takes nothing, and the cut ends keep their round caps.
* The live preview paints the stage colour over everything, including objects and other layers
  that the stroke will keep.

## Not yet

* Erasing inside symbols (needs an edit-in-place mode).
* Erasing gradient, bitmap or width-profiled shapes (needs them in the merge).
* Eraser pressure.
* A preview through the merge.
* The Eyedropper picking a bitmap's pixel colour (it picks the bitmap fill itself) and text
  styles.
* An eyedropper cursor.
* Showing a gradient in the toolbar swatch.

## Note (2026-11-13)

ADR 0093: width-profiled lines and gradient fills erase, each piece keeping its widths or its
part of the gradient; inside an open symbol or drawing object the Eraser erases its shapes, and
on the stage `draw.erase` reports what it kept (`kept`) with a hint.
