//! A small Rust lexer for the shell's sources (tests only): finds every string literal, with
//! what comes before and after it, so one pass serves both the guideline that interface text
//! is translated (`tests.rs`) and the extractor that lists strings for translators.
//!
//! It knows comments, strings with escapes and line continuations, raw strings, byte strings,
//! and char literals versus lifetimes; it never panics on odd input.

/// A string literal in the source.
#[derive(Clone, Debug)]
pub struct Literal {
    /// 1-based line of the opening quote.
    pub line: usize,
    /// The literal's value (escapes resolved).
    pub value: String,
    /// The code between the previous literal (or the start of the file) and this one, with
    /// comments left out.
    pub before: String,
    /// The rest of the line after the closing quote.
    pub after: String,
    /// The line up to the opening quote.
    pub line_before: String,
    /// A byte string (`b"…"`): never interface text.
    pub bytes: bool,
}

/// How a literal reaches the interface, from the macro around it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// `tr!("…")`, `tr_noop!("…")`: a source text.
    Source,
    /// `trc!("menu", …)`: the context word.
    Context,
    /// `trc!(…, "Open")`: a source text with the context before it.
    ContextSource,
    /// `trn!("{n} frame", …)`: a plural's `one` form.
    One,
    /// `trn!(…, "{n} frames", n)`: a plural's `other` form.
    Other,
    /// Not in a translation macro.
    Plain,
}

/// Every literal in `src`, in order.
pub fn literals(src: &str) -> Vec<Literal> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut before = String::new();
    let mut line = 1usize;
    let mut line_start = 0usize;
    let mut i = 0usize;
    let at = |k: usize| chars.get(k).copied().unwrap_or('\0');
    let slice = |a: usize, b: usize| chars.get(a..b.min(chars.len())).map(|s| s.iter().collect::<String>()).unwrap_or_default();
    let rest_of_line = |from: usize| {
        let end = (from..chars.len()).find(|k| at(*k) == '\n').unwrap_or(chars.len());
        slice(from, end)
    };
    while i < chars.len() {
        let c = at(i);
        // Comments.
        if c == '/' && at(i + 1) == '/' {
            while i < chars.len() && at(i) != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && at(i + 1) == '*' {
            let mut depth = 0usize;
            while i < chars.len() {
                if at(i) == '/' && at(i + 1) == '*' {
                    depth += 1;
                    i += 2;
                } else if at(i) == '*' && at(i + 1) == '/' {
                    depth = depth.saturating_sub(1);
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    if at(i) == '\n' {
                        line += 1;
                        line_start = i + 1;
                    }
                    i += 1;
                }
            }
            continue;
        }
        let ident_before = i > 0 && (at(i - 1).is_alphanumeric() || at(i - 1) == '_');
        // Raw strings: r"…", r#"…"#, br"…".
        let raw_start = if c == 'r' && !ident_before && (at(i + 1) == '"' || at(i + 1) == '#') {
            Some((i + 1, false))
        } else if c == 'b' && !ident_before && at(i + 1) == 'r' && (at(i + 2) == '"' || at(i + 2) == '#') {
            Some((i + 2, true))
        } else {
            None
        };
        if let Some((mut k, bytes)) = raw_start {
            let mut hashes = 0usize;
            while at(k) == '#' {
                hashes += 1;
                k += 1;
            }
            if at(k) == '"' {
                let (start_line, line_before) = (line, slice(line_start, i));
                let body = k + 1;
                let mut j = body;
                loop {
                    if j >= chars.len() {
                        break;
                    }
                    if at(j) == '"' && (1..=hashes).all(|h| at(j + h) == '#') {
                        break;
                    }
                    if at(j) == '\n' {
                        line += 1;
                        line_start = j + 1;
                    }
                    j += 1;
                }
                let value = slice(body, j);
                let end = (j + 1 + hashes).min(chars.len());
                out.push(Literal { line: start_line, value, before: std::mem::take(&mut before), after: rest_of_line(end), line_before, bytes });
                i = end;
                continue;
            }
        }
        // Ordinary and byte strings.
        if c == '"' {
            let bytes = ident_before && at(i - 1) == 'b' && (i < 2 || !(at(i - 2).is_alphanumeric() || at(i - 2) == '_'));
            if bytes {
                before.pop();
            }
            let (start_line, line_before) = (line, slice(line_start, i));
            let mut value = String::new();
            let mut j = i + 1;
            while j < chars.len() && at(j) != '"' {
                let ch = at(j);
                if ch == '\n' {
                    line += 1;
                    line_start = j + 1;
                }
                if ch != '\\' {
                    value.push(ch);
                    j += 1;
                    continue;
                }
                let e = at(j + 1);
                j += 2;
                match e {
                    'n' => value.push('\n'),
                    't' => value.push('\t'),
                    'r' => value.push('\r'),
                    '0' => value.push('\0'),
                    '\\' | '"' | '\'' => value.push(e),
                    'x' => {
                        let hex = slice(j, j + 2);
                        value.extend(u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32));
                        j += 2;
                    }
                    'u' => {
                        // \u{…}
                        let close = (j..chars.len().min(j + 10)).find(|k| at(*k) == '}').unwrap_or(j);
                        let hex = slice(j + 1, close);
                        value.extend(u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32));
                        j = close + 1;
                    }
                    '\n' => {
                        // A line continuation: the newline and the next line's indent go.
                        line += 1;
                        line_start = j;
                        while at(j) == ' ' || at(j) == '\t' {
                            j += 1;
                        }
                    }
                    _ => {}
                }
            }
            let end = (j + 1).min(chars.len());
            out.push(Literal { line: start_line, value, before: std::mem::take(&mut before), after: rest_of_line(end), line_before, bytes });
            i = end;
            continue;
        }
        // Char literals ('x', '\n', '"') and lifetimes ('a, 'static).
        if c == '\'' {
            if at(i + 1) == '\\' {
                // Past the backslash and the escaped character, to the closing quote.
                let mut j = i + 3;
                while j < chars.len() && at(j) != '\'' && at(j) != '\n' {
                    j += 1;
                }
                before.push_str("' '");
                i = j + 1;
                continue;
            }
            if at(i + 2) == '\'' {
                before.push_str("' '");
                i += 3;
                continue;
            }
        }
        if c == '\n' {
            line += 1;
            line_start = i + 1;
        }
        before.push(c);
        // Only the end of the gap matters (which macro opened); keep it short.
        if before.len() > 400 {
            let cut = before.char_indices().nth(before.chars().count().saturating_sub(200)).map_or(0, |(k, _)| k);
            before = before.split_off(cut);
        }
        i += 1;
    }
    out
}

