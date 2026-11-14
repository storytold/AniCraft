# ADR 0074: Gradient editor, the Gradient Transform tool and Lock Fill

* Status: accepted (2026-11-12)
* Relates to: ADR 0002 (render pipeline), ADR 0019 / 0029 (GPU stage), ADR 0034 (Eyedropper),
  ADR 0040 (interface languages), ADR 0064 (accessibility), user issues #208 and #203, roadmap M1

## Context

The renderer, the file format, the importers and exporters and the Eyedropper already handled
linear and radial gradients and bitmap fills, but an artist couldn't make or change one: there
was no gradient editor and no way to move, turn or stretch where a fill lies. Animate's
everyday workflow is the Color panel's fill type and colour stops, the Gradient Transform tool
(**F**) and Lock Fill on the Paint Bucket and Brush. Animate's gradients also carry what our
model lacked: overflow (Extend, Reflect, Repeat), Linear RGB, a focal point and elliptical,
turned radial gradients. XFL import reported all four as approximations.

## Decision

### Model (`crates/doc`)

1. **Options on the existing variants, not a new paint type.** `Paint::Linear` gains `spread`
   and `linear_rgb`; `Paint::Radial` gains those plus `focal` (Animate's focal point ratio, along
   the gradient's own x axis, held to ±0.98 of the radius) and `matrix` (gradient space → shape
   space, so any ellipse at any angle). Each is `#[serde(default)]` and skipped when unset, so
   older files open unchanged and plain gradients are written byte-for-byte as before.
2. **A radial gradient is stored as a circle when it is one**, else as a circle with a matrix
   about its centre (`Paint::with_frame` canonicalises). `Paint::transformed` composes the
   matrix, so a radial fill is now exact under any transform (it used the mean scale before).
   A linear gradient keeps `start`/`end`: its handles only move, turn and stretch it along its
   axis, as Animate's do.
3. **Fill frames.** `Paint::frame` maps the box −1..1 onto a fill (a linear ramp's span, a radial
   ring, one copy of a bitmap; Animate's gradient square), and `with_frame` sets it. The tool,
   the command and fitting all work on frames, one representation for three kinds of fill.
4. **`Spread::apply`** is the one definition of pad, reflect and repeat; the GPU shader, its
   emulator and the HTML exporter follow it.

### Rendering and interchange

5. **CPU oracle**: spread on the ramp position; the focal point by solving
   `(1 − f²)t² − 2(f·d)t − d·d = 0` in the ring's unit space (the cone of SVG's and canvas's
   two-circle gradient with the inner circle at the focal point, radius 0); Linear RGB by mixing
   the stops in linear light when the 256-entry table is built (so the GPU gets it for free).
   Analytic tests cover each.
6. **GPU stage**: a gradient's paint record had a free vec4 (its colour slot); it now holds the
   spread code and the focal ratio, and `fine.wgsl` evaluates both. The radial `inv` already
   included the matrix. Parity scene: reflect + linear RGB, repeat, a repeating focal gradient
   on a turned ellipse.
7. **SVG export** writes `spreadMethod`, `color-interpolation="linearRGB"`, `fx`/`fy` and the
   matrix in `gradientTransform`: exact. **HTML canvas** has only padded gradients, so reflect
   and repeat are drawn as the ramp laid end to end (bounded copies), Linear RGB as sampled
   stops, the focal point with the two-circle gradient; an ellipse uses its mean radius.
   **Lottie** reports spread, Linear RGB and focal points (it has none of them).
8. **Import**: XFL's `spreadMethod`, `interpolationMethod`, `focalPointRatio` and elliptical
   matrices and SVG's `spreadMethod`, `color-interpolation`, `fx`/`fy` and skewed radial
   transforms come in exactly. The XFL corpus's gradients golden was regenerated: it now shows
   what the file asks for.

### Commands (`crates/engine/src/cmds_paint.rs`)

