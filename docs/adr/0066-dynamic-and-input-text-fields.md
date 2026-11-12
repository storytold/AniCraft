# ADR 0066: Dynamic and input text fields

* Status: accepted (2026-11-11), first slice built
* Relates to: ADR 0012 (text), ADR 0014 (text from Animate documents), ADR 0013 and ADR 0052
  (ActionScript tiers 2 and 2.5), ADR 0024 (the pure player and Test Movie), ADR 0028 (HTML
  canvas export), ADR 0022 (accessibility), roadmap M5, requirements (static, dynamic and input
  text, P1)

## Context

Animate has three text types. **Static** text is artwork. **Dynamic** text is a field scripts
fill in while the movie plays (a score, a question, a name). **Input** text is a field the
viewer types into (a name, an answer, a password), which scripts then read. AniCraft's text
(ADR 0012) was static only: a block's outlines are laid out by the engine when it is edited and
stored, so rendering never needs fonts. XFL import (ADR 0014) brought dynamic and input fields
in as static text, keeping only the instance name. Scripts (ADR 0013, 0052) could not name text
at all.

Two constraints shape the design. **Time is a function**: the authoring stage and every export
evaluate the document, never a played session, and the player is pure and deterministic.
**Layering**: the player lives in `anicraft-doc` (layer 1), and the fonts in `anicraft-text`,
also layer 1, which `anicraft-doc` may not use.

## Decision

1. **A field is a text block with field settings.** `TextBlock.field: Option<TextField>`
   (`None`: static text, so every older file opens unchanged and saves byte for byte the same;
   `#[serde(default)]` throughout). `TextField` holds the kind (Dynamic, Input), the line type
   (Single line, Multiline, Multiline no wrap, Password; password only for input), Selectable,
   border and background with their colours, the maximum characters a viewer may type, and an
   ActionScript 1/2 variable name kept from Animate documents (not run). The element's name is
   its instance name (`instance.set_name` now takes fields; static text has none).
2. **The block's box is the field's box.** For a field, `box_width × box_height` is the fixed
   box the author drew, not the text's size, and the stored outline is the shown text laid out
   2 px (`FIELD_GUTTER`, Animate's inset) inside it, aligned in the box, wrapped at the box less
   the gutters when multiline, as `*` when a password. So everything that measures, picks,
   transforms, finds or breaks apart text works on fields unchanged, and **the stage and every
   non-interactive export (PNG, GIF, APNG, WebP, SVG, Lottie, video, sprite sheets) draw a
   field with its authored text**, purely and deterministically. `evaluate` adds what a field
   has that static text doesn't: its background, a clip to its box (a mask clip, so every
   renderer already supports it) and a 1 px border.
3. **Fields are instances in the player.** The player's instance tree gets text nodes
   (`NodeKind::Text`) for dynamic and input fields, so instance-name paths reach them
   (`panel.name_txt`), and properties scripts set (`x`, `alpha`, `visible`, …) apply to them as
   to movie clips. `PlayerState.fields` holds, per field instance, the text once a script or the
   viewer changed it (`None`: the timeline's own, so a later keyframe's words still show), its
   laid-out look, caret, selection and scroll; `PlayerState.focus` is the field with the
   keyboard. A field's state leaves with the field, as a clip's playhead does.
4. **Scripts: `field.text`, `field.appendText(…)`, `field.length`.** The script IR gains
   `Prop::Text` and `Prop::Length` (read) and `LValue::Text` (write); `appendText(v)` lowers
   to `text += v`, so it needs no new statement. They are member-only (`score_txt.text`): a bare
   `text` stays a variable, as it is in ActionScript frame scripts. Setting `null` stops the
   script with a message (ActionScript throws); `\r` and `\r\n` are line breaks, as ActionScript
   writes them; text is capped at 16,384 characters. Naming a movie clip or the stage as a field
   writes the same messages as other wrong-kind names.
5. **Runtime layout through the host.** The player can't reach the fonts, so `ScriptHost` (the
   interface it already uses for plug-in frame scripts) gains `lay_out_field(block, shown)`.
   The engine's `ScriptRunner` answers it with `anicraft-text` and the same `field_layout` the
   commands use, so a field looks the same before and after a script sets its text. The result
   (`FieldLook`: outline and caret stops per line) is stored in the state; drawing a state
   needs nothing else, and the same document, state and events always give the same picture
   on one computer. A host that can't lay out (`NoScripts`) still sets and reads text; the
   field then draws only its box. `anicraft-text`'s `Layout` now reports **caret stops** (each
   line's character range and the x of every boundary a glyph starts at), which input editing
   and click-to-place need.
