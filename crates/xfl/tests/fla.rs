//! The XFL corpus as `.fla` archives (ADR 0054): every package, packed every way zip writers
//! lay one out, imports exactly as its folder does (same document, same report); hostile
//! archives built from the corpus never panic and fail with a readable error; and a generated
//! 50 MB `.fla` measures import time (`cargo test --release -p anicraft-xfl --test fla --
//! --ignored --nocapture`). The golden renders of the `.fla` forms are checked in
//! `crates/engine/tests/xfl_corpus.rs`.

#[path = "support/fla.rs"]
mod pack;

use anicraft_doc::{Document, evaluate};
use anicraft_text::Fonts;
use anicraft_xfl::{Report, XflError, import_fla_with_fonts, import_with_fonts};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus")
}

fn cases() -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(corpus()).unwrap().flatten().filter(|e| e.path().is_dir()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    out.sort();
    out
}

fn no_fonts() -> Fonts {
    Fonts::from_faces(Vec::new())
}

fn from_folder(files: &BTreeMap<String, Vec<u8>>, fonts: &Fonts) -> (Document, Report) {
    let read = |p: &str| -> Result<Option<Vec<u8>>, XflError> { Ok(files.get(p).cloned()) };
    import_with_fonts(&read, fonts).unwrap()
}

#[test]
fn every_package_imports_identically_from_every_fla_variant() {
    let fonts = no_fonts();
    let mut checked = 0;
    for case in cases() {
        let files = pack::package(&corpus().join(&case));
        let (doc, report) = from_folder(&files, &fonts);
        for v in pack::VARIANTS {
            let fla = pack::pack(&files, v);
            let (d, r) = import_fla_with_fonts(&fla, &fonts).unwrap_or_else(|e| panic!("{case}, {}: {e}", v.name));
            assert!(d == doc, "{case}, {}: the document differs from the folder's", v.name);
            assert_eq!(r, report, "{case}, {}: the report differs", v.name);
            checked += 1;
        }
    }
    assert_eq!(checked, cases().len() * pack::VARIANTS.len());
}

/// From disk, through the same entry point the app uses, with this computer's fonts: the
/// folder, its `.xfl` file, its `DOMDocument.xml`, the `.fla`, and the `.fla` renamed.
#[test]
fn every_package_opens_the_same_from_disk_in_every_form() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("xfl-fla-forms");
    std::fs::create_dir_all(&dir).unwrap();
    for case in cases() {
        let folder = corpus().join(&case);
        let (doc, report) = anicraft_xfl::import_path(&folder).unwrap_or_else(|e| panic!("{case}: {e}"));
        let fla = dir.join(format!("{case}.fla"));
        std::fs::write(&fla, pack::pack(&pack::package(&folder), &pack::VARIANTS[0])).unwrap();
        let renamed = dir.join(format!("{case} (copy).zip"));
        std::fs::copy(&fla, &renamed).unwrap();
        for path in [folder.join(format!("{case}.xfl")), folder.join("DOMDocument.xml"), fla, renamed] {
            let (d, r) = anicraft_xfl::import_path(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            assert!(d == doc && r == report, "{} opens differently from its folder", path.display());
        }
    }
}

/// Names outside ASCII, in every encoding: the symbols package with a library file renamed.
#[test]
fn non_ascii_names_import_identically_in_every_encoding() {
    let fonts = no_fonts();
    let mut files = pack::package(&corpus().join("symbols"));
    let doc_xml = String::from_utf8(files["DOMDocument.xml"].clone()).unwrap();
    let old = files.keys().find(|p| p.starts_with("LIBRARY/") && p.ends_with(".xml")).unwrap().clone();
    let href = old.trim_start_matches("LIBRARY/").to_string();
    let new_href = format!("Café ü/Ünïcödé ½ {href}");
    assert!(doc_xml.contains(&format!("href=\"{href}\"")), "{href}");
    files.insert("DOMDocument.xml".into(), doc_xml.replace(&format!("href=\"{href}\""), &format!("href=\"{new_href}\"")).into_bytes());
    let body = files.remove(&old).unwrap();
    files.insert(format!("LIBRARY/{new_href}"), body);
    let (doc, report) = from_folder(&files, &fonts);
    assert!(doc.library.len() >= 5, "the renamed symbol still imports");
    for v in pack::VARIANTS {
        let (d, r) = import_fla_with_fonts(&pack::pack(&files, v), &fonts).unwrap_or_else(|e| panic!("{}: {e}", v.name));
        assert!(d == doc && r == report, "{}", v.name);
    }
}

