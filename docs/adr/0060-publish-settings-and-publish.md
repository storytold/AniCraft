# ADR 0060: Publish Settings and Publish

* Status: accepted (2026-11-11)
* Relates to: ADR 0003 (file format), ADR 0004 (destructive actions), ADR 0017 (plug-in
  policy), ADR 0028 (HTML canvas export), ADR 0036 (web build), ADR 0040 (interface languages)

## Context

AniCraft can export a PNG frame or sequence, an animated GIF, an SVG frame, a sprite sheet and
its JSON map, video through the user's ffmpeg, Lottie, an HTML canvas page and the soundtrack,
each as its own command with its own prompt. Animate collects these in **File › Publish
Settings** (Ctrl+Shift+F12): you tick formats, set each one's options and output name, and
the settings are saved **in the document**. **File › Publish** (Shift+F12) then writes every
ticked format at once. Profiles can be saved and reused. Artists publish many times a day, so
publishing has to be one key, not a run of dialogs, and must not freeze the editor while a
long video renders.

Behaviour was learned from Adobe's public help pages and from using the feature. No Adobe
code, SDK or asset was used.

## Decision

1. **The profile is part of the document** (`anicraft_doc::publish::PublishProfile`, the
   document's `publish` field). Every format is always there, in a fixed order (HTML canvas,
   GIF, video, Lottie, sprite sheet, PNG sequence, PNG, SVG, audio), and each one is on or off.
   Each has an output template and a **typed** settings struct holding the same options its
   export command takes: frame range, scale, loop, background, columns, padding, title,
   sound, fit, sample rate, bits. Typed settings let the file format check the values
   (`sanitize` brings anything a file holds into range). A free-form JSON blob would let
   values drift away from what the exporters accept.
   * `#[serde(default)]` everywhere. A file from before this ADR, or one missing some fields,
     opens with the defaults: **the HTML page is on and the rest are off**, matching Animate
     for an HTML5 Canvas document. A profile still at its defaults is not written, so the
     files of people who never open Publish Settings don't change. The file format version
     stays at 3.
   * Changing the settings changes the document: it is undoable and marks the file unsaved,
     as in Animate. Drags and typing are grouped into one undo step.
   * The ffmpeg path is not in the profile. It belongs to the computer (the `ffmpeg` param,
     `ANICRAFT_FFMPEG`, or the PATH), not to the document.
2. **Outputs are relative to the document's folder.** `{name}` is the document's file name
   without its extension, and is the only placeholder. An output can go into subfolders
   (`web/{name}.html`), which are created as needed. It **can't be absolute, contain `..`, or
   name a drive**. A document opened from someone else must never be able to write elsewhere
   on this computer. Users who want files elsewhere can save the document there, which keeps
   publishing predictable. If an output has no extension, the format's extension is added. A
   wrong extension is an error (for example `.png` for the GIF). If two formats would write
   the same file (compared case-insensitively, as Windows and macOS do), the plan is refused
   and the message names both formats. An untitled document can't publish on the desktop:
   the shell asks for Save As first. In the browser, files go to the downloads, so an
   untitled document publishes as "Untitled".
3. **Publishing reuses the export commands.** `engine::publish::plan` turns each format that
   is on into a `Step`: the export command id and its params. `Publisher` then runs the steps
   in order on a **session of its own**, built from a copy of the document taken when Publish
   started, through the same `RunFn` that File › Export uses. Nothing about an export is
   implemented twice, and later edits never change a publish already running. If one format
   fails (no ffmpeg, a GIF that would be too large), the others still publish. Some formats
   are **skipped with a reason**: video in a browser (no ffmpeg), and audio when the movie
   has no sounds.
4. **Cancel and progress** go through a shared `publish::Control` (an atomic cancel flag and
   a frames-done/frames-total counter) set on the session (`Session::job`). The frame-based
   exporters (GIF, sprite sheet, PNG sequence, video) call `publish::tick` after each frame
   or batch. When the run is cancelled they stop with `EngineError::Cancelled`:
   * GIF and sprite sheet write nothing, because their file is written only at the end.
   * Video kills ffmpeg and deletes its unfinished file.
   * The PNG sequence keeps the frames it has written, and the report says so.
   * Formats that haven't started don't run.
   * HTML, Lottie, SVG, PNG and audio are quick single passes and finish the one they are
     on. Outside a publish, `job` is `None` and the exporters behave exactly as before.
