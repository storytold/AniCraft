//! Files dropped onto the window and recent files (ADR 0059): routing by type and target, what
//! `file.drop` brings in and where, one undo step, and the recent list that opening and saving
//! keep.

use super::*;
use crate::cmds_drop::{DropAction, DropKind, DropTarget, kind_of, route};
use serde_json::json;
use std::path::Path;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    match s.execute(id, &p) {
        Ok(v) => v,
        Err(e) => panic!("{id} {p}: {e}"),
    }
}

fn temp_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("anicraft-drop-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A 4 × 2 PNG, a one-rectangle SVG and a short WAV in `dir`.
fn media(dir: &Path) -> (String, String, String) {
    let png = dir.join("photo.png");
    std::fs::write(&png, anicraft_render::encode_png(4, 2, &[255u8; 4 * 2 * 4]).unwrap()).unwrap();
    let svg = dir.join("badge.svg");
    std::fs::write(&svg, r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect width="40" height="20" fill="#f00"/></svg>"##).unwrap();
    let wav = dir.join("beep.wav");
    std::fs::write(&wav, crate::cmds_sound::test_wav(8000, &[0.25; 4000])).unwrap();
    (png.display().to_string(), svg.display().to_string(), wav.display().to_string())
}

#[test]
fn drops_route_by_file_type_and_target() {
    use DropAction::*;
    use DropTarget::*;
    let k = |p: &str| kind_of(Path::new(p));
    assert_eq!(k("a.anicraft"), DropKind::Document);
    assert_eq!(k("A.FLA"), DropKind::Document);
    assert_eq!(k("b.xfl"), DropKind::Document);
    assert_eq!(k("c.svg"), DropKind::Drawing);
    assert_eq!(k("d.JPEG"), DropKind::Image);
    assert_eq!(k("d.jpg"), DropKind::Image);
    assert_eq!(k("e.png"), DropKind::Image);
    assert_eq!(k("f.mp3"), DropKind::Sound);
    assert_eq!(k("g.ogg"), DropKind::Sound);
    assert_eq!(k("h.wav"), DropKind::Sound);
    assert_eq!(k("i.psd"), DropKind::Unsupported);
    assert_eq!(k("no extension"), DropKind::Unsupported);
    // An XFL package is a folder with a DOMDocument.xml.
    let dir = temp_dir("xfl").join("Movie");
    std::fs::create_dir_all(&dir).unwrap();
    assert_eq!(kind_of(&dir), DropKind::Unsupported, "a plain folder");
    std::fs::write(dir.join("DOMDocument.xml"), "<DOMDocument/>").unwrap();
    assert_eq!(kind_of(&dir), DropKind::Document);
    // Documents open wherever they land; media go where they are dropped.
    for t in [Stage, Library, Window] {
        assert_eq!(route(DropKind::Document, t), Open);
        assert!(matches!(route(DropKind::Unsupported, t), Refuse(_)));
    }
    for kind in [DropKind::Drawing, DropKind::Image, DropKind::Sound] {
        assert_eq!(route(kind, Stage), ImportToStage);
        assert!(matches!(route(kind, Window), Refuse(_)), "only the stage, the timeline and the Library take media");
        // The Library takes drawings too, as graphic symbols (ADR 0065); a layer's frame takes all.
        assert_eq!(route(kind, Library), ImportToLibrary);
        assert_eq!(route(kind, Timeline), ImportToTimeline);
    }
    assert!(matches!(route(DropKind::Unsupported, Timeline), Refuse(_)));
    assert_eq!(route(DropKind::Document, Timeline), Open);
}

#[test]
fn media_dropped_on_the_stage_land_on_the_active_layer_at_the_drop_point_in_one_step() {
    let dir = temp_dir("stage");
    let (png, svg, wav) = media(&dir);
    let mut s = Session::default();
    run(&mut s, "timeline.insert_frame", json!({"count": 20}));
    s.set_frame(5);
    let layers = s.timeline().layers.len();
    let history = s.history_len();
    let v = run(&mut s, "file.drop", json!({"paths": [png, svg, wav], "to": "stage", "x": 100, "y": 60}));
    assert_eq!(s.history_len(), history + 1, "one undo step for the whole drop");
    assert_eq!(s.timeline().layers.len(), layers, "everything went onto the active layer");
    assert_eq!(v["elements"].as_array().map(Vec::len), Some(2), "{v}");
    assert_eq!(s.selection.len(), 2, "what was dropped is selected");
    // Centred on the drop point, in the keyframe that shows at the playhead.
    let layer = s.active_layer().unwrap();
    let key = layer.keyframe_at(5).unwrap();
    assert!(s.selection.ids().all(|id| key.elements.iter().any(|e| e.id == id)));
    for id in s.selection.ids().collect::<Vec<_>>() {
        let b = s.element_bounds(id);
        assert!((b.center().x - 100.0).abs() < 0.01 && (b.center().y - 60.0).abs() < 0.01, "{b:?}");
    }
    assert!(key.sound.is_some(), "the sound is on the keyframe at the playhead");
    assert_eq!((s.doc.bitmaps.len(), s.doc.sounds.len()), (1, 1), "the image and the sound are in the Library");
    run(&mut s, "edit.undo", json!({}));
    assert!(s.doc.bitmaps.is_empty() && s.doc.sounds.is_empty() && s.active_layer().unwrap().keyframe_at(5).is_none_or(|k| k.elements.is_empty()));
}

#[test]
fn media_dropped_on_the_library_go_into_the_folder_under_the_pointer() {
    let dir = temp_dir("library");
    let (png, svg, wav) = media(&dir);
    let mut s = Session::default();
    let folder = run(&mut s, "library.new_folder", json!({"name": "Props"}))["folder"].as_u64().unwrap();
    let history = s.history_len();
    let v = run(&mut s, "file.drop", json!({"paths": [png.clone(), wav], "to": "library", "folder": folder}));
    assert_eq!(s.history_len(), history + 1);
    assert_eq!(v["items"].as_array().map(Vec::len), Some(2));
    let in_folder = s.doc.folder_contents(anicraft_doc::FolderId(folder));
    assert_eq!(in_folder.len(), 2, "{in_folder:?}");
    assert!(s.timeline().layers.iter().all(|l| l.keyframes.iter().all(|k| k.elements.is_empty() && k.sound.is_none())), "nothing placed on the stage");
    // The same image again: numbered, not refused.
    run(&mut s, "file.drop", json!({"path": png.clone(), "to": "library", "folder": folder}));
    let names: Vec<String> = s.doc.folder_contents(anicraft_doc::FolderId(folder)).into_iter().filter_map(|e| s.doc.entry_name(e).map(str::to_string)).collect();
    assert!(names.contains(&"photo.png".to_string()) && names.contains(&"photo.png 2".to_string()), "{names:?}");
    // At the top level without a folder.
    run(&mut s, "file.drop", json!({"path": png, "to": "library"}));
    assert_eq!(s.doc.bitmaps.len(), 3);
    // A drawing goes in as one graphic symbol named after the file (ADR 0065), into the folder.
    let symbols = s.doc.library.len();
    let v = run(&mut s, "file.drop", json!({"paths": [svg], "to": "library", "folder": folder}));
    assert_eq!(s.doc.library.len(), symbols + 1, "{v}");
    let sym = s.doc.library.last().unwrap();
    assert_eq!((sym.name.as_str(), sym.kind), ("badge", anicraft_doc::SymbolKind::Graphic));
    assert!(s.doc.folder_contents(anicraft_doc::FolderId(folder)).contains(&anicraft_doc::LibraryEntry::Item(anicraft_doc::LibraryItem::Symbol(sym.id))));
    // Nothing is imported when one file is refused.
    let before = s.doc.clone();
    assert!(s.execute("file.drop", &json!({"paths": [png.clone(), dir.join("notes.txt").display().to_string()], "to": "library"})).is_err());
    assert!(s.doc == before);
}

#[test]
fn drawings_with_layers_keep_them_and_bad_drops_change_nothing() {
    let dir = temp_dir("bad");
    let (png, _, _) = media(&dir);
    let masked = dir.join("masked.svg");
    std::fs::write(&masked, r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40"><clipPath id="c"><circle cx="20" cy="20" r="10"/></clipPath><rect width="40" height="40" fill="#0f0" clip-path="url(#c)"/></svg>"##).unwrap();
    let mut s = Session::default();
    let layers = s.timeline().layers.len();
    run(&mut s, "file.drop", json!({"path": masked.display().to_string(), "to": "stage", "x": 10, "y": 10}));
    assert!(s.timeline().layers.len() > layers + 1, "a masked drawing brings its folder and mask layer");
    let before = s.doc.clone();
    let doc = dir.join("x.anicraft");
    for p in [
        json!({"path": doc.display().to_string(), "to": "stage"}),
        json!({"path": dir.join("y.psd").display().to_string(), "to": "stage"}),
        json!({"paths": [png.clone(), dir.join("gone.png").display().to_string()], "to": "stage"}),
        json!({"path": png.clone(), "to": "timeline"}),
        json!({"path": png.clone(), "to": "library", "folder": u64::MAX}),
        json!({"path": png.clone(), "to": "stage", "folder": 1}),
        json!({"path": png.clone(), "to": "stage", "x": f64::MAX, "y": 0}),
        json!({"paths": [], "to": "stage"}),
        json!({"paths": vec![png.clone(); 65], "to": "stage"}),
        json!({"paths": "x", "to": "stage"}),
        json!({"paths": [1, 2], "to": "library"}),
        json!({"to": "stage"}),
        json!({}),
        Value::Null,
    ] {
        assert!(s.execute("file.drop", &p).is_err(), "{p}");
        assert!(s.doc == before, "{p} changed the document");
    }
    // A locked layer refuses the drop and says why.
    let top = s.timeline().layers.len() - 1;
    run(&mut s, "layer.select", json!({"index": top}));
    run(&mut s, "layer.toggle_lock", json!({}));
    let e = s.execute("file.drop", &json!({"path": png, "to": "stage"})).unwrap_err();
    assert!(e.to_string().contains("locked"), "{e}");
}

#[test]
fn opening_and_saving_keep_the_recent_list_and_open_recent_reopens() {
    let dir = temp_dir("recent");
    let a = dir.join("a.anicraft");
    let b = dir.join("b.anicraft");
    let mut s = Session::new(anicraft_doc::sample::demo());
    run(&mut s, "file.save_as", json!({"path": a.display().to_string()}));
    run(&mut s, "file.new", json!({"width": 1280, "height": 720}));
    run(&mut s, "file.save_as", json!({"path": b.display().to_string()}));
    let list = run(&mut s, "file.recent_list", json!({}));
    let names: Vec<&str> = list["files"].as_array().unwrap().iter().filter_map(|f| f["name"].as_str()).collect();
    assert_eq!(names, ["b.anicraft", "a.anicraft"], "most recent first; New doesn't clear it");
    // Open Recent by index opens it, and moves it to the top.
    run(&mut s, "file.open_recent", json!({"index": 1}));
    assert_eq!(s.path.as_deref(), Some(a.as_path()));
    assert_eq!(s.doc.stage.width, anicraft_doc::sample::demo().stage.width);
    assert_eq!(s.recent.entries().first().map(|e| e.path.clone()), Some(a.clone()));
    // Over unsaved work it asks first, like Open.
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 5, "h": 5}));
    assert!(s.confirmation("file.open_recent", &json!({"index": 1})).is_some_and(|c| c.irreversible));
    assert!(s.execute("file.open_recent", &json!({"index": 1})).is_err());
    run(&mut s, "file.open_recent", json!({"index": 1, "confirm": true}));
    assert_eq!(s.path.as_deref(), Some(b.as_path()));
    // A file that is gone stays listed, marked missing, and says so when opened.
    std::fs::remove_file(&a).unwrap();
    let list = run(&mut s, "file.recent_list", json!({}));
    let gone = list["files"].as_array().unwrap().iter().find(|f| f["name"] == "a.anicraft").unwrap();
    assert_eq!(gone["missing"], true);
    let e = s.execute("file.open_recent", &json!({"path": a.display().to_string()})).unwrap_err();
    assert!(e.to_string().contains("no longer there"), "{e}");
    // Pin, remove, clear.
    run(&mut s, "prefs.recent_pin", json!({"path": a.display().to_string()}));
    assert!(s.recent.entries().first().is_some_and(|e| e.pinned));
    run(&mut s, "prefs.recent_clear", json!({}));
    assert_eq!(s.recent.len(), 1, "the pinned file stays");
    run(&mut s, "prefs.recent_remove", json!({"index": 0}));
    assert!(s.recent.is_empty());
    // Hostile params never panic and never change the list.
    run(&mut s, "file.open", json!({"path": b.display().to_string()}));
    let before = s.recent.clone();
    for id in ["file.open_recent", "prefs.recent_pin", "prefs.recent_remove"] {
        for p in [json!({"index": u64::MAX}), json!({"index": -1}), json!({"path": ""}), json!({"index": 0, "path": "x"}), json!({"pinned": "x"}), json!("x"), Value::Null] {
            let _ = s.execute(id, &p);
            assert_eq!(s.recent, before, "{id} {p}");
        }
    }
    // Restored autosaves and untitled opens aren't recent files.
    let mut t = Session::default();
    run(&mut t, "file.open", json!({"path": b.display().to_string(), "unsaved": true}));
    run(&mut t, "file.open", json!({"path": b.display().to_string(), "untitled": true, "confirm": true}));
    assert!(t.recent.is_empty());
}
