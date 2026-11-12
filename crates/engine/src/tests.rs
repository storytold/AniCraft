use super::*;
use anicraft_doc::sample;
use serde_json::json;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    match s.execute(id, &p) {
        Ok(v) => v,
        Err(e) => panic!("{id} {p}: {e}"),
    }
}

#[test]
fn command_ids_are_unique_and_well_formed() {
    let cmds = commands();
    let mut ids: Vec<&str> = cmds.iter().map(|c| c.id).collect();
    ids.sort_unstable();
    let n = ids.len();
    ids.dedup();
    assert_eq!(ids.len(), n, "duplicate command ids");
    for c in &cmds {
        assert!(c.id.contains('.'), "{}", c.id);
        assert!(!c.label.is_empty());
    }
}

#[test]
fn draw_undo_redo() {
    let mut s = Session::default();
    run(&mut s, "draw.rect", json!({"x": 10, "y": 10, "w": 100, "h": 50}));
    assert_eq!(s.visible_elements().len(), 1);
    assert_eq!(s.selection.len(), 1);
    let rev = s.revision;
    run(&mut s, "edit.undo", json!({}));
    assert!(s.visible_elements().is_empty());
    assert!(s.revision > rev);
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(s.visible_elements().len(), 1);
}

#[test]
fn gestures_coalesce_into_one_undo_step() {
    let mut s = Session::default();
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 10, "h": 10}));
    let steps = s.history_len();
    s.begin_gesture("Move");
    for _ in 0..30 {
        run(&mut s, "element.translate", json!({"dx": 1, "dy": 0}));
    }
    s.end_gesture();
    assert_eq!(s.history_len(), steps + 1);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.visible_elements()[0].transform.translation().x, 5.0);
}

#[test]
fn undo_step_in_a_large_document_costs_only_what_changed() {
    // 20 layers × 500 shapes × 120 frames: 200k element instances.
    let mut s = Session::new(sample::stress(20, 500, 120));
    let total = history::document_bytes(&s.doc);
    run(&mut s, "layer.select", json!({"index": 3}));
    s.set_frame(30);
    let id = s.visible_elements()[0].id.0;
    run(&mut s, "element.select", json!({"ids": [id]}));
    run(&mut s, "element.translate", json!({"dx": 5, "dy": 0}));
    let step = s.history().bytes();
    assert!(step * 100 < total, "one edit kept {step} bytes of a {total}-byte document");
    // Untouched layers are shared with the undo snapshot, pointer-for-pointer.
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.redo", json!({}));
    let before = s.doc.clone();
    run(&mut s, "element.translate", json!({"dx": 1, "dy": 0}));
    let shared = before.timeline(0).layers.iter().zip(&s.doc.timeline(s.scene).layers).filter(|(a, b)| std::sync::Arc::ptr_eq(a, b)).count();
    assert_eq!(shared, 19);
}

#[test]
fn dirty_tracks_the_save_point_through_undo() {
    let mut s = Session::default();
    assert!(!s.is_dirty());
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 10, "h": 10}));
    assert!(s.is_dirty());
    run(&mut s, "edit.undo", json!({}));
    assert!(!s.is_dirty(), "undoing back to the saved state is clean");
    run(&mut s, "edit.redo", json!({}));
    assert!(s.is_dirty());
    s.mark_saved();
    run(&mut s, "edit.undo", json!({}));
    assert!(s.is_dirty());
}

#[test]
fn state_ids_identify_content() {
    let mut s = Session::default();
    let a = s.state;
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 10, "h": 10}));
    let b = s.state;
    assert_ne!(a, b);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.state, a, "undo returns to the earlier state id (render cache hit)");
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(s.state, b);
    // a no-op command keeps the state
    let _ = s.execute("timeline.goto", &json!({"frame": 0}));
    assert_eq!(s.state, b);
}

#[test]
fn history_is_bounded_by_memory() {
    let mut s = Session::default();
    s.set_history_budget(64 * 1024);
    for i in 0..400 {
        run(&mut s, "draw.rect", json!({"x": i, "y": 0, "w": 10, "h": 10}));
    }
    assert!(s.history().bytes() <= 64 * 1024 + 8 * 1024, "{}", s.history().bytes());
    assert!(s.history().evicted > 0);
    assert!(s.can_undo());
}

#[test]
fn travel_to_moves_through_history() {
    let mut s = Session::default();
    for i in 0..5 {
        run(&mut s, "draw.rect", json!({"x": i * 20, "y": 0, "w": 10, "h": 10}));
    }
    s.travel_to(2).unwrap();
    assert_eq!(s.visible_elements().len(), 2);
    assert_eq!(s.history().redo_len(), 3);
    s.travel_to(5).unwrap();
    assert_eq!(s.visible_elements().len(), 5);
}

#[test]
fn display_list_cache_follows_state_and_frame() {
    let mut s = Session::new(sample::demo());
    let a = s.display_list();
    assert!(std::sync::Arc::ptr_eq(&a, &s.display_list()), "same state+frame is cached");
    s.set_frame(5);
    let b = s.display_list();
    assert!(!std::sync::Arc::ptr_eq(&a, &b));
    run(&mut s, "layer.toggle_visible", json!({}));
    assert_ne!(s.display_list().items.len(), b.items.len());
}

#[test]
fn parallel_sequence_export_matches_single_frames() {
    let dir = std::env::temp_dir().join(format!("anicraft-seq-{}", std::process::id()));
    let mut s = Session::new(sample::demo());
    run(&mut s, "file.export_png_sequence", json!({"dir": dir.display().to_string(), "from": 0, "to": 5, "scale": 0.2}));
    for f in 0..=5u32 {
        let on_disk = std::fs::read(dir.join(format!("frame_{:05}.png", f + 1))).unwrap();
        assert_eq!(on_disk, cmds_file::render_png(&s.doc, f, 0.2).unwrap());
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn discarding_unsaved_work_needs_confirmation() {
    let mut s = Session::default();
    // clean document: no confirmation
    assert!(s.confirmation("file.new", &json!({})).is_none());
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 10, "h": 10}));
    let c = s.confirmation("file.new", &json!({})).unwrap();
    assert!(c.irreversible);
    // refused without consent, document untouched
    let err = s.execute("file.new", &json!({})).unwrap_err();
    assert!(matches!(err, EngineError::NeedsConfirmation(_)));
    assert_eq!(s.visible_elements().len(), 1);
    assert!(s.execute("file.new_sample", &json!({})).is_err());
    // explicit consent proceeds
    run(&mut s, "file.new", json!({"confirm": true}));
    assert!(s.visible_elements().is_empty());
}

#[test]
fn overwriting_files_needs_confirmation() {
    let dir = std::env::temp_dir().join(format!("anicraft-confirm-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let png = dir.join("f.png");
    let mut s = Session::new(sample::demo());
    let p = json!({"path": png.display().to_string(), "scale": 0.1});
    assert!(s.confirmation("file.export_png", &p).is_none());
    run(&mut s, "file.export_png", p.clone());
    assert!(s.confirmation("file.export_png", &p).unwrap().irreversible);
    assert!(s.execute("file.export_png", &p).is_err());
    run(&mut s, "file.export_png", json!({"path": png.display().to_string(), "scale": 0.1, "confirm": true}));
    // saving over the document's own file is a normal save
    let doc = dir.join("d.anicraft");
    run(&mut s, "file.save_as", json!({"path": doc.display().to_string()}));
    assert!(s.confirmation("file.save", &json!({"path": doc.display().to_string()})).is_none());
    let mut t = Session::default();
    assert!(t.execute("file.save_as", &json!({"path": doc.display().to_string()})).is_err(), "another document must not silently replace it");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn undoable_bulk_deletions_ask_but_are_not_blocked() {
    let mut s = Session::new(sample::demo());
    let c = s.confirmation("layer.delete", &json!({})).unwrap();
    assert!(!c.irreversible);
    assert!(c.message.contains("Ball"));
    // headless callers may proceed without consent; undo restores
    run(&mut s, "layer.delete", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc.timeline(s.scene).layers.len(), 5);
    // empty layer: no question
    run(&mut s, "layer.new", json!({}));
    assert!(s.confirmation("layer.delete", &json!({})).is_none());
    // clearing a keyframe with artwork asks; routine element delete does not
    run(&mut s, "layer.select", json!({"index": 1}));
    assert!(s.confirmation("timeline.clear_keyframe", &json!({"frame": 0})).is_some());
    assert!(s.confirmation("edit.delete", &json!({})).is_none());
}

#[test]
fn copy_paste_cut_and_paste_in_place() {
    let mut s = Session::default();
    run(&mut s, "draw.rect", json!({"x": 10, "y": 10, "w": 20, "h": 20}));
    let orig = s.selection.ids().next().unwrap();
    run(&mut s, "edit.copy", json!({}));
    run(&mut s, "edit.paste", json!({}));
    let els = s.visible_elements();
    assert_eq!(els.len(), 2);
    assert_ne!(s.selection.ids().next().unwrap(), orig, "pasted elements get fresh ids");
    let pasted = els.iter().find(|e| e.id == s.selection.ids().next().unwrap()).unwrap();
    assert_eq!(pasted.transform.translation(), Vec2::new(30.0, 30.0));
    run(&mut s, "edit.paste_in_place", json!({}));
    let last = s.visible_elements().into_iter().find(|e| e.id == s.selection.ids().next().unwrap()).unwrap();
    assert_eq!(last.transform.translation(), Vec2::new(20.0, 20.0));
    // paste centred on a point
    run(&mut s, "edit.paste", json!({"x": 500, "y": 400}));
    assert_eq!(s.selection_bounds().center(), Vec2::new(500.0, 400.0));
    // cut removes, undo restores
    run(&mut s, "edit.cut", json!({}));
    assert_eq!(s.visible_elements().len(), 3);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.visible_elements().len(), 4);
    assert!(Session::default().execute("edit.paste", &json!({})).is_err(), "empty clipboard");
}

#[test]
fn clipboard_carries_symbols_into_another_document() {
    let mut s = Session::new(sample::demo());
    run(&mut s, "layer.select", json!({"index": 0}));
    // Select All takes every unlocked layer's artwork (ADR 0044).
    run(&mut s, "edit.select_all", json!({}));
    let copied = run(&mut s, "edit.copy", json!({}));
    let used = s.doc.library.len();
    run(&mut s, "file.new", json!({"confirm": true}));
    assert!(s.doc.library.is_empty());
    run(&mut s, "edit.paste_in_place", json!({}));
    assert!(s.doc.library.iter().any(|x| x.name == "Ball"), "the Ball symbol came along");
    assert_eq!(s.doc.library.len(), used);
    // Each copied layer lands on a layer of its own, made for it here.
    assert_eq!(s.doc.timeline(0).layers.len() as u64, copied["layers"].as_u64().unwrap());
    assert!(!anicraft_doc::evaluate(&s.doc, 0).items.is_empty(), "and it renders");
    let ids: Vec<u64> = s.doc.library.iter().map(|x| x.id.0).collect();
    assert!(s.doc.next_id > *ids.iter().max().unwrap());
}

#[test]
fn rotate_flip_and_free_transform() {
    let mut s = Session::default();
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 100, "h": 20}));
    let b0 = s.selection_bounds();
    run(&mut s, "modify.rotate_cw", json!({}));
    let b1 = s.selection_bounds();
    assert!((b1.width() - b0.height()).abs() < 0.5 && (b1.height() - b0.width()).abs() < 0.5, "{b1:?}");
    assert!(b1.center().distance(b0.center()) < 0.01, "rotates about the centre");
    run(&mut s, "modify.rotate_ccw", json!({}));
    run(&mut s, "modify.flip_h", json!({}));
    run(&mut s, "modify.flip_h", json!({}));
    let t = s.visible_elements()[0].transform;
    assert!((t.m[0] - 1.0).abs() < 1e-4 && t.m[1].abs() < 1e-4, "{t:?}");
    run(&mut s, "modify.transform", json!({"a": 2, "b": 0, "c": 0, "d": 1, "e": 5, "f": 0}));
    assert!((s.selection_bounds().width() - 2.0 * b0.width()).abs() < 1.0);
    assert!(s.execute("modify.transform", &json!({"a": 0, "b": 0, "c": 0, "d": 0, "e": 0, "f": 0})).is_err(), "degenerate");
    assert!(s.execute("modify.transform", &json!({"a": 1e6, "b": 0, "c": 0, "d": 1e6, "e": 0, "f": 0})).is_err(), "absurd scale");
}

#[test]
fn align_and_distribute() {
    let mut s = Session::default();
    for (x, y) in [(10, 50), (200, 10), (90, 120)] {
        run(&mut s, "draw.rect", json!({"x": x, "y": y, "w": 20, "h": 20, "stroke": null}));
    }
    run(&mut s, "edit.select_all", json!({}));
    run(&mut s, "align.left", json!({}));
    let lefts: Vec<f32> = s.selection.ids().map(|id| s.element_bounds(id).min.x).collect();
    assert!(lefts.iter().all(|x| (x - 10.0).abs() < 1e-3), "{lefts:?}");
    run(&mut s, "align.distribute_v", json!({}));
    let mut ys: Vec<f32> = s.selection.ids().map(|id| s.element_bounds(id).center().y).collect();
    ys.sort_by(f32::total_cmp);
    assert!(((ys[1] - ys[0]) - (ys[2] - ys[1])).abs() < 1e-3, "{ys:?}");
    // a single element aligns to the stage
    run(&mut s, "edit.deselect", json!({}));
    let id = s.visible_elements()[0].id.0;
    run(&mut s, "element.select", json!({"ids": [id]}));
    run(&mut s, "align.right", json!({}));
    assert!((s.selection_bounds().max.x - s.doc.stage.width as f32).abs() < 1e-3);
}

