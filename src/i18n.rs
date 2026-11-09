//! Interface translations (ADR 0027). Strings in the code stay English and are the lookup keys;
//! the craft-i18n catalogs ([storytold/craft-i18n](https://github.com/storytold/craft-i18n):
//! `locales/<code>/common.tsv`, shared by the Crafting Apps, and `anicraft.tsv`) give the text in
//! the interface language when it is drawn. Lookup, fallback, plurals, language names and the
//! system locale are the `craft-i18n` crate's (craft-libs). `build.rs` embeds the catalogs from
//! `CRAFT_I18N_DIR`; without it the interface is English only. A missing translation is the
//! English itself: nothing here fails.
//!
//! Command ids, document text, file names, the CLI, `serve` and MCP never see translated text.
//!
//! Adding a language: craft-i18n gets `locales/<code>/anicraft.tsv` (its shared `common.tsv`
//! already has many of our strings). Preferences offers it from the next build with that commit.
//!
//! Catalogs stay in logical (typing) order: the egui fork lays right-to-left text out, so
//! nothing here reorders it.

use std::cell::Cell;
use std::sync::LazyLock;

use craft_i18n::I18n;
pub use craft_i18n::Lang;

include!(concat!(env!("OUT_DIR"), "/craft_i18n.rs"));

static I18N: LazyLock<I18n> = LazyLock::new(|| {
    // Every locale with AniCraft's own catalog is offered (`build.rs` embeds no other): a
    // language is added in craft-i18n alone. The interface reads right to left for the languages
    // that do, so none has to wait for its layout.
    let offered: Vec<&str> = CRAFT_I18N.iter().map(|(code, ..)| *code).collect();
    let (i18n, problems) = I18n::new(CRAFT_I18N, &offered);
    for p in problems {
        eprintln!("interface catalogs: {p}");
    }
    i18n
});

/// The `language` preference that follows the system's language.
pub const AUTO: &str = "auto";

thread_local! {
    // Per thread: tests draw on threads of their own and must not change each other's language.
    static CURRENT: Cell<Lang> = const { Cell::new(Lang::EN) };
}

/// The language the interface is drawn in; the shell sets it from the preference every frame.
pub fn set_current(lang: Lang) {
    CURRENT.set(lang);
}

pub fn current() -> Lang {
    CURRENT.get()
}

/// The `language` preference as a language: an offered code, else ([`AUTO`], or a code this
/// build doesn't have) the system's language, which is English when none fits.
pub fn from_pref(pref: &str) -> Lang {
    I18N.from_code(pref).unwrap_or_else(system_lang)
}

fn system_lang() -> Lang {
    // Tests find widgets by their English labels whatever the machine's language is.
    if cfg!(test) { Lang::EN } else { I18N.system_lang() }
}

/// The languages to offer in Preferences: English first, then those whose catalogs this build has.
pub fn languages() -> Vec<Lang> {
    I18N.languages()
}

/// A language's code (`en`, `ar`), as the preference stores it.
pub fn code(lang: Lang) -> &'static str {
    I18N.code(lang)
}

/// A language's name in itself (`العربية`), for the picker.
pub fn name(lang: Lang) -> &'static str {
    I18N.name(lang)
}

/// Whether the interface language is written right to left.
pub fn rtl() -> bool {
    I18N.rtl(current())
}

/// `source` in the interface language.
pub fn t(source: &str) -> &str {
    I18N.t(current(), source)
}

/// A command's label by its id (`@id` rows), else the English label's translation.
pub fn t_id<'a>(id: &str, label: &'a str) -> &'a str {
    I18N.tid(current(), id, label)
}

/// A count's wording (`{n}` filled in): `one` and `other` are the English forms and the key.
pub fn trn(n: u64, one: &str, other: &str) -> String {
    I18N.trn(current(), n, one, other)
}

/// `template` in the interface language with its `{name}` placeholders filled in one pass, so a
/// value containing `{n}` stays as written. Values in another direction are isolated.
pub fn fmt(template: &str, args: &[(&str, &str)]) -> String {
    craft_i18n::fmt(t(template), args)
}

/// Whether this build embeds the catalogs (`CRAFT_I18N_DIR` was set). Tests that need a
/// translation skip without them.
#[cfg(test)]
pub fn embedded() -> bool {
    !CRAFT_I18N.is_empty()
}

/// `tl!("Save")`: the text in the interface language.
macro_rules! tl {
    ($s:expr) => {
        $crate::i18n::t($s)
    };
}
pub(crate) use tl;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_and_unknown_codes_follow_the_system() {
        // Tests run in English whatever the machine's language; a code from a newer version
        // follows the system's language too.
        for pref in [AUTO, "xx-unknown", "", "en"] {
            assert_eq!(code(from_pref(pref)), "en", "{pref:?}");
        }
        assert!(!rtl());
        assert_eq!(t("Timeline"), "Timeline");
    }

    #[test]
    fn arabic_is_offered_and_reads_right_to_left() {
        if !embedded() {
            eprintln!("skipped: built without CRAFT_I18N_DIR");
            return;
        }
        let ar = languages().into_iter().find(|l| code(*l) == "ar");
        assert!(ar.is_some(), "a craft-i18n checkout with locales/ar/anicraft.tsv offers Arabic");
        let Some(ar) = ar else { return };
        assert_eq!(from_pref("AR"), ar, "codes are matched in any case");
        assert_eq!(name(ar), "العربية");
        set_current(ar);
        assert!(rtl());
        assert_ne!(t("Timeline"), "Timeline");
        // Placeholders are filled after the lookup, and a count picks its form.
        assert!(!fmt("Saved {path}", &[("path", "a.anicraft")]).starts_with("Saved"));
        assert!(fmt("Saved {path}", &[("path", "a.anicraft")]).contains("a.anicraft"));
        assert!(trn(2, "{n} frame", "{n} frames").contains('2'));
        // Hand-painted widgets mirror; rows run from the right.
        let r = egui::Rect::from_min_max(egui::pos2(100.0, 0.0), egui::pos2(300.0, 20.0));
        assert_eq!(crate::theme::mirror_x(r, 110.0), 290.0);
        assert_eq!(crate::theme::mirror(r, egui::Rect::from_min_max(egui::pos2(100.0, 0.0), egui::pos2(103.0, 20.0))).min.x, 297.0);
        assert_eq!(crate::theme::start_row(egui::Align::Center).main_dir, egui::Direction::RightToLeft);
        set_current(Lang::EN);
        assert_eq!(crate::theme::mirror_x(r, 110.0), 110.0, "left to right leaves it");
    }
}
