# ADR 0084: Bone springs, joint translation and editing armature members in place

* Status: accepted (2026-11-13)
* Relates to: roadmap M7 (Bone/IK tool), ADR 0043 (armatures), ADR 0071 (shape armatures),
  ADR 0003 (file format), ADR 0004 (destructive actions), ADR 0056 (colour effects)

## Context

ADR 0043 and ADR 0071 left three things Animate artists reach for when rigging: **joint
translation** (a joint that slides along X and/or Y within limits, like a piston or a neck that
extends), **springs** (per-bone strength and damping: a tail that lags behind and swings after
the body moves), and **editing a member's look** (colour effect, filters, blending) on the
armature layer, which ADR 0043 refused with every other artwork edit. Behaviour was learned from
Adobe's public help pages and from watching the program; nothing was copied. The hard
constraint is ADR 0001's: time is a function. A spring is a simulation, and simulations carry
state from frame to frame; `evaluate` must not.

## Decision

1. **Joint translation is part of the pose.** `Bone.slide_x` / `slide_y: Option<Limits>` (in
   pixels, at most ±10,000; `None`: the joint doesn't slide that way; ±10,000 is "enabled,
   unconstrained"). `JointAngle` gains `x`, `y`, the slide at that pose, written only when not
   0 (`#[serde(default, skip_serializing_if)]`), so poses without slides read and write exactly
   as before. The slide is along the **x and y axes of what the bone starts on** (the parent
   instance, or the parent joint node of a shape armature), normalised to unit length so the
   limits are pixels whatever the parent's scale, and turning with the parent so a piston keeps
   its line as the arm holding it swings. Forward kinematics becomes
   `W_child = R_joint'(a) · T(d) · W_parent · rest`, with `d` the slide in layer space and
   `joint'` the joint moved by `d`. With no slide it is ADR 0043's formula, computed the same
   way, so existing armatures place bit for bit. Slides tween linearly with the angles and are
   clamped to the limits on evaluation, as angles are.
2. **IK slides only where turning can't reach.** `armature::solve` runs ADR 0043's CCD on the
   angles alone first; only when that leaves the effector farther than the tolerance and a
   movable bone slides does a second round of sweeps also slide each joint, after turning it,
   by the projection of the remaining error on its enabled axes (the exact 2×2 solve when both
   are on), clamped. A target the joints reach by turning is solved exactly as before, which
   keeps posing predictable: a bone that may slide still bends first and stretches only when
   pulled beyond its reach, and a joint that may not turn (limits 0…0) but slides is a piston.
3. **A spring is evaluated, never stored.** `Bone.spring: Option<Spring { strength, damping }>`
   (each 0…1, shown 0–100). Each sprung bone carries an angle `φ` off its keyed angle, driven
   by the keyed motion's accelerations (the joint's linear acceleration across the bone and the
   keyed direction's angular acceleration: the inertia of a pendulum on a moving pivot) and
   pulled back by a damped spring: `φ'' = −θ_key'' − (a_joint · n)/L − ω²φ − 2ζωφ'`. Strength
   maps to the natural frequency (0.5 Hz to 10 Hz, even in pitch); damping `d` to the ratio
   `ζ = ½·d/(1−d)` (0 swings on, ⅔ is critical, 1 is infinite: the swing never starts). Springs
   chain: a bone's keyed direction is taken from its parent *as swung*, so a tail whips.
   `spring::sprung_pose(layer, frame, fps)` replays the motion from a fixed start for every
   frame it is asked about: at most six seconds back (720 frames at most), never before the run
   of keyframes the frame is on, two frames before that only to know the motion's speed. Every
   step is a fixed function of the frames before it, so **seeking to a frame gives exactly the
   bits playing to it does**, and the stage, the player, Test Movie and every export agree with
   no cache. Motion in the older half of the window counts less and less (a smoothstep taper of
   the drive), so a swing fades out smoothly instead of jumping when its cause leaves the
   window; this bounds a damping-0 spring's ring to six seconds, the price of statelessness.
   The integrator is backward Euler with four steps per frame, stable for any strength, damping
   and frame rate; the swing is clamped to half a turn and stopped by the joint's limits. Time
   is in seconds (the stage's frame rate), so a spring looks alike at 24 and 48 fps.
   Measured (release, this machine): 16 sprung bones, 24 fps, 144 frames replayed: 0.33 ms per
   evaluated frame.
4. **Where springs apply.** `evaluate_shown(layer, frame, guide, fps)` is `evaluate_guided`
   with the sprung pose; `eval_timeline` (so `evaluate`, every raster and vector export, onion
   skins, the GPU stage), the player and the HTML canvas exporter (which bakes every frame) use
   it, so every export sees the swing with no change of its own. Commands keep editing the
   **keyed** pose through `evaluate_guided`: springs never write to the file, and the solver
   works on what is keyed. The stage draws the bones swung, as the artwork shows. `Armature.
   springs_off` (Properties › Bone › Springs: Enable, `bone.enable_springs`) plays the keyed
   poses exactly, as Animate's per-armature spring switch does, for posing without the swing.
   Damping 100, strength 0 or springs off give the keyed pose bit for bit (tested).
5. **Members are edited in place on every pose.** `element.set_color_effect`, `set_alpha`,
   `set_blend` and `set_filters` work on armature members through
   `Session::selected_props_mut`: on an armature layer the selected members change on every
   keyframe of the layer, in one undo step, with no frame keyed, the bones and poses untouched.
   The look is a property of the member, not of a pose, which is what an artist tinting a limb
   expects, and it leaves nothing for a pose tween to blend. Every other artwork edit is still
   refused there (ADR 0043). Picking a bone selects the instance it moves, and Properties shows
   only the member's Colour effect, Blending and Filters, with a line saying its bone places it.
6. **Commands** (in the `bone.*` namespace plug-ins may already call): `bone.set_translation
   {bone, axis, enabled?, constrain?, min?, max?}`, `bone.set_spring {bone, strength?,
   damping?}`, `bone.enable_springs {enabled, layer? | bone?}`, `slide_x` / `slide_y` on
   `bone.pose`, and `bone.info` reports slides, translation limits, springs, each bone's swing
   at the frame and whether springs are on. Each is one undo step; none destroys anything, so
   none asks (ADR 0004).

## Instances and shapes in one armature: proposed, not built

It doesn't fit cleanly in this slice. ADR 0071's skin is the armature's *one* shape, its joint
nodes are no elements, and `place`, `roots`, `sanitize`, `bone.remove` and the Bind tool all
branch on "skinned or not". Mixing them needs nodes that may be either an instance or a joint
node inside a shape, more than one skin per armature, and rules for what removing a bone frees.
The proposal: `Armature.skins: Vec<Skin>` keyed by shape, a bone's `parent`/`child` naming an
instance, a shape, or a node of one, `place` unchanged (it already places both), and the Bone
tool allowed to drag from a shape's bone end onto an instance. It is its own ADR and PR.

## Consequences

* Files without slides or springs are written exactly as before; an older AniCraft ignores the
  new fields and plays the keyed poses without slides or springs (the file holds the keyed
  poses, and each keyframe's elements as posed).
* Evaluating a sprung armature costs a replay of up to six seconds of its motion per frame;
  armatures without live springs cost nothing extra.
* Hostile files are repaired on load: non-finite limits and springs go, limits are clamped to
  ±10,000 px, slides in poses to the same, strengths and damping to 0…1.

## Not yet

* Instances and shapes in one armature (above); gravity, wind and per-armature spring presets;
  springs on the root's own motion of a shape armature's outline points (only bones swing).
* An IK drag that slides joints by preference rather than only to reach; translation limits
  drawn on the stage.
* Runtime armatures driven by scripts, XFL import of armatures (with their springs and
  translation), and baking the swing into keyframes (Convert to Keyframes) for formats that
  can't evaluate it; every export today evaluates through `evaluate`, so none needs it yet.
