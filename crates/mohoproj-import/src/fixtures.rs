//! Corpus gate: the maop-corpus fixtures (Moho Pro 14.4 native saves) all import, and what
//! each one reports matches CASES.md. These tests run wherever the corpus sits (its README:
//! point `MAOHOU_FIXTURES` at the `maop/` directory, or keep the checkout beside the
//! workspace as `maop-corpus/`); without it they pass trivially, so machines without the
//! corpus still build green.

use crate::import::import_path;
use std::path::{Path, PathBuf};

/// Finds the corpus fixtures directory, or `None` (tests then say why they didn't run).
fn corpus() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("MAOHOU_FIXTURES") {
        let p = PathBuf::from(dir);
        if p.is_dir() {
            return Some(p);
        }
    }
    let beside = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../maop-corpus/maop");
    beside.is_dir().then_some(beside)
}

/// The fixture's path, or `None` after explaining why the test didn't run.
fn fixture(name: &str) -> Option<PathBuf> {
    let dir = corpus()?;
    let f = dir.join(format!("{name}.moho"));
    f.is_file().then_some(f).or_else(|| {
        eprintln!("skipped {name}: no {name}.moho in the corpus");
        None
    })
}

fn missing_corpus() {
    eprintln!("skipped: no maop-corpus at MAOHOU_FIXTURES or ../../maop-corpus");
}

fn layer_names(doc: &anicraft_doc::Document) -> Vec<&str> {
    doc.timeline(0).layers.iter().map(|l| l.name.as_str()).collect()
}

// --- one test per fixture; bodies assert what CASES.md records, from the actual files ---

#[test]
fn empty_imports() {
    let Some(path) = fixture("empty") else { return missing_corpus() };
    let (doc, report) = import_path(&path).unwrap_or_else(|e| panic!("empty.moho: {e}"));
    assert_eq!(layer_names(&doc), ["Layer 1"]);
    assert_eq!(doc.stage.width, 1280);
    assert_eq!(doc.stage.height, 720);
    assert!(report.skipped.is_empty(), "{:?}", report.skipped);
    assert_eq!(report.summary(), "Imported everything in the file.");
    // The sidecar keeps the whole original.
    assert_eq!(doc.maop_maohou.as_ref().unwrap().raw["version"], 1045);
}

#[test]
fn vectors_imports() {
    let Some(path) = fixture("vectors") else { return missing_corpus() };
    let (doc, report) = import_path(&path).unwrap_or_else(|e| panic!("vectors.moho: {e}"));
    // Bottom-first in the file: Layer 1, Text, Shapes → top-first here.
    assert_eq!(layer_names(&doc), ["Shapes", "Text", "Layer 1"]);
    assert!(report.skipped.is_empty(), "{:?}", report.skipped);
}

#[test]
fn keyframes_imports() {
    let Some(path) = fixture("keyframes") else { return missing_corpus() };
    let (doc, report) = import_path(&path).unwrap_or_else(|e| panic!("keyframes.moho: {e}"));
    assert_eq!(layer_names(&doc), ["KFDemo", "Layer 1"]);
    assert!(report.skipped.is_empty(), "{:?}", report.skipped);
}

#[test]
fn text_imports() {
    let Some(path) = fixture("text") else { return missing_corpus() };
    let (doc, report) = import_path(&path).unwrap_or_else(|e| panic!("text.moho: {e}"));
    assert_eq!(layer_names(&doc), ["Title", "Layer 1"]);
    let els = &doc.timeline(0).layers[0].keyframes[0].elements;
    assert_eq!(els.len(), 1, "the text layer lays out one block");
    let anicraft_doc::ElementKind::Text(block) = &els[0].kind else { panic!("a text element") };
    assert_eq!(block.text, "Text");
    // 60 pt at Moho's 72 pt/inch and 320 px/inch = 266.7 px.
    assert!((block.size - 800.0 / 3.0).abs() < 0.5, "size {} px", block.size);
    assert!(report.skipped.is_empty(), "{:?}", report.skipped);
}

#[test]
fn switch_imports() {
    let Some(path) = fixture("switch") else { return missing_corpus() };
    let (doc, report) = import_path(&path).unwrap_or_else(|e| panic!("switch.moho: {e}"));
    assert_eq!(layer_names(&doc), ["Mouths", "Smile", "Frown", "Layer 1"]);
    // The fixture's switch has no selection: every variant stays visible, noted as such.
    assert!(report.notes.iter().any(|n| n.contains("switch layer without a selection")), "{report:?}");
    let mouths = doc.timeline(0).layers.iter().find(|l| l.name == "Mouths").unwrap();
    assert_eq!(mouths.kind, anicraft_doc::LayerKind::Folder);
    for v in ["Smile", "Frown"] {
        let l = doc.timeline(0).layers.iter().find(|l| l.name == v).unwrap();
        assert_eq!(l.parent, Some(mouths.id), "{v} inside the switch");
        assert!(l.visible);
    }
}

