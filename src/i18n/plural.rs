//! CLDR cardinal plural rules for whole numbers, written from the public Unicode CLDR
//! `plurals.xml` rules (Unicode-3.0 data; credited in ATTRIBUTION.md). Only what a count of
//! things needs: integers, no fractions, so the operands `v`, `f` and `t` are always 0.
//!
//! A craft-i18n `@plural` row lists a language's forms in CLDR order (`zero`, `one`, `two`,
//! `few`, `many`, `other`), leaving out those whole numbers from 0 to 1000 never reach
//! (`docs/catalog-format.md` there). [`forms`] gives that list, [`index`] the form a count takes.

/// A CLDR plural category.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Zero,
    One,
    Two,
    Few,
    Many,
    Other,
}

use Category::{Few, Many, One, Other, Two, Zero};

/// The rule family a language follows (several languages share each).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Rule {
    /// No plural forms: Japanese, Chinese, Korean, Thai, Vietnamese, Indonesian, Lao.
    None,
    /// `one` for exactly 1: English, German, Spanish, Italian, the Nordic languages, Greek, …
    OneIsOne,
    /// `one` for 0 and 1: French, Brazilian Portuguese, Hindi, Bengali, Persian.
    OneIsZeroOrOne,
    /// Russian, Ukrainian: one, few, many.
    EastSlavic,
    /// Serbian, Croatian, Bosnian: one, few, other.
    SouthSlavic,
    /// Polish: one, few, many.
    Polish,
    /// Czech, Slovak: one, few, other (their `many` is for fractions).
    CzechSlovak,
    /// Lithuanian: one, few, other (its `many` is for fractions).
    Lithuanian,
    /// Latvian: zero, one, other.
    Latvian,
    /// Romanian: one, few, other.
    Romanian,
    /// Slovenian: one, two, few, other.
    Slovenian,
    /// Hebrew: one, two, other.
    Hebrew,
    /// Arabic: zero, one, two, few, many, other.
    Arabic,
}

/// The language part of a locale code: `pt-br` → `pt`, `sr-latn` → `sr`.
fn language(code: &str) -> &str {
    code.split(['-', '_']).next().unwrap_or(code)
}

fn rule(code: &str) -> Rule {
    match language(&code.to_ascii_lowercase()) {
        "ja" | "zh" | "ko" | "th" | "vi" | "id" | "lo" | "ms" | "my" | "km" => Rule::None,
        "fr" | "pt" | "hi" | "bn" | "fa" | "gu" | "kn" | "zu" | "am" | "as" => {
            // European Portuguese counts only 1 as `one`.
            if code.eq_ignore_ascii_case("pt-pt") { Rule::OneIsOne } else { Rule::OneIsZeroOrOne }
        }
        "ru" | "uk" | "be" => Rule::EastSlavic,
        "sr" | "hr" | "bs" | "sh" => Rule::SouthSlavic,
        "pl" => Rule::Polish,
        "cs" | "sk" => Rule::CzechSlovak,
        "lt" => Rule::Lithuanian,
        "lv" => Rule::Latvian,
        "ro" | "mo" => Rule::Romanian,
        "sl" => Rule::Slovenian,
        "he" | "iw" => Rule::Hebrew,
        "ar" => Rule::Arabic,
        // English and most others: one for 1 (also the guess for a language not listed here).
        _ => Rule::OneIsOne,
    }
}

/// The category whole number `n` takes in the language `code`.
pub fn category(code: &str, n: u64) -> Category {
    let (m10, m100) = (n % 10, n % 100);
    match rule(code) {
        Rule::None => Other,
        Rule::OneIsOne => {
            if n == 1 { One } else { Other }
        }
        Rule::OneIsZeroOrOne => {
            if n <= 1 { One } else { Other }
        }
        Rule::EastSlavic => {
            if m10 == 1 && m100 != 11 {
                One
            } else if (2..=4).contains(&m10) && !(12..=14).contains(&m100) {
                Few
            } else {
                Many
            }
        }
        Rule::SouthSlavic => {
            if m10 == 1 && m100 != 11 {
                One
            } else if (2..=4).contains(&m10) && !(12..=14).contains(&m100) {
                Few
            } else {
                Other
            }
        }
        Rule::Polish => {
            if n == 1 {
                One
            } else if (2..=4).contains(&m10) && !(12..=14).contains(&m100) {
                Few
            } else {
                Many
            }
        }
        Rule::CzechSlovak => match n {
            1 => One,
            2..=4 => Few,
            _ => Other,
        },
        Rule::Lithuanian => {
            if m10 == 1 && !(11..=19).contains(&m100) {
                One
            } else if (2..=9).contains(&m10) && !(11..=19).contains(&m100) {
                Few
            } else {
                Other
            }
        }
        Rule::Latvian => {
            if m10 == 0 || (11..=19).contains(&m100) {
                Zero
            } else if m10 == 1 && m100 != 11 {
                One
            } else {
                Other
            }
        }
        Rule::Romanian => {
            if n == 1 {
                One
            } else if n == 0 || (1..=19).contains(&m100) {
                Few
            } else {
                Other
            }
        }
        Rule::Slovenian => match m100 {
            1 => One,
            2 => Two,
            3 | 4 => Few,
            _ => Other,
        },
        Rule::Hebrew => match n {
            1 => One,
            2 => Two,
            _ => Other,
        },
        Rule::Arabic => match (n, m100) {
            (0, _) => Zero,
            (1, _) => One,
            (2, _) => Two,
            (_, 3..=10) => Few,
            (_, 11..=99) => Many,
            _ => Other,
        },
    }
}

