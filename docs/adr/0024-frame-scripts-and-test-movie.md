# ADR 0024: Frame actions, frame scripts and Test Movie

* Status: accepted (2026-11-09), first slice
* Relates to: ADR 0013 (ActionScript: the action IR and the pure player), ADR 0017
  (WebAssembly plug-ins), ADR 0004 (destructive actions), requirements U2, roadmap M6

## Context

ADR 0013 fixed the shape of interactivity: one small typed **timeline action IR** that every
front end lowers to (actions edited in a panel, WebAssembly frame scripts, later the
ActionScript subset and the HTML canvas exporter), run by a **pure player** so `evaluate` and
every export stay script-free. ADR 0017 gave us the sandbox. This ADR places the parts, fixes
the player's semantics and decides the frame-script ABI addition, for a first slice that
artists can use without writing code: stop a movie on a frame, a button that goes somewhere,
movie clips told what to do by name, and a Test Movie window to try it.

## Decision

### Placement

| Part | Where | Why |
|---|---|---|
| Action IR (`Action`, `Target`, `FrameRef`, `ButtonEvent`, `EventHandler`, `PluginScript`, `FrameScript`) | new crate **`anicraft-actions` (L0)**, serde only | It is stored in documents (so `anicraft-doc` needs it) *and* produced by front ends that know nothing about documents: the ActionScript parser is planned at L0 (ADR 0013), and an L0 crate can't use an L1 type. Targets are name paths and frames numbers or labels, resolved at run time, so the IR needs no document. |
| Player (`PlayerState`, `start`, `step`, `input`, `tree`, `hit`) and `evaluate_player` | `anicraft-doc` (L1), `player.rs` beside `eval.rs` | It walks timelines exactly as evaluation does and must stay in lock-step with it. `evaluate` gains nothing but an optional player state threaded through one private function; with none it is unchanged. |
| Frame scripts on the plug-in host, a JSON view of the player, headless runs | `anicraft-engine` (L3), `player.rs` | The player calls scripts through a `ScriptHost` trait it defines; the engine implements it over the session's plug-in registry (L0 `anicraft-plugins`). The doc crate never sees `wasmi`. |
| Commands | `anicraft-engine`, `cmds_actions.rs` | Everything is a command. |
| Actions panel, Test Movie window, Properties rows, timeline mark | shell | View state only. |

### Model

* `Keyframe.script: Option<FrameScript>`: what runs when the playhead **enters** the keyframe
  (its first frame), either `Actions(Vec<Action>)` or `Plugin { id, entry }`. One field rather
  than two keeps "actions *or* a script" true by construction; ADR 0013's tier 1 adds a third
  variant (kept ActionScript source) to the same enum. Hidden, guide and folder layers don't
  run (what is drawn is what plays, as in our exports).
