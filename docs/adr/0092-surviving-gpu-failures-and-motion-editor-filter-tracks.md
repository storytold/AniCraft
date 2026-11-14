# ADR 0092: Surviving GPU failures mid-session, and Motion Editor filter tracks

* Status: accepted (2026-11-13)
* Relates to: roadmap M3 (GPU stage) and M5 (filters), issues #207 and #255, ADR 0006
  (cross-platform start-up), ADR 0015 (autosave), ADR 0016 (motion tweens), ADR 0029 and 0032
  (the GPU stage on the window's device), ADR 0081 (eases), ADR 0083 (retiming), ADR 0091
  (filter lists)

Two parts, one PR: both finish things earlier ADRs left as "later".

## Part 1: surviving GPU failures mid-session

### Context

Start-up already falls back between graphics interfaces (`startup::plan`, #207). A running
session did not. Under heavy GPU load the lead saw `Surface::configure` fail with "Failed to wait
for GPU to come idle before reconfiguring the Surface" (#255), and AniCraft closed.

The cause is in wgpu, not the driver: every error nobody catches goes to the device's
uncaptured-error handler, and **wgpu's default handler panics**. `Surface::configure` reports its
errors that way. eframe installs no handler, so the panic unwound out of eframe and the window
closed, with whatever was unsaved since the last autosave.

The GPU stage was already careful on its own: each call runs in error scopes and `catch_unwind`,
and a broken renderer handed every frame to the CPU. But it never came back, and a renderer on
the window's device never heard that the device was lost (ADR 0029 installed nothing on the
window's device).

### Decision

1. **The window's device gets our handlers** (`src/gpu_watch.rs`), installed by the app at
   creation:
   * an uncaptured-error handler that writes the error down (the log gets the first 20 in full,
     then one in 500) instead of panicking;
   * a device-lost callback that records the loss (a deliberate `Destroyed` isn't one).

   This replaces ADR 0029's "nothing is installed on the device". The window owns its device, so
   the app installs the handlers; the stage renderer still uses only its own thread-local error
   scopes.
2. **The surface is retried, not skipped for ever.** eframe asks `on_surface_status` what to do
   with a frame the surface couldn't give. Ours (set on every wgpu attempt) reconfigures after
   `Outdated` or a validation error. After three failed reconfigures in a row it makes the
   surface anew; a lost surface is made anew at once. Hidden windows and timeouts skip the frame.
   Before, a surface whose configure had failed stayed unconfigured, and every later frame was
   skipped.
3. **The app acts on the news each frame:**
   * **An error the window survived** autosaves unsaved work at once (not waiting for the
     interval), and the status bar says AniCraft carried on, at most once a minute.
   * **The device lost, or the surface failing for 8 s on end**, can't be repaired: eframe 0.36
     can't make a new device for a running window. So the app:
     * writes the work to the recovery folder one last time;
     * records the attempt in `Logs/graphics-lost`;
     * asks `main` to reopen;
     * closes the window.

     `main` then opens the window again with that attempt last. The Recover window offers the
     work back (a saved, unchanged document simply reopens). It does this at most twice per run.
4. **The startup record.** `startup::begin` returns the attempts to try last:
   * the one in `graphics-lost`, for the next 3 starts (`LOST_STARTS`, counted down each start);
   * then the one a start died in (the existing marker).

   After the third start the fastest way gets its chance again: the driver may have been updated,
   or the load that brought it down may be gone. A panic after the window was up that reads as a
   graphics failure (wgpu, surface, DXGI, Vulkan…) leaves the same record.
5. **The GPU stage falls back at once and retries with a bounded backoff** (`gpu_stage::Retry`):
   * **Falling back.** When the renderer reports itself broken, the service's rasterizer is
     removed and the CPU draws. The frame that failed was already redrawn by the oracle, so
     nothing is ever missing. A status-bar notice and a log line say why.
   * **Retrying.** The GPU is tried again after 3, 15, 60 and 300 s. Each try counts as a failure
     if it can't start. A change of the stage's size brings a try forward, but no sooner than 1 s
     after the failure. After five failures in a row the CPU keeps the stage for the session. A
     renderer that runs for 10 minutes without trouble starts the count afresh.
   * **The window's device lost.** It is never retried, because the window reopens. A renderer
     sharing that device is failed from outside (`GpuRenderer::fail`), since a device has one
     loss callback and the window owns it.
   * **Preferences and the status bar** show the new state (`Stopped`: why, and whether a retry
     is coming).

### Tests

* `anicraft-gpu`: a device destroyed mid-session (as a driver reset takes it) makes every call
  an error that says why, never a panic. A renderer failed from outside refuses with that reason.
  This needs a GPU or a software adapter, and skips without one.
* Shell, with a stand-in renderer (unit tests open no device). An injected loss:
  * the frame is drawn on the CPU;
  * the rasterizer goes, with a notice;
  * no retry before its time, then a retry that draws again ("back");
  * a longer wait after a second failure;
  * a resize bringing the try forward;
  * a failed retry waiting longer still.

  A lost window device is never retried. The backoff's rules are tested on their own.
* The surface policy; errors and the loss reaching the app once; the notice rate.
* The start-up plan after a mid-session death, the record's parsing, and its countdown over three
  starts, through the real log folder.

### Not in this part

* Recreating the window's device in place: eframe doesn't offer it. Reopening is the fallback.
* A GPU runner in CI to exercise real driver resets.

## Part 2: Motion Editor filter tracks

### Context

ADR 0091 left "motion-tween tracks per filter property … addressed by slot and parameter …
following moves and removals of filters" for later. Animate's Motion Editor animates filter
values beside position, transform and colour, and artists expect a blur to ease in like
anything else.

### Decision

1. **A property is also a filter's number.** `Property::Filter(FilterProp { slot, param })`:
   * `slot` is the filter's place in the instance's list (0 is applied first, at most 16);
   * `param` is a `FilterParam`: Blur X, Blur Y, Strength (%), Angle (°), Distance (px), and
     Adjust Color's Brightness, Contrast, Saturation and Hue.

   `Filter::params` says which numbers a kind has. Colours, ramps, quality and switches are not
   numbers on a curve and stay the object's own. Everything built on `Property` gets filter
   tracks for free, and they behave as any track: keys, per-segment eases (ADR 0081), moving keys
   in time, stretching, split and join (ADR 0083), the All lane, and `motion.move_keys`.
2. **Addressed as `filter.<slot>.<param>`** in commands and files (`filter.0.blur_x`). The other
   properties keep their names. `Property` serializes as that string (it was the serde name
   before, so nothing else changes).
3. **Files keep them apart.**
   * Filter tracks are written under the tween's `filter_tracks` key, and `tracks` holds only the
     twenty properties older builds know.
   * An older build ignores the new key and opens the file, with the filters holding still. If
     the filter tracks were mixed into `tracks`, an unknown property name would make it refuse
     the whole document.
   * Reading merges the two lists, so a hand-written file with a filter in `tracks` works too.
   * Files without filter tracks are byte-for-byte as before.
4. **Evaluation stays pure.** `MotionTween::apply` sets each tracked number on the target's
   filter at that slot. A track whose slot is empty, or whose filter has no such number (the
   filter was replaced by another kind), does nothing. Off filters are still left out by
   `evaluate` (ADR 0091), so no renderer or exporter changed. The CPU oracle, the GPU stage and
   every export draw the tweened values.
5. **Auto-key, like moving the object.** A command that edits the target inside the tween (Properties
   › Filters, `element.set_filters`, `filter.*`) goes through `finish_edit` as every edit does.
   * **The list kept its shape** (the same kinds in order): each number it changed is keyed at
     the playhead. On the span's first frame an untracked number changes the object's own value
     instead. Tracked numbers keep the object's own values underneath.
   * **The shape changed** (moved, removed, pasted): each filter is found again by what it holds,
     its tracks follow it to its new slot, and the tracks of filters that went go too. The moved
     filters get the object's own values back underneath.
   * Commands don't need to know any of this. One undo step, as before.
6. **F6 inside a tween** keys the filters' numbers too. `motion.info` lists filter tracks with
   their filter (`{"slot", "kind"}`) and the filters' values at the frame.
7. **The Motion Editor** lists each filter after the colour effect: the filter's name as a row
   (faint and "(off)" when switched off), its numbers under it. Each row has the value field, the
   key diamond, the curve and the ease footer, like the other properties. Screen readers hear
   "Blur: Blur X".
   Filters are added in Properties, as in Animate; the Motion Editor animates what is there.

### Tests

* Doc:
  * a blur tweening on its own curve, eased;
  * strength in percent;
  * inert tracks never panicking;
  * the file round trip with `filter_tracks` and old files unchanged;
  * key parsing and its refusals;
  * remapping;
  * retiming, stretching, split and join keeping every frame.
* Engine, through the commands:
  * keys, eases, `motion.move_keys` and removal on a blur, and the render following it;
  * errors that say why;
  * auto-key from `element.set_filters` with one undo step;
  * a switched-off filter keeping its tween;
  * tracks following `filter.move`, going with `filter.remove`, untouched by `filter.paste`;
  * F6, and the document round trip.
* Shell: the filter rows under their names. Screenshot: `--open motionfilters` (the Star's blur
  tweening, eased, in the Motion Editor).

### Not in this part

* Filter colours (shadow, glow, highlight) as tracks: they would need a colour row like the
  tint's.
* Quality and switches as stepped tracks.
* Animate's filter tracks from XFL motion tweens.

## Consequences

* A graphics driver failure now costs the stage's speed for a while, or the window for a moment,
  but never the work or the session.
* `Property` is no longer a closed list of twenty. Code that walks every property for an object
  (F6, join, the editor's rows) adds `filter_properties(target)`. Code indexing `Props` still sees
  only the twenty.
