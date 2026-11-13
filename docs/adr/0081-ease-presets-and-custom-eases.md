# ADR 0081: Ease presets, custom eases and saved eases

* Status: accepted (2026-11-12)
* Relates to: roadmap M7 (graph-based easing presets library) and M2 (Motion Editor), ADR 0016
  (motion tweens; eases as `Easing`), ADR 0035 (XFL corpus; one-segment custom eases), ADR 0003
  (file format), ADR 0004 (destructive actions), ADR 0028 (HTML canvas export), ADR 0040
  (interface languages)

## Context

Tweens had eight eases: linear, quad and cubic in/out/in-out, the Classic slider and one cubic
Bézier. Artists coming from Animate expect its ease library (Classic Ease with a strength, and
Quad, Cubic, Quart, Quint, Sine, Back, Circ, Bounce and Elastic, each In, Out and In Out), a
custom ease drawn as a curve of several segments, eases they can save and reuse, classic tweens
eased property group by property group, and the same choices in the Motion Editor. Animate files
carry custom eases of several segments and per-property curves; ADR 0035 drew those with the
slider's ease and reported them. Behaviour was learned from Adobe's public help pages and from
the public XFL descriptions; the ease formulas are the classic public easing equations.

## Decision

1. **Presets are pure functions of t, one enum.** `Easing` keeps its unit variants (the file and
   command names, `"QuadIn"` … `"ElasticInOut"`) and adds the 21 new ones, so files from before
   read unchanged. Each family defines its ease in, `In(t)`; `Out(t) = 1 − In(1 − t)` and
   `InOut` is In over the first half and Out over the second (Quad and Cubic in-outs are the
   same curves as before). Back overshoots by about 10 % (c = 1.70158); Elastic is
   `−2^(10t−10)·sin((10t − 10.75)·2π/3)`; Bounce is four parabolas. Values may leave 0..1;
   time never does (`t` is clamped).
2. **A custom ease is a curve of Bézier segments, stored by value.** `Easing::Custom(Arc<
   CustomEase>)`: 2 to 64 points from (0, 0) to (1, 1), each with in and out handles (offsets)
   and a smooth flag. The type keeps itself valid however it is made (a file, a command, an
   edit): non-finite points go, points sort by x, longer curves are thinned evenly, the ends are
   pinned, values stay within ±10, and every handle's x lies between its point and the
   neighbour it reaches towards. That last rule is what keeps time running forwards: a
   segment whose inner control points lie between its ends in x has a non-decreasing x(s)
   (`(u − v)² ≤ u(1 − v)` for any `0 ≤ v < u ≤ 1`), so solving x(s) = t has one answer, found
   by Newton steps inside a shrinking bracket. Edits are methods that return a new valid curve
   (`insert_at` splits a segment by de Casteljau so the shape is unchanged; `move_point` keeps
   x between the neighbours; `move_handle` turns a smooth point's other handle; `set_smooth`;
   `remove`). `Arc` keeps copying a tween cheap; `Easing` is no longer `Copy`.
