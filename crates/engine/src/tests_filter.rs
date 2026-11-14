//! Filter lists (ADR 0091): order, on/off, copy and paste, presets, and tweened values.

use super::*;
use anicraft_doc::{ElementKind, Filter, FilterEntry};
use serde_json::json;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    match s.execute(id, &p) {
        Ok(v) => v,
        Err(e) => panic!("{id} {p}: {e}"),
    }
}

/// A red 100 × 100 movie clip at (100, 100), selected.
fn clip() -> Session {
    let mut s = Session::default();
    run(&mut s, "draw.rect", json!({"x": 100, "y": 100, "w": 100, "h": 100, "fill": "#ff0000", "stroke": null}));
    run(&mut s, "modify.convert_to_symbol", json!({"name": "Box"}));
    run(&mut s, "symbol.set_kind", json!({"kind": "movie_clip"}));
    s
}

fn list(s: &Session) -> Vec<FilterEntry> {
    match &s.doc.timeline(s.scene).layers[0].keyframes[0].elements[0].kind {
        ElementKind::Symbol { filters, .. } => filters.clone(),
        _ => Vec::new(),
    }
}

fn kinds(s: &Session) -> Vec<(&'static str, bool)> {
    list(s).iter().map(|e| (e.filter.label(), e.enabled)).collect()
}

fn px(s: &Session, x: u32, y: u32) -> [f32; 4] {
    anicraft_render::render_frame(&s.doc, s.frame, 1.0).unwrap().pixel(x, y).unwrap()
}

#[test]
fn the_order_changes_the_pixels() {
    // A green glow outside the box, then Adjust Color taking all the saturation: the glow
    // turns grey. The other way round the glow is added after the colour is gone: green.
    let mut s = clip();
    let glow = json!({"type": "glow", "blur_x": 0, "blur_y": 0, "color": "#00ff00", "strength": 1});
    let grey = json!({"type": "adjust_color", "saturation": -100});
    // A glow with no blur is the artwork's own outline: widen it with a blur instead.
    let glow = {
        let mut g = glow;
        g["blur_x"] = json!(20);
        g["blur_y"] = json!(20);
        g["strength"] = json!(10);
        g
    };
    run(&mut s, "element.set_filters", json!({"filters": [glow, grey]}));
    let [r, g, b, a] = px(&s, 96, 150);
    assert!(a > 0.5 && (r - g).abs() < 0.02 && (g - b).abs() < 0.02, "glow then grey: a grey glow {:?}", [r, g, b, a]);
    let inside_grey = px(&s, 150, 150);
    run(&mut s, "filter.move", json!({"from": 1, "to": 0}));
    assert_eq!(kinds(&s), [("Adjust Color", true), ("Glow", true)]);
    let [r, g, b, a] = px(&s, 96, 150);
    assert!(a > 0.5 && g > 0.9 && r < 0.05 && b < 0.05, "grey then glow: a green glow {:?}", [r, g, b, a]);
    assert_eq!(px(&s, 150, 150), inside_grey, "the box is grey either way");
    // One undo step puts the order back.
    s.undo().unwrap();
    assert_eq!(kinds(&s), [("Glow", true), ("Adjust Color", true)]);
    // `to` past the end is the end; an index the list doesn't have is refused.
    run(&mut s, "filter.move", json!({"from": 0, "to": 99}));
    assert_eq!(kinds(&s), [("Adjust Color", true), ("Glow", true)]);
    assert!(s.execute("filter.move", &json!({"from": 5, "to": 0})).is_err());
    assert!(s.execute("filter.move", &json!({"from": 0})).is_err());
}

