//! Sounds end to end: import, place, settings, delete, the file format, frame copy and paste,
//! and the mixed soundtrack, checked sample for sample.

use super::*;
use anicraft_doc::audio;
use anicraft_doc::{SoundId, SoundSync};
use serde_json::json;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    match s.execute(id, &p) {
        Ok(v) => v,
        Err(e) => panic!("{id} {p}: {e}"),
    }
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("anicraft-sound-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 2500 frames of 16-bit mono at 8 kHz: sample `i` is `(i % 200 - 100) / 256`, exact in
/// 16 bits and in `f32`.
fn source() -> Vec<f32> {
    (0..2500).map(|i| ((i % 200) as f32 - 100.0) / 256.0).collect()
}

/// A session at 8 fps (one frame = 1000 samples at 8 kHz) with the source sound in the
/// library; its id.
fn session_with_sound(dir: &std::path::Path) -> (Session, u64) {
    let wav = dir.join("beat.wav");
    std::fs::write(&wav, crate::cmds_sound::test_wav(8000, &source())).unwrap();
    let mut s = Session::default();
    run(&mut s, "doc.set_stage", json!({"fps": 8}));
    let v = run(&mut s, "file.import_to_library", json!({"path": wav.display().to_string()}));
    assert_eq!((v["name"].as_str(), v["rate"].as_u64(), v["channels"].as_u64()), (Some("beat.wav"), Some(8000), Some(1)));
    (s, v["sound"].as_u64().unwrap())
}

fn export(s: &mut Session, dir: &std::path::Path, name: &str, extra: Value) -> audio::Pcm {
    let out = dir.join(name);
    let mut p = json!({"path": out.display().to_string(), "rate": 8000, "bits": 32, "confirm": true});
    if let (Value::Object(m), Value::Object(x)) = (&mut p, extra) {
        m.extend(x);
    }
    run(s, "file.export_audio", p);
    audio::decode(&std::fs::read(&out).unwrap()).unwrap()
}

fn left(p: &audio::Pcm) -> Vec<f32> {
    p.samples.iter().step_by(2).copied().collect()
}

#[test]
fn a_placed_sound_mixes_into_the_exported_wav_sample_exactly() {
    let dir = temp_dir("mix");
    let (mut s, id) = session_with_sound(&dir);
    let v = run(&mut s, "sound.place", json!({"sound": id, "frame": 2, "extend": true}));
    assert_eq!((v["keyframe"].as_u64(), v["duration"].as_u64(), v["frames"].as_u64()), (Some(2), Some(3), Some(3)), "2500 samples span 3 frames at 8 fps");
    assert_eq!(s.frame_count(), 5);
    let mix = export(&mut s, &dir, "a.wav", json!({}));
    assert_eq!((mix.rate, mix.channels, mix.frames()), (8000, 2, 5000), "the movie's length");
    let l = left(&mix);
    assert!(l[..2000].iter().all(|&x| x == 0.0), "silent before the keyframe");
    assert_eq!(&l[2000..4500], &source()[..], "then the sound, sample for sample");
    assert!(l[4500..].iter().all(|&x| x == 0.0));
    assert_eq!(l, mix.samples.iter().skip(1).step_by(2).copied().collect::<Vec<_>>(), "mono plays on both sides");

    // Repeat twice: the second play runs past the movie's end and is cut there.
    run(&mut s, "sound.set", json!({"frame": 2, "repeat": 2}));
    let l = left(&export(&mut s, &dir, "b.wav", json!({})));
    assert_eq!(&l[4500..5000], &source()[..500]);

    // Stream: tied to its span. Shorten the span to 2 frames by a keyframe after it.
    run(&mut s, "sound.set", json!({"frame": 2, "sync": "stream", "repeat": 1}));
    run(&mut s, "timeline.insert_blank_keyframe", json!({"frame": 4}));
    let l = left(&export(&mut s, &dir, "c.wav", json!({})));
    assert_eq!(&l[2000..4000], &source()[..2000]);
    assert!(l[4000..].iter().all(|&x| x == 0.0), "a stream ends with its span");
    // A range starting inside a stream picks it up mid-way; an event started earlier is not heard.
    let part = left(&export(&mut s, &dir, "d.wav", json!({"from": 3, "to": 3})));
    assert_eq!(part, source()[1000..2000].to_vec());
    run(&mut s, "sound.set", json!({"frame": 2, "sync": "event"}));
    let part = left(&export(&mut s, &dir, "e.wav", json!({"from": 3, "to": 3})));
    assert!(part.iter().all(|&x| x == 0.0));

    // Other rates resample; 16 bits rounds.
    let out = dir.join("f.wav");
    let v = run(&mut s, "file.export_audio", json!({"path": out.display().to_string(), "rate": 44100}));
    assert_eq!(v["sounds"], 1);
    let p = audio::decode(&std::fs::read(&out).unwrap()).unwrap();
    assert_eq!((p.rate, p.frames()), (44_100, 27_563));
    // Replacing a file needs consent; bad params are errors.
    assert!(s.execute("file.export_audio", &json!({"path": out.display().to_string()})).is_err());
    for bad in [json!({"path": "x.mp3"}), json!({"path": "x.wav", "rate": 5}), json!({"path": "x.wav", "bits": 8}), json!({"path": "x.wav", "from": 5, "to": 1})] {
        assert!(s.execute("file.export_audio", &bad).is_err(), "{bad}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn start_and_stop_sync_and_looping() {
    let dir = temp_dir("sync");
    let (mut s, id) = session_with_sound(&dir);
    // An event loop from frame 0, a Start at 1 (skipped: it is playing), a Stop at 3.
    run(&mut s, "sound.place", json!({"sound": id, "frame": 0, "loop": true}));
    run(&mut s, "sound.place", json!({"sound": id, "frame": 1, "sync": "start"}));
    run(&mut s, "sound.place", json!({"sound": id, "frame": 3, "sync": "stop"}));
    let l = left(&export(&mut s, &dir, "a.wav", json!({})));
    let src = source();
    assert_eq!(&l[..2500], &src[..]);
    assert_eq!(&l[2500..3000], &src[..500], "looping");
    assert!(l[3000..].iter().all(|&x| x == 0.0), "stopped at frame 3");
    let info = s.info();
    assert_eq!(info["layers"][0]["keyframes"][1]["sound"]["sync"], "start");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn sounds_survive_save_undo_and_copy_paste_into_another_document() {
    let dir = temp_dir("file");
    let (mut s, id) = session_with_sound(&dir);
    run(&mut s, "sound.place", json!({"sound": id, "frame": 0, "sync": "stream", "loop": true, "extend": true}));
    let json = format::to_json(&s.doc).unwrap();
    let back = format::from_json(&json).unwrap();
    assert_eq!(back, s.doc, "lossless");
    let k = &back.timeline(0).layers[0].keyframes[0];
    assert_eq!(k.sound.map(|f| (f.sync, f.looping)), Some((SoundSync::Stream, true)));
    assert_eq!(back.sounds[0].decoded().unwrap().pcm.frames(), 2500, "decodes again after loading");
    // Older files (no sounds) still open: covered by every older-format test; and a file
    // without the field reads as no sound.
    assert!(format::from_json(&json.replace("\"sound\": {", "\"unknown\": {")).is_ok());

    // Undo takes the placement back; redo restores it.
    run(&mut s, "edit.undo", json!({}));
    assert!(s.doc.timeline(0).layers[0].keyframes[0].sound.is_none());
    run(&mut s, "edit.redo", json!({}));
    assert!(s.doc.timeline(0).layers[0].keyframes[0].sound.is_some());

    // Copy the frames; paste into a new document: the sound comes along.
    run(&mut s, "timeline.copy_frames", json!({"frame": 0, "count": 4}));
    let clip = s.frame_clipboard.clone();
    let mut other = Session { frame_clipboard: clip, ..Default::default() };
    run(&mut other, "timeline.paste_frames", json!({"frame": 0}));
    assert_eq!(other.doc.sounds.len(), 1);
    assert!(other.doc.timeline(0).layers[0].keyframes[0].sound.is_some_and(|f| f.sound == SoundId(id)));
    assert!(other.doc.next_id > id);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn settings_removal_deletion_and_import_to_stage() {
    let dir = temp_dir("edit");
    let (mut s, id) = session_with_sound(&dir);
    // Settings need a sound on the keyframe; bad values are refused.
    assert!(s.execute("sound.set", &json!({"sync": "stream"})).is_err(), "no sound on it yet");
    run(&mut s, "sound.place", json!({"sound": id}));
    for bad in [json!({"sync": "sometimes"}), json!({"repeat": 0}), json!({"repeat": 10_000}), json!({"sound": 999_999}), json!({"loop": "yes"})] {
        assert!(s.execute("sound.set", &bad).is_err(), "{bad}");
    }
    assert!(s.execute("sound.place", &json!({"sound": 424_242})).is_err());
    run(&mut s, "sound.set", json!({"sync": "stream", "repeat": 3}));
    run(&mut s, "sound.set", json!({"loop": true}));
    let f = s.doc.timeline(0).layers[0].keyframes[0].sound.unwrap();
    assert_eq!((f.sync, f.repeat, f.looping), (SoundSync::Stream, 3, true));
    let v = run(&mut s, "sound.remove", json!({}));
    assert_eq!(v["removed"], true);
    assert!(s.doc.timeline(0).layers[0].keyframes[0].sound.is_none());

    // Import to Stage with a sound: a new layer named after it, at the playhead, as long as it.
    run(&mut s, "timeline.goto", json!({"frame": 1}));
    let wav = dir.join("beat.wav");
    let v = run(&mut s, "file.import", json!({"path": wav.display().to_string()}));
    assert_eq!(v["frames"], 3);
    let l = s.active_layer().unwrap();
    assert_eq!(l.name, "beat");
    let k = l.keyframe_at(1).unwrap();
    assert_eq!((k.start, k.duration, k.sound.map(|f| f.sync)), (1, 3, Some(SoundSync::Event)));
    let second = k.sound.unwrap().sound;
    assert_ne!(second.0, id, "a second library item");

    // Deleting a sound in use asks first (undoable tier) and takes it off its keyframes.
    let c = s.confirmation("sound.delete", &json!({"sound": second.0})).unwrap();
    assert!(!c.irreversible);
    assert!(s.confirmation("sound.delete", &json!({"sound": id})).is_none(), "the first one is unused now");
    run(&mut s, "sound.rename", json!({"sound": second.0, "name": "Drums"}));
    assert_eq!(s.doc.sound(second).unwrap().name, "Drums");
    let v = run(&mut s, "sound.delete", json!({"sound": second.0}));
    assert_eq!(v["keyframes"], 1);
    assert!(s.doc.sound(second).is_none() && s.active_layer().unwrap().keyframe_at(1).unwrap().sound.is_none());
    // A folder holds no keyframes for sounds; bad files are errors.
    run(&mut s, "layer.new_folder", json!({}));
    assert!(s.execute("sound.place", &json!({"sound": id})).is_err());
    let junk = dir.join("junk.wav");
    std::fs::write(&junk, b"RIFF\0\0\0\0WAVEjunk").unwrap();
    assert!(s.execute("file.import_to_library", &json!({"path": junk.display().to_string()})).is_err());
    assert!(s.execute("file.import_to_library", &json!({"path": dir.join("x.txt").display().to_string()})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn video_export_carries_the_soundtrack() {
    let program = std::env::var("ANICRAFT_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
    let have = std::process::Command::new(&program).arg("-version").stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|st| st.success());
    if !have {
        eprintln!("no ffmpeg here: video soundtrack not tested");
        return;
    }
    let dir = temp_dir("video");
    let (mut s, id) = session_with_sound(&dir);
    run(&mut s, "sound.place", json!({"sound": id, "frame": 0, "extend": true}));
    for (name, codec) in [("a.webm", &b"A_OPUS"[..]), ("a.mp4", b"mp4a")] {
        let out = dir.join(name);
        let v = run(&mut s, "file.export_video", json!({"path": out.display().to_string(), "scale": 0.05}));
        assert_eq!(v["audio"], true);
        let bytes = std::fs::read(&out).unwrap();
        assert!(bytes.windows(codec.len()).any(|w| w == codec), "{name} has an audio track");
        let v = run(&mut s, "file.export_video", json!({"path": dir.join(format!("silent-{name}")).display().to_string(), "scale": 0.05, "audio": false}));
        assert_eq!(v["audio"], false);
    }
    let _ = std::fs::remove_dir_all(&dir);
}