/// What each literal is to the translation macros, in the same order.
pub fn roles(lits: &[Literal]) -> Vec<Role> {
    let mut out: Vec<Role> = Vec::with_capacity(lits.len());
    for l in lits {
        let b = l.before.trim_end();
        let prev = out.last().copied();
        // The macro's name, not the end of a longer one (`include_str!(`).
        let opens = |m: &str| b.strip_suffix(m).is_some_and(|rest| !rest.ends_with(|c: char| c.is_alphanumeric() || c == '_'));
        let role = if opens("tr!(") || opens("tr_noop!(") {
            Role::Source
        } else if opens("trc!(") {
            Role::Context
        } else if opens("trn!(") {
            Role::One
        } else if b.trim() == "," && prev == Some(Role::Context) {
            Role::ContextSource
        } else if b.trim() == "," && prev == Some(Role::One) {
            Role::Other
        } else {
            Role::Plain
        };
        out.push(role);
    }
    out
}

/// A translatable key found in the source: `(context, source)`, or a plural's `one|other`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Key {
    Text { context: String, source: String },
    Plural { one: String, other: String },
}

/// Every key the macros in `src` use.
pub fn keys(src: &str) -> Vec<Key> {
    let lits = literals(src);
    let roles = roles(&lits);
    let mut out = Vec::new();
    for (k, (l, r)) in lits.iter().zip(&roles).enumerate() {
        let prev = k.checked_sub(1).and_then(|p| lits.get(p)).map(|p| p.value.clone()).unwrap_or_default();
        match r {
            Role::Source => out.push(Key::Text { context: String::new(), source: l.value.clone() }),
            Role::ContextSource => out.push(Key::Text { context: prev, source: l.value.clone() }),
            Role::Other => out.push(Key::Plural { one: prev, other: l.value.clone() }),
            Role::Context | Role::One | Role::Plain => {}
        }
    }
    out
}

