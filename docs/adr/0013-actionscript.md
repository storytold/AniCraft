# ADR 0013: ActionScript compatibility

* Status: accepted (2026-11-08) as scope and placement; the parts are built later (roadmap M6)
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
