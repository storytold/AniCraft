# ADR 0075: Dockable panels and workspaces

* Status: accepted (2026-11-12)
* Relates to: roadmap M8, issue #249, ADR 0005 (UI guidelines), ADR 0022 and ADR 0064
  (accessibility, Ctrl+F6), ADR 0040 (localisation), ADR 0058 (Movie Explorer tab), ADR 0062
  (Test Movie's window of its own and its fallback)

## Context

The shell had a fixed layout: tools on the left, Properties, Align, Library/Movie Explorer and
History as bands in the right column, the timeline (with the Motion Editor as a second tab) at
the bottom, Actions as a floating egui window. Artists coming from Animate rearrange panels
constantly; the first thing the reporter of #249 does in Animate is dock the timeline above the
stage. We need panels that dock on any side, stack as tabs, resize, float in windows of their
own, close and reopen, and named workspaces, kept between sessions, without losing what the
fixed layout guaranteed: the default look (`--screenshot`), Ctrl+F6 focus cycling, the
accessibility audit, high contrast, translation, and the timeline header fitting at 1600 pt.

Animate's behaviour was learned from Adobe's public help pages (Workspace, Panels, Workspaces)
and from observation; no code or assets were used.

## Decision

1. **Our own small dock, not a crate.** `egui_dock` 0.21 (MIT, matches egui 0.36) was the
   obvious candidate and was evaluated. It was not taken because:
   * its tabs, close buttons and splitters carry no AccessKit role or name, so the audit that
     every interactive node is named (ADR 0022) would fail, and naming its widgets from outside
     means depending on its private ids;
   * it floats tabs only as egui windows inside the main window; windows of their own (the
     brief, and Animate) would still have to be written on top of it;
   * its tree model allows any nesting, which makes "reset to Essentials", reopening a closed
     panel where it was, and a predictable Ctrl+F6 order harder to define, while Animate's
     own model is simpler: docks around the document, each a list of tab groups;
   * it brings three more crates (`duplicate`, `paste`, `thiserror`), one unmaintained.

   The own implementation is about 1,300 lines with its tests, split as the code base is:
   `src/dock.rs` is the pure model (no drawing, tested headless), `src/dock_ui.rs` draws it.
2. **The model.** A `Layout` has four `Zone`s (left, right, top, bottom). A zone has a size
   (`None`: from the window, the old formula) and `Group`s: stacked top to bottom in the side
   docks, side by side above and below the stage. A group is tabs (`Panel`s), the shown tab, a
   `share` of its dock (only ratios count) and `folded` (side docks). Floating panels have a
   place and size. `homes` remembers, per panel, where it was when it left a dock: its side, a
   panel it was stacked with or next to, and whether it was floating when closed. Moves are
   expressed against panels, not indices (`Target::Stack(with)`, `Target::Split { side,
   before }`, `Target::Float(at)`), so a move stays right while the groups shift under it.
   Every panel is in at most one place; `tidy` holds anything read from a file to that and to
   finite, bounded sizes. The ten panels: Properties, Align, Colour, Library, Movie Explorer,
   History, Timeline, Motion Editor, Actions, Output. Colour (stroke and fill, Swap, Black and
   White, applied to the selection as one undo step) and Output (the running Test Movie's
   lines, kept after it closes) are new, small panels; the Movie Explorer and Motion Editor
   stop being special tabs and become panels. The tools column stays fixed (see Not yet).
3. **Drawing.** Each non-empty zone is an `egui::Panel` (resizable; its id carries an epoch
   bumped when a workspace is switched or reset, so the new sizes apply), drawn in the old
   nesting order (tools, left, right, bottom, top, then the stage), so Essentials draws exactly
   as before. Groups in a side dock get the old band header (accent edge, fold chevron) with
   the tabs on it; groups above and below the stage get the old timeline header row, with the
   timeline's own transport and buttons beside the tabs, as before. Each header ends with a
   panel menu (☰; also on right-click of a tab): dock on any side, float, fold, close, close
   the tab group, so everything a drag does can be done from the keyboard. Lines between
   groups are draggable splitters, named for screen readers.
4. **Dragging a tab** moves its panel. While dragging, the target is shown over the editor:
   a group's header stacks it there; the first or second half of a group's body makes a group
   of its own before or after it; near an edge of the stage (22 %) docks it on that side
   (this is how the timeline goes above the stage); anywhere else floats it there. Esc
   cancels. The drop targets are the rectangles the groups were drawn in this frame.
5. **Floating** uses ADR 0062's host choice: a window of its own (an egui immediate viewport,
   drawn in the editor's frame with the editor's state, no threads), or, where the platform
   says there can't be one (the web build, embedded viewports), an egui window inside the
   editor. Both have **Dock** (back to its home) and the panel's contents. In a window of its
   own the app's Ctrl/Alt shortcuts are forwarded (as Test Movie does), unless a text field
   has the keyboard. Its desktop place and size are recorded while it is open and written to
   the preferences when AniCraft quits (never fed back into the viewport builder while open,
   which would fight the window manager).
