//! Golden renders of the self-authored XFL corpus (`crates/xfl/tests/corpus/`, ADR 0035).
//!
//! Each package is imported (without fonts, so text is reported rather than drawn in whatever
//! fonts this computer has, and the renders match everywhere), chosen frames are rendered by
//! the CPU oracle at half size (160 × 90), and each is compared with the golden PNG in the
//! package's `golden/` folder. Probes then check, by hand-worked colours, what the golden
//! should show. `UPDATE_GOLDENS=1` rewrites the goldens (see `golden/mod.rs`).
//!
//! The report and structure of each package, and its hostile variants, are checked in the xfl
//! crate (`crates/xfl/tests/corpus.rs`).

mod golden;

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
    ("filters", &[0]),
    ("gradients", &[0]),
    ("guides", &[0, 6, 12]),
    ("masks", &[0]),
    ("primitives", &[0]),
    ("scenes", &[0, 3, 7, 8, 10]),
    ("scripts", &[0, 4]),
    ("shapes", &[0]),
    ("symbols", &[0, 5]),
    ("text", &[0]),
    ("tweens", &[0, 6, 12]),
];

/// Render scale: the corpus stages are 320 × 180.
const SCALE: f32 = 0.5;

/// What a frame must show at a stage point: (package, frame, x, y, colour).
const PROBES: &[(&str, u32, f32, f32, u32)] = &[
    // Both sides of a shared edge, the hole, the hex-coordinate triangle.
    ("shapes", 0, 45.0, 50.0, 0xD9452B),
    ("shapes", 0, 80.0, 30.0, 0x2B6CD9),
    ("shapes", 0, 95.0, 50.0, 0xF4F1EA),
    ("shapes", 0, 270.0, 60.0, 0xF29E1F),
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
    // Brightness +60% and -50% of #6D597A.
    ("symbols", 0, 230.0, 130.0, 0xC5BDCA),
    ("symbols", 0, 260.0, 130.0, 0x372D3D),
    ("tweens", 0, 20.0, 25.0, 0xF4A261),
    ("tweens", 12, 300.0, 25.0, 0xF4A261),
    // Masked stripes: inside the spot, and outside it.
    ("masks", 0, 60.0, 22.0, 0x264653),
    ("masks", 0, 60.0, 27.0, 0xFFFFFF),
    ("masks", 0, 10.0, 22.0, 0xFFFFFF),
    ("masks", 0, 145.0, 10.0, 0xFFFFFF),
    ("masks", 0, 270.0, 70.0, 0xCB997E),
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