#[test]
fn an_old_binary_fla_says_what_to_do() {
    let mut old = vec![0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
    old.resize(4096, 0);
    let e = anicraft_xfl::import_fla(&old).err().unwrap().to_string();
    assert!(e.contains("Flash CS4 or earlier") && e.contains("save it again"), "{e}");
    let e = anicraft_xfl::import_fla(b"PK\x03\x04 cut short").err().unwrap().to_string();
    assert_eq!(e, "not a zip archive (no end-of-directory record)");
}

/// A small deterministic generator (xorshift64*).
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// Imports a hostile archive: it may fail, with a readable error, but must not panic, and
/// what it imports must evaluate.
fn survive(fla: &[u8], fonts: &Fonts, what: &str, errors: &mut BTreeMap<String, usize>) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match import_fla_with_fonts(fla, fonts) {
        Ok((doc, _)) => {
            let n = doc.frame_count();
            for f in [0, n / 2, n.saturating_sub(1)] {
                let _ = evaluate(&doc, f);
            }
            None
        }
        Err(e) => {
            // Grouped by kind for the summary: `Zip(Bomb`, `Xml(…`.
            let kind: String = format!("{e:?}").split('(').take(2).collect::<Vec<_>>().join("(");
            Some((e.to_string(), kind))
        }
    }));
    match result {
        Err(_) => panic!("panicked on {what}"),
        Ok(Some((e, kind))) => {
            assert!(e.len() > 10 && !e.contains("Error") && !e.contains('{'), "{what}: an unreadable error: {e:?}");
            *errors.entry(kind).or_default() += 1;
        }
        Ok(None) => *errors.entry("(imported)".into()).or_default() += 1,
    }
}

/// Offsets of every central record in `z` (found through the end record, or its ZIP64 form).
fn central_records(z: &[u8]) -> Vec<usize> {
    let end = z.len() - 22;
    let mut at = u32::from_le_bytes(z[end + 16..end + 20].try_into().unwrap()) as usize;
    if at == u32::MAX as usize {
        let record = u64::from_le_bytes(z[end - 12..end - 4].try_into().unwrap()) as usize;
        at = u64::from_le_bytes(z[record + 48..record + 56].try_into().unwrap()) as usize;
    }
    let mut out = Vec::new();
    let mut i = at;
    while z.get(i..i + 4) == Some(&[0x50, 0x4b, 1, 2]) {
        out.push(i);
        let n = |o: usize| u16::from_le_bytes([z[i + o], z[i + o + 1]]) as usize;
        i += 46 + n(28) + n(30) + n(32);
    }
    out
}

/// A change to an archive's entries before it is written.
type Edit = Box<dyn Fn(&mut Vec<pack::Raw>)>;

