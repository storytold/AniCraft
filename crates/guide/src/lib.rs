//! AniCraft's user guide as a book (layer L0; ADR 0069).
//!
//! The chapters are Markdown in `docs/guide/`, readable on GitHub as they are. This crate builds
//! them into the bundled book: every page and picture is compiled in, so Help › User Guide works
//! offline, and [`site`] turns them into static HTML pages with one stylesheet, a chapter list
//! and previous / next links. Links between chapters are written to `.md` files and become
//! `.html` links; headings get GitHub's anchor ids, so `timeline.md#onion-skin` works in both
//! places.
//!
//! [`check`] is the book's proof-reader: every internal link and picture resolves, every anchor
//! names a heading, every page is reachable from the contents page and has one title, nothing
//! links outside the book, and there is no raw HTML. Its test keeps the committed guide honest;
//! the generated command and shortcut references are checked for freshness by the app's own
//! tests (`src/guide_reference_tests.rs`), which know the menus and keys.
//!
//! Rendering escapes raw HTML in the Markdown instead of passing it through, so a page can never
//! carry markup or script of its own.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use pulldown_cmark::{CowStr, Event, Options, Parser, Tag, TagEnd};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;

/// One chapter: its file under `docs/guide/` and its Markdown.
#[derive(Clone, Copy, Debug)]
pub struct Page {
    pub file: &'static str,
    pub markdown: &'static str,
}

macro_rules! page {
    ($f:literal) => {
        Page { file: $f, markdown: include_str!(concat!("../../../docs/guide/", $f)) }
    };
}

macro_rules! image {
    ($f:literal) => {
        ($f, include_bytes!(concat!("../../../docs/guide/", $f)).as_slice())
    };
}

/// The book in reading order. The contents page comes first; the generated references last.
pub const PAGES: &[Page] = &[
    page!("index.md"),
    page!("getting-started.md"),
    page!("workspace.md"),
    page!("drawing.md"),
    page!("symbols-and-library.md"),
    page!("timeline.md"),
    page!("tweens.md"),
    page!("masks-guides-camera-bones.md"),
    page!("text.md"),
    page!("images-and-sound.md"),
    page!("interactivity.md"),
    page!("exporting-and-publish.md"),
    page!("opening-animate-files.md"),
    page!("preferences-shortcuts-accessibility.md"),
    page!("plugins.md"),
    page!("working-headless.md"),
    page!("troubleshooting-and-faq.md"),
    page!("coming-from-animate.md"),
    page!("reference/commands.md"),
    page!("reference/shortcuts.md"),
];

/// The book's pictures (screenshots from `scripts/guide-shots.sh`), by their path in the book.
pub const IMAGES: &[(&str, &[u8])] = &[
    image!("images/workspace.png"),
    image!("images/start-screen.png"),
    image!("images/library.png"),
    image!("images/onion-skin.png"),
    image!("images/motion-editor.png"),
    image!("images/test-movie.png"),
    image!("images/publish-settings.png"),
    image!("images/preferences.png"),
];

const STYLE: &str = include_str!("style.css");
const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The stylesheet's path in the built book.
pub const STYLESHEET: &str = "style.css";

/// One file of the built book: its path (with `/`) and contents.
#[derive(Clone, Debug, PartialEq)]
pub struct SiteFile {
    pub path: String,
    pub bytes: Vec<u8>,
}

/// The page a chapter becomes: `reference/commands.md` → `reference/commands.html`.
pub fn html_path(md: &str) -> String {
    match md.strip_suffix(".md") {
        Some(stem) => format!("{stem}.html"),
        None => md.to_string(),
    }
}

/// The parser options the book is written for: CommonMark with GFM tables and strikethrough.
fn options() -> Options {
    Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH
}

/// A link target inside the book (not `https:`, `mailto:` or another scheme).
fn is_relative(url: &str) -> bool {
    !url.contains(':') && !url.starts_with('/') && !url.starts_with('\\')
}

/// `foo.md#bar` → `foo.html#bar`; other links stay as they are.
fn rewrite_link(url: &str) -> String {
    if !is_relative(url) {
        return url.to_string();
    }
    match url.split_once('#') {
        Some((path, anchor)) if path.ends_with(".md") => format!("{}#{anchor}", html_path(path)),
        None if url.ends_with(".md") => html_path(url),
        _ => url.to_string(),
    }
}

/// GitHub's heading anchor: lower case, spaces to `-`, letters, digits, `-` and `_` kept.
pub fn slug(heading: &str) -> String {
    heading
        .trim()
        .chars()
        .flat_map(char::to_lowercase)
        .filter_map(|c| match c {
            ' ' => Some('-'),
            c if c.is_alphanumeric() || c == '-' || c == '_' => Some(c),
            _ => None,
        })
        .collect()
}

