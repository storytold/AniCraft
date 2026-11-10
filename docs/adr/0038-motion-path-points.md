# ADR 0038: Editing motion path points and moving the whole path

* Status: accepted (2026-11-09)
* Relates to: ADR 0030 (motion paths), ADR 0016 (motion tweens), ADR 0004 (destructive
  actions), roadmap M2

## Context

ADR 0030 gave motion tweens a cubic Bézier path that the object travels by arc length, timed
by the X/Y curves. Artists could bend it (Selection), drag its anchors and handles
(Subselection) and straighten it, but not add or delete points, make a point smooth or a
corner, or move the whole path. In Animate those edits work on a motion path as they do on
any path: the Subselection and Pen tools add and delete anchor points, a point is converted
between smooth and corner, and the whole path is moved by selecting and dragging it. We
learned this behaviour from Adobe's public help pages and from watching the program; nothing
was copied. The shape point editing in `geom::edit` and `cmds_path` (`path.add_point`,
`path.delete_points`, `path.convert_point`) already does most of it on shapes.

## Decision

1. **Point edits are commands on the path, reusing `geom::edit`.** `motion.add_point`
   splits the nearest segment by de Casteljau subdivision (`Path::split_segment`).
   `motion.delete_points` removes anchors, and `motion.convert_point` uses `make_smooth` and
   `make_corner`. All three edit the tween's path, or the straight path its X/Y curves trace,
   which the edit makes the tween's own (as a drag does in ADR 0030). Each one then goes
   through `MotionTween::set_path`. That keeps the normal form, and it keeps the ends' X/Y keys
   exactly when the ends didn't move, so **no point edit changes the timing**. Anchor `i` is
   path command `i`, because a motion path is a move followed by cubics. `motion.info` and the
   path commands list `anchors` in that order, so agents can address them.
2. **Adding a point changes nothing you can see.** The split curve is the same curve, so the
   object takes the same path on the same frames. Tests check this to 0.01 px on every frame.
   A path that already has `MAX_PATH_SEGMENTS` segments refuses another point instead of
   silently cutting off its end.
3. **Deleting a point refits instead of dropping.** Dropping an anchor (`remove_anchor`, as
   shapes do) keeps the next segment's control points, and that warps the curve. A motion path
   is the object's route, so it should stay close to where it was. `Path::merge_at` replaces the
   two segments around the anchor with the one cubic that fits them best:
   * It keeps the outer anchors and the directions their handles leave in.
   * It finds the handle lengths by least squares over samples of both segments, with Newton
     steps refining each sample's parameter (Schneider, *Graphics Gems*, 1990).
   * The samples start at the de Casteljau ratio of the middle anchor's handles. That ratio is
     exact when the two segments were once one, so **deleting a point that was added gives the
     original curve back** (tested to 0.01 px on the handles).
4. **The ends can't be deleted.** They are where the motion starts and ends: the first and
   last position keys hold them (ADR 0030 §3). Deleting an end would have to drop keys, or
   move the start somewhere the artist didn't put it. Asking to delete only ends is refused
   with a message that says why and what to do instead (drag them, or Straighten). Asking for
   ends and middle points deletes the middle points and reports `kept_ends`, which the status
   bar repeats. Deleting points is undoable and routine, so it doesn't ask first (ADR 0004).
5. **Moving the whole path keeps the timing exactly.** `motion.move_path {dx, dy}`
   (`MotionTween::translate`) moves:
   * the path;
   * every X and Y key;
   * the object's own position, which is the value of an axis that has no keys.

   The timing only compares positions with each other (chords and how far along them the X/Y
   curves are), so every frame lands exactly `d` over, with the same turn. Tests check this to
   0.001 px and 0.001° on every frame. A move that would leave the ±1,000,000 px range is
   refused and changes nothing. With no path, the straight motion moves the same way.
6. **On the stage** (`src/motion_path.rs`):
   * Subselection keeps a set of selected anchors. A click selects one; Shift adds or removes.
     A selected anchor shows its handles.
   * A double-click on the path adds a point there and selects it.
   * Delete or Backspace removes the selected anchors. These keys are taken before the
     global shortcuts, so the object isn't deleted.
   * A right-click on an anchor offers Make Smooth, Make Corner, Delete Point(s) and
     Straighten Path. Delete is disabled, with a tooltip, when only ends are selected.
   * The Pen shows the anchors. **Alt-click** on the path adds a point, and the Pen now does
     the same on a selected shape's outline. While Alt is held, the stretch the click would
     split widens and the cursor shows a plus.
   * **Ctrl-drag** (⌘ on macOS) the path, an anchor or the object, with Selection or
     Subselection, to move the whole path. We chose Ctrl because a plain drag already bends
     the path or moves the object. Alt-drag is how Animate users copy, and Shift constrains.
     Ctrl (⌘) is free on the stage, and as a modifier it reads as "the whole thing".
   * Properties › Frame shows **Path start** X/Y, which is where the motion starts. Editing
     it moves the whole path. It is also the keyboard and screen-reader route to the move.
   * Each gesture is one undo step: drags and the Properties fields through session gestures,
     and converting several anchors as one gesture. Edits show in the same frame.
7. **The file format doesn't change.** No new field: the path, keys and object were already
   saved.

## Anchors tied to frames: not in this slice

Pinning an anchor to a frame (spatial keys), so the object passes it exactly then, doesn't fit
the timing model of ADR 0030 cleanly:

* The X/Y curves are the timing, and the share of the path covered between two position keys
  is set by the chords between the keys' positions.
* A pinned anchor would be a second, competing source of timing. Which wins when they
  disagree? What happens to the X/Y keys when the pin moves, and to a pin when points are
  added or deleted, since anchor indices shift?
* Doing it properly means position keys *become* anchors: the timing comes from per-anchor
  frames plus an ease along the path, and the X/Y curves turn into a view of that, not its
  source. That is a model change with its own ADR, file format fields and Motion Editor
  behaviour.

Until then, moving the object at a frame bends the path through that point (ADR 0030 §4),
which covers the common "be here on this frame" case without a new model.

## Consequences

* The X/Y keys never change from a point edit, so retiming and path editing stay independent.
* A deleted point's neighbours keep their directions, so a smooth path stays smooth. A sharp
  S folded into one segment can only be approximated: one cubic has one inflection's worth of
  freedom.
* Performance, measured with the optimised dev profile on the development machine:
  * One add-and-delete pair through the command registry, undo snapshot included: 40 µs on
    an 8-segment path, 0.32 ms at 100 segments, 4.2 ms at 999 segments.
  * The refit itself: 7 µs.
  * One drag frame of `motion.move_path`: 15 µs, 0.1 ms and 1.8 ms on the same paths.

  Most of the cost is measuring the path again (`CurveWalk`) and snapshotting the keyframe.
* Not yet (later PRs):
  * anchors tied to frames (above);
  * a marquee over motion path anchors;
  * arrow-key nudging and Free Transform (scale, rotate) of a whole path;
  * deleting an end by giving the start to the next anchor (with its keys);
  * the Convert Anchor Point tool's drag-out of handles;
  * moving the path of a tween inside a symbol edited in place;
  * and the rest of ADR 0030's list: copying and pasting paths, eases on loop paths, snapping
    the ends, XFL motion path import.
