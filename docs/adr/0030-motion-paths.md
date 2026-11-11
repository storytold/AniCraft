# ADR 0030: Motion paths for motion tweens

* Status: accepted (2026-11-09); §4 between the ends superseded by ADR 0049 (pins)
* Relates to: ADR 0016 (motion tweens), ADR 0003 (file format), ADR 0004 (destructive
  actions), roadmap M2

## Context

ADR 0016 gave motion tweens property curves; position was two independent curves, X and Y, so
an object could only travel the path those curves trace (a straight line between keys, unless
X and Y ease differently). In Animate the position of a motion tween is a *motion path* on the
stage: a curve through the object's positions with a dot per frame, bent by dragging it with
the Selection tool or by its points and handles with the Subselection tool, and *Orient to
path* turns the object along it. Artists coming from Animate expect to see and shape that path
where the object is. Behaviour was learned from Adobe's public help pages and from observing
the program; nothing was copied.

## Decision

1. **The path is an optional field of the tween**: `MotionTween.path: Option<Path>` and
   `MotionTween.orient: bool` (`crates/doc/src/motion.rs`). The path is a `geom::Path` in the
   layer's space (stage space for top-level layers), kept in one normal form
   (`motion_path`): its first subpath as one move and cubics (lines and quadratics raised
   exactly, a closing edge written out), at most 1,000 segments, finite and inside the X/Y
   range, of some length. Commands and loading both normalise; a path that can't be repaired
   is dropped. Reusing `geom::Path` means the point-editing code (`geom::edit`) and the compact
   SVG serialisation come for free.
2. **The object travels the path by arc length, and the X/Y curves set the timing.**
   `geom::walk::CurveWalk` measures the cubic itself (dense samples of the curve parameter
   against arc length), so points and tangents are exact on the curve and an oriented object
   turns smoothly, not in the steps a flattened polyline would give. The *position keys*
   (frames holding an X or Y key) cut the span into legs; on each leg the X/Y curves run from
   one key's position to the next, and how far along that chord they are (signed) is how far
   along the leg's share of the path the object is, the shares being the chords' lengths. So:
   * a straight path reproduces the X/Y curves exactly (tested to 0.002 px with linear,
     eased and overshooting curves);
   * eases, holds and overshoot carry over (an overshooting ease runs past the path's end
     along its end direction and comes back);
   * a path that comes back to its start (chords adding up to nothing) is travelled at an
     even pace between the first and last position keys.
   We chose this over (a) measuring the arc length of the X/Y trace, which always runs
   forward and so loses overshoot, and (b) giving anchors their own frames (spatial keys),
   which is a larger model than this slice needs (see Consequences). Evaluation stays pure:
   `MotionTween::apply` places the registration point on the path and, with `orient`, adds the
   path's turn since its start to the rotation curve's value. The turn is relative so turning
   orient on never makes the object jump; it is the object's pose at the start that the path
   turns. Every export, hit testing, Test Movie and the HTML export see paths through
   `evaluate` with no extra code.
3. **The path's ends are the start and end positions.** `motion.set_path` writes the path's
   ends as the X/Y keys at the first and last position keys (at the span's first and last
   frame when there are none), leaving keys alone when an end didn't move, so bending a path
   never changes its timing. Removing the path (Straighten) leaves the object on the straight
   X/Y motion between the same ends, and turns orient off.
4. **Moving the object on a path reshapes the path** (auto-key, ADR 0016 §5): at or before the
   first position key the path's start moves, at or after the last its end, and in between the
   path bends until the object lands where it was put; no X/Y key is added, so the timing
   stays. A bend moves the segment's two control points by the least-squares solution of
   `b₁·o₁ + b₂·o₂ = d` (Bernstein weights `b₁ = 3(1−t)²t`, `b₂ = 3(1−t)t²`), which carries the
   grabbed point exactly; since a bend changes the path's length and the object then sits a
   little further along, `move_on_path` repeats it (at most 48 bends; a 40 px move takes 21
   to land within 0.002 px, a drag's per-frame moves far fewer). Rotation and the other
   properties key as before, on top of the turn.
5. **Splitting a span keeps the motion**: the second half gets the path from where the object
   is (`CurveWalk::tail`), its rotation keys shifted by the turn so far; past the path's end it
   holds still where the path ended.
6. **On the stage** (`src/motion_path.rs`), the path of the active layer's motion tween shows
   when its object or a frame of its span is selected: the curve in the theme's motion colour
   (`tween_motion`, the timeline's motion-span colour) on an `ink` halo so it reads on any
   artwork, a dot per frame (larger on position keys, ringed at the playhead). A tween without
   a path shows the straight path its X/Y curves trace (through their positions at the keys)
   and the first edit makes it a path. Selection: drag either end (before the object under
   it) or the path (where no object is) to bend it; Subselection: drag any anchor, or a
   selected anchor's handles (a smooth pair stays in line; Alt breaks it). Every change is
   `motion.set_path` inside a session gesture, so a drag is one undo step, and edits show in
   the same frame. Properties › Frame shows the path (Straight, or Curved · n segments, with
   Straighten) and an *Orient to path* checkbox; turning it on for a tween without a path
   makes the straight path its own.
7. **The file format stays version 3**: both fields are additive and `#[serde(default)]`,
   skipped when empty. Older files open unchanged; an older build ignores the fields and shows
   the straight X/Y motion between the same ends.

Headless: `motion.set_path {d}` (full SVG path data), `motion.clear_path`,
`motion.orient {orient}`, and `motion.info` reports `path`, `orient` and `position` (on a path
its `values.x/y` are the timing curves, not where the object is). Removing a path is undoable
and loses nothing irreversible, so it doesn't ask (ADR 0004).

## Consequences

* A path's anchors are not tied to frames: an anchor in the middle of the path is reached when
  the timing says so, not on a frame of its own. Moving the object at a middle frame bends the
  path through that point rather than adding an anchor there. *Since ADR 0049, anchors can be
  pinned to frames, and moving the object at a middle frame pins a new anchor there instead;
  this timing still applies, leg by leg between pinned anchors, and is unchanged for files
  without pins.*
* The X/Y values the Motion Editor shows are timing on a path, not the object's place. Editing
  them retimes the motion along the path.
* Performance: evaluating a tween on an 8-segment path with orient takes 10–15 µs per object
  per frame, and the stage's 240 frame dots 20–30 µs together (one `CurveWalk` per call;
  measured on the development machine, release and dev profiles).
* Adding, deleting and converting path points and moving the whole path came next, in
  ADR 0038, which also explains why anchors tied to frames wait for a model change.
* Not yet (later PRs): ~~position keys as anchors (spatial keys)~~ (done in ADR 0049), copying and pasting
  a path from a stroke or onto a guide, ease along the path for loop paths, the path shown
  for tweens inside symbols edited in place, snapping the ends, and importing motion paths
  from XFL (motion tween import still waits on samples, ADR 0016).
