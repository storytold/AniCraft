# ADR 0013: ActionScript compatibility

* Status: accepted (2026-11-08) as scope and placement. **Tier 1 built** (2026-11-09, see
  "Tier 1 as built" below). **Tier 2 built** (2026-11-10, see "Tier 2 as built"). **Tier 2.5,
  small logic, built** (2026-11-11, ADR 0052)
* Relates to: requirements U2 (scripting surface), roadmap M6, ADR 0009 (XFL import),
  `docs/legal/compliance.md`

## Context

The project lead asked for ActionScript on the roadmap if it fits our constraints, at low
priority, since it serves a smaller group of users. U2 already settled AniCraft's own scripting:
the Craft-family WebAssembly plug-ins, with frame scripts on the same sandbox. The ActionScript
question is about compatibility for people who bring Animate projects with code in them.

What those users have, in order of how common it is:

1. **Timeline control in frame scripts**: `stop()`, `play()`, `gotoAndPlay`/`gotoAndStop` by
   frame number or label, `nextFrame`/`prevFrame`, the same calls on a named instance
   (`hero_mc.gotoAndStop("run")`), and button clicks wired to those calls with
   `addEventListener(MouseEvent.CLICK, …)`. Most animation and e-learning work only uses this.
2. **Small logic**: variables, `if`, loops, `trace`, setting instance properties
   (`x`, `alpha`, `visible`), simple functions.
3. **Applications and games**: classes in packages, the document class, linked symbol classes,
   the full `flash.*` platform API (display list, events, networking, sound, text), compiled
   to bytecode for a player.

Current Animate documents run ActionScript 3.0 only; ActionScript 1 and 2 live only in old
files. Today the XFL importer drops frame scripts without a word.

Our constraints: Rust only, no unsafe, never crash on hostile input, everything is a command,
time is a function (`evaluate` is pure), clean room (public documents and observation only),
permissive dependencies from crates.io, and Adobe names used only nominatively.

## Decision

ActionScript fits as a **compatibility front end in three tiers**, built in order, each one
useful alone. Only the first two are planned. It stays low priority: in M6 after the
WebAssembly plug-ins, frame scripts and HTML canvas export, and it may slip past M8 without
blocking anything else.

1. **Keep (tier 1).** Scripts are content and are never lost.
   * The document model gets `Keyframe.script: Option<Arc<Script>>`, where `Script` holds the
     source text verbatim and its language (`ActionScript3`, `ActionScript1or2`, `Other`), plus
     symbol linkage and document class names as plain strings. Saved in `.anicraft` (a minor
     format version with `#[serde(default)]`, so older files open unchanged).
   * The XFL importer reads `<Actionscript><script>` on frames (scenes and symbols) and the
     linkage attributes. The import report counts them as kept, not run. *As of this ADR it
     reports them as skipped; that line moves when tier 1 lands.*
   * An **Actions** panel (the generic name, see Legal) shows and edits a keyframe's script
     in a monospace editor with line numbers. Keyframes with a script get a small marker in
     the timeline. Commands: `script.set`, `script.clear`, `script.list`, so agents and the
     CLI can read and write scripts too.
   * Rendering and every export ignore scripts. Tier 1 runs nothing.
