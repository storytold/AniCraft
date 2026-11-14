//! Searching the book (ADR 0096).
//!
//! The book is split into sections, one per heading, each with its plain text. [`index_js`]
//! writes them into `search-index.js`, which the book's search page loads with a `<script>` tag
//! (a page opened from a `file://` folder may not fetch a JSON file, but it may load a script
//! beside it). `search.js` ranks the sections in the browser with the same rules as [`search`],
//! which `anicraft-cli guide search` prints, so the two give the same results; a test runs the
//! script under Node, when it is installed, and compares them.
//!
//! The rules are deliberately simple and integer-only, so a port can't drift through floating
//! point: a query is split into words (lower case, letters and digits), common words dropped;
//! a word matches a word of the book exactly, as its beginning (`tween` → `tweens`), or as a
//! longer form of it (`masks` → `mask`). Every query word must match somewhere in a section.
//! A match in the section's heading counts most, then in the chapter's title, then each match in
//! its text (up to a cap); the words together as a phrase add a bonus. Ties keep book order.

use crate::{Page, events, html_path, title};
use pulldown_cmark::{Event, HeadingLevel, Tag, TagEnd};

/// One heading of the book and the text under it, up to the next heading.
#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    /// The page's index in the book.
    pub page: usize,
    /// The heading's id (GitHub's, as in the page).
    pub anchor: String,
    pub heading: String,
    pub level: u8,
    /// The text under the heading as plain words, whitespace collapsed.
    pub text: String,
}

/// One search result.
#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    /// The chapter's title.
    pub chapter: String,
    pub heading: String,
    /// The page and heading in the book, as a link from its root: `timeline.html#onion-skin`
    /// (a chapter's title links to the page itself).
    pub url: String,
    /// A short passage around the first match.
    pub snippet: String,
    pub score: u32,
}

/// The longest query read, in characters, and the most words it may have.
pub const MAX_QUERY: usize = 200;
pub const MAX_TERMS: usize = 8;
/// Snippet length in characters, and how far before the first match it starts.
const SNIPPET: usize = 180;
const LEAD: usize = 60;

/// Words too common to search for (dropped unless the query is made only of them).
pub const STOP_WORDS: &[&str] = &["a", "an", "and", "are", "as", "at", "be", "by", "can", "do", "does", "for", "from", "how", "i", "if", "in", "is", "it", "my", "of", "on", "or", "the", "to", "what", "when", "where", "with"];