* `ElementKind::Symbol.events: Vec<EventHandler>`: on button **and** movie clip instances, one
  handler per event (`click`, `press`, `release`, `over`, `out`). Handlers run on the
  timeline the instance is on (AS2's `on(release)` and the common AS3 pattern agree).
* Instance names are the existing `Element.name`; `instance.set_name` validates them as
  identifiers (letters, digits, `_`, `$`; not `this`, `root`, `parent`) and only on movie
  clips and buttons, as Animate does.
* Frames in the IR are **0-based**, like every frame in the engine and its commands; the app
  shows them 1-based. (ActionScript's `gotoAndStop(1)` lowers to frame 0.)
* Both fields are `#[serde(default)]` and skipped when empty, so older files open unchanged and
  the format version stays 3; loading repairs hostile lists (lengths, targets, numbers).

### Player semantics (one `step` = one frame)

1. Every playing timeline advances one frame: the main timeline into the next scene at a
   scene's end and back to the first at the movie's end (Test Movie loops, as the player does
   by default); movie clips round their own loop.
2. Movie clips that appeared start at their first frame; those that left are forgotten (an
   instance keeps its playhead while the same element id stays on stage across keyframes,
   matching `evaluate`'s `instance_start`).
3. The actions of every frame **entered** run, parent before child, top layer first. A `goto`
   enters its frame too, so its actions run; the rest of the list still runs (as code after
   `gotoAndStop` does). A frame entered twice in one step runs once.
4. Pointer events of that frame are routed: the topmost button (or movie clip with handlers)
   under the pointer by its **Hit frame** (the fourth; the Up frame when the button has fewer
   frames) gets `over`/`out`; `press` on down; `release` on up over it, then `click` when it
   was also pressed there. Buttons show Up, Over or Down; a shorter button timeline shows its
   last frame. `input` routes events between frames without moving playheads (the window
   uses it so buttons answer the pointer at once).
5. `set_property` (x, y, alpha, visible, rotation, scale) overrides that property on the
   instance while it lives; `visible = 0` hides it and makes it unclickable, and it keeps
   playing.
6. Targets resolve from the action's timeline: `""`/`this`, child names, `parent`, `root`, dot
   paths; graphics are see-through for names (their contents belong to the timeline they are
   on), buttons and graphics have no playhead. Anything that can't run (unknown target or
   label, a graphic's playhead) writes a line to Output and the movie goes on.
7. **Budgets per step**: 10,000 actions, 1,000 frames entered, 10,000 instances walked,
   1,000 output lines (the rest counted), 256 events. A `goto` loop ends in one Output line,
   not a hang. Nothing reads a clock or a random source, and maps are ordered, so the same
   document, state and events always give the same step (tested).

### Frame scripts (ABI v1, additive)

* Manifest `kind: "frame-script"` with optional `entries` (default `["main"]`). A keyframe
  names a plug-in id and entry. `plugin.run` refuses frame scripts; the Commands menu doesn't
  list them.
* On enter-frame the engine runs `ac_run` in a **fresh instance** with
  `{entry, frame, label, timeline}`. Requests: `{"op":"player"}` (the player state: scene,
  frame, playing, every movie clip's path, frame and playing flag, the pointer, the script's
  own timeline), `{"op":"action","action":{…}}` and `{"op":"actions","actions":[…]}` (checked
  IR, at most 256 per run), `log` (to Output). `command`, `document` and `elements` are
  refused: a script can't change or read the document, only steer playback.
* Limits (`Limits::frame_script`): **20 M instructions per frame, shared** by every script of
  that frame, 100 ms wall clock per run, 1,000 requests, 64 KiB requests, 1 MiB responses,
  50 log lines. A failing or runaway script emits nothing (no half-run), uses up the frame's
  budget, writes one Output line, and the movie plays on. Measured: ~50 µs per run for a
  script that reads the player and emits an action (debug build).
* No state survives between runs in this slice (fresh instance each time); scripts read what
  they need from the player view. Persistent script state is a later, deliberate addition.

### Commands

`actions.set` (keyframe actions or a frame script by `{symbol?, layer?, frame?}`; or an
instance's handler by `{element, event}`; an empty list clears), `actions.clear`,
`actions.list` (everything, every scene and symbol), `instance.set_name`, and
`control.test_movie` (Control › Test Movie, **Ctrl+Enter**): in the app it opens the window,
headless it plays `frames` steps with timed pointer `events` and returns the Output lines and
the final player state. Edits are routine and undoable (ADR 0004: one step each, no
confirmation). Command plug-ins may call `actions.*` and `instance.*` but not `control.*`.

### App

* **Actions panel** (Window › Actions, **F9**, as in Animate): a *Frame* tab for the keyframe
  at the playhead on the active layer (or any movie clip's layer and keyframe), and an
  *Instance* tab for the selected button or movie clip: its name and its events. Actions are
  added from a menu and edited in place: targets from the document's instance names (dot
  paths, marked button or movie clip), frames by number or picked label, scenes from the
  scene list, properties from a list. No typing except a trace message.
* **Properties**: *Instance* name and *Events* rows at the top for movie clips and buttons;
  an *Actions* row for the keyframe; both open the panel. The timeline marks keyframes that
  run actions with an "a" after the dot.
* **Test Movie** window: a snapshot of the document played by the player at its frame rate,
  the pointer routed to buttons (hand cursor over them), Restart (Ctrl+Enter), Pause, Step,
  the scene and frame, and an **Output** panel. The stage editor, playhead and history are
  untouched. It is an in-app window (screenshots, all platforms alike); a separate OS window
  can come with multi-viewport support.

## Consequences

* Exports, the stage and `evaluate` behave exactly as before: documents with actions render
  the authored timeline (as Animate's video export does). Only Test Movie and headless
  `control.test_movie` play interactively, and the HTML canvas exporter will reuse the IR.
* The player is fast enough to run on the UI thread: ~1–3 µs per step headless for the
  sample movies; the window's cost is the CPU render of each new frame (≈5 ms at 760 px).
* Not in this slice: ActionScript parsing (ADR 0013 tiers), HTML canvas export, sound
  actions, keyboard events, drag, variables and conditions, persistent script state, scripts
  inside graphic symbols (they never run, as in Animate) and button frame scripts, masks in
  hit tests, a separate OS window for Test Movie (done in ADR 0062: a window of its own where
  the system has windows, this floating window as the fallback).
