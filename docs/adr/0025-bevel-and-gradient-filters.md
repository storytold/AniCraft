# ADR 0025: Bevel, Gradient Glow and Gradient Bevel filters

* Status: accepted (2026-11-09)
* Relates to: roadmap M5 (filters on movie clips), ADR 0002 (render pipeline: the CPU
  renderer is the oracle), ADR 0009 (XFL import), ADR 0019 (GPU stage falls back to the CPU)

## Context

Movie clip and button instances already carry filters (`doc::Filter`: Drop Shadow, Blur,
Glow, Adjust Color). A filtered instance is drawn on its own into a buffer grown by the
filters' reach, filtered in order on premultiplied pixels (`render::filters`), and composited
back as one image. SVG export writes matching `<filter>`s where it can, Lottie reports
filters, and XFL import reported Animate's three remaining filters (Bevel, Gradient Glow,
Gradient Bevel) as skipped. Artists coming from Animate use bevels and gradient glows for
buttons, titles and badges, so they expect all seven in the Filters menu with the same
values: blur X/Y, strength, quality, angle, distance, knockout, a type (inner, outer, full),
shadow and highlight colours for Bevel, and a ramp of colour stops for the gradient filters.

How they look is learnt from public descriptions only: Animate's user guide and the public
ActionScript 3.0 reference describe the parameters (and that the gradient filters' ramp is
indexed by the effect's intensity), and the image-processing literature describes embossing
as the difference of an image offset along the light direction. No Adobe code, SDK or asset
was used.

## Decision

1. **One signed field for bevels.** The blurred opacity `B` (the same box-blur passes as the
   other filters) is sampled a distance towards the light and a distance away from it:
   `D(p) = B(p + o) − B(p − o)` with `o` the angle's offset (45° = shadow down-right, light
   from the top left). `D` is in −1..=1: positive on edges that face the light, negative on
   edges that face away, 0 on flat areas and far from the artwork. Bevel paints the highlight
   colour at `clamp(D·strength)` and the shadow colour at `clamp(−D·strength)`; Gradient Bevel
   looks the ramp up at `0.5 − D·strength / 2` (highlight end 0, flat 0.5, shadow end 1).
   The construction is point-symmetric by design (tested), and each side is `distance` wide.
2. **Gradient Glow** uses the glow's intensity (the offset, blurred opacity, or for inner the
   transparency, times strength) as the ramp position, 0 to 1.
3. **One compositing rule for inner, outer, full and knockout.** Given the effect `E`
   (premultiplied) and the artwork `S` with opacity `a`: inner puts `E` atop the artwork
   (`E·a + S·(1 − Eα)`), outer under it (`S + E·(1 − a)`), full both (which is `E` over `S`);
   knockout keeps only the effect cut to its side (`E·a`, `E·(1 − a)`, or `E`). Gradient Glow
   uses its inner and outer intensities for the two sides. Drop Shadow and Glow keep their own
   code; their behaviour is unchanged.
4. **The ramp's rest stop is pinned transparent**, as Animate's panel pins it: Gradient
   Glow's stop at 0, Gradient Bevel's at 0.5. `Filter::sanitized` enforces it (inserting the
   stop with the ramp's colour there if missing, within the 15-stop limit; of several stops
   that close, the nearest is pinned and the others step aside, so sanitizing is idempotent),
   so a gradient filter never floods its whole buffer, whatever a file or command sends. The
   UI shows the pinned stop as fixed (colour editable, position and removal disabled). Ramps
   are looked up through a 257-entry table, so 0, ½ and 1 are exact samples.
5. **Model and file.** `Filter::Bevel { …, shadow_color, highlight_color, side, knockout }`,
   `Filter::GradientGlow { …, stops, side, knockout }` and `Filter::GradientBevel { … }`, with
   `side: inner | outer | full` (`type` is taken by the variant tag) and `stops` as the
   gradient fills' `{offset, color}`. Every field has a serde default (Animate's: blur 5,
   strength 100%, quality Low, angle 45°, distance 5, Bevel inner, Gradient Glow outer,
   Gradient Bevel inner), so `{"type": "bevel"}` opens, and files without these filters are
   unchanged. Older builds can't read a file holding them (an unknown variant), as with any
   new kind.
