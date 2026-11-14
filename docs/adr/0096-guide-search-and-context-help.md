# ADR 0096: User guide search and context help

* Status: accepted (2026-11-13)
* Relates to: ADR 0069 (the bundled guide), ADR 0004 (destructive actions), ADR 0022 and 0064
  (accessibility, Ctrl+F6 panel focus), ADR 0036 (web build), ADR 0040 (interface languages),
  ADR 0075 and 0087 (dockable panels), roadmap M8 ("next: translated guides, search, PDF, context
  help")

## Context

ADR 0069 bundled the user guide as static HTML that Help › User Guide opens offline. Two things
were missing for artists: finding a topic without knowing which chapter holds it, and getting
from the thing in front of them (a tool, a panel, a dialog) to the page about it. Animate users
expect **F1** to open help. The book is opened from a `file://` folder, often with no network, so
nothing may come from a server or a CDN, and a `file://` page may not `fetch` a JSON file.

## Decision

1. **Search: a prebuilt index, a small script, one ranking.** `anicraft_guide::search` splits
   the book into sections (one per heading, every heading of every page, levels 1 to 6) with
   their plain text. The book gains three files: `search-index.js` (the sections as JSON assigned
   to a global, so a `<script>` tag loads it from `file://`), `search.js` (no libraries) and
   `search.html`. The index is generated from the compiled-in chapters whenever the book is
   built (Help › User Guide, context help, `anicraft-cli guide`, the web build's
   `scripts/build-web.sh`), so it always matches the pages beside it. It is 329 KB for 366
   sections, the size of the text itself; we chose plain text over an inverted index because
   snippets need the text anyway and a 366-section scan is instant.
2. **The ranking is simple, integer-only and written twice.** Query words: lower case letters and
   digits, repeats and common words ("how", "do", "the") dropped unless nothing else is left, at
   most 8 words and 200 characters. A word matches a book word exactly (3), as its beginning
   (2, `tween` → "tweens"), or as a longer form of it (1, `masks` → "mask"). Every word must
   match in a section. Score per word: 30 × best in the heading + 8 × best in the chapter title
   + matches in the text (capped at 15); +60 when the words appear together in the heading, +20
   in the text. Ties keep book order. Integers keep the Rust (`anicraft-cli guide search`) and
   JavaScript ports identical; a test runs `search.js` under Node, when it is installed, and
   requires the same results, scores and snippets for a set of queries. We rejected stemming
   libraries and BM25-style weighting: they add floating point and a dependency to the browser
   side and did not change the top result for any query we tried.
3. **The search page is keyboard-first and safe.** Every page has a Search field in its header
   (a GET form to `search.html`, so it works without JavaScript reaching the search page); `/`
   focuses it from anywhere; ↓/↑ move through results, Enter opens the first, Esc clears. The
   result count is a polite live region. Results are built with `textContent` only (a test
   checks the script never uses `innerHTML`), and the index escapes `<`, U+2028 and U+2029.
4. **Context help is a map in the shell, checked against the book.** `src/context_help.rs` maps
   every tool, panel, dialog and command namespace (longest id prefix wins, so `file.export`
   can differ from `file.`) to a chapter link such as `timeline.md#onion-skin`. Tests require an
   entry for every `Tool`, `Panel`, `Dialog` and registry namespace, and pass every target
   through `anicraft_guide::check_targets`, the guide's own link check, so renaming a heading
   fails a test as a broken chapter link does. Links are written in Markdown form so they read
   like the chapters' own and resolve on GitHub too.
5. **What F1 is about**, in order: a dialog that holds the editor (a confirmation or prompt
   opens its command's section); the command palette's chosen entry; the window under the
   pointer (by its egui window id); a tool button under the pointer or with keyboard focus; the
   panel under the pointer (the Ctrl+F6 anchors, now with their layer, so a floating panel wins
   over a docked one beneath it); the stage, which means the tool in hand; the focused panel;
   the window in front; else the tool in hand. **Help › Help for This** does the same without the
   pointer (it is on the menu). Every panel's ☰ menu has **? Help for {panel}**. F1 works in text
   fields and while a modal dialog is open; only the shortcut editor's key recording takes it.
6. **F1 moves from the shortcut editor to help**, as in Animate and most desktop software. The
   shortcut editor keeps **Ctrl+Alt+Shift+K** (Animate's key). F1 stays a fixed key, now listed
   as "help for what you are pointing at"; the generated shortcut reference and the chapters
   that named F1 are updated.
7. **Opening at an anchor.** Opening `page.html#anchor` through the system's file handler loses
   the anchor on Windows, so the desktop writes the book, then a one-line `help.html` beside it
   whose meta refresh goes to the section, and opens that. The web build opens
   `guide/page.html#anchor` in a new tab. The status bar names the section ("Opened the user
   guide at “Building an armature”"). Context help is a shell action like Help › User Guide;
   the headless equivalent is `anicraft-cli guide search`.
8. **PDF: proposed, not built.** The pure-Rust, permissively licensed routes (`printpdf`,
   `krilla`, MIT or Apache-2.0) write PDF objects but leave typesetting to us: line breaking,
   tables, code blocks and lists, and they need font files embedded, which we may not commit and
   craft-fonts does not carry yet. That is a typesetter, not a slice. Proposal: next, a single-
   page print edition (`print.html`, every chapter with prefixed ids and in-page links) with the
   existing print stylesheet, so any browser's Save as PDF gives the whole book; a native PDF
   only if craft-fonts gains the UI typefaces and someone needs it headless. Until then each
   chapter prints cleanly (the chapter list, search field and pager are hidden in print).

## Consequences

* The book grows by about 340 KB (index and script); building it with the index takes about
  50 ms (`anicraft-cli guide --out`, 33 files, Windows), and one search from the CLI about 20 ms
  including start-up.
* A new tool, panel, dialog or command namespace fails a test until it has a place in the guide.
* Users who pressed F1 for the shortcut editor now get the guide; the guide, the shortcut sheet
  and Help › Keyboard Shortcuts show Ctrl+Alt+Shift+K.
* The web build (which doesn't link the book) opens the right page but names no section in the
  status bar.

## Not yet

* Translated guides (their catalogs belong in craft-i18n) and search across languages.
* The print edition and a PDF (see 8).
* Help for a single field or menu item; F1 in browsers that also open their own help page on F1
  (eframe doesn't stop that key's default), where Help › Help for This works.
