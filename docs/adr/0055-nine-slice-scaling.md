# ADR 0055: 9-slice scaling for symbols

* Status: accepted (2026-11-11)
* Relates to: ADR 0045 (editing a symbol in place), ADR 0050 (Library management), ADR 0035
  (XFL corpus and goldens), ADR 0019/0032 (GPU stage), ADR 0028 (HTML canvas export), ADR 0004
  (destructive actions)

## Context

Animate's Symbol Properties has **Enable guides for 9-slice scaling**. A movie clip or graphic
symbol then has four guides (two vertical, two horizontal) that cut it into nine cells. Scaling
an instance keeps the four corner cells at their size, stretches the top and bottom edges only
across, the left and right edges only down, and the centre both ways: rounded buttons and
panels resize without their corners distorting. While the symbol is open for editing, the
guides show as dashed lines that can be dragged. Strokes keep their width. Nested symbols, text
and bitmaps inside scale as a whole. Rotation and skew still apply to the whole instance.

Behaviour was learned from Adobe's public help pages, the XFL attributes of documents we
authored ourselves (`scaleGridLeft`, `scaleGridRight`, `scaleGridTop`, `scaleGridBottom` on
`DOMSymbolItem`) and the general technique as public UI toolkits describe it. No Adobe code,
SDK or asset was used.

## Decision

1. **Model.** `Symbol::scale_grid: Option<ScaleGrid>` (`#[serde(default)]`, left out when
   `None`, so older files open unchanged and files without guides are byte-identical). A grid
   is four coordinates in the symbol's own space (`left ≤ right`, `top ≤ bottom`, finite,
   within ±1,000,000). Loading drops a grid that isn't usable, so the symbol scales normally.

2. **Evaluation stays pure** (`doc::evaluate`, `nine_slice.rs`). Entering an instance of a
   symbol with a grid, its transform is decomposed as `T · R · Skew · S`. If `S` is 1 on both
   axes nothing changes. Otherwise the symbol's own shapes are drawn through the **9-slice
   map** of the symbol's box scaled by `|S|`, and then by `T · R · Skew` (with a flip, if `S`
   had one). So rotation and skew apply after slicing, and a flipped instance is sliced by its
   magnitude, then mirrored.
   * **The box** is the control-point bounds of everything on every keyframe of the symbol
     (guide, folder and camera layers left out; strokes don't widen it), so it doesn't move as
     the symbol plays.
   * **Each axis** is three linear pieces (`geom::slice::Axis`). The box lands where plain
     scaling would put it (`k·lo .. k·hi`, about the registration point). The parts outside the
     guides keep their size, and the centre gets the rest. **When there is no room for the
     corners** (the instance is narrower than the two corners), they share what room there is
     in proportion and the centre closes. Nothing turns inside out. Guides outside the box are
     clamped into it.
   * **Exact geometry.** Inside one cell the map is affine, so a Bézier piece maps exactly by
     its control points. `slice_path` cuts each segment at the exact parameters where it
     crosses a guide. It splits the segment into monotone runs at the roots of its derivative,
     then bisects each run (40 steps; at most three crossings per guide). It then splits by de
     Casteljau and maps each piece by its cell. Lines stay lines and curves stay curves of the
     same degree. An implied closing line is written out, because it may bend at a guide. A
     100 × 40 rounded rectangle scaled 3× keeps its radius-10 arcs to within flattening
     tolerance (tested analytically, and pixel-for-pixel against the unscaled corner by the
     CPU oracle).
   * **Strokes keep their width.** The item's transform has no 9-slice scale left in it, so
     the width is the symbol's own (times a shape's own scale inside the symbol). Linear
     gradients follow the map. Radial ones keep their centre on the map, and their radius
     scales by the mean scale.
   * **Scale as a whole, as in Animate:** nested symbols (they keep the whole transform, so
     they move and stretch with the instance), text, and shapes with bitmap fills. Each nested
     symbol is sliced by its own grid, if it has one, from its own instance scale. Grids don't
     pass to nested symbols.