#[test]
fn a_filter_switched_off_is_as_if_absent_and_keeps_its_settings() {
    let blur = json!({"type": "blur", "blur_x": 6, "blur_y": 6});
    let glow = json!({"type": "glow", "blur_x": 12, "blur_y": 12, "color": "#0000ff", "strength": 3});
    let mut only = clip();
    run(&mut only, "element.set_filters", json!({"filters": [blur.clone()]}));
    let mut off = clip();
    run(&mut off, "element.set_filters", json!({"filters": [blur, glow]}));
    let both = anicraft_render::render_frame(&off.doc, 0, 1.0).unwrap();
    run(&mut off, "filter.enable", json!({"index": 1, "enabled": false}));
    assert_eq!(kinds(&off), [("Blur", true), ("Glow", false)]);
    // The oracle, the SVG and the HTML page all see the list without it.
    let a = anicraft_render::render_frame(&only.doc, 0, 1.0).unwrap();
    let b = anicraft_render::render_frame(&off.doc, 0, 1.0).unwrap();
    assert_eq!(a.data, b.data, "off draws exactly as absent");
    assert_ne!(both.data, b.data, "the glow did draw when on");
    let svg = |s: &Session| anicraft_render::svg::to_svg(&anicraft_doc::evaluate(&s.doc, 0), &s.doc.stage);
    assert_eq!(svg(&only), svg(&off));
    let html = |s: &Session| anicraft_render::html::movie(&s.doc, &Default::default()).json;
    assert_eq!(html(&only), html(&off));
    // Its settings stay, and the file keeps the switch.
    let Some(FilterEntry { filter: Filter::Glow { strength, blur_x, .. }, enabled: false }) = list(&off).get(1).cloned() else { panic!("{:?}", list(&off)) };
    assert_eq!((strength, blur_x), (3.0, 12.0));
    let text = crate::format::to_json(&off.doc).unwrap();
    assert!(text.contains("\"enabled\":false") || text.contains("\"enabled\": false"), "saved switched off");
    let back = crate::format::from_json(&text).unwrap();
    assert_eq!(back, off.doc, "round trip");
    // Back on: as before.
    run(&mut off, "filter.enable", json!({"enabled": true}));
    assert_eq!(anicraft_render::render_frame(&off.doc, 0, 1.0).unwrap().data, both.data);
    // All off, then removing one and all.
    run(&mut off, "filter.enable", json!({"enabled": false}));
    assert_eq!(anicraft_render::render_frame(&off.doc, 0, 1.0).unwrap().data, anicraft_render::render_frame(&clip().doc, 0, 1.0).unwrap().data, "all off is no filters");
    run(&mut off, "filter.remove", json!({"index": 0}));
    assert_eq!(kinds(&off), [("Glow", false)]);
    assert!(off.execute("filter.enable", &json!({"index": 3, "enabled": true})).is_err());
    assert!(off.execute("filter.enable", &json!({"index": 0})).is_err(), "enabled is required");
    run(&mut off, "filter.remove", json!({}));
    assert!(list(&off).is_empty());
    assert!(off.execute("filter.remove", &json!({})).is_err(), "nothing to remove");
    // `element.set_filters` takes the switch too.
    run(&mut off, "element.set_filters", json!({"filters": [{"type": "blur", "enabled": false}]}));
    assert_eq!(kinds(&off), [("Blur", false)]);
}

#[test]
fn graphics_take_no_filters() {
    let mut s = Session::default();
    run(&mut s, "draw.rect", json!({"x": 100, "y": 100, "w": 100, "h": 100, "fill": "#ff0000", "stroke": null}));
    run(&mut s, "modify.convert_to_symbol", json!({"name": "G"}));
    for (id, p) in [("filter.enable", json!({"enabled": false})), ("filter.remove", json!({})), ("filter.move", json!({"from": 0, "to": 1})), ("filter.copy", json!({}))] {
        let e = s.execute(id, &p).unwrap_err().to_string();
        assert!(e.contains("movie clip"), "{id}: {e}");
    }
}

