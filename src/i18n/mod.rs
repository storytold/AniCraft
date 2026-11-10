//! Interface languages (ADR 0040, docs/localisation.md).
//!
//! Strings stay English in the code and are the lookup keys, as in craft-i18n's catalogs:
//! `tr!("Save")` is the text in the interface language, `tr!("Saved {path}", path = p)` fills
//! placeholders after the lookup, `trn!("{n} frame", "{n} frames", n)` picks a plural form by
//! CLDR rules, `trc!("menu", "Open")` tells two meanings of the same English apart, and
//! `tr_noop!("Graphite")` marks a string in a table for the extractor, to be translated where it
//! is drawn with [`t`]. A missing translation is the English itself: nothing here fails.
//!
//! Catalogs come from the Craft family's shared craft-i18n repository, embedded at build time
//! from `CRAFT_I18N_DIR` (build.rs); without it the interface is English, plus the pseudo-locale
//! that tests layout. Command ids, document text, file names, the CLI, `serve` and MCP never see
//! translated text; menus translate the registry's labels when they are drawn.

pub mod catalog;
pub mod plural;
pub mod pseudo;
#[cfg(test)]
pub mod scan;

use std::borrow::Cow;
use std::cell::Cell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{LazyLock, Mutex};

use catalog::Catalog;

// `CRAFT_I18N: &[(&str, &str, &str)]`: (locale, its shared `common.tsv`, AniCraft's
// `anicraft.tsv`), `""` for a missing file; empty without `CRAFT_I18N_DIR`.
include!(concat!(env!("OUT_DIR"), "/craft_i18n.rs"));

/// The `language` preference that follows the system's language.
pub const AUTO: &str = "auto";
pub const ENGLISH: &str = "en";
/// Accented, longer English (`pseudo.rs`), for checking layout and missed strings.
pub const PSEUDO: &str = "pseudo";

/// Languages the interface can be drawn in today: written left to right, in scripts the bundled
/// faces cover and egui lays out a letter at a time (Latin, Greek, Cyrillic). A catalog for any
/// other locale is embedded but not offered (docs/localisation.md › Scripts and direction).
/// A regional variant (`es-mx`) is ready when its language is.
pub const READY: &[&str] = &[
    "az", "bg", "bs", "ca", "cs", "da", "de", "el", "es", "et", "fi", "fr", "gl", "hr", "hu", "id", "it", "kk", "lt", "lv", "nb", "nl", "pl", "pt", "pt-br", "ro", "ru", "sk", "sl", "sq", "sr", "sr-latn", "sv", "tr", "uk",
];

enum Kind {
    English,
    Pseudo,
    Catalog(Catalog),
}

struct Language {
    code: String,
    name: String,
    kind: Kind,
}

/// The last subtag is a region (`br`, `419`): the locale falls back to its base language.
fn regional_base(code: &str) -> Option<&str> {
    let (base, region) = code.rsplit_once('-')?;
    let is_region = (region.len() == 2 && region.chars().all(|c| c.is_ascii_alphabetic())) || (region.len() == 3 && region.chars().all(|c| c.is_ascii_digit()));
    is_region.then_some(base)
}

fn ready(code: &str, ready: &[&str]) -> bool {
    ready.contains(&code) || regional_base(code).is_some_and(|b| ready.contains(&b))
}

/// Every language this build offers, English first and the pseudo-locale last.
pub struct Languages {
    list: Vec<Language>,
}

