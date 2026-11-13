# UI guidelines

AniCraft's interface serves artists in a flow state (see `product-principles.md`). These rules
keep it professional, calm and predictable. **Tokens live in `src/theme.rs`**: colours in `T`,
typography in `Type`. Panels never hard-code colours, font families or font sizes; a test
(`guidelines_tests` in `src/main.rs`) fails the build if they do.

## 1. Layout

The layout is familiar to animators: **tools** on the left, **stage** in the centre over a darker
pasteboard, **timeline** at the bottom, **Properties / Library / History** docked on the right,
menu bar and edit bar on top, and a status bar at the bottom.

* The most frequent actions are one click away (timeline header buttons, panel buttons) **and**
  one key away (registry shortcuts). Everything is reachable from the command palette (Ctrl+K).
* Right-click offers the actions that apply to what is under the pointer (stage, frames, layer
  names), with the same labels and shortcuts as the menus (`menus::cmd_item`).
* Panels virtualize long content (timeline rows and columns); nothing paints off-screen work.

## 2. Typography: prose vs data

Two families, chosen by **what the text is**, not by where it sits:

| Role (`theme::Type`) | Family | Use for | Examples |
|---|---|---|---|
| `Title` | proportional 17 | Dialog titles | "Delete layer?" |
| `Heading` | proportional 12.5, bold, sentence case | Panel/section names | Properties, Timeline, Library |
| `Body` | proportional 13 | Names, list items, menu items, buttons | layer and symbol names |
| `Label` | proportional 12 | Field labels, status messages | "Position", "Saved…" |
| `Caption` | proportional 11 | Hints, explanations, footnotes | "Enter confirms · Esc cancels" |
| `Data` | **monospace** 12 | Values and readouts | X/Y, scale, rotation, sizes, fps, seconds, zoom %, cursor coordinates, render stats |
| `DataSmall` | **monospace** 10 | Dense numeric scales | timeline ruler, playhead number |
| `DataLarge` | **monospace** 14.5 | The hero number | current frame in the timeline header |
| `Key` | **monospace** 11.5 | Shortcuts and keycaps | Ctrl+Shift+S, F6 |
| `Path` | **monospace** 11 | Paths, ids, preformatted text | file locations, licence texts, frame scripts, class names |

**Why monospace for data:** fixed-width digits don't jitter while a value is scrubbed or
played back, numbers line up in columns, and technical readouts read as instruments rather than
prose. Pro audio/video tools use the same idea. We adopt the idea only; we don't copy any
vendor's typeface.

**Never monospace:** dialog descriptions, explanations, menu and button labels, layer names,
tooltips' prose, error messages and anything a person reads as a sentence. Mixed lines keep
their label proportional and their value monospace.

Typefaces (ADR 0005): **Inter** (prose) and **JetBrains Mono** (data), both SIL OFL 1.1,
embedded from the shared craft-fonts input (`CRAFT_FONTS_DIR`, or discovered next to the repo)
as in PhotoCraft. Users can pick any **installed system font** for the interface and any
installed **monospaced** font for data in Preferences › Appearance. Roles stay the same; only
the family changes. Interface scale 75–200% is a preference too. The fallback
is egui's bundled Ubuntu + Hack, so builds without craft-fonts still work. Fonts must be
permissive in use and small enough to bundle (≤ 2.5 MB embedded in total). Platform system fonts
are not used for UI text, because they would change metrics per OS.

## 3. Colour (ADR 0008)

* **Graphite neutrals**, darkest at the edges: `chrome` (menu and status bars) → `panel` →
  `panel_alt` (section headers, alternate rows, windows) → `header` (buttons at rest) → `hover`.
  Text and number fields are inset: `field`, darker than the panel. The artwork is the most
  colourful thing on the stage.
* **Vibrant colour only where it means something, mostly on the timeline**: mint `span` =
  frames with artwork, violet `tween` = classic tween, graphite `span_empty` = empty frames,
  red `playhead`. Marks drawn on those blocks use `ink`.
* **One accent**, cobalt `accent`, for selection, focus and the active tool. Red only for the
  playhead, errors and destructive buttons (`danger`).
* Text has three steps: `text`, `text_dim`, `text_faint`. Use the role's default colour unless
  you are showing a state.
* **Contrast** (checked by `guidelines_tests` for every palette): body text ≥ 7:1 on panels;
  dim, faint, accent-coloured (`accent_text`), error and ok text ≥ 4.5:1; text on filled
  buttons (`on_fill`) and on the active tool (`on_active`) ≥ 4.5:1; marks on timeline blocks and
  the playhead flag (`ink`) ≥ 4.5:1; focus rings, selection, checkbox edges and timeline blocks
  ≥ 3:1 against their background.
