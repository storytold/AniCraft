# ADR 0082: Camera, second slice: layer depth in perspective, attaching layers, colour

* Status: accepted (2026-11-12)
* Relates to: ADR 0027 (camera layer), ADR 0056 (colour effects), ADR 0016 (motion tweens),
  ADR 0024 (player), ADR 0004 (destructive actions), roadmap M7

## Context

ADR 0027 shipped one camera per scene with pan-only parallax ("maintain size"): a layer's depth
scaled how far a pan moved it, while zoom and rotation were the same at every depth. Its "not
yet" list: perspective depth, a Layer Depth panel, depth over time, attaching layers to the camera
(HUDs), camera colour effects beyond tint, editing while previewing, and XFL import. Behaviour was
learned from Adobe's public help pages on the camera and Layer Depth panel and from observation;
nothing was copied.

## Decision

1. **A pinhole camera with a fixed focal length.** At zoom `z` the camera is `FOCAL / z` from the
   stage plane (`FOCAL` = 1,000 px). A layer `depth` px behind the stage is drawn as a pinhole
   would see it, enlarged so that **the camera at rest shows every layer as drawn**: with
   `d = FOCAL / z + depth`, a stage point `p` lands at
   `mid + R(−rotation) · ((FOCAL + depth) / d · (p − mid) − FOCAL / d · (centre − mid))`.
   * A pan by `Δ` at zoom 1 moves a layer by `Δ · FOCAL / (FOCAL + depth)`: exactly ADR 0027's
     parallax, so files without zoom look the same.
   * A zoom grows the stage plane by `z` and a deeper layer by less (a far sky barely changes),
     a nearer one by more.
   * **Depth 0 is the stage view bit for bit**: `Camera::view` returns `stage_view` for depth 0
     before any perspective arithmetic, so artists who never touch depth see no change.
   * A layer the camera has reached or passed (`d < NEAR` = 10 px, a near layer under a strong
     zoom) isn't drawn; every view is finite for any depth, NaN depths read as 0.
   * We keep the rest-normalisation (Animate's "maintain size") rather than shrinking deep
     layers at rest: depth then never changes the composition the artist drew, only how the
     camera's moves affect it. That is the behaviour that lets depth be set late without
     re-scaling artwork.
   * **Changed behaviour:** ADR 0027 kept zoom equal at every depth. A file that has both a
     depth and a zoomed camera now renders differently. ADR 0027 shipped one day earlier in
     the same unreleased alpha, so we take the one behaviour now rather than carry a
     "pan-only" mode.
2. **Depth draws in order.** `Timeline::draw_order(frame)`: deepest first, a stable sort, so
   ties (and every layer at 0) keep the timeline order. A masked layer sorts with its mask, so a
   mask stays with what it masks. The stage while editing, every export, the player's hit
   testing and the HTML canvas export use the same order, so stacking matches everywhere.
   Nested timelines aren't sorted (depth belongs to a scene's layers) and allocate nothing.
3. **Depth over time.** `Layer.depth_keys: Vec<DepthKey { frame, depth }>` (sorted, one per
   frame, at most 4,096, clamped on load; `#[serde(default, skip_serializing_if = empty)]`).
   With keys, `Layer::depth_at(frame)` is linear between keys and held outside them; without,
   it is `Layer.depth`. Keys live on the layer by frame number, not on keyframes: depth is a
   property of the layer's place in the scene, it must tween across keyframe boundaries, and
   it must not be lost when artwork keyframes are cut and pasted. The cost: inserting frames
   doesn't move depth keys yet (listed below).
4. **Attach to camera.** `Layer.attached` (`skip_serializing_if = false`): through the camera,
   the layer's view is the identity, so it stays where it was drawn on screen; depth plays no
   part in placing it (it still sorts by it). While editing (stage space) it is drawn where its
   coordinates say, which is where it will appear with the camera at rest: the artist designs
   a HUD in the frame. The camera's colour effect still colours it (the whole picture is seen
   through the camera).
5. **Camera colour effects.** `Camera.color` is a `ColorEffect` (ADR 0056) instead of a tint:
   None, Brightness, Tint or Advanced. The camera has no opacity (`camera_effect`: Alpha reads as
   None, Advanced keeps alpha untouched), so the movie never fades to transparent through its
   camera. Classic and motion tweens of the camera element already interpolate colour effects;
   every renderer already composes the camera's transform around everything.
6. **Editing while previewing.** The model allows it for moves: `Session::camera_view(layer)`
   is each layer's stage → picture transform, and `Session::hit_test_camera` picks in the
   camera's display list. With View › Preview Through Camera, the Selection tool's click runs
   `element.pick {view: camera}`, a drag maps each pointer step back through the inverse of the
   active layer's view and runs the ordinary `element.translate`, and selection boxes are drawn
   through the view. The other tools still pause there: they draw or reshape in stage space and
   would each need their geometry mapped per layer (proposed below).
7. **Commands.** `layer.set_depth {depth, layer?, key?, frame?}` (with `key`, a key at the frame;
   the first key away from frame 0 also keys frame 0 with the depth it had, so the past doesn't
   change; `key: false` makes the depth one value again), `layer.remove_depth_key`,
   `layer.set_attached {attached?}` (toggles), `layer.depths {frame?}` (nearest first, ties in
   timeline order), `element.pick {view: stage|camera}`, and `camera.set` takes
   `element.set_color_effect`'s colour params except alpha. The parsing is shared
   (`EffectEdit`). Depth commands are refused on camera, folder and guide layers and while a
   symbol is open. All live in existing namespaces (`layer`, `camera`, `element`), which plug-ins
   may call. Every change is routine and undoable (ADR 0004 tier 3).
8. **The shell.** **Window › Layer Depth** (docked beside the Library): the layers nearest
   first, each with its colour, name, value, a key diamond, Attach, and a full-width track laid
   out by parallax (`FOCAL / (FOCAL + depth)`), so the busy range near the stage gets the room;
   a drag is one undo step. Properties › Frame's Depth row gains the key diamond and Attach.
   The timeline shows a camera toggle on each layer row once the scene has a camera.

## XFL

Animate's public documentation describes the camera and the Layer Depth panel as features but
not how XFL stores them (the camera layer, layer depth, attachment). Without a public format
description we don't import them; documents keep importing as before. Reported, not guessed.

## Consequences

* Every export draws the evaluated display list (PNG, sequences, GIF, video, SVG, Lottie) or
  bakes each layer's view per frame (HTML canvas, now through `Camera::layer_view`), so all of
  them match the stage; parity is covered by the existing export tests through the shared paths.
* Files without depth keys or attached layers are written exactly as before.
* Measured: no per-item work is added; a scene's draw order is computed once per evaluation, and
  only when some layer has a depth.

## Not yet

* Tools other than Selection while previewing (drawing and reshaping through the camera need
  per-layer pointer mapping in every tool).
* Depth inside symbols; depth keys moving with inserted and removed frames; easing on depth keys.
* XFL import of cameras, layer depth and attachment (no public format description).