5. **One report.** Each format gets an `Outcome`: published, skipped, failed or cancelled,
   plus its files and a message (the exporter's "approximated or left out" note for HTML and
   Lottie, or why it was skipped or failed). `Report::summary` joins these into one sentence
   per format, in order. `file.publish` returns the whole report as JSON. The shell shows it
   in a window, one row per format.
6. **Commands**, so all of this works headless, over the CLI and over MCP:
   * `publish.settings_get` (File › Publish Settings…, Ctrl+Shift+F12; the shell opens the
     window) returns the profile and, for each format, where it would go now.
   * `publish.settings_set` changes one `format` (switch, output, options; unknown options
     and out-of-range values are refused, with nothing changed) or sets a whole `profile`.
   * `file.publish {formats?, confirm?}` (Shift+F12) runs the publish on the caller's thread
     and returns the report. The CLI and MCP wait for it, which is what a script wants.
7. **Overwriting follows ADR 0004.** Replacing files on disk can't be undone, so
   `file.publish` needs confirmation when outputs already exist (for a PNG sequence, a folder
   that already holds exported frames). The single confirmation lists the files. To keep
   publishing a one-key action, **files this session's own publishes wrote are replaced
   without asking again** (`Session::published`, which is not stored in the document). So the
   first Publish over existing files in a session asks once, and the next ones don't. The
   desktop export commands keep their own confirmations, but Publish's confirmation covers
   them (its steps carry `confirm`).
8. **The desktop runs a publish on a worker thread**, `anicraft-publish`. The UI only reads a
   channel and the atomic counters each frame, so the editor never waits. The **Publish**
   window shows each format as waiting, running (a frame progress bar, or a spinner for
   formats without a frame count) or done, with a **Cancel** button. Closing the window also
   cancels. When the run finishes, the window becomes the report. **The web build** has no
   threads, so the same `Publisher` runs one format per frame on the UI thread, and the
   window still shows which format is running. The files come back as downloads, and
   **several files come back as one ZIP** (`publish::zip`: stored entries, UTF-8 names,
   folders kept, CRC-32 from the hardened `.fla` reader's module; the tests read every
   archive back with that reader). If the thread can't be started, the desktop falls back to
   the same stepping.
9. **Presets belong to the user, not the document.** They are kept in Preferences
   (`publish_presets`, a name → profile map, at most 100 presets, names up to 64
   characters, each one sanitized on load). Publish Settings can apply one (one undo step on
   the document), save the current settings as one (replacing a preset of the same name), or
   delete one. A preset is gone for good once deleted, so Delete takes two clicks in place
   (Delete, then "Delete “name”") rather than opening a modal dialog. Headless users already
   have presets as data: `publish.settings_get`'s profile can go into another document's
   `publish.settings_set {profile}`.
10. **Plug-ins** may read the settings (`publish.settings_get`) but not change them, and
    can't publish. `file.*` is already refused, and `publish.settings_set` would change what
    a later Publish writes and where.

## Consequences

* One new document field, which defaults for older files, and no change to the file format
  version.
* Publishing is one key after the first setup. It never blocks the editor, and what it does
  is reported once.
* A publish uses the document as it was when Publish started. Edits made while it runs go
  into the next publish.
* Only `{name}` is supported as a placeholder, and outputs stay below the document's folder.
  Both can be loosened later without changing the file format. Loosening them would need a
  new rule for files from other people.

## Not yet

* Importing Animate's own publish settings (`PublishSettings.xml` in XFL).
* More placeholders (`{scene}`, `{date}`), outputs outside the document's folder, and a
  per-format "open when done".
* Progress for HTML, Lottie and audio, and cancel inside those formats (they are single
  passes).
* Exporting and importing presets as files (headless, the profile JSON already does this).
* Publishing in a browser that keeps running while the tab is in the background (one format
  per frame stops when the browser stops drawing the page).
