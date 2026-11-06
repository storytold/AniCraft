//! Fixed-grid sprite sheets. One RGBA8 atlas and one rendered frame are held at a time.

use crate::params::{opt_bool, opt_u32, str_req};
use crate::{EngineError, Result, Session};
use anicraft_doc::Document;
use anicraft_render::{MAX_PIXELS, View, encode_png, render};
use serde_json::{Value, json};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const MAX_EDGE: u32 = 8192;
pub const MAX_FRAMES: u32 = 10_000;

/// Validated layout, also used for the desktop's pre-export size preview.
#[derive(Clone, Debug)]
pub struct Layout {
    pub from: u32,
    pub to: u32,
    pub columns: u32,
    pub rows: u32,
    pub width: u32,
    pub height: u32,
    pub frame_width: u32,
    pub frame_height: u32,
    view: View,
}

impl Layout {
    pub fn new(doc: &Document, p: &Value) -> Result<Self> {
        let count = doc.timeline.frame_count();
        let from = opt_u32(p, "from")?.unwrap_or(0);
        let to = opt_u32(p, "to")?.unwrap_or(count.saturating_sub(1));
        if from > to || to >= count {
            return Err(EngineError::Param(format!("frame range must be within 0..{} with 'from' <= 'to'", count.saturating_sub(1))));
        }
        let n = to - from + 1;
        if n > MAX_FRAMES {
            return Err(EngineError::Param(format!("sprite sheets are limited to {MAX_FRAMES} frames; shorten the range")));
        }
        let mut view = View::stage(doc, crate::cmds_file::scale_param(p)?);
        if opt_bool(p, "transparent")?.unwrap_or(true) {
            view.background = None;
        }
        let (w, h) = (view.width, view.height);
        let too_large = || EngineError::Param(format!("sprite sheet exceeds {MAX_EDGE} px per side or {MAX_PIXELS} pixels; lower 'scale' or shorten the frame range"));
        if w > MAX_EDGE || h > MAX_EDGE || u64::from(w) * u64::from(h) * u64::from(n) > MAX_PIXELS {
            return Err(too_large());
        }
        let fits = |c: u32| {
            let width = u64::from(c) * u64::from(w);
            let height = u64::from(n.div_ceil(c)) * u64::from(h);
            width <= u64::from(MAX_EDGE) && height <= u64::from(MAX_EDGE) && width * height <= MAX_PIXELS
        };
        let columns = match opt_u32(p, "columns")? {
            Some(c) if c == 0 || c > n => return Err(EngineError::Param(format!("'columns' must be between 1 and {n} (omit for automatic)"))),
            Some(c) if fits(c) => c,
            Some(_) => return Err(too_large()),
            // Choose the most nearly square valid sheet; prefer less unused space on ties.
            None => (1..=n)
                .filter(|&c| fits(c))
                .min_by_key(|&c| {
                    let (width, height) = (c * w, n.div_ceil(c) * h);
                    (width.max(height), u64::from(width) * u64::from(height))
                })
                .ok_or_else(too_large)?,
        };
        let rows = n.div_ceil(columns);
        Ok(Self { from, to, columns, rows, width: columns * w, height: rows * h, frame_width: w, frame_height: h, view })
    }
}

/// The chosen PNG and its sibling JSON. A fixed suffix prevents the outputs colliding.
pub fn output_paths(p: &Value) -> Result<(PathBuf, PathBuf)> {
    let png = PathBuf::from(str_req(p, "path")?);
    if !png.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")) || png.file_stem().is_none() {
        return Err(EngineError::Param("sprite sheet 'path' must end in .png; a sibling .json is also written".into()));
    }
    let json = png.with_extension("json");
    Ok((png, json))
}