/// A page's events, with heading ids (GitHub's, numbered on repeats), `.md` links pointing at
/// pages, and raw HTML turned into visible text.
fn events(markdown: &str) -> Vec<Event<'_>> {
    let mut out: Vec<Event> = Parser::new_ext(markdown, options())
        .map(|e| match e {
            Event::Html(h) | Event::InlineHtml(h) => Event::Text(h),
            Event::Start(Tag::Link { link_type, dest_url, title, id }) => Event::Start(Tag::Link { link_type, dest_url: CowStr::from(rewrite_link(&dest_url)), title, id }),
            Event::Start(Tag::Image { link_type, dest_url, title, id }) => Event::Start(Tag::Image { link_type, dest_url: CowStr::from(rewrite_link(&dest_url)), title, id }),
            other => other,
        })
        .collect();
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    let mut i = 0;
    while i < out.len() {
        if let Some(Event::Start(Tag::Heading { .. })) = out.get(i) {
            let text = heading_text(out.get(i + 1..).unwrap_or_default());
            let id = unique_slug(&mut seen, &text);
            if let Some(Event::Start(Tag::Heading { id: slot, .. })) = out.get_mut(i) {
                *slot = Some(CowStr::from(id));
            }
        }
        i += 1;
    }
    out
}

/// The text of a heading whose start event came just before `rest`.
fn heading_text(rest: &[Event]) -> String {
    let mut s = String::new();
    for e in rest {
        match e {
            Event::End(TagEnd::Heading(_)) => break,
            Event::Text(t) | Event::Code(t) => s.push_str(t),
            _ => {}
        }
    }
    s
}

fn unique_slug(seen: &mut BTreeMap<String, usize>, text: &str) -> String {
    let base = slug(text);
    let n = seen.entry(base.clone()).or_insert(0);
    let id = if *n == 0 { base } else { format!("{base}-{n}") };
    *n += 1;
    id
}

/// A page's title: its first level-one heading.
pub fn title(markdown: &str) -> String {
    let mut parser = Parser::new_ext(markdown, options());
    while let Some(e) = parser.next() {
        if let Event::Start(Tag::Heading { level: pulldown_cmark::HeadingLevel::H1, .. }) = e {
            let rest: Vec<Event> = parser.by_ref().take_while(|e| !matches!(e, Event::End(TagEnd::Heading(_)))).collect();
            return heading_text(&rest);
        }
    }
    String::new()
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// `../` once per folder the page is in, to reach the book's root.
fn root_prefix(file: &str) -> String {
    "../".repeat(file.matches('/').count())
}

/// A chapter as a complete HTML page, with the chapter list and previous / next links.
pub fn render_page(pages: &[Page], at: usize) -> String {
    let Some(page) = pages.get(at) else { return String::new() };
    let root = root_prefix(page.file);
    let mut body = String::new();
    pulldown_cmark::html::push_html(&mut body, events(page.markdown).into_iter());
    let title = title(page.markdown);
    let mut toc = String::new();
    for (i, p) in pages.iter().enumerate() {
        let current = if i == at { " aria-current=\"page\"" } else { "" };
        toc.push_str(&format!("<li><a href=\"{root}{}\"{current}>{}</a></li>\n", html_path(p.file), escape(&self::title(p.markdown))));
    }
    let link = |p: Option<&Page>, rel: &str, word: &str| {
        p.map_or_else(String::new, |p| format!("<a rel=\"{rel}\" href=\"{root}{}\">{word}: {}</a>", html_path(p.file), escape(&self::title(p.markdown))))
    };
    let prev = link(at.checked_sub(1).and_then(|i| pages.get(i)), "prev", "Previous");
    let next = link(pages.get(at + 1), "next", "Next");
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
         <title>{t} · AniCraft User Guide</title>\n<link rel=\"stylesheet\" href=\"{root}{STYLESHEET}\">\n</head>\n<body>\n\
         <a class=\"skip\" href=\"#content\">Skip to the chapter</a>\n\
         <header class=\"top\"><a href=\"{root}index.html\">AniCraft User Guide</a> <span class=\"version\">{VERSION}</span></header>\n\
         <div class=\"layout\">\n<nav class=\"toc\" aria-label=\"Chapters\">\n<ol>\n{toc}</ol>\n</nav>\n\
         <main id=\"content\">\n{body}<nav class=\"pager\" aria-label=\"Previous and next chapters\">{prev}{next}</nav>\n</main>\n</div>\n\
         <footer>AniCraft {VERSION} · MIT OR Apache-2.0 · AniCraft is not affiliated with or endorsed by Adobe.</footer>\n</body>\n</html>\n",
        t = escape(&title),
    )
}

/// The whole book: one HTML page per chapter, the stylesheet and the pictures.
pub fn site() -> Vec<SiteFile> {
    let mut out: Vec<SiteFile> = (0..PAGES.len()).filter_map(|i| PAGES.get(i).map(|p| SiteFile { path: html_path(p.file), bytes: render_page(PAGES, i).into_bytes() })).collect();
    out.push(SiteFile { path: STYLESHEET.to_string(), bytes: STYLE.as_bytes().to_vec() });
    out.extend(IMAGES.iter().map(|(path, bytes)| SiteFile { path: (*path).to_string(), bytes: bytes.to_vec() }));
    out
}

