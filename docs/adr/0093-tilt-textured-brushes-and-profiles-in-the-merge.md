# ADR 0093: Tilt, textured brushes, erasing inside symbols, and profiles in the merge

* Status: accepted (2026-11-13)
* Relates to: ADR 0033 (Brush tool), ADR 0034 (Eraser), ADR 0068 (brush library), ADR 0031 and
  0042 (merge drawing), ADR 0018 (variable-width strokes), ADR 0045 and 0080 (editing in place)

## Context

The first brush and eraser slices left four gaps that Animate users notice:

* **Tilt.** In Animate a leaning pen can turn and widen the brush.
* **Textured brushes.** Dry media (charcoal, chalk on rough paper) and spatter.
* **Erasing inside symbols.** The Eraser kept symbols whole with no hint of what to do instead.
* **Gradients and width profiles in the merge.** A variable-width line was always kept whole,
  as an object, so the Eraser couldn't touch it and a pressured Paint Brush line was never raw.
  Gradients already merged (ADR 0042), but nothing tested that a cut keeps them in place.

We learned the behaviour from Adobe's public help pages and by watching the program. Nothing
was copied: no code, presets, textures or brushes.

## Decision

1. **Tilt is per sample and shapes the tip analytically.**
   * A sample may carry the pen's tilt as the W3C Pointer Events spec gives it: `altitude` (90°
     upright to 0° flat) and `azimuth` (the way the pen leans on the stage, clockwise from +x).
     Over the CLI and MCP a point is `[x, y, pressure, altitude, azimuth]`. Out-of-range
     altitudes are clamped (it's a measurement), and azimuths wrap.
   * `lean = 1 − altitude / 90°`. The lean is carried along the centre line as a vector toward
     the azimuth, so it blends smoothly between samples and through upright.
   * **Angle** (`tilt_angle`): the tip's angle is the brush's angle plus the azimuth. Below a lean
     of 0.02 the azimuth is noise, so the brush's own angle holds.
   * **Width** (`tilt_width`): the size factor is multiplied by `1 + lean` (up to 2× lying flat).
     For the Paint Brush this goes into its width profile.
   * A mouse sends no tilt, so it is never affected. Strokes without tilt give exactly the
     outline they gave before: the same code path, and turning tilt on without tilt in the points changes nothing (tested).

   *The sweep with a turning tip.* ADR 0033's distance field lives in one tip space, which no
   longer exists once the tip turns. When any point's angle differs from the brush's, the field
   is sampled in stage space (in tip radii), and the stroke is cut into pieces that turn at most
   3° each. Each piece measures its distance in its own tip space; that distance times the
   roundness is a lower bound on the stage distance with the same zero level, which is all the
   tracing needs. Cells are scaled by the roundness, so a thin nib is still resolved.

   *The platform gap.* winit 0.30 reports tilt only on iOS (`Force::Calibrated`'s
   `altitude_angle`), and egui 0.36's touch events carry only `force`. No desktop system passes
   a pen's tilt to the shell yet. The model, the point format and the command params are ready,
   so scripts, the CLI and MCP can paint with tilt today.

   *Hidden until a pen reports it.* A control that does nothing confuses artists, so
   Properties shows no Tilt options until a pointer event has carried tilt this session
   (`App::pen_tilt_seen`, set from `brushtool::frame_tilt`, the one place tilt is read). Then
   they appear by themselves. Brushes keep their tilt settings either way (Charcoal follows
   tilt), so a brush set moves between computers unchanged. A UI test checks both states. We didn't add a platform-specific tablet API (Wintab, Windows Ink, NSEvent tilt,
   XInput2 valuators): that would mean `unsafe`, FFI and a code path per OS for one value. The
   right place for it is upstream in winit.

2. **Textured brushes stay vector, from fixed patterns.**
   * **Texture** (`texture`, 0–100 %) is a paper grain: two-octave value noise, fixed on the
     stage and measured in tip radii (so it scales with the brush). Where it falls below a
     threshold of `0.2 + 0.4 × texture`, the paint has a hole. The field takes
     `min(distance, (noise − threshold) × grain size)`, roughly a distance, so tracing gives
     clean hole outlines. Holes only ever cut into the paint: the outline never grows. Strokes
     laid over each other share the grain, as on real paper. Cells are finer (1/18 of a radius)
     and an outline keeps at most 20,000 loops.
   * **Scatter** (`scatter`, 0–200 % of the size) throws each dab sideways from the line and
     sizes it at 60–100 %, both from a fixed hash of the dab's number. A continuous brush with
     scatter stamps dabs a quarter size apart.
   * Both are deterministic, and both make ordinary filled shapes that merge, erase, tween and
     export everywhere.
   * New built-ins: **Charcoal** (an oval with grain, following tilt) and **Spatter**.

   *Rejected:* bitmap tips and textures. They would need new paint along a path in every
   renderer and export. Vector paint also renders crisply at any zoom and cost nothing new
   downstream.

