# ADR 0042: Subselection on raw drawings, and merging everywhere

* Status: accepted (2026-11-10)
* Relates to: ADR 0031 (merge drawing), ADR 0041 (Selection tool on raw drawings), ADR 0004
  (destructive actions), ADR 0009 and 0035 (XFL import and its corpus), issue #42 (section 3,
  and the rest of section 1)

## Context

ADR 0031 made the shape tools draw raw strokes and fills: one element per fill region and per
stroke run, which cut and join where they meet. ADR 0041 taught the Selection tool to bend,
pull corners, snap and select parts. Three things still did not behave as issue #42 describes.

* **Subselection (A)** treated raw shapes as objects. Clicking a raw side of a rectangle showed
  only that side's two points. In Animate, a click shows the anchors of **everything
  connected**, and a point drag moves every shape that has the point.
* **Not everything merged.** These came in as objects that never cut or joined the drawing:
  * the Paint Bucket's fills;
  * pasted raw shapes;
  * Animate's own raw shapes (XFL `DOMShape`);
  * gradient and bitmap fills.
* **Snapping could not be switched off.** Animate has View > Snapping > Snap to Objects (the
  magnet).

Behaviour was learned from the issue, Adobe's public help pages and watching the program;
nothing was copied.

## Decision

1. **Connected means sharing a point.** Two raw shapes are connected when a point of one (an
   anchor) lies within `2 × SNAP` (0.4 units) of the other's outline:
   * a corner both have;
   * an edge two fills share;
   * a line ending on an edge.

   `cmds_rawsub::connected` takes that transitively over the active layer's raw shapes. A grid
   over their bounds keeps it near linear, and it is bounded (20,000 shapes).

2. **Anchors are positions, listed once.** ADR 0041 stores a shared corner once in each shape
   that has it, so the Subselection tool works in stage positions:
   * `Session::raw_points` lists each anchor once, merging copies within `SNAP`;
   * it lists each curve handle once, a fill's edge and its outline having the same one;
   * a point drag is `raw.move_point` with `with: [[x, y], ..]` for the other selected anchors,
     which finds every shape's copy of each.

   Which anchors are selected is view state in the editor (`App::raw_anchors`), as it is for
   drawing objects. Positions survive the re-merge a drag ends with: the merge renumbers
   elements but moves a point by at most `SNAP`, so the editor keeps each selected anchor on
   the nearest point within 0.5 units.

   *Rejected:* addressing anchors as `(element, command)` pairs. Every merge renumbers elements,
   so the selection would be lost after every edit.

3. **Commands for each gesture** (namespace `raw`, already allowed for plug-ins):
   * `raw.subselect {x, y, add?}` is the click. It selects the connected raw shapes, and
     returns their anchors, handles, and the clicked edge's ends (whose handles show). An
     object is selected alone.
   * `raw.subselect_rect {x0, y0, x1, y1, add?}` is the marquee, on the **active layer only**.
     It selects whole the shapes it touches (raw ones with everything connected to them), and
     returns the raw anchors inside it. It never cuts; cutting is the Selection tool's.
   * `raw.points {ids?}` lists anchors and handles.
   * `raw.move_handle {id, cmd, slot, dx, dy, merge?}` moves a control point. Every raw shape
     with that edge follows (`edit_edge`, which `path.bend_segment` now shares; built on
     `anicraft_geom::raw::follow`).

   As in ADR 0041, a drag runs its command every frame with `merge: false` and `raw.merge` on
   release. The whole drag is one gesture, so one undo step (ADR 0004: undoable, no
   confirmation). Arrow keys move the selected anchors 1 screen px (Shift 10) as one step each,
   merged where they land, so ends within 0.2 px meet as ADR 0041 describes.

4. **What a Subselection press takes**, in order:
   * a shown handle: the drag bends that curve;
   * a shown anchor: it moves, with the other selected anchors (Shift adds to them);
   * a raw anchor not shown yet: its shapes show and the anchor moves;
   * a raw line or fill: its connected shapes show and **all of them move**, merged on release;
   * otherwise, the object behaviour as before.

   Anchors are hollow squares (solid when selected). The handles of the selected anchors' curves,
   and of the clicked edge, are lines ending in filled dots. On a drawing object, the
   Subselection tool behaves as before.

