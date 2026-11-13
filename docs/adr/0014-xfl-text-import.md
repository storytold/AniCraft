# ADR 0014: Text from Animate documents

* Status: accepted (2026-11-09)
* Relates to: roadmap M4 (XFL import), ADR 0009 (XFL import), ADR 0012 (text)

## Context

Animate documents are full of text: titles, captions, speech bubbles, credits, score
counters. The XFL importer reported every text field as skipped. AniCraft now has a text
engine (ADR 0012): `ElementKind::Text(TextBlock)`, one style per block, laid out by
`anicraft-text` and stored as outlines.

An XFL text field (`DOMStaticText`, `DOMDynamicText`, `DOMInputText`) has a matrix, a box
(`width`, `height`, optional `left`/`top`), `autoExpand` for auto-size versus fixed width, and
a list of text runs: `<DOMTextRun><characters>…</characters><textAttrs><DOMTextAttrs …/>`.
The attributes give a face (a PostScript name such as `Arial-BoldMT`, or a device font such
as `_sans`), size, fill colour and alpha, alignment, letter spacing, line spacing, margins
and indent. Lines end with a carriage return (`&#xD;`). This structure comes from public
descriptions of XFL and from self-authored files only.

AniCraft's block has one style, but a field may have runs with different styles.

## Decision

1. **A field becomes one text block per run of consecutive lines that share a style.** Runs
   are split into lines at line breaks; each line takes one style; consecutive lines with the
   same style make one block, and the blocks stack downwards, each starting where the line
   after the previous block would (`lines × line_height`, which `anicraft-text` now reports).
   This keeps the most common mixed case exact: a big title line over smaller body lines,
   each block editable as text. A **line** that mixes styles (one bold word in a sentence)
   takes the style of most of its visible characters, and the report counts it. Splitting
   inside a line would need per-glyph positions from one shared layout; it waits for
   per-character styles in the text model (ADR 0012, "not yet").
2. **Placement.** The element's transform is the field's matrix, then the box offset
   (`left`, `top`) plus a 2-pixel gutter, the inset of text in Animate's text fields. A
   fixed-width field wraps at `width` (less its margins); an auto-size field never wraps, and
   its centred or right-aligned blocks align against the widest block, as lines of one field
   do.
3. **Fonts.** A face is looked up on this computer by its PostScript name, then as a family
   name (ignoring case and spaces), then as the family the PostScript name suggests
   (`TimesNewRomanPS-BoldItalicMT` is Times New Roman, bold, italic); `_sans`, `_serif` and
   `_typewriter` map to installed sans, serif and monospaced families. Boldness and slant
   come from the face (and `bold`/`italic` attributes when present). A family that isn't
   installed falls back as the text engine does (ADR 0012), the block records the family
   actually used, and the report names the missing families and the fallback: "Fonts not on
   this computer, drawn in Arial: Futura." The lookup lives in `anicraft-text`
   (`Fonts::find_face`, `guess_family`), so other importers can use it.
4. **Dynamic and input text come across as static text**, keeping their instance name on the
   element, and are counted in the report. They become fields with scripting (M6).
5. **Report, don't guess** (ADR 0009): justified text (drawn left-aligned), first-line
   indents, superscript and subscript, vertical text, filters on text, text cut to the
   engine's 100,000-character limit, and text on a computer with no fonts at all (nothing is
   drawn) are counted. Empty fields make nothing.
6. **Import lays text out once, with this computer's fonts.** The imported document stores
   the outlines like any text (ADR 0012), so it then looks the same everywhere. The desktop app
   registers its embedded Inter before opening a file from the command line, so command-line
   and menu imports lay out with the same fonts.

## Consequences

* Imported titles and captions render, export and stay editable as text (Text tool, Properties,
  `text.edit`).
* Lines with mixed styles lose their minor styles until text blocks have per-character styles;
  the report says how many.
* Positions depend on the fonts' metrics: with the original fonts installed, text sits where
  Animate puts it within about a pixel; a fallback family changes widths, as it would in
  Animate.
* The shared XML reader now keeps character data (`Element::text`), needed for
  `<characters>`; indentation between elements is dropped so memory doesn't grow with it.
* Not yet: per-character styles, kerning and rendering-mode settings, links on text, text
  along a path, input and dynamic fields with scripting (M6), embedded fonts in the document.

## Note (2026-11-11): fields stay fields

Point 4 is superseded by ADR 0066: `DOMDynamicText` and `DOMInputText` now import as one dynamic
or input field each, with their instance name, box, line type, border, selectability, maximum
length and (kept, not run) variable name, and run in Test Movie and the HTML export.