3. **Width profiles go through the merge.**
   * Each segment in the planar merge now carries its place on its item's path as a *path
     position*: `k + t` is parameter `t` along the segment that command `k` draws.
   * Each stroke piece reports the stretches it runs along (`Piece::spans`, in order,
     backwards where it runs backwards). Straight-run joins and closed loops are handled.
   * `WidthProfile::along(path, spans)` (in the model) gives the piece its part of the profile:
     * every width point inside a stretch moves to the same place on the piece, by arc length
       (composite Gauss–Legendre on the source's cubics);
     * where the piece starts, ends or jumps mid-profile, a point holds the width it had;
     * running backwards swaps left and right;
     * a piece with one width gets no profile.
   * A profiled stroke is a stroke style of its own in the merge, so its pieces never chain into
     another stroke's.
   * An uncut stroke keeps its profile point for point (tested). Merging the pieces again keeps
     them (tested).
   * Lines are still cut by their centre line, as in ADR 0034.
   * As a result, a variable-width line drawn with Object Drawing off is raw, and so is a
     pressured Paint Brush line (ADR 0068 drew these as objects).

4. **Gradients keep their place when cut.** This works through ADR 0042's paint carried into
   keyframe space. It is now tested: after an erase, each piece's paint mapped to the stage
   equals the original within a thousandth of a unit, and the renderer shows both ends' colours.

5. **Erasing inside symbols uses editing in place.**
   * Inside an open symbol, group or drawing object (ADR 0045, 0073, 0080), `draw.erase` already
     works on that timeline in its own space, and every instance shows the change. That is now
     tested: one erase, one undo step.
   * On the stage, symbols and drawing objects stay whole, as in Animate. `draw.erase` now
     reports what it passed over and kept (`kept: {symbols, objects}`). When nothing was erased,
     its reason says to double-click one with the Selection tool. The shell's status bar says
     the same in a translatable sentence.

6. **Fix in the curve fitter.** Schneider's least-squares solve could put a handle a hundred
   times the stretch's length away on a tiny, nearly closed stretch (the grain's small holes
   found it). A handle longer than the points' own arc length now falls back, and the error
   check splits the stretch instead. Chalk's ragged outline went from 321 to 245 anchors, with
   its area within its tested bounds.

7. **Commands.**
   * `draw.brush` gains 5-number points, `texture`, `scatter`, `tilt_angle` and `tilt_width`.
     So does `brush.preset_save`, and brush presets keep them (`#[serde(default)]`: older brush
     sets and preferences load as before).
   * There is no new namespace, so the plug-in policy is unchanged (`draw.*` may be called).
   * Shell: Texture and Scatter for the Brush; Properties › Tilt (Angle for the Brush, Width for
     both brushes) once a pen has reported tilt.

## Consequences

* Measured in release, 32 threads, median of 15, 400-unit wave (`preset_timings`):

  | Stroke | Time | Anchors |
  | --- | --- | --- |
  | Charcoal (grain, oval) | 7.2 ms | 896 |
  | Spatter | 2.6 ms | 395 |
  | Flat 15 % nib turning 180° with tilt, 201 samples | 21.4 ms | 36 |
  | Round 10, widening with tilt | 1.5 ms | 16 |
  | Chalk, before and after the fitter fix | 3.0 → 3.1 ms | 321 → 245 |

* A textured stroke has many small loops. A very long one may pass the merge's 20,000-segment
  budget, and then it is added whole as an object (ADR 0031), with nothing lost.
* The live preview while dragging still shows the plain tip, without the grain, scatter or
  turning.
* The file format is unchanged. Older files open as they did, and older builds read raw
  profiled strokes as before (the `merge` flag is ignored there).

## Not yet

* Reading tilt from a pen on Windows, macOS and Linux (waits for winit), and so the Tilt
  options in Properties there.
* Bitmap, art and pattern brushes; custom tip shapes.
* Eraser pressure, tilt and presets.
* A live preview through texture, scatter and tilt.
* Erasing symbols on the stage without opening them (Animate doesn't either).
