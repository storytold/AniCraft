# ADR 0031: Merge drawing

* Status: accepted (2026-11-10)
* Relates to: ADR 0004 (destructive actions), ADR 0018 (variable-width strokes), ADR 0003 (file
  format), issue #42

## Context

In Animate, the Rectangle, Oval, PolyStar, Line, Pen and Pencil tools don't make objects by
default. They draw raw strokes and fills onto the layer, which interact (*merge drawing*):

* a fill drawn over another replaces what it covers, or joins it when the colour is the same;
* lines cut each other where they cross, and a line right across a fill cuts it in two;
* a fill on top hides the lines under it;
* points closer than about 0.2 px are one point.

Clicking with the Selection tool selects one fill or one stroke segment, shown with a dot
pattern rather than a box. Double-clicking selects what is connected. A dragged fill cuts into
whatever it is dropped on. *Object Drawing* (J) switches this off.

AniCraft only had drawing objects: every shape was an element of its own that never touched
another. Artists coming from Animate (issue #42) found that the most visible way AniCraft did
not behave like Animate. Behaviour was learned from Adobe's public help pages and from watching
the program; nothing was copied.

## Decision

1. **Raw shapes are ordinary shape elements with a flag.** `Shape.merge: bool` (default false,
   left out of files when false). A keyframe's raw shapes, between them, are the layer's merged
   drawing:
   * one element per fill region, with no stroke;
   * one element per stroke run, with no fill;
   * fills placed under strokes.

   Every existing system already works on them: renderer, GPU stage, exports, picking, undo,
   clipboard, Properties, tweens and the file format. Selecting "one fill" or "one segment" is
   ordinary element selection. Older files open unchanged (everything in them is an object),
   and older builds read the new files (the flag is ignored there).

   *Rejected:* a new element kind holding a shared planar map (edges with fill styles on their
   left and right, as XFL stores them). It is the more literal model, and shared vertices would
   come free. But every command, the selection model and every exporter would have to address
   parts of an element. That is a rewrite of the editor for the first slice. The flag keeps that
   door open: the merge already builds the planar map, so a later change can keep it.

2. **The merge is pure geometry** (`anicraft_geom::planar`), with no styles or documents.
   * **Input:** paths bottom to top, each with a fill-style number, a stroke-style number and a
     fill rule.
   * **Cutting:** every segment is cut where it meets another. Pieces are exact sub-curves (de
     Casteljau), found by bounding-box subdivision, plus ends that lie on a curve (T-junctions)
     and stretches where two curves run together.
   * **Snapping:** points within `SNAP` = 0.2 units become one vertex (greedy clustering, so no
     point moves farther than that). Pieces that run along each other become one edge.
   * **Faces:** the planar map's faces are traced (half-edges ordered round each vertex by
     where each first reaches a small circle, which also sorts curves that leave along one
     tangent). Each face takes the fill of the topmost path covering a point well inside it.
   * **Output:**
     * faces of one fill with no visible stroke between them are one region;
     * a stroke shows unless a later fill covers both sides of it;
     * strokes are chained through points where exactly two of one style meet smoothly (within
       20°), so a crossing or a corner starts a new piece, as in Animate where a rectangle's
       sides are selected one at a time;
     * pieces of one original segment are joined back into one exact sub-curve, and straight
       runs into one line.

   The arithmetic is `f64` with no platform maths (no `atan2`), so results are the same
   everywhere. Merging its own output again gives the same pieces (tested).

3. **What takes part.** The engine (`crates/engine/src/merge.rs`) merges the new raw shapes
   with the keyframe's raw shapes whose bounds touch them. Element transforms are applied to the
   geometry, and stroke width scales by the transform's mean scale.
   * **Kept whole, as objects:** non-solid fills (gradients, bitmaps) and strokes with a width
     profile, until the merge can carry them along.
   * **Too large:** past `MAX_SEGMENTS` (20,000) or the pair budget, the merge declines and the
     new shape is added whole, as an object. Nothing is lost.
   * **Placement:** the result goes where the lowest shape it took in was, so raw shapes stay
     under the objects on the layer, as in Animate. The pieces that came from the new shape are
     selected.
   * **Undo:** one command, one undo step, like any other edit (ADR 0004: undoable, so no
     confirmation).

4. **Object Drawing is session tool state**: `ToolStyle.object_drawing`, toggled by
   `draw.object_drawing` (J). Every drawing command takes `object?: bool` to override it.
   * **The editor starts with it off**, as Animate does.
   * **A new headless session starts with it on**, so a script's or an AI assistant's
     `draw.rect` still makes exactly one element. CLI and MCP behaviour is unchanged unless they
     ask for `"object": false`.

5. **Editing raw shapes.**
   * `draw.drop` merges the selected raw shapes back into the drawing (on top). The Selection
     tool runs it when a drag ends and after each arrow-key nudge, in the same undo step as the
     move.
   * `element.select_connected` is the double-click. On a stroke, it selects the stroke and
     every stroke joined to it end to end. On a fill, it selects the fill and the strokes along
     its edge or inside it.
   * On the stage, selected raw shapes show Animate's dot pattern (light and dark dots on a
     fixed 3 px screen grid) instead of a bounding box.
   * Arrow keys move the selection 1 screen pixel (Shift: 10), so they move less the further
     you zoom in.

## Consequences

* Drawing with the shape tools now behaves as Animate users expect. Scripts keep their
  one-element-per-command behaviour.
* A raw rectangle with an outline is five elements (a fill and four sides). That is what
  Animate's selection model needs. Large raw drawings make many small elements, each cheap; the
  merge only touches what the new shape's bounds reach.
* Adjacent fills of different colours are separate paths, so antialiasing can show a faint seam
  where they meet, as in Animate.
* Remaining parts of issue #42 (later slices, on this model):
  * bending a segment by dragging it with the Selection tool;
  * snapping dragged points to points and strokes within 10 screen pixels;
  * moving a point to move every shape that shares it (Subselection and arrows);
  * a marquee that selects parts of fills and strokes;
  * Subselection anchors and handles of connected raw shapes;
  * merging the Paint Bucket's fills and pasted raw shapes;
  * importing Animate's raw shapes (XFL) as raw shapes;
  * gradients and width profiles in the merge.
