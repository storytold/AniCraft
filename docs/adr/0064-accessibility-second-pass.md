# ADR 0064: Accessibility, second pass: stage elements, timeline frames, keyboard menu bar, system contrast

* Status: accepted (2026-11-11)
* Relates to: ADR 0022 (first pass), ADR 0004 (no modal interruptions), ADR 0006 (platforms),
  ADR 0040 (translatable interface), ADR 0044 (selection across layers); `docs/accessibility.md`
  is the user guide

## Context

The first pass (ADR 0022) named every control, added Ctrl+F6 panel focus, a high-contrast
palette and a 200 % scale. Its known gaps were the parts of the window where the work happens,
plus two platform conventions:

* The stage was one canvas node. A screen reader heard "Stage · Selection tool · frame 12 ·
  1 element selected" and nothing about what was on it. Selecting anything took a pointer
  (or Ctrl+A).
* Each layer's frames were one clickable area: a screen reader couldn't hear what was at a
  frame (keyframe, tween, label), and the arrow keys on the focused timeline nudged artwork.
* The menu bar couldn't be reached from the keyboard. On Windows and Linux Alt and F10 do that
  in every application.
* Windows' contrast themes, macOS' Increase Contrast and the GNOME/portal contrast switch were
  ignored. High contrast had to be chosen in Preferences.

egui 0.36 publishes its widgets through AccessKit but has no API for nodes that aren't
widgets, and no keyboard navigation of its menu bar.

## Decisions

1. **Stage elements are a list box under the stage, only while the stage has the keyboard**
   (`src/stage_a11y.rs`). The canvas node gets one child, a multi-select `ListBox` "12 elements
   on frame 4". It has one `ListBoxOption` per element on the current frame: layers top to
   bottom, each front to back (picking order). An option carries:
   * its **name**: the instance name first (document text), then what the element is.
     Shapes are named by their outline: "Rectangle" (axis-aligned four sides, or rounded),
     "Oval" (four curves), "Polygon", "Line", else "Shape". The rest are "Bitmap",
     "Movie clip: Ball", "Text: Hello…" (one line, 80 characters).
   * its **description**: the layer, position and size in pixels.
   * its **bounds** on screen, so the reader's highlight lands on the artwork.
   * **selected**, its place in the set, **disabled** on a locked layer, and a Click action.

   The tree stays the size it was while the stage isn't focused, and the rest of the window
   keeps its tab order. **At most 2,000 options**; beyond that one label says how many more
   there are and how to reach them (picking, Select All, Find). Options are egui widgets with a
   zero-size interaction rect: keyboard focus stops with the right AccessKit focus, but no
   pointer area over the canvas. Their parent is set with `UiBuilder::accessibility_parent`.
   The bounds of all elements come from one pass over the display list, not one per element.
   Rejected: options for every element at all times (thousands of nodes and Tab stops in every
   frame), and patching AccessKit's tree after egui (no API; it would fight egui's ids).

2. **Keyboard selection on the stage.** Tab on the focused stage goes to the first selected
   element, or the first one. From there Tab, Shift+Tab and the arrows move between elements,
   wrapping (a composite widget, like a grid). Enter or Space selects the element alone;
   Shift+Enter or Shift+Space adds it to the selection or takes it out (Shift-click). Esc goes
   back to the stage itself, where the arrows nudge again, and Ctrl+F6 leaves the panel.
   * The focused element has a 2 px accent ring on the artwork, and the status bar (a live
     region) says its name and state.
   * Selection runs `element.select`, the command a click, the CLI and MCP use. It gains
     `toggle?: bool` for Shift+Space. A locked layer's element is listed but refuses, as a
     click does.
   * Other shortcuts keep working on a focused element: select with Space, then Delete,
     F8 or Ctrl+D.

3. **Timeline frames.** The active layer's frame grid gets a `List` of its keyframe spans.
   Each item says "Frames 1–24: keyframe · motion tween · label “walk” · sound · actions"
   (blank keyframe; classic, shape, motion or broken tween). The span under the playhead is
   selected, and a reader's Click moves the playhead there. At most 1,000 items, then a
   summary. On the **focused timeline** Left/Right move the playhead a frame and Up/Down move
   the active layer (keyboard focus follows the row), instead of nudging artwork. Moves made
   while the timeline has the keyboard (arrows, `,` `.` Home End) are announced through the
   status bar: "Frame 12 · Ball · Frames 1–24: keyframe · motion tween". Nothing is announced
   during playback.