3. **Classic tweens may ease property groups separately.** `Tween.groups` holds the groups with
   their own ease (Position, Rotation, Scale, Colour, Filters, as Animate's custom ease offers);
   a group without one follows the tween's. `evaluate` computes one eased progress per group
   (`Progress`) and interpolates position, rotation and skew, scale, colour effect and filters
   each with its own; outlines, fills and strokes follow the tween's ease. With no groups the
   result is bit for bit what it was. Shape tweens keep one ease, as in Animate. Filters now
   tween in classic tweens when both keyframes list the same kinds in the same order (numbers
   and colours travel; switches and quality hold), which the Filters group needs.
4. **Saved eases live in the document, and applying one copies it.** `Document.eases`: named
   eases (any kind, usually custom curves), each name once ignoring case, at most 256. A tween
   given a saved ease gets its curve, not a reference, as Animate's presets do: evaluation stays
   a function of the layer alone, a deleted or edited saved ease never changes a tween behind
   the artist's back, and copied frames and pasted symbols need nothing extra. Menus show a
   tween's curve by the saved name it equals. Deleting a saved ease is a routine, undoable edit
   (ADR 0004): no tween loses anything.
5. **Everything is an `ease.*` command, addressing one ease.** The target is a tween's ease (or
   one group's, `group`), a motion-tween property's segment (`property`), or a saved ease
   (`saved`). `ease.info`, `ease.set`, `ease.add_point`, `ease.move_point`, `ease.move_handle`,
   `ease.delete_point`, `ease.convert_point`, `ease.separate`, `ease.save`, `ease.delete`,
   `ease.rename`, `ease.list`, `ease.copy`, `ease.paste`. Point edits turn a preset into the
   curve that draws it (`Easing::to_custom`: exact for linear, Classic, Bézier, Quad, Cubic,
   Back and Bounce; Circ as two circular arcs; Quart, Quint, Sine and Elastic fitted by value and
   slope, all within 0.5 %). `timeline.set_easing` (with `group`) and `motion.set_ease` take the
   same ease params, saved names included. Plug-ins may use every `ease.*` command except copy
   and paste (the clipboard is the artist's).
6. **The interface.** One ease menu everywhere (timeline header, Properties › Frame, Motion
   Editor): No Ease, Classic Ease, the families as curve thumbnails In / Out / In Out, the saved
   eases, Edit Ease…. The **Custom Ease** window (non-modal) shows the graph, overshoot
   included: click the curve to add a point, drag points and handles (one undo step per drag),
   double-click for smooth/corner, right-click to delete; the selected point's time and value
   can be typed; with the graph focused `[` `]` pick a point, the arrows move it, `S`, `+` and
   Delete do the rest, taken before the app's shortcuts so Delete never reaches the stage. It
   also turns property groups on, saves, copies, pastes and resets. A named preset shows as its
   curve alone; editing it makes it Custom. The Motion Editor keeps its handles for eases one
   Bézier draws and opens the window for the rest.
7. **XFL custom eases import exactly.** A `<CustomEase>` of 3n + 1 points becomes a cubic Bézier
   (one segment) or a custom ease; with `useSingleEaseCurve="false"` each target (`position`,
   `rotation`, `scale`, `color`, `filters`) becomes that group's ease and `all` (or the slider)
   the tween's. What can't be read is reported: a missing curve, a broken point list, an
   unknown target, ends not at (0, 0) and (1, 1), more than 64 points (thinned). The corpus'
   tweens package gains a per-property layer with a three-segment position curve, with golden
   frames and colour probes.
8. **Exports go through `evaluate`.** Lottie bakes every frame (its own eases are one cubic per
   key pair, which can't draw a multi-segment, Bounce or Elastic curve exactly), and the HTML
   canvas page plays the frames AniCraft evaluates; a parity test runs a custom, per-group and
   Elastic movie in the page's JavaScript player against the Rust player step by step.
9. **The file format stays version 3.** `eases`, `groups` and the new `Easing` variants are
   additive (`#[serde(default)]`, skipped when empty). Older files open unchanged. An older
   build ignores saved eases and property groups, but refuses a file that uses one of the new
   eases (it can't read the ease), so such files need this build.

## Consequences

* Ease presets, custom curves and saved eases are one model across classic tweens, motion
  tweens, commands, files, XFL and every export.
* A saved ease edited later doesn't update the tweens that used it; Animate behaves the same.
* Not yet (later PRs): an "update every tween using this ease" action; ease presets shared
  between documents (a user library in the preferences); Lottie export that maps eases one
  Bézier can draw onto Lottie's own eases instead of baking; per-property eases of motion
  tweens imported from XFL (motion tweens themselves still come in as keyframes); 3D property
  groups; dragging keys in time in the Motion Editor.
