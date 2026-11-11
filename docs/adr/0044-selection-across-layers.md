# ADR 0044: Selection across layers

* Status: accepted (2026-11-10)
* Relates to: ADR 0041 (Selection tool on raw drawings, decision 6), ADR 0042 (Subselection),
  ADR 0001 (undo snapshots), ADR 0004 (destructive actions), issue #42 (section 4)

## Context

Until now the session's selection was a list of element ids on the active layer. ADR 0041 left
the marquee on one layer: the active layer when it had something under the rectangle, else the
topmost visible, unlocked layer that did. Issue #42 says the Selection tool's marquee selects
"the specific parts of fills and stroke segments that fall under the selection rectangle across
all unlocked layers". In Animate a selection holds elements on several layers: Shift-click adds
from any layer, Select All takes every unlocked layer, and moving, deleting, aligning, copying,
transforming and the Properties panel act on all of them.

Behaviour was learned from the issue, Adobe's public help pages and watching the program;
nothing was copied.

## Decision

1. **The selection is a set of `(layer, element)` pairs at the playhead**
   (`anicraft_engine::Selection`, `crates/engine/src/selection.rs`). It keeps selection order
   and a hash set for membership, and is saved in every undo snapshot as before. Element ids
   stay unique within a document, so commands keep their `ids` params and results; the pairs make
   the layer explicit instead of implied by the active layer.

   * `fix_selection` (after every command, undo, redo and frame change) drops pairs whose
     element is not on its layer at the playhead, and pairs on a layer that is gone, hidden or
     locked, directly or through a folder. **A locked or hidden layer is never selected.**
   * The active layer stays a separate thing: the layer the last click, `element.select` or
     marquee landed on. A selection may hold items on layers other than the active one.

   *Rejected:* keeping `Vec<ElementId>` and finding each id's layer on demand. It would work
   while ids are unique, but every reader would search every layer, and a selection could not
   say which copy it means if ids ever repeat (imports, frames pasted with their ids).

2. **Every command that reads the selection is classified**, and a selection on the active layer
   alone runs exactly the code it ran before (`Session::each_layer` calls the body once,
   unchanged):

   | How | Commands |
   |---|---|
   | **Per layer**: the single-layer body runs once for each layer with selected items, top to bottom, with that layer active and the selection narrowed to it | `edit.delete`, `edit.duplicate`, `element.translate` (and nudges), `element.set_transform`, `element.set_alpha`, `element.rename`, `arrange.*` (within each layer), `modify.transform`, rotate and flip (about the combined centre), `modify.remove_transform`, `draw.drop`, `raw.merge` (default ids) |
   | **Per layer, where it applies**: a layer where the edit does not apply (no shapes, no movie clips, no text) is left as it was; the command fails only when it applies nowhere | `element.set_fill`, `element.set_stroke`, `element.set_filters`, `element.set_blend`, `element.set_loop`, `stroke.set_profile`, `paint.apply_style`, `text.edit`, `modify.break_apart` |
   | **Across** the layers | `align.*` and `align.distribute_*` (bounds of every selected element, each moved on its own layer), the Free Transform box (`selection_bounds`), `edit.copy` and `edit.cut`, `edit.paste*`, `modify.convert_to_symbol`, snapping a move, `symbol.set_kind` (the first selected instance), the Actions panel's selected instance |
   | **On the active layer**, as Animate does | the Subselection tool and its marquee (ADR 0042), the Width tool, the Eraser's selected fills, `raw.move_point`, `path.bend_segment`, `raw.merge {ids}` |

   Per-layer runs reuse the single-layer code, so the auto-key rules hold on each layer: inside
   a classic tween every selected layer gets a keyframe at the playhead, and a motion tween's
   target becomes property keys on its own layer (the session finishes each layer's motion edit
   after its run). A per-layer command returns one result: lists joined, counts added, other
   values from the first layer, and `layers` naming the layers it ran on.