* **Three palettes, one set of roles** (ADR 0022, ADR 0077): Graphite (dark), Light and High
  contrast (Preferences › Appearance › Theme). Code reads `T.role`, never a palette by name, and
  never assumes a colour or a direction: text on an accent fill is `T.on_fill`, not white; an icon
  under the pointer is `T.icon_hot`; a mark drawn across the timeline is `T.text`, not white,
  because the panels may be light. A new token needs a value in every palette and a line in the
  contrast test. Check UI changes with `--theme light` as well.
* No raw colours outside `theme.rs`: add a token.

## 4. Shapes, borders, spacing and sizes (ADR 0008)

* **Corners follow the role of a shape**, never one value for everything:
  * *Structure is square* (`theme::SQUARE`): panels, dock bands, the timeline grid, its blocks
    and selections, the stage. These tile against each other.
  * *Controls are softened* (`T.rc()`, 3 px): buttons, fields, tools, list highlights, swatches,
    the playhead flag. Geometric and rectangular, but not sharp.
  * *Floating surfaces are rounder*: menus and tooltips 6 px, dialogs and windows 10 px, with
    a soft, tight downward shadow so they lift off the work.
  * *Round only where the thing is round*: keyframe dots, dials, onion-skin marks.
* **Sections** use `theme::section`: a full-width header (chevron, bold title, and in panels
  and dialogs a horizontal rule across the rest of the row) that folds its content away. The
  whole row is the click target, it takes keyboard focus, and assistive technology hears
  "expanded/collapsed". Dock sections are `Section::Band`, groups inside a panel or dialog
  `Section::Rule`. Don't separate groups by indentation alone.
