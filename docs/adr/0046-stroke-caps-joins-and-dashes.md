# ADR 0046: Stroke caps, joins, miter limit and dashes

* Status: accepted (2026-11-10)
* Relates to: roadmap M7, requirements (Fills and strokes: caps, joins, miter limit, dashes),
  ADR 0002 (render pipeline), ADR 0003 (file format), ADR 0018 (variable-width strokes),
  ADR 0021 (SVG import), ADR 0031 (merge drawing), ADR 0035 (XFL corpus; supersedes its
  "caps and joins other than round" report)

## Context

Every AniCraft stroke had round caps and round joins and was solid. Artists coming from Animate
expect Properties › Stroke to offer Cap (None, Round, Square), Join (Miter, Round, Bevel) with
a Miter limit, and a Style (Solid, Dashed, Dotted, Ragged, Stipple, Hatched). SVG files use
`stroke-linecap`, `stroke-linejoin`, `stroke-miterlimit` and `stroke-dasharray`; the SVG and XFL
importers reported all of these as drawn differently.

## Decision

1. **Model: a `StrokeStyle` on every `Stroke`** (`anicraft_doc::stroke_style`): `cap`, `join`,
   `miter_limit` and an optional `dash`. Its default is what every stroke was before (round,
   round, solid; miter limit 3, Animate's default) and is written as nothing
   (`#[serde(default, skip_serializing_if)]`), so older files open unchanged and unstyled
   strokes are stored byte for byte as before. No format version change. Styles are sanitized on
   every way in (files, commands, importers): miter limit in 1..=100, dash lengths finite and in
   0..=10 000, at most 16 entries.
2. **Dashes are a general pattern; dots are their own kind.** `Dash::Dashed { lengths, offset }`
   holds SVG's model: on/off lengths starting with a dash (an odd count repeats, as in SVG; no
   gap at all means solid), and how far into the pattern each subpath starts.
   `Dash::Dotted { spacing }` is Animate's dotted line: round dots one stroke width across,
   `spacing` apart edge to edge, drawn as zero-length dashes with round caps whatever the cap
   (so they export exactly to SVG, canvas and Lottie). Lengths are in the shape's own units
   like the width, so scaling a shape scales its dashes (`Stroke::scaled`), and the pattern
   restarts on every subpath, as in SVG. Properties edits the first dash and gap; a longer
   imported pattern keeps the rest.
3. **Out of scope: Animate's Ragged, Stipple and Hatched styles.** They are randomised textures
   with no open description and no equivalent in SVG, canvas or Lottie. The XFL importer draws
   them solid and says so.
4. **Rendering in the CPU oracle: the same positive-pieces stroker, generalised.** A styled
   stroke is a set of positively wound polygons filled non-zero (ADR 0002, 0018): a trapezoid
   per segment, a join on the outer side of each bend, caps on open ends. Butt caps add nothing;
   square caps carry the cross-section half its thickness past the end; round caps are the
   existing half discs. Miter joins fill to where the outer edges meet,
   `p + (n₀ + n₁) / (1 + n₀·n₁) · w`, when that is within the limit (SVG's definition: the
   point's distance over the half-width, `1 / cos(θ/2)` for a turn θ, at most the limit);
   otherwise, and for bevel joins, the corner is cut straight across. Dashes cut each subpath by
   arc length into open pieces stroked with the caps and joins, so a dash bends round a corner
   with the stroke's join. Round, solid strokes keep the existing code paths, pixel for pixel.
5. **Dash lengths are measured along the output lines, scaled like the width** (by the mean
   scale of the transform), the same rule the SVG and canvas exports already use for widths.
   The exports therefore write `stroke-dasharray` and `setLineDash` exactly as the renderer
   draws them; under a non-uniform scale the pattern follows the mean scale, as the width does.
   A width profile still varies by position along each whole subpath, so a dash takes the
   widths of its place (dashed tapers work).
6. **Patterns finer than a pixel draw solid, faded by their ink.** At small zooms a dash
   pattern can repeat many times per pixel; the renderer then draws the line solid at the share
   of it that is on (as it already does for strokes thinner than a pixel). Every stroke is cut
   into at most 200 000 dashes; past that, the rest of a subpath draws solid. Vector outlines
   (exports of width-profiled strokes) always get the exact dashes.
7. **The GPU stage** draws the oracle's prepared polygons (ADR 0019), so styles need nothing of
   their own there; the parity scenes include caps, joins, dashes and dotted tapers.
8. **Exports.** SVG writes `stroke-linecap`, `stroke-linejoin`, `stroke-miterlimit` (miter
   joins), `stroke-dasharray` and `stroke-dashoffset`; HTML canvas sets `lineCap`, `lineJoin`,
   `miterLimit`, `setLineDash` and `lineDashOffset` from a `k` entry written only for styled
   strokes; Lottie writes `lc`, `lj`, `ml` and `d` (dash, gap…, offset). Width-profiled
   strokes stay filled outlines (ADR 0018), now with their caps, joins and dashes. Default
   strokes are written exactly as before.
9. **Imports.** SVG reads the four properties and `stroke-dashoffset`, inherited, from
   attributes and inline styles, with SVG's own defaults (butt, miter, limit 4); an invalid
   dash list draws solid, as the SVG specification says. XFL reads `caps`, `joints` and
   `miterLimit` and the `DashedStroke` (`dash1`, `dash2`) and `DottedStroke` (`dotSpace`)
   elements, from public descriptions of the format and our own corpus.
10. **Everything is a command.** `stroke.set_style` sets any of `cap`, `join`, `miter`,
    `style` (solid, dashed, dotted), `dash`, `dash_offset`, `dot_spacing` on the selection,
    leaving the rest as it was; a new pattern is sized by the stroke (dashes three widths, gaps
    two; dots one width apart). The same params work on `element.set_stroke`, the drawing
    commands and `paint.ink`; the drawing tools have a current line style (tool options), the
    Ink Bottle gives it, and the Eyedropper picks it (dashes scaled as drawn).
11. **Tweens.** Shape tweens blend the miter limit and dash lengths when both ends have the
    same kind and count; caps, joins and different kinds hold the start's until the end
    keyframe.
12. **Merge drawing** keeps the style on every piece a raw stroke is cut into; equal styles are
    one style to the merge, so differently dashed lines stay apart. The dash pattern restarts on
    each piece.

## Consequences

* Styled strokes cost no more than round ones (measured: 300 curved strokes at 1280×720,
  32 threads, median 2.5 ms uniform round, 2.2 ms square caps with miter joins, 1.8 ms dashed,
  1.9 ms dotted, 2.2 ms dashed taper).
* A dashed raw stroke cut by merge drawing restarts its pattern at each cut, so the dashes can
  shift where lines cross.
* Bounds and picking use the stroke's reach, now including miter points (up to the limit) and
  square caps; picking treats a dashed line as solid, so clicking a gap still selects it.

## Not yet

Ragged, stipple and hatched styles; custom dash patterns of more than one dash and gap in
Properties (they are kept and editable through commands); Animate's stroke Scale (normal,
horizontal, vertical, none) and hairlines; a dash pattern that continues across subpaths;
dashed strokes in XFL export (there is no XFL export yet).
