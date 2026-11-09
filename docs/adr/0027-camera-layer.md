# ADR 0027: Camera layer and layer depth

* Status: accepted (2026-11-09)
* Relates to: roadmap M7 (camera layer with depth), ADR 0003 (file format), ADR 0004
  (destructive actions), ADR 0016 (motion tweens), ADR 0019 (GPU stage), ADR 0024 (player)

## Context

Animate has a camera: a special layer at the top of the timeline whose camera views the stage.
Pan, zoom and rotate it, animate it with tweens like any object, tint the whole view with its
colour effect, and give layers a depth so the camera's moves shift near and far layers by
different amounts (parallax). Behaviour was learned from Adobe's public help pages and from
observing the program; nothing was copied. The question is how to fit it into a model where
time is a function (`evaluate` is pure), every renderer and export draws a display list, and
editing tools work in stage space.

## Decision

1. **A layer kind and an element kind.** `LayerKind::Camera` marks the camera layer;
   its keyframes hold one `ElementKind::Camera` element. The element's transform is the camera
   on the stage, `T(x, y) · R(rotation) · S(zoom)` (the camera's centre at `(x, y)`), and its
   colour effect's tint colours the view. Because the camera is an element, classic tweens and
   motion tweens (ADR 0016) animate it with no new code: decomposed interpolation gives a
   linear zoom, a straight pan and the shortest turn; the Motion Editor shows the zoom as
   Scale X. Scale Y, skew and opacity of a camera play no part (Animate's camera has one zoom
   and no opacity). A camera keyframe with no camera element is the camera at rest.
2. **One camera, on top.** `Timeline::repair_camera` (run by `repair_masks`, which every
   structural edit already calls, and on load) keeps the invariant: the first camera layer is
   moved to the top and out of any folder; extra camera layers become normal layers (nothing is
   deleted). Only scenes have cameras; a symbol's camera layer loads as a normal layer. The
   camera layer holds only the camera: drawing, pasting artwork frames onto it or moving them
   there is refused with a message, and camera frames can't go onto artwork layers.
3. **The movie is the stage seen through the camera; editing stays in stage space.**
   `evaluate` (every export: PNG, sequences, GIF, video, sprite sheets, SVG, Lottie),
   `evaluate_player` (Test Movie, with button hit areas where the pointer sees them) and the
   new `evaluate_scene_camera` place each top-level layer through
   `Camera::view(stage, depth) = T(mid) · S(zoom) · R(−rotation) · T(−centre')`, and the
   camera's tint is the colour effect around the whole timeline. `evaluate_scene` (the stage
   while editing, picking, bounds, every tool) is unchanged, so no tool needs to know about the
   camera. A hidden camera layer is a camera turned off. The camera at rest is skipped, so a
   document with an unmoved camera evaluates exactly as one without.
4. **Layer depth is pan-only parallax.** `Layer.depth` (pixels behind the stage, −500 to
   10,000, `#[serde(default, skip_serializing_if = "is_zero")]`) scales how far the camera's
   pan moves the layer: `centre' = mid + (centre − mid) · FOCAL / (FOCAL + depth)`, with
   `FOCAL` = 1,000 px. A layer 1,000 px back moves half as far; zoom and rotation are the same
   at every depth, and sizes don't change (Animate's "maintain size" behaviour, the one that
   lets artists set depth without re-scaling artwork). Depth applies to a scene's own layers;
   artwork inside a symbol moves with its instance.
5. **Commands** (`camera.*`, a document namespace plug-ins may call): `camera.add` (a camera
   layer on top, at rest, spanning the timeline; no-op if there is one), `camera.remove`,
   `camera.set` (absolute `x`, `y`, `zoom` %, `rotation` °, `tint`, `tint_amount` %, or
   relative `dx`, `dy`, `zoom_by`, `rotate_by`, or `reset`; at the playhead or `frame`),
   `camera.info`, and `layer.set_depth`. `camera.set` edits through the session's editable
   keyframe, so inside a classic tween it keys the frame and inside a motion tween it becomes
   property keys, like editing any tweened object. Removing an animated camera (more than one
   keyframe, or a tween) asks first (undoable tier, ADR 0004); a camera at rest goes without
   asking.
6. **The shell.** The camera frame is drawn on the stage as an outline in the camera layer's
   colour (orange), with its centre marked. The **Camera tool** (C) drags to pan, turns the
   camera with a round handle above the frame and zooms with the wheel; each drag, and each
   burst of wheel turns, is one undo step. Picking the tool adds a camera if there is none, as
   does the camera toggle in the timeline header (which removes it when on). **View › Preview
   Through Camera** renders the stage tiles through `evaluate_scene_camera` (jobs keyed apart
   by `FrameJob::through_camera`), so the GPU stage and the CPU oracle show the movie's
   picture; there a drag moves the picture under the pointer. While previewing, artwork tools
   pause with a message (what they edit is in stage space). Properties shows a Camera section
   (position, zoom, rotation, tint, reset, preview) and, once a scene has a camera, a Depth row
   for the active layer.

## Consequences

* Every renderer and exporter sees the camera with no change: they draw display lists, and
  the GPU stage draws the same fills (parity tests include a camera scene).
* Files without a camera, or with every layer at depth 0, are written exactly as before.
* The colour model is the existing one: a camera tint and an element's own tint compose by
  keeping the stronger (`ColorEffect::concat`), so a strongly tinted instance shows its own
  tint over the camera's.

## Not yet

* Editing artwork while previewing through the camera (tools would map the pointer through
  the inverse view; parallax makes that per layer).
* Animate's other camera colour effects (brightness, advanced colour), a perspective
  ("scale with depth") mode, depth inside symbols, a Layer Depth panel listing every layer,
  attaching layers to the camera (HUDs that ignore it), and XFL import of camera layers.
