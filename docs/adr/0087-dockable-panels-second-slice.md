# ADR 0087: Dockable panels, second slice

* Status: accepted (2026-11-13)
* Relates to: roadmap M8, ADR 0075 (dockable panels and workspaces, whose "Not yet" this
  works through), ADR 0005 (UI guidelines), ADR 0022 and ADR 0064 (accessibility), ADR 0040
  (localisation), ADR 0062 (windows of their own and their fallback), ADR 0004 (destructive
  actions)

## Context

ADR 0075 left out: moving the tools column, dragging a floating window back into a dock,
several panels as tabs in one floating window, collapsing a dock to icons, and renaming and
deleting workspaces. Separately, at the right dock's narrowest the Properties rows clipped
("Snap to objects", "1280 px", the Align buttons and "To stage" ran off the edge). Animate's
behaviour was learned from Adobe's public help pages (Workspace, Panels, Workspaces) and from
observation; no code or assets were used.

## Decision

1. **The tools are a panel with a bar of their own.** `Panel::Tools` joins the panels (the
   Window menu, closing and reopening, the panel menu). Where they usually live is a **slim
   bar** outside the docks, `Layout::tools_bar: Option<Side>`, drawn first so it keeps the
   old full-height column on the left (Essentials looks as before, with a small grip at the
   top). Above or below the stage it is one row. Dropped on a group or window it is a tab like
   any other, its buttons laid out in as many columns as fit (`panels::ToolFlow::Grid`). A
   dragged tools panel dropped near an edge of the stage becomes the bar on that side
   (`Target::Bar`); other panels treat `Bar` as a group of their own there.
   *Why not a group in the left dock?* The old column sits outside the docks and is not
   resizable; making it a group would have changed Essentials (a band header, a 200 pt dock)
   and every saved layout. **Older preference files** have no `tools_bar`: the field defaults
   to the left, so they open unchanged; an explicit `null` means the tools are elsewhere or
   closed. `Home::bar` remembers the bar so the tools reopen there.
2. **Floating windows hold tabs.** `Float` has `tabs`, `active` and an `id` naming its window
   while its tabs change (its viewport id and the drop targets use it). It still writes the
   first tab as `panel`, so a version before this one opens such a window with that panel;
   it reads files with only `panel`. `tidy` gives every window a unique non-zero id. `Home`
   gains `beside` (a panel it floated with), so a closed tab reopens in its window.
   `Target::Stack` accepts a floating panel; stacking onto a tab already in the same group or
   window only brings it forward (it never reorders). `move_window` moves a whole window:
   stacked into a group or window, a group of its own in a dock, or just moved.
3. **Dragging a window back.** A floating window has a strip: a grip, its tabs, Dock and the
   shown panel's menu. Dragging a tab moves that panel; dragging the grip, or, inside the
   editor, the egui window's title bar (`__title_click`), moves the whole window, with the
   same drop targets as a tab, drawn over the editor. A window of its own holds the mouse
   while its button is down (Windows, macOS, X11 and Wayland all capture), so it reports the
   pointer, converted to the editor's points through both windows' desktop rectangles
   (`DockView::remote`), and the editor draws and applies the drop. Windows of their own are
   also drop targets (by their desktop rectangle). Dragging a window by the system's title bar
   only moves it: that drag belongs to the window manager.
4. **Docks collapsed to icons.** `Zone::collapsed` (default false). A collapsed dock is a strip
   (`STRIP`, a tool button wide or high) with an expand button and one icon per panel, groups
   parted by a rule, names as tooltips and accessible names. A click opens a **flyout**: the
   panel in a foreground area beside the strip, as wide (or high) as the dock was, with a
   header (name, menu, close). Esc, or a press outside the flyout and its strip that isn't in
   a popup it opened, closes it, focus going back to its icon. The flyout is view state
   (`DockView::flyout`), never saved. The » button on a side dock's first header and the
   panel menu collapse a dock; the Window menu opens a collapsed panel as its flyout, and is
   ticked while it is open (`App::panel_visible`). Ctrl+F6 lands on the strip's first icon,
   or on the open flyout. New panel icons are AniCraft originals on Lucide's grid.
5. **Workspaces renamed and deleted.** `Workspaces::rename` and `delete` work only on your own
   (the built-in ones can be reset but not renamed or deleted; deleting the one in use goes to
   Essentials as last arranged). Names follow `save_as`'s rules, plus no clash with another
   of yours. **Window › Workspace › Manage Workspaces…** lists every workspace with Rename,
   Delete and Reset (Reset also for one not in use: its changes forgotten). Deleting can't be
   undone (preferences have no undo), so per ADR 0004 it asks, with a second click on the
   same button rather than a dialog.
6. **Panels at narrow widths.** The side docks' minimum is 200 pt (was 160, at which a label
   column left about 50 pt for values). Three causes of clipping, three fixes:
   * `theme::form_row` remembers, per row, how wide its controls were beside the label; when
     that doesn't fit, the value goes **under the label**, in a wrapping row. A change of
     choice redraws the pass at once (`request_discard`), so it never shows a frame late.
     Labels wrap inside the label column.
   * Fixed-width fields and lists take `theme::fit(ui, w)`: `w`, or the room left on the row
     (never under `MIN_FIELD`, 56 pt). In a wrapping row egui's `available_width` is a whole
     row, so `fit` uses the room before the wrap.
   * Button rows (Align, Colour, Swatches, Library, Actions) wrap. Each panel's scroll
     content is held to the panel's width: egui grows a vertical scroll area to its widest
     row, so one overflowing row used to widen every row after it.
   The timeline's layer column gives up width (to 150 pt) to keep a strip of frames; long
   section titles end in "…". The **overflow check** (`dock_narrow_tests`) puts every panel
   alone in the right dock at 200 pt, in five states (document, a shape selected, the
   Rectangle, Text and Brush tools), in English and the pseudo-locale, and fails on any label
   or control in the accessibility tree whose bounds run past the panel's edge.
7. **Still view state.** As in ADR 0075, none of this is a command or in a document.

## Consequences

* Essentials draws as before apart from the tools' grip (the tools start 8 pt lower) and the
  » button on the Properties header.
* Fixed while testing: a drag-only handle laid over a tab took its clicks (egui gives a press
  to the topmost widget sensing it), so clicking a tab above or below the stage could start a
  drag; tabs and icons now sense click and drag on one widget. A floating window inside the
  editor grew a little every frame (its outer size fed back as its content's minimum).
* Tests: the model (tools bar, tabbed windows, move/close/dock back of windows, files from
  other versions with `panel` only and no `tools_bar`, rename and delete), the shell (moving
  the tools by their grip and menu, a tab dropped on a floating window, a window dragged by
  its title bar into a dock, collapsing, flyouts opened by click and the Window menu and
  closed by Esc and a click outside, a window's size stable), the narrow-width check, and the
  accessibility audit with an icon dock and flyout, a tabbed window, the tools moved, the
  narrow dock and Manage Workspaces.
* Screenshots: `--open dockicons`, `floattabs`, `docknarrow` (with `--lang pseudo`) and
  `workspaces`.

## Not yet

* Dragging between windows of their own verified by hand on macOS and Linux and on screens
  with different scales (the conversion assumes one scale).
* Panels side by side inside one floating window; several columns of groups in one dock;
  minimum sizes per panel; Animate's panel shortcut keys.