impl Languages {
    /// From embedded `(locale, common, app)` catalogs. A locale is offered when it has its own
    /// AniCraft catalog and is [`READY`]; one with only shared strings, or a base language
    /// embedded for its regional variants' sake, is a fallback only. Returns the skipped rows
    /// too, as `locale/file:line: why`.
    pub fn new(embedded: &[(&str, &str, &str)], ready_codes: &[&str]) -> (Languages, Vec<String>) {
        let mut problems = Vec::new();
        let mut parsed: HashMap<String, (Vec<catalog::Row>, Vec<catalog::Row>, bool)> = HashMap::new();
        for (code, common, app) in embedded {
            let code = code.to_ascii_lowercase();
            let mut read = |file: &str, text: &str| {
                let (rows, bad) = catalog::parse(text);
                problems.extend(bad.into_iter().map(|p| format!("{code}/{file}:{}: {}", p.line, p.why)));
                rows
            };
            let entry = (read("common.tsv", common), read("anicraft.tsv", app), !app.is_empty());
            parsed.insert(code, entry);
        }
        let mut list = vec![Language { code: ENGLISH.into(), name: "English".into(), kind: Kind::English }];
        let mut codes: Vec<&String> = parsed.keys().filter(|c| parsed.get(*c).is_some_and(|e| e.2) && ready(c, ready_codes)).collect();
        codes.sort();
        for code in codes {
            // pt-br: pt shared, pt app, pt-br shared, pt-br app (each later one wins), so an app
            // row overrides the shared one in its own locale only.
            let mut cat = Catalog::default();
            let mut chain = vec![code.as_str()];
            if let Some(base) = regional_base(code) {
                chain.insert(0, base);
            }
            for c in chain {
                if let Some((common, app, _)) = parsed.get(c) {
                    cat.merge(common.clone());
                    cat.merge(app.clone());
                }
            }
            let name = cat.id("language.name").unwrap_or(code).to_string();
            list.push(Language { code: code.clone(), name, kind: Kind::Catalog(cat) });
        }
        list.push(Language { code: PSEUDO.into(), name: "Pseudo-locale (layout test)".into(), kind: Kind::Pseudo });
        (Languages { list }, problems)
    }

    fn get(&self, i: usize) -> &Language {
        // Index 0 is always English.
        self.list.get(i).or(self.list.first()).unwrap_or(&ENGLISH_LANGUAGE)
    }

    fn find(&self, code: &str) -> Option<usize> {
        let code = code.trim().to_ascii_lowercase().replace('_', "-");
        self.list.iter().position(|l| l.code == code)
    }

    /// The language a preference stands for: an offered code, else ([`AUTO`], or a code this
    /// build doesn't have) the system's language, which is English when none fits.
    fn resolve(&self, pref: &str, system: Option<&str>) -> usize {
        if !pref.eq_ignore_ascii_case(AUTO)
            && let Some(i) = self.find(pref)
        {
            return i;
        }
        system.and_then(|s| self.match_system(s)).unwrap_or(0)
    }

    /// The best offered language for a system locale (`de-AT`, `pt_BR.UTF-8`, `zh-Hans-CN`).
    fn match_system(&self, locale: &str) -> Option<usize> {
        let locale = locale.split(['.', '@']).next().unwrap_or(locale).to_ascii_lowercase().replace('_', "-");
        let tags: Vec<&str> = locale.split('-').filter(|t| !t.is_empty()).collect();
        let lang = *tags.first()?;
        let mut candidates = vec![locale.clone()];
        if let Some(second) = tags.get(1) {
            candidates.push(format!("{lang}-{second}"));
        }
        if lang == "zh" {
            let traditional = tags.iter().any(|t| matches!(*t, "hant" | "tw" | "hk" | "mo"));
            candidates.push(if traditional { "zh-hant".into() } else { "zh-hans".into() });
        }
        candidates.push(lang.to_string());
        // The pseudo-locale is never the system's.
        candidates.iter().filter_map(|c| self.find(c)).find(|i| self.get(*i).code != PSEUDO)
    }

    #[cfg(test)]
    fn lookup<'a>(&'a self, i: usize, context: &str, source: &'a str) -> Option<&'a str> {
        match &self.get(i).kind {
            Kind::Catalog(c) => c.get(context, source),
            _ => None,
        }
    }

    fn plural<'a>(&'a self, i: usize, one: &'a str, other: &'a str, n: u64) -> &'a str {
        let lang = self.get(i);
        let english = if n == 1 { one } else { other };
        let Kind::Catalog(c) = &lang.kind else { return english };
        match c.plural(one, other) {
            Some(forms) if forms.len() == plural::forms(&lang.code).len() => forms.get(plural::index(&lang.code, n)).map_or(english, String::as_str),
            // A row with the wrong number of forms is skipped (English), as craft-i18n's apps do.
            _ => english,
        }
    }
}

/// Stands in for the list's English entry, which `Languages::new` always makes.
static ENGLISH_LANGUAGE: Language = Language { code: String::new(), name: String::new(), kind: Kind::English };

static LANGUAGES: LazyLock<Languages> = LazyLock::new(|| {
    let (languages, problems) = Languages::new(CRAFT_I18N, READY);
    for p in problems {
        eprintln!("interface catalog: {p} (shown in English)");
    }
    languages
});

