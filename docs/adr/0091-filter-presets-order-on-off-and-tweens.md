# ADR 0091: Filter presets, order, on/off and tweened filter values

* Status: accepted (2026-11-13)
* Relates to: roadmap M5 (filters on movie clips, second slice), ADR 0025 (bevel and gradient
  filters), ADR 0032 (GPU filters), ADR 0068 (brush presets kept in the session and the
  preferences), ADR 0072 (filters in the HTML canvas export), ADR 0081 (classic tweens ease
  filters as a group), ADR 0004 (destructive actions), ADR 0009 (XFL import)

## Context

Movie clip and button instances carry a list of filters (ADR 0025), applied in order. Artists
coming from Animate expect more of the Filters section than adding and removing: to reorder
the list (the order changes the picture: a glow before a desaturating Adjust Color is grey,
after it is in colour), to switch a filter off and on without losing its settings, to copy
filters from one instance to another, and to keep favourite lists as **presets** that work in
any document. Classic tweens already interpolate matching lists (ADR 0081); switching a filter
off must not break that. Animate files mark switched-off filters with `enabled="false"`, which
the importer used to drop.

Behaviour was learnt from Adobe's public help pages for the Filters panel (presets replace the
instance's filters when applied; filters tween when both keyframes have the same filters in the
same order) and by observation. No Adobe code or asset was used.

## Decision

1. **The switch is the list's, not the filter's.** `ElementKind::Symbol.filters` is a
   `Vec<FilterEntry>`; `FilterEntry { filter: Filter, enabled: bool }` serializes as the filter's
   own object with `"enabled": false` added (flattened; left out when on). Files from before
   read unchanged, and an older build reads a newer file (it ignores the key; the filter then
   draws). `Filter` itself, which every renderer and exporter consumes, is untouched.
2. **Off equals absent, decided once in `evaluate`.** The display list's group carries only
   the filters that are on (`filter::active`), and an instance whose filters are all off gets
   no group at all. So the CPU oracle, the GPU stage, SVG, Lottie, the plug-in exporters and
   every raster export draw a switched-off filter exactly as if it were not there, with no
   renderer change and so no GPU parity work. The HTML canvas page, which places instances
   from the document itself, applies the same rule when it writes `fx`. Tests compare pixels,
   SVG text and the page's data with and without the filter.
3. **Commands, one undo step each, on every selected movie clip and button.** `filter.move`
   `{from, to}`, `filter.enable {enabled, index?}` (no index: all), `filter.remove {index?}`,
   `filter.copy {index?}` and `filter.paste {replace?}` (added after the instance's own, as in
   Animate; at most 16 filters, refused whole past that), and `element.set_filters` takes
   `enabled` per filter. An index acts where an instance has one; the command fails only when
   it acts nowhere. Graphics are refused with the reason, as before.
4. **Presets belong to the session and live in the preferences**, like brushes (ADR 0068):
   `Session.filter_presets`, put there by the desktop app at start and written back when a
   command changes them, so the engine never reads the settings folder and the CLI and MCP use
   the same commands (`filter.preset_list`, `_save`, `_apply`, `_rename`, `_delete`, `_export`,
   `_import`). A preset is a name (unique ignoring case, at most 64 characters) and a filter
   list, switches included; at most 200. **Applying copies the list** onto the instances,
   replacing theirs (Animate's behaviour): a preset changed or deleted later never changes
   artwork, and documents need nothing extra to open elsewhere. The file is
   `{"format": "anicraft-filter-presets", "version": 1, "presets": [...]}`, at most 1 MB;
   importing skips bad entries, clamps values and numbers clashing names ("Glow 2").
5. **Destructive tiers (ADR 0004).** Deleting a preset, saving over one, and exporting over a
   file are irreversible, so the engine asks (`"confirm": true` headless). Everything that
   touches the document is undoable and doesn't ask.
6. **Tweens.** A classic tween pairs two lists when they have the same kinds in the same
   order, switched-off filters included, so switching one off never breaks a tween; numbers
   and colours travel, and on/off (like Inner, Knockout and Quality) holds from the first
   keyframe. Reordering one keyframe's list makes the lists no longer pair up, and the first
   keyframe's filters hold, as in Animate.
7. **XFL.** `enabled="false"` filters now import switched off, in their place, instead of being
   dropped; the corpus already holds one and its golden render is unchanged.
8. **Clipboard and plug-ins.** Copied filters live in `Session.filter_clipboard`, outside undo,
   and survive New and Open like copied artwork. Plug-ins may move, switch, remove filters and
   list and apply presets; they may not use the clipboard or change the user's presets or
   preset files (`plugin_may_call`).
9. **Interface.** Properties › Filters keeps **Add filter**, and gains a presets button (apply
   by name, Save as Preset… named inline, Rename and Delete submenus, Import… and Export…) and
   a **…** menu (Copy All Filters, Paste Filters, Enable All, Disable All, Remove All). Each
   filter's header has a grip (drag to reorder with a drop line; focused, Alt+Up/Alt+Down;
   right-click for Move Up/Down, Turn Off/On, Copy Filter, Remove Filter), an eye toggle, the
   name (faint and marked "(off)" when off) and a bin. All of it is named for screen readers
   and translatable.

## Consequences

* One model of the list, its order and its switches across commands, files, XFL, the stage and
  every export; renderers see only active filters.
* Not yet (later PRs): **motion-tween tracks per filter property in the Motion Editor**
  (planned as filter properties addressed by slot and parameter beside the twenty transform
  and colour properties, following moves and removals of filters), filters on text fields,
  and built-in presets.
