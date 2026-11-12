# ADR 0059: Files from the desktop, recent files and a start screen

* Status: accepted (2026-11-11)
* Relates to: ADR 0004 (destructive actions), ADR 0006 (platform layer), ADR 0022
  (accessibility), ADR 0036 (web build), ADR 0040 (interface languages), ADR 0050 and 0053
  (Library folders, Place on Stage), roadmap M8

## Context

Files came in only through File › Open and the Import dialogs. Artists coming from Animate expect
the everyday paths: drop a file on the window, reopen something from File › Open Recent, and a Home
screen when the app starts with nothing open. All three touch the shell, the platform layer and the
engine, and two of them keep state (the recent list, its pictures) between sessions.

## Decision

1. **Routing is a pure function in the engine** (`cmds_drop::kind_of`, `cmds_drop::route`). A file's
   kind comes from its name (an XFL folder from its `DOMDocument.xml`): document (`.anicraft`, `.fla`,
   `.xfl`), drawing (SVG), image (PNG, JPEG), sound (WAV, Ogg, MP3) or unsupported. With the target
   (stage, Library, elsewhere) it gives one action: open, import to the stage, import to the Library,
   or refuse with a reason. The shell's `drop::plan` adds what only the window knows: one document
   opens at a time (the others in the drop are left out, and the status line says so), and over the
   start screen nothing imports.
2. **`file.drop` does the import, one undo step** (`{paths, to: stage | library, x, y, folder}`).
   On the stage each image and sound goes into the Library and is placed as `library.place` places
   it: centred on the drop point in the active layer's keyframe at the playhead (sounds onto that
   keyframe). A one-layer SVG goes into the same keyframe (`file.import`'s new `layer: "active"` and
   `cx`/`cy`); a drawing with several layers or masks keeps its own layers, as Import to Stage gives
   it. On the Library, images and sounds go into the folder under the pointer, numbered when the
   folder already has that name. Every file is checked before anything is imported; a locked layer
   refuses the whole drop. Documents go through `file.open` with the unsaved-changes guard
   (`request`), never through `file.drop`.
3. **Where a drop lands needs the pointer, which the desktop doesn't give during a drag.** winit
   0.30 reports dragged files without a position, and Windows and macOS send no pointer moves while
   a drag is over the window. So a dropped file waits up to 0.35 s for the pointer's first move
   (the system sends one as the drag ends) and is routed there; without one, media land on the stage
   in the middle of the view and the status line says so. In a browser our own `dragover` listener
   on the canvas keeps the position, so the target is exact and the highlight follows the pointer;
   the browser then reads the bytes into the in-memory store (`vfs`) and the same `file.drop` runs.
   The highlight shows the target when the position is known (the stage outlined with a crosshair
   and "Place … on <layer> at frame N", the Library folder row or panel with "Add … to <folder>",
   the whole window for a document); when it isn't, it says what each place does. winit 0.31's drag
   events carry a position: moving to it makes the desktop exact too.
4. **The recent list is the session's, persisted by the shell.** `cmds_recent::RecentFiles` lives on
   `Session` (it survives New and Open, like plug-ins): pinned favourites first (at most 10), then
   the last 10 files opened or saved, most recent first, never twice (paths compared without case
   on Windows and macOS). `file.open` and `file.save` add to it (not restored autosaves or untitled
   opens); `file.open_recent`, `file.recent_list`, `prefs.recent_pin`, `prefs.recent_remove` and
   `prefs.recent_clear` (pinned files stay unless `pinned: true`) work in the CLI and MCP like any
   command. The window copies the list into `Prefs::recent` whenever it changes and puts it back at
   start; the engine never reads the settings folder. Preferences parse the list leniently (a bad
   entry is skipped, not every preference reset) and sanitise it. A missing file stays listed, greyed
   out with Remove, because a disconnected drive comes back; `file.open_recent` on it says it is no
   longer there. `prefs` is a new namespace: plug-ins may not call it.
5. **Thumbnails are captured on save** (and on open when none exists): the CPU oracle renders the
   first frame to fit 240 × 135 (`cmds_recent::thumbnail_png`, about 1.2 ms and 6 KB for the sample
   in a release build), kept as `Thumbnails/<key>.png` in the settings folder (browser storage,
   base64, on the web), the key a 64-bit FNV-1a of the normalised path. Screenshot and test runs
   write nothing.
6. **The start screen replaces the workspace, never covers it.** It shows when AniCraft starts with
   no file to open (Preferences › `start_screen`, a checkbox on the screen itself), from File › Start
   Screen, and from Open Recent in the palette. The menus and status bar stay; Esc or "Back to the
   document" returns to the document behind, untouched. New offers HD 1280 × 720, Full HD 1920 ×
   1080, Square 1080 × 1080, Vertical 1080 × 1920 and a custom size with a frame rate; Open…, the
   sample animation and the recent files (picture, name, folder, Pin or Remove) sit beside it. Every
   choice goes through `request`, so unsaved work asks first, and the screen goes once the document
   is there (a refused or cancelled New leaves it up).

## Consequences

* Drops, recent files and the start screen work headless and from scripts; the shell only decides
  where the pointer was.
* On the desktop the drop target depends on the system's first pointer move after a drag. Where it
  doesn't come, media land in the middle of the view: never lost, but not where they were dropped.
* On the web a recent file can be reopened while the page keeps its bytes; after a reload it shows
  as missing (the browser never gives a path back), its picture still there.
* The start screen at every launch is the default; artists who want the sample or their last
  document straight away switch it off on the screen.

## Not yet

* Exact desktop drop positions (winit 0.31; eframe 0.36 and 0.37 are on winit 0.30, ADR 0065), and
  dropping folders of images as a sequence.
* ~~SVG into the Library, dropping onto the timeline, templates on the start screen~~: done in
  ADR 0065.
* Opening several dropped documents (needs document tabs), and reopening the last document at start.
* Thumbnails that follow the poster frame.
