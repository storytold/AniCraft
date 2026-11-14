# ADR 0097: Command macros (saved commands)

* Status: accepted (2026-11-14)
* Relates to: roadmap M6 (command macros), ADR 0001 (undo over persistent snapshots), ADR 0004
  (destructive actions), ADR 0017 (WebAssembly plug-ins and `plugin_may_call`), ADR 0065
  (templates: files in the settings folder the shell passes in), ADR 0073 (groups refuse
  timeline commands), ADR 0016 (auto-keying motion tweens)

## Context

Animate users repeat the same few edits all day (rotate, fade, nudge, add a keyframe, set an
ease) and expect to keep them: in Animate's History panel they select steps and choose **Save As
Command**; the command then appears at the end of the **Commands** menu, can get a keyboard
shortcut, and **Manage Saved Commands** renames and deletes them. **Run Command…** runs a
command file someone sent. Behaviour was learnt from Adobe's public help pages for the History
panel and the Commands menu and by observation; no Adobe code, file format or asset was used
(Animate stores JSFL scripts; ours are JSON lists of registry commands).

Everything in AniCraft is already a registry command, so a macro is naturally "these command ids
with these params, in order". The questions were: where the steps come from, what a step that
names this document's things (element and layer ids) means elsewhere, what a macro may call,
how a run stays one undo step and transactional, where the files live, and how macros get
shortcuts from a shortcut map whose actions are all known at build time.

## Decision

1. **History records the commands of each step.** `history::Snapshot` gains
   `commands: Option<Arc<[Recorded]>>`, the `(id, params)` of every top-level mutating command
   that changed the document since the previous step: one for a plain edit, all of them for a
   gesture (a drag), the marquee's provisional cut and the edit that used it together. Undo and
   redo carry them with the label. A live preview's revert drops what it reverted; undo, redo,
   healing a cut and opening a document drop what wasn't recorded. Consecutive plain
   `element.translate {dx, dy}` add up into one, so a drag of 600 moves is one step. A step keeps
   at most 4096 commands and 1 MiB of params (`None` past that: "too many to keep"), and the
   bytes count against the undo budget. Nested commands (a plug-in's, a macro's) are not
   recorded on their own: `macro.run` records its steps (`Session::record_as`), so a run can be
   saved again with other steps; `plugin.run` records itself and therefore can't be kept.