3. **Selecting.**
   * `element.pick {add: true}` (Shift-click) adds or removes the element under the point on
     whichever layer it is, keeping the rest; a plain click on something unselected selects only
     it; a click on something selected keeps the selection, so a drag moves it all. The clicked
     layer becomes active.
   * `element.select {ids, add?, layer?}` takes ids on any visible, unlocked layer (looking on
     the active layer first, then top to bottom, or only on `layer`); the active layer becomes
     the first id's layer unless it holds some of them.
   * A click on a layer's name in the timeline makes it active and selects all of its artwork at
     the playhead, as in Animate (`layer.select {layer, select: true}`; nothing on a locked or
     hidden layer). Without `select`, `layer.select` only changes the active layer, as before.
   * `edit.select_all` takes every visible, unlocked layer's artwork at the playhead (the camera
     and armature layers only while active: one holds the camera, the other's instances are posed
     by their bones). In a document with one layer nothing changes; with several, Select All now
     selects what Animate's does.

4. **The marquee works on every visible, unlocked layer** (`raw.select_rect`). One display list
   finds what each layer has wholly inside the rectangle and which raw shapes it crosses; raw
   shapes are then cut per layer (ADR 0041's provisional cut, `planar::merge_cut`), so a cut never
   joins artwork of two layers. The cut of all layers is one provisional change: it records no
   undo step, a marquee alone leaves a saved document saved, letting go takes back every layer's
   cut exactly, and the first edit that uses it records one step from before it. The active layer
   stays when the marquee took something there, else the topmost layer it took from becomes
   active. Re-merging after a move (`draw.drop`, `raw.merge`) runs per layer.

5. **Paste keeps each layer.** The clipboard keeps the copied artwork apart by layer (id and
   name). Artwork copied from one layer pastes on the active layer, as before. From several:
   back on the layers it came from when every one of them is in this scene with the same name
   and can be drawn on; otherwise on the active layer and the drawable layers below it in order,
   with layers created at the bottom for the rest, named after the copied ones, as Paste Frames
   does. Raw shapes merge into each target layer's drawing.

6. **Convert to Symbol** of a selection on several layers makes one symbol **with those layers
   inside it**, named and stacked as they were, registered at the centre of the combined box. The
   instance lands on the **topmost** selected layer, where its first selected element was; the
   other layers lose what was selected. On one layer it is unchanged (one inner layer,
   "Layer 1"). The issue asked which layer the symbol lands on: the topmost, as Animate does.

   *Rejected:* flattening everything onto one inner layer (it loses the stacking between
   layers, so masks and overlaps would change how the symbol looks).

7. **In the editor.**
   * Interface text goes through `tr!`/`trn!` (ADR 0040). "6 elements on 4 layers" has two
     counts but a template holds one plural: the layers' phrase is its own plural with its
     preposition ("on {n} layers"), placed by the outer one ("{n} elements {on_layers}"), so each
     language can inflect both and order them.
   * Every timeline row whose layer holds selected items shows a small square in the layer's
     colour after its name, as in Animate, and screen readers hear "has selected artwork".
   * Properties lists the selection from every layer and titles it "6 elements on 4 layers". For
     several elements, Position is the combined box's corner (a change moves them all), and
     Scale, Rotation, Alpha, Fill and Stroke say "mixed" when the elements differ (the fields
     show the first one's; a change applies to all).
   * The stage draws every selected element's box (or raw dots) whatever its layer; Free
     Transform's box covers them all.
   * `doc.info` reports `selection_layers`; plug-ins see the selected ids as before.

8. **Undo is one step per gesture**, as before: a drag across layers runs `element.translate`
   per frame and `draw.drop` on release inside one gesture.

## Consequences

* The Selection tool's marquee, Shift-click and Select All select across layers, and every
  command that reads the selection has a defined multi-layer behaviour (tested with a selection
  on two layers in `selection_tests.rs`, and in `panic_hunt_every_command`, which now also starts
  from raw drawings on two layers with a marquee's cut still provisional).
* Select All in a document with several layers takes them all (was: the active layer only).
* Measured on 1,440 raw shapes on 8 layers (optimised debug build, `selection_timings`):

  | Operation | Time |
  |---|---|
  | Select All | 0.06 ms |
  | `fix_selection` (after every command) | 0.05 ms |
  | Move frame, every layer | 0.32 ms |
  | Snapping move frame, every layer | 0.44 ms |
  | Marquee cut and take-back, every layer (461 parts, 202 cut) | 2.5 ms |

* Not in this slice:
  * a marquee or Select All that takes armature instances from layers that are not active;
  * Paste in Place across scenes keeping layers by name only (it uses the active layer and the
    ones below);
  * the plug-in `elements` view of other layers (it shows the active layer, as before);
  * "mixed" for every Properties row (filters, blending and looping show the first instance's).
