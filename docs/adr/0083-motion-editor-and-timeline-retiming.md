# ADR 0083: Motion Editor and timeline retiming

* Status: accepted (2026-11-13)
* Relates to: roadmap M2 (Motion Editor), ADR 0016 (motion tweens), ADR 0049 (motion path
  pins), ADR 0061 (timeline polish), ADR 0075 (dockable panels), ADR 0004 (destructive actions)

## Context

ADR 0016 shipped the Motion Editor with values and eases but no way to move a key in time. A
span could be resized, but its property keys stayed where they were, so a shortened tween lost
its end and a lengthened one held still. Artists coming from Animate expect to drag keys
sideways in the Motion Editor, to drag a tween span's end and have the motion stretch, to drag a
whole tween span (Alt-drag copies it), and to use **Split Motion** and **Join Motion**. We
learned how these behave from Adobe's public help pages and from using the program. Nothing was
copied.

The side docks' tab strip also clipped names under the panel menu button ("Layer Deptl").

## Decision

1. **Retiming is pure data on `MotionTween`** (`crates/doc/src/motion/retime.rs`), so
   `evaluate` doesn't change. Key frames stay offsets into the span.
   * `move_keys(keys, by, last)` moves the named keys by one shared delta and returns how far
     they really went. The delta is clamped so that no key leaves `0..=last` (a stale key past
     the end isn't pulled back) and no key lands on or passes a key of its track that isn't
     moving. Keys that move together don't block each other, so order is always kept. A move
     **stops at the neighbour** rather than failing, which is what a drag needs. Values and
     eases travel with their keys. A pinned frame's X and Y keys always move together, and its
     pin moves with them (ADR 0049).
   * `stretch(from, to)` maps every key and pin frame `f` to `round(f · (to−1)/(from−1))`. The
     first frame stays on the first and the last lands on the last, as in Animate. Keys that
     would round onto one frame are spread onto the frames next to it, first forwards and then
     back from the new last frame. Only when a track has more keys than the span has frames do
     keys merge: on frame 0 the first key stays, elsewhere the later key wins. A stretch there
     and back is the identity whenever no keys merged (tested).
   * `join(base, frames, next, next_base)` has two cases. If `next` is exactly what
     `split_at(frames)` of this tween gives, and its object is this one evaluated at the cut,
     the join gives back the original tween, keys past the first span included. So
     **split then join is an exact round trip**, motion paths and pins included. Otherwise the
     join works property by property: this tween's keys before the cut, a hold key on the last
     frame where the value jumps, then `next`'s keys shifted by `frames`. Rotation is unwrapped
     by whole turns, so the curve doesn't spin back. Every whole frame keeps its look. Two
     tweens that each have their own motion path don't join in this slice.
2. **Commands** (each one undo step; a drag is one session gesture):
   * `motion.move_keys {keys: [{property, frame}], by}`.
   * `timeline.resize_span {frames, scale_keys?}`. Later frames move with the end, as with
     F5 / Shift+F5. Motion keys stretch unless `scale_keys: false`. This one command replaced
     the insert/remove-frame pair the span-end drag used to send.
   * `timeline.move_spans {from, to, by, layer_by?, copy?}`. The spans that start in the range
     move whole, keeping their length and tween, and replace what they land on. The frames they
     leave are empty (a blank keyframe), as when keyframes are dragged onto another layer
     (ADR 0061, whose lift-and-place code it shares). `copy` leaves the originals.
   * `motion.split` cuts the span with the same copy that F6 makes outside a motion tween
     (`MotionTween::split_at`). The first half keeps its keys, including those past the cut
     (they aren't reached), and that is what makes the exact join possible.
   * `motion.join {to?}` joins adjacent motion spans of one object (same element kind, the
     other artwork unchanged). It refuses, with a message, a keyframe that has a sound or
     actions, because joining would drop them (ADR 0004: refuse rather than lose silently). A
     label moves to the joined span if it has none.
   * Command namespaces are existing ones, so the plug-in policy already covers them.
3. **Shell, Animate-style.**
   * **Motion Editor.** Picked keys are view state: `(property, offset)` pairs that persist
     across properties. Shift- or Ctrl-click adds a key to the pick or takes it out. A new
     **All** lane under the ruler has one diamond per keyed frame; clicking it picks every
     property's keys there, and dragging it moves them. A key drag locks to an axis after
     4 points of travel: sideways retimes, up and down changes the value. This choice avoids
     accidental value changes while retiming. The curve is now focusable, and Left and Right
     (Shift: ten) move the picked keys. That is the keyboard route the accessibility pass
     asks for. A readout shows the key count, the offset and the target frame.
   * **Timeline.** A press on a motion span picks the whole span and dragging moves it
     (Ctrl-click picks a frame). A selection that covers whole classic, shape or motion tween
     spans also moves as spans. Other selections keep the keyframe move of ADR 0061. **Alt**
     at the drop copies. Dragging a span's end stretches a motion tween's keys; **Shift** at
     the drop keeps them in place, and the readout says which will happen. **Split Motion**
     and **Join Motion** are in the span's context menu and in **Modify › Timeline**.
4. **Dock tab strip.** The strip ends before the menu button and is clipped there.
   `fit_tabs` gives the tabs their own widths when they fit. Otherwise it shrinks the longest
   names to one shared cap (water-filling), so short names stay whole, with an ellipsis and the
   full name in the tooltip, down to 44 points. Below that the strip scrolls with the wheel,
   arrows show at an edge that hides tabs, and the shown tab keeps as much of its name as fits
   and is scrolled into view. A test checks this at the narrowest side dock (160 points).

## Consequences

* Older files open unchanged: no field was added, and retiming only rewrites existing ones.
* Shrinking a span below its number of keys merges keys (undoable, and said in the guide).
* Not yet: joining two independent motion paths; Animate's span-based selection preference for
  classic tweens; moving the first key of a span by dragging the span's left edge; snapping
  retimed keys to other properties' keys; and the timeline's own tab row (top and bottom docks),
  which still lays out in one row.
