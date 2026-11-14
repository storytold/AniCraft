//! The localisation guideline (every interface string goes through `tr!`), the extractor that
//! lists strings for translators (`scripts/i18n-extract.sh`), and tests of the lookup.

use super::scan::{self, Key, Literal, Role};
use super::*;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn src_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// Every shell source file the guideline covers: all of `src/` but the i18n module itself and
/// test-only files. Read from disk, so a new file is covered without a list to keep.
fn shell_sources() -> Vec<(String, String)> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                if p.file_name().is_some_and(|n| n != "i18n") {
                    walk(&p, out);
                }
            } else if p.extension().is_some_and(|x| x == "rs") && !p.file_stem().is_some_and(|s| s.to_string_lossy().ends_with("_tests")) {
                out.push(p);
            }
        }
    }
    let mut files = Vec::new();
    walk(&src_dir(), &mut files);
    files.sort();
    let mut out = Vec::new();
    for f in files {
        let name = f.strip_prefix(src_dir()).map_or_else(|_| f.display().to_string(), |p| p.display().to_string().replace('\\', "/"));
        if let Ok(text) = std::fs::read_to_string(&f) {
            out.push((name, without_tests(&text)));
        }
    }
    assert!(out.len() > 25, "the shell's sources were not found under {}", src_dir().display());
    out
}

/// A file without its `#[cfg(test)] mod … {` block at the end (tests aren't interface).
fn without_tests(src: &str) -> String {
    let lines: Vec<&str> = src.lines().collect();
    let cut = lines.windows(2).position(|w| w.first().is_some_and(|l| l.trim() == "#[cfg(test)]") && w.get(1).is_some_and(|l| l.trim_start().starts_with("mod ") && l.trim_end().ends_with('{')));
    match cut {
        Some(n) => lines.get(..n).unwrap_or_default().join("\n"),
        None => src.to_string(),
    }
}

/// Code before a literal, on its line, that makes it something other than interface text:
/// logs and console output, ids and keys, commands and their parameters, attributes.
const NOT_UI: &[&str] = &[
    "eprintln!(", "println!(", "print!(", "panic!(", "assert", "unreachable!(", "Id::new(", "id_salt(", "persistent_id(", "json!(", "execute(", "perform(", "invoke(", ".run(", "find_command(", "#[", "env!(", "include_str!(", "include_bytes!(", "concat!(", "var(",
    "contains(", "starts_with(", "ends_with(", "strip_prefix(", "strip_suffix(", "matches!(", "== ", "!= ",
    // The start-up log (`startup::line`): English, like console messages, for bug reports.
    "line(&", "write(&format!(",
];

/// Whether a prose literal outside the macros is allowed: not interface text by its context,
/// or marked `i18n-exempt` (with the reason) on its line or the line above.
fn exempt(l: &Literal, lines: &[&str]) -> bool {
    let marked = |n: usize| n.checked_sub(1).and_then(|k| lines.get(k)).is_some_and(|t| t.contains("i18n-exempt"));
    if marked(l.line) || marked(l.line.saturating_sub(1)) {
        return true;
    }
    let after = l.after.trim_start();
    // A match arm's pattern (`"File" =>`, `"a" | "b"`, `"x" if …`).
    if after.starts_with("=>") || after.starts_with('|') || after.starts_with("if ") {
        return true;
    }
    NOT_UI.iter().any(|m| l.line_before.contains(m)) || l.value == "AniCraft"
}

#[test]
fn every_interface_string_goes_through_tr() {
    // docs/localisation.md › Writing interface text. Text a person reads is written
    // `tr!("…")` (or `tr_noop!` in a table, `trn!` for counts); this finds prose literals that
    // aren't, so a new label can't skip translation unnoticed.
    let mut found = Vec::new();
    for (file, src) in shell_sources() {
        let lines: Vec<&str> = src.lines().collect();
        let lits = scan::literals(&src);
        let roles = scan::roles(&lits);
        for (l, r) in lits.iter().zip(&roles) {
            if *r == Role::Plain && !l.bytes && scan::is_prose(&l.value) && !exempt(l, &lines) {
                let shown: String = l.value.chars().take(70).collect();
                found.push(format!("src/{file}:{}: {shown:?}", l.line));
            }
        }
    }
    assert!(
        found.is_empty(),
        "{} interface strings not routed through tr!/trn!/tr_noop! (or marked `// i18n-exempt: why` when not interface text):\n{}",
        found.len(),
        found.join("\n")
    );
}

