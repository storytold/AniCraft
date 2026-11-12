//! Publish Settings and Publish (ADR 0060): the profile in the file, where outputs go, the
//! plan, running every exporter with its options, cancelling, the combined report, the
//! confirmation, the browser's archive.

use super::*;
use crate::{EngineError, Session, vfs};
use anicraft_doc::sample;
use serde_json::json;
use std::path::{Path, PathBuf};

fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("anicraft-publish-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

/// The demo, saved as `walk.anicraft` in a fresh folder.
fn saved(name: &str) -> (Session, PathBuf) {
    let dir = temp(name);
    let mut s = Session::new(sample::demo());
    run(&mut s, "file.save", json!({"path": dir.join("walk.anicraft").display().to_string()}));
    (s, dir)
}

#[test]
fn templates_resolve_inside_the_documents_folder() {
    let r = |f, t: &str| resolve(f, t, "walk");
    assert_eq!(r(PublishFormat::Html, "{name}.html").unwrap(), PathBuf::from("walk.html"));
    assert_eq!(r(PublishFormat::Gif, "web/{name}-small.gif").unwrap(), Path::new("web").join("walk-small.gif"));
    assert_eq!(r(PublishFormat::Gif, "./{name}").unwrap(), PathBuf::from("walk.gif"), "a missing extension is the format's");
    assert_eq!(r(PublishFormat::Video, "{name}.MP4").unwrap(), PathBuf::from("walk.MP4"));
    assert_eq!(r(PublishFormat::PngSequence, "{name}_frames").unwrap(), PathBuf::from("walk_frames"), "a folder has no extension");
    for bad in ["../{name}.gif", "/tmp/{name}.gif", "web/../../x.gif", "{title}.gif", "{name.gif", "  ", "{name}.png"] {
        assert!(r(PublishFormat::Gif, bad).is_err(), "{bad}");
    }
    #[cfg(windows)]
    assert!(r(PublishFormat::Gif, "C:\\x.gif").is_err() && r(PublishFormat::Gif, "C:x.gif").is_err());
    assert!(r(PublishFormat::Gif, &"a".repeat(600)).is_err());
    // The folder and name come from the document's path.
    assert_eq!(base(Some(Path::new("art/walk.anicraft"))).unwrap(), (PathBuf::from("art"), "walk".to_string()));
    assert!(base(None).is_err(), "an untitled document on disk saves first");
}

#[test]
fn settings_round_trip_through_the_file_and_older_files_get_defaults() {
    let (mut s, dir) = saved("roundtrip");
    let v = run(&mut s, "publish.settings_set", json!({"format": "gif", "enabled": true, "output": "web/{name}.gif", "scale": 0.5, "loop": false, "from": 2, "to": 9}));
    let gif = v["formats"].as_array().unwrap().iter().find(|f| f["format"] == "gif").unwrap();
    assert_eq!(gif["path"], json!(dir.join("web").join("walk.gif").display().to_string()));
    run(&mut s, "publish.settings_set", json!({"format": "html", "title": "Walk cycle", "sound": false}));
    run(&mut s, "publish.settings_set", json!({"format": "audio", "rate": 22050, "bits": 24, "to": null}));
    let path = dir.join("walk.anicraft");
    run(&mut s, "file.save", json!({}));
    let back = crate::format::load(&path).unwrap();
    assert_eq!(*back.publish, *s.doc.publish);
    let p = &back.publish;
    assert!(p.gif.enabled && !p.gif.looping && p.gif.scale == 0.5 && p.gif.range == FrameRange { from: Some(2), to: Some(9) });
    assert_eq!((p.html.title.as_str(), p.html.sound, p.audio.rate, p.audio.bits), ("Walk cycle", false, 22050, 24));
    // An untouched profile isn't written at all, so files from before publish settings look
    // the same: they open with the HTML page on and the rest off.
    let mut fresh = Session::new(sample::demo());
    let text = serde_json::to_string(&fresh.doc).unwrap();
    assert!(!text.contains("\"publish\""));
    let older: anicraft_doc::Document = serde_json::from_str(&text).unwrap();
    assert_eq!(older.publish.enabled(), vec![PublishFormat::Html]);
    // Settings are a document change: undo takes them back.
    run(&mut fresh, "publish.settings_set", json!({"format": "svg", "enabled": true}));
    assert!(fresh.is_dirty() && fresh.doc.publish.svg.enabled);
    run(&mut fresh, "edit.undo", json!({}));
    assert!(!fresh.doc.publish.svg.enabled);
    // A whole profile, as publish.settings_get gives it.
    let whole = run(&mut s, "publish.settings_get", json!({}))["profile"].clone();
    run(&mut fresh, "publish.settings_set", json!({"profile": whole}));
    assert_eq!(*fresh.doc.publish, *s.doc.publish);
}

#[test]
fn settings_refuse_what_publish_couldnt_use() {
    let mut s = Session::new(sample::demo());
    let before = s.doc.publish.clone();
    for bad in [
        json!({"format": "flash"}),
        json!({"format": "gif", "rate": 44100}),
        json!({"format": "gif", "scale": 0}),
        json!({"format": "gif", "scale": 9}),
        json!({"format": "gif", "from": 9, "to": 2}),
        json!({"format": "gif", "output": "../x.gif"}),
        json!({"format": "gif", "output": "x.png"}),
        json!({"format": "audio", "bits": 8}),
        json!({"format": "audio", "rate": 1}),
        json!({"format": "sprite_sheet", "padding": 999}),
        json!({"format": "html", "title": "x".repeat(300)}),
        json!({"format": "html", "enabled": "yes"}),
        json!({"enabled": true}),
        json!({"profile": 5}),
        json!({"profile": {"gif": {"scale": "big"}}}),
    ] {
        assert!(s.execute("publish.settings_set", &bad).is_err(), "{bad}");
    }
    assert_eq!(s.doc.publish, before, "a refused change changes nothing");
    assert!(!s.is_dirty());
}

#[test]
fn the_plan_runs_each_exporter_with_its_options() {
    let mut doc = sample::demo();
    let p = std::sync::Arc::make_mut(&mut doc.publish);
    p.gif.enabled = true;
    p.gif.scale = 0.25;
    p.gif.looping = false;
    p.gif.range = FrameRange { from: Some(1), to: Some(4) };
    p.sprite_sheet.enabled = true;
    p.sprite_sheet.columns = 2;
    p.svg.enabled = true;
    p.svg.frame = 5;
    let steps = plan(&doc, Some(Path::new("art/walk.anicraft")), None).unwrap();
    let order: Vec<PublishFormat> = steps.iter().map(|s| s.format).collect();
    assert_eq!(order, [PublishFormat::Html, PublishFormat::Gif, PublishFormat::SpriteSheet, PublishFormat::Svg], "in Publish's order");
    let gif = &steps[1];
    assert_eq!(gif.command, "file.export_gif");
    assert_eq!((gif.params["scale"].as_f64(), gif.params["loop"].as_bool(), gif.params["from"].as_u64(), gif.params["to"].as_u64(), gif.frames), (Some(0.25), Some(false), Some(1), Some(4), 4));
    assert_eq!(gif.params["path"], json!(Path::new("art").join("walk.gif").display().to_string()));
    assert_eq!(steps[2].outputs, vec![Path::new("art").join("walk_sheet.png"), Path::new("art").join("walk_sheet.json")], "the sheet and its frame map");
    assert_eq!(steps[2].params["columns"], 2);
    assert_eq!((steps[3].scene, steps[3].params["frame"].as_u64()), (0, Some(5)));
    // Only some formats, when asked.
    let some = plan(&doc, Some(Path::new("walk.anicraft")), Some(&[PublishFormat::Svg])).unwrap();
    assert_eq!(some.len(), 1);
    // Two formats writing one file is refused, whatever the case.
    std::sync::Arc::make_mut(&mut doc.publish).png.enabled = true;
    std::sync::Arc::make_mut(&mut doc.publish).png.output = "{name}_SHEET.png".into();
    let err = plan(&doc, Some(Path::new("walk.anicraft")), None).unwrap_err();
    assert!(err.contains("Sprite sheet") && err.contains("PNG image"), "{err}");
    // Untitled, it asks to save first.
    assert!(plan(&doc, None, None).unwrap_err().contains("save"));
}

#[test]
fn publish_writes_every_format_and_reports_once() {
    let (mut s, dir) = saved("all");
    for p in [
        json!({"format": "gif", "enabled": true, "scale": 0.1, "to": 3}),
        json!({"format": "lottie", "enabled": true, "to": 3}),
        json!({"format": "sprite_sheet", "enabled": true, "scale": 0.1, "to": 3, "columns": 2}),
        json!({"format": "png_sequence", "enabled": true, "scale": 0.1, "to": 2, "output": "out/{name}_frames"}),
        json!({"format": "png", "enabled": true, "scale": 0.1, "frame": 2}),
        json!({"format": "svg", "enabled": true}),
        json!({"format": "audio", "enabled": true}),
        json!({"format": "video", "enabled": false}),
    ] {
        run(&mut s, "publish.settings_set", p);
    }
    let v = run(&mut s, "file.publish", json!({}));
    let formats = v["formats"].as_array().unwrap();
    let status = |k: &str| formats.iter().find(|f| f["format"] == k).map(|f| f["status"].clone()).unwrap_or(Value::Null);
    for k in ["html", "gif", "lottie", "sprite_sheet", "png_sequence", "png", "svg"] {
        assert_eq!(status(k), "published", "{k}: {v}");
    }
    assert_eq!(status("audio"), "skipped", "the demo has no sounds");
    assert_eq!(status("video"), Value::Null, "off formats don't run");
    assert_eq!((v["published"].as_u64(), v["skipped"].as_u64(), v["failed"].as_u64(), v["cancelled"].as_bool()), (Some(7), Some(1), Some(0), Some(false)));
    let report = v["report"].as_str().unwrap();
    assert!(report.contains("Audio skipped: the movie has no sounds."), "{report}");
    // The files are there, made with each format's options.
    for f in ["walk.html", "walk.gif", "walk.json", "walk_sheet.png", "walk_sheet.json", "walk.png", "walk.svg"] {
        assert!(dir.join(f).is_file(), "{f}");
    }
    let frames = std::fs::read_dir(dir.join("out").join("walk_frames")).unwrap().count();
    assert_eq!(frames, 3, "frames 1 to 3, in a folder made for them");
    let gif = std::fs::read(dir.join("walk.gif")).unwrap();
    let (w, h) = (u16::from_le_bytes([gif[6], gif[7]]), u16::from_le_bytes([gif[8], gif[9]]));
    let view = anicraft_render::View::stage(&s.doc, 0.1);
    assert_eq!((u32::from(w), u32::from(h)), (view.width, view.height), "the GIF's scale");
    let sheet: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("walk_sheet.json")).unwrap()).unwrap();
    assert_eq!(sheet["frames"].as_object().unwrap().len(), 4);
    assert_eq!(v["files"].as_array().unwrap().len(), 8);

    // Publishing again replaces this session's own files without asking...
    run(&mut s, "file.publish", json!({"formats": ["svg"]}));
    // ...but another session (the next time the app starts) asks first.
    run(&mut s, "file.save", json!({}));
    let mut t = Session::default();
    run(&mut t, "file.open", json!({"path": dir.join("walk.anicraft").display().to_string()}));
    let err = t.execute("file.publish", &json!({})).unwrap_err();
    assert!(matches!(err, EngineError::NeedsConfirmation(_)), "{err}");
    assert!(err.to_string().contains("walk.html"), "{err}");
    let c = t.confirmation("file.publish", &json!({})).unwrap();
    assert!(c.irreversible && c.message.contains("and 5 more"), "{}", c.message);
    run(&mut t, "file.publish", json!({"confirm": true}));
    assert!(t.confirmation("file.publish", &json!({})).is_none());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn one_failing_format_doesnt_stop_the_others() {
    let (mut s, dir) = saved("fail");
    run(&mut s, "publish.settings_set", json!({"format": "gif", "enabled": true, "scale": 8}));
    run(&mut s, "publish.settings_set", json!({"format": "svg", "enabled": true}));
    let v = run(&mut s, "file.publish", json!({}));
    assert_eq!((v["published"].as_u64(), v["failed"].as_u64()), (Some(2), Some(1)), "{v}");
    assert!(v["report"].as_str().unwrap().starts_with("Animated GIF failed: that GIF would be too large"), "{v}");
    assert!(dir.join("walk.svg").is_file() && !dir.join("walk.gif").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cancel_stops_cleanly() {
    let (mut s, dir) = saved("cancel");
    for p in [json!({"format": "gif", "enabled": true, "scale": 0.1}), json!({"format": "svg", "enabled": true}), json!({"format": "png_sequence", "enabled": true, "scale": 0.1})] {
        run(&mut s, "publish.settings_set", p);
    }
    // Cancelled after the first format: the rest never run.
    let control = Arc::new(Control::default());
    let steps = plan(&s.doc, s.path.as_deref(), None).unwrap();
    let mut job = Publisher::new(s.doc.clone(), s.path.clone(), steps, control.clone());
    assert_eq!(job.current(), Some(PublishFormat::Html));
    assert_eq!(job.step().map(|o| o.status), Some(Status::Published));
    control.cancel();
    job.run_all();
    let r = job.report();
    assert!(job.done() && r.count(Status::Cancelled) == 3, "{:?}", r.to_json());
    assert!(dir.join("walk.html").is_file() && !dir.join("walk.gif").exists() && !dir.join("walk.svg").exists() && !dir.join("walk_frames").exists());
    assert_eq!(r.to_json()["cancelled"], true);

    // Cancelled inside an export: the GIF stops between frames and writes nothing.
    let stop = Arc::new(Control::default());
    stop.cancel();
    s.job = Some(stop.clone());
    let gif = dir.join("inside.gif");
    assert!(matches!(s.execute("file.export_gif", &json!({"path": gif.display().to_string(), "scale": 0.1})), Err(EngineError::Cancelled)));
    assert!(!gif.exists());
    let seq = dir.join("seq");
    assert!(matches!(s.execute("file.export_png_sequence", &json!({"dir": seq.display().to_string(), "scale": 0.1})), Err(EngineError::Cancelled)));
    assert!(matches!(s.execute("file.export_sprite_sheet", &json!({"path": dir.join("s.png").display().to_string(), "scale": 0.1})), Err(EngineError::Cancelled)));
    assert!(!dir.join("s.png").exists());
    // Progress counts frames made.
    let watch = Arc::new(Control::default());
    s.job = Some(watch.clone());
    watch.begin(4);
    run(&mut s, "file.export_gif", json!({"path": gif.display().to_string(), "scale": 0.1, "to": 3}));
    assert_eq!(watch.progress(), (4, 4));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn publish_refuses_what_it_cant_run() {
    let mut s = Session::new(sample::demo());
    assert!(s.execute("file.publish", &json!({})).unwrap_err().to_string().contains("save the document first"));
    for bad in [json!({"formats": "gif"}), json!({"formats": ["gif", 3]}), json!({"formats": ["nope"]}), json!({"formats": [null]})] {
        assert!(s.execute("file.publish", &bad).is_err(), "{bad}");
        assert!(s.confirmation("file.publish", &bad).is_none());
    }
    let (mut t, dir) = saved("none");
    run(&mut t, "publish.settings_set", json!({"format": "html", "enabled": false}));
    assert!(t.execute("file.publish", &json!({})).unwrap_err().to_string().contains("no format is on"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn in_a_browser_every_file_comes_back_and_several_make_one_zip() {
    std::thread::spawn(|| {
        vfs::use_memory();
        // Untitled: the browser has no folders, so it publishes as "Untitled".
        let mut s = Session::new(sample::demo());
        for p in [json!({"format": "gif", "enabled": true, "scale": 0.1, "to": 2}), json!({"format": "video", "enabled": true}), json!({"format": "svg", "enabled": true, "output": "img/{name}.svg"})] {
            run(&mut s, "publish.settings_set", p);
        }
        let v = run(&mut s, "file.publish", json!({}));
        let video = v["formats"].as_array().unwrap().iter().find(|f| f["format"] == "video").unwrap().clone();
        assert_eq!(video["status"], "skipped");
        assert!(video["message"].as_str().unwrap().contains("web browser"));
        let files = vfs::take_written();
        let names: Vec<String> = files.iter().map(|(p, _)| p.to_string_lossy().replace('\\', "/")).collect();
        assert_eq!(names, ["Untitled.gif", "Untitled.html", "img/Untitled.svg"]);
        let z = zip(&files).unwrap();
        // The end record says three entries; each name is in the archive with `/` between folders.
        let end = z.len() - 22;
        assert_eq!(&z[end..end + 4], &[0x50, 0x4b, 0x05, 0x06]);
        assert_eq!(u16::from_le_bytes([z[end + 10], z[end + 11]]), 3);
        assert!(z.windows(16).any(|w| w == b"img/Untitled.svg"));
        // AniCraft's hardened ZIP reader (the .fla one) opens it, checks every CRC, and gets
        // each file back exactly.
        let archive = anicraft_xfl::zip::Archive::open(&z).unwrap();
        for (p, data) in &files {
            let name = p.to_string_lossy().replace('\\', "/");
            assert_eq!(archive.read(&name).unwrap().as_ref(), Some(data), "{name}");
        }
        let total: usize = files.iter().map(|(_, d)| d.len()).sum();
        assert!(z.len() > total && z.len() < total + 400, "stored, not compressed: {} for {total}", z.len());
    })
    .join()
    .unwrap();
}


#[test]
fn plugins_may_read_but_not_change_or_run_publishing() {
    use crate::cmds_plugin::plugin_may_call;
    assert!(plugin_may_call("publish.settings_get").is_ok());
    assert!(plugin_may_call("publish.settings_set").is_err());
    assert!(plugin_may_call("file.publish").is_err());
}
