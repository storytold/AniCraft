//! Layer view properties (ADR 0061): onion toggles and solo, colour, View as Outlines (on the
//! stage only), row height, Layer Properties as one step, the file, and onion skin of the
//! timeline around a symbol open in place.

use crate::Session;
use crate::onion::{Markers, OnionSkin};
use anicraft_doc::{ElementId, LayerId, Rgba};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn rect(s: &mut Session, x: f32) -> ElementId {
    ElementId(run(s, "draw.rect", json!({"x": x, "y": 10, "w": 20, "h": 20, "fill": "#000000", "stroke": null, "object": true}))["element"].as_u64().unwrap())
}

/// Two layers, each a square that moves 40 px right on frames 0, 2 and 4: `top` starts at x 0,
/// `bottom` at x 200. The playhead is on frame 2.
fn two_walks() -> (Session, LayerId, LayerId) {
    let mut s = Session::default();
    let bottom = s.layer;
    let b = rect(&mut s, 200.0);
    let top = LayerId(run(&mut s, "layer.new", json!({}))["layer"].as_u64().unwrap());
    let t = rect(&mut s, 0.0);
    for f in [2, 4] {
        run(&mut s, "timeline.goto", json!({"frame": f}));
        for (l, e) in [(bottom, b), (top, t)] {
            run(&mut s, "layer.select", json!({"layer": l.0}));
            run(&mut s, "timeline.insert_keyframe", json!({}));
            run(&mut s, "element.select", json!({"ids": [e.0]}));
            run(&mut s, "element.translate", json!({"dx": 40, "dy": 0}));
        }
    }
    run(&mut s, "timeline.goto", json!({"frame": 2}));
    (s, top, bottom)
}

fn layer(s: &Session, id: LayerId) -> &anicraft_doc::Layer {
    s.timeline().layer(id).unwrap()
}

/// What the stage shows with `skin`: the layers and left edges of the onion frames (tinted items).
fn ghosts(s: &Session, skin: &OnionSkin) -> Vec<(LayerId, f32)> {
    let dl = crate::render_service::display_list_with_onion(&s.doc, s.frame, Some(skin));
    dl.items.iter().filter(|i| !i.color.is_identity()).map(|i| (i.layer, i.path.bounds().transformed(&i.transform).min.x)).collect()
}

#[test]
fn a_layer_with_onion_off_is_not_onion_skinned() {
    let (mut s, top, bottom) = two_walks();
    let skin = OnionSkin { keyframes_only: true, ..OnionSkin::default() };
    let mut both = ghosts(&s, &skin);
    both.sort_by(|a, b| a.1.total_cmp(&b.1));
    assert_eq!(both, vec![(top, 0.0), (top, 80.0), (bottom, 200.0), (bottom, 280.0)], "frames 0 and 4 on both layers");
    let r = run(&mut s, "layer.set_onion", json!({"layer": bottom.0}));
    assert_eq!(r["onion"], false, "toggles");
    assert!(!layer(&s, bottom).onion && layer(&s, top).onion);
    assert!(ghosts(&s, &skin).iter().all(|(l, _)| *l == top), "only the top layer's frames show");
    // Its playhead frame still shows, at full strength.
    let dl = crate::render_service::display_list_with_onion(&s.doc, s.frame, Some(&skin));
    assert!(dl.items.iter().any(|i| i.layer == bottom && i.color.is_identity()));
    // Edit Multiple Frames isn't onion skin: it still shows and edits the layer's keyframes.
    let multi = OnionSkin { edit_multiple: true, ..skin };
    assert!(s.multi_frame_items(0, 4).iter().any(|(it, _)| it.layer == bottom && it.frame == 0));
    let dl = crate::render_service::display_list_with_onion(&s.doc, s.frame, Some(&multi));
    assert_eq!(dl.items.iter().filter(|i| i.layer == bottom).count(), 3);
    // Undo brings it back.
    run(&mut s, "edit.undo", json!({}));
    assert!(layer(&s, bottom).onion);
    assert!(s.execute("layer.set_onion", &json!({"layer": 999_999})).is_err());
    assert!(s.execute("layer.set_onion", &json!({"onion": "yes"})).is_err());
}