/// Every key: the macros' literals in the shell, the registry's labels and menu names.
fn all_keys() -> BTreeSet<Key> {
    let mut keys: BTreeSet<Key> = shell_sources().iter().flat_map(|(_, src)| scan::keys(src)).collect();
    let mut add = |s: &str| {
        keys.insert(Key::Text { context: String::new(), source: s.to_string() });
    };
    for c in anicraft_engine::commands() {
        add(c.label);
        c.menu.split('/').filter(|p| !p.is_empty()).for_each(&mut add);
    }
    // Labels from below the shell that it translates where it draws them (`i18n::t`).
    use anicraft_engine::doc::{self, actions};
    use anicraft_engine::{cmds_brush::PaintMode, cmds_erase::EraseMode};
    doc::BlendMode::ALL.iter().for_each(|m| add(m.label()));
    doc::Filter::KINDS.iter().for_each(|(_, l)| add(l));
    doc::FilterSide::ALL.iter().for_each(|s| add(s.label()));
    doc::Easing::PRESETS.iter().for_each(|(n, _)| add(n));
    for e in doc::SoundEffect::PRESETS.iter().chain([&doc::SoundEffect::Custom(Vec::new())]) {
        add(e.label());
        add(e.hint());
    }
    add(&doc::Easing::Linear.label());
    doc::Property::ALL.iter().for_each(|p| add(p.label()));
    doc::FilterParam::ALL.iter().for_each(|p| add(p.label()));
    add("Filters");
    add("Filter");
    doc::ColorStyle::ALL.iter().for_each(|s| add(s.label()));
    doc::EaseFamily::ALL.iter().for_each(|f| add(f.label()));
    doc::EaseDir::ALL.iter().for_each(|d| add(d.label()));
    doc::EaseGroup::ALL.iter().for_each(|g| add(g.label()));
    doc::ProfilePreset::ALL.iter().for_each(|p| add(p.label()));
    doc::Cap::ALL.iter().for_each(|c| add(c.label()));
    doc::Join::ALL.iter().for_each(|j| add(j.label()));
    doc::DashKind::ALL.iter().for_each(|k| add(k.label()));
    for s in doc::SoundSync::ALL {
        add(s.label());
        add(s.hint());
    }
    [doc::SoundFormat::Wav, doc::SoundFormat::Ogg, doc::SoundFormat::Mp3].iter().for_each(|f| add(f.label()));
    for e in actions::ButtonEvent::ALL {
        add(e.label());
        add(e.hint());
    }
    actions::KINDS.iter().for_each(|(_, l)| add(l));
    actions::Property::ALL.iter().for_each(|p| add(p.label()));
    actions::Language::ALL.iter().for_each(|l| add(l.label()));
    for m in EraseMode::ALL {
        add(m.label());
        add(m.hint());
    }
    for m in PaintMode::ALL {
        add(m.label());
        add(m.hint());
    }
    // The brush library (ADR 0068): built-in brushes, their hints and the pressure curves.
    for (k, n) in anicraft_engine::brush_library::BUILTIN {
        add(n);
        add(anicraft_engine::brush_library::builtin_hint(k));
    }
    anicraft_engine::geom::pressure::CURVE_PRESETS.iter().for_each(|(_, l)| add(l));
    keys
}

#[test]
fn every_key_is_a_valid_catalog_source() {
    // A key a catalog row could never match (unpaired braces, a plural whose forms disagree)
    // would stay English in every language.
    let keys = all_keys();
    assert!(keys.len() > 300, "only {} keys found", keys.len());
    let mut bad = Vec::new();
    for k in &keys {
        let row = match k {
            Key::Text { context, source } => format!("{context}\t{}\t{}", escape(source), escape(source)),
            Key::Plural { one, other } => format!("@plural\t{}|{}\t{}|{}", escape(one), escape(other), escape(one), escape(other)),
        };
        let (rows, problems) = catalog::parse(&row);
        if rows.len() != 1 {
            bad.push(format!("{k:?}: {problems:?}"));
        }
    }
    assert!(bad.is_empty(), "keys no catalog row can translate:\n{}", bad.join("\n"));
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\n', "\\n").replace('\t', "\\t")
}

fn row(k: &Key) -> (String, String) {
    match k {
        Key::Text { context, source } => (context.clone(), escape(source)),
        Key::Plural { one, other } => ("@plural".into(), format!("{}|{}", escape(one), escape(other))),
    }
}

const HEADER: &str = "\
# <Language> (<locale>) interface catalog for AniCraft.
# Columns, separated by a TAB:  context <TAB> English source text <TAB> translation
#   context  usually empty. `@plural`, `@id` and `@msg` mean something to the apps; any other
#            word is a label that tells apart two meanings of the same English.
#   escapes  \\n  \\t  \\\\  inside a column. `{name}` placeholders must appear in both columns (in any
#            order), and a trailing \"…\" in one needs one in the other.
#   @plural  English `one|other`; the translation lists this language's plural forms, `|`-separated.
#   Lines starting with # and blank lines are ignored. A missing entry shows in English.
#
# Clean room: every entry was written from the plain meaning of the English text in ordinary
# computing vocabulary. No other product's localisation, string table or translated UI was
# consulted or copied; add entries the same way.
";

/// A craft-i18n locale's merged catalog, read from a checkout on disk: the base language's
/// files, then the locale's, shared before AniCraft's.
fn disk_catalog(locales: &Path, code: &str) -> (Catalog, Vec<catalog::Row>) {
    let read = |c: &str, f: &str| std::fs::read_to_string(locales.join(c).join(f)).map(|t| catalog::parse(&t).0).unwrap_or_default();
    let mut cat = Catalog::default();
    let mut chain = vec![code.to_string()];
    if let Some(base) = regional_base(code) {
        chain.insert(0, base.to_string());
    }
    for c in &chain {
        cat.merge(read(c, "common.tsv"));
        cat.merge(read(c, "anicraft.tsv"));
    }
    (cat, read(code, "anicraft.tsv"))
}

fn translated(cat: &Catalog, k: &Key) -> bool {
    match k {
        Key::Text { context, source } => cat.get(context, source).is_some(),
        Key::Plural { one, other } => cat.plural(one, other).is_some(),
    }
}

