# ADR 0047: Onion skin controls and Edit Multiple Frames

* Status: accepted (2026-11-10)
* Relates to: ADR 0002 (render pipeline, frame cache), ADR 0007 (tiled stage), ADR 0019/0032
  (GPU stage), ADR 0044 (selection across layers), ADR 0001 (undo), ADR 0040 (interface
  languages)

## Context

Onion skin was a toggle that drew the two frames before the playhead in red and the two after
in green, under the whole picture, with no ADR. `docs/requirements.md` asks for a before/after
range, tint and outlines (P1). Artists coming from Animate expect its controls: onion markers
on the frame ruler that they drag, Always Show Markers, Anchor Markers, the Onion 2 / Onion 5 /
Onion All presets, outlines, and Edit Multiple Frames to move a whole animation at once.

Behaviour was learned from Adobe's public help pages on onion skinning and Edit Multiple Frames
and from watching the program; nothing was copied.

## Decision

1. **Settings are view state, composition is a pure function.** `anicraft_engine::onion::
   OnionSkin` holds: on/off, Edit Multiple Frames on/off, the markers, tint, past and future
   colours, fade by distance, outlines, keyframes only. The shell owns one (`App::onion`), and
   passes it with each render job (`FrameJob::onion`, was a `bool`). `onion::compose(doc, scene,
   frame, skin, eval)` builds the display list; the document never changes for it (golden rule
   3). Render keys hash the settings (`OnionSkin::key`; nothing when it shows nothing), so the
   frame cache keeps every look apart and a second look is a hit: going back to a frame or
   undoing to a state already seen draws nothing new.

2. **Markers.** `Markers::Relative { before, after }` follow the playhead;
   `Markers::Anchored { first, last }` stay on their frames. `range(playhead, frames)` always
   lies inside the timeline with `first <= last` (a playhead past the end counts as the last
   frame). Dragging a marker never crosses the playhead (relative) or the other marker
   (anchored). Anchoring keeps the frames shown; un-anchoring keeps their distances from the
   playhead, and a marker the playhead has passed lands on it. Onion 2 and Onion 5 are relative
   ±2 and ±5; **Onion All** is anchored from frame 1 to `MAX_FRAME`, so it grows with the
   timeline.

3. **Which frames.** Onion skin draws every frame within the markers except the playhead's,
   nearest on top, the past one first at each distance. **Locked layers are not onion-skinned**,
   as in Animate (it is how artists keep a background out of the way), nor hidden ones.
   **Keyframes only** keeps, per layer, the frames where that layer has a keyframe; the keyframe
   under the playhead is left out unless it tweens (it would only repeat the playhead's frame).
   At most `MAX_EXTRA_FRAMES` (120) extra frames are evaluated, the nearest ones.

4. **Look.** An onion frame's colour effect is `alpha` × the past or future colour at
   `TINT_AMOUNT` (0.8: fills and lines inside it stay apart). With **fade**, alpha falls
   linearly from `NEAR_ALPHA` (0.5) next to the playhead to `FAR_ALPHA` (0.12) at the marker;
   without, every frame has 0.5. **Tint off** keeps the artwork's own colours (only faded).
   **Outlines** draw onion frames as hairlines in their colour and alpha, through the existing
   outline-mode path of the renderers (`DisplayItem::outline`), so the CPU oracle and the GPU
   stage draw them alike. The default colours are theme tokens (`onion_past` blue and
   `onion_future` green, as Animate users know them, brighter in high contrast); the onion menu
   changes them, and Reset goes back to the theme's.

5. **Layering.** Extra frames are **interleaved layer by layer**: each layer's onion frames sit
   under that layer's artwork at the playhead and over the layers below. (Before, they went
   under the whole picture, so an opaque background layer hid them.) Each evaluated frame is
   already back to front; one k-way merge by layer moves every item once. This is safe because
   a group's run lies within one layer and both renderers take mask artwork apart from what
   they draw. Extra frames are evaluated in parallel.

