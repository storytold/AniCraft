# ADR 0077: A light theme, and Follow system for light or dark

* Status: accepted (2026-11-12)
* Relates to: ADR 0008 (visual language), ADR 0022 (high contrast), ADR 0064 (following the
  system's contrast), ADR 0005 (UI guidelines), ADR 0006 (platforms); issue #227

## Context

The shell had two palettes over the same roles in `src/theme.rs`: Graphite (dark, ADR 0008) and
high contrast (white on black, ADR 0022). Some people can't work on dark interfaces at all (#227:
"my eyes can't see a darn thing with dark themes, even with the high contrast one"), and many
work in bright rooms. Every operating system has a light/dark switch, and "Follow system"
(ADR 0064) only followed contrast.

A light theme done badly is an inverted dark one: muddy greys, timeline blocks that glow on white,
shadows that look like smudges, invisible field edges. It has to be designed against the same
rules as Graphite.

## Decisions

1. **A third palette, `LIGHT`, over the same roles.** Every colour still comes from `Tokens`; the
   guideline test (no raw colours outside `theme.rs`) is unchanged, so a light palette applies
   everywhere at once. The look:
   * *Surfaces* are cool paper greys a step apart, lightest where you type: fields white, raised
     surfaces (windows, section bands, headers) `#f8f8fa`, panels `#f0f0f3`, the frame around the
     work (menu and status bars) `#e1e1e6`, buttons `#e3e3e8`. The pasteboard (`#d2d2d9`) is a
     step darker than the panels so a white stage stands off it.
   * *Text* is near-black ink (`#17171b`) at 7:1 and more; dim and faint steps keep 4.5:1 on every
     surface, as in Graphite. Icons follow text colours.
   * *Controls* carry a hairline in the border colour at rest (`control_hairline`): a flat grey on
     a light panel reads as a printed box, not a button. Graphite keeps flat buttons; high
     contrast keeps its 3:1 outlines.
   * *The accent* is a deeper blue (`#2457e0`) for focus, selection and the active tool; the
     active tool is a pale blue block with dark-navy icon (7:1); selected rows `#d3dffb`.
   * *Timeline blocks* keep their meaning and hue (mint artwork, violet classic tween, amber shape
     tween, sky motion tween, red playhead) re-tuned as deeper mid-tones: each sits at 3:1
     against the panels (a component) and keeps its dark marks (keyframe dots, arrows, labels) at
     4.5:1. That window is narrow on light grey (relative luminance about 0.20 to 0.25), so each
     colour was chosen inside it, not lightened from Graphite's.
   * *Depth* is soft: window and menu shadows at about a third of Graphite's opacity, a light
     stage shadow and a 27 % black edge around the stage. The every-fifth-frame tint is a shade,
     not a light.
   * Drag marks on the timeline (a drop outline, a span's new end) were hard-coded white; they now
     use `text`, so they show on light cells too.
2. **`Tokens` gained the parts that differed by palette but were hard-coded**: `dark` (which of
   egui's base styles to start from, so the text cursor, scroll bars and resize grips are right,
   and which theme to give the native title bar through `ctx.set_theme`), `control_hairline`,
   `cell_tint`, `stage_edge`, `window_shadow` and `popup_shadow`.
3. **The contrast test checks all three palettes** with the same rules (text 4.5:1, body text
   7:1, components and focus 3:1, marks on blocks 4.5:1, blocks against panels 3:1, waveforms,
   hint letters, selection on a white stage), plus light-only rules (text on the pasteboard,
   selection on panels, the pasteboard against a white stage) and a structural one: `dark` must
   agree with which way the text goes, and light surfaces step from fields down to the frame.
4. **Preferences › Appearance › Theme** lists Follow system, Dark (Graphite), Light and High
   contrast, applied live on click (as before). The palette is saved as it was (`"theme":
   "Light"`; Graphite keeps its old name, so files from before still open). A palette name this
   build doesn't know reads as Graphite instead of resetting every preference.
5. **Follow system follows light or dark too.** `platform::system_look()` asks, off the UI thread
   and on the same schedule as ADR 0064 (start, every 4 s, when the window comes back), both
   whether more contrast is asked for and whether apps are dark:
   * Windows: `HKCU\…\Themes\Personalize\AppsUseLightTheme` (0 = dark).
   * macOS: `defaults read -g AppleInterfaceStyle` (`Dark` while dark, also in Auto).
   * Linux: the desktop portal's `org.freedesktop.appearance color-scheme` (1 dark, 2 light, 0 no
     preference), else GNOME's `color-scheme` through `gsettings`.
   * Web: `prefers-color-scheme: dark`.

   More contrast wins; otherwise a light system gets Light and a dark one, or one that doesn't
   say, Graphite. **Default (the lead's call, #266):** someone who never chose gets Dark
   (Graphite), still switching to High contrast when the system asks for more contrast (ADR
   0064); creative tools open dark whatever the system's light/dark setting. Follow system
   (light/dark too) is an explicit choice.
6. **`--theme dark|light|high-contrast`** previews a palette for one run (`graphite` still works),
   for screenshots.
7. **Tests get a palette per thread.** The palette in use is process-wide view state; under
   `cfg(test)` it is thread-local, so tests that switch themes run side by side without seeing
   each other's.

## Consequences

* People on a light system who never touched the setting keep the dark editor, as creative
  tools do; choosing Light or Follow system in Preferences is one click (decided on #266).
* A new role must be given a value in three palettes; the contrast test fails until it meets the
  rules in each.
* Not yet: a light high-contrast palette (Windows' white contrast themes still get the black one),
  a user accent colour, following the system's accent colour, and the guide's screenshots in
  light.