2. **Steps are rewritten to act on the selection, or refused with the step named.** Save As
   Command never stores ids of this document's things. For each recorded param in `TARGETS`
   (`layer(s)`, `element(s)`, `id(s)`, `shape`, `path`, `symbol`): it is dropped when the
   command's params doc marks it optional (`key?`) and, but for layers, says its default is the
   selection; the step then acts on the selection or the active layer. Any other target, and any
   param in `FIXED` (`bitmap`, `sound`, `folder`, `bone`, `parent`, `items`, and a bone's
   `from`/`to`), refuses the step: "step 4 “Move Points” can't be kept: it works on particular
   objects of this document". The params docs are the contract the registry already keeps for
   every command (the guide's command reference is generated from them); a test pins the
   behaviour on representative commands. Selections and the playhead are not steps (selection
   commands don't change the document), so a macro runs on whatever is selected, as in Animate.

3. **A macro may call what a plug-in may, and never `macro.*`.** `macro_may_call` is
   `plugin_may_call` plus the `macro` namespace refused. So macros never open or write files,
   use the clipboard, undo, change preferences or run plug-ins or other macros (no recursion),
   and plug-ins may not call any `macro.*` command (reading files and running macros is outside
   the sandbox). The rule is checked when a file is read and again before each step runs.

4. **A run is one undo step and transactional.** `macro.run` is a mutating command whose steps
   run through `execute_nested` (checked and atomic, no history of their own). At the first
   failing step it returns "“Turn and Fade” stopped at step 3 of 3 (Taller row): … Nothing was
   changed." and `Session::execute` puts the document, selection and playhead back. The step is
   labelled with the macro's name. `execute_nested` now also refuses timeline commands inside an
   open group (as `execute` does, ADR 0073) and finishes each command's motion-tween auto-key on
   its own, so a step in a macro keys a tween exactly as the command alone does (plug-in runs
   benefit too).

5. **Files: one small JSON file per macro in the settings folder.** `Commands/<name>.json` =
   `{"format": "anicraft-command", "version": 1, "steps": [{"cmd", "params", "label"}]}`. The
   file's name is the macro's name (no name inside to go stale when a file is renamed or shared);
   a bare JSON array of `{cmd, params}` (an `anicraft-cli run` script) reads too, and `command`
   is accepted for `cmd` (the MCP spelling). Reading is bounded: 1 MiB, 1000 steps, known
   commands only, object params, labels cut to 200 characters without control characters; a
   newer version is refused with a clear message. The order of the Commands menu is
   `Commands/.order.json` (a dot name no macro can have); files not in it follow by name. Names
   follow the template rules (no path characters, no Windows device names). As with templates
   (ADR 0065) the shell passes the folder (`platform::commands_dir`, `ANICRAFT_COMMANDS_DIR`
   for tests); screenshot runs use a temporary folder; the browser build has none yet.

6. **Commands.** `macro.run {name + folder | path | steps}` (Commands › Run Command…, asks for a
   file), `macro.list {folder}` (Commands › Manage Saved Commands…), `macro.history` (every
   history step as Save As Command keeps it: `keep`, `why`, `rewritten`), `macro.save {folder,
   name, history | steps}`, `macro.rename`, `macro.move`, `macro.delete` and `macro.import`.
   Replacing a file asks first and deleting always does (ADR 0004: undo can't bring a file back),
   for every front end. Everything works headless: `anicraft-cli exec doc macro.run '{"path":
   …}'`, `anicraft-cli run doc macro.json` (it reads either format) and MCP's `anicraft_run`.

7. **Shortcuts through the existing map.** A macro is an action `macro:<name>` in the "Commands"
   group of Keyboard Shortcuts, bound, reset and checked for clashes like any other; Manage Saved
   Commands' **Shortcut…** opens the editor on it, already waiting for the keys. Actions are
   `&'static str` throughout the map and the editor, so macro names are interned once each
   (bounded at 4096 distinct names a run) rather than reworking the shortcut system for owned
   strings. Bindings live in the preferences under the action id and survive a start before the
   folder is read; a rename moves them, a delete drops them; a chord bound twice by a hand-edited
   file stays with the action listed first.

8. **Desktop flow.** In the History panel Ctrl+click picks steps and Shift+click a range (a plain
   click still travels); a step's context menu picks it or saves it. **Save As Command…** (the
   panel's footer, or the context menu) shows the picked steps with what was rewritten and why a
   step can't be kept, and asks for a name; Save is disabled until every step can be kept. Saved
   macros appear at the end of the Commands menu with their keys. Manage Saved Commands lists
   them with Run, ↑/↓ (reorder), Shortcut…, Rename…, Delete… and Add from File….

## Consequences

* Every undo step costs a few more bytes (its commands' JSON), counted in the undo budget.
* A step whose params name document things the command can't do without (moving particular
  path points, painting bone weights, placing a particular Library item) can't be saved; the
  message says which step and why. Steps that ran as plug-ins can't be saved either.
* The rewrite relies on params docs following the `key?` and "default: the selected …"
  conventions; a new command that names an element without saying so is refused, never
  replayed wrongly.
* Interning keeps each macro name for the life of the process (bytes per name).

## Not yet

Replaying picked steps without saving them (Animate's Replay), a slider for travelling history
separately from picking, macros in the browser build (no folder) and the command palette,
editing a macro's steps or params in the app, recording selection changes as steps, and
macros that ask for a value when they run.
