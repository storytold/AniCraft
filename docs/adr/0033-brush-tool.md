# ADR 0033: Brush tool

* Status: accepted (2026-11-09)
* Relates to: ADR 0031 (merge drawing), ADR 0018 (variable-width strokes), ADR 0004
  (destructive actions), ADR 0002 (render pipeline)

## Context

Animate's Brush tool (B) doesn't draw a stroke. It paints a **filled shape**: the area its tip
sweeps along the drag, in the fill colour. The tip has a size and a shape (round, oval or flat
at an angle, square), the width follows pen pressure where there is a tablet, and the hand's
wobble is smoothed. Paint modes decide what the paint covers on the layer:

* Paint Normal covers lines and fills.
* Paint Fills covers fills and empty areas but leaves lines.
* Paint Behind fills only empty areas.
* Paint Selection paints only inside the selected fill.
* Paint Inside paints only inside the fill where the stroke started.

Artists coming from Animate block in characters and backgrounds this way, and AniCraft had no
brush. We learned the behaviour from Adobe's public help pages and by watching the program.
Nothing was copied.

## Decision

1. **A brush stroke is an ordinary shape.** It is filled in the fill colour, with no stroke and
   the even-odd rule. Every system already handles it: renderer, GPU stage, exports, picking,
   Subselection, tweens, the file format. There is no new element kind and no format change.
   Off Object Drawing it is a raw shape and merges with the drawing (ADR 0031); on, it is a
   drawing object.

2. **The outline is the exact sweep, traced from a distance field**
   (`anicraft_geom::brush`, pure geometry).
   * **Centre line.** The samples are smoothed by the existing curve fitter (`fit_curve`, at a
     tolerance from the Smoothing option) and flattened. Each point takes the pressure found at
     the same fraction of the stroke's length.
   * **Tip space.** Everything is mapped by the inverse of the tip's transform (size,
     roundness, angle), so the tip becomes the unit circle or unit square.
   * **Distance field.** Between two samples, the tip (moving and resizing linearly) sweeps
     exactly the convex hull of the tip at both ends. The signed distance to that hull is
     `min over t of |x − c(t)| − r(t)` (L2 for round tips, L∞ for square ones). That is
     convex in `t` and has a closed-form minimum. The minimum over all segments is the true
     distance to the whole sweep outside it. Each segment is sampled only over its own bounds,
     and long segments are cut into short pieces.
   * **Tracing.** Marching squares (the Paint Bucket's tracer) gives the zero level. Linear
     interpolation is accurate well below a cell, because the field is a real distance. The
     loops are mapped back and fitted with cubics again, keeping corners.
   * **Resolution.** Cells are a sixth of the tip radius (an eighth for square tips), and at
     least 2.5 cells span the thinnest pressured part. The grid has at most 4 M cells; past
     that the cells grow, so cost and memory are bounded for any input.

   *Rejected:* an offset outline (left and right offset curves plus caps). It self-intersects on
   the inside of tight bends and where pressure changes quickly, and needs a robust boolean
   cleanup. *Also rejected:* unioning stamp polygons through the planar merge. It is exact, but
   costs grow with the overlaps (thousands of segments per stroke) and run into the merge's
   budget. The distance field gives one clean set of loops (outer edges and holes) with
   bounded work.

3. **Pressure.** Pen pressure scales the tip from 10 % (no pressure) to 100 % (full). It comes
   from egui's touch events (`force`), which winit fills from tablets. A mouse paints at full
   size. Points carry pressure as an optional third number, clamped to 0..1 (it's a
   measurement, so tablets and arithmetic may stray past the ends).

4. **Paint modes in this slice: Normal, Fills, Behind.** Each is a placement in the merge
   (`merge::Order`). The planar merge already gives each face the topmost fill and hides a line
   only under a later fill on both sides, so:
   * Normal adds the paint on top.
   * Fills puts it above the touched fills and below the touched lines.
   * Behind puts it under everything it touches; touching nothing, it goes to the bottom of the
     layer.

   As drawing objects (Object Drawing on), Behind inserts the object at the bottom of the
   layer; Normal and Fills add it on top. Paint Selection and Paint Inside need the paint
   clipped to a region (a boolean intersection), which the merge doesn't offer yet. They are
   the next slice.

5. **Everything is a command.** `draw.brush` takes:
   * `points` (`[x, y]` or `[x, y, pressure]`; one point paints a dab);
   * `size`, `shape` (round or square), `angle` and `roundness` (an oval or flat tip);
   * `smoothing` (0–100), `mode`, `color` (the fill colour by default) and `object`;
   * `unit` (stage units per screen point), so smoothing and accuracy follow the screen.

   Smoothing at 50 lets the centre line wander 3.3 screen points from the hand. The outline is
   fitted within a third of a point, or 1 % of the size for small brushes. One command is one
   undo step (ADR 0004: undoable, so no confirmation). It is in the `draw` namespace, so
   plug-ins may call it.

6. **The tool** (B, after Pencil) is view state over that command.
   * A drag collects samples (half a screen point apart) and shows the paint live: the hull of
     the tip at each pair of samples, opaque, drawn over the stage. Letting go sends one command;
     a click paints a dab.
   * The tip's outline follows the pointer.
   * Properties shows the tool options: Fill, Object drawing, Size, Shape, Angle, Roundness,
     Smoothing and Mode.
   * `[` and `]` step the size: by 1 below 10, 5 below 50, 10 below 200, then 50.

## Consequences

* Measured in release, 32 threads (the outline is computed once per stroke, when it ends):

  | Stroke | Median | Anchors |
  | --- | --- | --- |
  | 1000 px wave, size 10, mouse | 6.9 ms | 40 |
  | Same, with pressure (finer cells for the thin ends) | 43 ms | 43 |
  | Flat 45° tip, size 40 | 4.0 ms | 34 |
  | Square tip, size 100 | 1.4 ms | 60 |
  | Diagonal across a 1280 × 720 stage, size 3 | 18.9 ms | 89 |

* A straight round stroke's area is within 1 % of length × size + a circle (tested).
* Square tips are square-ish. Tracing softens their corners by under 1/30 of the size, and the
  fit keeps them as corners.
* Building it found a bug in `fit_curve`: a closed loop whose first point wasn't a corner got a
  straight edge across it. That also affected Paint Bucket fills. It is fixed, with a test.
* The live preview is drawn from the raw samples, so it can differ slightly from the smoothed
  result. A translucent fill colour previews opaque.

## Not yet

Paint Selection and Paint Inside; a brush library (custom tip shapes, tapers, textures); a
pressure curve and tilt; brush strokes with a stroke colour (Animate's Paint Brush, Y, which
draws width-profiled strokes, ADR 0018); gradients and bitmap fills as brush paint; the live
preview through the merge; the Eraser (shares this sweep) and Eyedropper, which come in the next
PR.

## Note (2026-11-11)

ADR 0068 adds the brush library (parameter presets with tapers, dabs and a ragged edge),
pressure curves, Velocity, and the Paint Brush (Y), which draws width-profiled strokes through
`draw.brush`'s `as_stroke`. Without the new params `draw.brush` paints exactly as described
here. Tilt waits for the toolkit to report it.