/// Each corpus package as a `.fla`, attacked: every header field of every entry set to edge
/// values, random bytes changed in the headers and anywhere, cut short, entries pointed at each
/// other, and names made unsafe.
#[test]
fn hostile_fla_archives_never_panic_and_fail_readably() {
    let fonts = no_fonts();
    let start = std::time::Instant::now();
    let mut errors = BTreeMap::new();
    let mut variants = 0;
    for case in cases() {
        let files = pack::package(&corpus().join(&case));
        for v in [&pack::VARIANTS[0], &pack::VARIANTS[7]] {
            let z = pack::pack(&files, v);
            let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ (case.len() as u64) << 20 ^ z.len() as u64);
            let records = central_records(&z);
            assert!(records.len() >= files.len(), "{case}");
            // Header fields at edge values: (offset in the central record, width).
            const FIELDS: &[(usize, usize)] = &[(8, 2), (10, 2), (16, 4), (20, 4), (24, 4), (28, 2), (30, 2), (32, 2), (34, 2), (42, 4)];
            for &rec in records.iter().take(6) {
                for &(off, width) in FIELDS {
                    for value in [0u32, 1, 0x7f, 0xffff, 0x7fff_ffff, 0xffff_ffff, rng.next() as u32] {
                        let mut t = z.clone();
                        t[rec + off..rec + off + width].copy_from_slice(&value.to_le_bytes()[..width]);
                        survive(&t, &fonts, &format!("{case}: record at {rec}, field {off} = {value:#x}"), &mut errors);
                        variants += 1;
                    }
                }
                // The local header's lengths too.
                let local = u32::from_le_bytes(z[rec + 42..rec + 46].try_into().unwrap()) as usize;
                for (off, value) in [(26usize, 0xffffu16), (28, 0xffff), (28, 0), (18, 0xffff), (22, 0)] {
                    let mut t = z.clone();
                    if let Some(f) = t.get_mut(local + off..local + off + 2) {
                        f.copy_from_slice(&value.to_le_bytes());
                    }
                    survive(&t, &fonts, &format!("{case}: local header at {local}, field {off}"), &mut errors);
                    variants += 1;
                }
            }
            // Random bytes in the central directory and end record, then anywhere.
            let dir_at = records.first().copied().unwrap_or(0);
            for n in 0..60 {
                let mut t = z.clone();
                let lo = if n < 40 { dir_at } else { 0 };
                for _ in 0..1 + rng.below(4) {
                    let i = lo + rng.below(t.len() - lo);
                    t[i] = rng.next() as u8;
                }
                survive(&t, &fonts, &format!("{case}: random bytes #{n}"), &mut errors);
                variants += 1;
            }
            // Cut short.
            for k in 0..24 {
                survive(&z[..z.len() * k / 24], &fonts, &format!("{case}: cut at {k}/24"), &mut errors);
                variants += 1;
            }
        }
        // Built hostile: names that leave the package, duplicates, overlaps, lying sizes.
        let mut entries = pack::entries(&files, &pack::VARIANTS[0]);
        let last = entries.len() - 1;
        let edits: Vec<(&str, Edit)> = vec![
            ("a .. name", Box::new(move |e| e[last].name = b"LIBRARY/../../../x.xml".to_vec())),
            ("an absolute name", Box::new(move |e| e[last].name = b"/etc/x.xml".to_vec())),
            ("a drive name", Box::new(move |e| e[last].name = b"C:\\x.xml".to_vec())),
            ("a duplicate", Box::new(|e| e.push(e[1].clone()))),
            ("a duplicate by separator", Box::new(|e| {
                let mut d = e[1].clone();
                d.name = d.name.iter().map(|&c| if c == b'/' { b'\\' } else { c }).collect();
                e.push(d);
            })),
            ("overlapping entries", Box::new(|e| {
                let mut d = e[1].clone();
                d.name = b"other.xml".to_vec();
                d.share = Some(1);
                e.push(d);
            })),
            ("a lying size", Box::new(|e| e[1].size = Some(3))),
            ("a lying checksum", Box::new(|e| e[1].crc = Some(1))),
            ("a ratio bomb", Box::new(|e| e[1].size = Some(200 << 20))),
            ("deep folders", Box::new(|e| e[1].name = "d/".repeat(200).into_bytes())),
        ];
        for (what, edit) in &edits {
            let mut t = entries.clone();
            edit(&mut t);
            let z = pack::write(&t, false);
            let before = errors.get("(imported)").copied();
            survive(&z, &fonts, &format!("{case}: {what}"), &mut errors);
            assert_eq!(errors.get("(imported)").copied(), before, "{case}: {what} imported");
            variants += 1;
        }
        entries.clear();
    }
    assert!(variants > 5000, "{variants}");
    eprintln!("{variants} hostile .fla archives in {:?}: {errors:#?}", start.elapsed());
}