4. **The menu bar from the keyboard, Windows and Linux conventions** (`menus::MenuKeys`).
   * **Alt** pressed and let go alone, or **F10**, focuses File; again (or Esc) gives the
     keyboard back to where it was.
   * Left/Right move between menus (and keep a menu open if one was). Down, Enter or Space
     opens one and focuses its first enabled item.
   * In a menu, Up/Down wrap. Right enters an open submenu, or else goes on to the next menu.
     Left comes back out of a submenu, or else goes to the previous menu.
   * Esc closes the menu, back on its title. An item chosen with Enter runs, and the keyboard
     returns to where it was before the bar.
   * **Alt+letter** opens a menu directly, and titles show their letter underlined while Alt
     is held or the bar has the keyboard. The letters are computed from the titles in the
     interface language: the first free letter, word starts first, ASCII only (File, Edit,
     View, Insert, Modify, Commands, c**O**ntrol, Window, Help). They are also each title's
     AccessKit access key.
   * Access keys run **before** the shortcut table because egui matches a plain "V" with Alt
     held. An Alt+letter the user has bound in the shortcut editor keeps its binding.
   * The Enter or Space that pressed an item is used up there, so it can't also press a button
     in what the item opens (the command palette's field).
   * Navigation reads each menu's focusable widgets from egui's widget list for that popup
     layer, sorted top to bottom. egui's own arrow navigation is geometric across the whole
     window and would leave the menu.
   * **macOS is unchanged.** Its conventions differ (Ctrl+F2 reaches the system menu bar, and
     Option isn't a menu key), and ⌘K reaches every menu command. `platform::menu_bar_keys`
     makes the choice, and a browser on a Mac follows the Mac.

5. **Following the system's contrast setting** (`platform::system_high_contrast`), each system
   read without `unsafe`:
   * Windows: the `HighContrast` `Flags` value in the user's registry (`HCF_HIGHCONTRASTON`),
     through `winreg` (MIT, already in the tree through craft-update).
   * macOS: `defaults read com.apple.universalaccess increaseContrast`.
   * Linux and the BSDs: the XDG desktop portal's `org.freedesktop.appearance contrast` through
     `gdbus` (1 s timeout), else GNOME's `high-contrast` through `gsettings`.
   * The web: `matchMedia("(prefers-contrast: more)")`.

   The app asks on a worker thread at start, every 4 s and when the window comes back to the
   front. A changed answer switches the palette in the same frame. Screenshot runs and tests
   never ask, so they look the same on every machine.

   **Preferences › Theme** gains **Follow system**, the default: high contrast while the system
   asks for it, graphite otherwise. Its note says what the system says now. The choice is
   `theme_follows_system: Option<bool>`. `None` (every file from before) follows the system,
   unless the file had chosen high contrast, which keeps it. `--theme` turns following off for
   the run.

   Rejected: a D-Bus crate for Linux (another dependency tree for one value; `zbus` is in the
   tree only async, under AccessKit), and a native API on each system (each needs `unsafe`).

6. **Light themes are not followed.** AniCraft has one standard palette (dark, as Animate's
   default). The system's light/dark setting has nothing to map to until a light palette
   exists. Windows' white contrast themes get our black high-contrast palette.

## Consequences

* The AccessKit audit now also covers the stage list with an element focused, the active
  layer's span list, and an open menu (egui's menu area node is named as a `Menu`, which the
  audit had never seen open).
* New tests drive the window headless:
  * keyboard selection on the stage (Tab, arrows, wrap, Space, Shift+Space, Enter, a locked
    element, Esc);
  * timeline arrows with announcements and layer focus;
  * menu bar navigation (F10, an Alt tap, Alt+letter, arrows, wrap, Esc twice, Enter running
    an item and focus returning);
  * the palette following the system;
  * old preference files.
* `--open stagekeys` and `--open menukeys` replay key presses for screenshots of those states.
* Not in this slice:
  * element names beyond the outline heuristic (a pencil stroke is a "Shape");
  * moving or transforming the focused element with keys other than nudge (Free Transform's
    fields do that);
  * per-item access keys inside menus;
  * a light palette;
  * Windows' white contrast theme as a white palette;
  * live screen-reader passes on real machines (`docs/accessibility.md` says what was
    verified).
