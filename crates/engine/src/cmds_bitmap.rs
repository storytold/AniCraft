//! Bitmap assets imported into the Library and optionally placed on the current keyframe.

use crate::params::*;
use crate::{CommandSpec, EngineError, Result, Session};
use anicraft_doc::{Bitmap, ColorEffect, Element, ElementId, ElementKind, Layer, LayerId, LoopMode, Paint, Shape, Symbol, SymbolId, SymbolKind, Timeline};
use anicraft_geom::{Affine, Path};
use serde_json::{Value, json};
use std::io::Read;
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "file.import_bitmap", label: "Import Bitmap…", menu: "File/Import", shortcut: Some("Ctrl+R"),
        params: "{path, name?, place?: true, x?, y?} (PNG/JPEG; x/y are the image centre)", mutates: true, run: import,
    }]
}

/// May run off the UI thread. The source is never retained as an external file dependency.
pub fn read(path: &FsPath) -> Result<Arc<Bitmap>> {
    let file = std::fs::File::open(path)?;
    if file.metadata()?.len() > anicraft_bitmap::MAX_INPUT_BYTES as u64 {
        return Err(EngineError::Param("bitmap file exceeds the 80 MiB input limit".into()));
    }
    let mut bytes = Vec::new();
    file.take(anicraft_bitmap::MAX_INPUT_BYTES as u64 + 1).read_to_end(&mut bytes)?;
    anicraft_bitmap::decode(&bytes).map(Arc::new).map_err(EngineError::Format)
}

/// One prepared decode passed from the shell to the regular command; consumed once.
pub struct Prepared {
    pub path: PathBuf,
    pub image: Arc<Bitmap>,
}

pub(crate) fn validate_document(doc: &anicraft_doc::Document) -> Result<()> {
    let images = anicraft_doc::bitmap_table::build_table(doc);
    if images.len() > 4096 || images.iter().map(|b| b.bytes()).sum::<usize>() > 256 * 1024 * 1024 {
        return Err(EngineError::State("document bitmap assets exceed 256 MiB or 4096 images".into()));
    }
    Ok(())
}