#[test]
fn solo_onion_keeps_one_layer_and_again_brings_back_all() {
    let (mut s, top, bottom) = two_walks();
    let folder = LayerId(run(&mut s, "layer.new_folder", json!({}))["layer"].as_u64().unwrap());
    let r = run(&mut s, "layer.set_onion", json!({"layer": top.0, "solo": true}));
    assert_eq!(r["onion_layers"], json!([top.0]));
    assert!(layer(&s, top).onion && !layer(&s, bottom).onion);
    assert!(layer(&s, folder).onion, "a folder has no frames to onion-skin; solo leaves it");
    // Solo on another layer moves the solo there.
    run(&mut s, "layer.set_onion", json!({"layer": bottom.0, "solo": true}));
    assert!(!layer(&s, top).onion && layer(&s, bottom).onion);
    // Again on the only layer left on: every layer is on again.
    run(&mut s, "layer.set_onion", json!({"layer": bottom.0, "solo": true}));
    assert!(layer(&s, top).onion && layer(&s, bottom).onion);
    // Setting it outright.
    run(&mut s, "layer.set_onion", json!({"layer": top.0, "onion": false}));
    run(&mut s, "layer.set_onion", json!({"layer": top.0, "onion": false}));
    assert!(!layer(&s, top).onion);
}

#[test]
fn outlines_use_the_layer_colour_on_the_stage_only() {
    let mut s = Session::default();
    let l = s.layer;
    rect(&mut s, 10.0);
    run(&mut s, "layer.set_color", json!({"color": "#ff8800"}));
    run(&mut s, "layer.toggle_outline", json!({}));
    let orange = Rgba::hex(0xff8800);
    assert_eq!(layer(&s, l).color, orange);
    // The stage: hairlines in the layer's colour.
    let stage = anicraft_doc::evaluate_scene(&s.doc, 0, 0);
    assert!(stage.items.iter().all(|i| i.outline == Some(orange)));
    assert!(s.display_list().items.iter().all(|i| i.outline == Some(orange)));
    // The movie (every export), and the player, draw the artwork as it is.
    let movie = anicraft_doc::evaluate(&s.doc, 0);
    assert!(!movie.items.is_empty() && movie.items.iter().all(|i| i.outline.is_none()));
    let player = anicraft_doc::evaluate_player(&s.doc, &anicraft_doc::player::PlayerState::default());
    assert!(player.items.iter().all(|i| i.outline.is_none()));
    let svg = crate::cmds_file::frame_svg(&s.doc, 0);
    assert!(!svg.to_ascii_lowercase().contains("ff8800"), "no outline colour in an SVG export");
    let png = |s: &Session| {
        let view = anicraft_render::View { transform: anicraft_geom::Affine::IDENTITY, width: 40, height: 40, background: Some(Rgba::WHITE) };
        anicraft_render::render(&anicraft_doc::evaluate(&s.doc, 0), &view).unwrap().0.to_rgba8()
    };
    let outlined = png(&s);
    run(&mut s, "layer.toggle_outline", json!({}));
    assert_eq!(outlined, png(&s), "an export looks the same with or without outline mode");
    // Inside a symbol on an outlined layer, the stage outlines everything in the layer's colour.
    run(&mut s, "layer.toggle_outline", json!({}));
    run(&mut s, "edit.select_all", json!({}));
    run(&mut s, "modify.convert_to_symbol", json!({"name": "Box"}));
    let stage = anicraft_doc::evaluate_scene(&s.doc, 0, 0);
    assert!(!stage.items.is_empty() && stage.items.iter().all(|i| i.outline == Some(orange)));
    assert!(anicraft_doc::evaluate(&s.doc, 0).items.iter().all(|i| i.outline.is_none()));
    // A colour is opaque, and a bad one is refused.
    run(&mut s, "layer.set_color", json!({"color": "#00ff0080"}));
    assert_eq!(layer(&s, l).color.a, 1.0);
    assert!(s.execute("layer.set_color", &json!({"color": "green"})).is_err());
    assert!(s.execute("layer.set_color", &json!({})).is_err());
}

#[test]
fn layer_height_is_100_200_or_300_percent() {
    let mut s = Session::default();
    let l = s.layer;
    assert_eq!(layer(&s, l).height(), 1);
    assert_eq!(run(&mut s, "layer.set_height", json!({"height": 300}))["height"], 300);
    assert_eq!(layer(&s, l).height(), 3);
    for bad in [json!(0), json!(150), json!(400), json!(-100), json!("tall")] {
        assert!(s.execute("layer.set_height", &json!({"height": bad})).is_err(), "{bad}");
    }
    assert!(s.execute("layer.set_height", &json!({})).is_err());
    assert_eq!(layer(&s, l).height(), 3);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(layer(&s, l).height(), 1);
}

