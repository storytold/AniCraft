# ADR 0096: Raw brush

* Status: proposed
* Relates to: ADR 0033 (brush tool), ADR 0031 (merge drawing), ADR 0068 (brush library,
  pressure curves, Paint Brush), ADR 0093 (tilt, texture, scatter)

## Context

This is the brush a person who drew in Flash CS6 all day would recognize. The reasons are
theirs.

You drew with the Brush. A stroke was a filled shape in the fill colour. Where it touched the
same colour it became one shape, so the eraser and the selection tool could clean the drawing
the way you clean ink. Object Drawing was the switch for when a stroke had to stay its own
object. Leaving it on for every stroke turned a drawing into a pile of pieces you could not
edit. The editor already starts with it off (ADR 0031). This record does not change that. A
script with no window still makes one element per stroke, because there is no stage there to
merge onto by eye.

Smoothing all the way off was a look, not a broken setting. The wobble of the hand stayed in
the vectors. You could see where the pen hesitated. People who inked that way put the slider
at 0 and left it there. A fit of even a third of a screen point sands that off: the line comes
back smoother than the hand, and you cannot correct a drawing you cannot see. The ink on the
screen while the pen is down has to be the ink that is there after you lift.

A tablet's lightest touch was a hairline. A hard press was the size you had set. Ten percent
of a 20 px brush is still a 2 px sausage, which is not a light touch. The first sample was
often a fat dot, because the system also sent a mouse event at full pressure with the
pen-down. The workaround was to start the stroke already moving. That is not a pen. Speed can
stand in for a mouse. It must not replace a pen, and turning it off must not freeze every
stroke at full width: a light pen is still light.

The curve is how you mean the pressure. Linear is a medium touch at a medium width. Firm
stays near the hairline until you press. Marker and dotted were always one width. Pressure
was never their point.

Nothing here is taken from Flash, from Wick Editor, or from any other program. No code, no
presets, no brushes. The stroke is still the distance-field outline (ADR 0033). Tilt, paper
grain and spatter (ADR 0093) stay. This is the ink under them. Tilt is still not read from
the desktop. winit 0.30 does not deliver a macOS stylus pressure either, and a light line
cannot be drawn without that one number.

## Decision

1. **Smoothing 0 is the points you drew.** At 0 the centre line is the samples and the outline
   is the distance-field contour written as line segments. There is no cubic. Just above 0 the
   old fit returns: the centre line within a fraction of a screen point of the hand, and at
   100 it may wander several screen points. A straight run whose pressure changes is sampled
   along its length, so a thin point is stamped instead of being lost between the two ends.
   The plain Brush tool opens at 0. A command that omits `smoothing` stays at 50, so a script
   that never asked for raw ink does not suddenly get it. Built-in presets keep the smoothing
   they were drawn with.

2. **You see the ink you draw.** Every pointer position in the frame is kept. One position
   per frame cuts the corners out of a fast stroke, and the line comes back smoother than the
   hand. While the pen is down the preview is one filled mesh of those samples. Separate
   shapes flashed where they met. The mesh stays up until the stage picture shows that
   stroke, or the document has moved on, so the ink does not vanish and come back as a
   different line. At smoothing 0 the stored point is the latest sample. Higher smoothing
   still steadies the recent hand, newest heaviest.

3. **Pressure runs from Thinnest to Max.** Thinnest (`min_size`, default 0.01 stage pixels)
   is the width at no pressure. Max (`size`) is the width at full pressure. Both are in
   Properties and on `draw.brush`, and Thinnest never goes above Max. The factor never hits
   0, so the tip does not drop out of the sweep. A shaping that sets no Thinnest keeps the
   old floor, a tenth of the size (ADR 0068), so geometry that was drawn that way stays
   valid. Built-in presets that use pressure pin their lightest width at a tenth of their
   size for the same reason. The plain Brush is the one that opens at 0.01. A taper still
   comes down to 5 % unless a Thinnest is set, and then it stops at the width of no pressure
   so a point does not open a hole. Marker and Dotted stay one width (`pressure_size: false`).
   The Curve is unchanged.

4. **A real pen is the pen. Speed is only the mouse.** On macOS the pressure is the tablet
   point AppKit already delivers. winit drops it, so `pen-macos` listens for it. The shell
   does not call AppKit and has no `unsafe`. The crate has two `unsafe` sites, each with a
   safety comment: the monitor block borrows the event AppKit is dispatching, and it
   registers that monitor. Reading pressure or the button mask on the wrong event kind aborts
   the process, so those reads run only on mouse and tablet events, inside
   `objc2::exception::catch`. The workspace lint forbids `unsafe` and cannot be inherited and
   then overridden, so this crate sets its own lint (`unsafe_code = allow`) and does not take
   the workspace lints. That is the whole exception. It reads pressure only. Tilt stays where
   ADR 0093 left it.
   A tip-down sample in the last 30 ms wins over the mouse event sent with the pen, once, so
   the start can be thin. After that the pen's own pressure is kept for the stroke. Turning
   **Speed as pressure** off leaves the pen alone. With it off, a mouse paints full width.
   With it on (the plain Brush's default), the opening dash's own speed is the width of that
   start, down to Thinnest, not a half-width dot. At smoothing 0 a later sample is that
   sample's own speed. Higher smoothing still eases it, so one jerk does not show.

5. **The pen can be smaller than a pixel, and you can zoom in far enough to use it.** Brush
   and eraser size, and Thinnest, go down to 0.01 stage pixels. Widths are stage pixels, the
   same at any zoom. The view runs from 5 % to 25600 %. The zoom menu lists those stops; the
   wheel and the zoom keys already followed the clamp. Ruler steps of 0.25 and 0.5 stage
   pixels keep the ruler readable that far in. It stays finite so stage coordinates remain
   exact.

6. **Strokes still join unless you ask them not to.** Unchanged from ADR 0031. Object
   Drawing (J) keeps a new stroke as its own object. A headless session still starts with
   Object Drawing on.

## Consequences

* The live mesh is aliased. The picture that replaces it is anti-aliased, so the edge can
  soften by a fraction of a cell when the tile arrives. At smoothing 0 the vectors are the
  samples. At a higher smoothing the mesh can still differ from the fitted outline, which is
  the steadying you asked for.
* A long hairline can coarsen. The distance-field grid will not grow past its cell budget, and
  a sample thinner than a cell is stamped at least that wide so it is not skipped. Zoom in to
  place it; the stored width is still the stage width.
* A driver that reports a hover as a tiny pressure with the tip up is treated as a touch.
  Hover with the tip up and pressure 0 is ignored.
* A saved brush that omitted `min_size` reads as 0.01 after it is checked. One that stored a
  smoothing value is unchanged. Built-in presets are unchanged in look.
* Desktop tilt is still unread (ADR 0093). Opacity along a stroke is still one average
  (ADR 0068).
