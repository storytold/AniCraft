# ADR 0080: Drawing parity leftovers

* Status: accepted (2026-11-12)
* Relates to: ADR 0031 (merge drawing), ADR 0033 (Brush tool), ADR 0045 (symbols in place),
  ADR 0073 (groups and Combine Objects), ADR 0004 (destructive actions), ADR 0002 (render
  pipeline)

## Context

The user guide's drawing chapter listed everyday Animate drawing features AniCraft still
lacked: PolyStar's Polygon option, the Brush's Paint Selection and Paint Inside modes, entering a
drawing object with a double-click, Envelope (and Delete Envelope), and gradient and bitmap
strokes. Artists coming from Animate reach for these without thinking. We learned the behaviour
from Adobe's public help pages and by watching the program; nothing was copied.

## Decision

### 1. PolyStar options

`draw.star` takes `shape` (`star` or `polygon`), `sides` (3 to 64; `points` still works) and
`point_size` (0 to 1). The tool shows them inline in its options (Shape, Sides or Points,
Point size), not as Animate's modal Tool Settings dialog: our tools keep their settings in
Properties, and the principles ask for no modal interruptions. The row is called **Shape**
because **Style** already names the line style just above it.

**Point size**, our definition: the inner corners lie at `r · size · cos(π/n)`. At 1 they are on
the edges of the polygon through the tips, so the star *is* that polygon; at 0 they meet in the
centre (needle points); between, the depth is proportional. Animate documents only the range and
"closer to 0, narrower points", which this matches. An explicit `inner` radius still wins, so
older scripts draw what they drew. The outline goes through `add_shape`, so it merges or is a
drawing object exactly as the other shape tools (ADR 0031). Geometry: `Path::polystar`,
`polystar_corners`, `star_inner_radius` in `anicraft_geom`, tested analytically (polygon area
n/2 · r² · sin(2π/n), star area n · r · r_in · sin(π/n)).

### 2. Paint Selection and Paint Inside

ADR 0033 left them out because the merge couldn't clip paint to a region. It can now:
`planar::intersect(subject, rule, region)` is `merge_cut` of the subject alone, keeping the
pieces inside the cutter; `planar::union` merges fills of one style into non-overlapping loops.
Both are exact to the merge's snap (0.2 units), like everything else on the planar map.

* **Paint Selection**: the brush outline is clipped to the union of the selected fills on the
  active layer. Afterwards the selection is what lies in that region (the fills' pieces and the
  paint), so the next stroke paints there too, as in Animate.
