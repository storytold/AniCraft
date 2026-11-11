# Accessibility

AniCraft aims to be usable with the keyboard alone, with a screen reader, at large interface
sizes and in high contrast. This page says what works today, how to check it, and what doesn't
work yet. Design decisions are in ADR 0022; the rules for contributors are in
`docs/ui-guidelines.md`.

## What is supported

### Keyboard

* **Every command has a keyboard route**: its shortcut (F1 lists them all and lets you change
  them) or the command palette (Ctrl+K, ⌘K on macOS), which finds every menu command, tool and
  view setting by name.
* **Move between panels with Ctrl+F6** (⌘F6 on macOS); **Ctrl+Shift+F6** goes back. The order
  is Stage → Timeline → Properties → Library → Tools. The status bar says where you landed, and
  a screen reader reads the panel's name. Both keys can be changed in the shortcut sheet
  (View › Next Panel, Previous Panel). F6 alone is still Insert Keyframe.
* **Inside a panel, Tab and Shift+Tab** move between controls in reading order; Enter or Space
  presses the focused button, ticks the focused checkbox or opens the focused menu; the arrow
  keys change a focused slider or number field.
* **Focus is always visible**: a 2 px accent ring (yellow in high contrast) around the focused
  control, and around the whole stage when it has focus.
* **Who gets the keys:** with nothing focused, or with the stage or timeline focused, every
  shortcut works, including Enter (play) and the arrows (nudge). With a button or checkbox
  focused, the other shortcuts still work but Enter, Space and the arrows belong to it. In a
  text field every key types. Esc leaves a field.
* **Library items without dragging** (ADR 0053): after clicking a Library row, **Enter** places
  the selected items at the centre of the view on the active layer (sounds on its keyframe at
  the playhead), as dragging them onto the stage does; F2 renames and Delete deletes. Place on
  Stage is also a button under the list and in each row's right-click menu.
* Frames are reached with `,` and `.` (previous and next), Home and End, F5/F6/F7 to insert,
  and Shift+`,`/`.`; layers with the layer rows (Tab from the timeline anchor) and the
  Up/Down buttons under them.

### Screen readers

AniCraft publishes its interface through [AccessKit](https://accesskit.dev) (built into eframe):
UI Automation on Windows, NSAccessibility on macOS, AT-SPI on Linux.

* Every button, toggle, field, slider, menu and list row has a name and a role. Icon-only
  buttons are named with their shortcut, as their tooltip is: "Pen tool (P)", "Next frame (.)".
  Toggles say whether they are on (phrasing varies by reader): "Onion skin, toggle button,
  pressed"; "Lock Hills, checkbox, not checked".
* Number fields say what they are and what they hold: "Position X, spin button, 120".
* The stage is announced as a canvas with the tool in hand, the frame and the selection:
  "Stage · Selection tool · frame 12 · 1 element selected".
* The status bar is a live region: after an action a screen reader reads its result ("Deleted
  layer — Ctrl+Z to undo") without moving focus.
* A test (`a11y::tests`) runs the whole interface without a window in 23 states and fails if
  any interactive control has no name or no role.

### High contrast

Preferences › Appearance › Theme › **High contrast**: white text on black, yellow for focus,
selection and the active tool, a visible outline on every button and field, brighter timeline
blocks with black marks. Every text colour reads at 7:1 or more on its background (WCAG AAA);
controls, focus rings and timeline blocks at 3:1 or more. The default Graphite theme meets WCAG
AA (4.5:1 for text, 3:1 for controls). Both are checked by `guidelines_tests`. The artwork on
the stage keeps its own colours.

### Interface scale

Preferences › Appearance › **Interface scale**, 75–200 %, on top of the system's display
scaling. The change is live. At large scales the stage refits, the tools column scrolls, the
timeline header scrolls sideways (Shift+wheel) and the status bar shortens its message rather
than overlapping.

## How to test

Run from a checkout:

```sh
cargo run                                              # as you use it
cargo run -- --theme high-contrast --scale 200         # high contrast at 200 %, not saved
cargo run -- --theme high-contrast --open prefs --screenshot shots/hc.png
cargo test --bin anicraft a11y                         # the AccessKit audit and panel focus
cargo test --bin anicraft palette                      # the contrast checks
```

### Windows: Narrator (or NVDA)

1. Start AniCraft, then Narrator with **Win+Ctrl+Enter**.
2. Press **Ctrl+F6**: Narrator reads the stage, for example "Stage · Selection tool · frame 1,
   canvas". Press it again for the timeline (the active layer, "Ball · layer"), Properties and
   Library (their section headers) and the tools ("Selection tool (V)"). Exact phrasing differs
   between readers.
3. **Tab** through the tools and Properties: every control is read with a name, number fields
   with their value ("Stage width, 1280 px").
4. Press **Delete** with something selected: Narrator reads the status message.
5. Accessibility Insights for Windows (free, from Microsoft) shows the UI Automation tree if you
   want to inspect names and roles directly.

### macOS: VoiceOver

1. Start AniCraft, then VoiceOver with **⌘F5**.
2. Use **⌘F6** to move between panels and **Tab** within them. VO+arrow keys also walk the
   window's elements.
3. The Accessibility Inspector (Xcode › Open Developer Tool) shows names, roles and values.

### Linux: Orca

1. Orca needs the AT-SPI bus (on by default in GNOME and KDE). Start it with **Super+Alt+S** in
   GNOME, or `orca` from a terminal.
2. Start AniCraft from the same session and use **Ctrl+F6** and **Tab** as above.
3. Accerciser shows the AT-SPI tree.

## Known gaps

* **Elements on the stage are not exposed one by one.** A screen reader hears the stage, the
  tool and how many elements are selected, not each shape. Select with the keyboard
  (Ctrl+A, Tab through Properties) and edit values in Properties.
* **Single frames are not exposed.** Each layer's frames are one clickable area; the keyboard
  route is `,` `.` Home End and F5/F6/F7 at the playhead.
* **The Motion Editor graph is pointer only.** Its values are editable in the property list
  beside it, and so are the eases (the Ease menu).
* **Adding a point in Edit Envelope takes a click.** Each point is named for screen readers
  ("Point 2 of 4, Left channel: 80 % at 0.400 s"), and the selected point's time and levels
  are fields that take typed values; the Effect menu sets any preset from the keyboard.
* **The system's high-contrast or "increase contrast" setting is not followed automatically.**
  Choose High contrast in Preferences.
* **The menu bar can't be opened from the keyboard** (no Alt or F10 yet). Every menu command is
  in the command palette (Ctrl+K) and most have a shortcut.
* **Tab order across panels** follows how the window is built (menu bar, edit bar, tools,
  right-hand panels, timeline, stage). Ctrl+F6 gives the panel order listed above.
* **Scroll bars inside lists** have no name (egui doesn't offer one).
* **Slider tracks** are dim in high contrast; their handles and number fields are outlined.
* **Names follow the interface language** (ADR 0040): the audit runs the window in the
  pseudo-locale too and fails on a name that stayed English. Document text (layer and symbol
  names) is announced as written.
* **Manual passes with Narrator, VoiceOver and Orca** are still to do on real machines for each
  platform. The automated audit checks names and roles, not how each reader phrases them.
