//! Golden renders of the self-authored XFL corpus (`crates/xfl/tests/corpus/`, ADR 0035).
//!
//! Each package is imported (without fonts, so text is reported rather than drawn in whatever
//! fonts this computer has, and the renders match everywhere), chosen frames are rendered by
//! the CPU oracle at half size (160 × 90), and each is compared with the golden PNG in the
//! package's `golden/` folder. Probes then check, by hand-worked colours, what the golden
//! should show. `UPDATE_GOLDENS=1` rewrites the goldens (see `golden/mod.rs`).
//!
//! Each package also opens as a `.fla` archive, packed in every layout zip writers use, with
//! `file.open`, as the same document and the same golden frames (ADR 0054).
//!
//! The report and structure of each package, and its hostile variants, are checked in the xfl
//! crate (`crates/xfl/tests/corpus.rs`).

mod golden;
#[path = "../../xfl/tests/support/fla.rs"]
mod pack;

use anicraft_engine::Session;
use anicraft_engine::doc::Document;
use anicraft_engine::text::Fonts;
use serde_json::json;
use std::path::{Path, PathBuf};

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../xfl/tests/corpus")
}

/// The frames rendered for each package (frames count across scenes, from 0).
const FRAMES: &[(&str, &[u32])] = &[
    ("bitmaps", &[0]),
    ("colour", &[0, 6, 12]),
    ("filters", &[0]),
    ("gradients", &[0]),
    ("guides", &[0, 6, 12]),
    ("interactive", &[0, 9]),
    ("masks", &[0]),
    ("nineslice", &[0]),
    ("primitives", &[0]),
    ("scenes", &[0, 3, 7, 8, 10]),
    ("scripts", &[0, 4]),
    ("shapes", &[0]),
    ("sounds", &[0]),
    ("symbols", &[0, 5]),
    ("text", &[0]),
    ("textfields", &[0]),
    ("tweens", &[0, 6, 12]),
    ("video", &[0]),
];

/// Render scale: the corpus stages are 320 × 180.
const SCALE: f32 = 0.5;

