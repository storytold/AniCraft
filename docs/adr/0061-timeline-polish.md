# ADR 0061: Timeline polish: per-layer onion skin, outline mode, layer height, Layer Properties, frame view

* Status: accepted (2026-11-11)
* Relates to: ADR 0047 / 0051 (onion skin), ADR 0045 (symbols edited in place), ADR 0004
  (destructive actions), ADR 0017 (plug-in policy), ADR 0022 (accessibility), ADR 0040
  (interface languages)

## Context

Artists coming from Animate expect the everyday timeline details around onion skin and layers:
leaving a background layer out of onion skin, seeing a layer as outlines in its colour, making a
sound or a busy layer's row taller, one dialog for a layer's settings, and frame width presets.
ADR 0047 and 0051 left per-layer onion toggles and onion frames of the timeline around an open
symbol as "not yet". Outline mode existed, but `doc::evaluate` applied it everywhere, so a layer
left in outline mode exported as hairlines.

Behaviour was learned from Adobe's public help pages on layers, onion skinning and the timeline,
the public JSFL reference (`layer.height`: 100, 200 or 300) and from watching the program.
Nothing was copied.

## Decision

1. **What is saved in the document and what is view state.** Per-layer onion, outline, colour
   and height belong to the layer (Animate saves them in the file; a background left out of onion
   skin should stay out tomorrow). They are `Layer` fields with `#[serde(default)]` and skipped
   at their defaults: `onion: bool` (default true) and `height` (rows, 1–3, read through
   `Layer::height()` which clamps whatever a file says). Older files open unchanged. Frame width
   and Short rows are view state in the shell; Short is kept in the preferences.
2. **Commands** (`crates/engine/src/cmds_layer.rs`, all undoable, plug-ins may call them):
   `layer.set_onion {onion?, solo?, layer?}` (toggle, set, or Animate's Alt-click solo: this
   layer alone; solo again on the only layer left on turns every layer back on; folders and the
   camera are left alone), `layer.set_color`, `layer.set_height {height: 100|200|300}`,
   `layer.set_properties` (Layer Properties: name, type, colour, outline, height, show, lock,
   onion, every param checked before anything changes, one undo step) and `layer.properties`
   (read). XFL import reads `heightMultiplier`.
3. **Onion skin per layer.** `onion::ghosted` also needs the layer's toggle. Edit Multiple Frames
   does not: the toggle is about the faded frames, and hiding keyframes from a multi-frame edit
   because of it would be surprising.
4. **Outline mode on the stage only.** The evaluation context has a `stage` flag: on for the
   evaluations the stage edits with (`evaluate_scene`, `evaluate_timeline[_at]`), off for the
   movie (`evaluate`, which every export draws, `evaluate_scene_camera`, the player and button hit
   areas). So exports, the player, Test Movie and Preview Through Camera draw the artwork as it
   is. *Rejected:* stripping outlines in each exporter. There are many and a new one would forget.
5. **Onion frames of the parent timeline.** `Front` (the open symbol in a stage picture) keeps
   `parent: Option<Around>`, the timeline the symbol was opened from (without the instance), its
   frame, placement and dim opacity. With onion skin on (not Edit Multiple Frames),
   `display_list_inside` draws that timeline through `compose_timeline` too, every item in one
   group at the dim opacity, so it fades as one, as the dimmed instance it replaces did. Relative
   markers reach as many frames from the parent's frame; anchored markers (which name the symbol's
   frames) reach as far as they do from the symbol's playhead. Only the timeline right under the
   symbol gets onion frames; deeper ones stay still. The other picture layers are drawn one by
   one, bottom up, so the open symbol stays in front.
6. **In the timeline** (`src/timeline.rs`, `src/layer_props.rs`):
   * An onion column before the eye: bright while onion skin is on and draws the layer, faint
     while it is off, struck through when the layer is left out. Alt-click solos. The layer's
     right-click menu has Onion Skin too.
   * The colour square toggles View as Outlines (as before, now with a tooltip). The colour
     chip on the row's edge opens the layer colour: twelve swatches, a hex field and More… (Layer
     Properties' full picker). It is an accessible popup.
   * Every layer has its type's icon (normal layers get a new AniCraft icon, a row with a keyframe
     and its span). Double-clicking the icon opens **Layer Properties**; double-clicking the name
     still renames. Also on the right-click menu (Properties…) and Modify › Timeline.
   * **Layer Height** 100 / 200 / 300 % on the right-click menu and in Layer Properties. Rows are
     laid out by height (`rows_fitting`, `max_first_row`, `first_showing`, `row_at`), so
     scrolling, bringing the active layer into view and dragging across rows follow the real
     rows.
   * **Frame zoom moves to the timeline's bottom bar**, at the right end of the scroll bar, as
     in Animate. The header had grown (onion options, the camera) until the zoom field covered
     the tween-kind menu at the default window size. The header now has the dock's full width
     and fits at 1600 × 940; a test measures it (`the_header_fits_at_the_default_window_size`).
     Sideways scrolling stays the fallback for small windows, large interface scales and long
     translations. *Rejected:* an overflow menu for the onion and camera toggles. They are
     everyday controls, and hiding them costs a click every time.
   * **Frame View** menu beside the zoom slider: Tiny, Small, Normal, Medium, Large (frame widths
     4, 8, 12, 18, 28 points; the slider still sets anything between) and **Short** rows (20
     points; a 200 % layer is two short rows).

## Consequences

* A layer left in outline mode no longer exports as hairlines (a fix as much as a feature).
* Layer Properties and the per-layer toggles work headless, over the CLI and MCP.
* Short rows are below the 28-point target size. They are a deliberate choice for dense
  timelines, as in Animate, and their toggles shrink with them.
* Measured: no new per-frame cost for scenes. With a symbol open in place and onion skin on, the
  parent's onion frames are one more `compose_timeline` of the parent (the same cost as onion skin
  on the scene).
* Not in this slice:
  * a per-layer onion toggle on folders (toggling everything inside);
  * onion frames of timelines deeper than the one right under the open symbol;
  * Layer Properties for several layers at once, and its Transparency (layer alpha) option;
  * a timeline Preview / Preview in Context frame view (thumbnails in frames);
