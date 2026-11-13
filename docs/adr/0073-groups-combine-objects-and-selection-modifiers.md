# ADR 0073: Groups, Combine Objects and Selection-tool modifiers

* Status: accepted (2026-11-12)
* Relates to: ADR 0031 (merge drawing), ADR 0041 (Selection tool on raw drawings), ADR 0045
  (editing symbols in place), ADR 0042 (raw shapes from Animate files), ADR 0004 (destructive
  actions), ADR 0003 (file format), issues #206, #229, #200, #241

## Context

Artists coming from Animate asked for (issue #206) **Modify › Group** (Ctrl+G) and **Ungroup**
(Ctrl+Shift+G), and (issue #229) for Animate's vector workflow around drawing objects:
**Modify › Combine Objects** (Union, Intersect, Punch, Crop) and the **Selection tool's
modifiers** (Ctrl/Cmd-drag pulls a corner, issue #200; Alt-drag copies; Shift constrains).
AniCraft had no groups: XFL `DOMGroup`s were flattened on import, and the guide told people to
make a symbol instead. Issue #241 asks, among other things, to open symbols by double-clicking
them (done in ADR 0045) and for snapping (Snap to Objects, ADR 0042); nothing else in it is in
this area.

In Animate, a group is a container without a Library symbol: it moves, scales and stacks as one;
a double-click opens it in place with the rest of the stage dimmed; groups nest; the raw shapes
inside merge only with each other, and a group sits above the layer's raw shapes as objects do.
Behaviour was learned from Adobe's public help pages, the issues and watching the program;
nothing was copied.

## Decision

1. **A group is an element kind**, `ElementKind::Group { members: Timeline }` (`#[serde(default)]`:
   files without groups are byte-for-byte unchanged; older builds can't read a file with a group,
   as with any new kind). Its members are a timeline of **one layer and one frame**. That is the
   whole trick: Session's editing stack (ADR 0045) already edits "a timeline", so a group opens in
   place with **every command and tool working inside it unchanged** (merge drawing, the
   Selection tool, Subselection, the Pen, the Paint Bucket…), and raw shapes inside merge only
   with each other because a merge never leaves its keyframe.
   * *Rejected:* `members: Vec<Element>`. The literal model, but every command addresses "the
     keyframe at the playhead of the edited timeline"; editing inside a group would have needed a
     second addressing scheme through every command.
   * *Rejected:* a hidden graphic symbol per group. Everything would work for free, but a group is
     a value: copies (Duplicate, F6, paste) must be independent, and a shared symbol isn't. The
     lead also asked for no Library symbol.
   * The one-layer, one-frame shape is kept by the engine: while a group is open, commands in the
     `timeline.`, `layer.`, `camera.`, `bone.`, `motion.`, `tween.` and `frames.` namespaces, and
     `sound.place`, `script.set` and `script.clear`, are refused with a message, and the playhead
     stays on frame 0. `Document::sanitize` repairs hostile files (one keyframe at 0 per layer, no
     camera, armature or folder layers, at most `MAX_GROUP_DEPTH` = 32 levels; ids inside count).

2. **Evaluation: a group's members stand on its layer.** `eval_element` draws each member as if
   it were on the group's layer and keyframe, placed by the group's transform and coloured by its
   colour effect (composed exactly as `layer · group · member`), and picked as the group. So a
   graphic or movie clip inside a group keeps time with the timeline the group is on. Below full
   opacity a group fades as one (a display-list group, as instances do). No new rendering
   semantics: every export goes through `evaluate`, and the HTML canvas player (which places
   symbols itself) gets the members placed by the group. The player, sounds, Library uses,
   Delete Unused, deleting Library items, the clipboard and the Movie Explorer all see inside
   groups (`Document::timelines` includes group timelines; `expand_groups` places members).

3. **Commands** (namespace `modify`, already allowed to plug-ins):
   * `modify.group` (Modify › Group, **Ctrl+G**): the selected elements on each layer become one
     group there, at the topmost one's place in the stacking, its transform the identity, so
     every member keeps its world transform bit for bit (tested).
   * `modify.ungroup` (Modify › Ungroup, **Ctrl+Shift+G**; Break Apart on a group does the same
     with fresh ids): members go back where the group was, each with `group · member` as its
     transform and the colour effects nested; they keep their ids unless a copy of the group was
     ungrouped beside them. Raw shapes among them merge with the drawing they now stand in, as a
     dropped selection does (in Animate, ungrouped shapes cut and join what is under them).
   * `symbol.edit {id}` and a **double-click** open a group in place; the edit bar says
     `Scene 1 › Group`; `symbol.exit` and a double-click on empty stage close it. Undo opens it
     again where the change was made; a level whose group went away (an undo, an ungroup) closes.
     `EditLevel` now says what it opens (`Opened::Symbol(id)` or `Opened::Group`);
     `edited_mut` returns a `Result` (it can't reach a group that is gone).
   * Groups are all one undo step each (ADR 0004: undoable, so no confirmation).

4. **Combine Objects** (`modify.combine {op}`, and Modify › Combine Objects › **Union**,
   **Intersect**, **Punch**, **Crop** as `modify.union` …): exact geometry on the planar map of
   merge drawing (`anicraft_geom::planar`, ADR 0031), on two or more selected shapes (raw or
   drawing objects) on one layer, bottom to top as stacked:
   * **Union**: everything merged as merge drawing merges it. One fill of one look becomes one
     drawing object; anything with several colours or lines left becomes a **group of the merged
     raw shapes**, AniCraft's equivalent of Animate's multi-colour drawing object.
   * **Intersect**: the area every shape's fill covers (each cutter first resolved to
     non-overlapping outlines, then `merge_cut`, keeping what is inside), in the topmost shape's
     fill and stroke, as Animate does.
   * **Punch** / **Crop**: each lower shape cut along the topmost one's fill area, keeping what is
     outside / inside it (its lines too); the topmost goes. Each result takes its source's place.
   * A result of one piece is a drawing object; of several, a group of raw pieces. Areas are
     tested analytically (squares: 17,500, 2,500, 7,500, 2,500; a 100² − 40² ring; a quarter
     circle within 0.1 %).
   * *Not in this slice:* **Delete Envelope** (AniCraft has no Envelope distortion yet; see
     below), and shapes whose stroke has a width profile (they merge as objects, ADR 0031).

5. **Selection-tool modifiers** (the shell composes existing commands, so a script does the same
   with the same commands):
   * **Ctrl/Cmd-drag** on an unselected raw line or fill edge pulls a **corner** out of it
     (issue #200). This PR first had its own version (`raw.add_point`, then `raw.move_point`);
     the drawing-tools fixes (#257) reached `main` with one that also makes the new point a
     corner on curved edges (`raw.convert_points`) and falls back to a bend where no point can
     go, so that one is kept and this ADR's modifiers sit in its `select_press`.
   * **Alt/Option-drag** on a selection or a fill moves a **copy** (`edit.duplicate` in place,
     then the move), leaving the original whole; raw copies merge on top where they land.
   * **Shift** while moving keeps the move horizontal, vertical or at 45° and turns snapping off
     for the move (as Shift does while drawing, ADR 0045).

6. **XFL import**: a `DOMGroup` now comes in as a group, placed by its matrix, its shapes raw
   inside it and merged with each other on open, as a keyframe's are (before, its members were
   flattened onto the frame as drawing objects).

## Consequences

* Animate users find Group, Ungroup and Combine Objects where they expect them, with the same
  keys; Animate files keep their groups.
* Measured on 288 raw shapes (12 × 12 outlined, rounded squares; optimised debug build):

  | Operation | Time |
  |---|---|
  | Group | 0.10 ms |
  | Evaluate the frame with the group | 0.07 ms |
  | Open it in place (with its stage picture) | 0.10 ms |
  | Ungroup, merging every raw shape back | 18 ms |
  | Union / Intersect / Punch / Crop (a circle and a 12-point star) | 0.27 / 0.33 / 0.28 / 0.27 ms |
* Drawing objects stay single shapes. A multi-colour drawing object (Union of colours, Punch of a
  stroked shape) is a group, so it is edited with a double-click instead of in place. Merging the
  two ideas (a drawing object as a group that tools reach into without opening it, issue #181)
  is a later step on this model.
* A classic or shape tween between two groups tweens the group's placement and colour, and its
  members pair by order and tween as they would alone (an Animate file's tweened groups play as
  in Animate; the XFL corpus checks it). A member without a partner holds.
* The HTML canvas export draws a translucent group's members one by one (their overlaps show
  through); every other output fades it as one.
* The Movie Explorer lists what is inside a group as on its keyframe; going to such a row selects
  the keyframe, not the member.
* Not in this slice (proposed):
  * Envelope and Delete Envelope (Animate's Free Transform envelope mode): a new transform kind
    on shapes, its own ADR;
  * drawing objects editable without opening them (issue #181), and Break Apart turning a drawing
    object into raw shapes;
  * Ctrl-click to isolate a sub-element on the canvas (issue #229 §3), which overlaps with what a
    double-click already does;
  * Combine Objects across layers and on symbols, text and groups (break them apart first).