/// What a frame must show at a stage point: (package, frame, x, y, colour).
const PROBES: &[(&str, u32, f32, f32, u32)] = &[
    // The door movie clip and the button, on the authored timeline (scripts never change it).
    ("interactive", 0, 90.0, 90.0, 0x8D5524),
    ("interactive", 9, 240.0, 90.0, 0x2A9D8F),
    // Both sides of a shared edge, the hole, the hex-coordinate triangle.
    ("shapes", 0, 45.0, 50.0, 0xD9452B),
    ("shapes", 0, 80.0, 30.0, 0x2B6CD9),
    ("shapes", 0, 95.0, 50.0, 0xF4F1EA),
    ("shapes", 0, 270.0, 60.0, 0xF29E1F),
    // ADR 0086: the sounds package's artwork under its sound layers, on its own stage colour.
    ("sounds", 0, 40.0, 60.0, 0x2A9D8F),
    ("sounds", 0, 160.0, 90.0, 0xE9C46A),
    ("sounds", 0, 10.0, 10.0, 0xF4F1EA),
    // The Screen Video's first frame at 1.5 times (20..116 × 20..92), its quarters the right
    // way up and round; the VP6 video draws nothing; the rectangle beside it.
    ("video", 0, 35.0, 35.0, 0xE63946),
    ("video", 0, 100.0, 35.0, 0x2A9D8F),
    ("video", 0, 35.0, 80.0, 0x2646C8),
    ("video", 0, 100.0, 80.0, 0xE9C46A),
    ("video", 0, 200.0, 50.0, 0x264653),
    ("video", 0, 10.0, 150.0, 0xFFFFFF),
    ("primitives", 0, 40.0, 50.0, 0xF2CC8F),
    ("primitives", 0, 210.0, 40.0, 0xE07A5F),
    ("primitives", 0, 6.0, 170.0, 0xFFFFFF),
    // The checker's transparent corner, then a teal and a cream square, four times as large.
    ("bitmaps", 0, 14.0, 14.0, 0xFFFFFF),
    ("bitmaps", 0, 34.0, 18.0, 0x2A9D8F),
    ("bitmaps", 0, 34.0, 34.0, 0xE9C46A),
    // Loop modes: frame 0, then frame 5.
    ("symbols", 0, 30.0, 40.0, 0xE63946),
    ("symbols", 0, 80.0, 40.0, 0x2A9D8F),
    ("symbols", 0, 130.0, 40.0, 0x457B9D),
    ("symbols", 0, 180.0, 40.0, 0xE9C46A),
    ("symbols", 5, 30.0, 40.0, 0x2A9D8F),
    ("symbols", 5, 80.0, 40.0, 0xE9C46A),
    ("symbols", 5, 130.0, 40.0, 0x457B9D),
    // The button's Up frame.
    ("symbols", 0, 82.0, 130.0, 0x2A9D8F),
    // Brightness +60% and -50% of #6D597A; Advanced: red ×20 %, green +120, at 80 % over white.
    ("symbols", 0, 230.0, 130.0, 0xC5BDCA),
    ("symbols", 0, 260.0, 130.0, 0x372D3D),
    ("symbols", 0, 290.0, 130.0, 0x44DA95),
    // Colour effects on #2A9D8F (ADR 0056): None, Brightness +50 %, Tint 60 % #E9C46A, Advanced
    // (red −100 % +255, green 50 %, blue −60), Alpha 25 % over white.
    ("colour", 0, 30.0, 40.0, 0x2A9D8F),
    ("colour", 0, 90.0, 40.0, 0x94CEC7),
    ("colour", 0, 150.0, 40.0, 0x9DB479),
    ("colour", 0, 210.0, 40.0, 0xD54F53),
    ("colour", 0, 270.0, 40.0, 0xCAE6E3),
    // Brightness −40 % around a 50 % red tint: both, exactly (not the stronger one alone).
    ("colour", 0, 60.0, 110.0, 0x592F2B),
    // Tint #264653 to Advanced: the ends, and halfway in the advanced space.
    ("colour", 0, 160.0, 105.0, 0x264653),
    ("colour", 6, 220.0, 105.0, 0x614771),
    ("colour", 12, 280.0, 105.0, 0x944C8D),
    // Brightness −100 % to +60 %: halfway is brightness −20 % (not the transforms' midpoint).
    ("colour", 0, 160.0, 152.0, 0x000000),
    ("colour", 6, 220.0, 152.0, 0x227E72),
    ("colour", 12, 280.0, 152.0, 0xAAD8D2),
    ("tweens", 0, 20.0, 25.0, 0xF4A261),
    ("tweens", 12, 300.0, 25.0, 0xF4A261),
    // Per-property custom eases (ADR 0081): halfway, position's three-segment curve has carried
    // the ball past its end (x 180) to about 214; at the end it is there, half as big again.
    ("tweens", 6, 214.0, 145.0, 0xF4A261),
    ("tweens", 12, 180.0, 145.0, 0xF4A261),
    // Masked stripes: inside the spot, and outside it.
    ("masks", 0, 60.0, 22.0, 0x264653),
    ("masks", 0, 60.0, 27.0, 0xFFFFFF),
    ("masks", 0, 10.0, 22.0, 0xFFFFFF),
    ("masks", 0, 145.0, 10.0, 0xFFFFFF),
    ("masks", 0, 270.0, 70.0, 0xCB997E),
    // 9-slice (ADR 0055): the stretched panel's top-left corner keeps its radius-10 curve (plain
    // scaling would leave the background there), its middle is the panel, and the graphic inside
    // it scales normally (a dot 2.8 times as wide).
    ("nineslice", 0, 27.0, 113.0, 0x2A9D8F),
    ("nineslice", 0, 137.0, 13.0, 0x2A9D8F),
    ("nineslice", 0, 100.0, 125.0, 0x2A9D8F),
    ("nineslice", 0, 151.0, 130.0, 0xE9C46A),
    ("nineslice", 0, 21.0, 111.0, 0xF1FAEE),
    // Guide artwork never draws.
    ("guides", 0, 5.0, 5.0, 0xFFFFFF),
    ("guides", 0, 160.0, 60.0, 0xFFFFFF),
    ("scenes", 0, 160.0, 165.0, 0xFCA311),
    ("scenes", 7, 160.0, 90.0, 0xE5E5E5),
    ("scenes", 8, 160.0, 90.0, 0xFCA311),
    ("scenes", 10, 160.0, 90.0, 0x14213D),
    // Kept scripts change nothing: the authored frames, the clip on its first frame.
    ("scripts", 0, 90.0, 90.0, 0xE76F51),
    ("scripts", 0, 240.0, 90.0, 0x2A9D8F),
    ("scripts", 0, 10.0, 170.0, 0x264653),
    ("scripts", 4, 160.0, 90.0, 0xE76F51),
    ("scripts", 4, 240.0, 90.0, 0xF4F1EA),
    ("text", 0, 300.0, 80.0, 0xE9EDC9),
    // Fields draw their box (white behind a border) with no fonts at all (ADR 0066).
    ("text", 0, 290.0, 134.0, 0xFFFFFF),
    ("textfields", 0, 100.0, 30.0, 0xFFFFFF),
    ("textfields", 0, 300.0, 170.0, 0xF4F1EA),
];