6. **Reach and blur.** Bevels sample `distance` both ways, and inner effects blur the
   transparency around the artwork, so every side's margin is blur + distance (inner Glow
   and Drop Shadow included: before, their blur read the buffer's edge as solid artwork and
   came out weak near it). Even box widths are centred (one more pixel each side at half
   weight) instead of shifted half a pixel, which keeps mirrored edges mirrored for every
   blur. How far past the surface a filtered group draws is capped by area, so its buffer
   always fits the renderer's 4096² limit while keeping the full padding around the artwork
   (before, a far-reaching filter could fail the whole frame).
7. **SVG export** builds all three from standard primitives in the same steps: the input's
   alpha (`feColorMatrix`), `feGaussianBlur` of the same spread as the box passes, two
   `feOffset`s, their difference (`feComposite operator="arithmetic"`, whose result is
   clamped to 0..1 by the spec) for highlight and shadow masks flooded with their colours, or
   the ramp position; the ramp as `feComponentTransfer` tables (33 samples: opaque colours from
   the position copied into R, G, B, cut by the ramp's alpha table); then `atop`, `over`,
   `in`, `out` composites for the type and knockout. Tables have 257 samples, as the CPU's.
   Every filter's region is now the stage grown by the filters' reach in user space (it was
   −50%/200% of the object, which cut far shadows and bevels on small artwork). No new
   reporting is needed.
8. **XFL import** reads `BevelFilter`, `GradientGlowFilter` and `GradientBevelFilter` with
   the same attribute names as the other filters (`blurX`, `strength`, `angle`, `distance`,
   `knockout`, `quality`), `shadowColor`/`shadowAlpha`, `highlightColor`/`highlightAlpha`,
   `type="inner|outer|full"`, and `GradientEntry` children (ratios 0..1, or Flash's 0..255).
   Anything else in `<filters>` is reported as an unknown filter. Tested on self-authored
   snippets.
9. **Command and UI.** `element.set_filters` takes the new kinds, with `#rrggbb[aa]` for
   `shadow_color`, `highlight_color` and each stop's `color`, and at most 15 stops. In
   Properties › Filters each new filter has the Blur, Quality, Strength, Offset, Type and
   Knockout rows; Bevel has Shadow and Highlight swatches with opacity; the gradient filters
   start with a compact ramp editor (no gradient-stop widget existed): the ramp over a
   checkerboard, a marker per stop (click to pick, drag to move between its neighbours and
   clear of the pinned stop, so stops keep their order; click the strip, or press it from the
   keyboard to add a stop in the widest gap), and the picked stop's colour, position and
   Remove below. The panel edits the sanitized filters, so a file holding values a command
   would refuse stays editable. Every edit is live and one undo step
   per gesture, through the same command.
10. **GPU stage**: unchanged; filtered groups fall back to the CPU per tile (ADR 0019).

## Consequences

* All seven of Animate's filters exist, render identically on stage and in PNG, GIF, video
  and sprite-sheet exports, and come across from Animate documents.
* The CPU oracle has analytic tests: bevel symmetry, inner/outer/full, knockout, strength 0,
  ramp ends and middle; plus engine, round-trip, SVG, XFL and hostile-value tests.
* Measured per 400 × 300 buffer, blur 16, quality Medium (release): Glow 0.87 ms, Bevel
  1.33 ms, Gradient Glow 1.66 ms, Gradient Bevel 2.42 ms.
* Not yet: filter presets and copy/paste of filters, enabling/disabling a filter without
  removing it, reordering filters in the panel, tweening filter values (motion tweens hold
  them), filters on the GPU, filters in Lottie (reported), XFL export.
