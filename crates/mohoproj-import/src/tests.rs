//! Synthetic-document tests: build small `.mohoproj` JSON by hand and check what the
//! importer maps, what it reports, and that hostile input never panics. The corpus fixtures
//! (Moho Pro 14.4 native saves) are gated on their presence in `tests::fixtures`.

use crate::import::{import_json, Report};
use crate::zip;
use serde_json::{json, Value};

/// A minimal legal document (31 top-level keys are Moho's; only what the reader touches is
/// listed here), one empty MeshLayer on a 1280×720 stage.
fn minimal() -> Value {
    json!({
        "mime_type": "application/x-vnd.lm_mohodoc",
        "version": 1045,
        "major_version": 1,
        "rev_version": 0,
        "comment": "",
        "doc_uuid": "00000000-0000-0000-0000-000000000000",
        "created_date": "Fri Nov 6 12:00:00 2026",
        "modified_date": "Fri Nov 6 12:00:00 2026",
        "project_data": {
            "width": 1280, "height": 720, "fps": 24.0, "start_frame": 1, "end_frame": 240,
            "back_color": {"r": 234, "g": 234, "b": 234, "a": 255}
        },
        "thumbnail": "",
        "styles": [],
        "layercomps": [],
        "action_refs": [],
        "metadata": {"what": 0},
        "onions_enabled": false, "onions_sellayer": true, "onions_filled": true, "onions_colored": true, "onions_relative": true, "onions_behind": true,
        "onions_frame0": -100000, "onions_frame1": -100000, "onions_frame2": -100000, "onions_frame3": -100000,
        "onions_frame4": -100000, "onions_frame5": -100000, "onions_frame6": -100000, "onions_frame7": -100000,
        "animated_values": {},
        "layers": [{"type": "MeshLayer", "name": "Layer 1", "mesh": {"type": "Mesh", "points": [], "curves": [], "shapes": []}}],
        "documentviewstate": {}
    })
}

fn channel(t: &str, when: Vec<i64>, val: Vec<Value>) -> Value {
    let interp = when.iter().map(|_| json!({"im": 1, "v1": 0.1, "v2": 0.5, "in": 1, "h": 0, "s": false, "t": 0})).collect::<Vec<_>>();
    json!({"type": t, "ref": false, "mute": false, "when": when, "val": val, "interp": interp})
}

fn import(root: &Value) -> (anicraft_doc::Document, Report) {
    let read = |_: &str| -> Result<Option<Vec<u8>>, crate::MohoError> { Ok(None) };
    import_json(root.clone(), &read, None).unwrap_or_else(|e| panic!("import failed: {e}"))
}

#[test]
fn minimal_document_maps_stage_and_one_layer() {
    let (doc, report) = import(&minimal());
    assert_eq!(doc.stage.width, 1280);
    assert_eq!(doc.stage.height, 720);
    assert!((doc.stage.fps - 24.0).abs() < 1e-6);
    let l = &doc.timeline(0).layers[0];
    assert_eq!(l.name, "Layer 1");
    assert_eq!(l.keyframes.len(), 1);
    assert_eq!(l.keyframes[0].duration, 240, "frames 1..240 become one 240-frame span");
    assert!(report.skipped.is_empty());
    assert!(report.summary().starts_with("Imported everything"));
    // The sidecar keeps the whole original.
    assert_eq!(doc.maop_maohou.as_ref().unwrap().source, "mohoproj 1045");
    assert_eq!(doc.maop_maohou.as_ref().unwrap().raw["version"], 1045);
}