#[test]
fn bones_imports() {
    let Some(path) = fixture("bones") else { return missing_corpus() };
    let (doc, report) = import_path(&path).unwrap_or_else(|e| panic!("bones.moho: {e}"));
    assert_eq!(layer_names(&doc), ["Rig", "Layer 1"]);
    let rig = &doc.timeline(0).layers[0];
    assert_eq!(rig.kind, anicraft_doc::LayerKind::Folder);
    assert_eq!(report.skipped.get("skeleton bones"), Some(&5), "{:?}", report.skipped);
}

#[test]
fn smartbones_imports() {
    let Some(path) = fixture("smartbones") else { return missing_corpus() };
    let (doc, report) = import_path(&path).unwrap_or_else(|e| panic!("smartbones.moho: {e}"));
    assert_eq!(layer_names(&doc), ["SmartRig", "Layer 1"]);
    assert_eq!(report.skipped.get("skeleton bones"), Some(&2), "{:?}", report.skipped);
}

#[test]
fn gradients_imports() {
    let Some(path) = fixture("gradients") else { return missing_corpus() };
    let (doc, report) = import_path(&path).unwrap_or_else(|e| panic!("gradients.moho: {e}"));
    assert_eq!(layer_names(&doc), ["Layer 1"]);
    let els = &doc.timeline(0).layers[0].keyframes[0].elements;
    // 17 shapes, each with an outline and 13 of them gradient-filled (4 radial mapped; 4
    // reflected + 5 angle reported): 13 fill elements + 17 stroke elements.
    assert_eq!(els.len(), 30, "13 fills + 17 strokes");
    let reflected_angle = *report.skipped.get("reflected and angle gradient fills").unwrap_or(&0);
    assert_eq!(reflected_angle, 9, "{:?}", report.skipped);
    let approximated = *report.skipped.get("gradient geometry (approximated from shape bounds)").unwrap_or(&0);
    assert_eq!(approximated, 4, "the four radial gradients");
    let radials = els.iter().filter(|el| matches!(&el.kind, anicraft_doc::ElementKind::Shape(s) if matches!(s.fill, Some(anicraft_doc::Paint::Radial { .. })))).count();
    assert_eq!(radials, 4);
}

#[test]
fn follow_path_imports() {
    let Some(path) = fixture("follow-path") else { return missing_corpus() };
    let (doc, report) = import_path(&path).unwrap_or_else(|e| panic!("follow-path.moho: {e}"));
    assert_eq!(layer_names(&doc), ["FollowLayer", "Layer 1"]);
    let els = &doc.timeline(0).layers[1].keyframes[0].elements;
    assert_eq!(els.len(), 30, "13 fill + 17 stroke elements from 17 shapes");
    assert_eq!(report.skipped.get("follow-path relations (positions are already baked)"), Some(&1), "{:?}", report.skipped);
    assert!(report.skipped.contains_key("rotate to follow"));
    assert!(report.skipped.contains_key("follow progress animation"));
}

#[test]
fn point_animation_imports() {
    let Some(path) = fixture("point-animation") else { return missing_corpus() };
    let (doc, report) = import_path(&path).unwrap_or_else(|e| panic!("point-animation.moho: {e}"));
    assert_eq!(layer_names(&doc), ["PointAnim", "Layer 1"]);
    assert!(report.skipped.is_empty(), "the fixture's meshes are empty: {:?}", report.skipped);
}

/// Every fixture that exists imports without error (the acceptance gate's first clause).
#[test]
fn all_corpus_fixtures_import() {
    let Some(dir) = corpus() else { return missing_corpus() };
    let mut n = 0;
    let mut failures = Vec::new();
    for e in std::fs::read_dir(&dir).expect("corpus dir reads") {
        let Ok(e) = e else { continue };
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("moho") {
            continue;
        }
        n += 1;
        if let Err(err) = import_path(&p) {
            failures.push(format!("{}: {err}", p.display()));
        }
    }
    assert!(n >= 8, "expected the corpus's fixtures, found {n} in {}", dir.display());
    assert!(failures.is_empty(), "fixtures failed to import:\n{}", failures.join("\n"));
}

