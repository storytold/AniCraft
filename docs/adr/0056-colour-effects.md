# ADR 0056: Colour effects: one colour transform for every style

* Status: accepted (2026-11-11)
* Relates to: ADR 0016 (motion tweens), ADR 0019 and 0032 (GPU stage), ADR 0027 (camera tint),
  ADR 0035 (XFL corpus), ADR 0053 (mixed Properties rows), roadmap M2

## Context

An instance's colour effect was `{alpha, tint, tint_amount}`. Brightness came in as a tint towards
white or black (ADR 0035), Animate's Advanced style kept only its alpha (reported), there was no
command or Properties control for anything but alpha, nested tints composed by keeping the stronger
one, and the motion tween slice (ADR 0016) had alpha and tint amount tracks only, the tint colour
left out.

Animate's Properties offers five styles: None, Brightness, Tint, Advanced (a multiplier in percent
and an offset in −255..255 for each of red, green, blue and alpha) and Alpha. Its public
documentation (ActionScript 3 `ColorTransform`, the XFL `Color` element's attributes) describes all
of them as one operation on each straight colour channel: `c ↦ c·multiplier + offset`, clamped.

## Decision

1. **Two types, authored and drawn** (`crates/doc/src/color_effect.rs`).
   * `ColorEffect` is what an element stores: an enum of the five styles with that style's values
     (`Brightness(b)`, `Tint { color, amount }`, `Advanced(ColorTransform)`, `Alpha(a)`, `None`).
     Properties shows the style the artist chose, and a tint keeps its colour at 0 %.
   * `ColorTransform { mul: [f32; 4], add: [f32; 4] }` is what everything draws with. Each style is
     a transform: brightness `b` is a tint towards white (`b > 0`) or black by `|b|`; a tint of `t`
     by `k` multiplies colour channels by `1 − k` and adds `k·t`; alpha multiplies alpha.
   * `DisplayItem::color` is a `ColorTransform`. Nested instances, the camera's tint and onion skin
     compose by `concat`, the exact composition of the two maps (no clamp between). The old "the
     stronger tint wins" rule is gone: a brightness around a tint now shows both, as in Animate.

2. **One arithmetic in every renderer.** The CPU oracle applies `clamp(c·mul + add)` to straight
   colour before premultiplying (solids, gradient ramps and each bitmap sample). The GPU stage
   carries a bitmap's transform as two entries of its ramp table (no new buffer, the record layout
   unchanged) and applies the same expression in `fine.wgsl`; solids and gradients arrive already
   transformed. Parity on the bitmap colour cases, Advanced with a negative multiplier and an
   alpha offset included, is exact (0 of 3072 pixels differ). NaN multipliers or offsets draw a
   channel of 0, as a NaN colour always did.

3. **Groups.** An instance whose transform only multiplies alpha still fades as one group at that
   alpha. An alpha *offset* changes each item's alpha on its own, so it
   stays in the items' transform and the group (for a blend or filters) is opaque.

4. **Files.** `ColorEffect` writes the fields older files have (`alpha`, `tint`, `tint_amount`,
   the look as near as they can say it), so None, Alpha and Tint are written byte for byte as
   before, and older builds open Brightness as the tint it looks like and Advanced as its alpha.
   `style` (with `brightness`, or `multiply` and `offset` as fractions, offset 1 = 255) is added
   only when those fields alone would read back as something else. A file without `style` reads as
   it always looked: a tint, an alpha, both together (Advanced: alpha after tint), or none. Values
   are clamped into Animate's ranges on read.

5. **Commands.** `element.set_color_effect {style?, alpha?, brightness?, tint?, tint_amount?,
   red_multiplier? … alpha_offset?}` on every selected element (per layer, one undo step). The
   style defaults to the one the given values belong to, else each element's own, so a change on
   a mixed selection sets only that value (ADR 0053). A new style starts from the look it can keep:
   Advanced is exactly the effect before, Alpha keeps the opacity, Brightness and Tint start at
   0 % (switching never jumps, except to None). `element.set_alpha` (plug-ins, ActionScript's
   `alpha`, tier 2 unchanged) sets the alpha multiplier and keeps the rest: None becomes Alpha;
   Brightness and Tint become Advanced below 1. Element listings report `color_effect`.

6. **Classic tweens.** Within one style its values travel (a tint's colour and amount, the
   brightness itself: −100 % to +100 % is 0 halfway); None meets any style as that style's rest
   value; any other change of style travels in the Advanced space (multipliers and offsets
   interpolated), as Animate does.

7. **Motion tweens.** New properties: `brightness`, `tint_red`/`tint_green`/`tint_blue` (0–255,
   beside the existing `tint` amount), and the eight Advanced ones (`red_multiplier` …
   `alpha_offset`). As in Animate a motion tween has one colour effect: the style its colour
   properties are keyed in replaces the target's own (which shows while none are keyed). Keys in
   several styles (older files keyed alpha and tint together, and the Motion Editor allows it)
   compose into Advanced, alpha outermost, then brightness, tint, Advanced. A style's properties
   start from the target's own values (a tinted target's tint keys start from its colour). The
   Motion Editor lists the styles in use (Alpha when none) and adds others with **Add Colour
   Effect**.

8. **Exports and import.**
   * SVG: colours and stops carry the effect (as before, now every style); a bitmap fill with more
     than alpha draws its image through an `feColorMatrix` (`sRGB`, the same matrix).
   * HTML canvas: the exported effect is the 8-number transform; the player composes it exactly
     (`player.js` mirrors `concat`) and a bitmap with more than alpha is drawn from a recoloured
     copy of its pixels (cached per bitmap and effect), so the old "tints on bitmaps" report entry
     is gone. JS parity with `evaluate_player` is tested under boa, Brightness, Tint and Advanced
     with an alpha offset included.
   * Lottie has no colour transform: every style is baked into the colours (tweens become colour
     keyframes), which maps fully; bitmap fills stay unexported as before.
   * XFL: a `Color` with `tintMultiplier`/`tintColor` is Tint, with `brightness` Brightness, with
     any other multiplier or an offset (−255..255) Advanced, with `alphaMultiplier` alone Alpha. The
     "advanced colour effects (only alpha kept)" report entry is gone. A new `colour` corpus package
     (every style, a brightness around a tint, a tint-to-Advanced and a brightness tween) has goldens
     and hand-worked probes; the `symbols` and `tweens` goldens were regenerated (the Advanced
     instance now draws its channels; None-to-Tint now keeps the tint's colour all the way).

9. **Properties.** A **Colour effect** row with the Style menu (None, Brightness, Tint, Advanced,
   Alpha), then the style's controls: an Alpha slider; a Brightness slider (−100..100 %); Tint's
   colour swatch and amount; Advanced's four channel rows, multiplier (%) and signed offset. Every
   row is mixed-aware (each value compared as the first element's style sees them). The camera
   keeps its own tint row.

## Consequences

* One representation reaches every output; adding a sixth style (none planned) is a new
  `ColorEffect` arm and a transform.
* Composition of nested effects changes for nested *tints*: older documents with a tint inside a
  tint now show both, exactly, instead of the stronger one. This is Animate's behaviour.
* An older motion tween with both alpha and tint tracks looks as it did; a tint track on an
  element whose tint colour was stored at 0 % (possible only through XFL import) now starts from
  white.
* `Props` holds 20 values; `Property::ALL` grew from 8 to 20.

## Not in this slice

* Colour effects on the camera beyond its tint (ADR 0027).
* A per-channel colour picker for the Motion Editor's tint colour (three 0–255 tracks for now).
* Lottie bitmaps (not exported at all yet).