9. **`paint.set_fill`** sets the fill swatch and, by default, the selected shapes' fills in one
   step: `type`, `color`, `stops` (2 to 15, `#rrggbbaa` for opacity), `spread`, `linear_rgb`,
   `focal`, `bitmap`. A shape keeping its kind of fill keeps where it lies (editing stops never
   moves a gradient); a shape changing kind gets the new fill fitted to its bounds. A new
   gradient keeps the colours of the gradient it replaces, else starts white to black
   (Animate's default).
10. **`paint.transform_fill`** sets a fill's frame (absolute, `frame`) or changes it (scale, skew,
    rotate about its centre, move, centre, focal), in the space being edited. The tool sends
    absolute frames worked out from the frame at the press, so a drag inside one gesture is one
    undo step and never drifts. Flattening a fill to nothing is refused.
11. **Lock Fill** is tool state (`ToolStyle::lock_fill`, `paint.lock_fill`). Off (the default, as
    in Animate): the Paint Bucket, the Brush and the shape tools fit a gradient or bitmap to
    each shape (`Paint::fitted`: linear across the width, radial centred touching the longer
    side, bitmap tiled from the top-left). On: the swatch's stage-space fill is carried into each
    shape, so one ramp runs across many. **The Eyedropper turns Lock Fill on** when it picks a
    gradient or bitmap, which keeps ADR 0034's behaviour (a picked gradient stays where it was)
    and is what Animate does.
12. The new commands are in the `paint` namespace, which plug-ins may already call; the
    `panic_hunt_every_command` fuzz covers them.

### Shell

13. **One colour picker, as Animate has one Color panel.** The colour picker (`src/colour.rs`:
    web-safe swatches, the mixer, hex, alpha) is where colours are chosen everywhere. A *fill's*
    picker (`src/colour_panel.rs`), opened from the Tools panel's Fill square or any Fill swatch
    in Properties (the selection's, or a tool's swatch), adds the fill's type on top, as
    Animate's Color Type menu: **None, Solid, Linear, Radial, Bitmap**. Solid shows the colour
    picker as it is; a gradient shows its stop bar, overflow, Linear RGB and focal point above
    the same colour picker, which then edits the picked stop's colour and opacity; a bitmap
    shows its Library image. The fill's No Colour is its type None; the Stroke square's picker
    keeps No Colour (strokes have no type). A gradient's swatch in Properties shows its ramp.
    The popup reports what was asked (`FillEdit`) and its caller applies it: solid colours and
    None exactly as each place did before (the Tools square recolours the selection with
    `element.set_fill`, a tool's swatch sets the tool style), everything else as
    `paint.set_fill` (from a tool's options with `selection: false`). An earlier draft of this
    PR had a separate Colour section in Properties; two places to choose a fill colour is what
    this decision avoids.
14. **The stop bar** is keyboard-operable (ADR 0064): the bar is a button that adds a stop in the
    widest gap; each stop is a focusable, named control; arrow keys move it (Shift: 10 %),
    Delete removes it; dragging a stop off the bar removes it, as in Animate.
15. **The Gradient Transform tool (F)** draws Animate's handles on the fill's frame: linear (edges,
    centre, width square, rotate knob), radial (ring, centre, focal triangle, width square, size
    circle, rotate knob) and bitmap (tile box, centre, width and height squares, skew diamonds,
    size circle, rotate knob). The plain selection box is hidden while it shows. In Animate it
    shares a flyout with Free Transform; AniCraft has no flyouts yet, so it has its own button.

## Consequences

* Gradients round-trip exactly through `.anicraft`, SVG and XFL; files without the new options
  are unchanged. Radial fills on skewed shapes are now drawn exactly.
* Scripts can build and place any gradient headless (`paint.set_fill`, `paint.transform_fill`).
* The colour picker popups are named dialogs for screen readers, and the swatch grid is a named
  control (the accessibility audit now covers the solid and gradient pickers).

## Not in this slice

* Gradient and bitmap **strokes** (strokes have one colour).
* Saved gradient swatches; the Brush's Lock Fill only for its fill colour mode.
* Overflow on HTML canvas beyond the bounded copies; elliptical radial gradients in Lottie and
  canvas (mean radius).
* Skewing a linear gradient's bands (its frame keeps only its axis).
