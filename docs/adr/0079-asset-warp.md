# ADR 0079: Asset Warp: warp meshes, pins and the Asset Warp tool

* Status: accepted (2026-11-12)
* Relates to: roadmap M7, issue #213 (mesh and warp deformers), ADR 0043 (bone poses, which
  pin poses mirror), ADR 0071 (shape armatures), ADR 0032 (bitmaps on the GPU), ADR 0004
  (destructive actions), ADR 0003 (file format)

## Context

Animate's Asset Warp tool puts pins on a shape or a bitmap; the artwork is covered by a
deformation mesh, and dragging pins bends it. Pins are posed per keyframe and tween. Behaviour
was learned from Adobe's public help pages and from watching the program; no code was copied.
We need: where the mesh and pins live, which deformation, how a bent bitmap is drawn by the CPU
oracle and the GPU stage, how pins tween, and what exports do, while `evaluate` stays pure and
everything is bounded.

## Decision

1. **The warp is on the shape.** A bitmap on the stage is already a shape filled with its image,
   so `Shape.warp: Option<Arc<Warp>>` (`#[serde(default, skip_serializing_if)]`: files without
   it read and write as before) covers both. `Warp { rest, resolution, pins, shown }`: the
   outline as drawn, cells along the mesh's longer side (2–48, default 12), pins
   `{id, rest, at}` (the point of the artwork as drawn and where it is now, in the shape's
   space; ids from the document's counter, counted on load) and a fingerprint of the outline it
   gave the shape. The shape's `path` is the bent outline, so the file holds what shows.
2. **The mesh is a grid, not stored.** A grid over the rest outline's control-point bounds,
   square-ish cells, each split into two triangles with the diagonals alternating like a
   chequerboard (no preferred direction). It is a pure function of `rest` and `resolution`, so
   it is never out of date and costs nothing in the file. At most 49 × 49 vertices.
3. **Deformation: rigid moving least squares** (S. Schaefer, T. McPhail and J. Warren, *Image
   deformation using moving least squares*, SIGGRAPH 2006, §2.3), evaluated at the mesh's
   vertices, piecewise affine between them. Each vertex moves by the rotation and translation
   that best carry the pins (weighted `1 / d²`) where they are. We chose it over the two-step
   as-rigid-as-possible method of Igarashi, Moscovich and Hughes (SIGGRAPH 2005) because it is a
   closed form per vertex: no sparse factorisation to write and refactor when pins change, no
   iterations to cap or converge, exactly deterministic, and its cost is bounded by
   vertices × pins (at most 2,401 × 128). It interpolates the pins, moves rigidly with one or
   two pins, and is near-rigid elsewhere (tested: a turn has distortion < 10⁻³; a 30° bend
   < 0.25). What ARAP on a mesh that follows the outline would add is topology awareness (two
   legs that touch don't drag each other); that is the place to go next (not yet).
4. **Outlines map through the mesh.** Straight edges are split where they cross the mesh's
   edges, so a line maps exactly to the mesh's image of it (a bitmap's outline is exactly the
   mesh's border). Curves are split into pieces no longer than half a cell; each end maps
   through its triangle, and a cubic's handles through the triangle of the end they belong to,
   so smooth points stay smooth. **At rest** (every pin where it was placed) the outline is the
   drawing's very `Arc`, so the rest pose reproduces the artwork bit for bit (tested on the
   path and on rendered pixels).
5. **Stale warps.** Other tools may change a warped shape's path. The warp then no longer shows
   (its fingerprint differs): `evaluate` draws the path as it is and ignores the warp, and the
   next warp command starts again from the shape as it now is, its pins kept where they show
   (`Warp::rebased`), so nothing jumps. A fingerprint is used rather than recomputing and
   comparing the outline, so floating-point differences between machines can never drop a warp.
6. **Pins per keyframe, tweened.** Adding a warp puts it on every keyframe of the layer holding
   the same artwork; adding or removing a pin does so on every keyframe bending the same mesh
   (an added pin sits where that point shows there). Moving a pin is the pose of the keyframe at
   the frame (keyed inside a classic or shape tween, as editing tweened objects is; motion tweens
   are refused). A classic tween between two keyframes on the same mesh blends the pins with the
   span's easing and bends the rest outline (`evaluate`'s `interpolate`), so a tween never
   blends two bent outlines and never drifts. A tweened element carries its blended warp, so a
   frame keyed mid-tween captures the pins.