#[test]
fn the_layer_tree_flattens_top_first_with_folders_first() {
    let mut root = minimal();
    // Moho lists bottom-first: the switch sits above its siblings, its last variant on top.
    root["layers"] = json!([
        {"type": "MeshLayer", "name": "Ground", "mesh": {"points": [], "curves": [], "shapes": []}},
        {"type": "SwitchLayer", "name": "Mouths", "switch_keys": channel("String", vec![0], vec![json!("Smile")]), "layers": [
            {"type": "MeshLayer", "name": "Frown", "mesh": {"points": [], "curves": [], "shapes": []}},
            {"type": "MeshLayer", "name": "Smile", "mesh": {"points": [], "curves": [], "shapes": []}}
        ]},
        {"type": "GroupLayer", "name": "Head", "layers": [
            {"type": "MeshLayer", "name": "Eyes", "mesh": {"points": [], "curves": [], "shapes": []}}
        ]}
    ]);
    let (doc, report) = import(&root);
    let names: Vec<&str> = doc.timeline(0).layers.iter().map(|l| l.name.as_str()).collect();
    assert_eq!(names, ["Head", "Eyes", "Mouths", "Smile", "Frown", "Ground"], "top-first, folder before its run");
    let tl = doc.timeline(0);
    // Parent links: Eyes → Head, Smile/Frown → Mouths; folders draw nothing.
    let head = tl.layers.iter().find(|l| l.name == "Head").unwrap();
    assert_eq!(head.kind, anicraft_doc::LayerKind::Folder);
    let eyes = tl.layers.iter().find(|l| l.name == "Eyes").unwrap();
    assert_eq!(eyes.parent, Some(head.id));
    // The frozen selection keeps Smile visible and hides Frown.
    let smile = tl.layers.iter().find(|l| l.name == "Smile").unwrap();
    let frown = tl.layers.iter().find(|l| l.name == "Frown").unwrap();
    assert!(smile.visible && !frown.visible);
    assert!(report.skipped.contains_key("switch selection (frozen: Smile)"));
}

#[test]
fn static_layer_transforms_bake_into_elements() {
    let mut root = minimal();
    // A 0.5-inch square at layer translation (1, 0) inches, rotated 90° (π/2, Moho radians,
    // y-up), scaled 2×. The stage is 1280×720 at 320 dpi: 4 × 2.25 inches, centre (640, 360).
    root["layers"][0]["transforms"] = json!({
        "translation": channel("Vec3", vec![0], vec![json!({"x": 1.0, "y": 0.0, "z": 0.0})]),
        "scale": channel("Vec3", vec![0], vec![json!({"x": 2.0, "y": 2.0, "z": 1.0})]),
        "rotation_z": channel("Val", vec![0], vec![json!(std::f64::consts::FRAC_PI_2)])
    });
    root["layers"][0]["mesh"] = json!({
        "points": [
            {"position": channel("Vec2", vec![0], vec![json!({"x": 0.0, "y": 0.0})])},
            {"position": channel("Vec2", vec![0], vec![json!({"x": 0.5, "y": 0.0})])},
            {"position": channel("Vec2", vec![0], vec![json!({"x": 0.5, "y": -0.5})])},
            {"position": channel("Vec2", vec![0], vec![json!({"x": 0.0, "y": -0.5})])}
        ],
        "curves": [{"closed": true, "points": [
            {"point": 0, "smoothness": channel("Val", vec![0], vec![json!(0.0)])},
            {"point": 1, "smoothness": channel("Val", vec![0], vec![json!(0.0)])},
            {"point": 2, "smoothness": channel("Val", vec![0], vec![json!(0.0)])},
            {"point": 3, "smoothness": channel("Val", vec![0], vec![json!(0.0)])}
        ]}],
        "shapes": [{"has_fill": true, "has_outline": false, "edges": {"curve": [0, 0, 0, 0], "segment": [0, 1, 2, 3], "flag": [0, 0, 0, 0]},
            "style": {"fill_color": channel("Color", vec![0], vec![json!({"r": 1.0, "g": 0.0, "b": 0.0, "a": 1.0})])}}]
    });
    let (doc, _) = import(&root);
    let layer = &doc.timeline(0).layers[0];
    let el = &layer.keyframes[0].elements[0];
    // Layer (0,0) inch lands at (640 + 320, 360) px, and the shape rotates with the layer.
    let origin = el.transform.apply(anicraft_geom::Vec2::ZERO);
    assert!((origin.x - 960.0).abs() < 1e-3 && (origin.y - 360.0).abs() < 1e-3, "origin+translation, y up → y down");
    // The shape's 0.5-inch x axis (160 px) turns upward (y flips with the rotation)…
    let along = el.transform.apply(anicraft_geom::Vec2::new(0.5, 0.0)) - origin;
    assert!(along.x.abs() < 1e-3 && (along.y - (-320.0)).abs() < 1e-3, "rotation π/2 maps +x to +y (up = −y on screen), 2× scale");
}

