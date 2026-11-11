# ADR 0049: Motion path anchors pinned to frames

* Status: accepted (2026-11-10)
* Relates to: ADR 0030 (motion paths), ADR 0038 (motion path points), ADR 0016 (motion
  tweens), ADR 0003 (file format), ADR 0004 (destructive actions), roadmap M2

## Context

ADR 0030 gave motion tweens a path that the object travels by arc length, timed by the X/Y
curves. Only the path's ends were tied to frames: the first and last position keys. A middle
anchor was reached whenever the timing got there, and moving the object at a middle frame bent
the path through that point without adding a key. ADR 0038 left anchors tied to frames out,
because a pinned anchor would compete with the X/Y curves for the timing.

In Animate, a motion tween's position keyframes *are* the path's anchors. Moving the object at
frame 12 adds an anchor the object passes on frame 12, and the eases set the speed between those
anchors. Artists coming from Animate expect to say "be here on this frame" and have it hold
while they keep editing the path. We learned this from Adobe's public help pages and from
watching the program. Nothing was copied.

## Decision

1. **A pin ties an anchor to a frame.** `MotionTween.pins: Vec<Pin { frame, anchor }>`
   (`crates/doc/src/motion.rs`) lists the anchors between the ends that are pinned, by frame.
   Frames and anchors both increase, so a later frame is always on a later anchor. The ends are
   always pinned, to the first and last position keys, and they are not stored. A pin's frame
   holds X and Y keys with the anchor's place, so the X/Y curves pass through every pinned
   anchor and stay a truthful picture of the motion. Anchors that aren't pinned still shape the
   path, as in ADR 0038. We chose this over the alternatives below:
   * Making every position key an anchor would break older files. Their middle position keys
     are timing keys under ADR 0030, and they would have to be turned into anchors, which moves
     objects.
   * Giving anchors a frame of their own, separate from the keys, would mean two sources of
     timing. That is the conflict ADR 0038 described.
2. **Timing goes leg by leg.** The pinned anchors, ends included, cut the span into legs. On each
   leg, ADR 0030's rule applies to the stretch of path between its two anchors:
   * Position keys inside the leg split it into pieces.
   * On each piece, how far along its chord the X/Y curves are (signed) is how far along the
     piece's share of the stretch the object is. The shares are the chords' lengths.
   * A leg whose chords add up to nothing (a loop) is crossed at an even pace.

   The results:
   * the object is **exactly on each pinned anchor on its frame**;
   * eases, holds and overshoot work between pins as they did between the ends;
   * a straight path still reproduces the X/Y curves exactly, pins or not;
   * **with no pins, it is ADR 0030's arithmetic, number for number.** A test compares
     evaluation bit for bit with a copy of the old code, over eased, overshooting and middle-key
     tweens and a loop.

   `CurveWalk::anchor_distance` gives each anchor's distance along the path. `evaluate` stays
   pure.
3. **Pinning and unpinning.**
   * `MotionTween::pin(anchor, frame)` writes the anchor's place as the X/Y keys on that frame.
     A pin already on that frame, or the same anchor's pin on another frame, gives way (that is
     how a pin is retimed). A pin that would break the order is refused, and the refusal names
     the pin in the way.
   * `pin_here(frame)` pins where the object is. It uses the anchor there, or splits the path at
     that point (the curve unchanged), so nothing moves on that frame.
   * `unpin(frame)` removes the pin **and its frame's X/Y keys**. The anchor stays and shapes the
     path only. Pinning a frame without keys and then unpinning it gives back the same tween,
     key for key. So unpinning brings back ADR 0038's behaviour instead of leaving timing keys
     where the pin was.
   * Removing the X or Y key of a pinned frame unpins it, taking both keys. A pin always has
     both.
4. **The path keeps its pins.**
   * `set_path` writes each pin's anchor place as its keys. Keys that already hold the place
     stay exactly as they are, so editing elsewhere never changes the timing.
   * Adding a point moves the later pins' indices up with their anchors.
   * Deleting a pinned point lets its pin go, with its keys. `motion.delete_points` reports how
     many it unpinned.
   * Moving the whole path, splitting a span (pins after the cut move to the second half,
     re-indexed onto the cut path) and Straighten keep the meaning. Straighten forgets the pins,
     but their keys stay as the straight path's corners.
   * A tween without a path of its own shows the straight path through its position keys, and
     those keys are pinned to its corners. The first edit that makes the path the tween's own
     keeps them pinned when the edited path has as many anchors. A path of another shape is a
     new path: only its ends are pinned.
