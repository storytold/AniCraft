# ADR 0045: Raw points, snapping while drawing, and raw shapes inside symbols

* Status: accepted (2026-11-10)
* Relates to: ADR 0031 (merge drawing), ADR 0041 (Selection tool on raw drawings), ADR 0042
  (Subselection and merging everywhere), ADR 0044 (selection across layers), ADR 0004
  (destructive actions), ADR 0001 (undo snapshots), issue #42 (sections 1 and 3)

## Context

ADR 0042 left these out of its slice:

* adding and deleting raw anchors (Delete only said it could not);
* Make Smooth and Make Corner on raw anchors, and smooth handle pairs (a handle moved alone);
* snapping while drawing with the shape tools;
* a default shortcut for Snap to Objects;
* Subselection (and, from ADR 0041, the Selection tool) inside symbols.

The last one needed more than the raw tools: AniCraft had no way to edit a symbol's timeline at
all. In Animate a double-click on an instance opens the symbol **in place**: the rest of the
stage dims, the timeline shows the symbol's layers, every tool works inside it, and the edit bar
says where you are (`Scene 1 › Badge`).

Behaviour was learned from the issue, Adobe's public help pages and watching the program;
nothing was copied.

## Decision

1. **Points are positions, as in ADR 0042.** A shared point is stored once in each raw shape
   that has it, so each command finds every copy near the position (within 1.5 × `SNAP`) and
   changes them alike. The pure parts are in `anicraft_geom::raw`: a path as subpaths of
   segments (`subpaths`, `from_subpaths`), the edges leaving a point (`edges_at`,
   `distinct_edges`), and the edits below.

2. **Adding a point** (`raw.add_point {x, y, slop?}`, the Pen's Alt-click and the Subselection
   double-click on a raw line or fill edge) splits the segment there in every raw shape that has
   that edge (de Casteljau, so the curve is unchanged; tested analytically). Nothing moved, so
   nothing merges. A point added on a **straight** line lasts while it is used: once a merge
   finds it still on a straight run (it was never moved), it is simplified away, as issue #42's
   continuous simplification asks. On a curve it stays.

3. **Deleting points** (`raw.delete_points {points}`, Delete or Backspace with Subselection,
   the point menu) works on the drawing's edges at each point, not on one shape:
   * **Two edges** (a corner, or a point along a line): they become one edge from the far end of
     one to the far end of the other: two lines a line, anything else the cubic that fits both
     best, keeping the directions they leave their far ends in (`join`, the fit `Path::merge_at`
     uses; deleting a point that adding made gives the curve back). **Every copy** of either
     edge becomes the new one: a fill's outline loses the corner, and a stroke that ended there
     now runs along the whole new edge. The drawing stays connected; the merge that follows
     joins the strokes that now overlap.
   * **One edge** (the free end of lines): the segment there goes.
   * **Three or more** (where lines cross or a line stands on another): the point is kept and
     the editor says why. Joining three edges into one has no single right answer.
   * What is left with nothing to show is removed: a subpath with no segments or no length, a
     filled outline with no area; a shape with no subpaths. So deleting a line's last points
     removes it.
   * One command, so one undo step (ADR 0004: undoable, no confirmation), merged again
     (`remerge_in_place`) like every raw edit.

   *Rejected:* deleting the point from each shape on its own (`Path::remove_anchor` per copy).
   Each shape would fit its own curve, and a fill and its outline would part.

