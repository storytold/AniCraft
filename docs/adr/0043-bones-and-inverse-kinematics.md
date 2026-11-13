# ADR 0043: Bone tool and inverse kinematics (symbol-instance armatures)

* Status: accepted (2026-11-10)
* Relates to: roadmap M7 (Bone/IK tool), ADR 0003 (file format), ADR 0004 (destructive
  actions), ADR 0016 (motion tweens), ADR 0027 (camera layer: the model for a special layer)

## Context

Animate's Bone tool links symbol instances into an **armature** on an armature (pose) layer.
Each bone starts at a joint on one instance and ends in the next; dragging a bone end moves the
chain by inverse kinematics; poses at frames are keyframes of the armature and tween between
them; joints can be limited and bones pinned. Behaviour was learned from Adobe's public help
pages and from observing the program; nothing was copied. The question is how to fit it into a
model where time is a function (`evaluate` is pure), every renderer and export draws a display
list, and every edit is a command.

## Decision

1. **Model.** `LayerKind::Armature` marks an armature layer, and `Layer.armature:
   Option<Arc<Armature>>` holds its bones (`#[serde(default, skip_serializing_if)]`, so older
   files read and write as before). A `Bone` joins a **parent** instance to a **child**
   instance: its `head` (the joint) is a point in the parent's local space, its `tail` (the end
   a drag pulls) a point in the child's, and `rest` is the child's transform in the parent's
   space as it was drawn. The child turns about the joint; an instance is the child of at most
   one bone, and the instance no bone ends in is the **root**, which keeps its own transform.
   Bones are listed parent first; `Armature::sanitize` (run on load) drops non-finite bones,
   repeats, second bones into one instance and cycles, sorts, and caps at 256 bones.
2. **Poses are joint angles on keyframes.** `Keyframe.pose` lists an angle per bone (radians;
   0 is the bone as drawn, measured against its parent, so turning a parent carries its
   children). Forward kinematics is `W_child = R_joint(angle) · W_parent · rest`, the rotation
   about the joint in the layer's space (so a scaled or flipped parent never shears its child).
   Between pose keyframes the angles tween linearly at the span's eased time (the existing
   classic tween and `Easing`); the root's transform tweens as any element does. At a keyframe
   the pose is exact.
3. **Pure evaluation.** `evaluate_guided` (behind `evaluate_layer_elements`, `evaluate`,
   `evaluate_player` and the HTML exporter) places an armature layer's instances by the pose at
   the frame. Every export, the GPU stage, onion skins and Test Movie see poses with no change
   of their own. Commands also write the posed transforms into the keyframe's elements, so the
   file holds what shows.
4. **Inverse kinematics is cyclic coordinate descent** (CCD; L.-C. T. Wang and C. C. Chen,
   *A combined optimization method for solving the inverse kinematics problem of mechanical
   manipulators*, IEEE Trans. Robotics and Automation 7(4), 1991; C. Welman, *Inverse
   Kinematics and Geometric Constraints for Articulated Figure Manipulation*, MSc thesis, Simon
   Fraser University, 1993). FABRIK (Aristidou and Lasenby, 2011) converges in fewer sweeps but
   moves joint *positions*, so angles must be recovered and limits re-projected after every
   step; CCD works on joint angles, which is what a pose stores, so a limit is a clamp per step
   and the result is a pose by construction. Each sweep turns the joints from the dragged bone
   upward so the effector points at the target; it stops within 0.01 px or when it stops
   moving, and a stall (CCD's blind spot: a straight chain whose target lies on its own line)
   bends the joints a little, at most four times, keeping the best pose seen. At most 2,048
   sweeps. An unreachable target leaves the chain stretched straight toward it, the root where
   it was. Measured from the rest pose: 0.007 ms per solve for 2 bones, 0.065 ms for 10, 4.7
   ms for 64, every target within 0.01 px. A drag starts each solve from the last pose, so it
   converges in far fewer sweeps.
5. **Pins.** A pinned bone and the bones it hangs from never turn in a solve, so its end stays
   put; a drag stops at the first such bone, and dragging a pinned bone itself is refused.
6. **Commands** (`bone.*`, a document namespace plug-ins may call): `bone.add {from, to,
   head?, tail?}` moves both instances to a new armature layer above them (one pose spanning
   from their keyframe to the end of the timeline, a classic tween on it) or adds the new one
   to an existing armature (in every pose, at angle 0). An instance that hangs from a bone has
   one joint, that bone's end: every bone from it starts there, wherever on the instance the
   drag began, so chains always join end to start (a root's bones start where they are drawn,
   by default at its registration point). Until #239 a given `head` won anywhere on the
   instance, so a drag started a few pixels off the end left the chain looking broken and
   turning about joints it didn't show. `bone.remove` takes a bone and those
   below it out of every pose; the instances they moved go to a normal layer of their own, as
   they look at the playhead; with the last bone the whole armature does and its layer goes.
   It asks first when the armature has more than one pose (undoable tier, ADR 0004).
   `bone.set_limits`, `bone.pin`, `bone.pose {bone, angle | rotate_by}` or `{dx, dy}` (moves
   the root), `bone.solve {bone, x, y, local?}` (headless IK) and `bone.info`. Pose edits go
   through `Session::editable_keyframe_in(KeyEdit::Pose)`: inside a pose tween they key the
   frame with the pose it shows, past the layer's end they carry the last pose on.
7. **The armature layer holds only its armature.** Artwork commands are refused there with a
   message that says how to pose instead (like the camera layer, ADR 0027); it can't become
   another kind of layer, and motion tweens are refused on it (poses tween with classic tweens).
8. **The shell.** The **Bone tool** (M) drags from one symbol instance to another to add a
   bone (starting on a bone's end continues the chain from it) and clicks to pick a bone. Bones
   draw over the artwork as wedges, widest by the round joint and narrowing to a smaller round
   end, in the armature layer's colour (green by default), the picked bone in the selection
   colour; they fade with other tools and on locked layers. The **Selection tool** drags a
   bone, or an instance a bone moves, to pose by IK: the grabbed point follows the pointer
   (a bone's end when the press is on it; along a bone, the point pressed, so the bone never
   jumps to put its end under the pointer, #239). The root instance, or the joint a root's bone
   starts at, moves the armature; each drag is one undo step. Properties shows a
   **Bone** section: the joint angle at the playhead, Constrain rotation with min and max, Pin,
   and Remove Bone; Delete removes the picked bone. The timeline shows the armature layer with
   a bone icon.

## Consequences

* Nothing that draws needed to change: poses become transforms in the layer evaluation.
* Files without armatures are written exactly as before; a pose on a keyframe of another kind
  of layer (pasted frames) is kept and plays no part; an armature on another kind of layer is
  dropped on load.
* Colour effects, names and other instance properties of armature members can't be edited on
  the armature layer in this slice (set them before rigging, or inside the symbol).

## Not yet

* ~~Shape armatures (bones inside a shape, with binding of its points) and the Bind tool~~:
  ADR 0071. Their bones join joint nodes inside the shape and reuse this ADR's poses, forward
  kinematics and CCD solver unchanged.
* ~~Spring and damping, joint translation (Animate's X/Y joint constraints)~~: ADR 0084.
  Animate's runtime armatures (poses that ActionScript drives), and XFL import of armature
  layers.
* ~~Editing instance properties of armature members in place~~ (colour effect, blending,
  filters: ADR 0084); armature layers under masks and motion guides; an IK drag that keeps pinned bones *below* the dragged one in place.