fn import(case: &str) -> Document {
    let dir = corpus().join(case);
    let read = |p: &str| -> Result<Option<Vec<u8>>, anicraft_xfl::XflError> {
        match std::fs::read(dir.join(p)) {
            Ok(b) => Ok(Some(b)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    };
    anicraft_xfl::import_with_fonts(&read, &Fonts::from_faces(Vec::new())).unwrap_or_else(|e| panic!("{case}: {e}")).0
}

fn render(doc: &Document, frame: u32) -> (u32, u32, Vec<u8>) {
    let pm = anicraft_render::render_frame(doc, frame, SCALE).unwrap();
    (pm.width, pm.height, pm.to_rgba8())
}

#[test]
fn every_package_has_golden_frames() {
    let mut cases: Vec<String> = std::fs::read_dir(corpus()).unwrap().flatten().filter(|e| e.path().is_dir()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    cases.sort();
    assert_eq!(cases, FRAMES.iter().map(|(c, _)| *c).collect::<Vec<_>>(), "give the new package its frames here and its report in crates/xfl/tests/corpus.rs");
}

#[test]
fn corpus_frames_match_their_goldens() {
    let mut failures = Vec::new();
    let mut checked = 0;
    let start = std::time::Instant::now();
    for (case, frames) in FRAMES {
        let doc = import(case);
        for &f in *frames {
            assert!(f < doc.frame_count(), "{case}: frame {f} of {}", doc.frame_count());
            let (w, h, rgba) = render(&doc, f);
            assert_eq!((w, h), (160, 90), "{case}");
            let golden = corpus().join(case).join("golden").join(format!("frame-{f:03}.png"));
            if let Err(e) = golden::check(&golden, &format!("{case}-frame-{f:03}"), w, h, &rgba, golden::DEFAULT) {
                failures.push(e);
            }
            checked += 1;
        }
    }
    eprintln!("{checked} corpus frames rendered and compared in {:?}", start.elapsed());
    assert!(failures.is_empty(), "{} of {checked} frames differ from their goldens:\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn corpus_frames_show_what_the_files_describe() {
    let mut wrong = Vec::new();
    for &(case, frame, x, y, want) in PROBES {
        let (w, _, rgba) = render(&import(case), frame);
        let (px, py) = ((x * SCALE) as u32, (y * SCALE) as u32);
        let i = ((py * w + px) * 4) as usize;
        let got = &rgba[i..i + 3];
        let want = [(want >> 16) as u8, (want >> 8) as u8, want as u8];
        if got.iter().zip(want).any(|(g, w)| g.abs_diff(w) > 6) {
            wrong.push(format!("{case} frame {frame} at ({x}, {y}): #{:02X}{:02X}{:02X}, want #{:02X}{:02X}{:02X}", got[0], got[1], got[2], want[0], want[1], want[2]));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// Opening a package is one command, like opening any file, and what it imports saves and
/// opens again as the same document.
#[test]
fn every_package_opens_with_file_open_and_round_trips_through_the_file_format() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("xfl-corpus-round-trip");
    std::fs::create_dir_all(&dir).unwrap();
    for (case, _) in FRAMES {
        let mut s = Session::default();
        let info = s.execute("file.open", &json!({"path": corpus().join(case).display().to_string()})).unwrap_or_else(|e| panic!("{case}: {e}"));
        assert!(info["imported"].is_string(), "{case}: {info}");
        assert!(s.path.is_none(), "an imported document is untitled");
        let path = dir.join(format!("{case}.anicraft"));
        // A rerun saves over the last run's file, which asks first (ADR 0004).
        s.execute("file.save", &json!({"path": path.display().to_string(), "confirm": true})).unwrap();
        let mut t = Session::default();
        t.execute("file.open", &json!({"path": path.display().to_string()})).unwrap();
        assert!(s.doc == t.doc, "{case}: the saved document opens differently");
    }
}

/// Animate's raw shapes open as raw shapes, each keyframe's merged into one drawing (ADR 0042),
/// and look as they did: the goldens still match. A raw outline is one stroke per side (ADR
/// 0031), so at a corner two round caps overlap where one round join was and their antialiased
/// edges add up: a few corner pixels may be darker (47 of the shapes frame's 14,400).
#[test]
fn raw_shapes_open_raw_merged_and_unchanged_to_the_eye() {
    let mut failures = Vec::new();
    let (mut raw_total, mut merged_total) = (0, 0);
    for (case, frames) in FRAMES.iter().filter(|(c, _)| !matches!(*c, "text" | "textfields")) {
        let mut s = Session::default();
        let info = s.execute("file.open", &json!({"path": corpus().join(case).display().to_string()})).unwrap_or_else(|e| panic!("{case}: {e}"));
        let raw = s.doc.scenes.iter().flat_map(|sc| sc.timeline.layers.iter()).chain(s.doc.library.iter().flat_map(|sym| sym.timeline.layers.iter())).flat_map(|l| l.keyframes.iter()).flat_map(|k| k.elements.iter()).filter(|e| matches!(&e.kind, anicraft_engine::doc::ElementKind::Shape(sh) if sh.merge)).count();
        raw_total += raw;
        // (Keyframes a tween joins keep their shapes as they came.)
        merged_total += info["merged_keyframes"].as_u64().unwrap_or(0);
        for &f in *frames {
            let (w, h, rgba) = render(&s.doc, f);
            let golden = corpus().join(case).join("golden").join(format!("frame-{f:03}.png"));
            let corners = golden::Tolerance { pixels: 0.005, ..golden::DEFAULT };
            if let Err(e) = golden::compare(&golden, &format!("{case}-merged-frame-{f:03}"), w, h, &rgba, corners) {
                failures.push(e);
            }
        }
    }
    assert!(raw_total > 0 && merged_total > 0, "the corpus has raw shapes, merged: {raw_total} shapes, {merged_total} keyframes");
    assert!(failures.is_empty(), "merged raw shapes look different:\n{}", failures.join("\n"));
}

/// Each package as a `.fla` file, in every zip layout (ADR 0054), opens with `file.open` as
/// the same document as its folder and renders the same golden frames.
#[test]
fn every_package_opens_from_a_fla_with_the_same_golden_frames() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("xfl-corpus-fla");
    std::fs::create_dir_all(&dir).unwrap();
    let mut failures = Vec::new();
    let mut checked = 0;
    for (case, frames) in FRAMES {
        let folder = corpus().join(case);
        let mut s = Session::default();
        let want = s.execute("file.open", &json!({"path": folder.display().to_string()})).unwrap_or_else(|e| panic!("{case}: {e}"));
        // Animate's own pointer file inside the folder opens it too.
        let mut x = Session::default();
        x.execute("file.open", &json!({"path": folder.join(format!("{case}.xfl")).display().to_string()})).unwrap_or_else(|e| panic!("{case}.xfl: {e}"));
        assert!(x.doc == s.doc, "{case}: the .xfl opens differently from its folder");
        let files = pack::package(&folder);
        for (n, v) in pack::VARIANTS.iter().enumerate() {
            let path = dir.join(format!("{case}-{n}.fla"));
            std::fs::write(&path, pack::pack(&files, v)).unwrap();
            let mut t = Session::default();
            let got = t.execute("file.open", &json!({"path": path.display().to_string()})).unwrap_or_else(|e| panic!("{case}, {}: {e}", v.name));
            assert_eq!(got["report"], want["report"], "{case}, {}", v.name);
            assert_eq!(got["named"], want["named"], "{case}, {}", v.name);
            assert!(t.doc == s.doc, "{case}, {}: the .fla opens differently from the folder", v.name);
        }
        // The goldens (imported without fonts, as they were made), rendered from the .fla in
        // its most unusual layout.
        let fla = pack::pack(&files, &pack::VARIANTS[pack::VARIANTS.len() - 1]);
        let (doc, _) = anicraft_xfl::import_fla_with_fonts(&fla, &Fonts::from_faces(Vec::new())).unwrap_or_else(|e| panic!("{case}: {e}"));
        for &f in *frames {
            let (w, h, rgba) = render(&doc, f);
            let golden = corpus().join(case).join("golden").join(format!("frame-{f:03}.png"));
            if let Err(e) = golden::compare(&golden, &format!("{case}-fla-frame-{f:03}"), w, h, &rgba, golden::DEFAULT) {
                failures.push(e);
            }
            checked += 1;
        }
    }
    assert!(failures.is_empty(), "{} of {checked} .fla frames differ from their goldens:
{}", failures.len(), failures.join("
"));
}

/// #226: each package as a `.fla` opens as the same document through every way in: File › Open,
/// the command line and a document the system hands over (all `file.open`), Open Recent and the
/// start screen's list (`file.open_recent`), a drop anywhere on the window (routed to
/// `file.open`), an upper-case extension, and a file a web browser hands over (`vfs`).
#[test]
fn every_entry_point_opens_the_corpus_as_a_fla() {
    use anicraft_engine::cmds_drop::{DropAction, DropKind, DropTarget, kind_of, route};
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("xfl-corpus-entry-points");
    std::fs::create_dir_all(&dir).unwrap();
    for (case, _) in FRAMES {
        let mut s = Session::default();
        s.execute("file.open", &json!({"path": corpus().join(case).display().to_string()})).unwrap_or_else(|e| panic!("{case}: {e}"));
        let bytes = pack::pack(&pack::package(&corpus().join(case)), &pack::VARIANTS[0]);
        let fla = dir.join(format!("{case}.fla"));
        std::fs::write(&fla, &bytes).unwrap();
        let upper = dir.join(format!("{case} COPY.FLA"));
        std::fs::write(&upper, &bytes).unwrap();
        let open = |path: &Path| {
            let mut t = Session::default();
            t.execute("file.open", &json!({"path": path.display().to_string()})).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            t
        };
        for path in [&fla, &upper] {
            let t = open(path);
            assert!(t.doc == s.doc, "{}: opens differently from its folder", path.display());
            assert!(t.path.is_none(), "{case}: imported untitled, so Save never writes over the Animate file");
        }
        // Open Recent reopens the Animate file it imported.
        let mut t = open(&fla);
        t.execute("file.new", &json!({})).unwrap();
        t.execute("file.open_recent", &json!({"index": 0, "confirm": true})).unwrap_or_else(|e| panic!("{case}: Open Recent: {e}"));
        assert!(t.doc == s.doc, "{case}: Open Recent opens it the same");
        // A drop anywhere opens it.
        for path in [&fla, &upper] {
            assert_eq!(kind_of(path), DropKind::Document, "{}", path.display());
            for target in [DropTarget::Stage, DropTarget::Timeline, DropTarget::Library, DropTarget::Window] {
                assert_eq!(route(kind_of(path), target), DropAction::Open, "{case} on {target:?}");
            }
        }
        // In a browser the file arrives in memory, on the thread that opens it (ADR 0036).
        let (name, data, want) = (format!("{case}.fla"), bytes.clone(), s.doc.clone());
        std::thread::spawn(move || {
            anicraft_engine::vfs::use_memory();
            anicraft_engine::vfs::put(Path::new(&name), data).unwrap();
            let mut w = Session::default();
            w.execute("file.open", &json!({"path": name})).unwrap_or_else(|e| panic!("{name} in a browser: {e}"));
            assert!(w.doc == want, "{name}: opens differently in a browser");
        })
        .join()
        .unwrap();
    }
}

/// ADR 0086: opening a document headless (the CLI and MCP) says which sounds and videos didn't
/// come across, by name, and the sounds that did are in the Library with their waveforms.
#[test]
fn file_open_names_the_sounds_and_videos_that_did_not_come_across() {
    let mut s = Session::default();
    let got = s.execute("file.open", &json!({"path": corpus().join("sounds").display().to_string()})).unwrap();
    assert_eq!(got["named"]["missing sound files"], json!(["lost.wav"]));
    assert!(got["report"].as_str().unwrap().contains("3 sounds in formats AniCraft can't decode (theme.aif: AIFF; voice.wav: WAV in IMA ADPCM; Sounds/loop: Animate's own sound data)"), "{}", got["report"]);
    assert_eq!(s.doc.sounds.len(), 2);
    assert!(s.doc.sounds.iter().all(|x| x.decoded().is_some_and(|d| d.pcm.frames() > 0)), "waveforms ready to draw");
    let got = s.execute("file.open", &json!({"path": corpus().join("video").display().to_string(), "confirm": true})).unwrap();
    assert_eq!(got["named"]["videos AniCraft can't play"], json!(["intro.flv: FLV, On2 VP6, 4.5 s", "gone.mp4: its file is missing, 1.0 s"]));
}