pub(crate) fn export(s: &mut Session, p: &Value) -> Result<Value> {
    let (png_path, json_path) = output_paths(p)?;
    let layout = Layout::new(&s.doc, p)?;
    // Refuse invalid output targets before doing any rendering.
    for path in [&png_path, &json_path] {
        if path.is_dir() {
            return Err(EngineError::Param(format!("{} is a directory; choose a file name", path.display())));
        }
    }
    let byte_count = layout.width as usize * layout.height as usize * 4;
    let mut atlas = Vec::new();
    atlas.try_reserve_exact(byte_count).map_err(|_| EngineError::State("not enough memory for the sprite sheet; lower 'scale' or shorten the range".into()))?;
    atlas.resize(byte_count, 0u8);
    let mut frames = Vec::new();
    let stride = layout.width as usize * 4;
    let row_bytes = layout.frame_width as usize * 4;
    for (i, f) in (layout.from..=layout.to).enumerate() {
        let x = (i as u32 % layout.columns) * layout.frame_width;
        let y = (i as u32 / layout.columns) * layout.frame_height;
        let (pm, _) = render(&anicraft_doc::evaluate(&s.doc, f), &layout.view)?;
        let rgba = pm.to_rgba8();
        for (row, source) in rgba.chunks_exact(row_bytes).enumerate() {
            let start = (y as usize + row) * stride + x as usize * 4;
            let dest = atlas.get_mut(start..start + row_bytes).ok_or_else(|| EngineError::State("sprite sheet row is outside the validated layout".into()))?;
            dest.copy_from_slice(source);
        }
        frames.push(json!({"frame": f, "rect": {"x": x, "y": y, "w": layout.frame_width, "h": layout.frame_height}, "duration_ms": 1000.0 / f64::from(s.doc.stage.fps)}));
    }
    let metadata = json!({
        "format": "anicraft-sprite-sheet", "version": 1,
        "image": png_path.file_name().and_then(|s| s.to_str()).ok_or_else(|| EngineError::Param("PNG file name must be valid UTF-8".into()))?,
        "size": {"w": layout.width, "h": layout.height},
        "columns": layout.columns, "rows": layout.rows,
        "fps": s.doc.stage.fps, "frames": frames
    });
    let metadata = serde_json::to_vec_pretty(&metadata).map_err(|e| EngineError::Format(e.to_string()))?;
    let png = encode_png(layout.width, layout.height, &atlas)?;
    // Stage both files before replacing either destination. Each rename is atomic; a pair
    // of filesystem entries cannot be committed atomically across an OS interruption.
    let staged_png = Staged::write(&png_path, &png)?;
    let staged_json = Staged::write(&json_path, &metadata)?;
    std::fs::rename(&staged_png.0, &png_path)?;
    std::fs::rename(&staged_json.0, &json_path)?;
    Ok(json!({"written": png_path.display().to_string(), "json": json_path.display().to_string(), "frames": layout.to - layout.from + 1, "width": layout.width, "height": layout.height, "columns": layout.columns, "rows": layout.rows}))
}

/// Unique staging files never overwrite user files and are removed on every error path.
struct Staged(PathBuf);