#[test]
fn layer_properties_set_everything_as_one_step_or_nothing() {
    let (mut s, top, bottom) = two_walks();
    let undo = s.history.undo_len();
    let r = run(&mut s, "layer.set_properties", json!({"layer": bottom.0, "name": "Sky", "color": "#123456", "outline": true, "height": 200, "locked": true, "visible": false, "onion": false, "type": "guide"}));
    assert_eq!(r, json!({"layer": bottom.0, "name": "Sky", "type": "guide", "color": "#123456", "outline": true, "height": 200, "visible": false, "locked": true, "onion": false}));
    assert_eq!(s.history.undo_len(), undo + 1, "one undo step");
    assert_eq!(run(&mut s, "layer.properties", json!({"layer": bottom.0})), r);
    // The same type again changes nothing (a masked layer stays masked by the same mask).
    run(&mut s, "layer.set_properties", json!({"layer": bottom.0, "type": "guide"}));
    assert_eq!(run(&mut s, "layer.properties", json!({"layer": bottom.0}))["type"], "guide");
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    let back = run(&mut s, "layer.properties", json!({"layer": bottom.0}));
    assert_eq!((back["name"].as_str(), back["type"].as_str(), back["height"].as_u64()), (Some("Layer 1"), Some("normal"), Some(100)));
    // A bad value anywhere changes nothing.
    for bad in [json!({"name": "  "}), json!({"name": "X", "height": 250}), json!({"name": "X", "color": "red"}), json!({"name": "X", "type": "masked"}), json!({"name": "X", "type": "banana"}), json!({"name": "X", "locked": 1})] {
        let mut p = bad.clone();
        p["layer"] = json!(top.0);
        assert!(s.execute("layer.set_properties", &p).is_err(), "{bad}");
        assert_eq!(layer(&s, top).name, "Layer 2", "{bad}");
    }
    assert!(s.execute("layer.properties", &json!({"layer": 999_999})).is_err());
}

#[test]
fn onion_and_height_are_saved_and_older_files_open_with_the_defaults() {
    let (mut s, top, bottom) = two_walks();
    run(&mut s, "layer.set_onion", json!({"layer": top.0}));
    run(&mut s, "layer.set_height", json!({"layer": bottom.0, "height": 200}));
    let text = crate::format::to_json(&s.doc).unwrap();
    let back = crate::format::from_json(&text).unwrap();
    assert_eq!(back, s.doc);
    let tl = back.timeline(0);
    assert!(!tl.layer(top).unwrap().onion && tl.layer(bottom).unwrap().onion);
    assert_eq!((tl.layer(top).unwrap().height(), tl.layer(bottom).unwrap().height()), (1, 2));
    // Defaults are left out of the file, so older files and files without them read the same.
    let v: Value = serde_json::from_str(&text).unwrap();
    fn find(v: &Value, id: u64) -> Option<Value> {
        match v {
            Value::Object(m) if m.get("id") == Some(&json!(id)) && m.contains_key("keyframes") => Some(v.clone()),
            Value::Object(m) => m.values().find_map(|x| find(x, id)),
            Value::Array(a) => a.iter().find_map(|x| find(x, id)),
            _ => None,
        }
    }
    let by_id = |id: LayerId| find(&v, id.0).unwrap_or_default();
    assert!(by_id(top).get("name").is_some());
    assert_eq!((by_id(top).get("onion"), by_id(top).get("height")), (Some(&json!(false)), None));
    assert_eq!((by_id(bottom).get("onion"), by_id(bottom).get("height")), (None, Some(&json!(2))));
    let old: anicraft_doc::Layer = serde_json::from_value(json!({"id": 7, "name": "Old"})).unwrap();
    assert!(old.onion && old.height() == 1);
    // A height out of range in a file reads as the nearest one.
    let tall: anicraft_doc::Layer = serde_json::from_value(json!({"id": 7, "name": "Tall", "height": 200})).unwrap();
    assert_eq!(tall.height(), 3);
}

