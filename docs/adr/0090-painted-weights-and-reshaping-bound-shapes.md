# ADR 0090: Painted weights and reshaping a bound shape

* Status: accepted (2026-11-13)
* Relates to: roadmap M7 (Bone/IK tool), ADR 0071 (shape armatures, which this extends),
  ADR 0084 (springs, translation, and the proposal for mixed armatures), ADR 0043 (armatures),
  ADR 0042/0045 (Subselection), ADR 0004 (destructive actions), ADR 0005 (UI guidelines)

## Context

ADR 0071 binds a shape's points to its bones automatically and lets the Bind tool override
whole points: a point follows one bone, shares one more equally, or drops one. Artists rigging
a tail or a sleeve need finer control. They want *how much* a bone moves the points across a
region, with soft transitions between bones, and an exact number when it matters. They also
redraw the outline after rigging. ADR 0071 refused that: the binding was indexed by point, so
any edit that added or removed a point would have scrambled it.

Animate's Bind tool and Subselection-at-rest behaviour was learned from Adobe's public help
pages and from watching the program. The weight brush follows what artists know from 2D and 3D
skinning tools in general (add / subtract / smooth with a soft falloff), not any one product's
code. Nothing was copied.

## Decision

1. **The binding stays where ADR 0071 put it.** `Skin.points` is still one `Binding` per anchor:
   up to four `[bone, weight]` pairs summing to 1, and the `manual` flag. Painting only changes
   the numbers, so **the file format is unchanged**: older files open as they are, and an older
   AniCraft opens newer files. The new code is pure functions in `anicraft_doc::weights`, with
   no state and nothing in `evaluate`. The posed outline is still ADR 0071's linear blend
   skinning, from the weights as they are.
2. **The brush** (`weights::paint`). A dab at a stage point reaches every point within its
   radius, measured where the stage shows the points at the frame (posed), so the brush
   paints what the artist sees. The falloff is `(1 − (d/r)²)²`: 1 at the centre, 0 at the rim,
   with no kink at either end, times the strength (0–100 %). *Add* raises the bone's share by
   that much and *Subtract* lowers it, additively, so 100 % at the centre sets the share
   outright. *Smooth* moves it that fraction toward the falloff-weighted mean share of the
   points under the dab, taken before the dab changed anything. The result doesn't depend on
   the order of points in the path.
3. **Normalisation per point** (`weights::with_share`). After every change the bone keeps the
   share it was painted to, and the point's other bones share the rest in the proportions they
   had. If the point followed only the painted bone, the rest goes to the bones automatic
   binding would give it without that bone, its nearest others. So Subtract always does
   something on a shape with two or more bones, and the point stays put the way a nearby bone
   moves it, which is what an artist expects. Then ADR 0071's rule applies: the strongest four
   are kept and they sum to 1. With no other bone at all, the bone keeps the whole point, or
   none of it at 0 (the point stays where it was drawn, as `bone.bind remove` leaves it).
   Painted points become hand-bound (`manual`), so adding a bone later doesn't undo the
   painting.
4. **Strokes and undo.** The shell spaces dabs a quarter of the radius apart along the
   pointer's path, so a stroke paints the same at any speed. It sends them as
   `bone.paint_weights` calls inside one gesture, which makes a stroke one undo step. A click
   is one dab. Headless callers give a list of dab centres (at most 4096 per call), so
   painting is scriptable and deterministic. Measured (release, this machine): 32 dabs of
   radius 40 over a 20,000-point shape (the limit), 1.15 ms.
5. **Numbers for the keyboard.** `bone.set_weight {bone, points, weight}` sets a bone's share of
   points exactly, normalising the same way. Properties › Bone shows a *focused point* (clicked
   with the Bind tool, or typed by number), the bone's share of it as a percentage field, and
   every bone the point follows with its share. Painting and binding move the focus to the point
   touched, and the stage rings it. This meets the accessibility rule (ADR 0064): everything the
   brush does can be done with typed numbers.