5. **Auto-key pins** (replacing ADR 0030 §4 between the ends). Moving the object on a path at a
   frame:
   * at a pinned frame (an end or a pin), moves that anchor with its handles, and its keys
     follow;
   * before the first position key, moves the start (as before);
   * between the ends, adds an anchor where the object is, pins it to that frame and moves it:
     Animate's "move it on frame 12, it passes there on frame 12";
   * after the last position key, carries the path on in a straight stretch to where the object
     was put. The new end is reached on that frame, and the old end stays pinned to its frame.

   Only where the object is off the path (an ease overshooting an end) does the old least-squares
   bend remain. A stage drag is one gesture, so it makes one pin and one undo step. Insert
   Keyframe (F6) inside a path tween pins where the object is.
6. **The Motion Editor shows X/Y keys as anchor timing.** On X and Y, a pinned frame's key is
   drawn larger in the motion colour. A note explains the blue keys, and the hover says which
   point a key belongs to. Setting the X or Y key of a pinned frame (dragging it in the graph or
   typing the value) moves the anchor, and the stage follows. That holds for the ends too, which
   ADR 0030 left inconsistent. The key toggle on a pinned frame says it unpins.
7. **On the stage** (`src/motion_path.rs`):
   * Pinned anchors are diamonds, the keyframe mark, with their frame number on a tag beside
     them. The other anchors stay squares.
   * Selection takes the ends and the pinned anchors (the motion's keyframes). Subselection
     takes every anchor.
   * Subselection's right-click menu has *Pin to Frame n* (the playhead's frame). It is
     disabled, with the reason, outside the motion or out of order. It also has *Unpin from
     Frame n*, and for an end, a disabled *Pinned to Frame n* that says why.
   * The timeline draws a larger diamond with a hole at pinned frames.
   * Properties › Frame shows how many points are pinned, with *Pin Here* or *Unpin* at the
     playhead. That is the keyboard and screen-reader route.
8. **Headless.** `motion.pin_point {anchor?, frame?}` pins an anchor to the frame, or, without
   `anchor`, pins where the object is. `motion.unpin_point {anchor? | frame?}` unpins.
   `motion.info` and every path command's result list `pins` as `{anchor, frame, end, at}`, ends
   included, frames counted from the timeline's start. Unpinning is undoable and loses nothing
   irreversible, so it doesn't ask (ADR 0004).
9. **File format.** `pins` is additive, `#[serde(default)]`, and not written when it is empty. A
   file from before pins has none and evaluates exactly as before. Loading drops pins that
   can't hold: out of order, outside the position keys, on an end or a missing anchor, or on a
   tween without a path. An older build ignores the field. It still has the pins' X/Y keys, so
   it shows the same path with those frames as timing keys (close to the pinned motion, but not
   exact).

## Consequences

* "Be here on this frame" now holds through later path edits, the way it does in Animate. Bending
  a segment, moving other anchors or adding points never moves a pinned frame's place.
* ADR 0030's statement that a middle anchor is reached "when the timing says so" now holds for
  unpinned anchors only. ADR 0038's "anchors tied to frames: not in this slice" is this ADR.
* An X or Y key added in the Motion Editor on a path frame that isn't pinned is still a timing
  key (ADR 0030). Pinning is a deliberate act: the stage, F6, the menu, Properties or the command.
* Performance, measured with the optimised dev profile on the development machine, on an
  8-segment oriented path: evaluating a frame takes 3.3 µs per object without pins and 4.0 µs
  with 7 pins. The stage's 97 frame dots take 9.5 µs and 11.4 µs. Release builds are 6–16%
  faster. The cost is the per-call list of live pins and one binary search per pin.
* Not yet (later PRs):
  * dragging keys in time in the Motion Editor or on the timeline to retime a pin (re-pinning to
    another frame does it today);
  * two pins on one anchor (a hold is an ease or a timing key today);
  * the pins' frame tags avoiding each other where points crowd;
  * pins on loop paths whose start and end meet;
  * importing Animate's motion path keyframes from XFL (motion tween import still waits on
    samples, ADR 0016).
