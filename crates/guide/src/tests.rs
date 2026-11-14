use super::*;

fn guide_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/guide")
}

/// Files under `dir` with this extension, as paths from `dir` with `/`.
fn files(dir: &Path, ext: &str, prefix: &str, out: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).expect("docs/guide exists").flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        if path.is_dir() {
            files(&path, ext, &format!("{prefix}{name}/"), out);
        } else if name.ends_with(ext) {
            out.push(format!("{prefix}{name}"));
        }
    }
}

fn image_paths() -> Vec<&'static str> {
    IMAGES.iter().map(|(p, _)| *p).collect()
}

#[test]
fn the_guide_has_no_broken_links_and_every_page_is_reachable() {
    let problems = check(PAGES, &image_paths());
    assert!(problems.is_empty(), "the user guide has problems:\n{}", problems.join("\n"));
}

#[test]
fn every_chapter_and_picture_in_docs_guide_is_in_the_book() {
    let mut md = Vec::new();
    files(&guide_dir(), ".md", "", &mut md);
    let mut book: Vec<String> = PAGES.iter().map(|p| p.file.to_string()).collect();
    md.sort();
    book.sort();
    assert_eq!(md, book, "docs/guide and PAGES must list the same chapters");
    let mut png = Vec::new();
    files(&guide_dir(), ".png", "", &mut png);
    let mut pictures: Vec<String> = image_paths().into_iter().map(str::to_string).collect();
    png.sort();
    pictures.sort();
    assert_eq!(png, pictures, "docs/guide and IMAGES must list the same pictures");
}

#[test]
fn pictures_stay_small() {
    // Committed screenshots are few and small (scripts/guide-shots.sh shrinks them).
    for (path, bytes) in IMAGES {
        assert!(bytes.len() <= 300 * 1024, "{path} is {} KB; keep guide pictures under 300 KB", bytes.len() / 1024);
        assert!(bytes.starts_with(b"\x89PNG"), "{path} is not a PNG");
    }
}

#[test]
fn every_page_has_a_title_and_the_site_has_a_page_for_each() {
    let site = site();
    for p in PAGES {
        assert!(!title(p.markdown).trim().is_empty(), "{} has no title", p.file);
        let html = html_path(p.file);
        let f = site.iter().find(|f| f.path == html).unwrap_or_else(|| panic!("no {html}"));
        let text = String::from_utf8_lossy(&f.bytes);
        assert!(text.contains("<nav class=\"toc\""), "{html} has no chapter list");
        for href in text.split("href=\"").skip(1).filter_map(|h| h.split('"').next()) {
            assert!(!is_relative(href) || !href.split('#').next().unwrap_or("").ends_with(".md"), "{html} still links to Markdown: {href}");
        }
    }
    assert!(site.iter().any(|f| f.path == STYLESHEET));
    assert!(site.iter().all(|f| !f.path.contains("..") && !f.path.starts_with('/')));
}

#[test]
fn slugs_match_github() {
    assert_eq!(slug("Onion skin"), "onion-skin");
    assert_eq!(slug("Tweens (classic, shape & motion)"), "tweens-classic-shape--motion");
    assert_eq!(slug("Edit Multiple Frames"), "edit-multiple-frames");
    assert_eq!(slug("`file.save` and Ctrl+S"), "filesave-and-ctrls");
    assert_eq!(slug("Préférences"), "préférences");
}

#[test]
fn links_become_pages_and_headings_get_ids() {
    let pages = [Page { file: "index.md", markdown: "# Guide\n\nSee [the timeline](timeline.md#onion-skin) and [web](https://example.com/a.md).\n\n## Same\n\n## Same\n\n<b>raw</b>\n" }, Page { file: "timeline.md", markdown: "# Timeline\n\n## Onion skin\n" }];
    let html = render_page(&pages, 0);
    assert!(html.contains("href=\"timeline.html#onion-skin\""), "{html}");
    assert!(html.contains("href=\"https://example.com/a.md\""), "external links stay as written");
    assert!(html.contains("id=\"same\"") && html.contains("id=\"same-1\""), "repeated headings are numbered");
    assert!(html.contains("&lt;b&gt;raw&lt;/b&gt;"), "raw HTML is shown, never passed through");
    assert!(html.contains("rel=\"next\""));
    let nested = [Page { file: "index.md", markdown: "# Guide\n\n[c](reference/commands.md)\n" }, Page { file: "reference/commands.md", markdown: "# Commands\n\n[back](../index.md)\n" }];
    let html = render_page(&nested, 1);
    assert!(html.contains("href=\"../style.css\"") && html.contains("href=\"../index.html\""), "{html}");
    assert!(check(&nested, &[]).is_empty(), "{:?}", check(&nested, &[]));
}