/// The interface language, an index into [`LANGUAGES`].
static CURRENT: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    // Tests draw on threads of their own: each picks its language without changing the
    // others' (`set_language` writes here under `cfg(test)`).
    static OVERRIDE: Cell<Option<usize>> = const { Cell::new(None) };
}

fn current() -> usize {
    OVERRIDE.with(Cell::get).unwrap_or_else(|| CURRENT.load(Ordering::Relaxed))
}

fn set_current(i: usize) {
    if cfg!(test) {
        OVERRIDE.with(|o| o.set(Some(i)));
    } else {
        CURRENT.store(i, Ordering::Relaxed);
    }
}

/// `--lang`: the language for this run, over the preference (not saved).
static FORCED: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Holds the interface language at `code` for the rest of the run, whatever the preference
/// says (`--lang`; set once, before the window opens).
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub fn force(code: Option<&str>) {
    if let Some(c) = code {
        let _ = FORCED.set(c.to_string());
        set_language(c);
    }
}

/// Sets the interface language from the `language` preference and returns the code it
/// resolved to (`--lang`, when given, wins). Takes effect on the next string drawn, so the
/// same frame.
pub fn set_language(pref: &str) -> &'static str {
    let system = if cfg!(test) { None } else { crate::platform::system_locale() };
    let pref = FORCED.get().map_or(pref, String::as_str);
    let i = LANGUAGES.resolve(pref, system.as_deref());
    set_current(i);
    code()
}

/// The interface language's code (`en`, `de`, `pseudo`).
pub fn code() -> &'static str {
    LANGUAGES.get(current()).code.as_str()
}

/// Whether a `--lang` / preference value names a language this build has (or is [`AUTO`]).
pub fn known(pref: &str) -> bool {
    pref.eq_ignore_ascii_case(AUTO) || LANGUAGES.find(pref).is_some()
}

/// `(code, name in that language)` for the Preferences picker, English first. The
/// pseudo-locale is listed in debug builds and while it is in use.
pub fn offered() -> Vec<(&'static str, &'static str)> {
    let now = current();
    LANGUAGES.list.iter().enumerate().filter(|(i, l)| !matches!(l.kind, Kind::Pseudo) || cfg!(debug_assertions) || *i == now).map(|(_, l)| (l.code.as_str(), l.name.as_str())).collect()
}

/// The name of the language [`AUTO`] picks on this computer, for the picker's first entry.
pub fn system_choice() -> &'static str {
    let system = if cfg!(test) { None } else { crate::platform::system_locale() };
    LANGUAGES.get(LANGUAGES.resolve(AUTO, system.as_deref())).name.as_str()
}

/// A language's own name, for the picker (the code itself when unknown).
pub fn name_of(code: &str) -> Cow<'static, str> {
    LANGUAGES.find(code).map_or_else(|| Cow::Owned(code.to_string()), |i| Cow::Borrowed(LANGUAGES.get(i).name.as_str()))
}

// ----- lookups ----------------------------------------------------------------------------------

/// Pseudo-locale strings, made once each. Keys are the program's own `'static` strings, so the
/// cache is bounded by the strings the program has.
static PSEUDO_CACHE: LazyLock<Mutex<HashMap<&'static str, &'static str>>> = LazyLock::new(Default::default);

fn pseudo_static(source: &'static str) -> &'static str {
    let mut cache = PSEUDO_CACHE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    cache.entry(source).or_insert_with(|| Box::leak(pseudo::transform(source).into_boxed_str()))
}

impl Languages {
    /// `source` in language `i` (English is the key itself: no lookup).
    fn text<'a>(&'a self, i: usize, context: &str, source: &'static str) -> &'a str {
        match &self.get(i).kind {
            Kind::English => source,
            Kind::Pseudo => pseudo_static(source),
            Kind::Catalog(c) => c.get(context, source).unwrap_or(source),
        }
    }

    /// Run-time English in language `i`, translated on an exact match.
    fn text_dyn<'a>(&'a self, i: usize, source: &'a str) -> Cow<'a, str> {
        match &self.get(i).kind {
            Kind::English => Cow::Borrowed(source),
            Kind::Pseudo => Cow::Owned(pseudo::transform(source)),
            Kind::Catalog(c) => Cow::Borrowed(c.get("", source).unwrap_or(source)),
        }
    }

    /// A command's label in language `i`: its `@id` row, else its label's row.
    fn command<'a>(&'a self, i: usize, id: &str, label: &'static str) -> &'a str {
        match &self.get(i).kind {
            Kind::Catalog(c) => c.id(id).unwrap_or_else(|| c.get("", label).unwrap_or(label)),
            _ => self.text(i, "", label),
        }
    }

    /// The plural form for `n` in language `i`.
    fn count<'a>(&'a self, i: usize, one: &'static str, other: &'static str, n: u64) -> &'a str {
        match self.get(i).kind {
            Kind::English => {
                if n == 1 { one } else { other }
            }
            Kind::Pseudo => pseudo_static(if n == 1 { one } else { other }),
            Kind::Catalog(_) => self.plural(i, one, other, n),
        }
    }
}

