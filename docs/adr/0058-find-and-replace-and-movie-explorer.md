# ADR 0058: Find and Replace, and the Movie Explorer

* Status: accepted (2026-11-11)
* Relates to: ADR 0004 (destructive actions), ADR 0017 (plug-in policy), ADR 0022
  (accessibility), ADR 0040 (interface languages), ADR 0045 (editing a symbol in place)

## Context

Animate users who work on big files rely on two tools to get around them. **Edit › Find and
Replace** finds text (in text fields, frame labels, layer names, instance names and scripts),
fonts, colours, symbols, sounds and bitmaps, in the current scene or the whole document. It
can replace one result or all of them, and Find Next goes to the frame and selects the item.
The **Movie Explorer** shows the movie as a tree of scenes, layers, frames and what is on
them, and of symbols inside instances. Show buttons filter it by kind. A double-click goes
there.

Behaviour was learned from Adobe's public help pages and from using the tools. No Adobe code,
SDK or asset was used.

## Decision

1. **Three commands and a query, in the engine** (`crates/engine/src/find.rs`). Everything
   works headless, over the CLI and MCP. The shell uses the same functions.
   * `find.search {kind, text | font | color | symbol | sound | bitmap, match_case?,
     whole_word?, regex?, in?, scope?, offset?, limit?}` lists **located hits**. Each hit
     has the timeline (`scene` index or `symbol` id), `layer`, `frame` (the keyframe's
     start; none for a layer name), `element`, `field` (`text`, `label`, `layer_name`,
     `instance_name`, `script`, `font`, `fill`, `stroke`, `text_color`, `instance`, `sound`,
     `bitmap`), the match's characters (`start..end`), what was found and the text around
     it. Pages hold up to 10,000 hits, and one search keeps at most 100,000 (`truncated`).
   * **Scope.** `scene` (the default, as in Animate) is the current scene plus every symbol
     it uses, however deep. `document` is every scene and every Library symbol, used or not.
   * **Colours** match on 8-bit RGB; alpha is ignored. They cover solid fills, gradient
     stops, strokes and text colour. A replacement keeps each place's alpha, so a
     half-transparent red becomes a half-transparent green.
   * **Fonts** match the family name, ignoring case and surrounding spaces.
   * `find.replace {query, with, at?}`. With `at` (a hit as `find.search` returned it),
     it replaces that one match (Replace). Without it, it replaces every hit (Replace All).
     Either way it is **one undo step**, because it is one mutating command. Text in a text
     field, or a new font, is laid out again with this computer's fonts. A font that isn't
     installed falls back to another, and the result says so in `notes`. Some places are
     skipped, and the result says why: a text field or layer name left empty, a script over
     1 MiB, or an instance that would put a symbol inside itself. An instance of `to` can't go
     into `to` or into any symbol `to` uses. When nothing could be replaced, the command
     fails and records nothing.
   * **Confirmation (ADR 0004).** Replace All over more than 100 places (`REPLACE_ASK`) asks
     first in the editor. The confirmation is undoable-tier (`irreversible: false`), so a
     script runs it directly, and "don't ask again" works.
   * `find.goto {layer?, scene?, symbol?, path?, frame?, element?}` goes to a place. It
     closes any open symbols and finds the layer's timeline (layer ids are unique in a
     document). A scene is switched to. A symbol is **opened in place through the instances
     that lead to it**: the `path` given (the Movie Explorer knows it), or else the shortest
     chain from the current scene, then from any scene, found breadth-first, each symbol
     visited once. It falls back to opening the symbol alone from the Library, as a
     double-click in the Library does. That happens when no instance leads there, or when
     one is on a hidden or locked layer. It then picks the layer and frame and selects the
     element. Going somewhere records nothing to undo.

2. **A bounded pattern engine.** Text matching uses the `regex` crate (MIT OR Apache-2.0,
   crates.io). Its engines run in time linear in the text. Patterns that would compile past
   1 MiB (`size_limit`, `dfa_size_limit`), or nest deeper than 64, are refused with a
   message. So `(a+)+$` over 50,000 characters returns at once, and `a{1000}{1000}` is
   refused. Plain text, whole words (`\b(?:…)\b`) and match case are the same engine, with
   the text escaped. Empty matches are never hits, so `x*` doesn't match every gap. Only the
   `std`, `perf`, `unicode-case` and `unicode-perl` features are built: case folding and
   word boundaries work for any language, without the other Unicode tables. In
   regular-expression mode `$1` and `${name}` in the replacement put back groups. In plain
   mode the replacement is used exactly as typed.