/// A scene with a square walking on a second layer (x 0, 40, 80 on frames 0, 2, 4), and an
/// instance of a symbol on the first, open in place on frame 2 of the scene.
fn parent_walk() -> (Session, LayerId) {
    let mut s = Session::default();
    let host = s.layer;
    rect(&mut s, 300.0);
    run(&mut s, "edit.select_all", json!({}));
    run(&mut s, "modify.convert_to_symbol", json!({"name": "Box"}));
    let inst = s.selection.ids().next().unwrap();
    let walker = LayerId(run(&mut s, "layer.new", json!({}))["layer"].as_u64().unwrap());
    let w = rect(&mut s, 0.0);
    run(&mut s, "timeline.insert_frame", json!({"layer": host.0, "frame": 0, "count": 4}));
    for f in [2, 4] {
        run(&mut s, "timeline.goto", json!({"frame": f}));
        run(&mut s, "timeline.insert_keyframe", json!({}));
        run(&mut s, "element.select", json!({"ids": [w.0]}));
        run(&mut s, "element.translate", json!({"dx": 40, "dy": 0}));
    }
    run(&mut s, "layer.select", json!({"layer": host.0}));
    run(&mut s, "timeline.goto", json!({"frame": 2}));
    run(&mut s, "symbol.edit", json!({"id": inst.0}));
    assert_eq!(s.editing.len(), 1);
    (s, walker)
}

#[test]
fn onion_skin_inside_a_symbol_shows_the_parent_timeline_frames_dimmed() {
    let (s, walker) = parent_walk();
    let pic = s.stage_picture();
    let front = pic.front.clone().unwrap();
    let parent = front.parent.clone().expect("the scene shows around the symbol");
    assert_eq!(parent.frame, 2);
    let skin = OnionSkin { markers: Markers::Relative { before: 2, after: 2 }, keyframes_only: true, ..OnionSkin::default() };
    let dl = crate::render_service::display_list_inside(&pic.doc, pic.frame, &front, &skin);
    let left = |i: &anicraft_doc::DisplayItem| i.path.bounds().transformed(&i.transform).min.x;
    // The walker on the scene: frame 2 (x 40) dimmed, frames 0 and 4 (x 0, 80) tinted and fainter.
    let mut walk: Vec<(f32, f32, bool)> = dl.items.iter().filter(|i| left(i) < 150.0).map(|i| (left(i), dl.effective_alpha(i), i.color.changes_colour())).collect();
    walk.sort_by(|a, b| a.0.total_cmp(&b.0));
    assert_eq!(walk.iter().map(|w| w.0).collect::<Vec<_>>(), vec![0.0, 40.0, 80.0], "the parent's onion frames around its frame 2");
    let dim = parent.dim;
    assert!((walk[1].1 - dim).abs() < 1e-4 && !walk[1].2, "the parent's own frame as dim as without onion skin");
    assert!(walk[0].2 && walk[2].2 && walk[0].1 < dim && walk[2].1 < dim, "its onion frames tinted, dimmer still");
    // The open symbol is in front at full strength.
    let box_item = dl.items.iter().find(|i| left(i) >= 290.0).unwrap();
    assert!((dl.effective_alpha(box_item) - 1.0).abs() < 1e-4);
    assert!(dl.items.last().is_some_and(|i| left(i) >= 290.0), "drawn over everything around it");
    // Without onion skin the picture is as before; the parent's layer toggle and Edit Multiple
    // Frames (the open symbol's frames only) leave its onion frames out.
    let plain = crate::render_service::display_list_with_onion(&pic.doc, pic.frame, None);
    assert_eq!(plain.items.len(), 2);
    let multi = OnionSkin { edit_multiple: true, ..skin };
    assert_eq!(crate::render_service::display_list_inside(&pic.doc, pic.frame, &front, &multi).items.len(), 2);
    let mut off = s;
    run(&mut off, "symbol.exit", json!({}));
    let inst = off.selection.ids().next().unwrap();
    run(&mut off, "layer.set_onion", json!({"layer": walker.0}));
    run(&mut off, "symbol.edit", json!({"id": inst.0}));
    let pic = off.stage_picture();
    let front = pic.front.clone().unwrap();
    assert_eq!(crate::render_service::display_list_inside(&pic.doc, pic.frame, &front, &skin).items.len(), 2);
}

#[test]
fn plug_ins_may_set_layer_properties() {
    for id in ["layer.set_onion", "layer.set_color", "layer.set_height", "layer.set_properties", "layer.properties"] {
        assert!(crate::find_command(id).is_some(), "{id}");
        assert!(crate::cmds_plugin::plugin_may_call(id).is_ok(), "{id}");
    }
}
