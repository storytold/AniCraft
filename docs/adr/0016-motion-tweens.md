# ADR 0016: Motion tweens and the Motion Editor

* Status: accepted (2026-11-09)
* Relates to: roadmap M2 (motion tween + Motion Editor curves), ADR 0001 (persistent document),
  ADR 0003 (file format), ADR 0004 (destructive actions), ADR 0009 (XFL import)

## Context

AniCraft had classic and shape tweens: a keyframe interpolates into the next keyframe with one
ease. Animate's *motion tween* works differently, and artists coming from Animate expect it: the
tween belongs to one keyframe span and one object in it, each property (position, scale,
rotation, skew, colour effect) has its own curve with *property keyframes* anywhere in the span,
moving the object at a later frame adds property keyframes instead of timeline keyframes, and
the Motion Editor shows each property's curve with Bézier easing. Behaviour was learned from
Adobe's public help pages and from observing the program; nothing was copied.

## Decision

1. **A motion tween is a field of a keyframe span**: `Keyframe.motion: Option<MotionTween>`
   (`crates/doc/src/motion.rs`). It names its `target` element and holds at most one `Track`
   per `Property` (`x`, `y`, `scale_x`, `scale_y`, `rotation`, `skew`, `alpha`, `tint`). A track
   is a sorted list of `PropertyKey { frame (offset into the span), value, ease }`. A span has
   either a classic/shape tween (`Keyframe.tween`) or a motion tween; commands keep it so, and
   loading drops a classic tween beside a motion tween.
2. **Values are stored in the units the editor shows**: pixels, percent and degrees (rotation
   is not wrapped, so 0 → 720 turns twice). The element on the keyframe is the *base*: how the
   object looks on the span's first frame. A property without a track keeps the base's value;
   when no transform property is keyed the base transform is used bit for bit, otherwise the
   transform is composed from the property values (`T · R · Skew · S`, the inverse of
   `Affine::decompose`). Before its first key a track holds the first value, after its last it
   holds the last.
3. **Per-segment ease is an `Easing`** on the key that starts the segment, so the existing
   presets work and a custom curve is `Easing::CubicBezier(x1, y1, x2, y2)` — CSS-style handles
   *relative to the segment's box* (time 0..1, value 0..1 with overshoot allowed). We chose
   relative handles over absolute (time, value) tangents because they survive value edits and
   retiming unchanged, keep time monotonic (x is clamped to 0..1), and reuse one easing type
   everywhere. The cost: a flat segment (equal values) shows no ease; the editor still shows its
   handles against a share of the view. `Easing::as_bezier` gives every preset its handles
   (exact except the two in-out polynomials).
4. **Evaluation stays pure**: `evaluate_guided` applies `MotionTween::apply(base, offset)` to
   the target; other elements on the key hold. Every export (PNG, GIF, SVG, Lottie, video,
   sprite sheet) and hit testing see motion tweens through `evaluate` with no extra code.
5. **Auto-key is done once, in the session, for every command.** When a command asks for the
   keyframe to edit (`Session::editable_keyframe`) inside a motion span, the target is swapped
   for how it looks at the playhead; after the command, what changed in its transform or colour
   effect becomes property keys at the playhead (rotation unwrapped against the curve, so a
   turn past 180° keeps turning the same way) and the base comes back, with any other change
   (fill, path, name) kept. On the span's first frame an unkeyed property changes the base
   itself. Moving, nudging, free transform, rotate/flip, align, the Properties fields and
   alpha all key properties without knowing about motion tweens. Insert Keyframe (F6) inside a
   span keys every property there instead of splitting the span.
6. **Timeline operations keep the motion.** Splitting a span (copy frames across it, moving
   keyframes onto it) gives the second half a re-based copy of the tween
   (`MotionTween::split_at`), pasted frames and duplicated scenes re-point the target at the
   copied object, and removing a motion tween (or replacing it by a classic one) leaves the
   object as it is on the first frame. When the target's id is gone but the key holds one
   element, that element is the target, so edits that replace the object (Convert to Symbol)
   keep its motion.
7. **Create Motion Tween** works on any span with artwork: several objects first become one
   graphic symbol (Animate asks; we do it and say so, and Ctrl+Z undoes it, per ADR 0004), and a
   one-frame span grows to one second, as in Animate.
8. **The file format stays version 3**: `motion` is an additive, `#[serde(default)]` field, as
   filters and blend modes were. Files without it open unchanged; a build older than this one
   would open a file with motion tweens and show the objects static (it ignores the field).
9. **The Motion Editor is a tab of the timeline dock** (Window › Motion Editor, or double-click
   a motion span), following the active layer and playhead: a property list (value at the
   playhead, editable; a key toggle) and the selected property's curve, with draggable keys and
   Bézier handles. It is view state; every change is a `motion.*` command, and each drag is one
   undo step (a session gesture).

Headless: `timeline.create_motion_tween`, `timeline.remove_tween`, `motion.set_key`,
`motion.remove_key`, `motion.set_ease`, `motion.info`.

## Consequences

* A motion tween animates one object. Artwork added to a motion span later holds still.
* Resizing a span does not rescale its property keys yet: keys past a shortened end are kept
  but not reached, and a lengthened span holds its last values.
* Motion paths drawn and edited on the stage, with orient to path, came in ADR 0030.
* Tint colour (red, green, blue), brightness and the eight Advanced properties came in ADR
  0056, with one colour effect style per tween as in Animate (keys in several styles compose).
* Not yet (later PRs): dragging keys in time in the Motion Editor, ease presets with
  strengths and saved custom eases, 3D rotation and position, Split Motion and joining tweens as commands, Lottie
  export of property curves as eased keyframes (it exports frame-accurate hold keys today),
  and importing motion tweens from XFL (`motionObjectXML`; still reported, kept as keyframes).
