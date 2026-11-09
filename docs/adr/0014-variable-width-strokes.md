# ADR 0014: Variable-width strokes

* Status: accepted (2026-11-09)
* Relates to: roadmap M7 (variable-width strokes and brush library), ADR 0002 (render
  pipeline), ADR 0003 (file format), ADR 0004 (destructive actions)

## Context

Hand-drawn line work rarely has one width: lines taper at their ends and swell where the hand
presses. Animate covers this with width profiles and the Width tool: width points along a
stroke, each with a width on either side, edited by dragging on the stage. Until now an
AniCraft stroke had a single width. The brush tool and a brush library come later and will
build on the same model.

## Decision

1. **Model: a profile on the stroke, relative to its width.** `Stroke` gains an optional
   `WidthProfile`: up to 64 width points, each a position `t` along the subpath (0 at its start,
   1 at its end, by arc length in the shape's own space) and a half-width factor on the left
   and on the right (1 = the nominal width set in Properties, up to 16). Factors rather than
   pixels, so changing the stroke width scales the whole profile, as in Animate. Left is the
   left of the direction of travel as seen on the stage. A shape's profile applies to each of
   its subpaths. A uniform profile is stored as no profile, so `None` means "one width" and
   older files load unchanged (`#[serde(default)]`, no format version change). Points are
   sanitized on every way in (files, commands): sorted, merged when closer than 0.001, clamped.
   `Stroke` stops being `Copy`; its profile is an `Arc<[WidthPoint]>`, so copies stay cheap.
2. **Interpolation: monotone cubic (Fritsch–Carlson).** Between points the widths are smooth
   and pass through every point, but never overshoot them: a taper never bulges past its end
   and never goes negative. Outside the first and last point the width holds.
3. **Rendering in the CPU oracle: positive pieces, not an offset outline.** Like the existing
   stroker, a variable stroke is a set of positively wound polygons filled non-zero, so their
   union is exact without computing a clean offset curve (which self-intersects on the inside
   of tight bends). Each flattened segment is a trapezoid perpendicular to it with its two end
   points' widths per side; the outer side of each bend gets a round join of that side's
   width; open ends get a half-disc cap across the cross-section (so a one-sided width stays on
   its side). Segments are subdivided to about 256 samples per subpath (¾–8 px apart) so the
   widths follow the profile on long straight segments. Arc length is measured on the
   flattened points mapped back to the shape's space, so widths stay put at any zoom or
   transform; a mirroring transform swaps the sides. Thin strokes fade by their widest part,
   like uniform strokes. Uniform strokes keep their existing code path, pixel for pixel.
4. **Vector exports write what the renderer fills.** SVG and Lottie can't vary a stroke's
   width, so `outline_variable_strokes` replaces each variable stroke with a filled item of
   the same polygons, right after its shape (same clip, group and colour effect). Nothing is
   reported as lost. (A cleaner single outline for hand editing in other tools is a later
   refinement.)
5. **Tweens.** Shape tweens blend profiles: point by point when both have the same number of
   points (exact at the keyframes), otherwise by blending both profiles' widths at every point
   of either. Uniform blends as all-ones, so a stroke can taper in over a tween.
6. **Everything is a command.** `stroke.set_profile` (a preset name or a point list),
   `stroke.set_width_point` (add, resize or slide one point; by `t` or by a stage point),
   `stroke.delete_width_point`, `stroke.width_at` (what is under a point, for tools and
   agents), and a `profile` param on `element.set_stroke`, the drawing commands and
   `paint.ink`. The drawing tools have a current profile (tool options), as Animate does.
7. **The Width tool (U)** is view state over those commands: hovering a stroke shows its points
   (a cross-section with a handle on each side and a dot on the line), dragging a handle sizes
   both sides (keeping their difference; Alt sizes one side), dragging the dot slides the point
   along, and dragging anywhere else on the stroke adds a point there. Each drag is one
   gesture, so one undo step. Delete removes the selected point; routine and undoable, so no
   confirmation (ADR 0004).
8. **Built-in profiles are our own.** Uniform, Taper both ends, Taper start, Taper end, Bulge
   and Pinch, defined from scratch; Animate's profile presets are not copied.

## Consequences

* Variable strokes cost about the same to render as uniform ones (measured: 300 curved strokes
  at 1280×720, median 2.9–3.2 ms uniform, 2.9–3.2 ms taper, 3.3–3.4 ms bulge, 32 threads).
* Exported SVG/Lottie outlines are many small closed subpaths: correct and compact enough, but
  not pleasant to hand-edit in another vector tool.
* Picking and bounds use the stroke's reach (nominal half-width × widest factor), a
  conservative superset; the Width tool tests against the actual width at the nearest point.

## Not yet

XFL import and export of Animate's width data (the format is not publicly documented), saving
custom profiles to the profile menu, pressure-driven widths (with the brush tool), dashes and
other stroke styles, and a cleaner single-outline SVG export.