#[test]
fn gradients_and_strokes_become_paints() {
    let mut root = minimal();
    root["layers"][0]["mesh"] = json!({
        "points": [
            {"position": channel("Vec2", vec![0], vec![json!({"x": -1.0, "y": 1.0})])},
            {"position": channel("Vec2", vec![0], vec![json!({"x": 1.0, "y": -1.0})])}
        ],
        "curves": [
            {"closed": true, "points": [
                {"point": 0, "smoothness": channel("Val", vec![0], vec![json!(1.0)])},
                {"point": 1, "smoothness": channel("Val", vec![0], vec![json!(1.0)])}
            ]},
            {"closed": false, "points": [
                {"point": 0, "smoothness": channel("Val", vec![0], vec![json!(0.0)])},
                {"point": 1, "smoothness": channel("Val", vec![0], vec![json!(0.0)])}
            ]}
        ],
        "shapes": [
            {"has_fill": true, "has_outline": false, "edges": {"curve": [0, 0], "segment": [0, 1], "flag": [0, 0]},
             "style": {"fill_style_id": 9, "fill_style": {"type": "SS_Gradient2", "gradient_type": 0, "gradients": [
                {"location": channel("Val", vec![0], vec![json!(0.0)]), "color": channel("Color", vec![0], vec![json!({"r": 1.0, "g": 0.0, "b": 0.0, "a": 1.0})])},
                {"location": channel("Val", vec![0], vec![json!(1.0)]), "color": channel("Color", vec![0], vec![json!({"r": 0.0, "g": 0.0, "b": 1.0, "a": 1.0})])}
             ]}, "fill_color": channel("Color", vec![0], vec![json!({"r": 1.0, "g": 1.0, "b": 1.0, "a": 1.0})])}},
            {"has_fill": false, "has_outline": true, "edges": {"curve": [1], "segment": [0], "flag": [1]},
             "style": {"line_width": 0.00625, "line_color": channel("Color", vec![0], vec![json!({"r": 0.0, "g": 0.0, "b": 0.0, "a": 1.0})])}}
        ]
    });
    let (doc, report) = import(&root);
    let els = &doc.timeline(0).layers[0].keyframes[0].elements;
    assert_eq!(els.len(), 2, "one fill element, one stroke element");
    let anicraft_doc::ElementKind::Shape(fill) = &els[0].kind else { panic!("fill element") };
    match &fill.fill {
        Some(anicraft_doc::Paint::Linear { stops, .. }) => {
            assert_eq!(stops.len(), 2);
            assert_eq!(stops[0].color, anicraft_doc::Rgba::new(1.0, 0.0, 0.0, 1.0));
        }
        other => panic!("linear gradient expected, got {other:?}"),
    }
    let anicraft_doc::ElementKind::Shape(line) = &els[1].kind else { panic!("stroke element") };
    let stroke = line.stroke.as_ref().unwrap();
    assert!((stroke.width - 0.00625).abs() < 1e-9, "line width stays in document units; the transform carries the scale");
    assert!(report.skipped.contains_key("gradient geometry (approximated from shape bounds)"));
}