3. **The Movie Explorer tree** (`crates/engine/src/explorer.rs`, `explorer.tree`). The rows
   that are showing come in order, each with a depth, a kind, document text (a name, a
   label, a text field's first line, a script's first line), whether it has children and
   whether it is open, and a stable **key** (`s0/l12/k0/e40/…`, a path of ids) that open
   rows are remembered by.
   * Scenes › layers (folder nesting as indentation; a closed folder hides its layers) ›
     keyframes that hold something (named by their label) › the frame script, the sound,
     symbol instances, text fields and bitmaps. Plain shapes aren't listed, as in Animate.
     An instance opens into its symbol's own layers. Under a separate row, the **symbol
     definitions** list every Library symbol, each opening into its timeline.
   * **Only open rows' children are made.** So the work follows what is shown, and a
     closed tree of a huge movie costs almost nothing. Limits: 200,000 rows (`truncated`)
     and 48 levels. A symbol already on the way down isn't entered again, so a symbol that
     holds itself, or two that hold each other, stops.
   * **Show filters**, as Animate's Show buttons: text, symbols, scripts, media (sounds and
     bitmaps), and frames. With frames off, layers and keyframes are left out, and what is
     on them is listed straight under the scene or the instance. The **text filter** looks
     through every row, as if all were open, and keeps the rows whose name holds it, with
     every row above them.
   * Every row carries its `find.goto` params (`go`). Rows inside instances carry the
     instance path, so a double-click opens the symbols in place. Rows under a definition
     open that symbol alone first, then follow the path.

4. **The shell** (`src/find_panel.rs`, `src/movie_explorer.rs`).
   * **Find and Replace** is a non-modal window: Edit › Find and Replace, Ctrl+F (⌘F),
     from the registry. It has a kind, a scope, a Find and a Replace with field (or a font
     list, colour wells or Library lists), the text options, and where to look. Results are
     listed **as you type**. The search runs again only when the query or the document
     changes; on 24,000 elements it takes under 10 ms. Clicking a result, Find Next (F3, or
     Enter in the Find field) and Find Previous (Shift+F3) go there. Replace replaces the
     current result and goes on to the next. Replace All goes through `App::request`, so the
     confirmation shows when it applies. Bad patterns are reported in the window, not in a
     dialog.
   * **Movie Explorer** is the other tab of the Library's dock band (Window › Movie
     Explorer, Alt+F3). The band's title follows the tab, and Properties gives up some
     height while the explorer shows. It has a search field and a Show menu (the kinds, all
     scenes or this scene, symbol definitions). A click on the chevron opens or closes a
     row, and a double-click or Enter goes there. The tree is made again only when the
     document or the view changes.
   * Both lists draw **only the rows in view** (`ScrollArea::show_rows`), so 100,000 rows
     scroll as smoothly as ten. Every row is named for screen readers: what it is, its
     text, and whether it is open or closed (ADR 0022). Every string goes through `tr!`
     (ADR 0040). Document text (names, labels, found text) is shown as written.

5. **Plug-ins** may call `find.*` and `explorer.*` (`plugin_may_call`). These commands read
   the document or edit it through the same undo path as every document command.

## Consequences

* An artist can rename a character across 60 scenes, swap a font, recolour a palette or swap
  a symbol, sound or bitmap everywhere in one step. Undo takes it all back.
* An agent over MCP can locate things (`find.search`), walk the tree page by page
  (`explorer.tree`) and go to any place (`find.goto`) without knowing the timeline's layout.
* Measured (dev profile, `cargo test -- --nocapture`). On 40 layers × 300 keyframes
  (12,000 text fields and 12,000 shapes) plus 500 symbols: a whole-word text search takes
  8.8 ms, a colour search 3.5 ms, and Replace All of 12,500 colours 25.5 ms, with its undo
  7.4 ms. The Movie Explorer on 100 layers × 120 keyframes, each with a text and an instance
  of a 3-layer symbol: everything open (144,112 rows) in 45 ms, the top rows (103) in
  0.08 ms, and a filtered tree in 67 ms.
* `regex` and its `regex-syntax`, `regex-automata` and `aho-corasick` crates are new in the
  tree. They are MIT OR Apache-2.0 (aho-corasick Unlicense OR MIT), so no new licence.

## Not in this slice

* Searching inside ActionScript string literals separately from code, and in action
  parameters (instance actions are listed in the Movie Explorer, not searched).
* Find by font size or style, and colour tolerance. Replacing a font keeps size and style.
* Video items (there are none yet), and editing names in place in the Movie Explorer.
  Animate's Movie Explorer can rename; here the Library and the timeline do that.
* The Movie Explorer's "Show movie elements" options for the stage selection, and copying
  the tree as text.
* Highlighting the match inside each result's text, and keeping the Find window's last
  query between sessions.
