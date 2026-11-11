//! Interface catalogs in the craft-i18n format (`docs/catalog-format.md` in
//! storytold/craft-i18n): one row per string, `context<TAB>English<TAB>translation`, keyed by the
//! English source text. Read here by our own small parser, so the app depends on no outside
//! crate for it, and a row it can't use is skipped (that string shows in English), never an
//! error.

use std::collections::HashMap;

/// Lines longer than this are skipped: no interface string is anywhere near it.
const MAX_LINE: usize = 16 * 1024;
/// Rows read per catalog at most (craft-i18n's largest shared catalog has about 1 300).
const MAX_ROWS: usize = 100_000;

/// One usable row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// Empty, a plain word (`menu`) or one of `@plural`, `@id`.
    pub context: String,
    pub source: String,
    pub translation: String,
}

/// A row that was skipped, and why (for the stale and missing report, never shown to users).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub line: usize,
    pub why: String,
}

/// `\n`, `\t` and `\\`; any other escape makes the row unusable.
fn unescape(s: &str) -> Option<String> {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('\\') => out.push('\\'),
            _ => return None,
        }
    }
    Some(out)
}

/// The `{name}` placeholders of a string, sorted (each once). `None` when the braces don't
/// pair up, so the row is skipped.
pub fn placeholders(s: &str) -> Option<Vec<&str>> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(open) = rest.find('{') {
        let after = rest.get(open + 1..)?;
        let close = after.find('}')?;
        let name = after.get(..close)?;
        if name.is_empty() || name.contains('{') || !name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return None;
        }
        out.push(name);
        rest = after.get(close + 1..)?;
    }
    if rest.contains('}') {
        return None;
    }
    out.sort_unstable();
    out.dedup();
    Some(out)
}

/// Why a row can't be used, if it can't.
fn check(context: &str, source: &str, translation: &str) -> Result<(), String> {
    if translation.is_empty() {
        return Err("empty translation".into());
    }
    if source.is_empty() {
        return Err("empty source text".into());
    }
    match context {
        "@id" | "@msg" => return Ok(()),
        "@plural" => {
            let forms: Vec<&str> = source.split('|').collect();
            if forms.len() != 2 {
                return Err("an @plural source needs exactly `one|other`".into());
            }
            let want = placeholders(source).ok_or("unpaired braces")?;
            for form in translation.split('|') {
                if form.is_empty() {
                    return Err("an empty plural form".into());
                }
                if placeholders(form).as_ref() != Some(&want) {
                    return Err(format!("plural form {form:?} has other placeholders than {source:?}"));
                }
            }
            return Ok(());
        }
        c if c.starts_with('@') => return Err(format!("unknown context {c}")),
        _ => {}
    }
    let (a, b) = (placeholders(source).ok_or("unpaired braces in the source")?, placeholders(translation).ok_or("unpaired braces in the translation")?);
    if a != b {
        return Err(format!("placeholders differ: {a:?} and {b:?}"));
    }
    if source.ends_with('…') != translation.ends_with('…') {
        return Err("a trailing … in one column needs one in the other".into());
    }
    Ok(())
}

/// Every usable row of a catalog file, and the problems with the others.
pub fn parse(text: &str) -> (Vec<Row>, Vec<Problem>) {
    let mut rows = Vec::new();
    let mut problems = Vec::new();
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    for (i, line) in text.lines().enumerate() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let mut problem = |why: String| problems.push(Problem { line: i + 1, why });
        if rows.len() >= MAX_ROWS {
            problem(format!("more than {MAX_ROWS} rows; the rest are skipped"));
            break;
        }
        if line.len() > MAX_LINE {
            problem("line too long".into());
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        let [context, source, translation] = cols.as_slice() else {
            problem(format!("{} columns, not 3", cols.len()));
            continue;
        };
        let (Some(context), Some(source), Some(translation)) = (unescape(context), unescape(source), unescape(translation)) else {
            problem("an escape other than \\n, \\t or \\\\".into());
            continue;
        };
        match check(&context, &source, &translation) {
            Ok(()) => rows.push(Row { context, source, translation }),
            Err(why) => problem(why),
        }
    }
    (rows, problems)
}

/// The translations of one interface language, merged from its catalogs (later wins).
#[derive(Default, Debug)]
pub struct Catalog {
    /// Source text → translation without a context.
    plain: HashMap<String, String>,
    /// Source text → (context, translation) for rows with a plain-word context.
    contextual: HashMap<String, Vec<(String, String)>>,
    /// English `one` form → (English `other` form, the language's forms).
    plural: HashMap<String, Vec<(String, Vec<String>)>>,
    /// `@id` rows: a command id (or `language.name`) → its label.
    ids: HashMap<String, String>,
}

impl Catalog {
    /// Adds rows over what is there: a row for the same key replaces the earlier one.
    pub fn merge(&mut self, rows: Vec<Row>) {
        for r in rows {
            match r.context.as_str() {
                "" => {
                    self.plain.insert(r.source, r.translation);
                }
                "@id" => {
                    self.ids.insert(r.source, r.translation);
                }
                // Templates for longer messages are not matched yet (docs/localisation.md).
                "@msg" => {}
                "@plural" => {
                    let Some((one, other)) = r.source.split_once('|') else { continue };
                    let forms = r.translation.split('|').map(str::to_string).collect();
                    let list = self.plural.entry(one.to_string()).or_default();
                    list.retain(|(o, _)| o != other);
                    list.push((other.to_string(), forms));
                }
                _ => {
                    let list = self.contextual.entry(r.source).or_default();
                    list.retain(|(c, _)| *c != r.context);
                    list.push((r.context, r.translation));
                }
            }
        }
    }

