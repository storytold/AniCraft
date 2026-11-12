//! The self-authored XFL corpus (`tests/corpus/`, ADR 0035): each package imports with exactly
//! the report it should, keeps its structure, and survives being cut short or garbled. The
//! golden renders of the same packages are in `crates/engine/tests/xfl_corpus.rs` (rendering
//! is a layer this crate may not use).

use anicraft_doc::{Document, Easing, ElementKind, LayerKind, LoopMode, SymbolKind, TweenKind, evaluate};
use anicraft_text::Fonts;
use anicraft_xfl::{Report, XflError, import_with_fonts};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus")
}

/// Every package in the corpus, by folder name.
fn cases() -> Vec<String> {
    let mut out: Vec<String> = std::fs::read_dir(corpus()).unwrap().flatten().filter(|e| e.path().is_dir()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    out.sort();
    out
}

/// A package's files by their path inside it (`DOMDocument.xml`, `LIBRARY/x.xml`), without
/// its golden renders.
fn files(case: &str) -> BTreeMap<String, Vec<u8>> {
    fn walk(dir: &Path, prefix: &str, out: &mut BTreeMap<String, Vec<u8>>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let path = if prefix.is_empty() { name.clone() } else { format!("{prefix}/{name}") };
            if e.path().is_dir() {
                if path != "golden" {
                    walk(&e.path(), &path, out);
                }
            } else {
                out.insert(path, std::fs::read(e.path()).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(&corpus().join(case), "", &mut out);
    out
}

fn import_files(files: &BTreeMap<String, Vec<u8>>, fonts: &Fonts) -> Result<(Document, Report), XflError> {
    let read = |p: &str| -> Result<Option<Vec<u8>>, XflError> { Ok(files.get(p).cloned()) };
    import_with_fonts(&read, fonts)
}

/// Imports a package without fonts, so the result is the same on every computer.
fn import(case: &str) -> (Document, Report) {
    import_files(&files(case), &Fonts::from_faces(Vec::new())).unwrap_or_else(|e| panic!("{case}: {e}"))
}

/// What each package's report says, imported without fonts. Every line is something the
/// package uses on purpose.
const REPORTS: &[(&str, &[(&str, usize)])] = &[
    ("bitmaps", &[("instances of missing bitmaps", 1), ("missing bitmap files", 1)]),
    ("colour", &[]),
    ("filters", &[("unknown filters", 1)]),
    (
        "gradients",
        &[
            ("elliptical radial gradients (drawn round)", 1),
            ("gradient and bitmap strokes (drawn in one colour)", 1),
            ("linear-RGB gradients (blended in sRGB)", 1),
            ("radial gradient focal points (drawn centred)", 1),
            ("repeating and reflecting gradients (drawn padded)", 2),
        ],
    ),
    ("guides", &[]),
    ("interactive", &[]),
    ("masks", &[]),
    ("nineslice", &[("9-slice guides that aren't usable (left out)", 1)]),
    ("primitives", &[]),
    ("scenes", &[("frame comments", 1)]),
    ("scripts", &[]),
    ("shapes", &[]),
    (
        "symbols",
        &[("instances of missing symbols", 1), ("missing library files", 1), ("reverse loop modes (played forward)", 1)],
    ),
    ("text", &[("text fields (no fonts on this computer)", 4)]),
    ("textfields", &[("text fields (no fonts on this computer)", 4)]),
    ("tweens", &[("motion tweens (kept as keyframes)", 1), ("tween spins (turned the shortest way)", 1)]),
];

/// What each package keeps without running (ADR 0013 tier 1: scripts outside the subset, and
/// class names); packages not listed keep nothing.
const KEPT: &[(&str, &[(&str, usize)])] = &[
    ("bitmaps", &[("library folders", 1)]),
    ("nineslice", &[("library folders", 1)]),
    ("scripts", &[("document class", 1), ("frame scripts in ActionScript 3.0", 1), ("linkage classes", 1)]),
    ("symbols", &[("library folders", 1)]),
    ("textfields", &[("text field variables (ActionScript 1.0 and 2.0)", 1)]),
];

/// The frame scripts each package brings that run (ADR 0013 tier 2: in the subset).
const RUNS: &[(&str, &[(&str, usize)])] = &[
    ("interactive", &[("frame scripts in ActionScript 3.0", 4)]),
    ("scenes", &[("frame scripts in ActionScript 3.0", 2)]),
    ("scripts", &[("frame scripts in ActionScript 3.0", 2)]),
    ("text", &[("dynamic text fields", 1), ("input text fields", 1)]),
    ("textfields", &[("dynamic text fields", 1), ("frame scripts in ActionScript 3.0", 1), ("input text fields", 3)]),
];

#[test]
fn every_package_has_a_report_expectation() {
    let listed: Vec<&str> = REPORTS.iter().map(|(c, _)| *c).collect();
    assert_eq!(cases(), listed, "add the new package to REPORTS here and to the golden frames in crates/engine/tests/xfl_corpus.rs");
}

#[test]
fn each_package_reports_exactly_what_it_uses_that_does_not_come_across() {
    for (case, want) in REPORTS {
        let (_, report) = import(case);
        let want: BTreeMap<String, usize> = want.iter().map(|(k, n)| (k.to_string(), *n)).collect();
        assert_eq!(report.skipped, want, "{case}");
        let kept: BTreeMap<String, usize> = KEPT.iter().find(|(c, _)| c == case).map(|(_, k)| k.iter().map(|(k, n)| (k.to_string(), *n)).collect()).unwrap_or_default();
        assert_eq!(report.kept, kept, "{case}: kept, not run");
        let runs: BTreeMap<String, usize> = RUNS.iter().find(|(c, _)| c == case).map(|(_, k)| k.iter().map(|(k, n)| (k.to_string(), *n)).collect()).unwrap_or_default();
        assert_eq!(report.runs, runs, "{case}: frame scripts that run");
        assert!(report.notes.is_empty() && report.missing_fonts.is_empty(), "{case}: {report:?}");
    }
}

#[test]
fn every_package_opens_from_its_folder() {
    for case in cases() {
        let (doc, _) = anicraft_xfl::import_path(&corpus().join(&case)).unwrap_or_else(|e| panic!("{case}: {e}"));
        let (mem, _) = import(&case);
        assert_eq!((doc.scenes.len(), doc.library.len(), doc.bitmaps.len(), doc.frame_count()), (mem.scenes.len(), mem.library.len(), mem.bitmaps.len(), mem.frame_count()), "{case}");
        // And through the .xfl file inside it, as Animate saves one.
        let (doc, _) = anicraft_xfl::import_path(&corpus().join(&case).join(format!("{case}.xfl"))).unwrap_or_else(|e| panic!("{case}: {e}"));
        assert_eq!(doc.scenes.len(), mem.scenes.len());
    }
}

fn layer_names(doc: &Document, scene: usize) -> Vec<&str> {
    doc.timeline(scene).layers.iter().map(|l| l.name.as_str()).collect()
}

#[test]
fn shapes_rebuild_every_fill_and_stroke() {
    let (doc, _) = import("shapes");
    assert_eq!((doc.stage.width, doc.stage.height, doc.stage.fps), (320, 180, 24.0));
    let fills = &doc.timeline(0).layers[1].keyframes[0].elements;
    let shapes: Vec<&anicraft_doc::Shape> = fills.iter().filter_map(|e| if let ElementKind::Shape(s) = &e.kind { Some(s) } else { None }).collect();
    assert_eq!(shapes.len(), 5, "four fills, then one stroke");
    let loops = |s: &anicraft_doc::Shape| s.path.flatten(&anicraft_geom::Affine::IDENTITY, 0.1).iter().filter(|p| p.closed).count();
    // Red is one loop (it shares an edge with blue); blue is a loop and its hole.
    assert_eq!(shapes.iter().take(4).map(|s| loops(s)).collect::<Vec<_>>(), [1, 2, 1, 1]);
    let b = shapes[0].path.bounds();
    assert!((b.min.x - 20.0).abs() < 0.01 && (b.max.x - 70.0).abs() < 0.01 && (b.max.y - 80.0).abs() < 0.01, "{b:?}");
    let tri = shapes[3].path.bounds();
    assert!((tri.min.x - 240.0).abs() < 0.01 && (tri.max.x - 300.0).abs() < 0.01 && (tri.min.y - 20.0).abs() < 0.01, "hex fixed point: {tri:?}");
    assert!(shapes[4].stroke.is_some() && shapes[4].fill.is_none());
    // The strokes layer: the zigzag split over two edges is one run.
    let strokes = &doc.timeline(0).layers[0].keyframes[0].elements;
    assert_eq!(strokes.len(), 6);
    let ElementKind::Shape(zig) = &strokes[0].kind else { panic!() };
    assert_eq!(zig.path.flatten(&anicraft_geom::Affine::IDENTITY, 0.1).len(), 1, "an edge starting where the last ended continues it");
    let ElementKind::Shape(gap) = &strokes[3].kind else { panic!() };
    assert_eq!(gap.path.flatten(&anicraft_geom::Affine::IDENTITY, 0.1).len(), 2, "a move away leaves a gap");
    // Caps, joints, the miter limit, dashes and dots come across (ADR 0046).
    use anicraft_doc::{Cap, Dash, Join, StrokeStyle};
    let style = |i: usize| match &strokes[i].kind {
        ElementKind::Shape(s) => s.stroke.as_ref().map(|st| st.style.clone()).unwrap(),
        _ => panic!(),
    };
    assert_eq!(style(0), StrokeStyle { cap: Cap::Butt, join: Join::Miter, miter_limit: 4.0, dash: None });
    assert_eq!(style(1), StrokeStyle { cap: Cap::Square, join: Join::Bevel, miter_limit: 3.0, dash: None });
    assert_eq!(style(2), StrokeStyle::default());
    assert_eq!(style(4), StrokeStyle { cap: Cap::Butt, dash: Dash::dashed(10.0, 5.0), ..StrokeStyle::default() });
    assert_eq!(style(5), StrokeStyle { dash: Dash::dotted(4.0), ..StrokeStyle::default() });
}

#[test]
fn symbols_keep_their_kinds_loop_modes_and_colour_effects() {
    let (doc, _) = import("symbols");
    let kind = |name: &str| doc.library.iter().find(|s| s.name == name).map(|s| s.kind);
    assert_eq!(kind("Spinner"), Some(SymbolKind::MovieClip), "no symbolType: a movie clip");
    assert_eq!(kind("Play Button"), Some(SymbolKind::Button));
    assert_eq!(kind("Blink"), Some(SymbolKind::Graphic));
    assert_eq!(doc.library.len(), 5);
    let loops = &doc.timeline(0).layers[2].keyframes[0].elements;
    let modes: Vec<(LoopMode, u32)> = loops.iter().filter_map(|e| if let ElementKind::Symbol { loop_mode, first_frame, .. } = &e.kind { Some((*loop_mode, *first_frame)) } else { None }).collect();
    assert_eq!(modes, [(LoopMode::Loop, 0), (LoopMode::PlayOnce, 1), (LoopMode::SingleFrame, 2), (LoopMode::PlayOnce, 3), (LoopMode::Loop, 0)]);
    let fx: Vec<ColorEffect> = doc.timeline(0).layers[0].keyframes[0].elements.iter().map(|e| e.color).collect();
    use anicraft_doc::{ColorEffect, ColorTransform, Rgba};
    assert_eq!(fx[0], ColorEffect::Alpha(0.4));
    assert_eq!(fx[1], ColorEffect::Tint { color: Rgba::hex(0x0000ff), amount: 0.7 });
    assert_eq!(fx[2], ColorEffect::Brightness(0.6));
    assert_eq!(fx[3], ColorEffect::Brightness(-0.5));
    assert_eq!(fx[4], ColorEffect::Advanced(ColorTransform { mul: [0.2, 1.0, 1.0, 0.8], add: [0.0, 120.0 / 255.0, 0.0, 0.0] }), "advanced: every channel");
    assert_eq!(doc.timeline(0).layers[1].keyframes[0].elements[0].name, "spinner", "a movie clip keeps its instance name");
}

#[test]
fn colour_effects_come_across_in_every_style() {
    use anicraft_doc::{ColorEffect, ColorTransform, Rgba};
    let (doc, _) = import("colour");
    let fx: Vec<ColorEffect> = doc.timeline(0).layers[0].keyframes[0].elements.iter().map(|e| e.color).collect();
    assert_eq!(fx[0], ColorEffect::None);
    assert_eq!(fx[1], ColorEffect::Brightness(0.5));
    assert_eq!(fx[2], ColorEffect::Tint { color: Rgba::hex(0xE9C46A), amount: 0.6 });
    assert_eq!(fx[3], ColorEffect::Advanced(ColorTransform { mul: [-1.0, 0.5, 1.0, 1.0], add: [1.0, 0.0, -60.0 / 255.0, 0.0] }));
    assert_eq!(fx[4], ColorEffect::Alpha(0.25));
    assert_eq!(fx[5], ColorEffect::Brightness(-0.4));
    let pair = doc.library.iter().find(|s| s.name == "Pair").expect("Pair");
    assert_eq!(pair.timeline.layers[0].keyframes[0].elements[0].color, ColorEffect::Tint { color: Rgba::hex(0xFF0000), amount: 0.5 });
    let ends = |l: usize| (doc.timeline(0).layers[l].keyframes[0].elements[0].color.style(), doc.timeline(0).layers[l].keyframes[1].elements[0].color.style());
    use anicraft_doc::ColorStyle;
    assert_eq!(ends(1), (ColorStyle::Tint, ColorStyle::Advanced));
    assert_eq!(ends(2), (ColorStyle::Brightness, ColorStyle::Brightness));
}

#[test]
fn tweens_keep_their_kinds_eases_and_hints() {
    let (doc, _) = import("tweens");
    let t = |l: usize| doc.timeline(0).layers[l].keyframes[0].tween.expect("a tween");
    assert_eq!((t(0).kind, t(0).easing), (TweenKind::Classic, Easing::Classic(-100.0)));
    assert_eq!(t(1).easing, Easing::CubicBezier(0.7, 0.0, 0.3, 1.0), "a one-segment custom ease is a cubic Bézier");
    assert_eq!((t(3).kind, t(3).easing), (TweenKind::Shape, Easing::Classic(50.0)));
    let hints = &doc.timeline(0).layers[3].keyframes[0].hints;
    assert_eq!(hints.len(), 1);
    assert!(hints[0].start.distance(anicraft_geom::Vec2::new(20.0, 120.0)) < 0.01 && hints[0].end.distance(anicraft_geom::Vec2::new(270.0, 115.0)) < 0.01, "{hints:?}");
    assert!(doc.timeline(0).layers[4].keyframes[0].tween.is_none(), "a motion tween stays a keyframe");
    assert_eq!(doc.frame_count(), 13);
}

#[test]
fn nineslice_guides_come_across_in_the_symbols_space() {
    let (doc, _) = import("nineslice");
    let grid = |name: &str| doc.library.iter().find(|s| s.name == name).and_then(|s| s.scale_grid);
    assert_eq!(grid("Panel"), Some(anicraft_doc::ScaleGrid { left: 10.0, right: 90.0, top: 10.0, bottom: 30.0 }));
    assert_eq!(grid("Pip"), None);
    assert_eq!(grid("Half"), None, "one guide alone isn't a grid");
    assert_eq!(doc.library.iter().find(|s| s.name == "Panel").map(|s| s.kind), Some(SymbolKind::MovieClip));
}

#[test]
fn masks_and_folders_link_up() {
    let (doc, _) = import("masks");
    let l = &doc.timeline(0).layers;
    assert_eq!(layer_names(&doc, 0), ["Spot", "Stripes", "Inside", "Window", "Sky", "Loose", "Ground"]);
    let kinds: Vec<LayerKind> = l.iter().map(|l| l.kind).collect();
    assert_eq!(kinds, [LayerKind::Mask, LayerKind::Normal, LayerKind::Folder, LayerKind::Mask, LayerKind::Normal, LayerKind::Normal, LayerKind::Normal]);
    let parents: Vec<Option<usize>> = l.iter().map(|x| x.parent.and_then(|p| l.iter().position(|y| y.id == p))).collect();
    assert_eq!(parents, [None, Some(0), None, Some(2), Some(3), Some(2), None]);
    let sym = &doc.library[0];
    assert_eq!(sym.timeline.layers[0].kind, LayerKind::Mask);
    assert_eq!(sym.timeline.layers[1].parent, Some(sym.timeline.layers[0].id), "a mask inside a symbol");
    // Three masks drawn: the scene's two and the symbol's, once per instance.
    assert_eq!(evaluate(&doc, 0).clip_parents.len(), 4);
}

#[test]
fn guides_guide_and_never_draw() {
    let (doc, _) = import("guides");
    let l = &doc.timeline(0).layers;
    assert_eq!((l[0].kind, l[1].kind), (LayerKind::Guide, LayerKind::Guide));
    assert_eq!((l[2].parent, l[3].parent), (Some(l[1].id), Some(l[1].id)), "both riders follow the arc");
    assert!(l[2].keyframes[0].tween.is_some_and(|t| t.orient));
    // Half-way the riders are on the arc, well above the straight line between their ends.
    let mid = evaluate(&doc, 6);
    let drawn: Vec<_> = mid.items.iter().filter(|i| !i.guide).collect();
    assert!(drawn.iter().any(|i| i.transform.translation().y < 100.0), "{:?}", drawn.iter().map(|i| i.transform.translation()).collect::<Vec<_>>());
}

#[test]
fn scenes_labels_and_scripts() {
    let (doc, _) = import("scenes");
    assert_eq!(doc.scenes.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Opening", "Credits", "Scene 3"]);
    let labels: Vec<&str> = doc.timeline(0).layers[1].keyframes.iter().map(|k| k.label.as_str()).collect();
    assert_eq!(labels, ["intro", "", "sunrise"], "a comment isn't a label; an anchor is");
    assert_eq!(doc.timeline(1).layers[0].keyframes[0].label, "credits");
    assert_eq!(doc.frame_count(), 6 + 4 + 1);
    assert_eq!(doc.stage.fps, 12.0);
    // The scripts are kept as written, comment included (compared without the line endings a
    // Windows checkout gives the file).
    let kept = |scene: usize, layer: usize, key: usize| doc.timeline(scene).layers[layer].keyframes[key].script.as_ref().and_then(|s| s.source()).map(|s| s.text.replace("\r\n", "\n"));
    assert_eq!(kept(0, 0, 1).as_deref(), Some("stop();\n// The sun is up; wait for a click."));
    assert_eq!(kept(1, 0, 1).as_deref(), Some(r#"gotoAndPlay("Opening", "intro");"#));
}

#[test]
fn scripts_linkage_and_the_document_class_come_across_verbatim() {
    let (doc, _) = import("scripts");
    let source = |tl: &anicraft_doc::Timeline, key: usize| tl.layers[0].keyframes[key].script.as_ref().and_then(|s| s.source()).cloned();
    let first = source(doc.timeline(0), 0).unwrap();
    assert_eq!(first.language, anicraft_doc::Language::ActionScript3);
    // Byte for byte what the CDATA holds: tabs, blank lines, < & ' " and all.
    let file = String::from_utf8(files("scripts")["DOMDocument.xml"].clone()).unwrap();
    let cdata = file.split("<![CDATA[").nth(1).and_then(|s| s.split("]]>").next()).unwrap();
    assert_eq!(first.text, cdata);
    assert!(first.text.contains("\tif (hero_mc.currentFrame < 2 && e.target != null) {") && first.lines() == 10, "{:?}", first.text);
    assert_eq!(source(doc.timeline(0), 1).unwrap().text.replace("\r\n", "\n"), "hero_mc.gotoAndStop(2);\ntrace(\"Walked > 0 & done\");");
    let hero = doc.library.iter().find(|s| s.name == "Hero").unwrap();
    assert_eq!(hero.kind, SymbolKind::MovieClip);
    assert_eq!(source(&hero.timeline, 1).unwrap().text, "stop();");
    let linkage = hero.linkage.as_ref().unwrap();
    assert_eq!((linkage.class.as_str(), linkage.base.as_str()), ("com.example.Hero", "flash.display.MovieClip"));
    assert!(doc.library.iter().find(|s| s.name == "Play Button").unwrap().linkage.is_none());
    assert_eq!(doc.document_class, "com.example.Main");
    // Nothing runs them: the movie still evaluates to its authored frames, and the instance
    // names the scripts use came across.
    let names: Vec<&str> = doc.timeline(0).layers[2].keyframes[0].elements.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["hero_mc", "play_btn"]);
    assert_eq!(evaluate(&doc, 4).items.len(), evaluate(&doc, 0).items.len() - 1);
}

#[test]
fn bitmaps_come_from_png_and_jpeg_files() {
    let (doc, _) = import("bitmaps");
    let b: Vec<(&str, u32, u32)> = doc.bitmaps.iter().map(|b| (b.name.as_str(), b.width, b.height)).collect();
    assert_eq!(b, [("checker.png", 16, 16), ("sunset.jpg", 32, 16)]);
    assert_eq!(evaluate(&doc, 0).items.iter().filter(|i| i.image.is_some()).count(), 5);
}

#[test]
fn filters_and_blend_modes_come_across() {
    let (doc, _) = import("filters");
    let filters: Vec<usize> = doc.timeline(0).layers[0].keyframes[0].elements.iter().map(|e| if let ElementKind::Symbol { filters, .. } = &e.kind { filters.len() } else { 99 }).collect();
    assert_eq!(filters, [1, 1, 1, 1, 1, 1, 1, 2, 0, 0, 1], "switched-off and unknown filters are left out");
    let blends: Vec<&str> = doc.timeline(0).layers[1].keyframes[0].elements.iter().filter_map(|e| if let ElementKind::Symbol { blend, .. } = &e.kind { Some(blend.key()) } else { None }).collect();
    assert_eq!(blends, ["multiply", "screen", "difference", "add", "subtract", "overlay", "hardlight", "invert"]);
}

#[test]
fn text_fields_become_blocks_with_this_computers_fonts() {
    let fonts = Fonts::get();
    if fonts.resolve("").is_none() {
        return; // A computer without fonts: the report test covers it.
    }
    let (doc, report) = import_files(&files("text"), fonts).unwrap();
    let blocks: Vec<(String, String, f32, bool)> = doc.timeline(0).layers[0].keyframes[0]
        .elements
        .iter()
        .filter_map(|e| if let ElementKind::Text(t) = &e.kind { Some((e.name.clone(), t.text.clone(), t.size, t.bold)) } else { None })
        .collect();
    let want = [("", "Corpus", 28.0, true), ("", "self-authored, line two", 12.0, false), ("", "A fixed-width field wraps its words onto lines.", 11.0, false), ("score", "Score 0042", 18.0, false), ("answer", "type here", 14.0, false)];
    assert_eq!(blocks.iter().map(|(n, t, s, b)| (n.as_str(), t.as_str(), *s, *b)).collect::<Vec<_>>(), want);
    assert_eq!(report.runs.get("dynamic text fields"), Some(&1), "fields run in Test Movie (ADR 0066)");
    assert!(!report.skipped.contains_key("text fields (no fonts on this computer)"));
    // The text fields package: fields with their boxes, kinds and names, laid out in the box.
    let (doc, _) = import_files(&files("textfields"), fonts).unwrap();
    let fields: Vec<(String, anicraft_doc::TextKind, f32, f32)> = doc.timeline(0).layers[1].keyframes[0]
        .elements
        .iter()
        .filter_map(|e| if let ElementKind::Text(t) = &e.kind { Some((e.name.clone(), t.kind(), t.box_width, t.box_height)) } else { None })
        .collect();
    use anicraft_doc::TextKind::{Dynamic, Input};
    assert_eq!(fields, [("score_txt".into(), Dynamic, 200.0, 28.0), ("name_txt".into(), Input, 200.0, 24.0), ("notes_txt".into(), Input, 140.0, 64.0), ("pw_txt".into(), Input, 120.0, 24.0)]);
}

/// A small deterministic generator (xorshift), so a failing variant can be found again.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// Bytes and strings that poke at the readers: markup, edge commands, numbers out of range.
const POISON: &[&str] = &["<", ">", "\"", "/>", "</", "&#xD;", "&bogus;", "!", "|", "[", "]", "#", "#FFFFFFFF.FF", "S9", "-", "1e39", "-nan", "inf", "99999999999", "-2147483649", " ", "\u{0}", "\u{2028}", "é", "parentLayerIndex=\"0\" ", "duration=\"100001\" ", "index=\"4294967295\" "];

/// `bytes` garbled one of several ways.
fn garble(bytes: &[u8], rng: &mut Rng) -> Vec<u8> {
    let mut out = bytes.to_vec();
    for _ in 0..1 + rng.below(4) {
        let at = rng.below(out.len() + 1);
        match rng.below(5) {
            // Overwrite a byte.
            0 if at < out.len() => out[at] = rng.next() as u8,
            // Insert poison.
            1 | 0 => {
                let p = POISON[rng.below(POISON.len())].as_bytes();
                out.splice(at..at, p.iter().copied());
            }
            // Delete a span.
            2 => {
                let end = (at + 1 + rng.below(64)).min(out.len());
                out.drain(at.min(end)..end);
            }
            // Repeat a span (deeper nesting, repeated layers and edges).
            3 => {
                let end = (at + 1 + rng.below(400)).min(out.len());
                let span: Vec<u8> = out[at.min(end)..end].to_vec();
                for _ in 0..1 + rng.below(3) {
                    out.splice(end..end, span.iter().copied());
                }
            }
            // Swap a digit for another, moving geometry and indices.
            _ => {
                if let Some(i) = (at..out.len()).find(|&i| out[i].is_ascii_digit()) {
                    out[i] = b'0' + rng.below(10) as u8;
                }
            }
        }
    }
    out
}

/// Imports a hostile variant: it may fail, but it must not panic, and what it imports must
/// evaluate.
fn survive(files: &BTreeMap<String, Vec<u8>>, fonts: &Fonts, what: &str) {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        if let Ok((doc, _)) = import_files(files, fonts) {
            let n = doc.frame_count();
            for f in [0, 1, n / 2, n.saturating_sub(1)] {
                let _ = evaluate(&doc, f);
            }
        }
    }));
    assert!(result.is_ok(), "panicked on {what}");
}

#[test]
fn hostile_variants_of_every_package_never_panic() {
    let fonts = Fonts::from_faces(Vec::new());
    let start = std::time::Instant::now();
    let mut variants = 0;
    for case in cases() {
        let original = files(&case);
        for (path, bytes) in original.iter().filter(|(p, _)| p.ends_with(".xml") || p.ends_with(".png") || p.ends_with(".jpg")) {
            let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ (path.len() as u64 * 0x1000_0001) ^ (case.len() as u64) << 32);
            // Cut short at evenly spaced points (and just after the start and before the end).
            let cuts = (0..24).map(|k| bytes.len() * k / 24).chain([1, bytes.len().saturating_sub(1)]);
            for cut in cuts {
                let mut f = original.clone();
                f.insert(path.clone(), bytes[..cut.min(bytes.len())].to_vec());
                survive(&f, &fonts, &format!("{case}/{path} cut at {cut}"));
                variants += 1;
            }
            for n in 0..96 {
                let mut f = original.clone();
                f.insert(path.clone(), garble(bytes, &mut rng));
                survive(&f, &fonts, &format!("{case}/{path} garbled #{n}"));
                variants += 1;
            }
        }
        // A package missing each of its files in turn.
        for path in original.keys() {
            let mut f = original.clone();
            f.remove(path);
            survive(&f, &fonts, &format!("{case} without {path}"));
            variants += 1;
        }
    }
    assert!(variants > 1000, "{variants}");
    eprintln!("{variants} hostile variants in {:?}", start.elapsed());
}

#[test]
fn interactive_scripts_run_in_the_player() {
    use anicraft_doc::player::{Event, NoScripts, start, step};
    use anicraft_geom::Vec2;
    let (doc, report) = import("interactive");
    assert!(report.summary().contains("Runs in Test Movie: 4 frame scripts in ActionScript 3.0."), "{}", report.summary());
    let door = doc.timeline(0).layers[2].keyframes[0].elements[0].id;
    // The first frame stops the movie and the door, and adds the listeners.
    let s = start(&doc, &mut NoScripts);
    assert!(s.output.is_empty(), "{:?}", s.output);
    assert_eq!((s.state.root.frame, s.state.root.playing), (0, false));
    assert_eq!(s.state.listeners.values().map(Vec::len).sum::<usize>(), 4);
    let mut st = step(&doc, &s.state, &[], &mut NoScripts).state;
    assert_eq!(st.root.frame, 0, "stopped");
    // Click the door: it opens (its own script traces), and the call outside the movie is
    // ignored.
    let at = Vec2::new(90.0, 90.0);
    let r = step(&doc, &st, &[Event::Move(at), Event::Down(at), Event::Up(at)], &mut NoScripts);
    assert_eq!(r.output, ["knock knock", "door open"]);
    assert_eq!(r.state.clips.get(&vec![door]).map(|c| (c.frame, c.playing)), Some((2, false)));
    st = r.state;
    // Over the button the door fades; off it, it comes back; a click goes to "end".
    let b = Vec2::new(240.0, 90.0);
    let r = step(&doc, &st, &[Event::Move(b)], &mut NoScripts);
    assert_eq!(r.state.props.get(&vec![door]).and_then(|o| o.alpha), Some(0.5));
    let r = step(&doc, &r.state, &[Event::Down(b), Event::Up(b)], &mut NoScripts);
    assert_eq!(r.output, ["The end: 10"]);
    assert_eq!((r.state.root.frame, r.state.root.playing), (9, false));
    let r = step(&doc, &r.state, &[Event::Move(Vec2::new(10.0, 10.0))], &mut NoScripts);
    assert_eq!(r.state.props.get(&vec![door]).and_then(|o| o.alpha), Some(1.0));
    // The scripts package: the frame with `if` is kept and doesn't run; the others run.
    let (doc, _) = import("scripts");
    let runs: Vec<bool> = doc.timeline(0).layers[0].keyframes.iter().filter_map(|k| k.script.as_ref().and_then(|s| s.source())).map(|s| s.runs()).collect();
    assert_eq!(runs, [false, true]);
}
