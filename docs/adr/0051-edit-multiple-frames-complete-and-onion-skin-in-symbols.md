# ADR 0051: Edit Multiple Frames, complete; onion skin inside symbols

* Status: accepted (2026-11-11)
* Relates to: ADR 0047 (onion skin and Edit Multiple Frames), ADR 0045 (symbols edited in place),
  ADR 0031 / 0041 (merge drawing, the Selection tool on raw shapes), ADR 0004 (destructive
  actions), ADR 0017 (plug-in policy), ADR 0040 (interface languages)

## Context

ADR 0047 shipped Edit Multiple Frames as a move-only tool. Raw (merge-drawing) shapes it moved
were not merged again with what they landed on. ADR 0045 switched both onion skin and Edit
Multiple Frames off while a symbol was open in place, because the stage then draws a picture
document of the symbol rather than the scene. Artists coming from Animate expect the rest of
Edit Multiple Frames: delete, cut, copy and paste across keyframes, Free Transform of the whole
multi-frame selection about one pivot, Align, arrow nudges, and onion skin inside the symbol
they are animating.

Behaviour was learned from Adobe's public help pages on onion skinning and Edit Multiple Frames
and from watching the program. Nothing was copied.

## Decision

1. **Commands on explicit items.** As in ADR 0047, the multi-frame selection is a list of
   `FrameItem { layer, frame (the keyframe's first frame), element }` kept by the shell. Every
   edit is a `frames.*` command (`crates/engine/src/multi.rs`) taking `{first, last}` (the
   markers) and `items?`. An item that the markers don't show, or that sits on a hidden or
   locked layer, is refused and nothing changes. Without `items`, a command acts on everything
   shown. Each command is one undo step, and each answers with `items`, the selection
   afterwards (merged pieces get new ids), so a front end never has to ask again.
   * `frames.edit_multiple` (moved from `onion.rs`) moves. `frames.transform` takes a stage
     affine `{a..f}` (what Free Transform sends) or `{rotate, scale_x, scale_y, skew_x, skew_y,
     pivot?}`, where the pivot defaults to **the centre of every item's bounds together**: one
     shared pivot, so each keyframe's copy lands exactly where the same transform of the whole
     multi-frame picture puts it. `frames.align {edge, to_stage?}` lines the items up with the
     edge of their bounds together. With one item, or with To Stage, it aligns to the stage.
     So Align Left on a walk cycle lines every frame's figure up.
   * `frames.delete`, `frames.copy`, `frames.cut`. Delete is an undoable routine edit with a
     status-bar hint, like `edit.delete` (ADR 0004 tier 3). Whole keyframes are never removed.
   * `frames.paste` is **Paste in Place onto the keyframes between the markers**. Each copied
     chunk remembers the keyframe it came from (`ClipLayer::frame`, `None` for a copy made on
     one frame). A multi-frame copy goes back keyframe by keyframe, onto the target layer's
     keyframe covering the frame each chunk came from. If those keyframes are outside the
     markers, the chunk is skipped and counted. A one-frame copy goes onto every keyframe of the
     layer shown. Artwork from several layers goes back on those layers when they are all
     edited there; otherwise everything goes on the active layer (or `layer`). **Paste**
     (Ctrl+V) with Edit Multiple Frames on pastes onto the current frame, as Animate does, and
     the pasted artwork becomes the multi-frame selection. Without Edit Multiple Frames, a
     multi-frame copy pastes like any copy, and several keyframes of one layer count as one
     layer (`paste_targets` maps each distinct source layer once).
   * **Re-merge.** Raw shapes that moved, were transformed or aligned, or were pasted go back
     into the drawing **on their own keyframe** as `draw.drop` does on one frame: taken out,
     then `merge_in` on top of what they touch. The editing helpers work at the playhead, so
     `at_keyframe` runs them with the playhead on the keyframe's first frame and restores it.
     A drag passes `merge: false` on each step and calls `frames.merge {items}` when it ends,
     inside the same gesture, so the drag stays one undo step and shapes are not cut while
     they pass over others.
   * **Motion tweens.** A tween's object can be moved across frames (its path and X/Y keys move
     with it, `MotionTween::translate`). Scaling, rotating or skewing it on several frames is
     refused with a message, because its look comes from property keys, not one transform.
   * **Plug-ins** may call the document edits (`frames.edit_multiple/transform/align/delete/
     merge`). `frames.copy/cut/paste` are refused like every clipboard command.

   *Rejected:* a multi-frame selection inside the engine (`Session::selection` with frames).
   Commands already take explicit items, so headless callers and agents get the same power with
   no second selection model to keep in step with undo. *Rejected:* a separate multi-frame
   clipboard. It would make Copy on one frame and Paste in Place across frames disagree about
   what was copied. One clipboard with a frame on each chunk keeps Animate's single clipboard.