#[test]
#[ignore = "a tool, run by scripts/i18n-extract.sh"]
fn extract() {
    // Writes every interface string as a craft-i18n catalog template (each row commented out,
    // as craft-i18n's starter files are) to $ANICRAFT_I18N_OUT, or prints it. With
    // $CRAFT_I18N_DIR, prints how much of AniCraft each craft-i18n locale already translates
    // (its shared catalog counts) and the stale rows of AniCraft's own catalogs; with
    // $ANICRAFT_I18N_LOCALE too, the template lists only what that locale is missing.
    let keys = all_keys();
    let checkout = std::env::var_os("CRAFT_I18N_DIR").map(|d| PathBuf::from(d).join("locales"));
    let only = std::env::var("ANICRAFT_I18N_LOCALE").ok().map(|l| l.to_ascii_lowercase());
    let mut missing_in: Option<Catalog> = None;
    if let Some(locales) = &checkout {
        let mut codes: Vec<String> = std::fs::read_dir(locales).map(|r| r.flatten().filter(|e| e.path().is_dir()).map(|e| e.file_name().to_string_lossy().into_owned()).collect()).unwrap_or_default();
        codes.sort();
        println!("locale     translated        stale  (of {} AniCraft strings; READY = offered once it has anicraft.tsv)", keys.len());
        for code in &codes {
            let (cat, own) = disk_catalog(locales, code);
            let n = keys.iter().filter(|k| translated(&cat, k)).count();
            let stale = own.iter().filter(|r| !r.context.starts_with('@') && !keys.contains(&Key::Text { context: r.context.clone(), source: r.source.clone() })).count();
            let ready = if super::ready(code, READY) { "READY" } else { "" };
            println!("{code:<10} {n:>5} ({:>3} %)  {stale:>8}  {ready}", n * 100 / keys.len().max(1));
            if only.as_deref() == Some(code.as_str()) {
                missing_in = Some(cat);
            }
        }
    }
    let mut text = String::from(HEADER);
    text.push('\n');
    let mut rows = 0;
    for k in &keys {
        if missing_in.as_ref().is_some_and(|c| translated(c, k)) {
            continue;
        }
        let (context, source) = row(k);
        text.push_str(&format!("#{context}\t{source}\t\n"));
        rows += 1;
    }
    match std::env::var_os("ANICRAFT_I18N_OUT") {
        Some(out) => {
            let out = PathBuf::from(out);
            if let Some(dir) = out.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            std::fs::write(&out, text).unwrap_or_else(|e| panic!("could not write {}: {e}", out.display()));
            println!("wrote {rows} strings to {}", out.display());
        }
        None => print!("{text}"),
    }
}

// ----- lookup -----------------------------------------------------------------------------------

const DE_COMMON: &str = "@id\tlanguage.name\tDeutsch\n\tSave\tSpeichern\n\tOpen\tÖffnen\n@plural\t{n} frame|{n} frames\t{n} Bild|{n} Bilder\n";
const DE_APP: &str = "\tOpen\tDatei öffnen\n\tSaved {path}\t{path} gespeichert\n@id\tedit.undo\tRückgängig\n";
const PT_COMMON: &str = "@id\tlanguage.name\tPortuguês\n\tSave\tGuardar\n\tOpen\tAbrir\n";
const PT_BR_APP: &str = "@id\tlanguage.name\tPortuguês (Brasil)\n\tSave\tSalvar\n@plural\t{n} frame|{n} frames\t{n} quadro|{n} quadros\n";
const RU_APP: &str = "@id\tlanguage.name\tРусский\n@plural\t{n} frame|{n} frames\t{n} кадр|{n} кадра|{n} кадров\n@plural\t{n} layer|{n} layers\t{n} слой|{n} слоя\n";

fn fixture() -> Languages {
    let embedded = [("de", DE_COMMON, DE_APP), ("pt", PT_COMMON, ""), ("pt-BR", "", PT_BR_APP), ("ru", "", RU_APP), ("ar", "\tSave\tحفظ\n", "\tOpen\tفتح\n"), ("fr", "\tSave\tEnregistrer\n", "")];
    let (l, problems) = Languages::new(&embedded, READY);
    assert!(problems.is_empty(), "{problems:?}");
    l
}

#[test]
fn offered_languages_have_their_own_catalog_and_a_ready_script() {
    let l = fixture();
    let codes: Vec<&str> = l.list.iter().map(|x| x.code.as_str()).collect();
    // French has only shared strings and Portuguese is a fallback: neither is offered. Arabic
    // needs right-to-left layout egui doesn't have yet.
    assert_eq!(codes, ["en", "de", "pt-br", "ru", "pseudo"]);
    let names: Vec<&str> = l.list.iter().map(|x| x.name.as_str()).collect();
    assert_eq!(names, ["English", "Deutsch", "Português (Brasil)", "Русский", "Pseudo-locale (layout test)"]);
}

#[test]
fn lookups_fall_back_from_app_to_shared_to_base_language_to_english() {
    let l = fixture();
    let (de, pt_br, ru) = (1, 2, 3);
    assert_eq!(l.lookup(de, "", "Save"), Some("Speichern"), "shared");
    assert_eq!(l.lookup(de, "", "Open"), Some("Datei öffnen"), "the app's row wins over the shared one");
    assert_eq!(l.lookup(de, "menu", "Open"), Some("Datei öffnen"), "a context falls back to the plain row");
    assert_eq!(l.lookup(de, "", "Missing"), None);
    assert_eq!(l.lookup(pt_br, "", "Save"), Some("Salvar"));
    assert_eq!(l.lookup(pt_br, "", "Open"), Some("Abrir"), "from the base language");
    assert_eq!(l.lookup(0, "", "Save"), None, "English has no catalog");
    assert_eq!(l.lookup(99, "", "Save"), None, "an index out of range is English");
    // Plurals by each language's rules; a row with the wrong number of forms is English.
    assert_eq!(l.plural(de, "{n} frame", "{n} frames", 1), "{n} Bild");
    assert_eq!(l.plural(de, "{n} frame", "{n} frames", 0), "{n} Bilder");
    assert_eq!(l.plural(pt_br, "{n} frame", "{n} frames", 0), "{n} quadro", "Portuguese counts 0 as one");
    assert_eq!([1, 3, 5, 21].map(|n| l.plural(ru, "{n} frame", "{n} frames", n)), ["{n} кадр", "{n} кадра", "{n} кадров", "{n} кадр"]);
    assert_eq!(l.plural(ru, "{n} layer", "{n} layers", 5), "{n} layers", "two forms for Russian: skipped");
    assert_eq!(l.plural(0, "{n} frame", "{n} frames", 1), "{n} frame");
}

