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
