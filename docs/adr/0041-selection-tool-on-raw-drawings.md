# ADR 0041: Selection tool on raw drawings

* Status: accepted (2026-11-10)
* Relates to: ADR 0031 (merge drawing), ADR 0004 (destructive actions), ADR 0001 (undo
  snapshots), issue #42 (sections 1, 2 and 4)

## Context

Merge drawing's first slice (ADR 0031) made the shape tools draw raw strokes and fills, stored as
one element per fill region and per stroke run. Clicking selected one of them, but the Selection
tool still treated them as objects:

* dragging an unselected line moved it;
* a corner could not be pulled;
* nothing snapped;
* a marquee selected whole shapes whose boxes it touched.

Issue #42, from an Animate user, describes the Animate behaviour:

* drag a segment away from its ends to curve it;
* drag an endpoint or corner to move it;
* drag a fill to move only that fill;
* points snap to points, strokes and fill edges within 10 screen pixels when moved by the mouse,
  and within 0.2 document pixels when moved by the keyboard (anchors meet at their midpoint; a
  point snaps onto a line without changing it);
* a marquee selects the parts of fills and strokes under it.

Behaviour was learned from the issue, Adobe's public help pages and watching the program; nothing
was copied.

## Decision

1. **Shared geometry is edited in every copy.** ADR 0031's model stores a shared edge or corner
   once in each raw shape that has it. We keep that model and make each edit change all the
   copies alike, then merge the shapes again. Pure parts are in `anicraft_geom::raw`:
   * `follow` replaces every stretch of a path that runs along an edge (within `SNAP`) with the
     same stretch of the bent edge. It splits a longer segment at the edge's ends first.
   * `anchors_at` gives a path's anchors at a point. When only an edge passes there (another
     shape ends on it), it splits the edge, so the T-junction moves with the point.

   *Rejected:* moving to a planar-map element (edges with fills on either side) now. It would
   make sharing automatic, but every command and exporter would have to address parts of an
   element (ADR 0031 rejected it for the same reason). The copies approach costs a geometric
   match per edit (measured below) and keeps every existing system working.

2. **Commands for each gesture** (headless, CLI, MCP, plug-ins: namespace `raw` added to the
   plug-in policy).
   * `path.bend_segment {x, y, dx, dy, id?, cmd?, t?, merge?}` bends a segment so its point at
     `t` moves exactly by `(dx, dy)`, ends fixed. This is the least-squares control-point move
     `Path::bend_segment` already used for motion paths. On a raw shape, every copy of the edge
     follows (the fill under an outline, a neighbouring fill). On an object, only it bends.
   * `raw.move_point {x, y, to_x, to_y, points?, snap?, merge?}` moves a corner or end with every
     raw shape that shares it. Handles move with their anchors, and straight segments stay
     straight.
   * `raw.select_rect {x0, y0, x1, y1, add?}` is the marquee (point 4).
   * `raw.merge {ids?}` merges shapes again in place, keeping their stacking.
   * `raw.hit {x, y}` says what a drag from a point takes.
   * `element.translate` takes `snap` and `from`.

   A drag runs its command every frame with `merge: false`, passing on the segment
   (`id, cmd, t`) or the `points` the first frame returned. Release runs `raw.merge` on what
   changed. The whole drag is one gesture, so one undo step (ADR 0004: undoable, no
   confirmation). Scripts call a command once with the default `merge: true`.

3. **What a Selection-tool drag takes** (`Session::raw_grab`), in this order:
   * anything selected: the drag moves the selection;
   * an unselected raw corner or end within 6 screen pixels: it moves;
   * an unselected raw stroke, or a raw fill's edge, within 3 screen pixels: it bends;
   * the inside of a raw fill: it is selected alone and moved (the outline stays);
   * an object: it moves, as before.

   A small corner or arc mark by the pointer shows which, like Animate's pointers.

4. **Snapping.**
   * **Mouse:** the target is the nearest anchor within 10 screen pixels, else the nearest point
     on a stroke or fill edge (`10 / zoom` document units, so it feels the same at any zoom).
     The shapes that move with the point are left out of the targets. A moving selection snaps
     its anchor nearest the pointer onto the shapes that do not move. A ring shows where it
     snapped.
   * **Keyboard:** nudges merge on drop as before. The planar merge now places a vertex at the
     midpoint of the distinct segment ends within `SNAP` (0.2), unless one of its points lies
     inside a segment, away from its ends. Then the vertex stays on that line and the line is
     not changed.

5. **The marquee cuts provisionally.** `planar::merge_cut` merges with a closed cutter whose
   edges divide fills and end stroke runs, but never show. Each piece says whether it is inside.
   Raw shapes partly under the rectangle are replaced by their pieces; the inside pieces are
   selected, with raw shapes and objects wholly inside.
   * In Animate a marquee is only a selection. So the cut records **no undo step** and leaves
     a saved document saved.
   * The session keeps the document from before the cut. Once the selection lets go of every
     inside piece with nothing else changed (a click elsewhere, a new marquee), the cut is
     taken back exactly. Saving, undo and redo also take it back first. Selected pieces map to
     the shapes they came from.
   * The first edit that uses the cut (a move, delete, recolour) records one step from before
     the cut, so undo goes straight back to the whole drawing.
   * Shift adds a marquee's parts to the selection; parts already selected stay selected when
     cut again.
   * The marquee only cuts on a keyframe's own frame: inside a tween, an edit would add a key.

   *Rejected:* keeping cuts permanently (seams and ever more fragments after every marquee), and
   a lazy "region" selection that every command would have to understand.

6. ~~**A selection lives on one layer.**~~ Superseded by ADR 0044: the selection is a set of
   `(layer, element)` pairs and the marquee works on every visible, unlocked layer, cutting raw
   parts per layer.

## Consequences

* The Selection tool now bends, pulls corners, snaps and selects parts as #42 describes. Every
  gesture is a command and one undo step.
* Measured on 1,320 raw shapes (12 × 12 outlined squares and 12 lines; optimised debug build,
  `raw_drag_timings`):

  | Operation | Time |
  |---|---|
  | Hover test | 0.04 ms |
  | Bend frame | 0.14 ms |
  | Point frame with snapping | 0.22 ms |
  | Snapping selection move frame | 0.29 ms |
  | Merge on release | 0.22 ms |
  | Marquee cut and take-back | 1.8 ms |

* The midpoint rule changes where any two ends within 0.2 meet (before, the lower shape's point
  won). The move is at most 0.2, and merging again changes nothing (tested).
* Not in this slice:
  * ~~selecting across layers~~ (ADR 0044);
  * ~~snapping while drawing with the shape tools~~ (ADR 0045);
  * ~~a Snap to Objects switch~~ (ADR 0042);
  * ~~Subselection anchors and handles of connected raw shapes~~ (ADR 0042);
  * ~~bending or moving points of raw shapes inside symbols~~ (ADR 0045);
  * the Free Transform tool's handles on raw parts.