#[test]
fn every_lookup_kind_reads_the_catalog() {
    // The same paths `tr!`, `t_dyn`, `t_cmd` and `trn!` take with a catalog built in.
    let l = fixture();
    let (de, pseudo) = (1, l.list.len() - 1);
    assert_eq!(l.text(de, "", "Save"), "Speichern");
    assert_eq!(l.text(de, "menu", "Nothing"), "Nothing");
    assert_eq!(l.text(0, "", "Save"), "Save");
    assert_eq!(l.text(pseudo, "", "Save"), "[Šàvé ~~]");
    assert_eq!(l.text_dyn(de, "Open"), "Datei öffnen");
    assert_eq!(l.text_dyn(de, "Delete Layer"), "Delete Layer");
    assert_eq!(l.text_dyn(0, "Open"), "Open");
    assert_eq!(l.text_dyn(pseudo, "Open"), "[Öpéñ ~~]");
    assert_eq!(l.command(de, "edit.undo", "Undo"), "Rückgängig", "an @id row first");
    assert_eq!(l.command(de, "file.save", "Save"), "Speichern", "then the label");
    assert_eq!(l.command(de, "file.other", "Other"), "Other");
    assert_eq!(l.command(0, "edit.undo", "Undo"), "Undo");
    assert_eq!(l.count(de, "{n} frame", "{n} frames", 2), "{n} Bilder");
    assert_eq!(l.count(0, "{n} frame", "{n} frames", 1), "{n} frame");
    assert_eq!(l.count(pseudo, "{n} frame", "{n} frames", 1), "[{n} fŕàmé ~~]");
}

#[test]
fn system_locales_and_preferences_resolve() {
    let l = fixture();
    let code = |pref: &str, sys: Option<&str>| l.get(l.resolve(pref, sys)).code.clone();
    assert_eq!(code(AUTO, Some("de-DE")), "de");
    assert_eq!(code(AUTO, Some("de_AT.UTF-8")), "de");
    assert_eq!(code(AUTO, Some("pt-BR")), "pt-br");
    assert_eq!(code(AUTO, Some("pt-PT")), "en", "Portuguese itself isn't offered");
    assert_eq!(code(AUTO, Some("ru")), "ru");
    assert_eq!(code(AUTO, Some("ar-EG")), "en", "not offered yet");
    assert_eq!(code(AUTO, Some("C")), "en");
    assert_eq!(code(AUTO, Some("zh-TW")), "en", "Chinese isn't offered yet (it needs a font)");
    assert_eq!(code(AUTO, Some("zh_CN")), "en");
    assert_eq!(code(AUTO, Some("")), "en");
    assert_eq!(code(AUTO, None), "en");
    assert_eq!(code("DE", Some("ru-RU")), "de", "a chosen language wins over the system's");
    assert_eq!(code("pt_BR", None), "pt-br");
    assert_eq!(code("xx-new", Some("ru-RU")), "ru", "a code this build lacks follows the system");
    assert_eq!(code(PSEUDO, Some("de")), "pseudo");
    assert_eq!(code(AUTO, Some("pseudo")), "en", "the system never picks the pseudo-locale");
}

#[test]
fn the_interface_language_changes_lookups_on_this_thread() {
    // Built without catalogs (tests don't set CRAFT_I18N_DIR): English and the pseudo-locale.
    assert_eq!(set_language("xx"), "en");
    assert_eq!(tr!("Save"), "Save");
    assert_eq!(tr!("Saved {path}", path = "a.anicraft"), "Saved a.anicraft");
    assert_eq!(trn!("{n} frame", "{n} frames", 1), "1 frame");
    assert_eq!(trn!("{n} frame", "{n} frames", 3usize), "3 frames");
    assert_eq!(trc!("menu", "Open"), "Open");
    assert_eq!(tr_noop!("Graphite"), "Graphite");
    assert_eq!(t_cmd("edit.undo", "Undo"), "Undo");
    assert_eq!(t_dyn("Delete Layer"), "Delete Layer");
    assert!(known(AUTO) && known("en") && known("pseudo") && !known("xx"));

    assert_eq!(set_language(PSEUDO), "pseudo");
    assert_eq!(tr!("Save"), "[Šàvé ~~]");
    assert!(std::ptr::eq(tr!("Save"), tr!("Save")), "made once, then cached");
    assert_eq!(tr!("Saved {path}", path = "a.anicraft"), "[Šàvéď a.anicraft ~~]", "values are filled after the lookup, untouched");
    assert_eq!(trn!("{n} frame", "{n} frames", 2), "[2 fŕàméš ~~]");
    assert_eq!(t_dyn("Undo"), "[Üñďö ~~]");
    assert_eq!(t_cmd("edit.undo", "Undo"), "[Üñďö ~~]");
    assert!(offered().iter().any(|(c, _)| *c == PSEUDO), "listed while in use");
    assert_eq!(name_of("pseudo"), "Pseudo-locale (layout test)");
    assert_eq!(name_of("zz"), "zz");

    set_language(AUTO);
    assert_eq!(code(), "en", "tests always follow English as the system language");
    assert_eq!(offered().first(), Some(&("en", "English")));
}

