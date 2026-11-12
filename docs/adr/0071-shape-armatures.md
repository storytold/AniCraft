# ADR 0071: Shape armatures (bones inside a shape) and the Bind tool

* Status: accepted (2026-11-12)
* Relates to: roadmap M7 (Bone/IK tool), ADR 0043 (symbol-instance armatures, which this
  extends), ADR 0031 (merge drawing), ADR 0004 (destructive actions), ADR 0003 (file format)

## Context

ADR 0043 joins symbol instances with bones. Animate can also draw bones inside one raw shape,
so the outline bends like a tail, a limb or a strand of hair. Its points follow the bones
automatically, and its Bind tool changes which points follow which bone. Behaviour was learned
from Adobe's public help pages and from watching the program; no code was copied. The questions
are how to model the bones, the binding and the bent outline without a second IK solver, while
keeping `evaluate` pure and drawing the same shape everywhere.

## Decision

1. **Same armature, different nodes.** A shape armature is an ordinary `Armature` (ADR 0043)
   on an armature layer, with `Armature.skin: Option<Skin>` (`#[serde(default,
   skip_serializing_if)]`, so files without it read and write exactly as before). Its bones join
   **joint nodes**, not instances. The root bones start on the shape's own element id, and
   every other bone starts on the node of the bone it hangs from. The node ids come from the
   document's id counter and are counted on load, so they never collide with new ids. Every
   node's rest frame is the shape's local space (`rest` is the identity), so a bone's head and
   tail are points of the shape as drawn. Forward kinematics (`armature::place`), inverse
   kinematics (`armature::solve`, CCD with limits and pins), poses keyed per frame, tweens
   with easing and every `bone.*` command are ADR 0043's, unchanged. `place` now also places
   nodes that are no element, and `bones_at` uses it to draw them.
2. **The skin.** `Skin { shape, rest, points }` keeps the outline as drawn (`rest`, the shape's
   space) and one `Binding` per **anchor** of it (every `MoveTo`/`LineTo`/`QuadTo`/`CubicTo`
   end, in path order): up to four `[bone, weight]` pairs summing to 1, plus a `manual` flag.
   Weights live on the armature, not on keyframes: one binding serves every pose. Artwork
   commands are refused on armature layers (ADR 0043), so the outline, and therefore the point
   indices, never change under the binding.
3. **Automatic binding** (as Animate does): each bone weighs `1 / max(d, 0.5 px)⁴`, where `d`
   is the anchor's distance to the bone's segment at rest. The strongest four are kept,
   anything under 1 % is dropped, and the rest are normalised. A point near one bone follows
   it alone, and a point near a joint is shared by the bones that meet there. The fourth power
   was chosen over the inverse square so that a long bone's middle doesn't drift with distant
   bones. Adding or removing a bone rebinds every point not bound by hand. A hand-bound point
   keeps its binding (bones that go away leave it, and one left with none goes back to
   automatic). A point unbound by hand on purpose stays where it was drawn.
4. **The bent outline is linear blend skinning** (N. Magnenat-Thalmann, R. Laperrière and
   D. Thalmann, *Joint-dependent local deformations for hand animation and object grasping*,
   Graphics Interface 1988; J. P. Lewis, M. Cordner and N. Fong, *Pose space deformation*,
   SIGGRAPH 2000, §2). Each bone's skinning matrix is its node's transform under the pose (the
   identity at rest). Each anchor moves by the weighted sum of its bones' matrices. Each cubic
   handle moves with the anchor it leaves or enters, and a quadratic's control point with the
   mean of its two ends, so curves stay smooth through a bend and the path keeps its commands.
   **At rest the outline is the drawing bit for bit**: the rest pose returns the very `Arc` of
   the drawn path, and a point whose bones didn't turn is left untouched rather than recomputed.
   The shape's transform is untouched, and strokes are drawn after the outline, so **strokes
   keep their width** (width profiles keep their place along the path). Dual-quaternion
   skinning avoids LBS's thinning at sharp bends, but for 2D rotations about joints the blend
   of two rotations about nearby centres differs little, and LBS is what a 2D animator expects.
   It is the place to start if collapse at sharp bends becomes a problem.