#[test]
fn the_check_finds_what_is_wrong() {
    let pages = [
        Page { file: "index.md", markdown: "# Guide\n\n[a](a.md) [gone](missing.md) [anchor](a.md#nope) [out](../README.md) [ftp](ftp://x) ![p](images/none.png)\n" },
        Page { file: "a.md", markdown: "# A\n\n# Second title\n" },
        Page { file: "island.md", markdown: "# Island\n\n<div>x</div>\n" },
    ];
    let problems = check(&pages, &["images/unused.png"]).join("\n");
    for expect in ["missing.md is not a page", "has no heading #nope", "../README.md leaves the book", "ftp://x is neither", "images/none.png is not one", "a.md: has 2 level-one", "island.md: can't be reached", "island.md: has raw HTML", "images/unused.png: no page shows it"] {
        assert!(problems.contains(expect), "expected {expect:?} in:\n{problems}");
    }
}

#[test]
fn writes_the_book_to_a_folder() {
    let dir = std::env::temp_dir().join(format!("anicraft-guide-test-{}", std::process::id()));
    let n = write_site(&dir).expect("writes");
    assert_eq!(n, site().len());
    assert!(dir.join("index.html").is_file() && dir.join("reference").join("commands.html").is_file());
    let _ = std::fs::remove_dir_all(&dir);
}

// ---- Search (ADR 0096) ----

fn top(query: &str, n: usize) -> Vec<String> {
    search::search(PAGES, &search::sections(PAGES), query, n).into_iter().map(|h| h.url).collect()
}

