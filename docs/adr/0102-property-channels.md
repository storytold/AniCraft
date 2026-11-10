# ADR 0102: Property channels — layer transforms, camera, scene range

* Status: accepted (2026-11-10)
* Relates to: ADR 0003 (file format v5), ADR 0101 (Moho import), ADR 0016 (motion tweens —
  the other property-curve model), ADR 0027 (the camera layer — the Animate-style camera this
  composes with: the imported Moho camera rides beside it as scene data), the Moho parity plan
  (fork plan › stage 2)

## Context

Moho animates *properties*: every layer transform, the camera, every bone and every mesh
point carries its own little timeline — a `{when[], val[], interp[]}` channel of keys.
AniCraft animates *artwork*: a whole keyframe tweens into the next one, elements move by
their own transforms. The stage-1 Moho import therefore froze every animated document at one
frame and counted the rest in the report. Without property channels there is no Moho layer
animation, no camera, and no foundation for bones (stage 4) or switch/point animation
(stage 5).

The interpolation semantics come from Moho's public scripting documentation (InterpSetting
and the `INTERP_*` constants): modes 0 Linear, 1 Smooth, 2/7/8 the ease family, 3 Step, 4
Noisy, 5 Cycle, 6 Pose, 9 Bézier, 10 Bounce, 11 Elastic; per-key `hold` frames and `interval`
(on 2's) fields; Bézier keys carrying per-component tangent handles (out/in angle and
percentage). Channel key frames are 0-based (corpus: CharWalk keys at 0..45, the Lua API's
`SetValue(value, 0)`). Moho's default camera zoom of 2 draws the canvas one-to-one —
verified by the corpus three-way gate (Moho canvas ↔ its preview ↔ our import at 320
px/inch).

## Decision

1. **Two animation models, on purpose.** Keyframe spans keep driving *artwork* — elements,
   tweens, shape morphs, the timeline UI. New `doc::channel::Channel<T>` drives *properties*
   — for now each layer's transform and the scene camera; bones, switches and points come
   later. This is the same split Moho and Animate's motion editor draw. One model would
   flatten the other: spans can't express per-property curves, channels can't express
   element-at-a-time authoring.
2. **`Channel<T>` is pure data with a pure sampler.** Sorted keys, one per frame; `sample`
   maps a frame to a value, no state, inside the `evaluate(doc, frame)` model. `T` may be a
   scalar, a `Vec2` (components ease independently — Moho's `b[]` is per-component), or a
   stepping value such as `bool`. Interp modes map to `Linear`, `Smooth` (auto-smoothed
   Catmull-Rom — Moho's exact formula is not public, so this is an approximation the parity
   gates calibrate), `Hold`, and `Bezier` handles (out on the key, in-hands from the next
   key's `ai`/`pi`, solved through `Easing::CubicBezier`, overshoot allowed). Moho's ease
   family approximates as Smooth, noisy/cycle/bounce/elastic hold, stagger drops — each
   counted in the import report. `hold` frames and `interval` on-2's are honoured; handles
   sanitize on load (percents in 0..1, angles bounded, non-finite values dropped).
3. **Layer transforms live on the layer, composed at the top.** `Layer` gains `translation`,
   `scale`, `rotation`, `shear`, `flip_h`, `flip_v` channels plus the `origin` pivot and
   Moho's `camera_immune` flag; `Layer::transform_at(frame)` composes them the way Moho does
   (translate, rotate, shear, scale, around the origin) and `evaluate_scene` applies the
   result to each *top-level* layer's items. AniCraft's folders don't render, so an animated
   *container* keeps the stage-1 static bake and the report says so until group rendering
   (parity stage 3); nested animated layers compose outside their ancestors' baked transforms
   and are likewise counted. Static layers import exactly as stage 1 did.
4. **The camera is a scene property.** `Scene.camera` holds `track` (stage pixels — Moho's
   inch position through the full coordinate mapping, canvas centre included), `zoom`
   (Moho's units, 2 = neutral, the view scales by `zoom/2`) and `roll` (screen-clockwise
   radians). The view pivots around the stage centre; `camera_immune` layers skip it. Moho's
   camera z (depth parallax) and pan/tilt have no 2D slot and are reported. The stage's own
   zoom/pan stay view state, untouched by all of this.
5. **Scenes carry markers and a playback range.** Moho's `timeline_markers` string channel
   becomes `Scene.markers` (one label per frame; markers are scene data, unlike keyframe
   labels), and `project_data.start_frame`/`end_frame` become `Scene.frames` — stored only
   when they differ from the imported content's own length. `Document::frame_count`,
   `locate` and `Session::frame_count` honour the range, so a 44-frame CharWalk plays 44
   frames.
6. **Files: format v5.** All new fields are optional keys; older files read exactly as
   before (no layer transform, the plain unmoving camera). Commands: `camera.set_track`,
   `camera.set_zoom`, `camera.set_roll`, `camera.clear`; `layer.set_transform`,
   `layer.clear_transform`; `scene.set_frames`, `scene.clear_frames`,
   `scene.add_marker`, `scene.remove_marker` (the camera namespace is in the plug-in policy's
   document set). Moho values ride in the sidecar untouched, so a later milestone can
   re-project anything these channels approximate.

## Consequences

* Import fidelity rises monotonically: animated Moho layers and cameras now play in
  AniCraft, exports included, because the display list already carries the composed
  transforms. The in-repo gate samples linear segments frame by frame against the file's own
  data (exact to f32 rounding); the Moho-runtime halves of the stage-2 gates (a Lua probe
  against `Channel::sample` at ±1e-6, and rendered-frame comparison for the camera) need
  Moho on the owner's machine and stay open on the parity plan.
* The Smooth approximation and the ease-family/noisy/cycle stand-ins are *documented
  approximations*: the report counts them, the sidecar keeps the truth, and the render
  comparison milestones calibrate them.
* Bones (stage 4) will reuse `Channel<f32>`/`Channel<Vec2>` directly; switch selectors
  (stage 5) will extend it with a string channel or resolve it at import.