7. **Bitmaps: textured triangles in the oracle, the same fill on the GPU.** `DisplayItem.warp`
   carries the bent mesh for a bent bitmap fill. The CPU oracle prepares the fill as an image of
   its own over the fill's pixel bounds: each triangle carries its pixel centres back to the
   image (its exact affine map) and samples it as an unwarped bitmap is sampled (bilinear, or the
   nearest pixel without Allow Smoothing); later triangles win where the mesh folds; edge pixels
   no triangle covers take a neighbour's colour so the outline's antialiasing has a colour. The
   fill is then a `Shader::Bitmap` of that image, one pixel per surface pixel, nearest, with the
   item's colour effect. `prepare_scene` hands the GPU stage that very fill, so the GPU needed no
   change and matches the oracle exactly (the parity test: 0 of 6,480 pixels differ). A GPU
   textured-triangle path, which would avoid preparing the image on the CPU each frame, is not
   yet.
8. **Exports through `evaluate`.** Raster exports (PNG, GIF, APNG, WebP, video) are the renderer.
   SVG and Lottie write warped shapes as their bent outlines; SVG bakes a bent bitmap into a
   PNG `<image>` at twice the stage's resolution where it shows (canvas and SVG have no mesh
   fills); Lottie already reports bitmap fills; the HTML export reports bent bitmaps (the image
   shows unbent in its bent outline).
9. **Commands** (`warp.*`, a namespace plug-ins may call): `warp.add {element?, resolution?}`
   (or the resolution of an existing warp; in Modify › Asset Warp), `warp.pin_add {element?, x,
   y}` (adds the mesh when there is none), `warp.pin_move {element?, pin, x?, y? | dx?, dy?,
   frame?}`, `warp.pin_remove`, `warp.remove` (the artwork as drawn again on every keyframe; in
   Modify › Asset Warp; undoable, so ADR 0004's routine tier: no confirmation) and `warp.info`.
   A warped shape becomes a drawing object, so merge drawing never cuts it.
10. **The shell.** The **Asset Warp** tool (Shift+Q, after Free Transform, as Animate groups
    it): a click on artwork pins it (the first pin makes the mesh) and picks the pin, a click on
    a pin picks it, a drag on a pin moves it live (many `warp.pin_move` in one gesture: one undo
    step), a drag on artwork adds a pin and drags it (one step), Delete removes the picked pin
    and Esc drops the pick. The mesh shows faintly in the layer's colour, pins as white dots
    (the picked one in the selection colour). Properties › Asset Warp: resolution, pin count,
    Remove Pin, Remove Warp.

## Bounds and numbers

Resolution 2–48; 128 pins; 20,000 anchors per warped outline (Optimize first beyond it); a bent
outline of at most 400,000 commands (past it nothing is split); coordinates within ±1,000,000;
a prepared bitmap image of at most 2²⁶ pixels. Measured (optimised dev profile, this machine):
bending a 1,200-anchor outline on a 48 × 48 mesh with 32 pins takes 0.36 ms per frame; drawing
a 960 × 480 px bent bitmap with a 48-cell mesh takes 7.9 ms against 3.9 ms flat.

## Consequences

* Files without warps are written exactly as before. An older AniCraft reading a newer file
  ignores `warp` and shows each keyframe's bent outline (a bitmap unbent inside it); pin poses
  hold instead of tweening.
* A warp with no extent or too many points is dropped on load (the shape keeps what it shows);
  non-finite and repeated pins are dropped.
* The `panic_hunt_every_command` fuzz runs a warped session (a rectangle and a bitmap, a pin
  posed inside a tween) with hostile warp params.

## Not yet

* As-rigid-as-possible solving on a mesh that follows the outline; meshes for symbols and text;
  warp pins bound to bones (Animate's bones on warped assets); a warp deformer over several
  layers (Live2D's, from #213).
* A GPU textured-triangle path; bent bitmaps in HTML canvas and Lottie.
* XFL import of warped assets.
