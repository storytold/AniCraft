# Accessibility

AniCraft aims to be usable with the keyboard alone, with a screen reader, at large interface
sizes and in high contrast. This page says what works today, how to check it, and what doesn't
work yet. Design decisions are in ADR 0022 (first pass) and ADR 0064 (stage elements, timeline
frames, the keyboard menu bar, the system's contrast setting); the rules for contributors are in
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
* **Select on the stage without a pointer** (ADR 0064). With the stage focused (Ctrl+F6),
  press **Tab**: focus goes to the first selected element, or the first one, and a ring shows
  it on the artwork.
  * **Tab**, **Shift+Tab** and the **arrow keys** move between elements, layers top to bottom
    and each front to back, wrapping.
  * **Enter** or **Space** selects the focused element alone. **Shift+Enter** or
    **Shift+Space** adds it to the selection or takes it out, as Shift-click does.
  * The other shortcuts keep working: select with Space, then Delete, F8 (Convert to Symbol)
    or Ctrl+D.
  * **Esc** goes back to the stage itself, where the arrows nudge the selection again.
  * Elements on locked layers are listed, but can't be selected.
* **The timeline from the keyboard.** With the timeline focused, **Left** and **Right** move
  the playhead a frame, and **Up** and **Down** move to the layer above or below.
  * Each move is announced: "Frame 12 · Ball · Frames 1–24: keyframe · motion tween".
  * `,` `.` Home and End go to frames as always, F5/F6/F7 insert, and Shift+`,`/`.` extend.
* **The menu bar from the keyboard** (Windows and Linux, ADR 0064). Press **Alt** alone (press
  and let go) or **F10**: File gets focus, and each menu title shows its letter underlined.
  * **Left** and **Right** move between menus. **Down**, **Enter** or **Space** opens one.
  * **Up** and **Down** move through its items, wrapping.
  * **Right** goes into a submenu (or on to the next menu), and **Left** comes back.
  * **Enter** runs an item, and the keyboard returns to where it was.
  * **Esc** closes the menu; press it again (or Alt, or F10) to leave the bar.
  * **Alt+letter** opens a menu directly: Alt+F File, Alt+E Edit, Alt+V View, Alt+I Insert,
    Alt+M Modify, Alt+C Commands, Alt+O Control, Alt+W Window, Alt+H Help. An Alt+letter you
    have bound in the shortcut sheet keeps your binding.
  * On macOS the menus stay as they are: ⌘K (the command palette) reaches every menu command.

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
* **While the stage has focus, its elements are a list** under it: "5 elements on frame 1".
  * Each element is an option with a name: the instance name first, then what it is, such as
    "Rectangle", "Oval", "Polygon", "Line", "Bitmap", "Text: Hello" or "ball · Graphic: Ball".
  * Its description gives the layer, place and size ("Layer Hills · x 120, y 80 · 200 × 40
    px").
  * It says whether it is selected, and its position in the list. Its bounds are where it is
    on screen.
  * A reader's "click" (activate) on an option selects it.
  * A frame with more than 2,000 elements lists the first 2,000, and says how many more there
    are.
* **The active layer's frames are a list** under its frame grid, one item per keyframe span:
  "Frames 1–24: keyframe · motion tween · label “walk” · sound · actions".
  * The span under the playhead is marked selected.
  * Activating a span moves the playhead to it.
* The status bar is a live region: after an action a screen reader reads its result ("Deleted
  layer — Ctrl+Z to undo") without moving focus.
* A test (`a11y::tests`) runs the whole interface without a window and fails if any
  interactive control has no name or no role. It covers 23 states, plus the stage list with an
  element focused, the timeline's span list and an open menu. Other tests drive the keyboard:
  stage selection, the timeline arrows, and the menu bar.

### High contrast

Preferences › Appearance › Theme › **Follow system** is the default: AniCraft switches to high
contrast while the system asks for more contrast, and back when it doesn't, within a few
seconds. It reads:

* **Windows:** a contrast theme (Settings › Accessibility › Contrast themes, or Left Alt +
  Left Shift + Print Screen).
* **macOS:** Increase Contrast (System Settings › Accessibility › Display).
* **Linux:** the desktop's contrast setting, through the XDG desktop portal, or GNOME's High
  Contrast where there is no portal.
* **The web:** the browser's `prefers-contrast: more`.

The note under the choice says what the system says now. Choosing **Graphite** or **High
contrast** keeps that palette whatever the system says.

Theme › **High contrast**: white text on black, yellow for focus,
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
cargo test --bin anicraft a11y                         # the AccessKit audit, panel focus, stage, timeline and menu keys
cargo run -- --open stagekeys --screenshot shots/s.png # an element focused on the stage
cargo run -- --open menukeys --screenshot shots/m.png  # the menu bar from the keyboard
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
5. On the stage, press **Tab**: Narrator reads the element ("star · Graphic: Star, not
   selected, 2 of 5"). Use the arrows to move, and **Space** to select. Narrator's scan mode
   (Caps Lock+Space) also walks the list.
6. Press **Alt**, then the arrows: Narrator reads each menu and item.
7. Accessibility Insights for Windows (free, from Microsoft) shows the UI Automation tree if you
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

* **Stage elements are named by what they look like.** A shape is a rectangle, oval, polygon
  or line only when its outline is one; a brush or pencil stroke is a "Shape". Give an
  instance a name in Properties to hear it. Moving or transforming the focused element uses
  the nudge keys (after Esc) or the Properties fields.
* **Only the active layer's frames are listed**, as spans (a keyframe and the frames after
  it), not one item per frame. Up and Down on the focused timeline change the active layer.
* **Menus have no per-item letters.** Inside an open menu, use the arrows. While a menu is
  open, single-letter tool shortcuts still work.
* **The Motion Editor graph is pointer only.** Its values are editable in the property list
  beside it, and so are the eases (the Ease menu).
* **Adding a point in Edit Envelope takes a click.** Each point is named for screen readers
  ("Point 2 of 4, Left channel: 80 % at 0.400 s"), and the selected point's time and levels
  are fields that take typed values; the Effect menu sets any preset from the keyboard.
* **The system's light or dark setting is not followed**: AniCraft has one standard (dark)
  palette. Windows' white contrast themes get the black high-contrast palette.
* **macOS has no keyboard route to AniCraft's own menu row**: use ⌘K, which lists every menu
  command.
* **Tab order across panels** follows how the window is built (menu bar, edit bar, tools,
  right-hand panels, timeline, stage). Ctrl+F6 gives the panel order listed above.
* **Scroll bars inside lists** have no name (egui doesn't offer one).
* **Slider tracks** are dim in high contrast; their handles and number fields are outlined.
* **Names follow the interface language** (ADR 0040): the audit runs the window in the
  pseudo-locale too and fails on a name that stayed English. Document text (layer and symbol
  names) is announced as written.
* **Manual passes with Narrator, VoiceOver and Orca** are still to do on real machines for each
  platform. **What has been verified (ADR 0064):** the AccessKit tree AniCraft publishes, by
  automated tests on Windows: names, roles, selected states, bounds, focus, the list box under
  the stage canvas and the span list under the frame grid. Screenshots show the keyboard
  states. No live screen reader has been run against the second pass yet, so how each reader
  phrases the stage options and announces the status bar is still to check.