fn import(s: &mut Session, p: &Value) -> Result<Value> {
    let path = PathBuf::from(str_req(p, "path")?);
    let name = opt_str(p, "name")?.map(str::to_string).unwrap_or_else(|| path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "Bitmap".into()));
    if name.trim().is_empty() || name.len() > 4096 { return Err(EngineError::Param("bitmap name must contain 1–4096 bytes".into())) }
    let place = opt_bool(p, "place")?.unwrap_or(true);
    let x = opt_f32(p, "x")?.unwrap_or(s.doc.stage.width as f32 * 0.5);
    let y = opt_f32(p, "y")?.unwrap_or(s.doc.stage.height as f32 * 0.5);
    if place { s.editable_keyframe()?; }
    let image = match s.prepared_bitmap.take().filter(|b| b.path == path) {
        Some(prepared) => prepared.image,
        None => read(&path)?,
    };
    let images = anicraft_doc::bitmap_table::build_table(&s.doc);
    // Re-importing identical pixels reuses the allocation, including when filenames differ.
    let image = images.iter().find(|b| ***b == *image).cloned().unwrap_or(image);
    if images.iter().all(|b| !Arc::ptr_eq(b, &image)) && (images.len() >= 4096 || images.iter().map(|b| b.bytes()).sum::<usize>() + image.bytes() > 256 * 1024 * 1024) {
        return Err(EngineError::State("document bitmap assets exceed 256 MiB or 4096 images".into()));
    }
    let (w, h) = (image.width(), image.height());
    let sid = SymbolId(s.doc.alloc_id());
    let mut layer = Layer::new(LayerId(s.doc.alloc_id()), "Bitmap");
    let child = Element {
        id: ElementId(s.doc.alloc_id()), name: name.clone(), transform: Affine::IDENTITY, color: ColorEffect::default(),
        kind: ElementKind::Shape(Shape::filled(Path::rect(-0.5 * w as f32, -0.5 * h as f32, w as f32, h as f32), Paint::Bitmap { image, transform: Affine::translate(-0.5 * w as f32, -0.5 * h as f32) })),
    };
    if let Some(k) = layer.keyframe_mut(0) { k.elements.push(Arc::new(child)); }
    s.doc.library.push(Arc::new(Symbol { id: sid, name: name.clone(), kind: SymbolKind::Bitmap, timeline: Timeline { layers: vec![Arc::new(layer)] } }));
    let instance = if place {
        let id = ElementId(s.doc.alloc_id());
        let el = Element { id, name: name.clone(), transform: Affine::translate(x, y), color: ColorEffect::default(), kind: ElementKind::Symbol { symbol: sid, loop_mode: LoopMode::SingleFrame, first_frame: 0 } };
        let kf = s.editable_keyframe()?;
        if kf.elements.len() >= 100_000 { return Err(EngineError::State("too many elements on this keyframe".into())) }
        kf.elements.push(Arc::new(el));
        s.selection = vec![id];
        Some(id.0)
    } else { None };
    Ok(json!({"symbol": sid.0, "element": instance, "name": name, "width": w, "height": h}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use anicraft_render::{View, render};

    fn image_path(name: &str, rgba: Vec<u8>) -> PathBuf {
        let path = std::env::temp_dir().join(format!("anicraft-bitmap-{name}-{}.png", std::process::id()));
        std::fs::write(&path, Bitmap::new(2, 1, rgba).unwrap().encode_png().unwrap()).unwrap();
        path
    }
    fn pixels(s: &Session) -> Vec<u8> {
        render(&anicraft_doc::evaluate(&s.doc, s.frame), &View::stage(&s.doc, 1.0)).unwrap().0.to_rgba8()
    }
    fn session() -> Session {
        let mut doc = anicraft_doc::Document::default();
        doc.stage.width = 8;
        doc.stage.height = 8;
        doc.stage.background = anicraft_doc::Rgba::TRANSPARENT;
        Session::new(doc)
    }
    fn first_image(doc: &anicraft_doc::Document) -> Arc<Bitmap> {
        anicraft_doc::bitmap_table::build_table(doc).into_iter().next().unwrap()
    }

    #[test]
    fn import_place_transform_undo_and_embedded_round_trip() {
        let path = image_path("round-trip", vec![255, 0, 0, 255, 0, 255, 0, 128]);
        let mut s = session();
        let before = s.doc.clone();
        let v = s.execute("file.import_bitmap", &json!({"path": path, "x": 2, "y": 2.5})).unwrap();
        assert_eq!(s.history().undo_len(), 1, "library asset and stage instance are one edit");
        assert_eq!(s.doc.library.len(), 1);
        assert_eq!(s.doc.library[0].kind, SymbolKind::Bitmap);
        let px = pixels(&s);
        assert_eq!(&px[(2 * 8 + 1) * 4..(2 * 8 + 2) * 4], &[255, 0, 0, 255]);
        assert_eq!(&px[(2 * 8 + 2) * 4..(2 * 8 + 3) * 4], &[0, 255, 0, 128]);
        assert_eq!(s.hit_test(anicraft_geom::Vec2::new(1.5, 2.5), 0.0).unwrap().1.0, v["element"].as_u64().unwrap());
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(s.doc, before);
        s.execute("edit.redo", &json!({})).unwrap();
        s.execute("modify.rotate_cw", &json!({})).unwrap();
        s.execute("timeline.insert_keyframe", &json!({"frame": 3})).unwrap();
        s.execute("edit.duplicate", &json!({})).unwrap();
        let rendered = pixels(&s);
        let text = crate::format::to_json(&s.doc).unwrap();
        let env: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(env["version"], 3);
        assert_eq!(env["bitmaps"].as_array().unwrap().len(), 1, "embed once despite duplicate instances and keyframes");
        std::fs::remove_file(&path).unwrap();
        let restored = crate::format::from_json(&text).unwrap();
        assert_eq!(s.doc, restored);
        let mut back = Session::new(restored);
        back.set_frame(s.frame);
        assert_eq!(pixels(&back), rendered, "source file is not required after save");
        assert!(Arc::ptr_eq(&first_image(&s.doc), &first_image(&s.doc.clone())));
        let svg = crate::cmds_file::frame_svg(&s.doc, s.frame);
        assert!(svg.contains("data:image/png;base64,"));
        assert_eq!(svg.matches("data:image/png;base64,").count(), 1, "SVG reuses shared images across instances");
        assert!(svg.contains("patternTransform") && svg.contains("clipPath"));
        // Breaking apart keeps both the pixels and the shared image allocation.
        let image = first_image(&s.doc);
        s.execute("modify.break_apart", &json!({})).unwrap();
        assert_eq!(pixels(&s), rendered);
        assert!(anicraft_doc::bitmap_table::build_table(&s.doc).iter().all(|b| Arc::ptr_eq(b, &image)));
        let back = crate::format::from_json(&crate::format::to_json(&s.doc).unwrap()).unwrap();
        assert_eq!(anicraft_doc::bitmap_table::build_table(&back).len(), 1, "load restores sharing between bitmap fills and Library artwork");
    }

    #[test]
    fn clipboard_remaps_colliding_bitmap_symbols_and_carries_pixels() {
        let red = image_path("red", vec![255, 0, 0, 255, 255, 0, 0, 255]);
        let green = image_path("green", vec![0, 255, 0, 255, 0, 255, 0, 255]);
        let mut a = session();
        a.execute("file.import_bitmap", &json!({"path": red, "x": 2, "y": 2.5})).unwrap();
        a.execute("edit.copy", &json!({})).unwrap();
        a.execute("timeline.copy_frames", &json!({"from": 0, "to": 0})).unwrap();
        let mut b = session();
        b.execute("file.import_bitmap", &json!({"path": green, "x": 6, "y": 2.5})).unwrap();
        b.clipboard = a.clipboard.clone();
        b.frame_clipboard = a.frame_clipboard.clone();
        assert_eq!(a.doc.library[0].id, b.doc.library[0].id, "exercise id collision");
        b.execute("edit.paste_in_place", &json!({})).unwrap();
        assert_eq!(b.doc.library.len(), 2);
        let px = pixels(&b);
        assert_eq!(&px[(2 * 8 + 1) * 4..(2 * 8 + 2) * 4], &[255, 0, 0, 255]);
        assert_eq!(&px[(2 * 8 + 5) * 4..(2 * 8 + 6) * 4], &[0, 255, 0, 255]);
        b.execute("timeline.paste_frames", &json!({"frame": 1})).unwrap();
        b.set_frame(1);
        let px = pixels(&b);
        assert_eq!(&px[(2 * 8 + 1) * 4..(2 * 8 + 2) * 4], &[255, 0, 0, 255]);
        std::fs::remove_file(red).unwrap();
        std::fs::remove_file(green).unwrap();
    }

    #[test]
    fn library_only_locked_layers_bad_input_and_resource_sharing() {
        let path = image_path("library", vec![255; 8]);
        let mut s = session();
        s.execute("layer.toggle_lock", &json!({})).unwrap();
        let before = s.doc.clone();
        assert!(s.execute("file.import_bitmap", &json!({"path": path})).is_err());
        assert_eq!(s.doc, before);
        s.execute("file.import_bitmap", &json!({"path": path, "place": false})).unwrap();
        assert!(s.visible_elements().is_empty());
        s.execute("file.import_bitmap", &json!({"path": path, "place": false})).unwrap();
        let (a, b) = (&s.doc.library[0], &s.doc.library[1]);
        let image_of = |sym: &Symbol| {
            let ElementKind::Shape(sh) = &sym.timeline.layers[0].keyframes[0].elements[0].kind else { panic!() };
            let Some(Paint::Bitmap { image, .. }) = &sh.fill else { panic!() }; image.clone()
        };
        assert!(Arc::ptr_eq(&image_of(a), &image_of(b)), "identical decoded pixels are shared");
        let symbol = a.id.0;
        s.execute("layer.toggle_lock", &json!({})).unwrap();
        s.execute("library.place", &json!({"symbol": symbol})).unwrap();
        let image = first_image(&s.doc);
        let bytes = s.history().bytes();
        s.execute("element.translate", &json!({"dx": 1, "dy": 1})).unwrap();
        assert!(Arc::ptr_eq(&image, &first_image(&s.doc)));
        assert!(s.history().bytes() - bytes < 4096, "moving an instance does not copy its pixels");
        for bad in [Value::Null, json!({"path": 7}), json!({"path": path, "place": "yes"}), json!({"path": path, "x": 1e300}), json!({"path": path, "name": ""})] {
            let before = s.doc.clone();
            assert!(s.execute("file.import_bitmap", &bad).is_err());
            assert_eq!(s.doc, before);
        }
        std::fs::write(&path, b"not an image").unwrap();
        assert!(s.execute("file.import_bitmap", &json!({"path": path})).is_err());
        std::fs::remove_file(path).unwrap();
    }
}