#[test]
fn copy_and_paste_filters_between_instances() {
    let mut s = clip();
    run(&mut s, "element.set_filters", json!({"filters": [{"type": "blur"}, {"type": "glow", "enabled": false}]}));
    let first = s.selection.on(s.layer);
    assert!(s.execute("filter.paste", &json!({})).is_err(), "nothing copied yet");
    run(&mut s, "filter.copy", json!({}));
    // A second clip with a bevel of its own.
    run(&mut s, "draw.rect", json!({"x": 300, "y": 100, "w": 50, "h": 50, "fill": "#00ff00", "stroke": null}));
    run(&mut s, "modify.convert_to_symbol", json!({"name": "Other", "kind": "movie_clip"}));
    run(&mut s, "element.set_filters", json!({"filters": [{"type": "bevel"}]}));
    let second = s.selection.on(s.layer);
    assert_ne!(first, second);
    run(&mut s, "filter.paste", json!({}));
    let of = |s: &Session, id: anicraft_doc::ElementId| match &s.doc.timeline(s.scene).layers[0].keyframes[0].elements.iter().find(|e| e.id == id).unwrap().kind {
        ElementKind::Symbol { filters, .. } => filters.iter().map(|e| (e.filter.label(), e.enabled)).collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    assert_eq!(of(&s, second[0]), [("Bevel", true), ("Blur", true), ("Glow", false)], "added after its own, switches kept");
    run(&mut s, "filter.paste", json!({"replace": true}));
    assert_eq!(of(&s, second[0]), [("Blur", true), ("Glow", false)]);
    // One filter; past the limit is refused, whole.
    run(&mut s, "filter.copy", json!({"index": 1}));
    run(&mut s, "filter.paste", json!({}));
    assert_eq!(of(&s, second[0]).len(), 3);
    for _ in 0..13 {
        run(&mut s, "filter.paste", json!({}));
    }
    assert_eq!(of(&s, second[0]).len(), 16);
    assert!(s.execute("filter.paste", &json!({})).is_err());
    assert!(s.execute("filter.copy", &json!({"index": 40})).is_err());
    // Copied filters carry across to another document, as copied artwork does.
    s.replace_document(anicraft_doc::Document::default(), None);
    assert_eq!(s.filter_clipboard.as_ref().map(Vec::len), Some(1));
}

#[test]
fn filter_presets_save_apply_rename_delete_and_round_trip() {
    let mut s = clip();
    run(&mut s, "element.set_filters", json!({"filters": [{"type": "drop_shadow", "distance": 9}, {"type": "blur", "enabled": false}]}));
    assert!(s.execute("filter.preset_apply", &json!({"name": "Shadowed"})).is_err());
    let rev = s.revision;
    let v = run(&mut s, "filter.preset_save", json!({"name": " Shadowed "}));
    assert_eq!((v["name"].as_str(), v["replaced"].as_bool()), (Some("Shadowed"), Some(false)));
    // Saving over it asks (the old one can't come back), and replaces when confirmed.
    assert!(matches!(s.execute("filter.preset_save", &json!({"name": "shadowed"})), Err(EngineError::NeedsConfirmation(_))));
    run(&mut s, "filter.preset_save", json!({"name": "shadowed", "filters": [{"type": "glow", "color": "#ff00ff"}, {"type": "blur", "enabled": false}], "confirm": true}));
    assert_eq!(s.filter_presets.presets().len(), 1);
    // Saving and listing are outside the document and its undo.
    assert_eq!(s.revision, rev);
    let listed = run(&mut s, "filter.preset_list", json!({}));
    assert_eq!(listed["presets"][0]["filters"][1]["enabled"], json!(false));
    // Applying replaces the instance's filters (as Animate does), one undo step.
    run(&mut s, "filter.preset_apply", json!({"name": "SHADOWED"}));
    assert_eq!(kinds(&s), [("Glow", true), ("Blur", false)]);
    s.undo().unwrap();
    assert_eq!(kinds(&s), [("Drop Shadow", true), ("Blur", false)]);
    run(&mut s, "filter.preset_rename", json!({"name": "shadowed", "to": "Neon"}));
    assert!(s.execute("filter.preset_rename", &json!({"name": "nope", "to": "x"})).is_err());
    // A file round trip, into a session that has a preset of that name already.
    let dir = std::env::temp_dir().join(format!("anicraft-filter-presets-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("presets.json");
    let _ = std::fs::remove_file(&path);
    run(&mut s, "filter.preset_export", json!({"path": path.display().to_string()}));
    assert!(matches!(s.execute("filter.preset_export", &json!({"path": path.display().to_string()})), Err(EngineError::NeedsConfirmation(_))), "replacing the file asks");
    let mut other = Session::default();
    run(&mut other, "filter.preset_save", json!({"name": "neon", "filters": [{"type": "blur"}]}));
    let v = run(&mut other, "filter.preset_import", json!({"path": path.display().to_string()}));
    assert_eq!(v["imported"], json!(["Neon 2"]));
    assert_eq!(other.filter_presets.find("neon 2").map(|p| p.filters.clone()), s.filter_presets.find("neon").map(|p| p.filters.clone()));
    std::fs::write(dir.join("junk.json"), "{\"format\": \"anicraft-brushes\"}").unwrap();
    assert!(other.execute("filter.preset_import", &json!({"path": dir.join("junk.json").display().to_string()})).is_err());
    // Deleting asks (it can't be undone); the artwork keeps the filters it got.
    run(&mut s, "filter.preset_apply", json!({"name": "neon"}));
    assert!(matches!(s.execute("filter.preset_delete", &json!({"name": "neon"})), Err(EngineError::NeedsConfirmation(_))));
    run(&mut s, "filter.preset_delete", json!({"name": "neon", "confirm": true}));
    assert!(s.filter_presets.is_empty());
    assert_eq!(kinds(&s), [("Glow", true), ("Blur", false)]);
    assert!(s.execute("filter.preset_export", &json!({"path": dir.join("none.json").display().to_string()})).is_err(), "nothing to export");
    // Presets survive New and Open, like brushes.
    run(&mut s, "filter.preset_save", json!({"name": "Keep"}));
    s.replace_document(anicraft_doc::Document::default(), None);
    assert!(s.filter_presets.find("keep").is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn classic_tweens_carry_blur_and_glow_to_their_midpoint() {
    // Blur 0 → 20 and a glow's strength 0 → 200 %, over a linear tween of 10 frames.
    let mut s = clip();
    run(&mut s, "element.set_filters", json!({"filters": [{"type": "blur", "blur_x": 0, "blur_y": 0}, {"type": "glow", "strength": 0, "color": "#0000ff"}]}));
    run(&mut s, "timeline.goto", json!({"frame": 10}));
    run(&mut s, "timeline.insert_keyframe", json!({}));
    run(&mut s, "edit.select_all", json!({}));
    run(&mut s, "element.set_filters", json!({"filters": [{"type": "blur", "blur_x": 20, "blur_y": 20}, {"type": "glow", "strength": 2, "color": "#0000ff"}]}));
    run(&mut s, "timeline.goto", json!({"frame": 0}));
    run(&mut s, "timeline.create_tween", json!({"easing": "Linear"}));
    let dl = anicraft_doc::evaluate(&s.doc, 5);
    let g = dl.groups.iter().find(|g| !g.filters.is_empty()).expect("a filtered group");
    assert_eq!(g.filters.len(), 2);
    assert!(matches!(g.filters[0], Filter::Blur { blur_x, blur_y, .. } if (blur_x - 10.0).abs() < 1e-4 && (blur_y - 10.0).abs() < 1e-4), "{:?}", g.filters[0]);
    assert!(matches!(g.filters[1], Filter::Glow { strength, .. } if (strength - 1.0).abs() < 1e-4), "{:?}", g.filters[1]);
    // The midpoint renders as a still instance with the midpoint's values.
    let mut still = clip();
    run(&mut still, "element.set_filters", json!({"filters": [{"type": "blur", "blur_x": 10, "blur_y": 10}, {"type": "glow", "strength": 1, "color": "#0000ff"}]}));
    assert_eq!(anicraft_render::render_frame(&s.doc, 5, 1.0).unwrap().data, anicraft_render::render_frame(&still.doc, 0, 1.0).unwrap().data);
    // A filter switched off on the first keyframe stays off through the tween, values travelling.
    s.set_frame(0);
    run(&mut s, "edit.select_all", json!({}));
    run(&mut s, "filter.enable", json!({"index": 1, "enabled": false}));
    let dl = anicraft_doc::evaluate(&s.doc, 5);
    let g = dl.groups.iter().find(|g| !g.filters.is_empty()).expect("a filtered group");
    assert_eq!(g.filters.len(), 1, "only the blur draws");
    // Reordered on one keyframe, the lists no longer pair up: the first keyframe's holds.
    run(&mut s, "filter.move", json!({"from": 1, "to": 0}));
    let dl = anicraft_doc::evaluate(&s.doc, 5);
    let g = dl.groups.iter().find(|g| !g.filters.is_empty()).expect("a filtered group");
    assert!(matches!(g.filters[0], Filter::Blur { blur_x, .. } if blur_x == 0.0), "{:?}", g.filters);
}