/// Writes the book into a folder (made if needed); the number of files written. The paths are
/// the book's own (checked by [`check`]), never input.
pub fn write_site(dir: &Path) -> std::io::Result<usize> {
    let files = site();
    for f in &files {
        let path = f.path.split('/').fold(dir.to_path_buf(), |p, part| p.join(part));
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, &f.bytes)?;
    }
    Ok(files.len())
}

/// `rel` (a link written in `from`) as a path from the book's root, or `None` when it climbs
/// out of the book.
fn resolve(from: &str, rel: &str) -> Option<String> {
    let mut parts: Vec<&str> = from.split('/').collect();
    parts.pop();
    for part in rel.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            p => parts.push(p),
        }
    }
    Some(parts.join("/"))
}

/// What a page links to and which anchors it offers.
struct Scan {
    links: Vec<String>,
    images: Vec<String>,
    anchors: BTreeSet<String>,
    titles: usize,
    raw_html: usize,
}

fn scan(markdown: &str) -> Scan {
    let mut s = Scan { links: Vec::new(), images: Vec::new(), anchors: BTreeSet::new(), titles: 0, raw_html: 0 };
    let raw: Vec<Event> = Parser::new_ext(markdown, options()).collect();
    for e in &raw {
        match e {
            Event::Start(Tag::Link { dest_url, .. }) => s.links.push(dest_url.to_string()),
            Event::Start(Tag::Image { dest_url, .. }) => s.images.push(dest_url.to_string()),
            Event::Start(Tag::Heading { level: pulldown_cmark::HeadingLevel::H1, .. }) => s.titles += 1,
            Event::Html(_) | Event::InlineHtml(_) => s.raw_html += 1,
            _ => {}
        }
    }
    for e in events(markdown) {
        if let Event::Start(Tag::Heading { id: Some(id), .. }) = e {
            s.anchors.insert(id.to_string());
        }
    }
    s
}

/// Everything wrong with a book: broken links, pictures or anchors, links out of the book, pages
/// the contents page can't reach, pages without exactly one title, raw HTML and unused
/// pictures. Empty when the book is sound. `pages[0]` is the contents page.
pub fn check(pages: &[Page], images: &[&str]) -> Vec<String> {
    let mut problems = Vec::new();
    let scans: BTreeMap<&str, Scan> = pages.iter().map(|p| (p.file, scan(p.markdown))).collect();
    let mut used_images = BTreeSet::new();
    let mut edges: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for (file, s) in &scans {
        if s.titles != 1 {
            problems.push(format!("{file}: has {} level-one headings; a page has exactly one, its title", s.titles));
        }
        if s.raw_html > 0 {
            problems.push(format!("{file}: has raw HTML; the book is plain Markdown"));
        }
        for url in &s.links {
            if !is_relative(url) {
                if !url.starts_with("https://") {
                    problems.push(format!("{file}: link {url} is neither in the book nor https"));
                }
                continue;
            }
            let (path, anchor) = url.split_once('#').map_or((url.as_str(), None), |(p, a)| (p, Some(a)));
            let target = if path.is_empty() { Some((*file).to_string()) } else { resolve(file, path) };
            let Some(target) = target else {
                problems.push(format!("{file}: link {url} leaves the book"));
                continue;
            };
            match scans.get(target.as_str()) {
                Some(t) => {
                    if let Some(a) = anchor
                        && !t.anchors.contains(a)
                    {
                        problems.push(format!("{file}: link {url}: {target} has no heading #{a}"));
                    }
                    edges.entry(file).or_default().push(target);
                }
                None if images.contains(&target.as_str()) => {
                    used_images.insert(target);
                }
                None => problems.push(format!("{file}: link {url}: {target} is not a page of the book")),
            }
        }
        for url in &s.images {
            match resolve(file, url).filter(|t| is_relative(url) && images.contains(&t.as_str())) {
                Some(t) => {
                    used_images.insert(t);
                }
                None => problems.push(format!("{file}: picture {url} is not one of the book's pictures")),
            }
        }
    }
    for image in images {
        if !used_images.contains(*image) {
            problems.push(format!("{image}: no page shows it"));
        }
    }
    // Every page reachable from the contents page.
    if let Some(first) = pages.first() {
        let mut reached = BTreeSet::from([first.file.to_string()]);
        let mut queue = VecDeque::from([first.file.to_string()]);
        while let Some(at) = queue.pop_front() {
            for next in edges.get(at.as_str()).into_iter().flatten() {
                if reached.insert(next.clone()) {
                    queue.push_back(next.clone());
                }
            }
        }
        for p in pages {
            if !reached.contains(p.file) {
                problems.push(format!("{}: can't be reached from {}", p.file, first.file));
            }
        }
    }
    problems
}

#[cfg(test)]
mod tests;