#[test]
fn fill_is_one_pass_and_keeps_what_it_cannot_fill() {
    assert_eq!(fill("{a} and {b}", &[("a", &"{b}"), ("b", &2)]), "{b} and 2");
    assert_eq!(fill("{missing} {", &[]), "{missing} {");
    assert_eq!(fill("}{x}{", &[("x", &"y")]), "}y{");
    assert_eq!(fill("", &[]), "");
    assert_eq!(fill("é{n}é", &[("n", &1)]), "é1é");
}

#[test]
fn lookups_are_cheap_enough_for_every_label_every_frame() {
    // A busy frame draws a few hundred labels. Measured, not asserted tightly (CI machines vary):
    // the bound only catches an accidental quadratic lookup.
    let l = fixture();
    let start = web_time::Instant::now();
    let mut hits = 0usize;
    for _ in 0..100_000 {
        hits += usize::from(l.lookup(1, "", "Open").is_some()) + usize::from(l.lookup(1, "", "Not in the catalog").is_some());
    }
    let catalog_ns = start.elapsed().as_nanos() / 200_000;
    set_language(PSEUDO);
    let start = web_time::Instant::now();
    for _ in 0..100_000 {
        hits += tr!("Save").len();
    }
    let pseudo_ns = start.elapsed().as_nanos() / 100_000;
    set_language(ENGLISH);
    let start = web_time::Instant::now();
    for _ in 0..100_000 {
        hits += tr!("Save").len();
    }
    let english_ns = start.elapsed().as_nanos() / 100_000;
    eprintln!("lookup: catalog {catalog_ns} ns, pseudo (cached) {pseudo_ns} ns, English {english_ns} ns per string");
    assert!(hits > 0);
    assert!(catalog_ns < 20_000 && pseudo_ns < 20_000, "lookups this slow would cost a frame");
}

// ----- the whole window in the pseudo-locale ------------------------------------------------------

/// What the document itself says (a document as JSON): its names, labels and text.
fn strings_in(v: &serde_json::Value, out: &mut BTreeSet<String>) {
    match v {
        serde_json::Value::Array(a) => a.iter().for_each(|x| strings_in(x, out)),
        serde_json::Value::Object(o) => {
            for (k, x) in o {
                match x {
                    serde_json::Value::String(s) if matches!(k.as_str(), "name" | "text" | "label") && s.chars().filter(char::is_ascii_alphabetic).count() >= 3 => {
                        out.insert(s.clone());
                    }
                    _ => strings_in(x, out),
                }
            }
        }
        _ => {}
    }
}

/// Names egui gives its own parts.
const EGUI_OWN: [&str; 1] = ["Close window"];

/// Names a screen reader would announce that are still plain English in the pseudo-locale,
/// for the window after `setup` (a few frames to settle).
/// The app on a headless context with AccessKit on, in the pseudo-locale, after `setup`.
fn pseudo_app(setup: impl FnOnce(&mut crate::app::App, &egui::Context)) -> (crate::app::App, egui::Context) {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let session = anicraft_engine::Session::new(anicraft_engine::doc::sample::demo());
    let mut app = crate::app::App::with_context(&ctx, session, Some(PathBuf::from("unused.png")));
    // In the preferences (unsaved in screenshot mode), so a setup that changes them keeps it.
    app.prefs.language = PSEUDO.into();
    app.apply_prefs(&ctx);
    setup(&mut app, &ctx);
    (app, ctx)
}

fn screen() -> egui::RawInput {
    egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 940.0))), ..Default::default() }
}

/// Names in `nodes` that are ours and still plain English.
fn plain_names(what: &str, app: &crate::app::App, nodes: Vec<(egui::accesskit::NodeId, egui::accesskit::Node)>) -> Vec<String> {
    // Document text is the document's (layer, symbol and scene names): not ours to translate.
    let mut doc_text = BTreeSet::new();
    if let Ok(v) = serde_json::to_value(&app.s.doc) {
        strings_in(&v, &mut doc_text);
    }
    let mut found = BTreeSet::new();
    for (_, n) in nodes {
        for name in [n.label(), n.description()].into_iter().flatten() {
            // egui names its own window close button; it offers no way to name it ourselves
            // (docs/localisation.md › Not yet). The product name stays as it is in every
            // language (the macOS app menu is titled with it, ADR 0006), as the source scan allows.
            let translated = name.chars().any(pseudo::is_accented) || EGUI_OWN.contains(&name) || name == "AniCraft";
            let ours = scan::is_prose(name) && !doc_text.contains(name) && !doc_text.iter().any(|d| name.contains(d.as_str()));
            if ours && !translated {
                found.insert(format!("[{what}] {:?} {name:?}", n.role()));
            }
        }
    }
    found.into_iter().collect()
}

fn untranslated_names(what: &str, setup: impl FnOnce(&mut crate::app::App, &egui::Context)) -> Vec<String> {
    let (mut app, ctx) = pseudo_app(setup);
    let mut update = None;
    for _ in 0..3 {
        let mut out = ctx.run_ui(screen(), |ui| app.frame(ui));
        out.textures_delta.clear();
        update = out.platform_output.accesskit_update;
    }
    plain_names(what, &app, update.map(|u| u.nodes).unwrap_or_default())
}