4. **Make Smooth and Make Corner** (`raw.convert_points {points, smooth}`, the point menu):
   smooth turns a point with two edges into one whose handles lie on one line, along the
   direction between the far ends, a third of each chord long (`Path::make_smooth`'s rule);
   corner draws every curve's handle there in onto the point. Each changed edge changes in
   every shape (`follow_edge`, built on ADR 0041's `follow`). A smooth point on a straight line
   is simplified away by the merge, as in point 2.

5. **Smooth handle pairs.** Dragging one handle of a smooth point (its two handles on one line,
   opposite, within 3°) turns the other to stay opposite, keeping its length: `raw.move_handle`
   and `path.move_points` take `smooth?: bool` (default true); **Alt** breaks the pair. The
   other handle is found among the edges at the point: exactly one other curve whose handle is
   opposite (a junction has no pair). Drawing objects behave the same, so both kinds of shape
   edit alike.

6. **Snapping while drawing.** `draw.snap {x, y, snap}` (no undo step) says where a point drawn
   at (x, y) goes: the nearest anchor within `snap`, else the nearest point on a line or fill
   edge, of what the stage shows near it (display-list items whose bounds reach the point, so it
   costs nothing far away). The Line, Rectangle, Oval, PolyStar and Pen tools snap their start
   and end (the Pen its knots, never its handles) within 10 screen pixels, and show the snap
   ring as you hover, where a press would start. Shift's constraint wins over snapping. Off when
   Snap to Objects is off (`ToolStyle::snap_radius`). Scripts pass exact coordinates; the query
   is there for them too.

7. **Snap to Objects is Ctrl+Shift+/**, Animate's chord. Where `/` and `?` share a key (US, UK
   layouts), the key arrives as `?`, so `Ctrl+Shift+?` is bound too (an engine alias). Neither
   clashed with a default (the keymap's clash test). ADR 0042's worry was the layout: on layouts
   where `/` needs Shift (German: Shift+7), Ctrl+Shift+that key arrives as `/`, which the first
   binding catches.

8. **Editing a symbol in place** (`crates/engine/src/editing.rs`):
   * The session keeps a stack of open symbols (`Session::editing`). While one is open,
     **every command works on its timeline** (`Session::timeline`; the 86 places that read
     the scene's timeline now read this): the active layer, the playhead and the selection are
     its, positions are in its space, the display list (picking, bounds, snapping) is its
     timeline evaluated alone (`anicraft_doc::evaluate_timeline`). So merge drawing, the
     Selection tool, Subselection, raw points, the Paint Bucket and the rest behave inside a
     symbol exactly as on the stage: they are the same commands.
   * `symbol.edit {id?, symbol?}` (Edit › Edit Symbols, **Ctrl+E**, a **double-click** on an
     instance with the Selection tool) opens the selected or given instance where it stands, at
     the frame a graphic shows (a movie clip or button at its first); `{symbol}` (a double-click on
     a symbol in the Library, as in Animate, or its right-click Edit Symbol; Place on stage still
     places it) opens one alone at the stage's centre. Ctrl+E with no instance selected goes
     back to the document, as in Animate. `symbol.exit {all?}` (Edit › Edit Document, a
     double-click on empty stage, the edit bar's path) closes one level or all, selecting the
     instance again.
   * Which symbol is open is view state, but **undo snapshots keep it**, so undoing a change
     made inside a symbol opens it again: the layer, frame and selection in the step belong to
     that timeline. A symbol that goes away (an undo, a deletion) closes. Changing scene closes
     every symbol. Placing or pasting a symbol inside itself (or inside a symbol it uses) is
     refused; a camera can't be added inside a symbol.
   * **What the stage draws** (`Session::stage_picture`): a picture document whose scene holds
     the open timeline at its playhead where the instance stands, over each timeline below it
     (without the instance it was opened from) at 35 % opacity. They are graphic instances of
     copies of those timelines, so the oracle and the GPU stage draw them unchanged, and the
     render cache keys them by a hash of the state and the open levels.
   * **The editor's view follows the instance** when it is only moved and scaled evenly: the
     stage's screen mapping becomes `origin + zoom · (t + k · p)`, which the shell's existing
     origin-and-zoom mapping holds exactly, so every tool, overlay and snap radius works
     unchanged. A rotated, skewed or flipped instance opens upright and unscaled at its
     registration point, with the artwork around it hidden (a picture that did not line up with
     the tools would mislead).
   * The edit bar shows `Scene 1 › Badge › Layer 1`; a click on a symbol goes back to it, the
     scene's menu goes back to the scene. A crosshair marks the registration point.

   *Rejected:* a full affine view mapping in the shell (rotated in-place editing). Every tool's
   screen-space overlay assumes an axis-aligned mapping (selection boxes, Free Transform
   handles, marquees); bending them all is a slice of its own. *Rejected:* addressing a symbol's
   timeline through a `symbol` param on each command (as the Actions panel's commands do):
   every command and tool would need it, and the tools would still need a view of the symbol.

## Consequences

* Issue #42's Subselection (§3) is complete on raw drawings, inside symbols too. The drawing
  tools snap as the issue's §1 describes.
* Measured on 744 raw shapes (12 × 12 outlined, rounded squares and 12 lines; optimised debug
  build, `raw_points_timings`):

  | Operation | Time |
  |---|---|
  | Add a point | 0.29 ms |
  | Delete a point (joined, merged again) | 0.30 ms |
  | Make Smooth | 0.34 ms |
  | Smooth handle pair drag frame | 0.14 ms |
  | Drawing snap (every frame while hovering) | 0.02 ms |
  | Open a symbol of all 744 shapes | 0.04 ms |
  | Its stage picture | 0.01 ms |
  | A point move inside it | 0.52 ms |

* `panic_hunt_every_command` also starts inside an open symbol with raw drawings, and renders
  the stage picture.
* Not in this slice:
  * in-place editing of rotated, skewed or flipped instances (they open upright, alone);
  * the parent symbol's frame moving with the open one (a nested graphic shows the frame it
    was opened at), and onion skin and Edit Multiple Frames (ADR 0047) while a symbol is open in
    place: the stage then draws a picture of the symbol, so both switch off (`frames.*` commands
    still work on the open timeline). ADR 0051 turns both on there;
  * deleting a point where three or more edges meet;
  * width profiles in the merge (ADR 0042);
  * a marquee across layers with the Subselection tool (Animate keeps it on one layer too).
