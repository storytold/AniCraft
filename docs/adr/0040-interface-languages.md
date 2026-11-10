# ADR 0040: Interface languages (localisation, first slice)

* Status: accepted (2026-11-10)
* Relates to: ADR 0005 (typography and fonts), ADR 0006 (platforms), ADR 0022 (accessibility),
  ADR 0036 (web build); outside PR #38 (ideas credited below)

## Context

M8 lists localisation. Artists coming from Animate use it in their own language, and the
Craft family already has a shared translation repository, **storytold/craft-i18n**
(MIT OR Apache-2.0): TSV catalogs keyed by the English text (`context<TAB>English<TAB>translation`,
`@plural`, `@id`, `@msg` rows), a `common.tsv` per locale with the strings the apps share
(about 1 200 rows in 50 locales today) and a `<app>.tsv` per app, consumed as an optional build
input (`CRAFT_I18N_DIR`), like craft-fonts. It has **no AniCraft catalog yet**.

The lookup code the other apps use lives in craft-libs, which is a git dependency; AniCraft
takes dependencies from crates.io only (`deny.toml`). Outside PR #38 built localisation on that
crate and on storytold's egui and winit forks (for Arabic), so it could not be taken as it is,
but its design was sound and several of its ideas are used here.

egui 0.36 from crates.io lays text out glyph by glyph: there is no Unicode bidirectional
algorithm (UAX #9) and no shaping. Arabic and Hebrew come out in the wrong order and Arabic
letters unjoined; Devanagari, Bengali, Tamil, Thai and Lao need shaping too. Layouts can be
built right to left (`Layout::right_to_left`), but egui's own widgets (menus, checkboxes,
sliders, scroll bars, text editing) have no mirrored mode. Chinese, Japanese, Korean and
Georgian need no shaping, only a font the app doesn't bundle.

## Decision

1. **English source text is the key, in our own small layer** (`src/i18n/`, shell only).
   Code keeps writing English: `tr!("Save")`, `tr!("Saved {path}", path = p)` (placeholders
   filled after the lookup, in one pass), `trn!("{n} frame", "{n} frames", n)` (CLDR plural
   rules for whole numbers, `plural.rs`), `trc!("menu", "Open")` (a context word), and
   `tr_noop!` to mark strings in tables that are translated where drawn (`i18n::t`). The macros
   take literals only, so the extractor sees every key. A missing translation, an unknown
   language or a malformed row is the English: the lookup never fails. Nothing below the shell
   changes: the command registry, command ids, the `.anicraft` format, the CLI, `serve` and MCP
   stay English. Menus are generated from the registry and translate each label when drawn
   (`@id <command id>` rows first, then the label's own row), so ids stay stable.

2. **Catalogs are craft-i18n's, read by our own parser.** We read the craft-i18n format as
   specified in its `docs/catalog-format.md` (rows, escapes, placeholders, `@plural` form
   counts, `@id`, `language.name`) in under 300 lines with its tests (`catalog.rs`) instead of depending on
   craft-libs. build.rs follows craft-i18n's `docs/integration.md`: `CRAFT_I18N_DIR=<checkout>`
   embeds `locales/<locale>/common.tsv` and `anicraft.tsv` with `include_str!` for every locale
   that has AniCraft's own catalog (plus a regional locale's base language, for fallback);
   unset, the interface is English; `CRAFT_I18N_REQUIRED=1` makes a missing checkout an error
   for releases. Fallback order for `pt-br`: pt-br app → pt-br shared → pt app → pt shared →
   English. No catalog files are committed here, and craft-i18n is never a Cargo dependency.
   One format means our strings go to craft-i18n without conversion, and every translation the
   family already has (its shared catalog) applies to AniCraft as soon as a locale has its
   `anicraft.tsv`.