6. **Workspaces** live in the preferences (`workspaces`): the current key, each workspace's
   live arrangement and the user's saved ones. Built in: **Essentials** (the old layout, with
   window-derived sizes), **Animator** (a tall timeline with Output and History beside it, a
   slim right dock) and **Designer** (a wide Properties with Colour and Align, a short
   timeline). As in Animate, changes to a workspace are kept until **Reset**, which goes back
   to the built-in layout or to the one saved. **Save Workspace…** names the current
   arrangement (not a built-in name; at most 50, 64 characters); saving under an existing
   name of yours replaces it. Every change is kept at once and written to the preferences
   when no pointer button is down (not in screenshot runs). Reading is lenient: an unknown
   panel (a newer version's), a missing or junk field drops only that entry, never the rest
   of the preferences; a missing current workspace falls back to Essentials.
7. **Keyboard focus (Ctrl+F6).** `a11y::Region` is now `Stage`, `Tools` or `Panel(p)`, and
   the cycle order is set each frame from the layout: the stage, the top, bottom, right and
   left docks' panels in order, floating panels drawn inside the editor, the tools. A panel
   whose tab is behind another, or folded, has no anchor and is skipped, as before. The shown
   tab is each panel's anchor, unless the panel registers its own (the timeline's frame grid,
   a surface). Panels in a window of their own are left out: that window has its own focus.
   Essentials therefore has seven stops (Align and History are their own panels now) instead
   of five.
8. **Not a command.** Where panels are is view state, like zoom and onion skin (golden rule 2
   keeps view state in the shell); it is never in a document, the CLI or MCP.

## Consequences

* The default screenshot is the old layout; History is now always visible at the bottom of
  the right dock (before, Properties and Library could push it out of view).
* The Motion Editor no longer shows the timeline's transport above it (it is its own panel,
  as in Animate). The Library band lost its inner Library / Movie Explorer pill tabs: the
  group's tabs replace them.
* Tests: the model (essentials, every panel reachable in every workspace, reopen where it was,
  move/stack/split/float/dock back, preferences round trip, files from other versions,
  workspaces keep changes until reset), the shell (the default layout, dragging the timeline
  tab above the stage and onto the Library's header, a drop away from the docks floats and Esc
  cancels, floating inside the editor without viewports and in a viewport with them, every
  panel opened from the Window menu draws, workspace switch with its sizes, reset and save),
  the accessibility audit with every panel shown, floating, in a window of its own, in Animator,
  Designer and with the timeline above the stage, and the Ctrl+F6 order.

## Not yet

* Moving the tools column; dragging a floating window's tab back into a dock (use **Dock** or
  the panel menu); several panels as tabs in one floating window; deleting and renaming saved
  workspaces; collapsing a dock to icons; minimum sizes per panel; Ctrl+L-style panel keys.