impl Staged {
    fn write(path: &Path, bytes: &[u8]) -> Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let name = path.file_name().ok_or_else(|| EngineError::Param("missing export file name".into()))?;
        let mut temporary = name.to_os_string();
        temporary.push(format!(".anicraft-{}-{}.tmp", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        let temporary = path.with_file_name(temporary);
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&temporary)?;
        let staged = Self(temporary);
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(staged)
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directory(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("anicraft-sprites-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn sheets_match_oracle_frames_and_keep_timing_and_unused_cells() {
        let dir = directory("pixels");
        let mut s = Session::new(anicraft_doc::sample::demo());
        s.doc.stage.fps = 29.97;
        s.set_frame(12);
        let before = s.doc.clone();
        for transparent in [true, false] {
            let path = dir.join(format!("frames-{transparent}.png"));
            let params = json!({"path": path, "from": 5, "to": 9, "columns": 3, "scale": 0.025, "transparent": transparent});
            let result = s.execute("file.export_sprite_sheet", &params).unwrap();
            assert_eq!(result["frames"], 5);
            let metadata: Value = serde_json::from_slice(&std::fs::read(path.with_extension("json")).unwrap()).unwrap();
            assert_eq!(metadata["format"], "anicraft-sprite-sheet");
            assert_eq!(metadata["version"], 1);
            assert_eq!(metadata["image"], path.file_name().unwrap().to_str().unwrap());
            assert_eq!(metadata["columns"], 3);
            assert_eq!(metadata["rows"], 2);
            let mut reader = png::Decoder::new(std::io::Cursor::new(std::fs::read(&path).unwrap())).read_info().unwrap();
            let mut rgba = vec![0; reader.output_buffer_size()];
            let info = reader.next_frame(&mut rgba).unwrap();
            assert_eq!((info.width, info.height), (96, 36));
            assert_eq!(info.color_type, png::ColorType::Rgba);
            assert_eq!(metadata["size"], json!({"w": 96, "h": 36}));
            let mut view = View::stage(&s.doc, 0.025);
            if transparent { view.background = None; }
            for (i, entry) in metadata["frames"].as_array().unwrap().iter().enumerate() {
                let (x, y) = ((i % 3) * 32, (i / 3) * 18);
                assert_eq!(entry["frame"], i + 5);
                assert_eq!(entry["rect"], json!({"x": x, "y": y, "w": 32, "h": 18}));
                assert!((entry["duration_ms"].as_f64().unwrap() - 1000.0 / f64::from(s.doc.stage.fps)).abs() < 1e-9);
                let expected = render(&anicraft_doc::evaluate(&s.doc, i as u32 + 5), &view).unwrap().0.to_rgba8();
                for row in 0..18 {
                    let start = ((y + row) * 96 + x) * 4;
                    assert_eq!(&rgba[start..start + 128], &expected[row * 128..(row + 1) * 128]);
                }
            }
            // The last grid cell is unused, even when the stage background is included.
            for row in 18..36 {
                assert!(rgba[(row * 96 + 64) * 4..(row * 96 + 96) * 4].iter().all(|&b| b == 0));
            }
        }
        assert_eq!(s.doc, before);
        assert_eq!(s.frame, 12);
        assert!(!s.is_dirty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn transparent_export_keeps_straight_alpha_and_can_include_stage_background() {
        let dir = directory("alpha");
        let path = dir.join("alpha.png");
        let mut s = Session::default();
        s.execute("doc.set_stage", &json!({"width": 8, "height": 8, "background": "#0000ff"})).unwrap();
        s.execute("draw.rect", &json!({"x": 2, "y": 2, "w": 4, "h": 4, "fill": "#ff000080", "stroke": null})).unwrap();
        for transparent in [true, false] {
            // The first export omits 'transparent' to exercise the public default.
            let mut params = json!({"path": path, "confirm": true});
            if !transparent { params["transparent"] = json!(false); }
            s.execute("file.export_sprite_sheet", &params).unwrap();
            let mut reader = png::Decoder::new(std::io::Cursor::new(std::fs::read(&path).unwrap())).read_info().unwrap();
            let mut rgba = vec![0; reader.output_buffer_size()];
            reader.next_frame(&mut rgba).unwrap();
            let centre = (3 * 8 + 3) * 4;
            if transparent {
                assert_eq!(&rgba[..4], &[0, 0, 0, 0]);
                assert_eq!(&rgba[centre..centre + 4], &[255, 0, 0, 128], "PNG uses straight alpha");
            } else {
                assert_eq!(&rgba[..4], &[0, 0, 255, 255]);
                assert_eq!(&rgba[centre..centre + 4], &[128, 0, 127, 255]);
            }
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn either_existing_file_requires_confirmation_and_invalid_targets_preserve_files() {
        let dir = directory("overwrite");
        let path = dir.join("sprites.PNG");
        let metadata = path.with_extension("json");
        let mut s = Session::default();
        let mut params = json!({"path": path, "scale": 0.01});
        std::fs::write(&metadata, "old metadata").unwrap();
        assert!(s.confirmation("file.export_sprite_sheet", &params).unwrap().irreversible);
        assert!(s.execute("file.export_sprite_sheet", &params).is_err());
        assert!(!path.exists());
        assert_eq!(std::fs::read_to_string(&metadata).unwrap(), "old metadata");
        params["confirm"] = json!(true);
        s.execute("file.export_sprite_sheet", &params).unwrap();
        let png = std::fs::read(&path).unwrap();
        std::fs::remove_file(&metadata).unwrap();
        params["confirm"] = json!(false);
        assert!(s.execute("file.export_sprite_sheet", &params).is_err(), "PNG alone also requires consent");
        std::fs::create_dir(&metadata).unwrap();
        params["confirm"] = json!(true);
        assert!(s.execute("file.export_sprite_sheet", &params).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), png);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2, "no staging files remain");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn validates_hostile_parameters_and_bounds_before_writing() {
        let dir = directory("validation");
        let mut s = Session::new(anicraft_doc::sample::demo());
        let path = dir.join("invalid.png");
        for bad in [
            json!({"from": 10, "to": 9}), json!({"to": 48}), json!({"from": -1}),
            json!({"to": u64::MAX}), json!({"columns": 0}), json!({"columns": 49}),
            json!({"columns": "3"}), json!({"columns": u32::MAX}),
            json!({"scale": 0}), json!({"scale": 9}), json!({"scale": 1e300}),
            json!({"transparent": "yes"}), json!({"scale": 1}),
        ] {
            let mut params = json!({"path": path, "scale": 0.01});
            params.as_object_mut().unwrap().extend(bad.as_object().unwrap().clone());
            assert!(s.execute("file.export_sprite_sheet", &params).is_err(), "{params}");
        }
        for bad in [Value::Null, json!([]), json!({"path": 7}), json!({"path": dir.join("same.json")})] {
            assert!(s.execute("file.export_sprite_sheet", &bad).is_err());
        }
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        s.execute("timeline.insert_frame", &json!({"frame": 0, "count": MAX_FRAMES})).unwrap();
        assert!(Layout::new(&s.doc, &json!({"scale": 0.001})).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn automatic_layout_fits_tall_frames_and_partial_rows() {
        let mut doc = Document::default();
        doc.stage.width = 1;
        doc.stage.height = MAX_EDGE;
        let mut s = Session::new(doc);
        s.execute("timeline.insert_frame", &json!({"frame": 0, "count": 2})).unwrap();
        let layout = Layout::new(&s.doc, &json!({})).unwrap();
        assert_eq!((layout.columns, layout.rows, layout.width, layout.height), (3, 1, 3, MAX_EDGE));
        assert!(Layout::new(&s.doc, &json!({"columns": 1})).is_err());
    }
}
