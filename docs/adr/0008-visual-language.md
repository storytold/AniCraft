# ADR 0008: Visual language — graphite, geometric, colour where it means something

* Status: accepted (2026-10-29), revised the same day (see "Revision 1")
* Extends: ADR 0005 (typography and fonts), `docs/ui-guidelines.md`
* Extended by: ADR 0077 (a light palette over the same roles and rules)

## Context

The project lead asked for a more modern look: clean with a brutalist edge, gray and deep-gray
palettes, vibrant colour for things like timeline blocks, square boxes and buttons, borders where
they separate layout (not on every button), and accessible controls with click targets that are
not too small. Functionality must not change.

The first shell used rounded corners, outlined buttons, a blue-gray palette, all-caps panel
headings and 22 px controls. It read as a generic dark toolkit theme, and several targets (layer
toggles 18 px, timeline icons 26 × 22, small buttons) were below WCAG 2.5.8's 24 px minimum.

## Decision

1. **Graphite neutrals** in five steps (`chrome` → `panel` → `panel_alt` → `header` → `hover`),
   with inset `field`s darker than the panel. The artwork stays the most colourful thing on the
   stage.
2. **Vibrant colour is spent in one place, the timeline, and always carries meaning**: mint
   `span` = frames with artwork, violet `tween` = classic tween, graphite `span_empty` = empty,
   red `playhead`. Marks on blocks (keyframe dots, tween arrows, labels) are `ink`, so they keep
   ≥ 4.5:1 contrast. Cobalt `accent` is selection, focus and the active tool, and nothing else.
3. ~~Square everywhere, with hard offset shadows.~~ Replaced by revision 1: corners by role.
4. **Borders separate layout, not widgets**: panel edges, section headers, the timeline's
   layer list and grid, popups. Buttons are flat fills; hover lightens them; pressed and
   keyboard-focused widgets get a 2 px accent ring (egui styles focus as "active"). Checkboxes
   and radio buttons are the exception: they keep a `control_edge` so the off state is visible.
5. **Targets**: `TARGET` = 28 px minimum, tools 40 × 40, timeline rows 28, layer toggles
   24 × 24. Timeline frame cells (12 px wide) are a dense-data exception; the ruler, rows and
   keyboard offer larger ways to reach the same frames.
6. **Sentence-case headings** in bold, with a short accent bar for dock sections, instead of
   all-caps labels.

## Revision 1 (2026-10-29): selective geometry, sections, Lucide icons

Feedback: squaring off everything is as indiscriminate as rounding everything. The geometric
look should be selective, as in well-designed audio software: rectangular controls and round
dials, with softer corners where a surface floats. Also: sections in Preferences only differed
from their content by indentation; they need a rule and should fold. And icons should come from
a mature open set.

* **Corners by role** (`theme.rs` › `SQUARE`, `radius_control`, `radius_menu`,
  `radius_window`): structure square (panels, dock bands, timeline grid and blocks, selections,
  stage); controls 3 px (buttons, fields, tools, list highlights, swatches, playhead flag);
  menus and tooltips 6 px; dialogs and windows 10 px. Round only where the thing is round
  (keyframe dots, dials).
* **Depth instead of hard shadows** for floating surfaces: a tight, downward, soft shadow.
* **Collapsible sections** (`theme::section`): `Band` for dock sections, `Rule` (title plus a
  horizontal rule across the row) for groups in Preferences, Properties and the shortcut sheet.
  Whole-row target, keyboard focusable, exposed to assistive technology as a collapsing header.
* **Lucide icons** (ISC). Rendered by parsing the SVGs with a new full SVG path-data reader in
  `geom` (`geom::svg`, all commands including elliptical arcs, from the W3C SVG spec) and
  stroking them as vectors. Rejected: an SVG rasterizer crate (resvg and friends add a large
  dependency tree to a build we keep small), and an icon font (glyph-based, no mirroring,
  harder to keep only the icons we use). The SVG reader is also the basis for SVG import (M4).

All of it lives in `src/theme.rs` and `src/icons.rs`. `guidelines_tests` fails the build on raw colours or radii
outside the theme, and checks the palette's contrast ratios and the target minimum.

## Alternatives considered

* **Keep the rounded, outlined look and only recolour.** Rejected: the brief asks for square,
  border-light; and outlines on every button add noise without adding information.
* **A single neon accent on near-black.** Rejected: it is a cliché of current dark UIs, and one
  accent cannot tell artwork, tween and selection apart on the timeline.
* **Layer colours as span colours.** Rejected for now: span colour already means state
  (artwork / tween / empty), which Animate users read at a glance; layer identity stays in the
  row's edge bar.

## Consequences

* The timeline shows fewer rows at the same height (28 px rows); the default timeline height
  grew to keep five layers visible.
* Any new colour or radius needs a token, and must pass the contrast test where it holds text.
