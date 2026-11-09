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