/// Key names and combinations: data, not prose (ADR 0005), and shown through
/// `platform::shortcut_label`.
fn is_key_name(s: &str) -> bool {
    const KEYS: [&str; 20] = ["Enter", "Escape", "Esc", "Space", "Tab", "Delete", "Backspace", "Home", "End", "PageUp", "PageDown", "Insert", "Up", "Down", "Left", "Right", "Shift", "Ctrl", "Alt", "Cmd"];
    KEYS.contains(&s) || ["Ctrl+", "Shift+", "Alt+", "Cmd+", "Option+"].iter().any(|m| s.contains(m))
}

/// Whether a literal reads as interface prose: words a person reads, rather than an id, a
/// path, a key, a number or a format.
pub fn is_prose(s: &str) -> bool {
    let has_word = s.as_bytes().windows(2).any(|w| w.iter().all(u8::is_ascii_lowercase));
    if !has_word || is_key_name(s.trim()) {
        return false;
    }
    // Units and formats (` pt`, `{s} fps`, `{n} ms`): what is left without the placeholders is
    // a short symbol, data rather than prose.
    let mut bare = s.to_string();
    while let (Some(a), Some(b)) = (bare.find('{'), bare.find('}')) {
        if b < a {
            break;
        }
        bare.replace_range(a..=b, "");
    }
    let bare = bare.trim();
    if !bare.contains(' ') && bare.chars().count() <= 3 && !bare.starts_with(|c: char| c.is_ascii_uppercase()) {
        return false;
    }
    let spaced = s.contains(' ');
    if !spaced && s.contains(['/', '\\', '.', '_', ':', '=', '#', '<', '>']) && !s.ends_with('…') {
        return false;
    }
    spaced || s.starts_with(|c: char| c.is_ascii_uppercase()) || s.ends_with('…')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literals_comments_raw_strings_and_chars() {
        let src = r####"
// "in a comment"
/* "block /* nested */ comment" */
let a = tr!("Save");
let b = 'x'; let q = '"'; let e = '\''; fn f<'a>(x: &'a str) {}
let c = r#"raw "quoted""#;
let d = b"bytes";
let m = trn!("{n} frame",
    "{n} frames", n);
let t = trc!("menu", "Open");
let s = "a \"quote\" and \u{2026} and \
         continued";
let n = tr_noop!("Graphite");
let f = include_str!("../NOTICE"); let g = crate::i18n::tr!("Path");
"####;
        let lits = literals(src);
        let values: Vec<&str> = lits.iter().map(|l| l.value.as_str()).collect();
        assert_eq!(values, ["Save", "raw \"quoted\"", "bytes", "{n} frame", "{n} frames", "menu", "Open", "a \"quote\" and … and continued", "Graphite", "../NOTICE", "Path"]);
        assert!(lits.get(2).is_some_and(|l| l.bytes));
        assert_eq!(lits.first().map(|l| l.line), Some(4));
        assert_eq!(lits.get(4).map(|l| l.line), Some(9));
        let keys = keys(src);
        assert_eq!(
            keys,
            [
                Key::Text { context: String::new(), source: "Save".into() },
                Key::Plural { one: "{n} frame".into(), other: "{n} frames".into() },
                Key::Text { context: "menu".into(), source: "Open".into() },
                Key::Text { context: String::new(), source: "Graphite".into() },
                Key::Text { context: String::new(), source: "Path".into() },
            ]
        );
    }

    #[test]
    fn odd_input_never_panics() {
        for src in ["\"", "r#\"", "'", "'\\", "/*", "\"\\u{", "\"\\x", "b", "r", "br#", "\"\\", "é\"é\\\né"] {
            let _ = literals(src);
        }
    }

    #[test]
    fn prose_is_told_from_ids_keys_and_paths() {
        for s in ["Save", "Add", "Save As…", "Could not open {name}: {e}", "Zoom in", "Export…"] {
            assert!(is_prose(s), "{s}");
        }
        for s in ["edit.undo", "prefs_ui_font", "Ctrl+K", "Enter", "{}", "{n} %", "F1", "file.anicraft", "Insert/Timeline", "x", "OK", "wgpu", "#ff0000", " pt", "{s} fps"] {
            assert!(!is_prose(s), "{s}");
        }
    }
}