2. **Run the timeline subset (tier 2).** The common case from item 1, plus a little of item 2.
   * New crate `anicraft-script` (layer 0; it knows the language, not the document): a hand-written
     lexer and recursive-descent parser for an ActionScript 3 subset, with bounded nesting
     depth, source length and token count. It lowers what it accepts to a small, typed
     **timeline action IR** (stop, play, go to frame or label (optionally in a scene), next
     and previous frame or scene, target an instance by its name path, a button event
     handler for click, over, out, press and release, `trace` output, and a few instance
     properties). Anything outside the subset is reported with line and column; the script
     is kept and not run. Nothing is half-run.
   * **The IR is the shared vocabulary.** WebAssembly frame scripts (U2) call the host with
     the same actions, and the HTML canvas exporter writes the same actions out as
     JavaScript. One runtime, many front ends; ActionScript is not a second engine.
   * **Time stays a function.** Scripts never change the document. Interactive playback is a
     separate, pure player in `anicraft-doc`: `PlayerState` (each timeline's playhead and playing flag) is
     stepped by `step(doc, &state, &events) -> state` and drawn by evaluating the document
     at that state. The authoring stage and every export stay script-free and
     deterministic; a **Test Movie** preview window and the HTML canvas export use the player.
     Fuel limits a step's work, so a bad loop can't hang the app.
   * `navigateToURL` and other effects outside the movie are ignored in preview (reported in
     its output), so opening a file can't make AniCraft do things outside the document.
3. **Not planned: the full language and platform (tier 3).** Classes, packages, the
   `flash.*` API, a bytecode virtual machine and SWF export with compiled code. That is a
   second product (a player), and open-source players for old content already exist. We
   revisit only if users ask for it. If we ever do, SWF export first: its specification is
   public (see compliance §4), and the tier-2 IR compiles down to it.

## Why this fits the constraints

* **Rust only, no unsafe, no new licences.** Tiers 1 and 2 are our own code; tier 1 needs no
  dependency at all. No embedded JavaScript or Flash engine.
* **Never crash.** The parser is bounded and fuzzed with the other importers; the player is
  fuel-limited; unsupported code is reported, not guessed.
* **Everything is a command** (`script.*`), so it works headless and over MCP.
* **Clean room.** The language is learned from public documents: the ECMAScript 3rd edition
  standard (ECMA-262) that ActionScript 3 builds on, Adobe's public ActionScript 3.0 language
  and component reference and its "Learning ActionScript 3.0" guide, and observing what
  self-authored files do. No Adobe code, SDKs, compilers or player internals. The XFL
  `<Actionscript>` element comes from the same public format descriptions and self-authored
  files as the rest of ADR 0009. Sources get a row in `ATTRIBUTION.md` when the code lands.
* **Legal.** "ActionScript" is an Adobe trademark. Docs and compatibility messages use it
  nominatively to say which language a script is written in (as import messages already name
  Animate). Feature names stay generic: the **Actions** panel, **frame scripts**, **Test
  Movie**. A programming language itself isn't protected by copyright; counsel review
  covers this together with XFL (compliance §7).

## Consequences

* Animate users keep their code when they open their files, and simple interactive pieces
  (menus, "click to continue", looping characters with stops) play in preview and in HTML
  canvas exports without being rewritten.
* Documents with scripts look the same in every export as before, because exports render
  the authored timeline, not a played session. That matches Animate's own video export.
* Games and applications written in ActionScript don't run in AniCraft and won't in this
  plan. Their artwork and timelines still come across, and their scripts are kept.
* The subset is a moving line. It grows from user reports of the "not run" messages, never
  by guessing what unsupported code would have done.

## Tier 1 as built (2026-11-09)

Tier 1 follows the decision above. Where the build refines it:

* **Where scripts live.** A kept script is a third variant of the frame-script enum ADR 0024
  put on keyframes: `Keyframe.script: Option<FrameScript>` with
  `FrameScript::Source(Arc<SourceScript>)` beside `Actions` and `Plugin`. `SourceScript` (in
  `anicraft-actions`, L0, so the tier-2 parser can read it) holds `language` (`actionscript3`,
  `actionscript1or2`, `other`) and `text`, byte for byte. A separate field would have allowed a
  keyframe to hold actions *and* a script, and then nobody could say which one runs once tier 2
  lowers scripts to actions. One field keeps "one of them" true by construction. Setting
  actions on a keyframe with a script replaces it (undo restores it), as switching between
  actions and plug-ins already did. The `Arc` means copying a keyframe, taking an undo
  snapshot or handing the document to the render thread never copies the text.
* **Class names.** `Symbol.linkage: Option<Linkage { class, base }>` and
  `Document.document_class: String`, as dotted identifiers (at most 256 characters). An older
  document's `linkageIdentifier` is stored as `class`. All of these fields are
  `#[serde(default)]` and are left out of the file when empty, so the format stays version 3,
  as it did for ADR 0024's fields, and older files open unchanged.
* **Limits and repair.** A script is at most 1 MiB. Loading drops a script that is empty or
  too long *whole*, because cutting code would change it, and drops class names that aren't
  names. The XFL importer reports dropped scripts the same way.
* **XFL.** `<Actionscript><script>` on frames in scenes and in every kind of symbol is kept
  (graphic and button timelines and guide layers too: nothing there runs, but it is content).
  Linkage comes from `linkageClassName` (else `linkageIdentifier`) unless `linkageExportForAS`
  turns it off, and `linkageBaseClass` gives the base class. The language and the document
  class come from the publish profile (`PublishSettings.xml`, the profile marked current, its
  `ActionScriptVersion` and `DocumentClass`). When a file doesn't say, the language is
  ActionScript 3.0. A missing or unreadable profile changes nothing, because it holds settings,
  not content. These element names come from public descriptions of the format and are checked
  against self-authored files only, as the rest of ADR 0009 is. The report gains a **kept, not
  run** list next to "not yet supported" (counts of frame scripts by language, linkage classes
  and the document class). Scripts on instances (older documents' `on (…)` handlers) and linkage
  on bitmaps and sounds have no place yet and are reported as left out.
* **Commands.** `script.set` (`text` and/or `language`; empty text clears), `script.clear`
  (kept scripts only, never actions), `script.list` (every kept script with its text, every
  linkage class and the document class) and `script.set_class` (a symbol's linkage, or the
  document class). They reach any symbol's timeline and guide layers, which `actions.*`
  can't (`Scope::Keeps`). **Plug-in policy:** command plug-ins may call all of `script.*`.
  Scripts are document content like actions and frame labels, which plug-ins already read and
  write (`actions.*` is allowed). Writing a script runs nothing, and when tier 2 runs scripts
  it does so in the pure, budgeted player, which can't reach outside the movie. Frame-script
  plug-ins still can't call commands at all.
* **App.** The Actions panel's Frame tab gets a *Type* row (Actions, Plug-in, Script). Script
  shows the language, the line count and the text in a monospace (`Type::Path`) editor with
  line numbers. Typing edits the document live, and a burst of typing is one undo step. Tabs
  are typed into the code. Esc leaves the editor. A file's CR LF line ends show as one break,
  and the stored text is changed only by an edit. Symbols with kept scripts appear in the
  panel's timeline list whatever their kind. The timeline's "a" marker covers kept scripts.
  Properties shows the symbol's linkage *Class* and the document *Class* in documents that use
  scripts. A class field commits when it loses focus, because a half-typed name isn't a name
  yet. Test Movie says how many kept scripts don't run (once, not every time one passes), and
  the HTML canvas export lists them in its report.
* **Not in tier 1:** scripts on instances, linkage on bitmaps and sounds, a document-wide
  default language, syntax colouring, and exporting scripts back to XFL.

## Tier 2 as built (2026-11-10)

Tier 2 follows the decision above. Where the build refines it:

* **Where the language lives: a module, not a crate.** The parser is `anicraft_actions::as3`
  (`lex.rs`, `parse.rs`, `lower.rs`), inside the L0 crate that holds the action IR, rather than
  a new `anicraft-script` crate. Two reasons. The layering rule (`src/architecture_tests.rs`)
  lets a crate use only *lower* layers, so an L0 `anicraft-script` could not name the L0 IR it
  lowers to; ADR 0024 put the IR at L0 for exactly the front ends that lower to it. And a kept
  script's compiled form is a pure function of its text, so it is cached on the script itself:
  `SourceScript::compiled()` fills a `OnceLock` the first time it is asked (never saved, never
  compared, emptied by a clone). The document shares scripts by `Arc`, so undo snapshots, the
  render thread, Test Movie, the HTML export and the importer all reuse one compile. A separate
  crate would have needed a cache keyed outside the script, or a compile on every frame
  entered. The module knows the language and the IR only; the crate stays serde-only.
* **The subset.** `stop()`, `play()`, `gotoAndPlay`/`gotoAndStop` (a frame number, 1-based, or
  a label; optionally a scene), `nextFrame()`, `prevFrame()`, `nextScene()`, `prevScene()`;
  the same on instance-name paths (`hero_mc.arm_mc.play()`, `this.parent`, `parent`, `root`,
  the casts `MovieClip(…)`, `Object(…)` and `… as MovieClip`); `addEventListener` for
  `MouseEvent.CLICK`, `MOUSE_DOWN`, `MOUSE_UP`, `ROLL_OVER`/`MOUSE_OVER` and
  `ROLL_OUT`/`MOUSE_OUT` (or their strings), with a function declared in the script or written
  in place, whose body is made of these calls; `e.currentTarget` in a listener is the instance
  listened to; `trace` of literals and of `+ - * / %` over them, joined and formatted as
  ActionScript does (`Number.toString`, ECMA-262 §9.8.1); setting `x`, `y`, `alpha`,
  `visible`, `rotation`, `scaleX`, `scaleY` to such values; a call to one of the script's own
  functions runs its body in place (no recursion; 16 deep at most). `import`, comments, missing
  semicolons (insertion as ECMA-262 §7.9), type annotations and `return;` read as the language
  does.
* **Never half-run.** Lowering accepts only what the IR says exactly. Anything else (variables,
  `if`, loops, reading a property, `removeEventListener`, other events, classes) is a problem
  with its line and column, and the whole script doesn't run. The parser reads more of the
  language than runs (statements and expressions in general, skipping loop headers and
  `switch`/`try` bodies by balanced brackets), so the problem names the construct and where it
  is, and lowering lists every problem (up to 32), not just the first. A syntax error stops at
  the first.
* **Outside the movie.** `navigateToURL`, `sendToURL`, `fscommand` and `ExternalInterface.call`
  are ignored and listed as *notes*; the script still runs. `buttonMode` and `useHandCursor`
  are noted too (the pointer is already a hand over everything clickable).
* **Bounds.** 256 KiB of source runs (the 1 MiB kept limit still holds), 65,536 tokens,
  nesting 64 deep (statements and expressions; deeper is a problem, never a stack overflow),
  100,000 statements lowered counting inlined copies (calls that fan out), 256 steps per
  script and per listener, 64 listeners per script, 32 problems. The player's per-step
  budgets (ADR 0024) count each step as one action. A hostile-input test (token soup and raw
  bytes, 5,000 scripts) runs in well under a second; it must never panic. Measured (dev
  profile, this build's optimised debug): a typical frame script compiles in ~9 µs, a 200 KB
  one of 2,500 functions in ~11 ms, once per script.
* **Listeners in the player.** The IR gains no new action: a compiled script is a list of
  `ScriptOp`s, each an `Action` or a `Listen` (a target, a `ButtonEvent`, the function's name
  and its actions). Running a `Listen` adds the listener to `PlayerState.listeners` under the
  instance it names (buttons and movie clips; a graphic, the main timeline or a missing name is
  an Output line). Adding the same function for the same event again changes nothing, as in
  ActionScript (an inline function counts as one function, so a looping frame doesn't pile up
  copies). A listener lives while its instance and its script's timeline stay on the stage. On
  an event the instance's own handler (ADR 0024) runs first, then its listeners in the order
  added, each on its script's timeline. A movie clip with a listener can be clicked, as one
  with handlers can. `nextScene()` in a movie clip's own script writes an Output line instead
  of moving the main timeline (a clip has one scene in ActionScript); the Actions panel's Next
  scene action keeps its meaning.
* **Where it shows.** Test Movie's Output starts with one line per kept script: "Frame script
  runs: Scene 1 › Actions, frame 1", or "not run" with the first problem; the window counts
  the scripts that don't run. The Actions panel's Script view says whether the script runs
  (and how many steps it lowered to), lists the notes, or lists each problem with its line and
  column above the code and marks those lines in the gutter; clicking a problem puts the
  cursor there. The XFL import report lists frame scripts that run apart from those kept.
  `script.set` and `script.list` say `runs` and `problems`; `script.check {text, language?}`
  compiles a text without storing it and returns its steps. The HTML canvas export writes the
  same steps for `player.js`, which mirrors the Rust player's listeners (tested step by step in
  Boa, state, output and display list alike); scripts outside the subset stay out of the page
  and are counted in its report.
* **Messages.** Problems are engine messages, in English like the rest of the engine's
  (`docs/localisation.md`); the panel's frame around them is translated.
* **Not in tier 2 yet:** variables and conditions, reading properties, `removeEventListener`,
  `Event.ENTER_FRAME` and keyboard events, functions declared on other frames of the timeline,
  event bubbling from a clip's children to its listeners, `e.target`, `getChildByName`,
  sound and `stage`, and ActionScript 1.0 and 2.0. Each is reported where it is, so the
  subset can grow from what users' files actually use.