5. **Snap to Objects is tool state**, like Object Drawing (ADR 0031):
   * it is `ToolStyle.snap_objects`, on by default, toggled by `draw.snap_to_objects`;
   * the switch is in View > Snapping, and also a magnet under the tools and a checkbox in the
     Selection, Subselection and Free Transform options;
   * off, every `snap` parameter does nothing (`ToolStyle::snap_radius`), so the Selection tool
     and Subselection, and scripts, stop snapping alike;
   * merging still joins points within 0.2 px: that is the drawing model, not snapping.

   There is no default shortcut. Animate's is Ctrl+Shift+/, and the slash's shifted key depends
   on the keyboard layout. Users can give it one in Keyboard Shortcuts.

6. **The Paint Bucket fills raw drawings exactly.** Unless `object` (default: Object Drawing),
   the fill is raw and merges **behind** the drawing, so it takes only what is empty. It joins
   the fills of its colour it touches, and the lines that bound it stay on top.
   * **The area** comes from the planar merge itself. A *probe* fill under everything is
     merged with the raw shapes. What is left of the probe is the empty area, divided by every
     line and fill edge. The piece under the click is the area, unless it reaches the probe's
     border (the area is open). The result is exact: up to the lines' centres, with holes.
   * **It is local.** The probe starts as a 128-unit window around the click, with only the
     shapes that reach into it. It grows fourfold while the area reaches its border. A shape
     wholly outside a window cannot touch an area inside it.
   * **Fallback.** When raw shapes do not close the area (drawing objects close it, or a gap
     has to be bridged), the rasterized flood fill of before is used, and its result merges the
     same way.
   * **Recolouring** a raw fill merges it again, so it joins neighbours of its new colour.

7. **Pasted raw shapes merge where they land** (Paste, Paste in Place), cutting and joining
   like a drop (ADR 0031). Objects on the clipboard go on top whole, as before.

8. **Animate's raw shapes import raw.** In `anicraft-xfl`, a `DOMShape` loose on a frame
   (not `isDrawingObject`, not inside a `DOMGroup`) makes shapes with `merge: true`. The
   importer is layer L2, below the engine. So when `file.open` imports, the engine merges each
   keyframe's raw shapes into one drawing (`merge::merge_document`), across scenes and symbols.
   * XFL arrives one shape per style; afterwards there is one element per fill region and per
     stroke run, as AniCraft draws.
   * Keyframes a tween joins keep their shapes as they came, because tweens pair shapes in
     order.
   * The corpus renders still match their goldens. A raw outline is one stroke per side, so at
     a corner two round caps overlap where one round join was, and their antialiased edges add
     up: 47 of the shapes frame's 14,400 pixels are slightly darker. The merged-corpus test
     allows that and compares only, never rewriting goldens.

9. **Gradients and bitmap fills take part in the merge.** A fill's paint is carried into
   keyframe space with the shape (`Paint::transformed`), and back into each piece. Each piece
   shows its part of the gradient where it was. Fill styles are matched with a thousandth-unit
   tolerance (`same_paint`), so a gradient cut in two joins again when the cut goes.

   **Width profiles stay whole, as objects.** A profile runs along the whole stroke (width
   points by arc length). Cutting it at crossings, and joining runs again, would need each
   piece's length range carried through the planar map, which it does not track. That is a
   later change to `planar`.

## Consequences

* Measured on 748 raw shapes (12 × 12 outlined, rounded squares and 12 lines; optimised debug
  build, `subselection_timings`):

  | Operation | Time |
  |---|---|
  | Subselection click (63 connected shapes, 122 anchors, 96 handles) | 0.9 ms |
  | Listing anchors and handles (every frame) | 0.1 ms |
  | Handle drag frame | 0.1 ms |
  | Merge on release | 0.2 ms |
  | Paint Bucket fill, exact | 0.3 ms (20 ms when the probe took the whole layer) |
  | Paint Bucket on an open area (every window, then the raster) | 29–36 ms |

* Opening an Animate file now gives merge drawing as Animate had it. Its drawings cut and join
  as soon as they are edited.
* A Paint Bucket fill in merge drawing lies exactly along its lines. Deleting a line later
  joins fills of one colour, as in Animate.
* Not in this slice:
  * ~~deleting raw anchors (Delete says so), and adding them~~ (ADR 0045);
  * ~~Make Smooth and Make Corner on raw anchors~~ (ADR 0045);
  * ~~smooth handle pairs (a handle moves alone, as on drawing objects)~~ (ADR 0045);
  * width profiles in the merge;
  * ~~Subselection inside symbols~~ (ADR 0045: symbols open for editing in place);
  * ~~a default shortcut for Snap to Objects~~ (ADR 0045: Ctrl+Shift+/);
  * ~~snapping while drawing with the shape tools~~ (ADR 0045);
  * ~~a marquee across layers~~ (ADR 0044, for the Selection tool).