* **Paint Inside**: clipped to the topmost fill under the stroke's first point. Started on empty
  stage, it is clipped to the empty area the lines and fills close off there (the Paint
  Bucket's `enclosed_area`); if that area is open, it paints only empty areas (Paint Behind).
* Both go **under the lines** (`Order::UnderLines`): clipped paint never covers a line.
* Off Object Drawing only raw fills count as regions (merged paint lies under drawing objects,
  so painting "inside" one would be hidden; it is edited inside it, §3). On Object Drawing any
  fill counts, and the clipped paint is a drawing object.
* Nothing painted (no selection, or the stroke misses the region) changes nothing: no keyframe,
  no undo step, `{"painted": false, "reason": …}`; the shell says why in the status bar.

The live preview still shows the raw sweep (ADR 0033); the clipped result appears on release.

### 3. Editing a drawing object in place

A double-click (or Ctrl+E, `symbol.edit`) opens a drawing object as groups open (ADR 0073), a new
editing level `Opened::Drawing`. On opening, the object is replaced *in place* by a group of its
one raw shape (same id, name, transform and colour effect; the shape unmoved inside), so every
command, merge drawing included, works inside it unchanged, and its raw shapes merge only with
each other. The picture doesn't change, so opening is no undo step.

Closing turns it back: one shape left becomes one drawing object again (its id, name and place;
the transforms compose); nothing left removes it; several pieces stay a **group of raw shapes**,
which is what Combine Objects' Union already makes of shapes of several colours (ADR 0073). A
drawing object of several pieces needs no new element kind. Opening and closing without an
edit restores the document and its state id exactly (the level keeps them), so it never looks
unsaved. Undo inside goes back inside (the level is in the snapshot, ADR 0045).

*Rejected:* a `drawing` flag on groups, or a multi-shape drawing object kind: a second
representation of an existing concept, in the file format, every exporter and the importers,
for no visible difference.

### 4. Envelope

**Modify › Transform › Envelope** opens an envelope on the selected shapes and drawing objects
(not symbols, text, groups or bitmaps, as in Animate). The bend is a **bicubic Bézier patch**
(`anicraft_geom::envelope`): 4 × 4 control points over the selection's outline bounds. At rest
they are a third of the way apart and the map is exactly the identity (Bernstein polynomials
reproduce linear functions; tested command for command). The shell shows the 12 boundary
handles: squares for corners (a corner's two tangents move with it) and circles for tangents.
The four inner points follow the boundary as a Coons patch, so a side bends the inside smoothly.

The image of a cubic under the patch is degree 9, so each segment is halved until the cubic
through its mapped control points is within 0.05 units of the true image at its quarter points
(at most 10 halvings, 200 000 commands). A line that stays straight (any affine part of the
mesh) stays one line.

**Baked, as Animate bakes it.** The shapes stay ordinary paths; there is no envelope in the
model, so there is nothing for Delete Envelope to delete, and it isn't added. Each move bends the
shapes *as they were when the envelope opened*, so moves never compound their approximation.
The session keeps, with each mesh, the shapes it made, and finds the mesh whose shapes are on
the stage: undo and redo restore the mesh with the shapes, with no change to the undo
snapshots. A drag (one gesture, one undo step) keeps only its last mesh. The envelope ends when
its shapes are no longer the selection or are edited otherwise; the shell then sends
`modify.envelope_close`, which merges bent raw shapes with the drawing they lie on (as after a
move). Gradients and bitmaps in the fills stay where they were (Animate doesn't bend them
either).

Commands: `modify.envelope` (opens; returns the mesh), `modify.envelope_move` (`points`, the
whole mesh, or `index`, `x`, `y`; one undo step), `modify.envelope_close`. They are in the
`modify` namespace, which plug-ins may already call.

### 5. Gradient and bitmap strokes: proposed, not in this slice

The CPU renderer path is straightforward: a stroke's outline is filled, so its shader could take
any `Paint` (`Shader::new(&Paint::Solid(st.color), …)` becomes the stroke's paint). The model
change would be `Stroke { paint: Option<Paint> }` beside `color` (`#[serde(default)]`; `color`
stays the solid colour older readers and the merge's style table use). But every consumer of
`Stroke::color` must carry it: the GPU stage's stroke path, SVG, HTML Canvas and Lottie export,
XFL import of `<stroke><LinearGradient>`, the merge's stroke styles, Ink Bottle, Eyedropper,
mixed-selection Properties and the colour picker. That is about 35 sites and its own golden
tests, so it is the next PR, not a corner of this one.

## Consequences

* Measured in release (median of 9, one thread):

  | Operation | Time |
  | --- | --- |
  | Brush, 1000 px wave, size 30, over an outlined oval, Paint Normal | 4.9 ms |
  | Same, Paint Inside (clip, then a smaller merge) | 4.7 ms |
  | Same, Paint Selection (union, clip, reselect) | 6.6 ms |
  | Envelope move on a 32-point star and an oval | 0.03 ms |

* Tests: PolyStar areas (analytic, merge and object), bad options; Paint Selection and Inside
  areas (a 100 × 20 band; a half-round cap plus a band), lines left whole, empty areas, no step
  when nothing paints; drawing objects opened and closed unchanged (document and state),
  merged only inside, kept as one object, removed when emptied, undo inside; Envelope identity
  at rest, an analytic bend (area + lift · width / 2), undo restoring the mesh, a 300-move drag
  as one step, moves that don't compound, refusals, raw shapes merging on close; `planar`
  union and intersection areas; the patch's identity, affine and tolerance properties. The
  `panic_hunt_every_command` fuzz covers the new commands and params.
* Saving while a drawing object is open saves it as a group of its pieces; it opens as a group.
* Warped artwork (Asset Warp, ADR 0079) neither opens in place nor takes an envelope: its warp
  keeps the outline as drawn on every keyframe of its layer, and either edit would fight it.
  The message says to edit its pins or remove the warp first.

## Not yet

Gradient and bitmap strokes (§5); envelope handles from the keyboard (the menu command and the
`modify.envelope_move` command work without a pointer, the handles don't take focus yet); an
envelope that remembers itself after it closes; the brush's live preview clipped as it paints.