/// The forms a catalog lists for `code`, in CLDR order: the categories 0 to 1000 reach.
pub fn forms(code: &str) -> &'static [Category] {
    match rule(code) {
        Rule::None => &[Other],
        Rule::OneIsOne | Rule::OneIsZeroOrOne => &[One, Other],
        Rule::EastSlavic | Rule::Polish => &[One, Few, Many],
        Rule::SouthSlavic | Rule::CzechSlovak | Rule::Lithuanian | Rule::Romanian => &[One, Few, Other],
        Rule::Latvian => &[Zero, One, Other],
        Rule::Slovenian => &[One, Two, Few, Other],
        Rule::Hebrew => &[One, Two, Other],
        Rule::Arabic => &[Zero, One, Two, Few, Many, Other],
    }
}

/// Which of a catalog's forms (`forms(code)`) the count `n` takes. A category the list leaves
/// out (Spanish `many` at a million) uses the last form, `other`'s place.
pub fn index(code: &str, n: u64) -> usize {
    let list = forms(code);
    let c = category(code, n);
    list.iter().position(|f| *f == c).unwrap_or(list.len().saturating_sub(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The categories 0..=1000 reach, in CLDR order, for a language.
    fn reached(code: &str) -> Vec<Category> {
        let all = [Zero, One, Two, Few, Many, Other];
        all.into_iter().filter(|c| (0..=1000).any(|n| category(code, n) == *c)).collect()
    }

    #[test]
    fn form_lists_are_exactly_the_categories_whole_numbers_reach() {
        // The craft-i18n checker counts forms the same way; a mismatch would make every plural
        // row of that language fall back to English.
        for code in ["en", "de", "fr", "pt-br", "pt-pt", "es", "ja", "zh-hans", "ru", "uk", "sr", "sr-latn", "hr", "bs", "pl", "cs", "sk", "lt", "lv", "ro", "sl", "he", "ar", "hi", "xx"] {
            assert_eq!(reached(code), forms(code).to_vec(), "{code}");
        }
        // The counts craft-i18n's docs name: 1 Japanese, 2 French, 3 Russian, 6 Arabic.
        assert_eq!([forms("ja").len(), forms("fr").len(), forms("ru").len(), forms("ar").len()], [1, 2, 3, 6]);
    }

    #[test]
    fn counts_pick_their_forms() {
        assert_eq!([0, 1, 2].map(|n| category("en", n)), [Other, One, Other]);
        assert_eq!([0, 1, 2].map(|n| category("fr", n)), [One, One, Other]);
        assert_eq!([1, 2, 5, 11, 21, 22, 25, 112].map(|n| category("ru", n)), [One, Few, Many, Many, One, Few, Many, Many]);
        assert_eq!([1, 2, 5, 12, 22, 25].map(|n| category("pl", n)), [One, Few, Many, Many, Few, Many]);
        assert_eq!([1, 3, 5].map(|n| category("cs", n)), [One, Few, Other]);
        assert_eq!([0, 1, 2, 3, 11, 100, 102].map(|n| category("ar", n)), [Zero, One, Two, Few, Many, Other, Other]);
        assert_eq!([1, 2, 3, 5, 101, 102].map(|n| category("sl", n)), [One, Two, Few, Other, One, Two]);
        assert_eq!([0, 1, 2, 19, 20, 101].map(|n| category("ro", n)), [Few, One, Few, Few, Other, Few]);
        assert_eq!([0, 1, 11, 21].map(|n| category("lv", n)), [Zero, One, Zero, One]);
        // Huge counts never overflow or panic.
        assert_eq!(category("ar", u64::MAX), Many);
        assert_eq!(index("es", 1_000_000), 1, "Spanish `many` falls to `other`");
        assert_eq!(index("ja", 1), 0);
        assert_eq!(index("ru", 5), 2);
    }
}