/// Clicks every control the window shows in `setup`'s state, one after another (through
/// AccessKit, as a screen reader would; number fields are stepped too), and returns the names
/// and status lines that came out in plain English. Prompts are dismissed after each click,
/// so nothing is written to disk.
fn click_everything(what: &str, setup: impl FnOnce(&mut crate::app::App, &egui::Context)) -> Vec<String> {
    use egui::accesskit::{Action, ActionRequest, Role, TreeId};
    let (mut app, ctx) = pseudo_app(setup);
    let frame = |app: &mut crate::app::App, events: Vec<egui::Event>| {
        let mut input = screen();
        input.events = events;
        let mut out = ctx.run_ui(input, |ui| app.frame(ui));
        out.textures_delta.clear();
        out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default()
    };
    let mut nodes = Vec::new();
    for _ in 0..3 {
        nodes = frame(&mut app, Vec::new());
    }
    let mut done: BTreeSet<String> = BTreeSet::new();
    let mut found = Vec::new();
    for _ in 0..90 {
        // The next control not clicked yet (by role and name: ids change as panels redraw).
        let next = nodes.iter().find_map(|(id, n)| {
            let key = format!("{:?} {}", n.role(), n.label().unwrap_or_default());
            let steps = n.supports_action(Action::Increment);
            let clickable = (steps || n.supports_action(Action::Click)) && !matches!(n.role(), Role::Window | Role::TitleBar | Role::ScrollBar);
            (clickable && !done.contains(&key)).then_some((*id, key, steps))
        });
        let Some((id, key, steps)) = next else { break };
        done.insert(key);
        let request = |action| egui::Event::AccessKitActionRequest(ActionRequest { action, target_tree: TreeId::ROOT, target_node: id, data: None });
        // A number field steps up (as an arrow key does); anything else is clicked.
        let events = vec![request(if steps { Action::Increment } else { Action::Click })];
        app.status = None;
        frame(&mut app, events);
        app.prompt = None;
        // Picking a language in Preferences is one of the controls: back to the pseudo-locale.
        if app.prefs.language != PSEUDO {
            app.prefs.language = PSEUDO.into();
            app.apply_prefs(&ctx);
        }
        nodes = frame(&mut app, Vec::new());
        if let Some(s) = &app.status
            && !s.text.chars().any(pseudo::is_accented)
        {
            found.push(format!("[{what}] status {:?}", s.text));
        }
        // Installed font families, and languages, are listed by their own names.
        let mut fonts: BTreeSet<String> = app.system_fonts.as_ref().map(|f| f.families.iter().cloned().collect()).unwrap_or_default();
        fonts.extend(offered().into_iter().map(|(_, name)| name.to_string()));
        found.extend(plain_names(what, &app, nodes.clone()).into_iter().filter(|f| !fonts.iter().any(|name| f.ends_with(&format!("{name:?}")))));
    }
    found.sort();
    found.dedup();
    found
}

#[test]
fn every_control_can_be_used_in_the_pseudo_locale() {
    // Pressing every button, opening every list and stepping every number in the main states
    // of the window: nothing may panic, and what each one shows next (a popup, a status line,
    // a dialog) is translated too.
    let mut found = click_everything("default", |_, _| {});
    found.extend(click_everything("shape selected", |a, _| {
        a.run("layer.select", serde_json::json!({"index": 0}));
        a.run("edit.select_all", serde_json::json!({}));
    }));
    found.extend(click_everything("text selected", |a, _| {
        a.run("text.create", serde_json::json!({"x": 40.0, "y": 40.0, "text": "Title"}));
        a.run("edit.select_all", serde_json::json!({}));
    }));
    found.extend(click_everything("symbol with filters", |a, _| {
        a.run("layer.select", serde_json::json!({"index": 0}));
        a.run("edit.select_all", serde_json::json!({}));
        let symbol = a.run("modify.convert_to_symbol", serde_json::json!({"name": "Clip"})).and_then(|v| v["symbol"].as_u64());
        a.run("symbol.set_kind", serde_json::json!({"symbol": symbol, "kind": "movie_clip"}));
        a.run("element.set_filters", serde_json::json!({"filters": [{"type": "drop_shadow"}, {"type": "gradient_glow"}]}));
    }));
    // Every colour effect style's controls in Properties (ADR 0056).
    for style in ["brightness", "tint", "advanced", "alpha"] {
        found.extend(click_everything(&format!("colour effect {style}"), |a, _| {
            a.run("layer.select", serde_json::json!({"index": 0}));
            a.run("edit.select_all", serde_json::json!({}));
            a.run("element.set_color_effect", serde_json::json!({"style": style}));
        }));
    }
    for panel in ["sound", "camera", "springbones", "shapebones", "shapeweights", "warp", "motion", "motionpins", "actions", "script", "prefs", "envelope", "onionmenu", "editmultiple", "edittransform", "onionsymbol", "find", "explorer", "publish", "publishreport", "publishanim", "start", "testmovie", "testmovieinside", "templates", "savetemplate", "brushlibrary", "brushcurve"] {
        found.extend(click_everything(panel, |a, _| a.open_panel(Some(panel))));
    }
    // Test Movie in a window of its own (ADR 0062; egui stands in for the window system here).
    found.extend(click_everything("test movie window", |a, ctx| {
        ctx.set_embed_viewports(false);
        a.open_panel(Some("testmovie"));
    }));
    for tool in [crate::app::Tool::Brush, crate::app::Tool::PaintBrush, crate::app::Tool::Eraser, crate::app::Tool::PaintBucket, crate::app::Tool::Pencil, crate::app::Tool::Width] {
        found.extend(click_everything(&format!("{tool:?}"), |a, _| a.tool = tool));
    }
    assert!(found.is_empty(), "{} things in plain English after using the controls:\n{}", found.len(), found.join("\n"));
}

