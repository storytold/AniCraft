# ADR 0089: Lottie export with keyframes and eases

* Status: accepted (2026-11-13)
* Relates to: roadmap M4 (Lottie export: easing curves instead of per-frame keys), ADR 0016
  (motion tweens), ADR 0030 (motion paths), ADR 0056 (colour effects baked into Lottie
  colours), ADR 0070 (soft masks as track mattes), ADR 0081 (ease presets and custom eases)

## Context

Lottie export was frame-accurate: every frame was evaluated, artwork that persisted became one
shape layer with its outline baked into stage space, and whatever changed was written as a hold
key at every frame where it changed. Any player showed AniCraft's frames exactly, but a tween
became a full outline per frame (the demo was 53 kB for two seconds), and players that draw
between frames (60 Hz screens) showed the motion stepping at the movie's frame rate. Lottie has
its own keyframes, eased by one cubic Bézier per key pair (`o`/`i` handles, the public Lottie
format description), and AniCraft's eases (ADR 0081) are, for most presets, exactly such cubics
or a few of them.

## Decision

1. **Sample, then key.** The exporter still evaluates every frame through `evaluate` (so every
   feature, nested symbols, the camera, masks, armatures, reaches the file with no extra
   code), and then writes each property's per-frame values as the fewest Lottie keyframes that
   draw the same frames (`crates/render/src/lottie_keys.rs`). Keys are not derived from the
   document's tweens, which would mean re-implementing `evaluate` for Lottie; the document only
   supplies the candidate eases. Time stays a function: nothing about the document changes.
2. **Candidate eases are the document's own.** Linear, plus every ease its classic tweens,
   their property groups and its motion-tween keys use, each as its cubic pieces: Linear,
   Classic Ease, Bézier, Quad/Cubic/Back In and Out are one cubic; their In Outs, Bounce and
   custom curves are several (ADR 0081's exact custom forms). Quart, Quint, Sine, Circ and
   Elastic have no exact cubic form and are not candidates. At most 64 distinct eases.
3. **A greedy, verified fit.** From each frame, the longest span (up to 2400 frames) that
   some candidate draws is taken: a key per cubic piece, at the time the piece starts
   (fractional frames, to 1/10000), its handles normalised to the piece's box; a piece that
   bends without changing value is halved (de Casteljau) until it can be keyed. **Every frame
   of the span is checked**: the keys, rounded as written, are evaluated as a Lottie player
   evaluates them and must land within 0.002 px (positions, outlines, widths), 0.001°, 0.002 %
   scale, 0.0015 of a colour channel or 0.02 % opacity of the sampled value. Where values stay,
   one hold key; where nothing fits, a hold key at that frame, exactly as before. A work budget
   per property (2 million frame checks) bounds the worst case (a minute of Elastic: 0.2 s).
4. **Moving artwork keeps its own space.** When a piece's transform changes over its layer's
   life, the outline is written in the artwork's own space and the motion as the group
   transform (`tr`: position, rotation, skew, scale; Lottie's T · R · Skew · S with the
   anchor at the origin), decomposed per frame QR-style like `Affine::decompose`, the rotation
   unwrapped so a turn keeps turning. Classic tweens interpolate exactly these components
   (position, rotation, shear and scale lerped), so a tween is two keys per property, and a
   tweened symbol inside a static, uniformly scaled parent too. Position is split into x and y
   (`"s": true`) when that takes fewer keys (a motion tween keys them apart). Gradients ride
   the group exactly (radial gradients become true ellipses under a stretch). **Exceptions stay
   in stage space**, as before: artwork that never moves, hairlines (outline mode), and strokes
   under a stretch or skew, since Lottie strokes in the group's space (a stretched pen) while
   the renderer widens strokes by the mean scale. A stroke under an even scale or a turn rides
   the group (its width scales with it, as the renderer draws it).
5. **A track follows the object across keyframes.** Artwork continues its layer when the next
   frame shows the same element, or else artwork that looks the same (structure, fill and
   stroke kinds, masks, blend) on the same layer whose track that frame didn't continue: a
   classic tween's end keyframe holds its own copy of the object in XFL files, and without this
   the tween's last frame was a layer of its own and its ease couldn't be fitted.
6. **Outlines are written one way.** A cubic whose handles sit a third and two thirds along
   its chord is written as the straight line it draws. Shape tweens blend copies of their
   outlines whose lines are such cubics while the keyframes hold plain lines; with one form, a
   shape tween between outlines of the same structure is two shape keys.
7. **Reported once.** Artwork keyed frame by frame while it moves (two frames in a row) is
   counted under one report line, "motion keyed frame by frame (Elastic, Sine and similar
   eases, motion guides, tweens inside tweening symbols…)": it is exact at every frame and
   holds between frames, as all Lottie export did before.