/// A real Moho-saved document with animation data (the fork's CharWalk sample, next to the
/// corpus in the local knowledge base): 39 keyframed channels, ten timeline markers and a
/// 44-frame project range. Verified against the file's own channel data — the in-repo half
/// of the stage-2 gate; the Moho-runtime half (Lua probe) stays with the owner's machine.
#[test]
fn charwalk_markers_and_range_import() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plan/moho/samples/CharWalk.mohoproj");
    if !path.is_file() {
        eprintln!("skipped: no CharWalk.mohoproj beside the corpus");
        return;
    }
    let (doc, _report) = import_path(&path).unwrap_or_else(|e| panic!("CharWalk: {e}"));
    // Its bones and point animation are later milestones; the report says so.
    // The 0-based project range 1..44 becomes (0, 44): a 44-frame timeline, as Moho shows.
    // It equals the imported content's length, so no explicit range is stored.
    assert_eq!(doc.scenes[0].frames, None, "range == content: nothing to store");
    assert_eq!(doc.frame_count(), 44);
    // The walk cycle's markers, one per frame, in order.
    let labels: Vec<(u32, &str)> = doc.scenes[0].markers.iter().map(|(f, s)| (*f, s.as_str())).collect();
    assert_eq!(labels, vec![(1, "Contact"), (6, "Passing"), (12, "Kickoff"), (16, "Float"), (23, "Contact"), (28, "Passing"), (34, "Kickoff"), (38, "Float"), (45, "Contact")], "{labels:?}");
    // Its camera is the default (zoom 2 at the centre): nothing stored.
    assert!(doc.scenes[0].camera.is_neutral());
    // Bone and point channels are later milestones and stay in the sidecar untouched.
    let raw = &doc.maop_maohou.as_ref().unwrap().raw;
    assert_eq!(raw["animated_values"]["timeline_markers"]["val"][1], "Contact");
}

/// The stage-2 real-machine sampling gate: the corpus fixture keyframes-layer.moho was
/// generated by Moho Pro 14.4 (Lua: gen_layeranim.lua — 3 linear translation keys, 2
/// rotation keys, camera zoom 2→4 and roll 0→20°, all INTERP_LINEAR), and Moho's own
/// per-frame evaluation of those channels (probe_layeranim.lua, AnimChannel:GetValue)
/// is the ground truth. Our importer converts inches→pixels; the comparison converts
/// back and demands the parity plan's ±1e-6 in Moho's native units, per frame, on
/// Channel::sample straight from the imported document.
#[test]
fn keyframes_layer_channels_match_moho_runtime_per_frame() {
    use anicraft_geom::Vec2;
    let Some(path) = fixture("keyframes-layer") else { return missing_corpus() };
    let probe = corpus().unwrap().join("inputs/keyframes-layer-probe.txt");
    if !probe.is_file() {
        eprintln!("skipped: no keyframes-layer-probe.txt beside the corpus");
        return;
    }
    let (doc, report) = import_path(&path).unwrap_or_else(|e| panic!("keyframes-layer.moho: {e}"));
    // The gradient shapes carry this fixture's canvas; their approximations are counted
    // elsewhere. What matters here: no animation was dropped.
    assert!(!report.skipped.contains_key("layer transform animation (drawn at frame 1)"));
    assert!(!report.skipped.contains_key("camera animation"));

    let layer = &doc.timeline(0).layers[0];
    let camera = &doc.scenes[0].camera;
    let text = std::fs::read_to_string(&probe).unwrap();
    let mut checked = 0u32;
    for line in text.lines().filter(|l| l.starts_with("frame ")) {
        // "frame N translation X Y Z rotation R camera_zoom Z camera_roll R camera_track X Y Z"
        let tok: Vec<&str> = line.split_whitespace().collect();
        let num = |i: usize| tok[i].parse::<f64>().unwrap();
        let (frame, tx, ty, _tz, rot, zoom, roll, camx, camy, _camz) =
            (num(1) as f32, num(3), num(4), num(5), num(7), num(9), num(11), num(13), num(14), num(15));
        // Our channels live in stage pixels (y down); back to Moho inches (y up).
        let t = layer.translation.sample_or(frame, Vec2::ZERO);
        let (ours_x, ours_y) = ((t.x as f64) / 320.0, (t.y as f64) / -320.0);
        assert!((ours_x - tx).abs() < 1e-6, "frame {frame} translation x: {ours_x} vs Moho {tx}");
        assert!((ours_y - ty).abs() < 1e-6, "frame {frame} translation y: {ours_y} vs Moho {ty}");
        // Rotation flips sign in the y-down space; stored screen-clockwise.
        let r = layer.rotation.sample_or(frame, 0.0) as f64;
        assert!((-r - rot).abs() < 1e-6, "frame {frame} rotation: {} vs Moho {rot}", -r);
        // The camera: zoom is Moho's own units, roll flips sign, track maps as a point.
        let z = camera.zoom.sample_or(frame, 2.0) as f64;
        assert!((z - zoom).abs() < 1e-6, "frame {frame} zoom: {z} vs Moho {zoom}");
        let roll_ours = camera.roll.sample_or(frame, 0.0) as f64;
        assert!((-roll_ours - roll).abs() < 1e-6, "frame {frame} roll: {} vs Moho {roll}", -roll_ours);
        let (cx, cy) = ((camx as f32) * 320.0 + 640.0, (-camy as f32) * 320.0 + 360.0);
        let track = camera.track.sample_or(frame, Vec2::new(640.0, 360.0));
        assert!((track.x - cx).abs() < 1e-4, "frame {frame} track x: {} vs Moho {cx}", track.x);
        assert!((track.y - cy).abs() < 1e-4, "frame {frame} track y: {} vs Moho {cy}", track.y);
        checked += 1;
    }
    assert_eq!(checked, 31, "every probed frame compared");
}