3. **Which languages are offered.** A locale is offered when it has its own AniCraft catalog
   (a locale with only shared strings is a fallback, so the app never offers a language it is
   barely translated into, as craft-i18n's integration rule says) **and** is in `i18n::READY`:
   left-to-right languages in Latin, Greek or Cyrillic script, which the bundled faces cover and
   egui draws correctly. Right-to-left (Arabic, Hebrew, Persian, Urdu, Kurdish, Dhivehi),
   shaped scripts (Indic, Thai, Lao) and CJK/Georgian (font) catalogs are embedded but not
   offered until the shell can draw them (consequences).

4. **A pseudo-locale ships instead of a real language** while craft-i18n has no AniCraft
   catalog. `--lang pseudo` (and Preferences in debug builds) draws every string accented,
   about a third longer and bracketed (`Save` → `[Šàvé ~~]`), with placeholders kept: untranslated
   text stands out, truncation loses the `]`, and layouts are tested for German-length text.
   It uses only Latin-1 and Latin Extended-A letters, which every bundled face has.

5. **Choosing the language.** Preferences › Appearance › Language, first row: *System language
   (…)*, the default, then each offered language by its own name. A change shows in the same
   frame. The system language comes from `platform::system_locale` (the `sys-locale` crate,
   MIT OR Apache-2.0: Windows' user locale, macOS' preferred languages, `LC_ALL`/`LC_MESSAGES`/
   `LANG`, the browser's languages on the web) and is matched by full tag, language-script or
   language-region, then language (`de-AT` → `de`, `zh-TW` → `zh-hant`). A stored code this
   build lacks is kept and follows the system, so a newer build shows it again.
   `--lang <code>|auto|pseudo` sets it for one run without saving (screenshots, tests).

6. **The current language is a global with a per-thread override in tests.** Lookups read an
   atomic index (English is a fast path that returns the key without hashing); under
   `cfg(test)` `set_language` writes a thread-local instead, so parallel tests (which always
   run in English, whatever the machine) never change each other's language.

7. **Guardrails.** `every_interface_string_goes_through_tr` lexes every shell source file
   (`scan.rs`: strings, raw strings, comments, char literals) and fails on a prose literal
   outside the macros, unless its context shows it isn't interface text (logs, ids, commands,
   comparisons, match patterns) or it is marked `// i18n-exempt: why`. The accessibility audit
   runs the whole window in the pseudo-locale and fails on a screen-reader name that wasn't
   translated, which also catches strings passed through variables. `scripts/i18n-extract.sh`
   writes the template for translators and, with a craft-i18n checkout, the coverage of every
   locale and stale rows.

## Consequences

* Every interface string in the shell is translatable today, and a translation needs only a
  craft-i18n PR: `locales/<locale>/anicraft.tsv`, then a build with that checkout.
* Text from below the shell stays English in this slice: engine error messages, import
  reports, the body of confirmations (their titles and buttons are translated on an exact
  match), default document names (`Layer 2`, `Symbol 1`) and plug-in names. `@msg` templates
  for engine messages are the next step.
* **Right to left needs upstream work**: text needs bidi and shaping in egui's text layout, and
  the widgets a mirrored mode. We don't fork egui (PR #38 used storytold's fork); we follow
  upstream and offer RTL languages when egui on crates.io can draw them. CJK and Georgian need
  only a font from craft-fonts and a `READY` entry.
* Per frame, a busy window looks up a few hundred strings: about 13 ns each from a catalog and
  16 ns in the pseudo-locale (made once, then cached), and English returns the key without a
  lookup (measured on the development machine, Windows 10, in
  `lookups_are_cheap_enough_for_every_label_every_frame`): a few microseconds a frame.
* Translated window titles keep their egui ids explicit, so a window keeps its place and state
  when the language changes.

## Credit

From PR #38 (SoldierPePe): English source text as the key with a safe fallback; command labels
by `@id` then by label; `trn`/`fmt`-style plural and one-pass placeholder helpers; a per-thread
language for tests; `--lang` for one run and a pseudo mode that shows strings that skip
translation; build.rs following craft-i18n's integration rules. Not taken: the git
dependencies, the egui/winit forks and their API moves, the MSRV raise, the Arabic font and
the right-to-left mirroring helpers (they need the forked text layout).
