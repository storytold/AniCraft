//! The sample plug-ins of `examples/plugins`, built to WebAssembly, through the engine: the
//! Plain Text Frames exporter on the sample movie and the Stepped Eases on a tween.
//!
//! They are built by `examples/plugins/build.sh` (CI's web job does, then sets
//! `ANICRAFT_EXAMPLE_PLUGINS` to their folder, so a missing module fails there). Without the
//! variable the test uses `examples/plugins/dist` when it is there and otherwise skips.

use anicraft_engine::Session;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::Instant;

fn folder() -> Option<PathBuf> {
    if let Ok(d) = std::env::var("ANICRAFT_EXAMPLE_PLUGINS") {
        let d = PathBuf::from(d);
        assert!(d.join("text_export.wasm").is_file() && d.join("stepped_ease.wasm").is_file(), "ANICRAFT_EXAMPLE_PLUGINS={} has no built sample plug-ins", d.display());
        return Some(d);
    }
    let d = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples/plugins/dist");
    if d.join("text_export.wasm").is_file() && d.join("stepped_ease.wasm").is_file() {
        Some(d)
    } else {
        eprintln!("skipped: build the sample plug-ins with examples/plugins/build.sh");
        None
    }
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

#[test]
fn the_text_exporter_writes_the_sample_movie() {
    let Some(dir) = folder() else { return };
    let mut s = Session::new(anicraft_doc::sample::demo());
    run(&mut s, "plugin.install", json!({"path": dir.join("text_export.wasm").display().to_string()}));
    let formats = run(&mut s, "file.export_formats", json!({}));
    assert!(formats["formats"].as_array().is_some_and(|f| f.iter().any(|x| x["id"] == "org.anicraft.text-export" && x["extensions"][0] == "txt")));
    let out = std::env::temp_dir().join(format!("anicraft-example-text-{}.txt", std::process::id()));
    let _ = std::fs::remove_file(&out);
    let t = Instant::now();
    let v = run(&mut s, "file.export_plugin", json!({"format": "org.anicraft.text-export", "path": out.display().to_string(), "options": {"detail": "Paths"}}));
    let ms = t.elapsed().as_secs_f64() * 1e3;
    let text = std::fs::read_to_string(&out).expect("written");
    let frames = s.doc.frame_count();
    assert_eq!(v["frames_read"].as_u64(), Some(u64::from(frames)));
    assert!(text.starts_with("# anicraft-example-text-"), "{}", &text[..80.min(text.len())]);
    assert_eq!(text.matches("\nframe ").count(), frames as usize);
    assert!(text.contains("\nframe 1 (scene 1): ") && text.contains("    M"), "shapes with their outlines");
    eprintln!("Plain Text Frames: {frames} frames, {} bytes, {} instructions, {ms:.0} ms", text.len(), v["fuel"]);
    // Every other frame, summary only.
    let v = run(&mut s, "file.export_plugin", json!({"format": "org.anicraft.text-export", "path": out.display().to_string(), "options": {"detail": "Summary", "every": 2}, "confirm": true}));
    assert_eq!(v["frames_read"].as_u64(), Some(u64::from(frames.div_ceil(2))));
    assert!(!std::fs::read_to_string(&out).expect("written").contains("element "));
    let _ = std::fs::remove_file(&out);
}

#[test]
fn the_stepped_eases_drive_a_tween() {
    let Some(dir) = folder() else { return };
    let mut s = Session::default();
    let t = Instant::now();
    run(&mut s, "plugin.install", json!({"path": dir.join("stepped_ease.wasm").display().to_string()}));
    eprintln!("Stepped Eases: loaded and sampled 4 eases in {:.1} ms", t.elapsed().as_secs_f64() * 1e3);
    let l = run(&mut s, "ease.list", json!({}));
    let names: Vec<&str> = l["plugins"].as_array().into_iter().flatten().filter_map(|e| e["name"].as_str()).collect();
    assert_eq!(names, ["Stepped 2", "Stepped 4", "Stepped 8", "Smooth Steps 4"]);
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 10, "h": 10}));
    run(&mut s, "timeline.goto", json!({"frame": 8}));
    run(&mut s, "timeline.insert_keyframe", json!({}));
    run(&mut s, "edit.select_all", json!({}));
    run(&mut s, "element.translate", json!({"dx": 80, "dy": 0}));
    run(&mut s, "timeline.create_tween", json!({"frame": 0, "easing": {"plugin": "org.anicraft.stepped-ease", "ease": "steps2"}}));
    let xs: Vec<f32> = (0..9)
        .map(|f| {
            s.set_frame(f);
            s.visible_elements().first().map_or(f32::NAN, |e| e.transform.translation().x - 5.0)
        })
        .collect();
    // On twos over eight frames: still for the first half, then half way, then there.
    assert_eq!(xs, [0.0, 0.0, 0.0, 0.0, 40.0, 40.0, 40.0, 40.0, 80.0]);
}