fn translate(context: &'static str, source: &'static str) -> &'static str {
    let languages: &'static Languages = &LANGUAGES;
    languages.text(current(), context, source)
}

/// `source` in the interface language. Use [`tr!`] for literals; this is for `'static` strings
/// from tables marked with [`tr_noop!`] (tool names, menu titles, the registry's labels).
pub fn t(source: &'static str) -> &'static str {
    translate("", source)
}

/// `source` with a context word, falling back to its plain translation.
pub fn tc(context: &'static str, source: &'static str) -> &'static str {
    translate(context, source)
}

/// English that arrives at run time from below the shell (an undo step's label, a
/// confirmation's title): translated when a catalog has that exact text.
pub fn t_dyn(source: &str) -> Cow<'_, str> {
    let languages: &'static Languages = &LANGUAGES;
    languages.text_dyn(current(), source)
}

/// A registry command's label: an `@id` row for its id first, else the label's own translation.
pub fn t_cmd(id: &str, label: &'static str) -> &'static str {
    let languages: &'static Languages = &LANGUAGES;
    languages.command(current(), id, label)
}

/// The plural form for `n`, placeholders not yet filled (see [`trn!`]).
pub fn tn(one: &'static str, other: &'static str, n: u64) -> &'static str {
    let languages: &'static Languages = &LANGUAGES;
    languages.count(current(), one, other, n)
}

/// `template` with its `{name}` placeholders filled from `args`, in one pass: a value that
/// itself contains `{name}` stays as written. A placeholder without a value stays too.
pub fn fill(template: &str, args: &[(&str, &dyn std::fmt::Display)]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(rest.get(..open).unwrap_or(""));
        let after = rest.get(open + 1..).unwrap_or("");
        let value = after.find('}').and_then(|close| {
            let name = after.get(..close)?;
            args.iter().find(|(k, _)| *k == name).map(|(_, v)| (*v, close))
        });
        match value {
            Some((v, close)) => {
                let _ = write!(out, "{v}");
                rest = after.get(close + 1..).unwrap_or("");
            }
            None => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// `tr!("Save")`: the literal in the interface language (`&'static str`).
/// `tr!("Saved {path}", path = p)`: the same with placeholders filled (`String`).
/// Only literals: the extractor (`scripts/i18n-extract.sh`) finds every key in the source.
macro_rules! tr {
    ($s:literal) => {
        $crate::i18n::t($s)
    };
    ($s:literal, $($k:ident = $v:expr),+ $(,)?) => {
        $crate::i18n::fill($crate::i18n::t($s), &[$((stringify!($k), &$v as &dyn std::fmt::Display)),+])
    };
}

/// `trc!("menu", "Open")`: a literal with a context word (craft-i18n's first column).
macro_rules! trc {
    ($c:literal, $s:literal) => {
        $crate::i18n::tc($c, $s)
    };
}

/// `trn!("{n} frame", "{n} frames", count)`: the plural form for `count`, `{n}` filled with
/// it; more placeholders as in [`tr!`]. `String`.
macro_rules! trn {
    ($one:literal, $other:literal, $n:expr $(, $k:ident = $v:expr)* $(,)?) => {{
        let n = $n;
        let count = u64::try_from(n).unwrap_or(u64::MAX);
        $crate::i18n::fill($crate::i18n::tn($one, $other, count), &[("n", &n as &dyn std::fmt::Display) $(, (stringify!($k), &$v as &dyn std::fmt::Display))*])
    }};
}

/// `tr_noop!("Graphite")`: the literal unchanged, marked for the extractor. For strings in
/// tables that are translated where they are drawn, with [`t`].
macro_rules! tr_noop {
    ($s:literal) => {
        $s
    };
}

// After the macros: `macro_rules!` are in scope only below their definition.
#[cfg(test)]
mod tests;