fn level(l: HeadingLevel) -> u8 {
    match l {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Every section of the book, in reading order.
pub fn sections(pages: &[Page]) -> Vec<Section> {
    let mut out = Vec::new();
    for (page, p) in pages.iter().enumerate() {
        let mut current: Option<Section> = None;
        let mut heading: Option<String> = None;
        let mut text = String::new();
        let finish = |current: &mut Option<Section>, text: &mut String, out: &mut Vec<Section>| {
            if let Some(mut s) = current.take() {
                s.text = collapse(text);
                out.push(s);
            }
            text.clear();
        };
        for e in events(p.markdown) {
            match e {
                Event::Start(Tag::Heading { level: l, id, .. }) => {
                    finish(&mut current, &mut text, &mut out);
                    current = Some(Section { page, anchor: id.map(|i| i.to_string()).unwrap_or_default(), heading: String::new(), level: level(l), text: String::new() });
                    heading = Some(String::new());
                }
                Event::End(TagEnd::Heading(_)) => {
                    if let (Some(s), Some(h)) = (current.as_mut(), heading.take()) {
                        s.heading = collapse(&h);
                    }
                }
                Event::Text(t) | Event::Code(t) => match heading.as_mut() {
                    Some(h) => h.push_str(&t),
                    None => text.push_str(&t),
                },
                // A space where words would otherwise run together: line breaks and the ends of
                // blocks (not of bold or links, which sit inside a sentence).
                Event::SoftBreak
                | Event::HardBreak
                | Event::End(TagEnd::Paragraph | TagEnd::Item | TagEnd::TableCell | TagEnd::TableRow | TagEnd::TableHead | TagEnd::CodeBlock | TagEnd::BlockQuote(_) | TagEnd::List(_)) => match heading.as_mut() {
                    Some(h) => h.push(' '),
                    None => text.push(' '),
                },
                _ => {}
            }
        }
        if current.is_none() && !text.trim().is_empty() {
            // Text before any heading belongs to the page (the book's pages all start with one).
            current = Some(Section { page, anchor: String::new(), heading: title(p.markdown), level: 1, text: String::new() });
        }
        finish(&mut current, &mut text, &mut out);
    }
    out
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric()
}

/// The words of `s`, lower case, each with the position (in characters) where it starts.
pub fn words(s: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut start = 0;
    for (i, c) in s.chars().enumerate() {
        if is_word(c) {
            if cur.is_empty() {
                start = i;
            }
            cur.extend(c.to_lowercase());
        } else if !cur.is_empty() {
            out.push((start, std::mem::take(&mut cur)));
        }
    }
    if !cur.is_empty() {
        out.push((start, cur));
    }
    out
}

/// A query's words: at most [`MAX_TERMS`], no repeats, common words dropped unless that leaves
/// none.
pub fn terms(query: &str) -> Vec<String> {
    let query: String = query.chars().take(MAX_QUERY).collect();
    let mut all: Vec<String> = Vec::new();
    for (_, w) in words(&query) {
        if !all.contains(&w) {
            all.push(w);
        }
    }
    let kept: Vec<String> = all.iter().filter(|w| !STOP_WORDS.contains(&w.as_str())).cloned().collect();
    let mut out = if kept.is_empty() { all } else { kept };
    out.truncate(MAX_TERMS);
    out
}

/// How well query word `q` matches book word `t`: 3 the same, 2 `t` begins with `q`, 1 `q` is
/// `t` with at most two letters more (a plural or ending), 0 not at all.
pub fn match_class(q: &str, t: &str) -> u32 {
    if q == t {
        return 3;
    }
    let (ql, tl) = (q.chars().count(), t.chars().count());
    if ql >= 2 && t.starts_with(q) {
        2
    } else if tl >= 3 && ql <= tl + 2 && q.starts_with(t) {
        1
    } else {
        0
    }
}

fn best(q: &str, ws: &[(usize, String)]) -> u32 {
    ws.iter().map(|(_, w)| match_class(q, w)).max().unwrap_or(0)
}

fn phrase(ws: &[(usize, String)]) -> String {
    let v: Vec<&str> = ws.iter().map(|(_, w)| w.as_str()).collect();
    format!(" {} ", v.join(" "))
}

/// A section's words, worked out once per search.
struct Prepared {
    heading: Vec<(usize, String)>,
    title: Vec<(usize, String)>,
    body: Vec<(usize, String)>,
}

fn score(p: &Prepared, terms: &[String]) -> Option<u32> {
    let mut total = 0;
    for q in terms {
        let body: u32 = p.body.iter().map(|(_, w)| match_class(q, w)).sum();
        let s = 30 * best(q, &p.heading) + 8 * best(q, &p.title) + body.min(15);
        if s == 0 {
            return None;
        }
        total += s;
    }
    if terms.len() > 1 {
        let wanted = format!(" {} ", terms.join(" "));
        if phrase(&p.heading).contains(&wanted) {
            total += 60;
        } else if phrase(&p.body).contains(&wanted) {
            total += 20;
        }
    }
    Some(total)
}

/// About [`SNIPPET`] characters of `text` around the first word matching a query word, cut at
/// spaces, with `…` where it was cut.
pub fn snippet(text: &str, terms: &[String]) -> String {
    let chars: Vec<char> = text.chars().collect();
    let ws = words(text);
    let at = ws.iter().find(|(_, w)| terms.iter().any(|q| match_class(q, w) > 0)).map_or(0, |(i, _)| *i);
    let mut start = at.saturating_sub(LEAD);
    if start > 0 {
        // Begin at a word: just after the next space before the match.
        while start < at && chars.get(start - 1).is_some_and(|c| *c != ' ') {
            start += 1;
        }
    }
    let mut end = (start + SNIPPET).min(chars.len());
    if end < chars.len() {
        let mut e = end;
        while e > start + SNIPPET / 2 && chars.get(e).is_some_and(|c| *c != ' ') {
            e -= 1;
        }
        end = e;
    }
    let body: String = chars.get(start..end).map(|s| s.iter().collect()).unwrap_or_default();
    let body = body.trim();
    format!("{}{}{}", if start > 0 { "…" } else { "" }, body, if end < chars.len() { "…" } else { "" })
}

/// The best `limit` sections for `query`, best first.
pub fn search(pages: &[Page], sections: &[Section], query: &str, limit: usize) -> Vec<Hit> {
    let terms = terms(query);
    if terms.is_empty() || limit == 0 {
        return Vec::new();
    }
    let titles: Vec<String> = pages.iter().map(|p| title(p.markdown)).collect();
    let title_words: Vec<Vec<(usize, String)>> = titles.iter().map(|t| words(t)).collect();
    let mut scored: Vec<(u32, &Section)> = sections
        .iter()
        .filter_map(|s| {
            let p = Prepared { heading: words(&s.heading), title: title_words.get(s.page).cloned().unwrap_or_default(), body: words(&s.text) };
            score(&p, &terms).map(|n| (n, s))
        })
        .collect();
    // Stable: equal scores keep the book's order.
    scored.sort_by_key(|s| std::cmp::Reverse(s.0));
    scored
        .into_iter()
        .take(limit)
        .map(|(n, s)| {
            let file = pages.get(s.page).map(|p| html_path(p.file)).unwrap_or_default();
            Hit {
                chapter: titles.get(s.page).cloned().unwrap_or_default(),
                heading: s.heading.clone(),
                url: if s.level == 1 || s.anchor.is_empty() { file } else { format!("{file}#{}", s.anchor) },
                snippet: snippet(&s.text, &terms),
                score: n,
            }
        })
        .collect()
}

/// A JSON string literal, safe inside a script file too (U+2028 and U+2029 escaped).
fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '<' => out.push_str("\\u003c"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The search index as JSON: the pages (`f` its file, `t` its title) and the sections (`p` the
/// page, `a` the anchor, `h` the heading, `l` the level, `x` the text).
pub fn index_json(pages: &[Page], sections: &[Section]) -> String {
    let mut out = String::from("{\"pages\":[");
    for (i, p) in pages.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!("{{\"f\":{},\"t\":{}}}", json_str(&html_path(p.file)), json_str(&title(p.markdown))));
    }
    out.push_str("],\"sections\":[");
    for (i, s) in sections.iter().enumerate() {
        if i > 0 {
            out.push_str(",\n");
        }
        out.push_str(&format!("{{\"p\":{},\"a\":{},\"h\":{},\"l\":{},\"x\":{}}}", s.page, json_str(&s.anchor), json_str(&s.heading), s.level, json_str(&s.text)));
    }
    out.push_str("]}");
    out
}

/// `search-index.js`: the index as a script the search page loads.
pub fn index_js(pages: &[Page]) -> String {
    format!("// AniCraft User Guide search index, generated with the book. Do not edit.\nwindow.ANICRAFT_GUIDE_INDEX = {};\n", index_json(pages, &sections(pages)))
}