5. **Pure evaluation, every output.** `armature::apply` (behind `evaluate_guided`, so
   `evaluate`, the player, the HTML exporter, every raster and vector export, onion skins, Test
   Movie and the GPU stage) replaces the shape's path with the outline bent by the pose at the
   frame, tweened between pose keyframes. Commands also write the bent outline into each
   keyframe's element, as ADR 0043 does with transforms, so the file holds what shows. It is
   always recomputed from `rest`, so poses never accumulate drift. Measured (release, this
   machine): binding 2,000 anchors to 16 bones takes 0.83 ms, and bending them takes 0.042 ms
   per frame.
6. **Merge drawing treats an armature's shape as an object.** The first bone moves the shape
   (raw or a drawing object) to a new armature layer of its own with `merge` off. Shapes drawn
   on other layers never cut it, and removing the last bone gives it back as a drawing object,
   bent as it looked at the playhead.
7. **Commands** (in the `bone.*` namespace plug-ins may already call):
   `bone.add_in_shape {shape | parent, head?, tail, name?}`. The first bone needs `head`
   inside the shape (on its fill or its line). With `parent` the joint defaults to that bone's
   end. Points given at a posed frame are mapped back through the parent node to the shape as
   drawn, and a new bone is at rest (angle 0) in every pose. Shapes are limited to 20,000
   points, with a message pointing to Optimize. Then
   `bone.bind {bone, points?, mode: only | add | remove | auto}`: `only` makes the points
   follow just that bone, `add` gives the bone an equal share alongside the bones they already
   follow, `remove` takes the bone away (renormalising), and `auto` rebinds them automatically.
   Each is one undo step. `bone.info` reports the armature's `shape`, how many points each bone
   moves, and with `points: true` every point on the stage with its bones. `bone.remove` on a
   shape armature rebinds the remaining points. It makes no layer for freed instances, because
   there are none, and it still asks first when the armature has more than one pose.
   `bone.pose`'s `dx/dy` moves the shape, which is the root.
8. **The shell.** The **Bone tool** draws a bone inside a shape by dragging from inside it
   (from a bone's end it continues the chain; releasing anywhere sets the end). The
   **Selection tool** bends the shape by dragging its bones and moves it by dragging the
   shape. The new **Bind tool** (Shift+M, next to the Bone tool) shows every point of shapes
   with bones: a square for a point that follows one bone, a triangle for one shared by
   several (Animate's convention). Clicking a bone picks it and fills the points it moves in
   the selection colour. Clicking a point makes it follow only that bone, Shift-click shares
   the bone, Ctrl-click (⌘ on macOS) takes the bone away, and a box drag does the same to
   every point inside it. Properties › Bone shows how many points the bone moves, plus **Bind
   Tool** and **Rebind All** buttons. Rebind All is undoable, so it follows ADR 0004's
   routine tier and doesn't ask.

## Consequences

* No renderer, exporter or the GPU stage changed: the bent outline is just a path in the
  display list. Hit testing, onion skins and selection outlines follow it.
* Files without shape armatures are written exactly as before. An older AniCraft reading a
  newer file ignores `skin` (an unknown field). It shows each keyframe's bent outline (the file
  holds what shows), and its bones draw but move nothing, so the file still opens.
* A skin whose shape is a bone's node, or whose outline is over the point limit, is dropped on
  load. Bones of a shape armature that don't hang from the shape are dropped. Weights of
  missing bones, non-finite or non-positive weights and repeats go, and the rest renormalise.

## Not yet

* Instances and shapes in one armature; more than one shape per armature; reshaping the
  outline after binding (Animate allows Subselection edits on the rest pose).
* Dragging points onto a bone with the Bind tool (only clicks and boxes for now), painting
  weights, and a falloff setting.
* Dual-quaternion or other volume-keeping skinning, spring, joint translation, runtime
  armatures, XFL import of shape armatures (`<IKTree>` on a shape).