/// A generated `.fla` of about 50 MB: 400 symbols with long edge lists (deflated XML) and 24
/// noisy PNG bitmaps (stored), placed on a 240-frame timeline.
fn big_fla() -> (Vec<u8>, usize) {
    let mut files = BTreeMap::new();
    let mut rng = Rng(42);
    let mut includes = String::new();
    for s in 0..400 {
        let mut edges = String::new();
        for e in 0..600 {
            let (x, y) = (rng.below(20_000), rng.below(20_000));
            edges.push_str(&format!("<Edge fillStyle1=\"1\" edges=\"!{x} {y}|{} {}|{} {}\"/>", x + 200, y + e, x + 100, y + 300));
        }
        let xml = format!("<DOMSymbolItem name=\"Symbol {s}\"><timeline><DOMTimeline name=\"Symbol {s}\"><layers><DOMLayer name=\"L\"><frames><DOMFrame index=\"0\"><elements><DOMShape><fills><FillStyle index=\"1\"><SolidColor color=\"#{:06X}\"/></FillStyle></fills><edges>{edges}</edges></DOMShape></elements></DOMFrame></frames></DOMLayer></layers></DOMTimeline></timeline></DOMSymbolItem>", rng.next() & 0xffffff);
        files.insert(format!("LIBRARY/Symbols/Symbol {s}.xml"), xml.into_bytes());
        includes.push_str(&format!("<Include href=\"Symbols/Symbol {s}.xml\"/>"));
    }
    let mut media = String::new();
    for b in 0..24 {
        let (w, h) = (1024u32, 512u32);
        let pixels: Vec<u8> = (0..w * h * 4).map(|_| rng.next() as u8).collect();
        let mut png = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut png, w, h);
            enc.set_color(png::ColorType::Rgba);
            enc.set_compression(png::Compression::Fast);
            let mut wr = enc.write_header().unwrap();
            wr.write_image_data(&pixels).unwrap();
        }
        files.insert(format!("LIBRARY/Images/noise {b}.png"), png);
        media.push_str(&format!("<DOMBitmapItem name=\"Images/noise {b}.png\" href=\"Images/noise {b}.png\"/>"));
    }
    let mut frames = String::new();
    for f in 0..240 {
        frames.push_str(&format!("<DOMFrame index=\"{f}\"><elements><DOMSymbolInstance libraryItemName=\"Symbol {}\"/><DOMBitmapInstance libraryItemName=\"Images/noise {}.png\"/></elements></DOMFrame>", f % 400, f % 24));
    }
    let doc = format!("<DOMDocument width=\"1920\" height=\"1080\" frameRate=\"24\"><media>{media}</media><symbols>{includes}</symbols><timelines><DOMTimeline name=\"Scene 1\"><layers><DOMLayer name=\"Layer 1\"><frames>{frames}</frames></DOMLayer></layers></DOMTimeline></timelines></DOMDocument>");
    files.insert("DOMDocument.xml".into(), doc.into_bytes());
    let n = files.len();
    (pack::pack(&files, &pack::VARIANTS[2]), n)
}

#[test]
#[ignore = "a measurement: run with --release -- --ignored --nocapture"]
fn measure_importing_a_50_mb_fla() {
    let (fla, files) = big_fla();
    let fonts = no_fonts();
    let mut times = Vec::new();
    for _ in 0..5 {
        let t = std::time::Instant::now();
        let a = anicraft_xfl::zip::Archive::open(&fla).unwrap();
        let open = t.elapsed();
        let names: Vec<String> = a.names().map(str::to_string).collect();
        let t = std::time::Instant::now();
        let bytes: usize = names.iter().map(|n| a.read(n).unwrap().map_or(0, |b| b.len())).sum();
        let read = t.elapsed();
        let t = std::time::Instant::now();
        let (doc, _) = import_fla_with_fonts(&fla, &fonts).unwrap();
        let import = t.elapsed();
        assert_eq!((doc.library.len(), doc.bitmaps.len()), (400, 24));
        times.push((open, read, bytes, import));
    }
    eprintln!("{:.1} MB .fla, {files} files:", fla.len() as f64 / 1e6);
    for (open, read, bytes, import) in times {
        eprintln!("  open {open:?}, read every entry {read:?} ({:.1} MB, {:.0} MB/s), import {import:?}", bytes as f64 / 1e6, bytes as f64 / 1e6 / read.as_secs_f64());
    }
}