#[test]
fn status_lines_for_every_result_are_translated() {
    // What the status bar says for results a test can't produce for real (saves, exports,
    // imports, recovered work, plug-ins), from results shaped as the commands return them.
    use serde_json::json;
    let (mut app, _ctx) = pseudo_app(|_, _| {});
    let full = json!({
        "saved": "walk.anicraft", "written": "walk.png", "seconds": 1.5, "frames": 3, "columns": 2, "rows": 2, "map": "walk.json",
        "kb": 12, "layers": 2, "masks": 1, "elements": 3, "report": "Imported walk.fla.", "copied": 2, "cut": 2, "pasted": 2,
        "removed": 2, "cleared": 2, "original_newer": true, "sounds": 1, "name": "beat", "sound": 1, "log": ["Did it."],
        "installed": [{"name": "Tint", "version": "1.0"}], "failed": ["bad.wasm: not a plug-in"], "replaced": "Tint 0.9",
        "plugin": {"name": "Tint", "version": "1.0"}, "undone": "Move", "redone": "Move", "bytes": 4096, "stored_frames": 2,
    });
    let bare = json!({});
    let ids = [
        "file.new", "file.open", "file.recover", "file.discard_recovery", "file.save", "file.save_as", "file.import", "file.import_to_library",
        "file.export_audio", "file.export_sprite_sheet", "file.export_html", "file.export_lottie", "file.export_png", "file.export_gif", "file.export_apng", "file.export_webp",
        "sound.delete", "edit.undo", "edit.redo", "edit.delete", "layer.delete", "draw.erase_all", "scene.delete", "scene.new",
        "timeline.clear_keyframe", "timeline.copy_frames", "timeline.cut_frames", "timeline.paste_frames", "timeline.remove_frame",
        "plugin.run", "plugin.install", "plugin.remove", "plugin.reload",
    ];
    let mut plain = Vec::new();
    for id in ids {
        for (shape, v) in [("full", &full), ("bare", &bare)] {
            app.status = None;
            app.announce(id, v, 2);
            if let Some(s) = &app.status
                && !s.text.chars().any(pseudo::is_accented)
            {
                plain.push(format!("{id} ({shape}): {:?}", s.text));
            }
        }
    }
    assert!(plain.is_empty(), "status lines in plain English:\n{}", plain.join("\n"));
}

