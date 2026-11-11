# Localisation

AniCraft's interface can be drawn in other languages (ADR 0040). This page is for contributors
who write interface text, translators, and whoever builds a release.

## What is translated

Everything the shell draws: menus (labels from the command registry, translated as they are
drawn), panels, the timeline, Properties, tool options, dialogs, tooltips, status messages,
on-stage hints, and the names screen readers announce.

Not translated, on purpose: your documents (layer, symbol and scene names), command ids and
parameters, the `.anicraft` format, `anicraft-cli`, `serve` and MCP (scripts and agents see
the same English everywhere), log output, and the `--help` text.

Not yet: engine messages (errors, import reports and the body of confirmations stay English;
their titles and buttons are translated), default names of new layers and symbols, and
plug-in names.

## Choosing a language

**Preferences › Appearance › Language.** *System language* (the default) follows the
computer's (the browser's on the web); a language this build has no catalog for falls back to
English. A change shows at once. For one run, without saving: `anicraft --lang de`
(`--lang auto`, `--lang pseudo`).

Which languages a build offers depends on the catalogs it was built with (below). Today
craft-i18n has no AniCraft catalog yet, so builds offer English, and the **pseudo-locale**
(`--lang pseudo`; listed in Preferences in debug builds) for testing.

## The pseudo-locale

Every string accented, about a third longer and in brackets: `Save` → `[Šàvé ~~]`. Use it to
check a change:

* Text that is still plain English skipped translation.
* A label whose `]` is cut off is truncated: give it room, or let it wrap.
* Layouts must survive text a third longer (German and Finnish run that long).

```sh
cargo run -- --lang pseudo --screenshot shots/pseudo.png
cargo run -- --lang pseudo --open prefs --screenshot shots/pseudo-prefs.png
```

## Writing interface text (contributors)

Write English in the code, through the macros (`src/i18n/mod.rs`; in scope everywhere):

| You write | You get |
|---|---|
| `tr!("Save")` | `&'static str` in the interface language |
| `tr!("Saved {path}", path = p)` | `String`; placeholders filled after the lookup |
| `trn!("{n} layer", "{n} layers", count)` | `String`; the plural form for `count` (CLDR rules), `{n}` filled |
| `trc!("menu", "Open")` | a context word, when the same English means two things |
| `tr_noop!("Graphite")` in a table, `i18n::t(x)` where drawn | for `const` tables and functions returning English keys |
| `i18n::t_cmd(spec.id, spec.label)` | a registry command's label |
| `i18n::t(blend.label())` | a lower crate's `&'static str` label |

Rules:

* **One sentence, one template.** Never glue translated pieces together; word order differs
  between languages. Name placeholders for what they hold (`{name}`, `{frame}`), never `{}`.
* **Counts use `trn!`**, never `if n == 1`. Languages have up to six plural forms.
* **Keys stay English.** Translate at the point of display only; anything compared, stored,
  sent to the engine or used as an egui id stays English.
* **Window titles carry their own id** (`egui::Window::new(tr!("…")).id(egui::Id::new("…"))`),
  so a window keeps its place when the language changes.
* **Screen-reader names are text too**: `.named(tr!("…"))`.
* Data is not translated: numbers, units, timecode, shortcuts (`platform::shortcut_label`).

`cargo test` enforces it: `every_interface_string_goes_through_tr` fails on a prose literal
outside the macros (mark one that isn't interface text with `// i18n-exempt: why`), and the
accessibility audit fails on a name that the pseudo-locale didn't reach.

## Translating (translators)

Translations live in the Craft family's shared repository,
[storytold/craft-i18n](https://github.com/storytold/craft-i18n) (MIT OR Apache-2.0), in its
format: `locales/<locale>/anicraft.tsv`, one row per string, `context<TAB>English<TAB>translation`
(see its `docs/catalog-format.md`). Strings the apps share are in `locales/<locale>/common.tsv`
and already apply to AniCraft; AniCraft's catalog adds the rest and may override a shared row.

To get the list of strings:

```sh
scripts/i18n-extract.sh                                  # target/i18n/anicraft.tsv: every string
CRAFT_I18N_DIR=/abs/path/craft-i18n scripts/i18n-extract.sh
    # also prints how much of AniCraft each locale already covers
CRAFT_I18N_DIR=/abs/path/craft-i18n ANICRAFT_I18N_LOCALE=de scripts/i18n-extract.sh de.tsv
    # only the strings German doesn't have yet
```

(`scripts/i18n-extract.ps1` on Windows.) Every row is commented out, as craft-i18n's starter
files are: delete the `#` and type the translation after the last TAB. Keep `{placeholders}`
and a trailing `…`. Then open a pull request in craft-i18n. Follow its clean-room rule: write
from the plain meaning of the English, never from another product's translated interface.

A language is offered once craft-i18n has its `anicraft.tsv` and it is in `i18n::READY`
(left-to-right Latin, Greek and Cyrillic scripts today).

## Building with translations

```sh
git clone https://github.com/storytold/craft-i18n ../craft-i18n
CRAFT_I18N_DIR="$PWD/../craft-i18n" cargo build --release      # an absolute path
```

Without `CRAFT_I18N_DIR` the build is English (and pseudo). `CRAFT_I18N_REQUIRED=1` turns a
missing or empty checkout into a build error, for releases. A row the app can't use (bad
placeholders, wrong number of plural forms, an unknown `@` context) is skipped with a message
on the console, and that string shows in English.

## Scripts and direction

egui 0.36 lays text out glyph by glyph, without the Unicode bidirectional algorithm or
shaping. So:

| Script | Today |
|---|---|
| Latin, Greek, Cyrillic | Offered when a catalog exists. |
| Chinese, Japanese, Korean, Georgian | Draw correctly, but need a font the app doesn't bundle (a craft-fonts face), then a `READY` entry. |
| Arabic, Hebrew, Persian, Urdu, Kurdish, Dhivehi | Not offered: right to left needs bidi and shaping in egui's text layout, and mirrored widgets. We follow upstream egui rather than fork it. |
| Devanagari, Bengali, Tamil, Telugu, Thai, Lao | Not offered: they need shaping. |
| Vietnamese | Not offered yet: needs a check that every fallback face has its letters. |

Catalogs for these are still embedded (as fallbacks) and pass through the parser, so turning a
language on is one line once the shell can draw it.