8. **The old keying stays reachable** as `LottieKeys::Frames` (`to_lottie_keyed`), the
   baseline the parity test and the size measurements compare with. It is not a command
   option: nothing a user sees needs it.
9. **Parity is tested by playing the file.** `crates/engine/tests/lottie_parity.rs` plays the
   export with a Lottie player written for the test from the public format description
   (keyframe holds and Bézier eases solved on their own, group transforms, fills, gradients,
   strokes, masks, track mattes, split position), draws each frame with the CPU oracle and
   compares it with the document's own frame, for the sample scenes, a movie of every kind of
   tween and ease, and the XFL corpus. Where nothing is reported lost, at most one pixel in a
   thousand may differ by more than 3/255; where something is (group fades, stretched radial
   gradients…), the eased file must draw at least as close as the frame-by-frame one. Breaking
   the player's ease makes all three suites fail.

## Measurements

| Movie | Frames | Frame by frame | Eased | Smaller by |
|---|---:|---:|---:|---:|
| Sample demo | 48 | 46.7 kB | 10.4 kB | 78 % |
| Sample interactive | 36 | 11.6 kB | 5.0 kB | 57 % |
| Every tween and ease (test movie) | 48 | 101.4 kB | 29.5 kB | 71 % |
| XFL corpus, 18 packages | 1–24 | 134.7 kB | 113.5 kB | 16 % (tweens package 53 %) |
| `sample::stress(20, 10, 240)` | 240 | 14.5 MB | 1.45 MB | 90 % |

Export time (release, this machine): the stress movie 931 ms frame by frame, 360 ms eased (less
JSON to write); the tween movie 9 ms and 14 ms; a minute of Elastic 20 ms and 218 ms (the
budget). Earlier builds wrote 53.3 kB for the demo; the frame-by-frame column is today's
`LottieKeys::Frames`, which shares this change's number format (whole numbers as `6`, not
`6.0`) and track continuity.

## Consequences

* Exported tweens are a few keys and play smoothly at any display rate; files shrink by
  roughly half to nine tenths; every frame still equals `evaluate`.
* Lottie players must ease as the format describes (they do: the handles are the common
  CSS-style cubic). Fractional key times are used for multi-piece eases.
* Not yet (later PRs): **motion along curved guides** as Lottie spatial tangents (`to`/`ti`):
  a Lottie player times spatial keys by the arc length it measures, and players measure it
  differently (sampled), so the frames can't be guaranteed to our tolerance; straight guides
  and motion-tween paths that are lines fit as ordinary keys. Orient to path as Lottie's
  auto-orient; colours eased past their range (they clamp, so they stay frame by frame); skew
  tweens of classic tweens (they lerp the shear, not its angle); a tween inside a tweening
  symbol as nested group transforms; shape tweens between outlines of different structure
  (the morph's re-parametrised copies); eases fitted to Sine, Quart, Quint, Circ and Elastic
  within a tolerance; the layer transform (`ks`) and parenting.