6. **Edit Multiple Frames** shows, on every visible, unlocked layer that holds artwork (not
   folders, the camera or armatures), every keyframe whose span reaches into the markers, at
   full strength, as it looks on its first frame, with the playhead's frame on top on each
   layer; onion frames are not drawn meanwhile. Its selection is a list of `FrameItem
   { layer, frame (the keyframe's first frame), element }` (element ids are shared between
   keyframes, so the frame says which copy), kept in the shell like the frame selection, since
   commands get explicit items (`Session::selection` stays the playhead's, ADR 0044).
   * `frames.artwork {first, last}` lists what it shows, with stage bounds;
     `frames.edit_multiple {first, last, dx, dy, items?}` moves the items given, or all of
     them, on each keyframe: one undo step (a drag is one gesture). A motion tween's path and
     X/Y keys move with its object (`MotionTween::translate`). Items outside the markers or on
     a locked or hidden layer are refused with nothing changed.
   * On the stage the Selection and Free Transform tools click, Shift-click, marquee (what lies
     wholly inside) and drag across those keyframes; Select All and Deselect All, and the arrow
     keys, act on them. The playhead's keyframe is boxed in the selection colour, the others a
     step lighter, and a badge on the stage names the frames.
   * A new command namespace, `frames`; plug-ins may call it (document edits).

   *Rejected:* a `range` param on `element.translate`. The selection it moves is the
   playhead's (pairs of layer and element), and the same id is on several keyframes; it would
   need a second kind of selection inside the engine for one command.

7. **In the editor.** The timeline has Onion Skin (Alt+Shift+O), Edit Multiple Frames and an
   onion menu (also on a right-click on the ruler and in View › Onion Skin Options): Onion
   Skin, Edit Multiple Frames, Outlines, Keyframes Only; Always Show Markers, Anchor Markers,
   Onion 2 / 5 / All; Tint, Fade by Distance, Before and After colours, Reset. The menu stays
   open while options change. The markers are brackets on the frame ruler in the past and future
   colours, with the frames between them tinted, pinned when anchored; dragging one shows the
   change live. Options, preset ranges and colours are saved in the preferences; on/off and
   anchored markers last for the session. Screen readers hear the markers as sliders ("Onion
   start marker", frame) and the menu by name; all text goes through `tr!`.

## Consequences

* Onion skin meets the P1 requirement (range, tint, outlines) and adds Animate's markers,
  keyframes only and Edit Multiple Frames.
* Onion frames now show over opaque lower layers (a visible change: the sample's sky no longer
  hides them).
* Measured at 1080p through the tiled path (release, 32 threads; `onion_timings`):

  | Scene | Off | ±5 tinted, fading | ±5 outlines | Edit Multiple Frames ±5 |
  |---|---|---|---|---|
  | Sample (7 items a frame) | 5.9–6.1 ms | 6.0–6.5 ms | 6.5–7.0 ms | 6.0–6.1 ms |
  | Stress, 2,000 items a frame (22,000 with ±5) | 7.2–7.9 ms | 16.2–17.2 ms | 34–39 ms | 6.6–7.3 ms |

  Composing the ±5 stress scene takes 3.0–3.3 ms (was 8.3 ms before the merge and parallel
  evaluation); a second look at the same 40 tiles is 0.02 ms, all cache hits. Hairline outlines
  cost more to rasterize than fills, so outline mode on very heavy scenes is the slow case.
* Not in this slice:
  * per-layer onion skin toggles, and onion skin inside a symbol being edited in place;
  * Edit Multiple Frames deleting, transforming (scale, rotate), copying or recolouring across
    keyframes; it moves only. Raw (merge) shapes moved there are not merged again with what
    they land on. (Done in ADR 0051, except recolouring; onion skin inside a symbol open in
    place too.)
  * a marker drag that also scrolls the timeline;
  * onion skin in Test Movie and exports (never: it is an editing aid).
