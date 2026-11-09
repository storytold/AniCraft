# ADR 0022: Accessibility, first pass: named controls, panel focus, high contrast, scale

* Status: accepted (2026-11-09)
* Relates to: requirements › Accessibility, roadmap M8, ADR 0005 (typography), ADR 0006
  (platforms), ADR 0008 (visual language, focus ring); `docs/accessibility.md` is the user guide

## Context

The requirement reads: "Keyboard reachable commands, scalable UI, high-contrast theme,
screen-reader labels via AccessKit." Commands were already keyboard reachable (registry
shortcuts, the command palette, the shortcut editor). The rest was not there:

* eframe already built with its `accesskit` feature, so Narrator, VoiceOver and Orca saw the
  window, but most of the shell is custom-painted (tools, layer rows and toggles, timeline icon
  buttons, Library rows, the stage, the Motion Editor). Those reached assistive technology as
  anonymous nodes with no role. Number fields, colour wells and menus had a value but no name.
* There was no way to move the keyboard between panels, and any focused widget silenced every
  shortcut, so Tab-ing to a button turned off V, P, F6 and the rest.
* `theme::T` was a `const`: the palette could not change at run time.
* The interface scale existed (75–200 %) but at 200 % on a laptop the tools column was cut off,
  the timeline header clipped its buttons, the status message ran under the readouts, and the
  stage stayed at the zoom it had before the scale changed.
* Two default-theme pairs missed WCAG AA: white frame numbers on the red playhead (3.3:1) and the
  accent-blue current-frame number (3.2:1).

## Decisions

1. **Every interactive node has a name and a role, enforced by a test.** `src/a11y.rs` holds
   the helpers: `control` gives a custom-painted control its role, name and on/off state and
   draws its focus ring; `Named::named` names a value-only widget ("Position X", "Fill
   colour"); `slider` pairs a slider with a named number field (egui's own field beside a
   slider is anonymous unless the slider shows a visible text label); `surface` marks the stage
   and the Motion Editor graph as canvases; `name_title_bar` and `name_splitter` name the parts
   egui leaves anonymous. The test `a11y::tests::every_interactive_node_has_a_name_and_a_role`
   runs the whole app on a headless `egui::Context` with AccessKit on, in 23 states (default,
   every tool's options, a selection, a movie clip with all four filters, the Motion Editor, a
   sound keyframe, every dialog, high contrast at 200 %), and fails on any focusable or
   clickable node without
   a name or with an unknown role. egui's scroll-area bars are the only exemption.
2. **Names say what and how.** An icon-only control's name is its tooltip's first line with the
   key in effect (`Pen tool (P)`, `Next frame (.)`), read from the keymap so a rebinding shows.
   Toggles are buttons with a toggled state (`Onion skin`, `Show Ball`). Repeated controls carry
   their row (`Lock Hills`, `Drop Shadow blur X`, `Restore Untitled`).
3. **Ctrl+F6 / Ctrl+Shift+F6 move keyboard focus between panels**: Stage, Timeline,
   Properties, Library, Tools, wrapping. F6 alone stays Insert Keyframe, as animators expect;
   Ctrl+F6 is the next-pane key of Windows applications, and on macOS our Ctrl is ⌘, so ⌘F6
   does not collide with the system's Control-F-key focus shortcuts. Both are view actions in
   the keymap (rebindable, listed in the shortcut sheet under View) and work even from a text
   field. Each panel registers one anchor per frame (the stage canvas, the active layer row or
   the Motion Editor tab, the Properties and Library headers, the active tool); an anchor not
   drawn this frame is skipped.
4. **Who owns the keyboard** (`a11y::keyboard`): nothing focused, or the stage or timeline
   anchor focused, gets every shortcut, Enter (play) and the arrows (nudge); a focused control
   (button, checkbox, menu, section header) keeps Enter, Space and the arrows for itself but
   the other shortcuts still work; a text field keeps every key. Before, any focus disabled all
   shortcuts.
5. **Pointer-only surfaces are not Tab stops.** The frame ruler, each layer's frame grid, the
   timeline scroll bar and the Motion Editor graph sense clicks and drags but are not focusable
   (`a11y::pointer_only`); their keyboard routes are `,` `.` Home End F5 F6 F7, Shift+wheel and
   the Motion Editor's property list. They are still named, so a screen reader can find and
   click them.
6. **The palette is chosen at run time.** `T` is now a zero-sized handle that dereferences to
   the active `Tokens` (an atomic selects `GRAPHITE` or `HIGH_CONTRAST`), so every `T.panel`
   call site stays as it was and a theme change shows in the same frame. `theme::apply_palette`
   restyles egui's own widgets. Rejected: passing the palette through every `Ui` (hundreds of
   signatures for no gain: one window, one look), and egui `Visuals` alone (it does not reach
   our custom painting). Contrast tests check the constant palettes directly, so tests running
   in parallel never depend on the global.
7. **High contrast is built from the same roles**, not a second set of colours in the code:
   black surfaces, white text (every text role ≥ 7:1, AAA), yellow for focus, selection, the
   active tool and accent text, outlined buttons and fields (`control_outline`), brighter
   timeline blocks with black marks, a deep-blue stage selection that shows on white artwork.
   New tokens make the roles explicit where the old code assumed white: `on_active`, `icon_hot`,
   `on_fill`, `accent_text`. `guidelines_tests` checks every pair a palette draws together (text
   4.5:1 or more, UI parts and focus 3:1) for both palettes.
8. **The two AA misses in the graphite theme are fixed** without changing its character: the
   playhead's frame number is drawn in `ink` (5.9:1 on the red flag), accent-coloured text uses
   `accent_text` (a lighter cobalt, 5.7:1), and `text_faint` is a step lighter (4.5:1 on raised
   surfaces too).
9. **Interface scale 75–200 % holds up.** The stage stays fitted when its canvas changes size
   (while nobody has zoomed or panned since the fit), docks start proportional to the window
   and are capped so the stage keeps room, the tools column scrolls, the timeline header
   scrolls sideways instead of clipping, and the status message truncates before the readouts
   (render and cache diagnostics drop out when narrow). The status message is a polite live
   region, so screen readers announce what an action did.

## Consequences

* A new control without a name fails `cargo test`, with its role, position and value in the
  message. New custom-painted controls go through `a11y::control`.
* Single-letter shortcuts now work while a button or checkbox has keyboard focus. Text fields
  are unaffected.
* `anicraft --theme high-contrast --scale 200` previews either for one run (screenshots,
  checks) without saving preferences.
* Not in this slice (`docs/accessibility.md` › Known gaps): elements on the stage and single
  frames are not exposed one by one; the OS high-contrast setting is not followed automatically;
  manual passes with Narrator, VoiceOver and Orca are still to do on each platform.