6. **The heat overlay.** With Paint Weights on, the picked bone's share shows on the outline and
   its points as a heat ramp: blue (none), cyan, green, yellow, red (all). It is drawn over the
   artwork, so it is the same in every palette (`theme::HEAT`, the only place its colours live),
   and it updates live as the stroke goes. The brush shows as a ring of its radius on the
   stage. Off, the Bind tool shows ADR 0071's squares and triangles.
7. **Reshaping at rest** (`weights::reshape`, `cmds_bone::reshape`). The Subselection commands
   on a drawing object (`path.move_points`, `path.add_point`, `path.delete_points`,
   `path.convert_point`) recognise the shape of a shape armature on the active layer. Then they
   edit the skin's rest outline instead of a keyframe, and carry the binding over:
   * the same number of anchors (moving points and handles, smooth/corner): every binding by
     index;
   * otherwise, anchors are matched old to new in path order by exact position (each new
     anchor takes the first old anchor after the last match that sits where it does). Deleted
     anchors drop their bindings. Each new anchor blends the bindings of the nearest matched
     anchors on either side along its subpath (wrapping round a closed one), weighted by
     distance along the outline between them, then normalised. It is hand-bound if either
     neighbour is. A new subpath with no neighbours is bound automatically.

   This is a general diff, not a contract with each command, so later point commands get it
   for free. Then every keyframe is re-bent from the new rest outline. **At rest the shown
   outline is the edited path bit for bit**: `deform` returns the very `Arc` of the rest path.
   Reshaping needs every bone at rest at the playhead (no angle, no slide). At a posed frame the
   command refuses with a message saying so: inverting a blend of bone matrices to map an edit
   back to the rest pose isn't well defined where weights mix, and Animate also edits bound
   shapes at rest. Each edit is one undo step, and none destroys anything, so none asks
   (ADR 0004).
8. **Commands** (in the `bone.*` namespace plug-ins may already call; the `path.*` ones were
   already callable): `bone.paint_weights {bone, at, radius?, strength?, mode?, frame?}` and
   `bone.set_weight {bone, points, weight}`. `bone.info {points: true}` already lists each point's
   bones and weights. Both are covered by the `panic_hunt_every_command` fuzz.

## Instances and shapes in one armature: proposed again, not built

It still doesn't fit cleanly, for the reasons ADR 0084 gives. `Armature.skin` is one shape, and
`place`, `roots`, `sanitize`, `bone.remove`, `bone.add` and the Bind and Bone tools all branch
on "skinned or not". Mixing the two needs `Armature.skins: Vec<Skin>` keyed by shape, nodes
that may be an instance or a joint node inside a shape, rules for what removing a bone frees
(instances to a layer, a shape's points rebound), and a Bone tool drag from a shape's bone end
onto an instance. That changes the file format (a list where there was one skin) and every
branch above. A slice whose subject is weights shouldn't carry it, so it stays its own ADR and
PR. Nothing in this ADR makes it harder: weights and reshaping are per skin.

## Consequences

* No renderer, exporter, the GPU stage or the file format changed. Weights and the rest outline
  are data ADR 0071 already stored, and every output sees them through `evaluate`.
* Hand-bound points are more common now (anything painted). **Rebind All** (`bone.bind auto`)
  still returns every point to automatic binding in one undoable step.
* Matching by exact position can mismatch when an edit both moves and adds points at once and
  a new point lands exactly on an old one further along. No single command does that today,
  and the result would still be normalised, finite weights.

## Not yet

* Instances and shapes in one armature (above); more than one shape per armature.
* Reshaping at a posed frame (the Pen's Alt-click to add a point works, as it is
  `path.add_point`); drawing new subpaths into a bound shape; a weight brush that paints across several bones at once (normalise-locked bones, mirror).
* A filled heat mesh across the shape's interior (the overlay is on its outline and points),
  dragging points onto a bone with the Bind tool, a global falloff setting for automatic
  binding.
