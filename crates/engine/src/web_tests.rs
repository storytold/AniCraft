//! Commands on the in-memory file store the web build uses (ADR 0036), run natively: the
//! browser's open (a file put in the store), save and exports (files taken back as downloads).

use crate::{Session, vfs};
use serde_json::json;
use std::path::{Path, PathBuf};

/// Runs `f` on a thread of its own that uses the in-memory store, as the browser does.
fn in_browser(f: impl FnOnce() + Send + 'static) {
    std::thread::spawn(move || {
        vfs::use_memory();
        f();
    })
    .join()
    .unwrap();
}

fn names(files: &[(PathBuf, Vec<u8>)]) -> Vec<String> {
    files.iter().map(|(p, _)| p.to_string_lossy().replace('\\', "/")).collect()
}

#[test]
fn save_downloads_and_a_picked_file_opens() {
    in_browser(|| {
        let mut s = Session::new(crate::doc::sample::demo());
        let frames = s.frame_count();
        s.execute("file.save", &json!({"path": "walk.anicraft"})).unwrap();
        let out = vfs::take_written();
        assert_eq!(names(&out), ["walk.anicraft"], "the temporary file was renamed into place");
        assert!(!Path::new("walk.anicraft").exists(), "nothing reached the disk");
        let bytes = out[0].1.clone();
        assert!(bytes.starts_with(b"{"));

        // The user picks it in the browser: the shell puts it in the store and opens it.
        let mut t = Session::default();
        vfs::put(Path::new("walk.anicraft"), bytes.clone()).unwrap();
        t.execute("file.open", &json!({"path": "walk.anicraft"})).unwrap();
        assert_eq!((t.frame_count(), t.path.as_deref(), t.is_dirty()), (frames, Some(Path::new("walk.anicraft")), false));
        assert!(vfs::take_written().is_empty(), "opening writes nothing, and the picked file is let go");

        // A restored autosave copy opens as unsaved work, with its name or without one.
        vfs::put(Path::new("recovered/s1/walk.anicraft"), bytes.clone()).unwrap();
        t.execute("file.open", &json!({"path": "recovered/s1/walk.anicraft", "unsaved": true, "confirm": true})).unwrap();
        assert_eq!((t.path.as_deref(), t.is_dirty()), (Some(Path::new("recovered/s1/walk.anicraft")), true));
        vfs::put(Path::new("recovered/s2/Untitled"), bytes).unwrap();
        t.execute("file.open", &json!({"path": "recovered/s2/Untitled", "unsaved": true, "untitled": true, "confirm": true})).unwrap();
        assert_eq!((t.path.as_deref(), t.is_dirty()), (None, true));
        assert!(t.execute("file.open", &json!({"path": "gone.anicraft"})).is_err(), "a missing file is an error, not a crash");
    });
}

#[test]
fn exports_come_back_as_files() {
    in_browser(|| {
        let mut s = Session::new(crate::doc::sample::demo());
        for (cmd, params) in [
            ("file.export_png", json!({"path": "frame.png"})),
            ("file.export_svg", json!({"path": "frame.svg"})),
            ("file.export_gif", json!({"path": "movie.gif", "to": 3})),
            ("file.export_apng", json!({"path": "movie.png", "to": 3})),
            ("file.export_webp", json!({"path": "movie.webp", "to": 3})),
            ("file.export_lottie", json!({"path": "movie.json"})),
            ("file.export_html", json!({"path": "movie.html"})),
            ("file.export_audio", json!({"path": "track.wav"})),
            ("file.export_sprite_sheet", json!({"path": "sheet.png", "to": 3})),
        ] {
            s.execute(cmd, &params).unwrap_or_else(|e| panic!("{cmd}: {e}"));
        }
        s.execute("file.export_png_sequence", &json!({"dir": "frames", "to": 2})).unwrap();
        let out = vfs::take_written();
        let n = names(&out);
        for want in ["frame.png", "frame.svg", "movie.gif", "movie.png", "movie.webp", "movie.json", "movie.html", "track.wav", "sheet.png", "sheet.json", "frames/frame_00001.png", "frames/frame_00003.png"] {
            assert!(n.iter().any(|x| x == want), "{want} missing from {n:?}");
        }
        assert!(out.iter().all(|(_, b)| !b.is_empty()));
        let png = &out.iter().find(|(p, _)| p == Path::new("frame.png")).unwrap().1;
        assert!(png.starts_with(b"\x89PNG"));
        let apng = &out.iter().find(|(p, _)| p == Path::new("movie.png")).unwrap().1;
        assert!(apng.starts_with(b"\x89PNG") && apng.windows(4).any(|w| w == b"acTL"), "an animated PNG");
        let webp = &out.iter().find(|(p, _)| p == Path::new("movie.webp")).unwrap().1;
        assert!(webp.starts_with(b"RIFF") && webp.windows(4).any(|w| w == b"ANMF"), "an animated WebP");
        let wav = &out.iter().find(|(p, _)| p == Path::new("track.wav")).unwrap().1;
        assert!(wav.starts_with(b"RIFF") && !n.iter().any(|x| x.ends_with(".tmp")));
        let err = s.execute("file.export_video", &json!({"path": "movie.webm"})).unwrap_err().to_string();
        assert!(err.contains("web browser"), "{err}");
        assert!(vfs::take_written().is_empty());
    });
}

#[test]
fn imports_read_picked_files() {
    in_browser(|| {
        let mut s = Session::new(crate::doc::sample::demo());
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect width="40" height="20" fill="#f00"/></svg>"##;
        vfs::put(Path::new("badge.svg"), svg.to_vec()).unwrap();
        let v = s.execute("file.import", &json!({"path": "badge.svg"})).unwrap();
        assert_eq!(v["elements"], 1);
        let png = crate::cmds_file::render_png(&s.doc, 0, 0.1).unwrap();
        vfs::put(Path::new("pic.png"), png).unwrap();
        let v = s.execute("file.import_to_library", &json!({"path": "pic.png"})).unwrap();
        assert!(v["bitmap"].as_u64().is_some());
        let err = s.execute("file.open", &json!({"path": "Scene.xfl", "confirm": true})).unwrap_err().to_string();
        assert!(err.contains(".fla"), "an XFL folder can't come from a browser: {err}");
    });
}