#[test]
fn the_index_covers_every_heading_of_every_page() {
    let sections = search::sections(PAGES);
    for (i, p) in PAGES.iter().enumerate() {
        let anchors = scan(p.markdown).anchors;
        let indexed: BTreeSet<String> = sections.iter().filter(|s| s.page == i).map(|s| s.anchor.clone()).collect();
        assert_eq!(anchors, indexed, "{}: every heading is a section of the index", p.file);
    }
    assert!(sections.iter().all(|s| !s.heading.trim().is_empty()), "every section has its heading");
    // search.js links only to `page.html#anchor` of this shape (its PAGE pattern).
    for s in &sections {
        let file = html_path(PAGES[s.page].file);
        let ok_file = file.ends_with(".html") && file.split('/').all(|part| !part.is_empty() && part.trim_end_matches(".html").chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'));
        let ok_anchor = s.anchor.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_');
        assert!(ok_file && ok_anchor, "{file}#{} would not be linked by search.js", s.anchor);
    }
    let json: serde_json::Value = serde_json::from_str(&search::index_json(PAGES, &sections)).expect("the index is JSON");
    assert_eq!(json["sections"].as_array().map(Vec::len), Some(sections.len()));
    assert_eq!(json["pages"].as_array().map(Vec::len), Some(PAGES.len()));
    // Text, not markup: the heading's words and the passage, no Markdown left.
    let onion = sections.iter().find(|s| s.anchor == "onion-skin").expect("onion skin");
    assert_eq!(onion.heading, "Onion skin");
    assert!(!onion.text.contains("**") && !onion.text.contains("]("), "{}", onion.text);
}

#[test]
fn search_ranks_the_section_you_mean_first() {
    for (query, first) in [
        ("onion skin", "timeline.html#onion-skin"),
        ("Onion Skin!", "timeline.html#onion-skin"),
        ("export gif", "getting-started.html#export-a-gif-or-a-video"),
        ("motion editor", "tweens.html#the-motion-editor"),
        ("pen", "drawing.html#the-pen"),
        ("how do I add a camera", "masks-guides-camera-bones.html#adding-a-camera"),
        ("shortcuts", "preferences-shortcuts-accessibility.html#keyboard-shortcuts"),
        ("mask", "masks-guides-camera-bones.html#making-a-mask"),
        ("find and replace", "workspace.html#find-and-replace"),
    ] {
        let got = top(query, 5);
        assert_eq!(got.first().map(String::as_str), Some(first), "{query:?} gave {got:?}");
    }
    assert!(top("zzzzqqq", 5).is_empty());
    assert!(top("", 5).is_empty() && top("  ¿? ", 5).is_empty());
    // Every word must match: two unrelated words find less than either alone.
    assert!(top("onion gif", 50).len() < top("onion", 50).len());
}

#[test]
fn query_words_and_matching() {
    assert_eq!(search::terms("How do I add a Camera?"), ["add", "camera"]);
    assert_eq!(search::terms("the"), ["the"], "a query of common words only still searches");
    assert_eq!(search::terms("tween tween TWEEN"), ["tween"]);
    assert!(search::terms(&"word ".repeat(1000)).len() <= search::MAX_TERMS);
    assert_eq!(search::match_class("tween", "tween"), 3);
    assert_eq!(search::match_class("tween", "tweens"), 2);
    assert_eq!(search::match_class("masks", "mask"), 1);
    assert_eq!(search::match_class("t", "tween"), 0, "one letter matches only itself");
    assert_eq!(search::match_class("maskingtape", "mask"), 0);
    let text = "word ".repeat(40) + "needle " + &"after ".repeat(60);
    let s = search::snippet(&text, &["needle".to_string()]);
    assert!(s.starts_with('…') && s.ends_with('…') && s.contains("needle"), "{s}");
    assert!(s.chars().count() <= 182, "{} chars", s.chars().count());
    assert_eq!(search::snippet("short text", &["zzz".to_string()]), "short text");
}

#[test]
fn search_results_carry_chapter_heading_and_passage() {
    let hits = search::search(PAGES, &search::sections(PAGES), "onion skin", 3);
    let h = hits.first().expect("a hit");
    assert_eq!((h.chapter.as_str(), h.heading.as_str()), ("The timeline", "Onion skin"));
    assert!(h.snippet.to_lowercase().contains("onion"), "{}", h.snippet);
    // A chapter's title links to its page.
    let hits = search::search(PAGES, &search::sections(PAGES), "tweens", 20);
    assert!(hits.iter().any(|h| h.url == "tweens.html"), "{:?}", hits.iter().map(|h| &h.url).collect::<Vec<_>>());
}

/// A result as both searches report it: url, heading, chapter, snippet, score.
type Row = (String, String, String, String, u32);

/// `search.js` ranks as `search.rs` does: run it under Node, when it is installed, on the same
/// index and queries.
#[test]
fn the_browser_search_gives_the_same_results_as_the_cli() {
    let Ok(out) = std::process::Command::new("node").arg("--version").output() else {
        eprintln!("node is not installed: the browser search is not compared");
        return;
    };
    if !out.status.success() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("anicraft-guide-js-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    std::fs::write(dir.join("search.js"), SEARCH_JS).expect("writes");
    std::fs::write(dir.join("index.json"), search::index_json(PAGES, &search::sections(PAGES))).expect("writes");
    let queries = ["onion skin", "export gif", "bones", "how do I add a camera", "masks", "Préférences", "the", "ctrl k", "zzzz", "swatch colour", "9-slice"];
    std::fs::write(dir.join("queries.json"), serde_json::to_string(&queries).expect("json")).expect("writes");
    let script = "const s = require('./search.js'); const fs = require('fs');\
        const index = JSON.parse(fs.readFileSync('index.json', 'utf8'));\
        const qs = JSON.parse(fs.readFileSync('queries.json', 'utf8'));\
        console.log(JSON.stringify(qs.map(q => s.search(index, q, 10).map(h => [h.url, h.heading, h.chapter, h.snippet, h.score]))));";
    let out = std::process::Command::new("node").arg("-e").arg(script).current_dir(&dir).output().expect("node runs");
    let _ = std::fs::remove_dir_all(&dir);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let js: Vec<Vec<Row>> = serde_json::from_slice(&out.stdout).expect("node prints JSON");
    let sections = search::sections(PAGES);
    for (q, js) in queries.iter().zip(js) {
        let rust: Vec<Row> = search::search(PAGES, &sections, q, 10).into_iter().map(|h| (h.url, h.heading, h.chapter, h.snippet, h.score)).collect();
        assert_eq!(rust, js, "search.js and search.rs differ for {q:?}");
    }
}

#[test]
fn the_site_has_a_search_page_and_every_page_a_search_field() {
    let site = site();
    for path in [SEARCH_PAGE, SEARCH_SCRIPT, SEARCH_INDEX] {
        assert!(site.iter().any(|f| f.path == path), "no {path}");
    }
    for f in site.iter().filter(|f| f.path.ends_with(".html")) {
        let text = String::from_utf8_lossy(&f.bytes);
        assert!(text.contains("role=\"search\"") && text.contains("search.js\"></script>"), "{} has no search field", f.path);
    }
    let nested = site.iter().find(|f| f.path == "reference/commands.html").expect("a nested page");
    assert!(String::from_utf8_lossy(&nested.bytes).contains("action=\"../search.html\""));
    let page = render_search_page(PAGES);
    assert!(page.contains("src=\"search-index.js\"") && page.contains("role=\"status\"") && page.contains("aria-live"));
    // The search script never builds markup from the index's text.
    assert!(!SEARCH_JS.contains("innerHTML") && !SEARCH_JS.contains("insertAdjacentHTML") && !SEARCH_JS.contains("document.write"));
    let js = search::index_js(PAGES);
    assert!(js.len() < 512 * 1024, "the index is {} KB", js.len() / 1024);
}

#[test]
fn context_help_targets_are_checked_and_named() {
    assert!(check_targets(PAGES, &["timeline.md#onion-skin", "drawing.md", "reference/commands.md#file"]).is_empty());
    let problems = check_targets(PAGES, &["timeline.md#nope", "nowhere.md#x"]).join("\n");
    assert!(problems.contains("has no heading #nope") && problems.contains("nowhere.md is not a page"), "{problems}");
    assert_eq!(heading_of(PAGES, "timeline.md#onion-skin").as_deref(), Some("Onion skin"));
    assert_eq!(heading_of(PAGES, "timeline.md").as_deref(), Some("The timeline"));
    assert_eq!(heading_of(PAGES, "timeline.md#nope"), None);
    assert_eq!(target_html("timeline.md#onion-skin"), "timeline.html#onion-skin");
    let r = redirect_page("timeline.html#a\"b");
    assert!(r.contains("url=timeline.html#a&quot;b") && !r.contains("a\"b"), "{r}");
}