#[test]
fn edit_multiple_frames_status_lines_are_translated() {
    // The Edit menu with Edit Multiple Frames on (ADR 0051), as the status bar shows it.
    let (mut app, _ctx) = pseudo_app(|a, _| a.open_panel(Some("edittransform")));
    let mut plain = Vec::new();
    for id in ["edit.copy", "edit.cut", "edit.paste_in_place", "edit.duplicate", "edit.delete"] {
        app.status = None;
        app.invoke(id);
        match &app.status {
            Some(s) if !s.text.chars().any(pseudo::is_accented) => plain.push(format!("{id}: {:?}", s.text)),
            None => plain.push(format!("{id}: said nothing")),
            _ => {}
        }
    }
    assert!(plain.is_empty(), "status lines in plain English:
{}", plain.join("
"));
}

#[test]
fn tool_status_lines_are_translated() {
    // The Eraser's and Eyedropper's results on the sample (on the sun and the hills, and on
    // empty stage), as the status bar shows them.
    let (mut app, _ctx) = pseudo_app(|_, _| {});
    let mut plain = Vec::new();
    let mut check = |app: &mut crate::app::App, what: &str| {
        if let Some(s) = app.status.take()
            && !s.text.chars().any(pseudo::is_accented)
        {
            plain.push(format!("{what}: {:?}", s.text));
        }
    };
    let sun = anicraft_engine::geom::Vec2::new(1000.0, 160.0);
    let hill = anicraft_engine::geom::Vec2::new(300.0, 600.0);
    let empty = anicraft_engine::geom::Vec2::new(-500.0, -500.0);
    for (p, shift) in [(sun, false), (sun, true), (hill, false), (empty, false)] {
        crate::erasertool::eyedropper(&mut app, p, shift, 1.0);
        check(&mut app, "eyedropper");
    }
    for p in [hill, empty] {
        crate::erasertool::faucet(&mut app, p, 1.0);
        check(&mut app, "faucet");
    }
    crate::erasertool::erase_everything(&mut app);
    check(&mut app, "erase everything");
    crate::erasertool::erase_everything(&mut app);
    check(&mut app, "erase everything again");
    assert!(plain.is_empty(), "tool status lines in plain English:\n{}", plain.join("\n"));
}

#[test]
fn every_menu_item_is_translated() {
    // The menus are built from the command registry when they open; each is drawn here as it
    // opens, in the pseudo-locale, and every item it shows must have been translated.
    let mut found = Vec::new();
    for top in crate::menus::TOP_MENUS {
        let (mut app, ctx) = pseudo_app(|_, _| {});
        let cmds = anicraft_engine::commands();
        let mut update = None;
        for _ in 0..2 {
            let mut out = ctx.run_ui(screen(), |ui| crate::menus::menu_contents(&mut app, ui, top, &cmds));
            out.textures_delta.clear();
            update = out.platform_output.accesskit_update;
        }
        let nodes = update.map(|u| u.nodes).unwrap_or_default();
        assert!(nodes.len() > 2, "the {top} menu drew nothing");
        found.extend(plain_names(top, &app, nodes));
    }
    assert!(found.is_empty(), "menu items not translated:\n{}", found.join("\n"));
}

#[test]
fn status_messages_after_commands_are_translated() {
    // What the status bar says after a command (and the screen reader announces) is ours.
    let (mut app, _ctx) = pseudo_app(|_, _| {});
    let steps: &[(&str, serde_json::Value)] = &[
        ("layer.select", serde_json::json!({"index": 0})),
        ("edit.select_all", serde_json::json!({})),
        ("edit.delete", serde_json::json!({})),
        ("edit.undo", serde_json::json!({})),
        ("edit.redo", serde_json::json!({})),
        ("timeline.clear_keyframe", serde_json::json!({})),
        ("timeline.remove_frame", serde_json::json!({})),
        ("draw.erase_all", serde_json::json!({})),
        ("layer.new", serde_json::json!({})),
        ("layer.delete", serde_json::json!({"confirm": true})),
        ("scene.new", serde_json::json!({})),
        ("scene.prev", serde_json::json!({})),
        ("scene.next", serde_json::json!({})),
        ("scene.delete", serde_json::json!({"confirm": true})),
        ("file.new_sample", serde_json::json!({"which": "interactive"})),
        ("file.new", serde_json::json!({"confirm": true})),
    ];
    let mut plain = Vec::new();
    for (id, params) in steps {
        app.status = None;
        app.perform(id, params.clone());
        match &app.status {
            Some(s) if !s.text.chars().any(pseudo::is_accented) => plain.push(format!("{id}: {:?}", s.text)),
            _ => {}
        }
    }
    assert!(plain.is_empty(), "status lines in plain English:\n{}", plain.join("\n"));
}

#[test]
fn screen_reader_names_are_translated_too() {
    // ADR 0022 names every control; ADR 0040 translates them. In the pseudo-locale every name
    // we wrote is accented, so a plain-English one skipped translation (this catches strings
    // the source guideline can't see: ones passed through variables or built at run time).
    use crate::app::Tool;
    let mut found = untranslated_names("default", |_, _| {});
    for tool in [Tool::Rect, Tool::Star, Tool::Text, Tool::Pencil, Tool::Brush, Tool::Eraser, Tool::PaintBucket, Tool::Eyedropper, Tool::Pen, Tool::Camera, Tool::Bone, Tool::Bind, Tool::Warp] {
        found.extend(untranslated_names(&format!("{tool:?} tool"), |a, _| a.tool = tool));
    }
    // Selections and states that change what Properties, the timeline and the dialogs show.
    found.extend(untranslated_names("shape selected", |a, _| {
        a.run("layer.select", serde_json::json!({"index": 0}));
        a.run("edit.select_all", serde_json::json!({}));
    }));
    found.extend(untranslated_names("text selected", |a, _| {
        a.run("text.create", serde_json::json!({"x": 40.0, "y": 40.0, "text": "Title"}));
        a.run("edit.select_all", serde_json::json!({}));
    }));
    found.extend(untranslated_names("button instance", |a, _| {
        a.perform("file.new_sample", serde_json::json!({"which": "interactive"}));
        if let Some(button) = a.s.doc.library.iter().find(|s| s.kind == anicraft_engine::doc::SymbolKind::Button).map(|s| s.id.0) {
            a.run("library.place", serde_json::json!({"symbol": button, "x": 100.0, "y": 100.0}));
        }
        crate::actions_panel::open(a, crate::actions_panel::Tab::Instance);
    }));
    for style in ["brightness", "tint", "advanced", "alpha"] {
        found.extend(untranslated_names(&format!("colour effect {style}"), |a, _| {
            a.run("layer.select", serde_json::json!({"index": 0}));
            a.run("edit.select_all", serde_json::json!({}));
            a.run("element.set_color_effect", serde_json::json!({"style": style}));
        }));
    }
    found.extend(untranslated_names("close guard", |a, _| {
        a.run("layer.new", serde_json::json!({}));
        a.close_dialog = true;
    }));
    found.extend(untranslated_names("library item", |a, _| {
        a.library_sel = a.s.doc.library.first().map(|s| crate::app::LibraryItem::Symbol(s.id));
    }));
    for panel in ["start", "drop", "droplibrary", "select", "motion", "motionspan", "motionpath", "motionedit", "camera", "sound", "prefs", "shortcuts", "palette", "about", "files", "licenses", "confirm", "recovery", "frames", "filters", "plugins", "actions", "script", "scriptproblems", "testmovie", "testmovieinside", "testscript", "exporthtml", "envelope", "templates", "savetemplate", "droptimeline"] {
        found.extend(untranslated_names(panel, |a, _| a.open_panel(Some(panel))));
    }
    assert!(found.is_empty(), "{} screen-reader names not translated in the pseudo-locale:\n{}", found.len(), found.join("\n"));
}

#[test]
fn the_selection_title_reads_like_the_engines_label_in_english() {
    // panels.rs keeps a translated copy of `anicraft_engine::element_label`; in English the two
    // must say the same, so a change to one shows here.
    let mut s = anicraft_engine::Session::new(anicraft_engine::doc::sample::demo());
    s.execute("edit.select_all", &serde_json::json!({})).ok();
    s.execute("modify.convert_to_symbol", &serde_json::json!({"name": "Clip"})).ok();
    s.execute("text.create", &serde_json::json!({"x": 10.0, "y": 10.0, "text": "A longer line of text than fits\nand a second"})).ok();
    set_language(ENGLISH);
    let doc = &s.doc;
    let mut seen = 0;
    for scene in &doc.scenes {
        for layer in &scene.timeline.layers {
            for k in &layer.keyframes {
                for e in &k.elements {
                    assert_eq!(crate::panels::element_title(doc, e), anicraft_engine::element_label(doc, e));
                    seen += 1;
                }
            }
        }
    }
    assert!(seen > 3, "the sample has elements ({seen})");
}