#[test]
fn svg_frame_export_writes_vectors_and_asks_before_replacing() {
    let dir = std::env::temp_dir().join(format!("anicraft-svg-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("frame.svg");
    let _ = std::fs::remove_file(&path);
    let mut s = Session::new(sample::demo());
    let p = json!({"path": path.display().to_string(), "frame": 10});
    let v = run(&mut s, "file.export_svg", p.clone());
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(v["bytes"].as_u64().unwrap() as usize, text.len());
    assert!(text.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1280\" height=\"720\""));
    assert!(text.contains("radialGradient") || text.contains("linearGradient"), "the sample's gradients stay vector");
    assert_eq!(text, crate::cmds_file::frame_svg(&s.doc, 10));
    assert!(s.execute("file.export_svg", &p).is_err(), "replacing a file needs consent");
    run(&mut s, "file.export_svg", json!({"path": path.display().to_string(), "frame": 10, "confirm": true}));
    assert!(s.execute("file.export_svg", &json!({"path": dir.join("x.svg").display().to_string(), "frame": 9999})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn svg_round_trip_looks_the_same_and_imports_as_one_undo_step() {
    let dir = std::env::temp_dir().join(format!("anicraft-svg-rt-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let svg = dir.join("scene.svg");
    let _ = std::fs::remove_file(&svg);
    let mut src = Session::new(sample::demo());
    run(&mut src, "file.export_svg", json!({"path": svg.display().to_string(), "frame": 10}));

    // Import it into an empty document of the same size, at the playhead (frame 3).
    let mut s = Session::default();
    run(&mut s, "timeline.insert_frame", json!({"count": 5}));
    run(&mut s, "timeline.goto", json!({"frame": 3}));
    let layers_before = s.doc.timeline(s.scene).layers.len();
    let v = run(&mut s, "file.import", json!({"path": svg.display().to_string()}));
    assert!(v["elements"].as_u64().unwrap() >= 5);
    assert_eq!(s.doc.timeline(s.scene).layers.len(), layers_before + 1);
    assert_eq!(s.active_layer().unwrap().name, "scene");
    assert_eq!(s.selection.len() as u64, v["elements"].as_u64().unwrap(), "the imported shapes are selected");
    assert!(anicraft_doc::evaluate(&s.doc, 2).items.is_empty(), "nothing before the playhead");

    // Pixel for pixel (nearly): the same picture comes back.
    let a = anicraft_render::render_frame(&src.doc, 10, 0.25).unwrap().to_rgba8();
    let b = anicraft_render::render_frame(&s.doc, 3, 0.25).unwrap().to_rgba8();
    let diff = a.iter().zip(&b).map(|(x, y)| (i32::from(*x) - i32::from(*y)).unsigned_abs() as f64).sum::<f64>() / a.len() as f64;
    assert!(diff < 1.0, "mean difference {diff:.3} of 255");

    // One undo step removes the whole import.
    s.undo().unwrap();
    assert_eq!(s.doc.timeline(s.scene).layers.len(), layers_before);
    // Anything but SVG, or a file that is not SVG, is refused cleanly.
    assert!(s.execute("file.import", &json!({"path": dir.join("x.png").display().to_string()})).is_err());
    let fake = dir.join("fake.svg");
    std::fs::write(&fake, "<html/>").unwrap();
    assert!(s.execute("file.import", &json!({"path": fake.display().to_string()})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn animated_gif_export() {
    let dir = std::env::temp_dir().join(format!("anicraft-gif-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ball.gif");
    let mut s = Session::new(sample::demo());
    let v = run(&mut s, "file.export_gif", json!({"path": path.display().to_string(), "scale": 0.125}));
    assert_eq!(v["frames"], 48);
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(&bytes[..6], b"GIF89a");
    // decode: 48 frames, 160×90, total delay ≈ 2 s at 24 fps
    let mut opts = gif::DecodeOptions::new();
    opts.set_color_output(gif::ColorOutput::Indexed);
    let mut dec = opts.read_info(std::fs::File::open(&path).unwrap()).unwrap();
    assert_eq!((dec.width(), dec.height()), (160, 90));
    let (mut n, mut total) = (0, 0u32);
    while let Some(f) = dec.read_next_frame().unwrap() {
        n += 1;
        total += u32::from(f.delay);
    }
    assert_eq!(n, 48);
    assert_eq!(total, 200, "delays sum to exactly 2 s");
    // overwriting asks; absurd sizes are refused
    assert!(s.execute("file.export_gif", &json!({"path": path.display().to_string()})).is_err());
    assert!(s.execute("file.export_gif", &json!({"path": dir.join("big.gif").display().to_string(), "scale": 8})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn gif_delays_are_exact_on_average() {
    let d = cmds_file::gif_delays(30, 30.0);
    assert_eq!(d.iter().map(|&x| u32::from(x)).sum::<u32>(), 100);
    assert!(d.iter().all(|&x| x == 3 || x == 4));
    assert!(cmds_file::gif_delays(5, f32::NAN).iter().all(|&x| x >= 2));
}

#[test]
fn failed_command_is_atomic() {
    let mut s = Session::default();
    let before = s.doc.clone();
    assert!(s.execute("draw.rect", &json!({"x": 0, "y": 0, "w": 10})).is_err());
    assert_eq!(s.doc, before);
    assert!(!s.can_undo());
}

#[test]
fn keyframes_and_tween_from_commands() {
    let mut s = Session::default();
    run(&mut s, "draw.ellipse", json!({"x": 0, "y": 0, "w": 20, "h": 20}));
    run(&mut s, "timeline.goto", json!({"frame": 10}));
    run(&mut s, "timeline.insert_keyframe", json!({}));
    run(&mut s, "edit.select_all", json!({}));
    run(&mut s, "element.translate", json!({"dx": 100, "dy": 0}));
    run(&mut s, "timeline.goto", json!({"frame": 0}));
    run(&mut s, "timeline.create_tween", json!({"easing": "Linear"}));
    assert_eq!(s.frame_count(), 11);
    s.set_frame(5);
    let el = &s.visible_elements()[0];
    assert!((el.transform.translation().x - 60.0).abs() < 1e-3, "{:?}", el.transform);
    // F6 inside the tween keeps the motion unchanged.
    run(&mut s, "timeline.insert_keyframe", json!({}));
    let el2 = &s.visible_elements()[0];
    assert!((el2.transform.translation().x - 60.0).abs() < 1e-3);
    s.set_frame(7);
    assert!((s.visible_elements()[0].transform.translation().x - 80.0).abs() < 1e-3);
}

#[test]
fn editing_inside_tween_auto_keys() {
    let mut s = Session::new(sample::demo());
    let ball = s.doc.timeline(s.scene).layers[0].id;
    run(&mut s, "layer.select", json!({"layer": ball.0}));
    s.set_frame(6);
    run(&mut s, "edit.select_all", json!({}));
    let keys_before = s.active_layer().map_or(0, |l| l.keyframes.len());
    run(&mut s, "element.translate", json!({"dx": 0, "dy": -50}));
    assert_eq!(s.active_layer().map_or(0, |l| l.keyframes.len()), keys_before + 1);
}

#[test]
fn insert_and_remove_frames() {
    let mut s = Session::default();
    run(&mut s, "timeline.insert_frame", json!({"frame": 9}));
    assert_eq!(s.frame_count(), 10);
    run(&mut s, "timeline.insert_frame", json!({"frame": 3}));
    assert_eq!(s.frame_count(), 11);
    run(&mut s, "timeline.remove_frame", json!({"frame": 0}));
    assert_eq!(s.frame_count(), 10);
    assert!(s.execute("timeline.remove_frame", &json!({"frame": 50})).is_err());
}

#[test]
fn layers() {
    let mut s = Session::default();
    run(&mut s, "layer.new", json!({"name": "Top"}));
    assert_eq!(s.doc.timeline(s.scene).layers[0].name, "Top");
    run(&mut s, "layer.toggle_lock", json!({}));
    assert!(s.execute("draw.rect", &json!({"x": 0, "y": 0, "w": 10, "h": 10})).is_err());
    run(&mut s, "layer.delete", json!({}));
    assert!(s.execute("layer.delete", &json!({})).is_err(), "last layer must stay");
}

#[test]
fn convert_to_symbol_and_break_apart_preserve_pixels() {
    let mut s = Session::default();
    run(&mut s, "draw.rect", json!({"x": 10, "y": 10, "w": 60, "h": 40, "fill": "#ff0000"}));
    run(&mut s, "draw.ellipse", json!({"x": 50, "y": 30, "w": 60, "h": 60, "fill": "#00ff00"}));
    let before = commands_png(&s);
    run(&mut s, "edit.select_all", json!({}));
    run(&mut s, "modify.convert_to_symbol", json!({"name": "Thing"}));
    assert_eq!(s.doc.library.len(), 1);
    assert_eq!(s.visible_elements().len(), 1);
    assert_eq!(commands_png(&s), before, "converting must not move anything");
    run(&mut s, "modify.break_apart", json!({}));
    assert_eq!(s.visible_elements().len(), 2);
    assert_eq!(commands_png(&s), before);
}

fn commands_png(s: &Session) -> Vec<u8> {
    match anicraft_render::render_frame(&s.doc, s.frame, 0.25) {
        Ok(p) => p.to_rgba8(),
        Err(e) => panic!("{e}"),
    }
}

#[test]
fn pick_selects_topmost() {
    let mut s = Session::default();
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 100, "h": 100}));
    let a = s.selection.ids().next().unwrap();
    run(&mut s, "draw.rect", json!({"x": 50, "y": 50, "w": 100, "h": 100}));
    let b = s.selection.ids().next().unwrap();
    run(&mut s, "element.pick", json!({"x": 75, "y": 75}));
    assert_eq!(s.selection.ids().collect::<Vec<_>>(), vec![b]);
    run(&mut s, "element.pick", json!({"x": 10, "y": 10}));
    assert_eq!(s.selection.ids().collect::<Vec<_>>(), vec![a]);
    run(&mut s, "element.pick", json!({"x": 500, "y": 500}));
    assert!(s.selection.is_empty());
}

#[test]
fn save_load_round_trip() {
    let dir = std::env::temp_dir().join(format!("anicraft-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("demo.anicraft");
    let mut s = Session::new(sample::demo());
    run(&mut s, "file.save", json!({"path": path.display().to_string()}));
    let mut t = Session::default();
    run(&mut t, "file.open", json!({"path": path.display().to_string()}));
    assert_eq!(s.doc, t.doc);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Adversarial params for every command: must return Ok or Err, never panic.
#[test]
fn panic_hunt_every_command() {
    let mut hostile: Vec<Value> = vec![
        Value::Null,
        json!({}),
        json!([]),
        json!("x"),
        json!({"x": null, "y": "a", "w": -1e30, "h": f64::MAX}),
        json!({"x": 1e300, "y": -1e300, "w": 0, "h": 0}),
        json!({"x": 0, "y": 0, "w": 1e7, "h": 1e7, "fill": "#zzzzzz"}),
        json!({"x": 1, "y": 1, "w": 10, "h": 10, "radius": -5, "stroke_width": 1e9}),
        json!({"frame": u64::MAX, "layer": u64::MAX}),
        json!({"frame": 99_999, "layer": 0}),
        json!({"frame": -1, "index": 1e9}),
        json!({"ids": [1, 2, 3, u64::MAX], "add": "yes"}),
        json!({"ids": "nope"}),
        json!({"dx": f64::MIN_POSITIVE, "dy": 1e-30, "alpha": 2.0}),
        json!({"sx": 0, "sy": 1e9, "rotation": 1e9}),
        json!({"easing": {"bezier": [1, 2]}, "label": ""}),
        json!({"easing": {"classic": "a"}}),
        json!({"easing": 7}),
        json!({"cx": 0, "cy": 0, "outer": 1e9, "points": 2}),
        json!({"cx": 0, "cy": 0, "outer": 10, "inner": -10, "points": 64}),
        json!({"x0": 0, "y0": 0, "x1": 0, "y1": 0}),
        json!({"name": "", "symbol": 0}),
        json!({"color": null, "width": -1}),
        json!({"width": 0, "height": 999_999, "fps": 0}),
        json!({"background": "#12345"}),
        json!({"layers": [1, 2, 3], "from": 0, "to": 100, "by": -5, "layer_by": -9999999}),
        json!({"from": 0, "to": 50, "by": 2, "layer_by": 1}),
        json!({"x": 0, "y": 0, "gap": 1e9, "color": "#zz", "width": 0}),
        json!({"x": 50, "y": 50, "gap": 999, "width": 999}),
        json!({"layers": [0, 1, 2, u64::MAX], "frame": 3, "count": 10_000, "from": 0, "to": 100_000, "by": -100_000}),
        json!({"layers": [], "from": 99_999, "by": 1e30, "count": 0}),
        json!({"d": "M0 0 L1e30 1e30 Z", "points": [[0, 0], [1e30, 1]], "mode": "ink"}),
        json!({"d": "M 0 0 C", "points": [[0, 0]], "mode": "straighten", "tolerance": 0}),
        json!({"id": 3, "points": [[0, 0], [99, 9], [1, 2]], "dx": 1e30, "dy": 0, "cmds": [0, 1, 2, 3, 4, 5, 99], "cmd": 0, "smooth": true}),
        json!({"id": 5, "points": [[1, 1]], "dx": 3, "dy": 4, "x": 0, "y": 0, "slop": 1e9, "cmds": [1, 1, 0], "cmd": 1, "smooth": false}),
        json!({"id": 3, "profile": [[0, 1e30, -5], ["x"], [0.5, 16, 16]], "index": u64::MAX, "t": -0.5, "left": 1e9, "width": f64::MAX}),
        json!({"id": 3, "profile": "taper_both", "index": 0, "t": 1, "x": 1e30, "y": -1e30, "left": 0, "right": 16}),
        json!({"profile": (0..500).map(|i| json!([f64::from(i) / 500.0, 2, 2])).collect::<Vec<_>>()}),
        // Brush presets, curves and the Paint Brush (ADR 0068): junk, extremes, both modes.
        json!({"points": [[0, 0, 0.5], [300, 40, 1], [600, 0, 0]], "preset": "chalk", "as_stroke": true, "curve": [[0, 1], [0.5, 0], [1, 1]], "taper_start": 50, "taper_end": 50}),
        json!({"points": [[0, 0], [1e5, 1e5]], "preset": "dotted", "spacing": 5, "jitter": 50, "size": 2000, "name": "x", "to": "y"}),
        json!({"points": [[0, 0], [10, 0]], "preset": "nope", "curve": "wobbly", "spacing": 1, "jitter": -1, "name": "", "from": "missing", "names": [1, 2]}),
        json!({"points": [[f64::MAX, 0]], "curve": [[f64::NAN, 2]], "pressure_size": "yes", "pressure_opacity": 1, "as_stroke": "no", "name": "Chalk", "to": "Pencil"}),
        json!({"points": (0..2000).map(|i| json!([f64::from(i), f64::from(i % 7) * 3.0, f64::from(i % 3) * 0.5])).collect::<Vec<_>>(), "preset": "ink_pen", "as_stroke": true, "name": "a".repeat(5000)}),
        // Stroke styles (ADR 0046): junk, extremes and patterns that would be very fine.
        json!({"cap": "none", "join": "miter", "miter": 100, "dash": [0, 1e-30], "dash_offset": -1e4, "style": "dashed"}),
        json!({"cap": 7, "join": "mitre", "miter": f64::NAN, "dash": vec![1e4; 16], "dot_spacing": 0, "style": "dotted"}),
        json!({"style": "dotted", "dot_spacing": 0, "x0": 0, "y0": 0, "x1": 1e6, "y1": 1, "stroke_width": 1000}),
        json!({"dash": [0.001, 0.001], "x0": -1e6, "y0": 0, "x1": 1e6, "y1": 0, "x": 0, "y": 0}),
        json!({"dash": "x", "cap": "square", "style": "ragged", "dash_offset": 1e30}),
        json!({"sound": SOUND, "sync": "stream", "repeat": 9999, "loop": true, "extend": true, "frame": 99_999}),
        json!({"sound": SOUND, "sync": "nope", "repeat": 0, "loop": "x", "name": ""}),
        json!({"sound": SOUND, "frame": 3, "extend": true, "name": "renamed"}),
        json!({"sound": u64::MAX, "repeat": 1e9}),
        json!({"sound": "a", "sync": 3}),
        // Effects, envelopes and layer volumes (ADR 0039).
        json!({"effect": "custom", "envelope": [{"at": 1e300, "left": 2, "right": -1}], "volume": 1e30, "muted": "x"}),
        json!({"effect": "fade_in", "envelope": (0..40).map(|i| json!([i, 0.5, 0.5])).collect::<Vec<_>>(), "volume": -0.5}),
        json!({"effect": "nope", "envelope": [[0, 1]], "volume": 0.5, "muted": true}),
        json!({"effect": "custom", "sync": "stream", "loop": true, "envelope": [[0, 0, 1], [86_400, 1, 0]]}),
        json!({"effect": 7, "envelope": "x", "volume": f64::NAN}),
        json!({"hint": "a", "x": 1e7, "y": -1e7, "side": "end"}),
        json!({"hint": u64::MAX, "side": "middle", "x": 0, "y": 0}),
        json!({"hint": "zz", "frame": 0}),
        json!({"filters": [{"type": "bevel", "blur_x": 1e30, "distance": -1e30, "angle": f64::MAX, "side": "full", "shadow_color": "#zz"}]}),
        json!({"filters": [{"type": "gradient_glow", "stops": [1, "x", null]}, {"type": "gradient_bevel", "stops": "x"}]}),
        json!({"filters": [{"type": "gradient_bevel", "stops": [{"offset": 1e30, "color": {"r": 9, "g": -9, "b": 0, "a": 2}}], "strength": -1, "quality": 255}]}),
        json!({"orient": true, "d": "M0 0 C 1e5 -1e5 -1e5 1e5 10 10 Q 5 5 0 0 Z M 9 9 L 1 1"}),
        json!({"orient": "x", "d": 5, "property": "x", "value": 1e6}),
        json!({"orient": false, "d": "M 1e6 1e6 L -1e6 -1e6", "dx": 1e6, "dy": -1e6}),
        // Motion path points (ADR 0038): ends, repeats and out-of-range anchors, segments.
        json!({"anchors": [0, 1, 2, 3, u64::MAX], "anchor": 1, "smooth": true, "segment": 1, "t": 0.5}),
        json!({"anchors": [1, 1, 1], "anchor": u64::MAX, "smooth": false, "segment": u64::MAX, "t": f64::NAN}),
        json!({"x": 75, "y": 0, "slop": 1000, "anchors": [1], "anchor": 0, "smooth": true, "dx": -0.5, "dy": 3}),
        // Brush strokes: huge tips over long drags, specks, pressure at the limits.
        json!({"points": [[0, 0, 0.0], [99_999, 99_999, 1.0], [-99_999, 5, 0.5]], "size": 2000, "shape": "square", "angle": 1e6, "roundness": 5, "smoothing": 0, "mode": "behind", "unit": 0.001}),
        json!({"points": (0..3000).map(|i| json!([i % 7, (i * 13) % 11, f64::from(i % 3) * 0.5])).collect::<Vec<_>>(), "size": 0.1, "mode": "fills", "smoothing": 100, "unit": 1000}),
        json!({"points": [[5, 5, -1], [6, 6, "x"]], "shape": 3, "mode": null, "roundness": f64::NAN}),
        // Eraser and Eyedropper (ADR 0034): strokes over the raw shapes, a tip larger than the
        // stage, picks into the other swatch, styles applied to whatever is selected.
        json!({"points": [[0, 0], [40, 40], [90, 5]], "size": 30, "mode": "selected_fills", "x": 30, "y": 30, "slop": 0}),
        json!({"points": [[45, 45]], "size": 2000, "mode": "inside", "x": 45, "y": 45, "other": true, "fill": false, "stroke": true}),
        json!({"points": [[5, 5], [55, 55]], "size": 4, "mode": "lines", "shape": "square", "x": 1, "y": 1, "fill": "yes"}),
        // The Selection tool on raw shapes (ADR 0041): corners and edges of the raw rectangles,
        // stale and hostile point lists, huge snaps and marquees, segments that aren't there.
        json!({"x": 30, "y": 30, "to_x": 1e7, "to_y": -1e7, "snap": 1e9, "merge": false, "dx": 1e7, "dy": -1e7}),
        json!({"x": 60, "y": 45, "to_x": 30.1, "to_y": 30, "snap": 10, "points": [[1, 99_999], [u64::MAX, 0]], "cmd": 1, "t": 2}),
        json!({"x": 45, "y": 30, "dx": 5, "dy": 5, "cmd": u64::MAX, "t": f64::NAN, "id": 2, "slop": 1e9}),
        json!({"x0": 10, "y0": 10, "x1": 45, "y1": 75, "add": true, "from": [1, 2, 3], "snap": 3}),
        json!({"x0": -1e7, "y0": 1e7, "x1": 1e7, "y1": -1e7, "add": false, "from": [45, 45], "snap": 0.2}),
        json!({"x0": 30, "y0": 30, "x1": 30, "y1": 30, "point_slop": -5, "ids": [u64::MAX], "points": "x"}),
        // Subselection on raw shapes (ADR 0042): handles that aren't there or move too far,
        // anchors to move along that are hostile, connected sets of anything.
        json!({"id": 3, "cmd": 1, "slot": 2, "dx": 1e30, "dy": 0, "x": 30, "y": 30, "to_x": 31, "to_y": 30, "with": [[30, 30], [1e30, 0]]}),
        json!({"id": 5, "cmd": u64::MAX, "slot": 9, "dx": 1, "dy": 1, "with": "x", "x0": 0, "y0": 0, "x1": 1e9, "y1": 1e9}),
        json!({"id": 4, "cmd": 2, "slot": 1, "dx": -3, "dy": 2, "x": 60, "y": 60, "to_x": 61, "to_y": 61, "with": [[60, 60], [30, 90], [90, 30]], "merge": false}),
        // Kept scripts (ADR 0013 tier 1): odd text, unknown languages, class names that aren't.
        json!({"text": "\u{0}\r\n]]>\u{202e}stop();", "language": "other", "class": "a.b", "base": "c", "symbol": 1}),
        json!({"text": "x".repeat(1 << 20), "language": 3, "class": "not a class", "base": ""}),
        json!({"text": "", "language": "ActionScript3", "class": "", "base": "Orphan"}),
        // Bones (ADR 0043): the rigged session's bones are 100 and 102 (its armature layer 101),
        // its instances 10–12.
        json!({"bone": 102, "x": 1e30, "y": -1e30, "local": [1e9, "a"], "frame": 99_999}),
        json!({"bone": 102, "x": 0, "y": 0, "local": [f64::MAX, 0], "angle": 1e9, "rotate_by": -1e9, "dx": 1e7, "dy": -1e7}),
        json!({"bone": 100, "min": 400, "max": -400, "constrain": true, "pinned": "x"}),
        json!({"bone": 100, "min": 10, "max": 10, "constrain": "yes", "pinned": true, "x": 250, "y": 250}),
        json!({"from": 10, "to": 12, "head": [0, 0], "tail": [1e30, 0], "name": "x".repeat(300)}),
        json!({"from": 12, "to": 10, "head": [5], "tail": "x", "bone": u64::MAX}),
        json!({"from": 11, "to": 13, "head": [-1e6, 1e6], "layer": 101}),
        // Edit Multiple Frames (ADR 0047): ranges past the end or reversed, items that aren't
        // there or aren't triples, moves at the limits.
        json!({"first": 0, "last": 99_999, "dx": 1e7, "dy": -1e7, "items": [[0, 0, 1], "x", [u64::MAX, u64::MAX, u64::MAX]]}),
        json!({"first": u64::MAX, "last": 0, "dx": 5, "dy": 5, "items": []}),
        json!({"first": 40, "last": 0, "dx": 3, "dy": -2}),
        json!({"first": 0, "last": 100_001, "dx": f64::NAN, "dy": 0, "items": [[1, 2]]}),
        // Edit Multiple Frames, complete (ADR 0051): transforms at the limits, pivots and edges
        // that aren't, pastes onto ranges with nothing in them, merges of what isn't raw.
        json!({"first": 0, "last": 99_999, "a": 0, "b": 0, "c": 0, "d": 0, "e": 1e30, "f": 0, "merge": "x"}),
        json!({"first": 0, "last": 50, "a": 1e9, "b": 1, "c": 1, "d": 1e-9, "e": 0, "f": f64::NAN}),
        json!({"first": 0, "last": 50, "rotate": 1e9, "scale_x": 1e-9, "scale_y": -1e9, "skew_x": 90, "skew_y": -90, "pivot": [1e30, 0]}),
        json!({"first": 0, "last": 50, "rotate": 45, "skew_x": 89.999, "pivot": [5], "edge": "nope", "to_stage": "x"}),
        json!({"first": 0, "last": 50, "scale_x": -1, "pivot": [0, 0], "edge": "vcenter", "to_stage": true, "merge": false}),
        json!({"first": 3, "last": 1, "edge": "right", "layer": u64::MAX, "items": [[0, 0, 0]]}),
        json!({"first": 0, "last": 99_999, "layer": 0, "items": (0..200).map(|i| json!([i, i, i])).collect::<Vec<_>>()}),
        // 9-slice guides (ADR 0055): crossed, huge, not numbers, moved while turned off.
        json!({"on": true, "left": 90, "right": 10, "top": 1e30, "bottom": -1e30, "symbol": 1}),
        json!({"on": false, "left": 5}),
        json!({"on": "x", "left": f64::NAN, "right": "y", "top": null}),
        json!({"left": -1e7, "right": 1e7, "top": -1e7, "bottom": 1e7}),
        json!({"on": true, "symbol": u64::MAX}),
        // Publish Settings (ADR 0060): ends past the movie, junk scales and names, whole
        // profiles out of range.
        json!({"format": "gif", "from": u64::MAX, "to": -1, "scale": f64::NAN, "output": "{name}\u{0}.gif"}),
        json!({"format": "png", "frame": u64::MAX, "scale": 1e-30, "output": "a/b/c/../../{name}"}),
        json!({"format": "sprite_sheet", "columns": u64::MAX, "padding": 256, "from": null, "to": null, "output": "{name}{name}{name}.png"}),
        json!({"profile": {"gif": {"scale": 1e30, "range": {"from": 5, "to": 1}, "output": ""}, "audio": {"bits": 3, "rate": 0}, "html": {"title": "x"}}}),
        json!({"profile": {"png": {"frame": 99_999, "enabled": true}}, "format": "svg", "enabled": true}),
    ];
    // File commands are exercised separately (they touch the disk).
    let skip = |id: &str| id.starts_with("file.");
    // A document with a sound in the library and on a keyframe.
    const SOUND: u64 = 900_000;
    let mut with_sound = sample::demo();
    let wav = crate::cmds_sound::test_wav(8000, &[0.25; 4000]);
    with_sound.sounds.push(Arc::new(anicraft_doc::Sound::from_data(anicraft_doc::SoundId(SOUND), "t.wav", wav).unwrap()));
    if let Some(k) = Arc::make_mut(&mut with_sound.timeline_mut(0).layers[0]).keyframe_mut(0) {
        k.sound = Some(anicraft_doc::FrameSound { effect: anicraft_doc::SoundEffect::FadeOut, ..anicraft_doc::FrameSound::new(anicraft_doc::SoundId(SOUND)) });
    }
    // ...and a stream with a custom envelope inside every symbol (sounds in symbols, ADR 0039).
    for sym in &mut with_sound.library {
        let tl = &mut Arc::make_mut(sym).timeline;
        if let Some(l) = tl.layers.first_mut()
            && let Some(k) = Arc::make_mut(l).keyframe_mut(0)
        {
            let env = anicraft_doc::SoundEffect::Custom(vec![anicraft_doc::EnvelopePoint::new(0.0, 0.0, 1.0), anicraft_doc::EnvelopePoint::new(0.1, 1.0, 0.0)]);
            k.sound = Some(anicraft_doc::FrameSound { sync: anicraft_doc::SoundSync::Stream, looping: true, effect: env, ..anicraft_doc::FrameSound::new(anicraft_doc::SoundId(SOUND)) });
        }
    }
    // A motion tween on an oriented motion path, playhead mid-span (ADR 0030).
    let mut on_path = Session::default();
    for (id, p) in [("draw.rect", json!({"x": 0, "y": 0, "w": 10, "h": 10})), ("timeline.create_motion_tween", json!({})), ("motion.set_path", json!({"d": "M0 0 C 50 -80 100 80 150 0"})), ("motion.orient", json!({"orient": true}))] {
        on_path.execute(id, &p).unwrap();
    }
    on_path.set_frame(9);
    // Merge drawing (ADR 0031): every drawing command cuts into raw shapes already there.
    let mut raw = Session::default();
    raw.style.object_drawing = false;
    for p in [json!({"x": 0, "y": 0, "w": 60, "h": 60}), json!({"x": 30, "y": 30, "w": 60, "h": 60, "fill": "#ff0000"}), json!({"x": 1, "y": 1, "w": 10, "h": 10, "radius": 5})] {
        raw.execute("draw.rect", &p).unwrap();
    }
    // An armature: two bones, posed at two frames with a limit and a pin (ADR 0043).
    let (mut rig, b1, b2) = crate::tests_bone::rigged();
    for (id, p) in [("bone.pose", json!({"bone": b1, "angle": 30, "frame": 8})), ("bone.set_limits", json!({"bone": b2, "min": -30, "max": 30})), ("bone.pin", json!({"bone": b1, "pinned": true}))] {
        rig.execute(id, &p).unwrap();
    }
    assert_eq!((b1, b2), (100, 102), "the hostile params name these bones");
    rig.set_frame(4);
    // Raw drawings and an object on two layers, selected on both through a marquee's cut that
    // is still provisional (ADR 0041, 0044).
    let mut layered = Session::default();
    layered.style.object_drawing = false;
    for (id, p) in [
        ("draw.rect", json!({"x": 0, "y": 0, "w": 60, "h": 60})),
        ("layer.new", json!({})),
        ("draw.rect", json!({"x": 30, "y": 30, "w": 60, "h": 60, "fill": "#ff0000"})),
        ("draw.rect", json!({"x": 200, "y": 0, "w": 20, "h": 20, "object": true})),
        ("raw.select_rect", json!({"x0": 20, "y0": 20, "x1": 250, "y1": 50})),
    ] {
        layered.execute(id, &p).unwrap();
    }
    assert_eq!(layered.selected_layers().len(), 2, "the fuzz starts with a selection on two layers");
    // Raw drawings inside a symbol open for editing in place (ADR 0045), with a point added.
    let mut in_symbol = Session::default();
    in_symbol.style.object_drawing = false;
    for (id, p) in [
        ("draw.rect", json!({"x": 0, "y": 0, "w": 60, "h": 60})),
        ("modify.convert_to_symbol", json!({"name": "Inner"})),
        ("element.translate", json!({"dx": 30, "dy": 15})),
        ("symbol.edit", json!({})),
        ("draw.rect", json!({"x": 30, "y": 30, "w": 60, "h": 60, "fill": "#ff0000"})),
        ("raw.add_point", json!({"x": 45, "y": 90})),
    ] {
        in_symbol.execute(id, &p).unwrap();
    }
    assert_eq!(in_symbol.editing.len(), 1, "the fuzz starts inside a symbol");
    // Raw drawings on several keyframes, artwork copied from two of them (ADR 0051).
    let mut keyed = Session::default();
    keyed.style.object_drawing = false;
    for (id, p) in [
        ("draw.rect", json!({"x": 0, "y": 0, "w": 60, "h": 60})),
        ("draw.rect", json!({"x": 100, "y": 0, "w": 60, "h": 60, "fill": "#ff0000"})),
        ("timeline.goto", json!({"frame": 4})),
        ("timeline.insert_keyframe", json!({})),
        ("timeline.goto", json!({"frame": 8})),
        ("timeline.insert_keyframe", json!({})),
        ("frames.copy", json!({"first": 0, "last": 8})),
        ("timeline.goto", json!({"frame": 4})),
    ] {
        keyed.execute(id, &p).unwrap();
    }
    // Library folders, nested, with items in them (ADR 0050); the fuzz moves, renames and
    // deletes them, folders into themselves, names that clash or hold a slash.
    let mut library = Session::new(sample::demo());
    let outer = library.execute("library.new_folder", &json!({"name": "Characters"})).unwrap()["folder"].as_u64().unwrap();
    let inner = library.execute("library.new_folder", &json!({"name": "Hero", "parent": outer})).unwrap()["folder"].as_u64().unwrap();
    let ball = library.doc.library.first().map(|x| x.id.0).unwrap();
    library.execute("library.move", &json!({"items": [{"symbol": ball}], "to": inner})).unwrap();
    library.execute("library.duplicate", &json!({"symbol": ball})).unwrap();
    // ...and a bitmap placed on the stage, for Swap Bitmap (ADR 0053).
    let bm = anicraft_doc::BitmapId(library.doc.alloc_id());
    let png = anicraft_render::encode_png(3, 2, &[0, 0, 255, 255].repeat(6)).unwrap();
    library.doc.bitmaps.push(Arc::new(anicraft_doc::Bitmap::from_png(bm, "b.png", png).unwrap()));
    library.execute("library.place", &json!({"bitmap": bm.0})).unwrap();
    hostile.push(json!({"bitmap": bm.0, "items": [{"bitmap": bm.0}, {"symbol": ball}], "x": 3, "y": 4}));
    hostile.extend([
        json!({"items": [{"folder": outer}], "to": inner, "folder": outer, "name": "Hero", "parent": inner, "confirm": true}),
        json!({"items": [{"folder": inner}, {"symbol": ball}, {"folder": inner}], "to": null, "folder": inner, "name": "a/b", "symbol": ball}),
        json!({"items": [{"symbol": ball, "folder": outer}, "x", {"sound": u64::MAX}], "to": u64::MAX, "parent": "x", "name": " "}),
        json!({"items": (0..200).map(|i| json!({"folder": i})).collect::<Vec<_>>(), "to": outer, "symbol": ball, "name": "x".repeat(300), "on": "x", "bitmap": 1}),
        json!({"items": [{"symbol": ball}], "to": outer, "symbol": ball, "name": "Ball copy", "confirm": true}),
        // Drops from the Library and Swap Bitmap (ADR 0053): mixed lists, two sounds, far points.
        json!({"items": [{"symbol": ball}, {"sound": SOUND}, {"bitmap": u64::MAX}], "x": 1e300, "y": -1e300}),
        json!({"items": [{"sound": SOUND}, {"sound": SOUND}, {"symbol": ball}], "x": 999_999, "y": -999_999, "bitmap": 0}),
        json!({"items": [{"symbol": ball}, {"symbol": ball}, {"folder": inner}, null], "x": "a"}),
        json!({"items": (0..1500).map(|_| json!({"symbol": ball})).collect::<Vec<_>>(), "x": 0, "y": 0}),
        json!({"sound": SOUND, "symbol": ball, "x": 10, "y": 10}),
        // Find and Replace and the Movie Explorer (ADR 0058): patterns that are slow in
        // backtracking engines or too large, replacements that would nest a symbol in itself,
        // stale hits, junk paths, rows and pages.
        json!({"kind": "text", "text": "(a+)+$", "regex": true, "with": "$1$1$1${x}", "in": ["text", "labels", "layers", "instances", "scripts"], "scope": "document"}),
        json!({"kind": "text", "text": "a{1000}{1000}", "regex": true, "with": ""}),
        json!({"kind": "text", "text": "", "with": "x"}),
        json!({"kind": "text", "text": "[a-z]", "regex": true, "with": "", "whole_word": false, "match_case": true, "confirm": true}),
        json!({"kind": "text", "text": "L", "with": "", "at": {"scene": 0, "layer": 1, "field": "layer_name", "start": u64::MAX}}),
        json!({"kind": "text", "text": "e", "with": "x".repeat(1000), "at": {"symbol": ball, "layer": u64::MAX, "field": "text", "frame": -1}}),
        json!({"kind": "color", "color": "#ff0000", "with": "#00ff00", "in": ["fills", "strokes", "text", "nope"]}),
        json!({"kind": "color", "color": "#3d8bfd", "with": "#000000", "scope": "document"}),
        json!({"kind": "font", "font": "Inter", "with": "No Such Font"}),
        json!({"kind": "symbol", "symbol": ball, "with": ball, "scope": "document"}),
        json!({"kind": "sound", "sound": SOUND, "with": SOUND}),
        json!({"kind": "bitmap", "bitmap": bm.0, "with": bm.0, "scope": "document"}),
        json!({"kind": "nope", "layer": u64::MAX, "path": [{"layer": 1, "frame": u32::MAX, "element": u64::MAX}], "symbol": ball, "scene": 0}),
        json!({"path": (0..100).map(|i| json!({"layer": i, "frame": i, "element": i})).collect::<Vec<_>>(), "layer": 1}),
        json!({"all": true, "filter": "a", "show": {"frames": false, "symbols": "x"}, "expanded": ["s0", 5], "offset": u64::MAX, "limit": u64::MAX}),
        json!({"all": true, "scope": "scene", "definitions": false, "show": {"frames": false}, "limit": 0}),
    ]);
    // A button open for editing, its hit area drawn over its other states (ADR 0057).
    let mut button = Session::new(sample::interactive());
    if let Some(id) = button.doc.library.iter().find(|x| x.kind == anicraft_doc::SymbolKind::Button).map(|x| x.id) {
        button.execute("symbol.edit", &json!({"symbol": id.0})).unwrap();
    }
    for start in [Session::default(), Session::new(sample::demo()), Session::new(with_sound), on_path, raw, rig, layered, in_symbol, keyed, library, button] {
        let mut s = start;
        for spec in commands().iter().filter(|c| !skip(c.id)) {
            for p in &hostile {
                let _ = s.execute(spec.id, p);
            }
            // and with a selection present
            let _ = s.execute("edit.select_all", &json!({}));
            for p in &hostile {
                let _ = s.execute(spec.id, p);
            }
        }
        // The document must still evaluate and render, and so must what the stage shows.
        for f in [0, 1, 5, 47, 1000] {
            let _ = anicraft_render::render_frame(&s.doc, f, 0.1);
        }
        s.hit_area = Some((anicraft_doc::Rgba::WHITE, 0.4));
        let pic = s.stage_picture();
        // ...and every button's hit area draws, under a pointer that goes over and presses.
        let mut ns = anicraft_doc::player::NoScripts;
        let st = anicraft_doc::player::start(&s.doc, &mut ns).state;
        let p = anicraft_geom::Vec2::new(640.0, 400.0);
        let st = anicraft_doc::player::input(&s.doc, &st, &[anicraft_doc::player::Event::Move(p), anicraft_doc::player::Event::Down(p), anicraft_doc::player::Event::Up(p)], &mut ns).state;
        let _ = anicraft_render::render(&anicraft_doc::player::hit_overlay(&s.doc, &st, anicraft_doc::Rgba::WHITE, 0.4), &anicraft_render::View { transform: anicraft_geom::Affine::scale(0.1, 0.1), width: 128, height: 72, background: None });
        let _ = anicraft_render::render_frame(&pic.doc, pic.frame, 0.1);
        // ...and its sounds lay out and mix.
        let voices = anicraft_doc::sound::voices(&s.doc, 8000);
        let mut out = vec![0.0; 2 * 4096];
        anicraft_doc::audio::mix(&voices, 8000, Default::default(), 0, &mut out);
        // ...and its Library tree stays whole: nothing a load would have to repair.
        let mut loaded = s.doc.clone();
        loaded.sanitize();
        assert_eq!(loaded.library_tree, s.doc.library_tree);
    }
}

#[test]
fn file_commands_reject_bad_params() {
    let mut s = Session::default();
    for id in ["file.open", "file.save", "file.export_png", "file.export_png_sequence"] {
        assert!(s.execute(id, &json!({"path": 5, "dir": []})).is_err(), "{id}");
    }
    assert!(s.execute("file.open", &json!({"path": "/definitely/not/here.anicraft"})).is_err());
    assert!(s.execute("file.new", &json!({"width": 0})).is_err());
    assert!(s.execute("no.such.command", &json!({})).is_err());
}

// ----- crash recovery (ADR 0015) -------------------------------------------------------------

/// A crash simulated end to end: a session autosaves unsaved work and dies without a clean
/// exit; the next session finds the copy, restores it as unsaved work with its original path,
/// and the copy can then be discarded (only with consent).
#[test]
fn crash_recovery_round_trip() {
    use crate::recovery::{Autosaver, Outcome, SessionLock};
    let dir = std::env::temp_dir().join(format!("anicraft-crash-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let original = dir.join("walk.anicraft");
    let d = dir.display().to_string();

    // Session 1: saves, keeps working, autosaves in the background, then "crashes".
    let mut s1 = Session::new(sample::demo());
    run(&mut s1, "file.save", json!({"path": original.display().to_string()}));
    run(&mut s1, "draw.rect", json!({"x": 10, "y": 10, "w": 50, "h": 40}));
    assert!(s1.is_dirty());
    let mut saver = Autosaver::start(SessionLock::acquire(&dir).unwrap()).unwrap();
    saver.write(s1.doc.clone(), s1.path.as_deref());
    let mut done = false;
    for _ in 0..500 {
        if saver.poll().iter().any(|o| matches!(o, Outcome::Written { .. })) {
            done = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(done, "autosave finished");
    // While it runs, nobody else is offered its copy.
    let mut s2 = Session::default();
    assert_eq!(run(&mut s2, "file.recovery_list", json!({"dir": d}))["documents"].as_array().unwrap().len(), 0);
    drop(saver); // the crash: no clean finish, the OS drops the lock

    // Session 2 (the next start) finds it.
    let list = run(&mut s2, "file.recovery_list", json!({"dir": d}));
    let docs = list["documents"].as_array().unwrap();
    assert_eq!(docs.len(), 1, "{list}");
    assert_eq!(docs[0]["name"], "walk.anicraft");
    assert_eq!(docs[0]["original_newer"], false);
    let id = docs[0]["session"].as_str().unwrap().to_string();

    // Restoring over unsaved work asks first (irreversible), like Open.
    run(&mut s2, "draw.rect", json!({"x": 0, "y": 0, "w": 5, "h": 5}));
    assert!(matches!(s2.execute("file.recover", &json!({"dir": d, "session": id})), Err(EngineError::NeedsConfirmation(_))));
    let v = run(&mut s2, "file.recover", json!({"dir": d, "session": id, "confirm": true}));
    assert_eq!(v["recovered"], json!(id));
    assert_eq!(s2.doc, s1.doc, "the work is back exactly");
    assert!(s2.is_dirty(), "restored work is unsaved");
    assert_eq!(s2.path.as_deref(), Some(original.as_path()), "Save goes back to the original");
    assert_eq!(s2.history_len(), 0);

    // Discarding needs explicit consent, whoever asks.
    assert!(matches!(s2.execute("file.discard_recovery", &json!({"dir": d, "session": id})), Err(EngineError::NeedsConfirmation(_))));
    assert!(s2.confirmation("file.discard_recovery", &json!({"dir": d, "session": id})).is_some_and(|c| c.irreversible));
    run(&mut s2, "file.discard_recovery", json!({"dir": d, "session": id, "confirm": true}));
    assert_eq!(run(&mut s2, "file.recovery_list", json!({"dir": d}))["documents"].as_array().unwrap().len(), 0);
    assert!(original.exists(), "the user's own file is untouched");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The original was saved after the autosave (a save just before the crash): restoring must
/// not point Save at it, or older work would replace newer work.
#[test]
fn recovery_never_replaces_a_newer_original() {
    let dir = std::env::temp_dir().join(format!("anicraft-crash-newer-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let original = dir.join("a.anicraft");
    let d = dir.display().to_string();
    let mut s = Session::new(sample::demo());
    s.path = Some(original.clone());
    let mut meta = crate::recovery::RecoveryMeta::describe("old", &s.doc, Some(&original));
    meta.saved_at = 1; // long ago
    crate::recovery::write(&dir, "old", &s.doc, &meta).unwrap();
    std::fs::write(&original, b"newer").unwrap();
    let mut s2 = Session::default();
    let v = run(&mut s2, "file.recover", json!({"dir": d, "session": "old"}));
    assert_eq!(v["original_newer"], true);
    assert!(s2.path.is_none(), "Save asks for a name instead");
    assert!(s2.is_dirty());
    assert_eq!(std::fs::read(&original).unwrap(), b"newer");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn recovery_commands_survive_hostile_params() {
    let dir = std::env::temp_dir().join(format!("anicraft-crash-hostile-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let d = dir.display().to_string();
    let mut s = Session::new(sample::demo());
    let hostile = [
        Value::Null,
        json!({}),
        json!({"dir": 5}),
        json!({"dir": ""}),
        json!({"dir": "   "}),
        json!({"dir": d, "session": "../../etc/passwd"}),
        json!({"dir": d, "session": "a\\..\\b"}),
        json!({"dir": d, "session": ""}),
        json!({"dir": d, "session": "x".repeat(10_000)}),
        json!({"dir": d, "session": 7, "confirm": "yes"}),
        json!({"dir": d, "session": "missing", "confirm": true}),
        json!({"dir": "/no/such/folder/at/all", "session": "abc", "confirm": true}),
    ];
    for id in ["file.recovery_list", "file.recover", "file.discard_recovery", "file.autosave"] {
        for p in &hostile {
            let _ = s.execute(id, p);
        }
    }
    // Nothing escaped the folder, and the document is intact.
    assert!(!dir.join("..").join("etc").exists() || std::path::Path::new("/etc").exists());
    // Headless autosave writes a copy that lists (no lock: headless sessions are not "live").
    let v = run(&mut s, "file.autosave", json!({"dir": d}));
    assert!(v["bytes"].as_u64().unwrap() > 0);
    let list = run(&mut s, "file.recovery_list", json!({"dir": d}));
    let ids: Vec<&str> = list["documents"].as_array().unwrap().iter().filter_map(|e| e["session"].as_str()).collect();
    assert!(ids.contains(&format!("headless-{}", std::process::id()).as_str()), "{ids:?}");
    assert!(ids.iter().all(|i| crate::recovery::valid_id(i)));
    let _ = std::fs::remove_dir_all(&dir);
}


// ----- paths: pen, pencil, point editing ---------------------------------------------------

fn shape_path(s: &Session, id: u64) -> anicraft_geom::Path {
    let el = s.visible_elements().into_iter().find(|e| e.id.0 == id).expect("element");
    match &el.kind {
        ElementKind::Shape(sh) => (*sh.path).clone(),
        _ => panic!("not a shape"),
    }
}

fn stage_anchor(s: &Session, id: u64, cmd: usize) -> Vec2 {
    let el = s.visible_elements().into_iter().find(|e| e.id.0 == id).expect("element");
    el.transform.apply(shape_path(s, id).point(cmd, 0).expect("anchor"))
}

#[test]
fn pen_paths_are_centred_and_closed_explicitly() {
    let mut s = Session::default();
    let id = run(&mut s, "draw.path", json!({"d": "M 100 100 L 200 100 C 220 150 180 200 150 200 Z"}))["element"].as_u64().unwrap();
    let el = s.visible_elements().into_iter().find(|e| e.id.0 == id).unwrap();
    let b = s.element_bounds(el.id);
    assert!((b.min.x - 99.0).abs() < 1.5 && (b.max.y - 201.0).abs() < 1.5, "{b:?}");
    let p = shape_path(&s, id);
    assert!(p.bounds().center().length() < 1e-3, "local origin is the centre");
    assert_eq!(p.editable_anchors().len(), 3, "the closing edge is a segment, the start is one anchor");
    // Open paths have no fill.
    let open = run(&mut s, "draw.path", json!({"d": "M 0 0 L 50 50 L 100 0"}))["element"].as_u64().unwrap();
    let el = s.visible_elements().into_iter().find(|e| e.id.0 == open).unwrap();
    assert!(matches!(&el.kind, ElementKind::Shape(sh) if sh.fill.is_none() && sh.stroke.is_some()));
    for bad in ["", "M 0 0", "M 0 0 L 0.1 0.1", "nonsense"] {
        assert!(s.execute("draw.path", &json!({"d": bad})).is_err(), "{bad}");
    }
}

#[test]
fn moving_an_anchor_brings_its_handles_and_is_one_undo_step_in_a_gesture() {
    let mut s = Session::default();
    let id = run(&mut s, "draw.path", json!({"d": "M 0 0 C 10 -20 30 -20 40 0 C 50 20 70 20 80 0"}))["element"].as_u64().unwrap();
    let before = shape_path(&s, id);
    let a0 = stage_anchor(&s, id, 1);
    let steps = s.history_len();
    s.begin_gesture("Move Points");
    for _ in 0..5 {
        run(&mut s, "path.move_points", json!({"id": id, "points": [[1, 0]], "dx": 2, "dy": 1}));
    }
    s.end_gesture();
    assert_eq!(s.history_len(), steps + 1, "one undo step");
    let a1 = stage_anchor(&s, id, 1);
    assert!((a1 - a0 - Vec2::new(10.0, 5.0)).length() < 1e-3);
    let after = shape_path(&s, id);
    for (cmd, slot) in [(1usize, 2u8), (2, 1)] {
        let d = after.point(cmd, slot).unwrap() - before.point(cmd, slot).unwrap();
        assert!((d - Vec2::new(10.0, 5.0)).length() < 1e-3, "handle ({cmd},{slot}) moved with the anchor");
    }
    assert_eq!(after.point(1, 1), before.point(1, 1), "the far handle stays");
    // A handle alone moves alone.
    run(&mut s, "path.move_points", json!({"id": id, "points": [[2, 2]], "dx": 0, "dy": 7}));
    assert_eq!(shape_path(&s, id).point(2, 0), after.point(2, 0));
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(shape_path(&s, id), before);
    assert!(s.execute("path.move_points", &json!({"id": id, "points": [[0, 2]], "dx": 1, "dy": 1})).is_err(), "MoveTo has no handles");
}

#[test]
fn moves_follow_the_element_transform() {
    let mut s = Session::default();
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 100, "h": 100}));
    let id = s.selection.ids().next().unwrap().0;
    run(&mut s, "element.set_transform", json!({"sx": 2, "rotation": 90}));
    let a0 = stage_anchor(&s, id, 0);
    run(&mut s, "path.move_points", json!({"id": id, "points": [[0, 0]], "dx": 10, "dy": 0}));
    assert!((stage_anchor(&s, id, 0) - a0 - Vec2::new(10.0, 0.0)).length() < 1e-3, "the point follows the pointer in stage space");
}

#[test]
fn add_delete_and_convert_points() {
    let mut s = Session::default();
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 100, "h": 100}));
    let id = s.selection.ids().next().unwrap().0;
    let area = |s: &Session| shape_path(s, id).flatten(&anicraft_geom::Affine::IDENTITY, 0.1)[0].points.len();
    let n0 = shape_path(&s, id).editable_anchors().len();
    // The closing edge (left side, x = 0) can take a point too.
    let r = run(&mut s, "path.add_point", json!({"id": id, "x": 0.5, "y": 50}));
    assert!(r["cmd"].is_u64());
    assert_eq!(shape_path(&s, id).editable_anchors().len(), n0 + 1);
    assert!(s.execute("path.add_point", &json!({"id": id, "x": 50, "y": 50})).is_err(), "nowhere near an edge");
    let cmd = r["cmd"].as_u64().unwrap();
    run(&mut s, "path.convert_point", json!({"id": id, "cmd": cmd, "smooth": true}));
    run(&mut s, "path.move_points", json!({"id": id, "points": [[cmd, 0]], "dx": -30, "dy": 0}));
    assert!(area(&s) > 8, "the edge is now a curve");
    run(&mut s, "path.convert_point", json!({"id": id, "cmd": cmd, "smooth": false}));
    run(&mut s, "path.delete_points", json!({"id": id, "cmds": [cmd]}));
    assert_eq!(shape_path(&s, id).editable_anchors().len(), n0);
    // Deleting all but two works; the last two stay.
    let all: Vec<u64> = shape_path(&s, id).editable_anchors().iter().map(|(i, _)| *i as u64).collect();
    let r = run(&mut s, "path.delete_points", json!({"id": id, "cmds": all}));
    assert_eq!(r["deleted"].as_u64(), Some(n0 as u64 - 2));
    assert!(s.execute("path.delete_points", &json!({"id": id, "cmds": [0]})).is_err());
    // Symbols are not point-editable.
    run(&mut s, "modify.convert_to_symbol", json!({}));
    let inst = s.selection.ids().next().unwrap().0;
    assert!(s.execute("path.add_point", &json!({"id": inst, "x": 0, "y": 0})).is_err());
}

#[test]
fn pencil_modes() {
    let mut s = Session::default();
    let wave: Vec<[f32; 2]> = (0..=200).map(|i| [100.0 + i as f32, 100.0 + (i as f32 * 0.05).sin() * 30.0]).collect();
    let count = |s: &mut Session, mode: &str| {
        let id = run(s, "draw.freehand", json!({"points": wave, "mode": mode, "tolerance": 1.0}))["element"].as_u64().unwrap();
        let el = s.visible_elements().into_iter().find(|e| e.id.0 == id).unwrap();
        let ElementKind::Shape(sh) = &el.kind else { panic!() };
        assert!(sh.fill.is_none() && sh.stroke.is_some(), "a pencil stroke is a line");
        sh.path.cmds.len()
    };
    let smooth = count(&mut s, "smooth");
    let ink = count(&mut s, "ink");
    let straight = count(&mut s, "straighten");
    assert!(smooth < 16, "smooth: {smooth}");
    assert!(straight < ink, "straighten {straight} < ink {ink}");
    assert!(s.execute("draw.freehand", &json!({"points": wave, "mode": "felt"})).is_err());
    assert!(s.execute("draw.freehand", &json!({"points": [[0, 0], [0, 0]]})).is_err());
}

// ----- timeline frame ranges ---------------------------------------------------------------

fn starts(s: &Session, layer: anicraft_doc::LayerId) -> Vec<(u32, u32)> {
    s.doc.timeline(s.scene).layer(layer).unwrap().keyframes.iter().map(|k| (k.start, k.duration)).collect()
}

#[test]
fn range_insert_and_remove_frames_across_layers() {
    let mut s = Session::default();
    let a = s.layer;
    run(&mut s, "timeline.insert_frame", json!({"frame": 0, "count": 9}));
    let b = anicraft_doc::LayerId(run(&mut s, "layer.new", json!({}))["layer"].as_u64().unwrap());
    assert_eq!(starts(&s, b), vec![(0, 10)], "a new layer spans the timeline");
    run(&mut s, "timeline.insert_keyframe", json!({"layers": [a.0, b.0], "frame": 4}));
    assert_eq!(starts(&s, a), vec![(0, 4), (4, 6)]);
    assert_eq!(starts(&s, b), vec![(0, 4), (4, 6)]);
    // F5 over frames 2�4 of both layers: three frames each, keys after move right.
    run(&mut s, "timeline.insert_frame", json!({"layers": [a.0, b.0], "frame": 1, "count": 3}));
    assert_eq!(starts(&s, a), vec![(0, 7), (7, 6)]);
    assert_eq!(starts(&s, b), starts(&s, a));
    // Shift+F5 takes them out again; one undo step restores both layers.
    let before = starts(&s, a);
    run(&mut s, "timeline.remove_frame", json!({"layers": [a.0, b.0], "frame": 1, "count": 3}));
    assert_eq!(starts(&s, a), vec![(0, 4), (4, 6)]);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(starts(&s, a), before);
    // Removing more frames than a layer has stops at its last frame instead of failing.
    let r = run(&mut s, "timeline.remove_frame", json!({"layers": [a.0], "frame": 0, "count": 500}));
    assert!(r["removed"].as_u64().unwrap() >= 12);
    assert!(!starts(&s, a).is_empty());
    assert!(s.execute("timeline.insert_frame", &json!({"layers": [], "frame": 0})).is_err());
    assert!(s.execute("timeline.insert_frame", &json!({"layers": [u64::MAX], "frame": 0})).is_err());
    assert!(s.execute("timeline.insert_frame", &json!({"count": 0})).is_err());
}

#[test]
fn range_clear_removes_every_key_in_the_span() {
    let mut s = Session::default();
    let a = s.layer;
    run(&mut s, "timeline.insert_frame", json!({"frame": 0, "count": 19}));
    for f in [5, 8, 12] {
        run(&mut s, "timeline.insert_keyframe", json!({"frame": f}));
    }
    run(&mut s, "timeline.clear_keyframe", json!({"frame": 4, "count": 6}));
    assert_eq!(starts(&s, a), vec![(0, 12), (12, 8)], "keys at 5 and 8 merged into the span before");
    assert!(s.execute("timeline.clear_keyframe", &json!({"frame": 1, "count": 3})).is_err(), "nothing to clear");
}

#[test]
fn moving_keyframes_by_drag() {
    let mut s = Session::default();
    let a = s.layer;
    run(&mut s, "timeline.insert_frame", json!({"frame": 0, "count": 19}));
    run(&mut s, "draw.rect", json!({"x": 10, "y": 10, "w": 20, "h": 20}));
    run(&mut s, "timeline.insert_keyframe", json!({"frame": 5}));
    run(&mut s, "timeline.insert_keyframe", json!({"frame": 10}));
    // Right: the span before grows into the frames left behind; the layer keeps its length.
    run(&mut s, "timeline.move_keyframes", json!({"from": 5, "by": 3}));
    assert_eq!(starts(&s, a), vec![(0, 8), (8, 2), (10, 10)]);
    // Onto another key: that key is replaced.
    run(&mut s, "timeline.move_keyframes", json!({"from": 8, "by": 2}));
    assert_eq!(starts(&s, a), vec![(0, 10), (10, 10)]);
    // A range moves together, left; nothing is lost at the end.
    run(&mut s, "timeline.insert_keyframe", json!({"frame": 14}));
    run(&mut s, "timeline.move_keyframes", json!({"from": 10, "to": 14, "by": -4}));
    assert_eq!(starts(&s, a), vec![(0, 6), (6, 4), (10, 10)]);
    assert_eq!(s.doc.frame_count(), 20);
    // Content travels with its key.
    s.set_frame(6);
    assert_eq!(s.visible_elements().len(), 1);
    assert!(s.execute("timeline.move_keyframes", &json!({"from": 0, "by": -1})).is_err(), "not before frame 1");
    assert!(s.execute("timeline.move_keyframes", &json!({"from": 5, "to": 2, "by": 1})).is_err());
    assert_eq!(run(&mut s, "timeline.move_keyframes", json!({"from": 1, "to": 3, "by": 2}))["moved"], 0);
}

#[test]
fn range_deletions_ask_when_artwork_would_go() {
    let mut s = Session::default();
    run(&mut s, "timeline.insert_frame", json!({"frame": 0, "count": 9}));
    run(&mut s, "timeline.insert_blank_keyframe", json!({"frame": 3}));
    s.set_frame(3);
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 10, "h": 10}));
    run(&mut s, "timeline.insert_blank_keyframe", json!({"frame": 5}));
    // Frames 4–5 hold the key 3..5 entirely: removing them deletes its artwork.
    let c = s.confirmation("timeline.remove_frame", &json!({"frame": 3, "count": 2})).expect("asks");
    assert!(!c.irreversible && c.action == "Remove Frames", "{c:?}");
    assert!(s.confirmation("timeline.remove_frame", &json!({"frame": 3, "count": 1})).is_none(), "the key keeps a frame");
    assert!(s.confirmation("timeline.clear_keyframe", &json!({"frame": 0, "count": 10})).is_some());
    assert!(s.confirmation("timeline.clear_keyframe", &json!({"frame": 5, "count": 5})).is_none(), "only empty keys");
}

// ----- frame clipboard ---------------------------------------------------------------------

#[test]
fn copy_and_paste_frames_inserts_and_cuts_at_the_selection_edges() {
    let mut s = Session::default();
    let a = s.layer;
    run(&mut s, "timeline.insert_frame", json!({"frame": 0, "count": 9}));
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 10, "h": 10}));
    run(&mut s, "timeline.insert_keyframe", json!({"frame": 9}));
    run(&mut s, "element.translate", json!({"dx": 90, "dy": 0}));
    s.set_frame(0);
    run(&mut s, "timeline.create_tween", json!({}));
    // Frames 4–6, mid-tween: the copy starts with the tweened state at frame 4.
    run(&mut s, "timeline.copy_frames", json!({"frame": 3, "count": 3}));
    let clip = s.frame_clipboard.clone();
    assert_eq!(clip.count, 3);
    assert_eq!(clip.layers[0].keys.len(), 1);
    assert_eq!((clip.layers[0].keys[0].start, clip.layers[0].keys[0].duration), (0, 3));
    let x_at_3 = { s.set_frame(3); s.visible_elements()[0].transform.translation().x };
    let copied_x = clip.layers[0].keys[0].elements[0].transform.translation().x;
    assert!((copied_x - x_at_3).abs() < 1e-3, "{copied_x} vs {x_at_3}");
    // Paste at frame 1: inserts three frames, later keys move right, ids are fresh.
    let before = s.doc.frame_count();
    let old_ids: Vec<u64> = s.doc.timeline(s.scene).layer(a).unwrap().keyframes.iter().flat_map(|k| k.elements.iter().map(|e| e.id.0)).collect();
    run(&mut s, "timeline.paste_frames", json!({"frame": 0}));
    assert_eq!(s.doc.frame_count(), before + 3);
    let l = s.doc.timeline(s.scene).layer(a).unwrap();
    assert_eq!(l.keyframes[0].start, 0);
    assert!(l.keyframe_starting_at(12).is_some(), "the key at frame 10 moved to frame 13");
    assert!(!old_ids.contains(&l.keyframes[0].elements[0].id.0), "fresh ids");
    // One undo step.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc.frame_count(), before);
    // Cut removes what it copied.
    run(&mut s, "timeline.cut_frames", json!({"frame": 0, "count": 2}));
    assert_eq!(s.doc.frame_count(), before - 2);
    assert!(s.execute("timeline.copy_frames", &json!({"frame": 500})).is_err(), "nothing there");
}

#[test]
fn pasting_several_layers_fills_the_layers_below_and_creates_missing_ones() {
    let mut s = Session::new(anicraft_doc::sample::demo());
    let layers: Vec<u64> = s.doc.timeline(s.scene).layers.iter().map(|l| l.id.0).collect();
    let n = layers.len();
    run(&mut s, "timeline.copy_frames", json!({"layers": [layers[1], layers[0]], "frame": 0, "count": 5}));
    assert_eq!(s.frame_clipboard.layers.len(), 2);
    assert_eq!(s.frame_clipboard.layers[0].name, s.doc.timeline(s.scene).layers[0].name, "kept top to bottom");
    // Onto the bottom layer: the second copied layer needs a new layer under it.
    run(&mut s, "timeline.paste_frames", json!({"layer": layers[n - 1], "frame": 0}));
    assert_eq!(s.doc.timeline(s.scene).layers.len(), n + 1);
    let fresh = s.doc.timeline(s.scene).layers.last().unwrap();
    assert_eq!(fresh.keyframes.first().map(|k| k.start), Some(0));
    assert!(s.doc.frame_count() >= 5);
    // Every element id in the document stays unique per layer.
    for l in &s.doc.timeline(s.scene).layers {
        let mut ids: Vec<u64> = l.keyframes.iter().filter(|k| k.start == 0).flat_map(|k| k.elements.iter().map(|e| e.id.0)).collect();
        let before = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), before);
    }
    let empty = Session::default().execute("timeline.paste_frames", &json!({}));
    assert!(empty.is_err(), "nothing copied");
}

// ----- paint bucket / ink bottle -----------------------------------------------------------

fn shapes_on_layer(s: &Session) -> Vec<(u64, Option<anicraft_doc::Paint>, Option<anicraft_doc::Stroke>)> {
    s.visible_elements()
        .iter()
        .filter_map(|e| match &e.kind {
            ElementKind::Shape(sh) => Some((e.id.0, sh.fill.clone(), sh.stroke.clone())),
            _ => None,
        })
        .collect()
}

#[test]
fn bucket_recolours_a_fill_and_fills_a_closed_outline_underneath() {
    let mut s = Session::default();
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 50, "h": 50, "fill": "#ff0000"}));
    let r = run(&mut s, "paint.bucket", json!({"x": 25, "y": 25, "color": "#00ff00"}));
    assert!(r["recolored"].is_u64());
    assert!(matches!(shapes_on_layer(&s)[0].1, Some(anicraft_doc::Paint::Solid(c)) if c == anicraft_doc::Rgba::hex(0x00ff00)));
    // An outline with no fill (a closed pen path): the bucket adds a fill below it.
    run(&mut s, "draw.path", json!({"d": "M 100 100 L 200 100 L 200 180 L 100 180 Z", "fill": null, "stroke": "#000000", "stroke_width": 2}));
    let r = run(&mut s, "paint.bucket", json!({"x": 150, "y": 140, "color": "#0000ff"}));
    let id = r["element"].as_u64().expect("a new fill");
    let first = s.visible_elements()[0].clone();
    assert_eq!(first.id.0, id, "inserted at the bottom of the layer");
    let b = s.element_bounds(first.id);
    assert!((b.min.x - 100.0).abs() < 2.5 && (b.max.x - 200.0).abs() < 2.5 && (b.min.y - 100.0).abs() < 2.5 && (b.max.y - 180.0).abs() < 2.5, "fills the inside, tucked under the line: {b:?}");
    let ElementKind::Shape(sh) = &first.kind else { panic!() };
    assert!(sh.stroke.is_none());
    // One undo step removes it.
    run(&mut s, "edit.undo", json!({}));
    assert!(s.visible_elements().iter().all(|e| e.id.0 != id));
}

#[test]
fn bucket_closes_small_gaps_only_when_asked() {
    let mut s = Session::default();
    // Four separate lines forming a square with a 4 px opening in the top edge.
    for (x0, y0, x1, y1) in [(0, 0, 48, 0), (52, 0, 100, 0), (100, 0, 100, 100), (100, 100, 0, 100), (0, 100, 0, 0)] {
        run(&mut s, "draw.line", json!({"x0": x0 + 100, "y0": y0 + 100, "x1": x1 + 100, "y1": y1 + 100, "stroke_width": 2}));
    }
    let n = s.visible_elements().len();
    let e = s.execute("paint.bucket", &json!({"x": 150, "y": 150}));
    assert!(e.is_err() && e.unwrap_err().to_string().contains("isn't closed"), "open without a gap size");
    assert_eq!(s.visible_elements().len(), n, "nothing added");
    run(&mut s, "paint.bucket", json!({"x": 150, "y": 150, "gap": 8}));
    assert_eq!(s.visible_elements().len(), n + 1, "a larger gap size closes it");
    // Outside everything: not enclosed.
    assert!(s.execute("paint.bucket", &json!({"x": 10, "y": 10})).is_err());
}

#[test]
fn ink_bottle_strokes_the_clicked_shape() {
    let mut s = Session::default();
    run(&mut s, "draw.ellipse", json!({"x": 0, "y": 0, "w": 40, "h": 40, "stroke": null}));
    assert!(shapes_on_layer(&s)[0].2.is_none());
    run(&mut s, "paint.ink", json!({"x": 20, "y": 20, "color": "#123456", "width": 4}));
    let st = shapes_on_layer(&s)[0].2.clone().expect("stroked");
    assert_eq!((st.color, st.width), (anicraft_doc::Rgba::hex(0x123456), 4.0));
    assert!(s.execute("paint.ink", &json!({"x": 300, "y": 300})).is_err(), "nothing there");
    // Symbols keep their colours.
    run(&mut s, "edit.select_all", json!({}));
    run(&mut s, "modify.convert_to_symbol", json!({}));
    assert!(s.execute("paint.ink", &json!({"x": 20, "y": 20})).is_err());
}

#[test]
fn dragging_keyframes_onto_another_layer() {
    let mut s = Session::default();
    let a = s.layer;
    run(&mut s, "timeline.insert_frame", json!({"frame": 0, "count": 19}));
    s.set_frame(5);
    run(&mut s, "timeline.insert_blank_keyframe", json!({"frame": 5}));
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 10, "h": 10}));
    run(&mut s, "timeline.insert_blank_keyframe", json!({"frame": 10}));
    let b = anicraft_doc::LayerId(run(&mut s, "layer.new", json!({}))["layer"].as_u64().unwrap());
    // New layers go above the active one: b is row 0, a is row 1. Drag a's frames 6–10 up a row.
    let content = |s: &Session, l| s.doc.timeline(s.scene).layer(l).unwrap().keyframe_at(5).map_or(0, |k| k.elements.len());
    assert_eq!((content(&s, a), content(&s, b)), (1, 0));
    run(&mut s, "timeline.move_keyframes", json!({"layers": [a.0], "from": 5, "to": 9, "by": 0, "layer_by": -1}));
    assert_eq!((content(&s, a), content(&s, b)), (0, 1), "the artwork moved up");
    assert_eq!(s.doc.timeline(s.scene).layer(a).unwrap().end_frame(), 20, "the source keeps its frames, now empty");
    assert!(s.doc.timeline(s.scene).layer(b).unwrap().keyframe_at(12).is_some(), "the target keeps the frames after the landing range");
    // One undo step puts it all back.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!((content(&s, a), content(&s, b)), (1, 0));
    // With a frame shift too, and no layer there: an error, nothing changes.
    run(&mut s, "timeline.move_keyframes", json!({"layers": [a.0], "from": 5, "to": 9, "by": 3, "layer_by": -1}));
    assert!(s.doc.timeline(s.scene).layer(b).unwrap().keyframe_starting_at(8).is_some_and(|i| !s.doc.timeline(s.scene).layer(b).unwrap().keyframes[i].elements.is_empty()));
    assert!(s.execute("timeline.move_keyframes", &json!({"layers": [b.0], "from": 0, "to": 30, "by": 0, "layer_by": -1})).is_err());
    assert!(s.execute("timeline.move_keyframes", &json!({"layers": [a.0], "from": 0, "by": 0, "layer_by": "x"})).is_err());
}

#[test]
fn opening_an_xfl_folder_imports_it_untitled() {
    let dir = std::env::temp_dir().join(format!("anicraft-open-xfl-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("DOMDocument.xml"),
        r##"<DOMDocument width="320" height="240" frameRate="12"><timelines><DOMTimeline name="Scene 1"><layers><DOMLayer name="Art"><frames><DOMFrame index="0" duration="6"><elements>
        <DOMShape><fills><FillStyle index="1"><SolidColor color="#FF8800"/></FillStyle></fills><edges><Edge fillStyle1="1" edges="!0 0|2000 0|2000 2000|0 2000|0 0"/></edges></DOMShape>
        <DOMDynamicText name="title" width="200"><matrix><Matrix tx="20" ty="30"/></matrix><textRuns><DOMTextRun><characters>Hello</characters><textAttrs><DOMTextAttrs face="NoSuchFamilyAtAll-Bold" size="20" fillColor="#0000FF"/></textAttrs></DOMTextRun></textRuns></DOMDynamicText>
        </elements></DOMFrame></frames></DOMLayer></layers></DOMTimeline></timelines></DOMDocument>"##,
    )
    .unwrap();
    let mut s = Session::default();
    let v = run(&mut s, "file.open", json!({"path": dir.display().to_string()}));
    assert_eq!((s.doc.stage.width, s.doc.stage.fps), (320, 12.0));
    assert_eq!(s.doc.timeline(s.scene).layers[0].name, "Art");
    assert_eq!(s.doc.frame_count(), 6);
    assert!(v["report"].as_str().unwrap().contains("Runs in Test Movie: 1 dynamic text fields"), "{v}");
    assert!(s.path.is_none(), "imported documents are untitled: Save never writes over the original");
    assert!(s.execute("file.save", &json!({})).is_err());
    // It renders.
    let dl = s.display_list();
    assert!(dl.items.iter().any(|i| matches!(i.fill, Some(anicraft_doc::Paint::Solid(c)) if c == anicraft_doc::Rgba::hex(0xff8800))));
    // Text comes across as an editable text block when this computer has fonts (ADR 0014),
    // with the missing family named.
    if !anicraft_text::Fonts::get().families().is_empty() {
        assert!(v["report"].as_str().unwrap().contains("Fonts not on this computer"), "{v}");
        assert_eq!(v["missing_fonts"], json!(["No Such Family At All"]));
        let text = s.doc.timeline(s.scene).layers[0].keyframes[0].elements.iter().find(|e| matches!(e.kind, ElementKind::Text(_))).cloned().expect("a text element");
        assert_eq!(text.name, "title");
        assert!(dl.items.iter().any(|i| matches!(i.fill, Some(anicraft_doc::Paint::Solid(c)) if c == anicraft_doc::Rgba::hex(0x0000ff))), "the text renders");
        s.select_only([text.id]);
        run(&mut s, "text.edit", json!({"text": "Hello again"}));
        // Saved and opened again, it is the same text.
        let file = dir.join("imported.anicraft");
        run(&mut s, "file.save", json!({"path": file.display().to_string()}));
        let mut back = Session::default();
        run(&mut back, "file.open", json!({"path": file.display().to_string()}));
        let ElementKind::Text(t) = &back.doc.timeline(0).layers[0].keyframes[0].elements.iter().find(|e| e.id == text.id).unwrap().kind else { panic!("text") };
        assert_eq!((t.text.as_str(), t.size, t.bold), ("Hello again", 20.0, true));
        assert_eq!(t.kind(), anicraft_doc::TextKind::Dynamic, "a dynamic field stays one (ADR 0066)");
        assert_eq!((t.box_width, t.field.as_ref().map(|f| f.lines)), (200.0, Some(anicraft_doc::LineType::Single)));
    }
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn animate_default_keys_map_to_the_same_commands() {
    // Muscle memory: Animate's default keys (as its documentation lists them) do the same here.
    let expected = [
        ("Ctrl+Z", "edit.undo"),
        ("Ctrl+Y", "edit.redo"),
        ("Ctrl+X", "edit.cut"),
        ("Ctrl+C", "edit.copy"),
        ("Ctrl+V", "edit.paste"),
        ("Ctrl+Shift+V", "edit.paste_in_place"),
        ("Ctrl+D", "edit.duplicate"),
        ("Ctrl+A", "edit.select_all"),
        ("Ctrl+Shift+A", "edit.deselect"),
        ("Delete", "edit.delete"),
        ("Backspace", "edit.delete"),
        ("F5", "timeline.insert_frame"),
        ("Shift+F5", "timeline.remove_frame"),
        ("F6", "timeline.insert_keyframe"),
        ("Shift+F6", "timeline.clear_keyframe"),
        ("F7", "timeline.insert_blank_keyframe"),
        ("Ctrl+Alt+X", "timeline.cut_frames"),
        ("Ctrl+Alt+C", "timeline.copy_frames"),
        ("Ctrl+Alt+V", "timeline.paste_frames"),
        (".", "timeline.next_frame"),
        (",", "timeline.prev_frame"),
        ("Shift+,", "timeline.first_frame"),
        ("Shift+.", "timeline.last_frame"),
        ("F8", "modify.convert_to_symbol"),
        ("Ctrl+Enter", "control.test_movie"),
        ("Ctrl+B", "modify.break_apart"),
        ("Ctrl+Shift+Z", "modify.remove_transform"),
        ("Ctrl+Shift+9", "modify.rotate_cw"),
        ("Ctrl+Shift+7", "modify.rotate_ccw"),
        ("Ctrl+Shift+Up", "arrange.bring_to_front"),
        ("Ctrl+Up", "arrange.bring_forward"),
        ("Ctrl+Down", "arrange.send_backward"),
        ("Ctrl+Shift+Down", "arrange.send_to_back"),
        ("Ctrl+Alt+1", "align.left"),
        ("Ctrl+Alt+2", "align.hcenter"),
        ("Ctrl+Alt+3", "align.right"),
        ("Ctrl+Alt+4", "align.top"),
        ("Ctrl+Alt+5", "align.vcenter"),
        ("Ctrl+Alt+6", "align.bottom"),
        ("Ctrl+N", "file.new"),
        ("Ctrl+O", "file.open"),
        ("Ctrl+S", "file.save"),
        ("Ctrl+Shift+S", "file.save_as"),
    ];
    let cmds = commands();
    for (keys, id) in expected {
        let c = cmds.iter().find(|c| c.id == id).unwrap_or_else(|| panic!("{id} missing"));
        let alias = SHORTCUT_ALIASES.iter().any(|(k, i)| *k == keys && *i == id);
        assert!(c.shortcut == Some(keys) || alias, "{id}: {:?}, expected {keys}", c.shortcut);
    }
    // No chord does two things.
    let mut all: Vec<&str> = cmds.iter().filter_map(|c| c.shortcut).chain(SHORTCUT_ALIASES.iter().map(|(k, _)| *k)).collect();
    all.sort_unstable();
    let n = all.len();
    all.dedup();
    assert_eq!(all.len(), n, "a key chord is bound twice");
}

#[test]
fn bring_forward_send_backward_and_remove_transform() {
    let mut s = Session::default();
    let ids: Vec<u64> = (0..3).map(|i| run(&mut s, "draw.rect", json!({"x": i * 10, "y": 0, "w": 50, "h": 50}))["element"].as_u64().unwrap()).collect();
    let order = |s: &Session| s.visible_elements().iter().map(|e| e.id.0).collect::<Vec<_>>();
    run(&mut s, "element.select", json!({"ids": [ids[0]]}));
    run(&mut s, "arrange.bring_forward", json!({}));
    assert_eq!(order(&s), vec![ids[1], ids[0], ids[2]], "one step up");
    run(&mut s, "arrange.bring_forward", json!({}));
    run(&mut s, "arrange.bring_forward", json!({}));
    assert_eq!(order(&s), vec![ids[1], ids[2], ids[0]], "stops at the top");
    run(&mut s, "arrange.send_backward", json!({}));
    assert_eq!(order(&s), vec![ids[1], ids[0], ids[2]]);
    // Remove Transform keeps the position, drops scale and rotation.
    run(&mut s, "element.set_transform", json!({"sx": 2, "rotation": 30}));
    let before = s.visible_elements().iter().find(|e| e.id.0 == ids[0]).unwrap().transform.translation();
    run(&mut s, "modify.remove_transform", json!({}));
    let t = s.visible_elements().iter().find(|e| e.id.0 == ids[0]).unwrap().transform;
    assert_eq!(t.m[..4], [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(t.translation(), before);
}

#[test]
fn masked_frames_round_trip_through_svg_onto_mask_layers() {
    // A drawing with a mask: a background, artwork under an ellipse mask, a layer on top.
    let mut src = Session::default();
    run(&mut src, "draw.rect", json!({"x": 0, "y": 0, "w": 640, "h": 360, "fill": "#20304a"}));
    run(&mut src, "layer.new", json!({"name": "Art"}));
    run(&mut src, "draw.rect", json!({"x": 100, "y": 60, "w": 400, "h": 240, "fill": "#ff6b4a"}));
    run(&mut src, "draw.star", json!({"cx": 300, "cy": 180, "outer": 150, "inner": 60, "points": 5, "fill": "#ffd54a"}));
    run(&mut src, "layer.new", json!({"name": "Mask"}));
    run(&mut src, "draw.ellipse", json!({"x": 180, "y": 80, "w": 260, "h": 200}));
    run(&mut src, "layer.toggle_mask", json!({}));
    run(&mut src, "layer.new", json!({"name": "Top"}));
    run(&mut src, "draw.rect", json!({"x": 20, "y": 20, "w": 60, "h": 60, "fill": "#66bb6a"}));
    let dir = std::env::temp_dir().join(format!("anicraft-svg-mask-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let svg = dir.join("masked.svg");
    std::fs::write(&svg, crate::cmds_file::frame_svg(&src.doc, 0)).unwrap();

    // Import it at the origin of an empty document of the same size.
    let mut s = Session::default();
    let before = s.doc.timeline(s.scene).layers.len();
    let v = run(&mut s, "file.import", json!({"path": svg.display().to_string(), "x": 0, "y": 0}));
    assert_eq!((v["masks"].as_u64(), v["layers"].as_u64()), (Some(1), Some(5)), "{v}");
    let tl = s.doc.timeline(s.scene);
    use anicraft_doc::LayerKind;
    let kinds: Vec<(LayerKind, usize)> = tl.layers.iter().take(5).map(|l| (l.kind, tl.depth(tl.layers.iter().position(|x| x.id == l.id).unwrap()))).collect();
    assert_eq!(kinds, [(LayerKind::Folder, 0), (LayerKind::Normal, 1), (LayerKind::Mask, 1), (LayerKind::Normal, 2), (LayerKind::Normal, 1)], "folder › top art, mask › masked art, background");
    assert_eq!(tl.layers[0].name, "masked");
    assert_eq!(tl.layers.len(), before + 5);
    assert_eq!(s.selection.len(), 2, "the top layer's artwork (exported as a fill and a stroke) is selected");

    // The same picture comes back.
    let a = anicraft_render::render_frame(&src.doc, 0, 0.5).unwrap().to_rgba8();
    let b = anicraft_render::render_frame(&s.doc, 0, 0.5).unwrap().to_rgba8();
    let worst = a.iter().zip(&b).map(|(x, y)| (i32::from(*x) - i32::from(*y)).unsigned_abs()).max().unwrap();
    let mean = a.iter().zip(&b).map(|(x, y)| (i32::from(*x) - i32::from(*y)).unsigned_abs() as f64).sum::<f64>() / a.len() as f64;
    assert!(mean < 0.01 && worst <= 2, "mean {mean:.3}, worst {worst} of 255");
    // Exporting the import again gives the same masks.
    assert_eq!(crate::cmds_file::frame_svg(&s.doc, 0).matches("<clipPath").count(), 1);

    // One undo step removes the folder and all its layers.
    s.undo().unwrap();
    assert_eq!(s.doc.timeline(s.scene).layers.len(), before);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn imported_svg_text_is_editable_text() {
    if anicraft_text::Fonts::get().families().is_empty() {
        return;
    }
    let dir = std::env::temp_dir().join(format!("anicraft-svg-text-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let svg = dir.join("title.svg");
    std::fs::write(&svg, r##"<svg width="300" height="80"><text x="150" y="50" text-anchor="middle" font-family="Zz Not Installed" font-size="32" fill="#ff0000">Title</text><rect width="10" height="10"/></svg>"##).unwrap();
    let mut s = Session::default();
    let v = run(&mut s, "file.import", json!({"path": svg.display().to_string()}));
    assert_eq!((v["texts"].as_u64(), v["elements"].as_u64(), v["layers"].as_u64()), (Some(1), Some(2), Some(1)));
    assert_eq!(v["missing_fonts"], json!(["Zz Not Installed"]));
    assert!(v["report"].as_str().unwrap().contains("Fonts not on this computer"));
    let text = s.visible_elements().into_iter().find(|e| matches!(e.kind, anicraft_doc::ElementKind::Text(_))).unwrap();
    run(&mut s, "element.select", json!({"ids": [text.id.0]}));
    run(&mut s, "text.edit", json!({"text": "Longer title"}));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn mask_layers_reveal_hide_pick_and_move_as_a_group() {
    let mut s = Session::default();
    let art = s.layer;
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 200, "h": 200}));
    let mask = run(&mut s, "layer.new", json!({"name": "Mask"}))["layer"].as_u64().map(LayerId).unwrap();
    run(&mut s, "draw.rect", json!({"x": 50, "y": 50, "w": 20, "h": 20}));
    let r = run(&mut s, "layer.toggle_mask", json!({}));
    assert_eq!(r["type"], "mask");
    assert_eq!(s.doc.timeline(s.scene).layer(art).unwrap().parent, Some(mask), "the layer under it is masked");

    // Only the mask's area shows, and only it can be picked.
    let px = |s: &Session, x, y| anicraft_render::render_frame(&s.doc, 0, 1.0).unwrap().pixel(x, y).unwrap();
    let bg = px(&s, 500, 500);
    assert_eq!(px(&s, 10, 10), bg, "masked away");
    assert_ne!(px(&s, 60, 60), bg, "revealed");
    run(&mut s, "layer.toggle_lock", json!({"layer": mask.0}));
    assert_eq!(s.hit_test(Vec2::new(10.0, 10.0), 0.0), None);
    assert_eq!(s.hit_test(Vec2::new(60.0, 60.0), 0.0).map(|(l, _)| l), Some(art));

    // A layer added above a masked layer is masked too; the mask moves with its group.
    run(&mut s, "layer.select", json!({"layer": art.0}));
    let extra = run(&mut s, "layer.new", json!({}))["layer"].as_u64().map(LayerId).unwrap();
    assert_eq!(s.doc.timeline(s.scene).layer(extra).unwrap().parent, Some(mask));
    let top = run(&mut s, "layer.new", json!({"name": "Top"}))["layer"].as_u64().map(LayerId).unwrap();
    run(&mut s, "layer.move_up", json!({"layer": top.0}));
    let names = |s: &Session| s.doc.timeline(s.scene).layers.iter().map(|l| l.id).collect::<Vec<_>>();
    // `Top` was added inside the group too (above `extra`); moving it up leaves the group.
    assert_eq!(s.doc.timeline(s.scene).layer(top).unwrap().parent, None);
    assert_eq!(names(&s), vec![top, mask, extra, art]);
    run(&mut s, "layer.move_up", json!({"layer": mask.0}));
    assert_eq!(names(&s), vec![mask, extra, art, top], "the whole group moved above Top");
    run(&mut s, "layer.move_down", json!({"layer": mask.0}));
    assert_eq!(names(&s), vec![top, mask, extra, art]);
    // Moving the last masked layer up past its mask frees it; undo puts it back.
    run(&mut s, "layer.move_up", json!({"layer": extra.0}));
    assert_eq!(s.doc.timeline(s.scene).layer(extra).unwrap().parent, None);
    s.undo().unwrap();
    assert_eq!(s.doc.timeline(s.scene).layer(extra).unwrap().parent, Some(mask));

    // `masked` needs a mask above; unknown types are refused.
    assert!(s.execute("layer.set_type", &json!({"type": "masked", "layer": top.0})).is_err());
    assert!(s.execute("layer.set_type", &json!({"type": "wobbly"})).is_err());
    // Turning the mask off frees its layers; deleting a mask does the same.
    run(&mut s, "layer.toggle_mask", json!({"layer": mask.0}));
    assert!(s.doc.timeline(s.scene).layers.iter().all(|l| l.parent.is_none()));
    assert_ne!(px(&s, 10, 10), bg, "everything shows again");
    s.undo().unwrap();
    run(&mut s, "layer.delete", json!({"layer": mask.0, "confirm": true}));
    assert!(s.doc.timeline(s.scene).layers.iter().all(|l| l.parent.is_none()));
}
#[test]
fn create_shape_tween_morphs_and_keeps_the_easing() {
    let mut s = Session::default();
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 100, "h": 100}));
    run(&mut s, "timeline.goto", json!({"frame": 9}));
    run(&mut s, "timeline.insert_blank_keyframe", json!({}));
    run(&mut s, "draw.star", json!({"cx": 300, "cy": 50, "outer": 50, "inner": 20, "points": 5}));
    run(&mut s, "timeline.goto", json!({"frame": 0}));
    run(&mut s, "timeline.create_tween", json!({"easing": "ease-in"}));
    let v = run(&mut s, "timeline.create_shape_tween", json!({}));
    assert_eq!(v["kind"], "shape");
    let t = s.active_layer().unwrap().keyframes[0].tween.unwrap();
    assert_eq!(t.kind, anicraft_doc::TweenKind::Shape);
    assert_ne!(t.easing, anicraft_doc::Easing::Linear, "switching kinds keeps the easing");
    // Midway the shape has moved off the square.
    let item = &anicraft_doc::evaluate(&s.doc, 5).items[0];
    let mid = item.path.bounds().transformed(&item.transform);
    assert!(mid.max.x > 110.0, "{mid:?}");
    s.undo().unwrap();
    assert_eq!(s.active_layer().unwrap().keyframes[0].tween.unwrap().kind, anicraft_doc::TweenKind::Classic);
}

#[test]
fn scenes_play_in_order_edit_separately_and_survive_save_and_undo() {
    let mut s = Session::default();
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 10, "h": 10}));
    run(&mut s, "timeline.insert_frame", json!({"count": 4})); // scene 1: 5 frames
    let v = run(&mut s, "scene.new", json!({}));
    assert_eq!((v["scene"].as_u64(), v["name"].as_str()), (Some(1), Some("Scene 2")));
    assert_eq!(s.frame_count(), 1, "the new scene starts empty, with one layer");
    assert!(s.visible_elements().is_empty());
    run(&mut s, "draw.ellipse", json!({"x": 0, "y": 0, "w": 20, "h": 20}));
    run(&mut s, "timeline.insert_frame", json!({"count": 2})); // scene 2: 3 frames
    // The movie is both scenes in order; the editor shows one scene.
    assert_eq!(s.doc.frame_count(), 8);
    assert_eq!(s.doc.locate(6), (1, 1));
    let kind = |f| anicraft_doc::evaluate(&s.doc, f).items.len();
    assert_eq!((kind(4), kind(5)), (1, 1));
    assert_ne!(anicraft_doc::evaluate(&s.doc, 4).items[0].path, anicraft_doc::evaluate(&s.doc, 5).items[0].path, "frame 6 of the movie is scene 2");
    // Navigation.
    run(&mut s, "scene.prev", json!({}));
    assert_eq!((s.scene, s.frame_count()), (0, 5));
    run(&mut s, "scene.next", json!({}));
    assert_eq!(s.scene, 1);
    // Duplicating copies the artwork with fresh ids, so picking stays unambiguous.
    run(&mut s, "scene.duplicate", json!({}));
    assert_eq!((s.scene, s.doc.scenes[2].name.as_str()), (2, "Scene 2 copy"));
    let ids = |i: usize| s.doc.timeline(i).layers.iter().flat_map(|l| l.keyframes.iter().flat_map(|k| k.elements.iter().map(|e| e.id))).collect::<Vec<_>>();
    assert!(ids(2).iter().all(|id| !ids(1).contains(id)));
    // Save and load keep the scenes; a sequence export covers the whole movie.
    let dir = std::env::temp_dir().join(format!("anicraft-scenes-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("two.anicraft");
    run(&mut s, "file.save", json!({"path": path.display().to_string()}));
    let mut t = Session::default();
    run(&mut t, "file.open", json!({"path": path.display().to_string()}));
    assert_eq!(t.doc.scenes.iter().map(|sc| sc.name.as_str()).collect::<Vec<_>>(), ["Scene 1", "Scene 2", "Scene 2 copy"]);
    let v = run(&mut t, "file.export_png_sequence", json!({"dir": dir.join("seq").display().to_string(), "scale": 0.05}));
    assert_eq!(v["frames"], 11);
    // Deleting a scene with artwork asks first (undoable), and undo brings it back.
    assert!(s.confirmation("scene.delete", &json!({})).is_some_and(|c| !c.irreversible));
    run(&mut s, "scene.delete", json!({}));
    assert_eq!((s.doc.scenes.len(), s.scene), (2, 1));
    s.undo().unwrap();
    assert_eq!((s.doc.scenes.len(), s.scene), (3, 2), "undo returns to the restored scene");
    // Reorder and rename.
    run(&mut s, "scene.move_up", json!({}));
    assert_eq!((s.scene, s.doc.scenes[1].name.as_str()), (1, "Scene 2 copy"));
    run(&mut s, "scene.rename", json!({"name": "Ending"}));
    assert_eq!(s.doc.scenes[1].name, "Ending");
    assert!(s.execute("scene.rename", &json!({"name": " "})).is_err());
    assert!(s.execute("scene.select", &json!({"scene": 99})).is_err());
    // The last scene can't be deleted.
    let mut one = Session::default();
    assert!(one.execute("scene.delete", &json!({})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn files_from_before_scenes_open_as_one_scene() {
    let v2 = r#"{"format":"anicraft","version":2,"paths":["M0 0L5 0L5 5Z"],"document":{"timeline":{"layers":[{"id":1,"name":"Art","keyframes":[{"start":0,"duration":3,"elements":[{"id":2,"kind":{"Shape":{"path":0,"fill":{"Solid":{"r":1,"g":0,"b":0,"a":1}}}}}]}]}]},"next_id":3}}"#;
    let d = crate::format::from_json(v2).unwrap();
    assert_eq!(d.scenes.len(), 1);
    assert_eq!(d.scenes[0].name, "Scene 1");
    assert_eq!(d.frame_count(), 3);
    assert_eq!(anicraft_doc::evaluate(&d, 0).items.len(), 1);
}

#[test]
fn layer_folders_hold_hide_lock_move_and_delete_their_layers() {
    let mut s = Session::default();
    let art = s.layer;
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 50, "h": 50}));
    let bg = run(&mut s, "layer.new", json!({"name": "Background"}))["layer"].as_u64().map(LayerId).unwrap();
    // Background sits above Layer 1; move it below so the order is: Layer 1, Background.
    run(&mut s, "layer.move_down", json!({"layer": bg.0}));
    run(&mut s, "layer.select", json!({"layer": art.0}));
    let folder = run(&mut s, "layer.new_folder", json!({}))["layer"].as_u64().map(LayerId).unwrap();
    let tl = |s: &Session| s.doc.timeline(s.scene).clone();
    let names = |s: &Session| tl(s).layers.iter().map(|l| l.name.clone()).collect::<Vec<_>>();
    assert_eq!(names(&s), ["Folder 1", "Layer 1", "Background"]);
    assert!(tl(&s).layers[0].keyframes.is_empty(), "a folder has no frames");
    // A new folder is empty; a layer moved in under it goes inside.
    assert_eq!(tl(&s).layers[1].parent, None);
    run(&mut s, "layer.move_up", json!({"layer": art.0}));
    run(&mut s, "layer.move_down", json!({"layer": art.0}));
    assert_eq!(tl(&s).layers[1].parent, Some(folder), "moved in under the folder: inside it");
    assert_eq!(tl(&s).depth(1), 1);

    // A hidden folder hides what's in it; a locked folder locks it.
    let items = |s: &Session| anicraft_doc::evaluate_scene(&s.doc, s.scene, 0).items.len();
    assert_eq!(items(&s), 1);
    run(&mut s, "layer.toggle_visible", json!({"layer": folder.0}));
    assert_eq!(items(&s), 0);
    run(&mut s, "layer.toggle_visible", json!({"layer": folder.0}));
    run(&mut s, "layer.toggle_lock", json!({"layer": folder.0}));
    assert_eq!(s.hit_test(Vec2::new(10.0, 10.0), 0.0), None, "locked by its folder");
    run(&mut s, "layer.select", json!({"layer": art.0}));
    assert!(s.execute("draw.rect", &json!({"x": 0, "y": 0, "w": 5, "h": 5})).is_err());
    run(&mut s, "layer.toggle_lock", json!({"layer": folder.0}));
    // Nothing draws on a folder, and frame commands refuse it.
    run(&mut s, "layer.select", json!({"layer": folder.0}));
    assert!(s.execute("draw.rect", &json!({"x": 0, "y": 0, "w": 5, "h": 5})).is_err());
    assert!(s.execute("timeline.insert_frame", &json!({})).is_err());

    // The folder moves with its contents, past a whole sibling.
    run(&mut s, "layer.move_down", json!({"layer": folder.0}));
    assert_eq!(names(&s), ["Background", "Folder 1", "Layer 1"]);
    assert_eq!(tl(&s).layers[2].parent, Some(folder));
    run(&mut s, "layer.move_up", json!({"layer": folder.0}));
    assert_eq!(names(&s), ["Folder 1", "Layer 1", "Background"]);

    // A mask inside the folder; turning it back into a normal layer keeps both in the folder.
    run(&mut s, "layer.select", json!({"layer": art.0}));
    let spot = run(&mut s, "layer.new", json!({"name": "Spot"}))["layer"].as_u64().map(LayerId).unwrap();
    assert_eq!(tl(&s).layer(spot).unwrap().parent, Some(folder), "added inside the folder");
    run(&mut s, "layer.toggle_mask", json!({"layer": spot.0}));
    assert_eq!(tl(&s).layer(art).unwrap().parent, Some(spot), "masked by the mask in the folder");
    run(&mut s, "layer.toggle_mask", json!({"layer": spot.0}));
    assert_eq!((tl(&s).layer(art).unwrap().parent, tl(&s).layer(spot).unwrap().parent), (Some(folder), Some(folder)));

    // Moving the first layer up and out of the folder.
    run(&mut s, "layer.move_up", json!({"layer": spot.0}));
    assert_eq!(names(&s), ["Spot", "Folder 1", "Layer 1", "Background"]);
    assert_eq!(tl(&s).layer(spot).unwrap().parent, None);

    // Deleting a folder takes its contents (it asks first), and undo brings them back.
    let c = s.confirmation("layer.delete", &json!({"layer": folder.0})).expect("asks");
    assert!(!c.irreversible && c.message.contains("1 layer"));
    let v = run(&mut s, "layer.delete", json!({"layer": folder.0}));
    assert_eq!(v["layers"], 2);
    assert_eq!(names(&s), ["Spot", "Background"]);
    s.undo().unwrap();
    assert_eq!(names(&s), ["Spot", "Folder 1", "Layer 1", "Background"]);
    // Folders save and load.
    let back = crate::format::from_json(&crate::format::to_json(&s.doc).unwrap()).unwrap();
    assert_eq!(back.timeline(0).layers[1].kind, anicraft_doc::LayerKind::Folder);
    assert_eq!(back.timeline(0).layers[2].parent, Some(folder));
    // A folder never becomes another kind of layer.
    assert!(s.execute("layer.set_type", &json!({"layer": folder.0, "type": "normal"})).is_err());
}

#[test]
fn add_motion_guide_steers_the_tween_and_stays_out_of_exports() {
    let mut s = Session::default();
    let art = s.layer;
    run(&mut s, "draw.ellipse", json!({"x": -10, "y": -10, "w": 20, "h": 20}));
    run(&mut s, "timeline.goto", json!({"frame": 9}));
    run(&mut s, "timeline.insert_keyframe", json!({}));
    run(&mut s, "edit.select_all", json!({}));
    run(&mut s, "element.translate", json!({"dx": 200, "dy": 0}));
    run(&mut s, "timeline.goto", json!({"frame": 0}));
    run(&mut s, "timeline.create_tween", json!({}));
    // Add the guide and draw an arch on it: up from (0,0) over to (200,0).
    let v = run(&mut s, "layer.add_motion_guide", json!({"layer": art.0}));
    let guide = LayerId(v["guide"].as_u64().unwrap());
    assert_eq!(s.layer, guide, "the new guide is active, ready to draw on");
    assert_eq!(s.doc.timeline(s.scene).layer(art).unwrap().parent, Some(guide));
    run(&mut s, "draw.path", json!({"d": "M0 0 C0 -150 200 -150 200 0"}));
    let y_at = |s: &Session, f| anicraft_doc::evaluate_scene(&s.doc, s.scene, f).items.iter().find(|i| i.layer == art).unwrap().transform.translation().y;
    assert!(y_at(&s, 5) < -60.0, "midway the ball is up on the arch: {}", y_at(&s, 5));
    // The guide line shows for editing but never in a rendered frame.
    let px = |s: &Session, x, y| anicraft_render::render_frame(&s.doc, 0, 1.0).unwrap().pixel(x, y).unwrap();
    assert_eq!(px(&s, 100, 300), px(&s, 600, 600), "no trace of the guide stroke in output");
    // Orient to path, and the guided type.
    run(&mut s, "layer.select", json!({"layer": art.0}));
    assert_eq!(run(&mut s, "timeline.set_orient", json!({"orient": true}))["orient"], true);
    assert!(s.active_layer().unwrap().keyframes[0].tween.unwrap().orient);
    assert_eq!(run(&mut s, "layer.set_type", json!({"type": "normal"}))["type"], "normal");
    assert_eq!(run(&mut s, "layer.set_type", json!({"type": "guided"}))["type"], "guided");
    // A masked layer can't also follow a guide.
    let mask = run(&mut s, "layer.new", json!({"name": "M"}))["layer"].as_u64().unwrap();
    run(&mut s, "layer.toggle_mask", json!({"layer": mask}));
    let masked = s.doc.timeline(s.scene).layers.iter().find(|l| l.parent == Some(LayerId(mask))).map(|l| l.id);
    let masked = masked.expect("the mask took the layer below it");
    assert!(s.execute("layer.add_motion_guide", &json!({"layer": masked.0})).is_err());
    // Undo takes back one step at a time: here, the mask.
    let n = s.doc.timeline(s.scene).layers.len();
    s.undo().unwrap();
    assert_eq!(s.doc.timeline(s.scene).layers.len(), n);
    assert!(s.doc.timeline(s.scene).layers.iter().all(|l| l.kind != anicraft_doc::LayerKind::Mask));
}

#[test]
fn sprite_sheet_export_lays_frames_in_a_grid_with_a_frame_map() {
    let dir = std::env::temp_dir().join(format!("anicraft-sheet-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let sheet = dir.join("ball.png");
    let _ = std::fs::remove_file(&sheet);
    let _ = std::fs::remove_file(dir.join("ball.json"));
    let mut s = Session::new(sample::demo());
    let p = json!({"path": sheet.display().to_string(), "from": 0, "to": 4, "scale": 0.1, "padding": 2});
    let v = run(&mut s, "file.export_sprite_sheet", p.clone());
    // 5 frames of 128×72 in 3 columns × 2 rows, 2 px apart.
    assert_eq!((v["columns"].as_u64(), v["rows"].as_u64()), (Some(3), Some(2)));
    assert_eq!((v["width"].as_u64(), v["height"].as_u64()), (Some(3 * 128 + 4), Some(2 * 72 + 2)));
    let map: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("ball.json")).unwrap()).unwrap();
    let frames = map["frames"].as_object().unwrap();
    assert_eq!(frames.len(), 5);
    assert_eq!(frames["ball_0005"]["frame"], json!({"x": 130, "y": 74, "w": 128, "h": 72}));
    assert_eq!(map["meta"]["image"], "ball.png");
    let total: u64 = frames.values().map(|f| f["duration"].as_u64().unwrap()).sum();
    assert!((200..=220).contains(&total), "5 frames at 24 fps ≈ 208 ms: {total}");
    // The PNG is that size, transparent between frames, and each cell matches a plain render.
    let dec = png::Decoder::new(std::io::Cursor::new(std::fs::read(&sheet).unwrap()));
    let mut r = dec.read_info().unwrap();
    let mut buf = vec![0; r.output_buffer_size()];
    let info = r.next_frame(&mut buf).unwrap();
    assert_eq!((info.width, info.height), (388, 146));
    let at = |x: usize, y: usize| buf[(y * 388 + x) * 4 + 3];
    assert_eq!(at(129, 10), 0, "the padding is transparent");
    // Replacing either file asks first; an impossible sheet is refused.
    assert!(s.execute("file.export_sprite_sheet", &p).is_err());
    assert!(s.execute("file.export_sprite_sheet", &json!({"path": dir.join("huge.png").display().to_string(), "scale": 8})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn symbol_type_and_graphic_looping_commands() {
    let mut s = Session::default();
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 10, "h": 10}));
    run(&mut s, "edit.select_all", json!({}));
    run(&mut s, "modify.convert_to_symbol", json!({"name": "Box"}));
    let v = run(&mut s, "symbol.set_kind", json!({"kind": "movie_clip"}));
    assert_eq!(v["kind"], "MovieClip");
    assert_eq!(s.doc.library[0].kind, anicraft_doc::SymbolKind::MovieClip);
    assert!(s.execute("symbol.set_kind", &json!({"kind": "sprite"})).is_err());
    run(&mut s, "symbol.set_kind", json!({"kind": "graphic"}));
    run(&mut s, "element.set_loop", json!({"loop": "single_frame", "first_frame": 3}));
    let el = s.visible_elements()[0].clone();
    assert!(matches!(el.kind, ElementKind::Symbol { loop_mode: anicraft_doc::LoopMode::SingleFrame, first_frame: 3, .. }));
    assert!(s.execute("element.set_loop", &json!({})).is_err());
    assert!(s.execute("element.set_loop", &json!({"loop": "sometimes"})).is_err());
    // Undo restores the previous looping in one step.
    s.undo().unwrap();
    assert!(matches!(s.visible_elements()[0].kind, ElementKind::Symbol { loop_mode: anicraft_doc::LoopMode::Loop, first_frame: 0, .. }));
    // Shapes have no looping.
    run(&mut s, "draw.ellipse", json!({"x": 50, "y": 50, "w": 10, "h": 10}));
    assert!(s.execute("element.set_loop", &json!({"loop": "loop"})).is_err());
}

#[test]
fn video_export_streams_frames_to_ffmpeg_or_explains_how_to_get_it() {
    let dir = std::env::temp_dir().join(format!("anicraft-video-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Session::new(sample::demo());
    // Without ffmpeg: a clear message, no file.
    let missing = dir.join("none.webm");
    let e = s.execute("file.export_video", &json!({"path": missing.display().to_string(), "ffmpeg": dir.join("no-such-ffmpeg").display().to_string(), "to": 2, "scale": 0.1})).unwrap_err();
    assert!(e.to_string().contains("ffmpeg"), "{e}");
    assert!(!missing.exists());
    assert!(s.execute("file.export_video", &json!({"path": dir.join("x.avi").display().to_string()})).is_err(), "only .mp4 and .webm");
    // With an ffmpeg (on the PATH or in ANICRAFT_FFMPEG), real files; skipped where there is none.
    let program = std::env::var("ANICRAFT_FFMPEG").unwrap_or_else(|_| "ffmpeg".into());
    let have = std::process::Command::new(&program).arg("-version").stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_ok_and(|st| st.success());
    if !have {
        eprintln!("no ffmpeg here: only the missing-ffmpeg path was tested");
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    for (name, magic_at, magic) in [("clip.webm", 0usize, &[0x1A, 0x45, 0xDF, 0xA3][..]), ("clip.mp4", 4, b"ftyp")] {
        let out = dir.join(name);
        let _ = std::fs::remove_file(&out);
        // Scale 0.1 makes a 128×72 picture; 0.105 an odd 134×76 → padded to even.
        let v = run(&mut s, "file.export_video", json!({"path": out.display().to_string(), "from": 0, "to": 5, "scale": 0.105}));
        assert_eq!(v["frames"], 6, "{name}");
        let bytes = std::fs::read(&out).unwrap();
        assert!(bytes.len() > 200, "{name}: {} bytes", bytes.len());
        assert_eq!(&bytes[magic_at..magic_at + magic.len()], magic, "{name} container");
        // Replacing it needs consent.
        assert!(s.execute("file.export_video", &json!({"path": out.display().to_string(), "to": 1, "scale": 0.1})).is_err());
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// An 8×8 solid red JPEG (made with ffmpeg for this test).
const TINY_JPEG_HEX: &str = "ffd8ffe000104a46494600010200000100010000fffe000f4c61766336332e322e31303000ffdb0043000804040404040505050505050606060606060606060606060607070708080807070706060707080808080909090808080809090a0a0a0c0c0b0b0e0e0e111114ffc4004c0001010000000000000000000000000000000601010100000000000000000000000000000607100100000000000000000000000000000000110100000000000000000000000000000000ffc00011080008000803012200021100031100ffda000c03010002110311003f008b004d7f7fffd9";

#[test]
fn images_import_as_bitmaps_render_save_and_export() {
    let dir = std::env::temp_dir().join(format!("anicraft-bitmap-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // A 4×2 PNG: left half red, right half translucent blue.
    let mut rgba = Vec::new();
    for _y in 0..2 {
        for x in 0..4 {
            rgba.extend_from_slice(if x < 2 { &[255, 0, 0, 255] } else { &[0, 0, 255, 128] });
        }
    }
    let png_path = dir.join("tile.png");
    std::fs::write(&png_path, anicraft_render::encode_png(4, 2, &rgba).unwrap()).unwrap();
    let jpg_path = dir.join("red.jpg");
    let jpg: Vec<u8> = (0..TINY_JPEG_HEX.len()).step_by(2).map(|i| u8::from_str_radix(&TINY_JPEG_HEX[i..i + 2], 16).unwrap()).collect();
    std::fs::write(&jpg_path, jpg).unwrap();

    let mut s = Session::default();
    let v = run(&mut s, "file.import", json!({"path": png_path.display().to_string(), "x": 10, "y": 20}));
    assert_eq!((v["width"].as_f64(), v["height"].as_f64()), (Some(4.0), Some(2.0)));
    assert_eq!(s.doc.bitmaps.len(), 1);
    // Scaled up 10× the left half renders red and the right half translucent blue over white.
    run(&mut s, "element.set_transform", json!({"sx": 10, "sy": 10}));
    let px = |s: &Session, x, y| anicraft_render::render_frame(&s.doc, 0, 1.0).unwrap().pixel(x, y).unwrap();
    let left = px(&s, 15, 25);
    assert!(left[0] > 0.99 && left[1] < 0.01 && left[2] < 0.01, "{left:?}");
    let right = px(&s, 45, 25);
    assert!((right[0] - 0.5).abs() < 0.05 && (right[2] - 1.0).abs() < 0.05, "half blue over white: {right:?}");
    assert_eq!(px(&s, 5, 5), [1.0, 1.0, 1.0, 1.0], "outside the image is untouched");

    // A JPEG imports too (kept as PNG).
    let v = run(&mut s, "file.import", json!({"path": jpg_path.display().to_string()}));
    assert_eq!(v["width"].as_f64(), Some(8.0));
    assert_eq!(s.doc.bitmaps.len(), 2);
    assert!(s.doc.bitmaps[1].png.starts_with(b"\x89PNG"));
    let c = s.doc.bitmaps[1].pixels().unwrap().at(4, 4);
    assert!(c[0] > 0.9 && c[1] < 0.1, "red: {c:?}");

    // Bitmaps save and load, and SVG export embeds each image once.
    let doc_path = dir.join("pics.anicraft");
    run(&mut s, "file.save", json!({"path": doc_path.display().to_string()}));
    let back = crate::format::load(&doc_path).unwrap();
    assert_eq!(back.bitmaps.len(), 2);
    assert_eq!(back.bitmaps[0].pixels().unwrap().rgba, rgba);
    run(&mut s, "edit.select_all", json!({}));
    run(&mut s, "edit.duplicate", json!({}));
    let svg = crate::cmds_file::frame_svg(&s.doc, 0);
    // Two images, four instances (Select All took both layers, ADR 0044): each image is
    // embedded once.
    assert_eq!(svg.matches("data:image/png;base64,").count(), 2);
    assert_eq!(svg.matches("<pattern ").count(), 4);
    // Undo takes an import back, bitmap and all.
    let n = s.doc.bitmaps.len();
    s.undo().unwrap();
    s.undo().unwrap();
    assert!(s.doc.bitmaps.len() < n);
    // Anything else is refused cleanly.
    std::fs::write(dir.join("fake.png"), b"not an image").unwrap();
    assert!(s.execute("file.import", &json!({"path": dir.join("fake.png").display().to_string()})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn library_bitmaps_place_centred_on_the_stage() {
    let dir = std::env::temp_dir().join(format!("anicraft-libbitmap-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let png_path = dir.join("dot.png");
    std::fs::write(&png_path, anicraft_render::encode_png(6, 4, &[0, 128, 0, 255].repeat(24)).unwrap()).unwrap();
    let mut s = Session::default();
    let v = run(&mut s, "file.import", json!({"path": png_path.display().to_string()}));
    let bitmap = v["bitmap"].as_u64().expect("the new bitmap's id");
    assert_eq!(s.doc.bitmaps[0].id.0, bitmap);
    // Placing it again from the Library adds a second picture, centred, on the current layer.
    let v = run(&mut s, "library.place", json!({"bitmap": bitmap, "x": 100, "y": 50}));
    let id = anicraft_doc::ElementId(v["element"].as_u64().unwrap());
    assert_eq!(s.selection.ids().collect::<Vec<_>>(), vec![id]);
    let el = s.active_layer().unwrap().keyframe_at(s.frame).unwrap().elements.iter().find(|e| e.id == id).unwrap().clone();
    assert_eq!(el.name, "dot.png");
    assert_eq!(el.transform.apply(anicraft_geom::Vec2::new(3.0, 2.0)), anicraft_geom::Vec2::new(100.0, 50.0), "centred on x/y");
    match &el.kind {
        anicraft_doc::ElementKind::Shape(sh) => assert_eq!(sh.fill, Some(anicraft_doc::Paint::Bitmap { bitmap: anicraft_doc::BitmapId(bitmap), repeat: false, matrix: anicraft_geom::Affine::IDENTITY })),
        k => panic!("{k:?}"),
    }
    assert_eq!(s.doc.bitmaps.len(), 1, "placing shares the bitmap");
    // Unknown bitmaps and missing params are refused.
    assert!(s.execute("library.place", &json!({"bitmap": 999_999})).is_err());
    assert!(s.execute("library.place", &json!({})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lottie_export_writes_the_movie_and_asks_before_replacing() {
    let dir = std::env::temp_dir().join(format!("anicraft-lottie-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("walk.json");
    let mut s = Session::new(sample::demo());
    let frames = s.doc.frame_count();
    let v = run(&mut s, "file.export_lottie", json!({"path": path.display().to_string()}));
    assert_eq!(v["frames"].as_u64(), Some(u64::from(frames)));
    let lottie: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(lottie["nm"], "walk");
    assert_eq!(lottie["op"].as_u64(), Some(u64::from(frames)));
    assert_eq!(lottie["layers"].as_array().unwrap().len() as u64, v["layers"].as_u64().unwrap(), "transparent by default");
    // Replacing needs consent; ranges are checked.
    let p = json!({"path": path.display().to_string(), "from": 5, "to": 9, "background": true});
    assert!(s.execute("file.export_lottie", &p).is_err());
    let mut p = p;
    p["confirm"] = json!(true);
    assert_eq!(run(&mut s, "file.export_lottie", p)["frames"], 5);
    assert!(s.execute("file.export_lottie", &json!({"path": dir.join("x.json").display().to_string(), "from": 9, "to": 5})).is_err());

    // A masked shape carries its mask; a bitmap fill is reported, not exported.
    let mut s = Session::default();
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 200, "h": 200, "fill": "#3366ff"}));
    run(&mut s, "layer.new", json!({"name": "Mask"}));
    run(&mut s, "draw.ellipse", json!({"x": 50, "y": 50, "w": 40, "h": 40}));
    run(&mut s, "layer.toggle_mask", json!({}));
    let out = anicraft_render::lottie::to_lottie(&s.doc, "m", 0, 0, false);
    let v: Value = serde_json::from_str(&out.json).unwrap();
    let layers = v["layers"].as_array().unwrap();
    assert_eq!(layers.len(), 1, "the mask's own artwork is not drawn");
    assert_eq!(layers[0]["hasMask"], true);
    assert_eq!(layers[0]["masksProperties"][0]["mode"], "a");
    assert!(out.report.is_empty(), "{:?}", out.report);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn html_export_writes_one_page_that_holds_the_interactive_movie() {
    let dir = std::env::temp_dir().join(format!("anicraft-html-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("menu.html");
    let mut s = Session::new(sample::interactive());
    let v = run(&mut s, "file.export_html", json!({"path": path.display().to_string()}));
    let html = std::fs::read_to_string(&path).unwrap();
    assert_eq!(v["bytes"].as_u64(), Some(html.len() as u64));
    assert!(html.starts_with("<!DOCTYPE html>") && html.contains("<title>menu</title>"), "the title defaults to the file name");
    assert!(html.contains("\"op\":\"goto_and_play\"") && html.contains("new AniCraft.Player"), "actions and the player are in the page");
    assert_eq!((v["scenes"].as_u64(), v["frames"].as_u64()), (Some(1), Some(u64::from(s.doc.frame_count()))));
    assert!(v["timelines"].as_u64().unwrap() >= 3, "the scene, the button and the spinner: {v}");
    assert_eq!(v["report"], "");
    // Replacing needs consent; params are checked.
    let p = json!({"path": path.display().to_string(), "title": "Menu", "fit": false, "sound": false});
    assert!(s.execute("file.export_html", &p).is_err());
    let mut p = p;
    p["confirm"] = json!(true);
    run(&mut s, "file.export_html", p);
    assert!(std::fs::read_to_string(&path).unwrap().contains("<title>Menu</title>"));
    assert!(s.execute("file.export_html", &json!({"path": dir.join("x.html").display().to_string(), "title": "t".repeat(300)})).is_err());
    assert!(s.execute("file.export_html", &json!({"path": dir.join("x.html").display().to_string(), "fit": "yes"})).is_err());
    assert!(s.execute("file.export_html", &json!({})).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn text_lays_out_renders_edits_saves_and_breaks_apart() {
    if anicraft_text::Fonts::get().families().is_empty() {
        eprintln!("no fonts on this computer: text test skipped");
        return;
    }
    let mut s = Session::default();
    let v = run(&mut s, "text.create", json!({"text": "Hello", "x": 100, "y": 50, "size": 64, "color": "#cc2200"}));
    let id = ElementId(v["element"].as_u64().unwrap());
    assert_eq!(s.selection.ids().collect::<Vec<_>>(), vec![id]);
    assert!(v["width"].as_f64().unwrap() > 64.0);
    let text = |s: &Session| match &s.visible_elements().iter().find(|e| e.id == id).unwrap().kind {
        ElementKind::Text(t) => t.clone(),
        k => panic!("{k:?}"),
    };
    let t = text(&s);
    assert_eq!(t.text, "Hello");
    assert!(!t.font.is_empty() && !t.outline.cmds.is_empty());
    // It renders: red ink inside its box, nothing outside.
    let frame = anicraft_render::render_frame(&s.doc, 0, 1.0).unwrap();
    let mut red = 0;
    for y in 50..(50 + t.box_height as u32) {
        for x in 100..(100 + t.box_width as u32) {
            let p = frame.pixel(x, y).unwrap();
            if p[0] > 0.5 && p[1] < 0.3 {
                red += 1;
            }
        }
    }
    assert!(red > 200, "{red} red pixels");
    assert_eq!(frame.pixel(90, 40).unwrap(), [1.0, 1.0, 1.0, 1.0]);
    // Bounds and picking follow the box.
    assert!(s.hit_test(Vec2::new(100.0 + t.box_width * 0.5, 50.0 + t.box_height * 0.5), 0.0).is_some());

    // Editing re-lays it out; undo brings the old words back; the style is remembered.
    run(&mut s, "text.edit", json!({"text": "Hello, world", "bold": true, "align": "center", "width": 400}));
    let edited = text(&s);
    assert_eq!((edited.text.as_str(), edited.bold, edited.width), ("Hello, world", true, Some(400.0)));
    assert_eq!(edited.box_width, 400.0);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(text(&s).text, "Hello");
    run(&mut s, "edit.redo", json!({}));
    assert!(s.execute("text.edit", &json!({"text": "  "})).is_err());
    assert!(s.execute("text.edit", &json!({"align": "justify"})).is_err());
    assert!(s.execute("text.create", &json!({"text": "x", "size": 0})).is_err());
    let next = run(&mut s, "text.create", json!({"text": "again"}));
    let next_id = ElementId(next["element"].as_u64().unwrap());
    let again = s.visible_elements().iter().find(|e| e.id == next_id).map(|e| e.kind.clone()).unwrap();
    assert!(matches!(again, ElementKind::Text(t) if t.bold && t.size == 64.0), "new text takes the last style");
    // Asking for a missing family falls back and says so.
    let v = run(&mut s, "text.create", json!({"text": "fallback", "font": "No Such Family 12345"}));
    assert!(v["note"].as_str().unwrap().contains("isn't installed"));

    // Text saves with its outlines, and breaks apart into the same shape.
    let dir = std::env::temp_dir().join(format!("anicraft-text-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("t.anicraft");
    run(&mut s, "file.save", json!({"path": path.display().to_string()}));
    let back = crate::format::load(&path).unwrap();
    assert_eq!(back, s.doc);
    run(&mut s, "element.select", json!({"ids": [id.0]}));
    let before = anicraft_render::render_frame(&s.doc, 0, 1.0).unwrap();
    run(&mut s, "modify.break_apart", json!({}));
    assert!(matches!(s.visible_elements().iter().find(|e| s.selection.has_id(e.id)).unwrap().kind, ElementKind::Shape(_)));
    assert_eq!(anicraft_render::render_frame(&s.doc, 0, 1.0).unwrap().data, before.data, "a shape exactly like the text");
    let fonts = run(&mut s, "text.fonts", json!({}));
    assert!(fonts["families"].as_array().is_some_and(|f| !f.is_empty()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn blend_modes_apply_to_movie_clips_and_survive_a_save() {
    let mut s = Session::default();
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 200, "h": 100, "fill": "#8080ff"}));
    run(&mut s, "layer.new", json!({}));
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 100, "h": 100, "fill": "#ff0000"}));
    run(&mut s, "modify.convert_to_symbol", json!({"name": "Red"}));
    // A graphic can't blend, as in Animate; a movie clip can.
    assert!(s.execute("element.set_blend", &json!({"mode": "multiply"})).unwrap_err().to_string().contains("movie clip"));
    run(&mut s, "symbol.set_kind", json!({"kind": "movie_clip"}));
    assert!(s.execute("element.set_blend", &json!({"mode": "sparkle"})).is_err());
    let v = run(&mut s, "element.set_blend", json!({"mode": "Multiply"}));
    assert_eq!(v["mode"], "multiply");
    let px = |s: &Session| anicraft_render::render_frame(&s.doc, 0, 1.0).unwrap().pixel(50, 50).unwrap();
    let p = px(&s);
    assert!((p[0] - 128.0 / 255.0).abs() < 0.01 && p[1] < 0.01 && p[2] < 0.01, "red × lavender: {p:?}");
    // Undo returns to normal; it saves and loads; SVG export writes the CSS blend mode.
    run(&mut s, "edit.undo", json!({}));
    assert!(px(&s)[0] > 0.99);
    run(&mut s, "edit.redo", json!({}));
    let text = crate::format::to_json(&s.doc).unwrap();
    assert!(text.contains("\"blend\": \"multiply\"") || text.contains("\"blend\":\"multiply\""));
    assert_eq!(crate::format::from_json(&text).unwrap(), s.doc);
    assert!(crate::cmds_file::frame_svg(&s.doc, 0).contains("mix-blend-mode:multiply"));
    let lottie = anicraft_render::lottie::to_lottie(&s.doc, "b", 0, 0, false);
    assert!(lottie.json.contains("\"bm\":1"), "Lottie multiply");
    assert!(lottie.report.contains_key("blend modes (applied shape by shape)"));
}

#[test]
fn filters_render_on_movie_clips_clip_after_masks_and_save() {
    let mut s = Session::default();
    run(&mut s, "draw.rect", json!({"x": 100, "y": 100, "w": 100, "h": 100, "fill": "#ff0000", "stroke": null}));
    run(&mut s, "modify.convert_to_symbol", json!({"name": "Box"}));
    let shadow = json!({"filters": [{"type": "drop_shadow", "blur_x": 0, "blur_y": 0, "distance": 20, "angle": 0, "color": "#000000"}]});
    assert!(s.execute("element.set_filters", &shadow).unwrap_err().to_string().contains("movie clip"), "graphics can't");
    run(&mut s, "symbol.set_kind", json!({"kind": "movie_clip"}));
    assert!(s.execute("element.set_filters", &json!({"filters": [{"type": "sparkle"}]})).is_err());
    run(&mut s, "element.set_filters", shadow);
    let px = |s: &Session, x, y| anicraft_render::render_frame(&s.doc, 0, 1.0).unwrap().pixel(x, y).unwrap();
    assert_eq!(px(&s, 150, 150), [1.0, 0.0, 0.0, 1.0], "the box on top");
    assert_eq!(px(&s, 210, 150), [0.0, 0.0, 0.0, 1.0], "its shadow 20 px to the right");
    assert_eq!(px(&s, 230, 150), [1.0, 1.0, 1.0, 1.0]);
    // A blur spreads past the edge.
    run(&mut s, "element.set_filters", json!({"filters": [{"type": "blur", "blur_x": 20, "blur_y": 20, "quality": 2}]}));
    let edge = px(&s, 95, 150);
    assert!(edge[1] < 0.95 && edge[1] > 0.2, "pink just outside: {edge:?}");
    // Adjust Color: greyscale.
    run(&mut s, "element.set_filters", json!({"filters": [{"type": "adjust_color", "saturation": -100}]}));
    let g = px(&s, 150, 150);
    assert!((g[0] - g[1]).abs() < 1e-3 && g[0] < 0.6, "grey: {g:?}");
    // It saves, loads and exports.
    let text = crate::format::to_json(&s.doc).unwrap();
    assert_eq!(crate::format::from_json(&text).unwrap(), s.doc);
    run(&mut s, "element.set_filters", json!({"filters": [{"type": "glow", "color": "#00ff00", "blur_x": 8, "blur_y": 8}]}));
    assert!(crate::cmds_file::frame_svg(&s.doc, 0).contains("<feDropShadow dx=\"0\" dy=\"0\""));
    assert!(anicraft_render::lottie::to_lottie(&s.doc, "f", 0, 0, false).report.keys().any(|k| k.starts_with("filters")));

    // Under a mask, the blur is clipped by the mask after filtering (as in Animate).
    run(&mut s, "element.set_filters", json!({"filters": [{"type": "blur", "blur_x": 40, "blur_y": 40, "quality": 3}]}));
    run(&mut s, "layer.new", json!({"name": "Mask"}));
    run(&mut s, "draw.rect", json!({"x": 0, "y": 0, "w": 150, "h": 400}));
    run(&mut s, "layer.toggle_mask", json!({}));
    let inside = px(&s, 95, 150);
    assert!(inside[1] < 0.99, "blurred colour inside the mask: {inside:?}");
    assert_eq!(px(&s, 160, 150), [1.0, 1.0, 1.0, 1.0], "nothing past the mask's edge");
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert!(px(&s, 160, 150)[1] < 0.5, "unmasked again: the box shows");
}

/// Extreme values in every new filter, on a movie clip: sanitized, rendered at several scales
/// and exported, never a panic or a non-number.
#[test]
fn hostile_bevel_and_gradient_filters_render_and_export() {
    let mut s = Session::default();
    run(&mut s, "draw.rect", json!({"x": 10, "y": 10, "w": 30, "h": 20, "fill": "#ff0000", "stroke": null}));
    run(&mut s, "modify.convert_to_symbol", json!({"name": "Box"}));
    run(&mut s, "symbol.set_kind", json!({"kind": "movie_clip"}));
    let stops = json!([{"offset": -5, "color": "#ff000000"}, {"offset": 0.5, "color": "#00ff00"}, {"offset": 0.5, "color": "#0000ff"}, {"offset": 99, "color": "#ffffff"}]);
    for side in ["inner", "outer", "full"] {
        for knockout in [false, true] {
            // Huge values once (they clamp to the largest reach, which is slow to draw).
            let huge = side == "full" && knockout;
            let cases: &[(f64, f64, f64)] = if huge { &[(1e30, 1e30, 1e30), (255.0, -1000.0, 10.0)] } else { &[(0.0, 0.0, 0.0), (0.5, 0.3, 0.001), (12.0, -40.0, 3.0)] };
            for &(blur, distance, strength) in cases {
                let common = json!({"blur_x": blur, "blur_y": blur / 3.0, "distance": distance, "strength": strength, "quality": 3, "angle": -720, "side": side, "knockout": knockout});
                let with = |t: &str| {
                    let mut f = common.clone();
                    f["type"] = json!(t);
                    f["stops"] = stops.clone();
                    f
                };
                run(&mut s, "element.set_filters", json!({"filters": [with("bevel"), with("gradient_glow"), with("gradient_bevel")]}));
                for scale in [0.05, if huge { 0.25 } else { 1.0 }] {
                    let img = anicraft_render::render_frame(&s.doc, 0, scale).unwrap();
                    assert!(img.pixel(0, 0).unwrap().iter().all(|v| v.is_finite()));
                }
                let svg = crate::cmds_file::frame_svg(&s.doc, 0);
                assert!(!svg.contains("NaN") && !svg.contains("inf"));
            }
        }
    }
}

/// A filtered instance on a surface as wide as the renderer allows keeps its shadow: the reach
/// is cut only where the buffer would outgrow the renderer, not around the artwork.
#[test]
fn filters_keep_their_reach_on_large_surfaces() {
    let mut s = Session::default();
    run(&mut s, "draw.rect", json!({"x": 100, "y": 100, "w": 100, "h": 100, "fill": "#ff0000", "stroke": null}));
    run(&mut s, "modify.convert_to_symbol", json!({"name": "Box"}));
    run(&mut s, "symbol.set_kind", json!({"kind": "movie_clip"}));
    run(&mut s, "element.set_filters", json!({"filters": [{"type": "drop_shadow", "blur_x": 0, "blur_y": 0, "distance": 20, "angle": 0, "color": "#000000"}]}));
    // The stage at 3.2× is 4096 px wide.
    let img = anicraft_render::render_frame(&s.doc, 0, 3.2).unwrap();
    assert_eq!(img.pixel(670, 480).unwrap(), [0.0, 0.0, 0.0, 1.0], "the shadow 20 px (64 px here) right of the box");
}

#[test]
fn bevel_and_gradient_filters_render_save_and_export() {
    use anicraft_doc::{ElementKind, Filter, FilterSide};
    let mut s = Session::default();
    run(&mut s, "draw.rect", json!({"x": 100, "y": 100, "w": 100, "h": 100, "fill": "#ff0000", "stroke": null}));
    run(&mut s, "modify.convert_to_symbol", json!({"name": "Box"}));
    run(&mut s, "symbol.set_kind", json!({"kind": "movie_clip"}));
    let px = |s: &Session, x, y| anicraft_render::render_frame(&s.doc, 0, 1.0).unwrap().pixel(x, y).unwrap();
    // A bevel lit from the top left, hex colours as in every command.
    run(&mut s, "element.set_filters", json!({"filters": [{"type": "bevel", "blur_x": 0, "blur_y": 0, "distance": 6, "highlight_color": "#00ff00", "shadow_color": "#0000ff"}]}));
    assert_eq!(px(&s, 102, 102), [0.0, 1.0, 0.0, 1.0], "highlight inside the top-left corner");
    assert_eq!(px(&s, 197, 197), [0.0, 0.0, 1.0, 1.0], "shadow inside the bottom-right corner");
    assert_eq!(px(&s, 150, 150), [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(px(&s, 95, 95), [1.0, 1.0, 1.0, 1.0], "inner: nothing outside");
    run(&mut s, "element.set_filters", json!({"filters": [{"type": "bevel", "blur_x": 0, "blur_y": 0, "distance": 6, "side": "outer", "knockout": true, "highlight_color": "#00ff00"}]}));
    assert_eq!(px(&s, 97, 97), [0.0, 1.0, 0.0, 1.0], "outer: highlight outside the lit corner");
    assert_eq!(px(&s, 150, 150), [1.0, 1.0, 1.0, 1.0], "knocked out");
    // A gradient glow whose ramp ends in blue; its start stays transparent whatever is sent.
    run(&mut s, "element.set_filters", json!({"filters": [{"type": "gradient_glow", "blur_x": 0, "blur_y": 0, "distance": 0, "stops": [{"offset": 0, "color": "#ff00ffff"}, {"offset": 1, "color": "#0000ff"}]}]}));
    let ElementKind::Symbol { filters, .. } = &s.doc.timeline(s.scene).layers[0].keyframes[0].elements[0].kind else { panic!() };
    let Some(Filter::GradientGlow { stops, side: FilterSide::Outer, .. }) = filters.first() else { panic!("{filters:?}") };
    assert_eq!(stops[0].color.a, 0.0, "pinned transparent");
    assert_eq!(px(&s, 150, 150), [1.0, 0.0, 0.0, 1.0]);
    assert!(s.execute("element.set_filters", &json!({"filters": [{"type": "gradient_bevel", "stops": vec![json!({"offset": 0.5, "color": "#ffffff"}); 16]}]})).is_err(), "too many stops");
    assert!(s.execute("element.set_filters", &json!({"filters": [{"type": "bevel", "side": "sideways"}]})).is_err());
    assert!(s.execute("element.set_filters", &json!({"filters": [{"type": "gradient_glow", "stops": [{"offset": 0, "color": "red"}]}]})).is_err());
    // All three save, load and export to SVG with standard primitives.
    run(&mut s, "element.set_filters", json!({"filters": [{"type": "bevel", "side": "full"}, {"type": "gradient_glow", "side": "full", "knockout": true}, {"type": "gradient_bevel"}]}));
    let text = crate::format::to_json(&s.doc).unwrap();
    assert_eq!(crate::format::from_json(&text).unwrap(), s.doc);
    let svg = crate::cmds_file::frame_svg(&s.doc, 0);
    for want in ["operator=\"arithmetic\" k2=\"1\" k3=\"-1\"", "<feFuncA type=\"table\"", "operator=\"atop\"", "k4=\"0.5\"", "result=\"ei1\"", "result=\"eo1\""] {
        assert!(svg.contains(want), "{want} in {svg}");
    }
    assert!(!svg.contains("NaN") && !svg.contains("inf"));
    // The filter region is the stage grown by the filters' reach (in stage units), so a far
    // bevel isn't cut; each margin is blur 5 × 0.5 + distance 5 + 1 = 8.5.
    assert!(svg.contains("filterUnits=\"userSpaceOnUse\" x=\"-25.5\" y=\"-25.5\" width=\"1331\" height=\"771\""), "{svg}");
    // A narrow ramp peak survives in the tables (one sample per CPU table step).
    run(&mut s, "element.set_filters", json!({"filters": [{"type": "gradient_glow", "stops": [{"offset": 0, "color": "#00000000"}, {"offset": 0.015625, "color": "#ff0000"}, {"offset": 0.03125, "color": "#00000000"}, {"offset": 1, "color": "#00000000"}]}]}));
    let svg = crate::cmds_file::frame_svg(&s.doc, 0);
    let alpha = svg.split("<feFuncA type=\"table\" tableValues=\"").nth(1).and_then(|t| t.split('"').next()).unwrap_or("");
    let values: Vec<&str> = alpha.split(' ').collect();
    assert_eq!(values.len(), 257);
    assert_eq!(values[4], "1", "the peak at 4/256");
    // Lottie reports them, as it does every filter.
    assert!(anicraft_render::lottie::to_lottie(&s.doc, "f", 0, 0, false).report.keys().any(|k| k.starts_with("filters")));
}