    /// The translation of `source` in `context` (falling back to the plain row), if any.
    pub fn get(&self, context: &str, source: &str) -> Option<&str> {
        if !context.is_empty()
            && let Some((_, t)) = self.contextual.get(source).and_then(|l| l.iter().find(|(c, _)| c == context))
        {
            return Some(t);
        }
        self.plain.get(source).map(String::as_str)
    }

    /// The label of a command id (`@id` rows).
    pub fn id(&self, id: &str) -> Option<&str> {
        self.ids.get(id).map(String::as_str)
    }

    /// The forms of an `@plural` row.
    pub fn plural(&self, one: &str, other: &str) -> Option<&[String]> {
        self.plural.get(one)?.iter().find(|(o, _)| o == other).map(|(_, f)| f.as_slice())
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.plain.len() + self.contextual.values().map(Vec::len).sum::<usize>() + self.plural.values().map(Vec::len).sum::<usize>() + self.ids.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\u{feff}# Header line\r\n\
        @id\tlanguage.name\tTestisch\r\n\
        \tSave\tSpeichern\r\n\
        menu\tOpen\tÖffnen (Menü)\n\
        \tOpen\tÖffnen\n\
        \tLine one\\nLine two\tZeile eins\\nZeile zwei\n\
        @plural\t{n} frame|{n} frames\t{n} Bild|{n} Bilder\n\
        \tSaved {path}\t{path} gespeichert\n\
        \tExport…\tExportieren…\n\
        @id\tedit.undo\tRückgängig\n\
        @msg\tCould not open {name}\t{name} lässt sich nicht öffnen\n\
        \n\
        \tBroken\t\n\
        \tTwo columns\n\
        \tSaved {path}x\t{file} gespeichert\n\
        \tExport…\tExportieren\n\
        \tBad \\q escape\tx\n\
        @future\tThing\tDing\n\
        @plural\t{n} layer\t{n} Ebene\n\
        @plural\t{n} key|{n} keys\t{n} Taste|{count} Tasten\n\
        \tOdd {brace\tx {brace\n";

    #[test]
    fn rows_are_read_and_bad_ones_skipped_with_a_reason() {
        let (rows, problems) = parse(SAMPLE);
        assert_eq!(rows.len(), 10, "{rows:#?}");
        assert_eq!(problems.len(), 9, "{problems:#?}");
        assert!(problems.iter().all(|p| p.line > 11));
        let mut c = Catalog::default();
        c.merge(rows);
        assert_eq!(c.get("", "Save"), Some("Speichern"));
        assert_eq!(c.get("menu", "Open"), Some("Öffnen (Menü)"));
        assert_eq!(c.get("", "Open"), Some("Öffnen"));
        assert_eq!(c.get("toolbar", "Open"), Some("Öffnen"), "an unknown context falls back to the plain row");
        assert_eq!(c.get("", "Line one\nLine two"), Some("Zeile eins\nZeile zwei"));
        assert_eq!(c.plural("{n} frame", "{n} frames"), Some(&["{n} Bild".to_string(), "{n} Bilder".to_string()][..]));
        assert_eq!(c.id("edit.undo"), Some("Rückgängig"));
        assert_eq!(c.id("language.name"), Some("Testisch"));
        assert_eq!(c.get("", "Export…"), Some("Exportieren…"));
        assert_eq!(c.get("", "Missing"), None);
    }

    #[test]
    fn later_catalogs_override_earlier_ones() {
        let mut c = Catalog::default();
        c.merge(parse("\tSave\tA\nmenu\tOpen\tB\n@plural\t{n} x|{n} xs\t{n} a|{n} b\n").0);
        c.merge(parse("\tSave\tC\nmenu\tOpen\tD\n@plural\t{n} x|{n} xs\t{n} c|{n} d\n").0);
        assert_eq!(c.get("", "Save"), Some("C"));
        assert_eq!(c.get("menu", "Open"), Some("D"));
        assert_eq!(c.plural("{n} x", "{n} xs").map(<[String]>::len), Some(2));
        assert_eq!(c.plural("{n} x", "{n} xs").and_then(|f| f.first()).map(String::as_str), Some("{n} c"));
        assert_eq!(c.len(), 3);
    }

    #[test]
    fn hostile_input_never_panics() {
        for text in ["", "\t", "\t\t", "\t\t\t", "{\t{\t{", "}\t}\t}", "@plural\t|\t|", "@plural\t{n}|{n}\t", "\\", "\t\\\tx", "\u{feff}", "\r\n\r\n", "\t{é}\t{é}"] {
            let _ = parse(text);
        }
        let long = format!("\t{}\tx", "a".repeat(MAX_LINE + 1));
        assert_eq!(parse(&long).0.len(), 0);
        assert_eq!(placeholders("{a} and {b} and {a}"), Some(vec!["a", "b"]));
        assert_eq!(placeholders("{a"), None);
        assert_eq!(placeholders("a}"), None);
        assert_eq!(placeholders("{}"), None);
    }
}
