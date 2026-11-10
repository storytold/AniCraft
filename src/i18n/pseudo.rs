//! The pseudo-locale: English made to look foreign, to test the interface without a real
//! translation. Letters take accents (text that skips translation stays plain and stands out),
//! every string grows by about a third (German and Finnish run that much longer than English)
//! and is bracketed (a cut-off label loses its `]`). `{name}` placeholders are kept, so values
//! still fill in. Only Latin-1 and Latin Extended-A letters are used: every bundled face has them.

/// The accented form of an ASCII letter (itself when Latin Extended-A has none).
fn accent(c: char) -> char {
    match c {
        'a' => 'à',
        'c' => 'ç',
        'd' => 'ď',
        'e' => 'é',
        'g' => 'ğ',
        'h' => 'ĥ',
        'i' => 'î',
        'j' => 'ĵ',
        'k' => 'ķ',
        'l' => 'ľ',
        'n' => 'ñ',
        'o' => 'ö',
        'r' => 'ŕ',
        's' => 'š',
        't' => 'ţ',
        'u' => 'ü',
        'w' => 'ŵ',
        'y' => 'ý',
        'z' => 'ž',
        'A' => 'Å',
        'C' => 'Ç',
        'D' => 'Ď',
        'E' => 'É',
        'G' => 'Ğ',
        'H' => 'Ĥ',
        'I' => 'Î',
        'J' => 'Ĵ',
        'K' => 'Ķ',
        'L' => 'Ľ',
        'N' => 'Ñ',
        'O' => 'Ö',
        'R' => 'Ŕ',
        'S' => 'Š',
        'T' => 'Ţ',
        'U' => 'Ü',
        'W' => 'Ŵ',
        'Y' => 'Ý',
        'Z' => 'Ž',
        other => other,
    }
}

/// Whether `c` is one of the accented letters [`transform`] writes (the audits' test).
#[cfg(test)]
pub fn is_accented(c: char) -> bool {
    !c.is_ascii() && ('a'..='z').chain('A'..='Z').any(|a| accent(a) == c)
}

/// `Save` → `[Šàvé ~~]`; `Saved {path}` → `[Šàvéď {path} ~~~~]`. Line breaks are kept.
pub fn transform(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2 + 4);
    out.push('[');
    let mut letters = 0usize;
    let mut in_placeholder = false;
    for c in s.chars() {
        match c {
            '{' => in_placeholder = true,
            '}' => in_placeholder = false,
            _ => {}
        }
        if in_placeholder || !c.is_ascii_alphabetic() {
            out.push(c);
        } else {
            letters += 1;
            out.push(accent(c));
        }
    }
    // About a third longer, at least one mark, and never absurdly long for a paragraph.
    let pad = (letters.div_ceil(3)).clamp(1, 40);
    out.push(' ');
    out.extend(std::iter::repeat_n('~', pad));
    out.push(']');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_take_accents_and_placeholders_survive() {
        assert_eq!(transform("Save"), "[Šàvé ~~]");
        assert_eq!(transform("Saved {path}"), "[Šàvéď {path} ~~]");
        assert_eq!(transform("{n} frames"), "[{n} fŕàméš ~~]");
        assert!(transform("A\nB").contains('\n'));
        assert!(transform("Export…").ends_with("~~]") && transform("Export…").contains('…'));
        assert!(transform("Save").chars().any(is_accented));
        assert!(!"Save".chars().any(is_accented));
        assert!(!"ß".chars().any(is_accented), "only our own marks count");
        // Every letter we write is in Latin-1 Supplement or Latin Extended-A (U+00C0..U+017F).
        for c in ('a'..='z').chain('A'..='Z').map(accent).filter(|c| !c.is_ascii()) {
            assert!(('\u{c0}'..='\u{17f}').contains(&c), "{c}");
        }
        // A paragraph grows, but not without bound.
        assert!(transform(&"word ".repeat(200)).len() < "word ".repeat(200).len() * 2 + 50);
    }
}