6. **Input editing is a pure state machine** (`anicraft_doc::field`): a press in an input field
   (or a selectable dynamic one) focuses it and places the caret; dragging selects; typed
   characters (`Event::Char`) replace the selection, respecting the line type and the maximum;
   Backspace, Delete, the arrows, Home and End move and edit by key code (Shift, key code 16,
   extends the selection); Enter starts a line in multiline fields; Tab moves to the next
   input field and selects its text; `Event::SelectAll` selects all. Keys still reach the
   stage's key listeners after editing, as they reach the stage from a focused field in
   ActionScript, so "press Enter to submit" scripts work. A one-line field scrolls sideways to
   keep the caret in view, a multiline one downwards. Dynamic fields select but never change.
7. **Test Movie** sends typed text, pastes and input-method commits as `Char`s, Ctrl+A (⌘A) as
   Select All and Ctrl+C (⌘C) as a copy of the selection (never a password's) while a field has
   the keyboard; it shows a text cursor over fields and tells the system's input methods where
   the field is. **Accessibility:** every field on the stage is an AccessKit node: input fields
   as text inputs (password and multiline inputs by their roles), dynamic fields as labels,
   named by instance name, with their current text as their value. `control.test_movie` takes
   `{type: text, text}` and `{type: select_all}` events and lists every field's text (and the
   focused field's caret and selection) in its state.
8. **HTML canvas export: native inputs, canvas text, authored outlines kept.**
   * Fields are exported with their box, settings and font. Until a script or the viewer
     changes a field's text, the page draws its **authored outline**, exactly as AniCraft
     does. `player.js` mirrors the Rust player's fields (tree nodes, `text`, `length`,
     `appendText`, messages), checked step by step against it in the Boa parity tests.
   * **Input fields are real, positioned `<input>` / `<textarea>` elements** over the canvas
     (`type=password` for passwords, `maxlength`, `aria-label` from the instance name), placed
     by the field's stage transform, transparent so the canvas draws the box. The browser then
     gives the viewer everything a text field should have that a canvas-drawn imitation would
     have to rebuild and would get wrong: input methods for every script, the on-screen
     keyboard on phones and tablets, the clipboard, each platform's own editing shortcuts,
     selection by touch, and screen readers. Their `input` event hands the value to the
     player, so scripts read what was typed. Canvas-drawn editing was rejected for these
     reasons; its one advantage (pixel parity of the caret with Test Movie) doesn't matter to
     viewers.
   * **Text a script sets is drawn with `fillText`** in the field's family, then the generic
     `sans-serif`, wrapped and aligned in the box as AniCraft does. Its glyphs come from the
     viewer's browser and may differ in shape and width from Test Movie when the family isn't
     on the viewer's computer. The export report says so for every page with fields ("text fields (their authored text keeps its outlines; text a script or the viewer changes is drawn with the browser's fonts)").
9. **Font embedding: not in this slice, by decision.** Embedding font files in the page would
   redistribute fonts the author may not be licensed to redistribute (most commercial desktop
   licences forbid web embedding), and the export can't know. Embedding outlines for a declared
   character set (as Animate's "Embed" does) has the same licensing question for glyph data,
   plus a UI and a size cost. Authored text is already exact (its outlines are in the page);
   only text scripts set at run time uses the viewer's fonts. When we add embedding it will be
   an explicit per-field character set the author opts into, with the licence question asked
   in the UI, recorded as its own decision.
10. **XFL import keeps fields.** `DOMDynamicText` and `DOMInputText` become one field each (a
    field is one instance, so it takes its main style; mixed styles are counted), with their
    instance name, box (`width`, `height`, `left`, `top`; auto-size fields grow to their text),
    line type (`lineType`), `border` (a black line round a white box, as Animate's "Show border"),
    `isSelectable`, `maxCharacters` and `variableName` (kept, counted under "kept, not run").
    Empty input fields are kept. The report lists them under "Runs in Test Movie". A self-authored
    corpus package (`textfields`) has golden renders.

## Consequences

* Animate projects with scores, quizzes and name entry play in Test Movie and in the HTML
  page without changes, and their fields stay editable as text in AniCraft.
* Exports other than HTML canvas show fields as authored; that matches Animate's video export.
* The document format is unchanged for files without fields; fields add one small object.
* Not yet: `htmlText` and HTML rendering in fields, per-character styles (ADR 0012),
  `Event.CHANGE`/`TextEvent` listeners and focus events, `restrict`, `scrollV`/`maxScrollV`,
  `selectionBeginIndex`/`setSelection` and other `TextField` properties and methods, text
  colour or format set by scripts, Cut, double-click word selection, a blinking caret, mouse
  wheel scrolling, embedded fonts in the HTML page (above), selecting dynamic text in the HTML
  page, clipping the HTML page's input elements by mask layers, `mouseX`/`mouseY` while the
  pointer is over one, and running ActionScript 1/2 variables.