3. **Everything sees it through `evaluate`.** The CPU renderer, the GPU stage (its binning is
   tested against the oracle on a sliced scene), PNG, GIF, SVG and Lottie draw the sliced
   outlines as ordinary display items. SVG and Lottie export baked outlines. Picking, onion skin
   and selection work on the same items. **HTML canvas export** places symbols in the browser
   at run time (ADR 0028), so its player can't slice them yet: a scaled 9-slice instance there
   scales as a whole, and the export's report counts it (*"9-slice scaling (scaled as a
   whole)"*).

4. **One command.** `symbol.set_scale_grid {symbol?, on?, left?, right?, top?, bottom?}` turns
   the guides on, turns them off, or moves them. Without `symbol` it acts on the symbol open
   for editing, otherwise on the selected instance's symbol. Turning them on puts them a
   quarter of the way in from each edge of the box. Moving a guide turns them on. Guides out of
   order are refused, not swapped. It is one undo step, or one per drag inside a gesture. It
   is in the `symbol` namespace, so plug-ins may call it, and it is in the fuzz.

5. **Shell.** The checkbox **Enable guides for 9-slice scaling** is in the Library's item
   properties (Animate's Symbol Properties), in Properties for a selected instance, and in
   Properties while editing the symbol with nothing selected. When it is on, each place also
   shows the four positions (`Guides x`, `Guides y`, monospace data, one undo step per drag). While the symbol is open
   (ADR 0045), the guides are dashed lines across the whole stage in the theme's `slice_guide`
   colour, over a light underlay. The Selection tool drags them (resize cursors, a heavier
   line under the pointer). Each drag is one undo step, *Move 9-Slice Guide*. A guide stops at
   the other guide of its pair. Edits show in the same frame.

6. **XFL.** `scaleGridLeft/Right/Top/Bottom` on `DOMSymbolItem` become the grid when all four
   are numbers; guides given the wrong way round are put in order. A partial or unreadable set
   is reported (*"9-slice guides that aren't usable (left out)"*). The corpus has a
   `nineslice` package (panel unscaled, scaled both ways, stretched wide; a nested graphic; a
   one-guide symbol) with a golden frame and colour probes. It also opens as a `.fla` (ADR
   0054).

## Performance

Slicing costs time per instance per frame: one pass over the symbol's keyframes for the box,
and the cut, which is linear in path size (at most 13 pieces per segment). Unscaled instances
cost nothing more, and their paths stay shared. Sliced paths are made new each frame, as
tweened shapes already are. Measured (release build, this Windows laptop), evaluating 1,000
instances of the rounded, outlined test panel scaled 3 × 2 takes **60 µs** as plain scaling
and **1.76 ms** sliced: about **1.7 µs per sliced instance**. A typical frame has a handful of
them. A path of more than 1,000,000 commands is mapped point by point instead of cut, so the
work stays bounded.

## Rejected

* **Slicing the whole instance's display list after evaluation.** It would slice nested symbols
  and bitmaps too, which Animate doesn't do, and it would lose which items are the symbol's
  own.
* **Corners scaled to the guides rather than to the box** (the centre scales by the instance
  scale exactly, and the instance grows by its corners). The instance would no longer have the
  bounds plain scaling gives it, and Free Transform's box would not match what is drawn.
* **Flattening to polygons before mapping.** It would be simpler, but curves would no longer be
  curves in SVG and Lottie, and the corners would not stay exact.

## Not yet

HTML canvas export slicing (its player), 9-slice for bitmaps (Animate doesn't either), guides
shown in Library previews, snapping guides to artwork, a symbol-wide "scale strokes" option,
and Lottie's native 9-slice (it has none).

## Update (2026-11-11, ADR 0057)

Turning the guides off no longer forgets them: they are kept (`Symbol::scale_grid_kept`, only
hidden) and turning 9-slice on again, or moving a guide, brings them back where they were.