#[test]
fn unsupported_things_count_and_degrade() {
    let mut root = minimal();
    root["layers"] = json!([
        {"type": "BoneLayer", "name": "Rig", "skeleton": {"type": "Skeleton", "binding_mode": 1, "bones": [
            {"type": "Bone", "name": "hips", "parent": -1, "anim_angle": channel("Val", vec![0, 24], vec![json!(0.0), json!(1.57)])}
        ], "bones_groups": []}},
        {"type": "ParticleLayer", "name": "Dust"},
        {"type": "MeshLayer", "name": "L", "blend_mode": 2, "physics": {"enabled": true}, "layer_effects": {"blur": channel("Val", vec![0], vec![json!(0.25)])}, "mesh": {"points": [], "curves": [], "shapes": []}}
    ]);
    let (doc, report) = import(&root);
    assert_eq!(report.skipped.get("skeleton bones"), Some(&1));
    assert!(report.skipped.contains_key("particle layers"));
    assert!(report.skipped.contains_key("layer blur"));
    assert!(report.skipped.contains_key("layer blend modes"));
    // The bones layer still arrives, as a folder.
    let rig = doc.timeline(0).layers.iter().find(|l| l.name == "Rig").unwrap();
    assert_eq!(rig.kind, anicraft_doc::LayerKind::Folder);
}

#[test]
fn animated_transform_channels_import_and_sample() {
    let mut root = minimal();
    // Linear keys at frames 0/12/24, inches → pixels (320/in, y flipped) on the layer.
    root["layers"][0]["transforms"] = json!({
        "translation": channel("Vec3", vec![0, 12, 24], vec![json!({"x": 0.0, "y": 0.0, "z": 0.0}), json!({"x": 1.0, "y": -0.5, "z": 0.0}), json!({"x": 2.0, "y": 0.0, "z": 0.0})]),
        "rotation_z": channel("Val", vec![0, 24], vec![json!(0.0), json!(std::f64::consts::PI)])
    });
    // The camera zooms 2 → 3 over the range; a marker at frame 6.
    root["animated_values"] = json!({
        "camera_zoom": channel("Val", vec![0, 48], vec![json!(2.0), json!(3.0)]),
        "timeline_markers": channel("String", vec![-1000000, 6], vec![json!(""), json!("Kick")])
    });
    let (doc, report) = import(&root);
    assert!(report.skipped.is_empty(), "{:?}", report.skipped);
    let l = &doc.timeline(0).layers[0];
    // The layer carries the channels; the elements keep only the inch→pixel mapping.
    assert_eq!(l.translation.keys.iter().map(|k| k.val.x).collect::<Vec<_>>(), vec![0.0, 320.0, 640.0]);
    assert_eq!(l.translation.keys[1].val.y, 160.0, "y flips with the coordinate system");
    assert!((l.rotation.keys[1].val - -std::f32::consts::PI).abs() < 1e-5, "rotation flips sign: {:?}", l.rotation.keys[1].val);
    // Frame-by-frame sampling (the parity gate's in-repo half): linear segments exact.
    let x = |f: u32| l.translation.sample_or(f as f32, anicraft_geom::Vec2::ZERO).x;
    for f in 0..=24u32 {
        assert!((x(f) - 26.666668 * f as f32).abs() < 1e-4, "frame {f}: {}", x(f));
    }
    assert!((l.rotation.sample_or(12.0, 0.0) - -std::f32::consts::FRAC_PI_2).abs() < 1e-5);
    // The camera zooms 2 → 3, so the view scale runs 1 → 1.5.
    let cam = &doc.scenes[0].camera;
    assert!(!cam.is_neutral());
    assert!((cam.zoom.sample_or(0.0, 2.0) - 2.0).abs() < 1e-6);
    assert!((cam.zoom.sample_or(48.0, 2.0) - 3.0).abs() < 1e-6);
    assert!((cam.zoom.sample_or(24.0, 2.0) - 2.5).abs() < 1e-6);
    assert_eq!(doc.scenes[0].markers, vec![(6, "Kick".into())]);
}