* **Borders separate layout, not widgets.** Use `T.border` lines between regions (panels,
  section headers, the timeline's layer list and grid, the timeline footer) and around popups.
  Buttons are flat fills with no resting border; hover lightens the fill. Use `T.separator`
  for fine grid lines inside a region.
* **Focus is always visible**: pressed and keyboard-focused widgets show a 2 px accent ring.
* **Checkboxes and radio buttons** go through `theme::checkbox` / `theme::radio_value`, which
  give them a visible edge (`control_edge`) so the off state does not disappear.
* **Click targets** (WCAG 2.5.8): at least `theme::TARGET` (28 px) for every button, row and
  field; tools `TOOL_BUTTON` (40 × 40); timeline rows 28; layer toggles 24 × 24; dialog
  buttons 32 high. Don't use small buttons. Timeline frame cells (12 px wide) are a
  dense-data exception: the ruler, rows and the keyboard reach the same frames.
* Item spacing 8×6, buttons pad 10×5. Panels pad 10–12 px. Align values in a column using the
  `row` helper.
* Active states are solid: the active tool is an accent block with a white icon; a selected
  list row is `accent_dim` with an accent edge bar; dock sections start with a short accent bar.

## 5. Icons

* **Lucide** (ISC licence, `assets/icons/`, credited in `ATTRIBUTION.md`): one consistent,
  open outline set with icons for everything an editor needs. Use `icons::paint` with an
  `icons::Icon` variant. The SVGs are parsed with the engine's own SVG path reader and stroked
  as vectors at the on-screen size (crisp at any interface scale, no rasterizer dependency).
* To add an icon: copy its SVG from the Lucide set into `assets/icons/`, add a variant and its
  file to `src/icons.rs`. Prefer an existing Lucide icon (mirrored if needed) to drawing one.
  When none fits, draw an original `anicraft-*.svg` on the same 24 grid with 2-unit round
  strokes (the Align panel's icons are such originals).
* Icons are placed on whole device pixels with whole-pixel stroke widths; tool icons are drawn
  at the set's native 24 px, where they are sharpest.
* **Selection is solid, Subselection hollow** (`Icon::solid`): the two arrows must be told apart
  at a glance, as animators expect.
* Forms use `theme::form_row` / `theme::form_note`: labels left-aligned in one column, controls
  in the next, and explanations under the control they explain (under a checkbox or radio
  button, aligned with its text), never back at the label column.
* 18–20 px glyph inside a larger target, the same colour as the text, white on an active
  (accent) block, faint when it shows an "off" state.
* Every icon button has a tooltip with its name and shortcut. No icon fonts, no vendor glyphs.

## 6. Feedback and flow

* Every action answers in the status bar (what happened, and "Ctrl+Z to undo" for deletions).
* Long work never blocks the UI (background render, ADR 0002); show the last good state while
  waiting.
* Drags are one undo step. Values scrubbed in fields are one undo step.
* Destructive actions follow ADR 0004 (tiered confirmation). Dialog buttons name the action
  ("Delete Layer", not "OK"). The destructive button is red; Cancel is always available and
  Esc cancels.

## 7. Keyboard

* Familiar animation shortcuts first (F5/F6/F7, Shift+F5/F6, F8, Ctrl+B, Enter to play, `,` `.`
  to step), then the platform conventions (Ctrl+Z/S/O/N…).
* Where the industry-standard animation defaults and a generic convention disagree, the
  animation default wins and the other becomes an alias (`SHORTCUT_ALIASES`): Ctrl+Y redoes,
  Ctrl+Shift+Z removes a transform, Ctrl+↑/↓ arranges, Shift+`,`/`.` jump to the first/last
  frame, Backspace deletes. A test fails if two commands share a chord.
* Tool keys are single letters; Shift constrains; Space pans; arrows nudge (Shift ×10).
* Scrolling follows the browser convention everywhere: the wheel scrolls vertically, Shift+wheel
  (and sideways wheels or trackpad swipes) horizontally, Ctrl+wheel zooms, over the whole
  panel, not only its empty parts.
* F1 shows every shortcut (generated from the registry); Ctrl+K runs anything.
* Ctrl+F6 / Ctrl+Shift+F6 move focus between the panels (Stage, Timeline, Properties, Library,
  Tools); Tab moves within one. A focused button keeps Enter, Space and the arrows; the other
  shortcuts still work (ADR 0022, `a11y::keyboard`).

## 7a. Accessibility (ADR 0022, `docs/accessibility.md`)

* **Every interactive thing has a name and a role** for screen readers; the AccessKit audit in
  `src/a11y.rs` fails the build otherwise. Widgets with their own text (buttons, checkboxes,
  menu items) name themselves. Others go through `a11y`:
  * a custom-painted control: `a11y::control(ui, &resp, WidgetType::Button, name, on)` (also
    draws its focus ring);
  * a field, colour well or menu showing a value: `.named("Position X")` /
    `.response.set_name("Easing")`;
  * a slider in a form row: `a11y::slider` (it names the number field beside it too).
* **Names say what and how**: an icon-only control is named like its tooltip's first line, with
  the key in effect (`a11y::with_key("Pen tool", key)` → "Pen tool (P)"); repeated controls
  carry their row ("Lock Hills", "Drop Shadow blur X").
* **Pointer-only surfaces are not Tab stops** (`a11y::pointer_only()`); say in the docs what
  the keyboard route to the same thing is.
* **Panels register an anchor** (`a11y::anchor`) for Ctrl+F6.
* Layouts survive the 200 % interface scale: scroll or truncate, never clip controls away.

## 8. Copy and numbers

* Sentence case for labels and titles; Title Case only for menu commands, matching familiar
  tools. Buttons are verbs. No exclamation marks.
* Frames are 1-based in the UI (frame 1 is the first). Time is in seconds with two decimals
  (`0.50 s`). Units follow the number with a space: `24.00 fps`, `1280 × 720 px`, `78%`.
* Use "×" for dimensions and "·" to separate inline facts.

## 8a. Interface languages (ADR 0040, `docs/localisation.md`)

* Write interface text in English through the macros: `tr!("Save")`, `tr!("Saved {path}",
  path = p)`, `trn!("{n} layer", "{n} layers", n)`; `tr_noop!` in tables, `i18n::t` where they
  are drawn. Screen-reader names too.
* One sentence is one template with named placeholders; never glue translated pieces. Counts
  use `trn!`, never `if n == 1`.
* Keys stay English (ids, stored values, engine params); translate at the point of display.
  Translated window titles carry an explicit `.id(..)`.
* Leave room: labels must survive text a third longer. Check with `--lang pseudo`.

## 9. Checklist for a UI change

1. Uses `theme::text(Type::…)` / `theme::font` / `with_type`, with no raw sizes or families,
   and colours, radii and sizes from `theme` (`T`, `TARGET`), with no raw values.
1. Targets ≥ `TARGET`, focus visible, contrast as in section 3, corners by role (section 4),
   borders only for layout, groups as collapsible `theme::section`s, Lucide icons.
2. Data in monospace, prose proportional (section 2).
3. Reachable by menu, palette and, if frequent, a shortcut and a right-click menu.
4. Screenshot checked (`cargo run -- --screenshot shots/now.png`, `--open <panel>` for dialogs),
   also with `--theme high-contrast` and `--scale 200`.
5. Named for screen readers (section 7a); `cargo test --bin anicraft a11y` passes.
6. Text goes through `tr!` (section 8a); `cargo test --bin anicraft i18n` passes and a
   `--lang pseudo` screenshot shows no plain English and no cut-off `]`.
