# ADR 0023: Shape hints

* Status: accepted (2026-11-09)
* Relates to: roadmap M2 (shape tween), ADR 0003 (file format), ADR 0004 (destructive
  actions), ADR 0009 (XFL import), ADR 0016 (motion tweens)

## Context

Shape tweens morph any outline into any other (`anicraft_geom::morph`): subpaths pair by size,
segments are subdivided to equal counts and closed outlines start at the best-matching anchor.
That is a good guess, not the artist's intent: a square into a circle may turn as it morphs,
and a letter into another letter often folds. Animate gives artists *shape hints*: lettered
markers `a` to `z`, each placed on the start shape and on the matching point of the end shape,
so the morph maps those points onto each other. Behaviour was learned from Adobe's public help
pages and from observing the program; the morph method is our own (below), from the public
literature on polygon and Bézier blending. Nothing was copied.

## Decision

1. **Hints belong to the keyframe that starts the shape tween**:
   `Keyframe.hints: Vec<ShapeHint>`, `ShapeHint { start, end }`, both points in the layer's
   own space (the stage's, at the scene root). The letter is the position in the list (`a`
   first), at most 26 (`MAX_SHAPE_HINTS`). Removing a hint moves the later letters down one.
   The field is `#[serde(default, skip_serializing_if = "Vec::is_empty")]`: older files open
   with none, and files without hints are unchanged byte for byte.
2. **"Placed" is derived, never stored.** A point is placed while it lies on a shape outline of
   its keyframe, within `HINT_TOLERANCE` (0.5 px). A new hint sits at the centre of the shapes'
   bounds on both keyframes, so it is unplaced until dragged onto an edge; an edit that moves
   the edge away unplaces it again, with no stale flag to keep in sync. Commands snap points to
   the nearest edge (`anicraft_geom::morph::nearest_point`: coarse samples per cubic, a short
   ternary search, then Newton steps on `(B − p)·B′`, since squared distance is too flat at its
   minimum for f32).
3. **The morph uses a hint only when it is placed on the same shape element on both keyframes**
   (elements pair by index, as tweens already do); the points are taken into each element's own
   coordinates. Unplaced and half-placed hints change nothing, so evaluation stays pure and
   tolerant: a hint can never break a tween, only steer it.
4. **How hints steer the morph** (`compatible_hinted`, `morph_hinted`):
   * subpaths joined by a hint pair with each other (the first hint on a subpath decides); the
     rest pair by size as before;
   * per pair (after the usual winding fix), each outline is cut at its hinted points (de
     Casteljau at the nearest parameter) and closed outlines are rotated to start at the first
     hint, so the pieces between consecutive hints correspond in order;
   * hints that cross the order of the others are dropped: on a loop the first hint anchors the
     order and the longest run in cyclic order after it is kept; on an open path, the longest
     increasing run;
   * inside each pair of pieces, points correspond **by length**: each piece gets the other's
     anchors at the same fractions of its length, then both are subdivided to equal counts.
     Count-based splitting alone (what unhinted pieces do) pairs a hair-thin piece with half an
     edge when a hint lands next to an anchor, and the in-between shape folds; by length, a
     square into a circle halfway keeps about 8,950 px² of area against 8,100–8,600 by count
     (tests in `morph.rs`).
   No hints is exactly the old morph (asserted by a test).
5. **Commands** (`crates/engine/src/cmds_hints.rs`): `tween.add_hint` (Modify › Shape › Add
   Shape Hint, Ctrl+Shift+H), `tween.move_hint` (`hint`, `x`, `y`, optional `side`; snaps),
   `tween.remove_hint`, `tween.clear_hints` (Modify › Shape › Remove All Hints) and
   `tween.hints` (read-only: points and placed state, for scripts and MCP). `frame` picks the
   tween as the stage does (`Layer::hint_tween_at`): a frame inside a shape tween's span is its
   start side; the first frame of the keyframe it ends on is its end side, unless that keyframe
   starts a shape tween of its own. Locked layers refuse. Plug-ins may call `tween.*`.
6. **Hints go with their shape tween**: replacing it by a classic or motion tween, removing the
   tween or clearing the keyframe drops them (all undoable). Splitting a tween with F6 keeps
   them on the first part; their end points then sit off the new end shape and show unplaced.
7. **Destructive tiers (ADR 0004)**: removing one or all hints is a routine, undoable edit:
   no confirmation, a status hint names the undo shortcut.
8. **Stage** (`src/shape_hints.rs`): on the first frame of either keyframe, with the selection
   tools, the active layer's hints show as 16-point lettered dots in AniCraft's own colours:
   rose (`hint_unplaced`) while not on an edge, amber (`hint_start`, the shape tween's colour)
   once placed on the first keyframe, mint (`hint_end`) once placed on the last (brighter in
   the high-contrast palette); ink letters meet 4.5:1 on all three in both palettes (tested). Dragging one moves it live (each pointer move runs
   `tween.move_hint`, coalesced into one undo step); right-click offers Remove Hint, Remove All
   Hints and Add Shape Hint.
9. **XFL import**: Adobe does not publish the XFL schema for hints, so the importer reads a
   tolerant, self-authored form under a frame's `shapeHints` element (the name the importer
   already detected): each child is a hint with its point on that keyframe as `x`/`y`,
   `startPoint="x, y"` or `startX`/`startY` (pixels), optionally its end as `endPoint` or
   `endX`/`endY`; otherwise the end is the same hint (by `index`, letter `name`, or order) in
   the next keyframe's `shapeHints`. Points within 2 px of an edge snap onto it. Hints past
   `z`, without a position or without an end are reported, never fatal. If real files turn out
   to use another layout, only this reader changes.

## Consequences

* Artists get Animate's workflow (Ctrl+Shift+H, drag on first and last keyframe, colours that
  say what's placed) and headless users the same through `tween.*` over the CLI and MCP.
* Placement is always truthful and costs a nearest-point search per hint per evaluation. On a
  20-segment star into a circle (optimized build): 2.3 µs per morph without hints, 22 µs with
  4, 119 µs with all 26; a 2,000-segment outline stays bounded (`MAX_SEGS`).
* Not yet: View › Show Shape Hints (they always show on the keyframes), hints inside symbols
  edited in place, hints on text, hints that also steer strokes separately from fills, and
  XFL export (AniCraft doesn't write XFL).