2. **In the editor** (`src/onion.rs`). With Edit Multiple Frames on, the Edit menu, shortcuts
   and the stage's right-click menu act on the multi-frame selection: Cut, Copy, Paste (current
   frame), Paste in Place (every keyframe), Clear (Delete/Backspace), Select All, Deselect; the
   Align panel and Ctrl+Alt+1…6 (To Stage honoured); Rotate 90° and Flip about the shared
   centre. The **Free Transform** tool draws one box around the selection on every keyframe,
   with the pivot at its centre. Its handles scale (Shift proportional, Alt from the centre),
   rotate (Shift 15°) and move it all, one undo step per drag, merged on release. Arrow keys
   nudge, one step each, merged. Duplicate, Convert to Symbol, Break Apart, Arrange and
   Distribute are disabled there and say why. Status lines go through `trn!`.

3. **Onion skin inside symbols.** The open symbol's timeline is the timeline both features
   compose. `StagePicture::front` names the picture's top layer (the open symbol's instance),
   the timeline, its frame and its placement `at`. A render job made with `FrameJob::inside(
   front)` draws, when onion settings show anything, the timelines around the symbol as they
   are, then `onion::compose_timeline(symbol timeline, frame, skin, eval)` with
   `eval = evaluate_timeline_at(doc, tl, f, at)`. That is the instance's own evaluation with
   the symbol's layers as top-level ones, so onion frames still interleave layer by layer and
   Edit Multiple Frames keeps per-layer keyframes. `compose` for a scene is now this function on
   the scene's timeline. The job key mixes in the front, so the frame cache keeps symbol and
   scene looks apart. Markers, keyframes-only and every other option refer to the symbol's own
   timeline (the playhead and frame count the shell already uses there, ADR 0045). Picking and
   bounds were already in the symbol's space.

## Consequences

* Edit Multiple Frames now does what Animate users use it for: re-posing, rescaling or
  re-aligning a whole animation, and cleaning up or copying artwork across keyframes. Raw
  drawings stay connected after a move.
* Onion skin and Edit Multiple Frames work inside a symbol edited in place, rotated instances
  (opened upright, ADR 0045) included.
* Measured (optimised dev build, `multi_timings`, 24 keyframes × (144 raw squares + 50
  objects) = 4,656 items shown):

  | Operation | Time |
  |---|---|
  | List what is shown | 1.4 ms |
  | Move 1,200 objects (one drag step) | 2.8 ms |
  | Rotate 1,200 objects about the shared pivot | 2.9 ms |
  | Move and merge 288 raw squares across 24 keyframes | 5.6 ms |
  | Copy 1,200 objects, Paste in Place | 3.1 ms |
  | Delete everything shown | 1.8 ms |

  The sample's spinning star with onion skin ±5 inside the symbol renders its tiles in 2.3 ms on
  the GPU stage.
* Not in this slice:
  * Duplicate, Distribute, Arrange, Convert to Symbol and recolouring across keyframes;
  * scaling, rotating or skewing a motion tween's object on several frames (it moves);
  * Subselection, the Width tool and point edits across keyframes;
  * a pivot the artist can drag (it is the centre of the box, or the opposite handle with
    scale, as on one frame);
  * per-layer onion skin toggles, and onion skin of the parent timeline's frames around an
    open symbol (only the symbol's own frames are composed).