#[test]
fn image_layers_find_their_file_or_report_it() {
    let mut root = minimal();
    root["layers"][0] = json!({"type": "ImageLayer", "name": "Photo", "fileref": {"path": "media/photo.png", "relativeTo": "MOHOProject"}});
    let read = |p: &str| -> Result<Option<Vec<u8>>, crate::MohoError> {
        if p == "media/photo.png" {
            // A 1×1 red PNG.
            let mut bytes: Vec<u8> = Vec::new();
            {
                let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
                encoder.set_color(png::ColorType::Rgba);
                encoder.set_depth(png::BitDepth::Eight);
                let mut writer = encoder.write_header().map_err(|e| crate::MohoError::NotMoho(e.to_string()))?;
                writer.write_image_data(&[255, 0, 0, 255]).map_err(|e| crate::MohoError::NotMoho(e.to_string()))?;
            }
            return Ok(Some(bytes));
        }
        Ok(None)
    };
    let (doc, report) = import_json(root.clone(), &read, None).unwrap();
    assert!(report.skipped.is_empty(), "the image is in the package");
    let bitmap = &doc.bitmaps[0];
    assert_eq!((bitmap.width, bitmap.height), (1, 1));
    let el = &doc.timeline(0).layers[0].keyframes[0].elements[0];
    let anicraft_doc::ElementKind::Shape(s) = &el.kind else { panic!("image element") };
    assert!(matches!(s.fill, Some(anicraft_doc::Paint::Bitmap { .. })));
    // Missing file: the layer stays, empty, and the report says what happened.
    root["layers"][0]["fileref"]["path"] = json!("media/gone.png");
    let read_none = |_: &str| -> Result<Option<Vec<u8>>, crate::MohoError> { Ok(None) };
    let (doc, report) = import_json(root, &read_none, None).unwrap();
    assert!(report.skipped.contains_key("missing image files"));
    assert!(doc.timeline(0).layers[0].keyframes[0].elements.is_empty());
}

#[test]
fn containers_and_garbage_fail_cleanly() {
    // Not a Moho document.
    let read = |_: &str| -> Result<Option<Vec<u8>>, crate::MohoError> { Ok(None) };
    assert!(import_json(json!({"mime_type": "application/json"}), &read, None).is_err());
    assert!(import_json(json!({"layers": []}), &read, None).is_err(), "Moho refuses empty layer lists");
    assert!(import_json(json!({"major_version": 2, "layers": [{}]}), &read, None).is_err());
    // Hostile values: NaN positions, out-of-range indices, deep nesting — reports, no panic.
    let mut hostile = minimal();
    hostile["layers"][0]["mesh"] = json!({
        "points": [{"position": channel("Vec2", vec![0], vec![json!(null)])}],
        "curves": [{"closed": false, "points": [{"point": 99}]}],
        "shapes": [{"has_fill": true, "edges": {"curve": [7], "segment": [3], "flag": [0]}}]
    });
    let (_, report) = import_json(hostile, &read, None).unwrap();
    assert!(!report.skipped.is_empty());
    // A deep layer nest stops at the depth cap.
    let mut deep = minimal();
    let mut layer = json!({"type": "MeshLayer", "name": "deepest", "mesh": {"points": [], "curves": [], "shapes": []}});
    for _ in 0..40 {
        layer = json!({"type": "GroupLayer", "name": "g", "layers": [layer]});
    }
    deep["layers"] = json!([layer]);
    let (_, report) = import_json(deep, &read, None).unwrap();
    assert!(report.skipped.contains_key("layers nested deeper than 16"));
}

#[test]
fn a_zip_package_reads_like_a_bare_file() {
    let project = serde_json::to_vec(&minimal()).unwrap();
    let zip = zip::tests::make_zip(&[("Project.mohoproj", &project, true), ("preview.jpg", b"jpeg", false)]);
    let archive = zip::Archive::open(&zip).unwrap();
    let read = |p: &str| -> Result<Option<Vec<u8>>, crate::MohoError> { Ok(archive.read(p)?) };
    let (doc, _) = crate::import::import_with(&read).unwrap();
    assert_eq!(doc.stage.width, 1280);
    // The same bytes through the disk path (a temp .moho package).
    let path = std::env::temp_dir().join(format!("anicraft-moho-test-{}.moho", std::process::id()));
    std::fs::write(&path, &zip).unwrap();
    let (doc2, _) = crate::import_path(&path).unwrap();
    std::fs::remove_file(&path).ok();
    assert_eq!(doc, doc2);
}
