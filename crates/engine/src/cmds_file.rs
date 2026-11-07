//! File, edit-history and document commands.

use crate::params::*;
use crate::{CommandSpec, EngineError, Result, Session, format};
use anicraft_doc::{Document, MAX_STAGE_EDGE, Stage, sample};
use anicraft_render::{View, render};
use rayon::prelude::*;
use serde_json::{Value, json};
use std::path::PathBuf;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec { id: "file.new", label: "New", menu: "File", shortcut: Some("Ctrl+N"), params: "{width?, height?, fps?}", mutates: false, run: new_doc },
        CommandSpec { id: "file.new_sample", label: "New from Sample", menu: "File", shortcut: None, params: "{}", mutates: false, run: new_sample },
        CommandSpec { id: "file.open", label: "Open…", menu: "File", shortcut: Some("Ctrl+O"), params: "{path}", mutates: false, run: open },
        CommandSpec { id: "file.save", label: "Save", menu: "File", shortcut: Some("Ctrl+S"), params: "{path?, confirm?}", mutates: false, run: save },
        CommandSpec {
            id: "file.save_as",
            label: "Save As…",
            menu: "File",
            shortcut: Some("Ctrl+Shift+S"),
            params: "{path, confirm?}",
            mutates: false,
            run: |s, p| {
                str_req(p, "path")?;
                save(s, p)
            },
        },
        CommandSpec { id: "file.export_png", label: "Export Frame as PNG…", menu: "File/Export", shortcut: None, params: "{path, frame? (in the current scene), scale?}", mutates: false, run: export_png },
        CommandSpec {
            id: "file.import",
            label: "Import to Stage…",
            menu: "File/Import",
            shortcut: Some("Ctrl+R"),
            params: "{path (.svg), x?, y? (top-left on the stage; default: centred)}",
            mutates: true,
            run: import_to_stage,
        },
        CommandSpec { id: "file.export_svg", label: "Export Frame as SVG…", menu: "File/Export", shortcut: None, params: "{path, frame? (in the current scene), confirm?}", mutates: false, run: export_svg },
        CommandSpec {
            id: "file.export_video",
            label: "Export Video…",
            menu: "File/Export",
            shortcut: None,
            params: "{path (.mp4 or .webm), from?, to? (movie frames), scale?, ffmpeg? (path; else ANICRAFT_FFMPEG, else ffmpeg on the PATH), confirm?}",
            mutates: false,
            run: export_video,
        },
        CommandSpec {
            id: "file.export_sprite_sheet",
            label: "Export Sprite Sheet…",
            menu: "File/Export",
            shortcut: None,
            params: "{path (.png; the frame map goes beside it as .json), from?, to? (movie frames), scale?, columns?, padding?, background?, confirm?}",
            mutates: false,
            run: export_sprite_sheet,
        },
        CommandSpec {
            id: "file.export_png_sequence",
            label: "Export PNG Sequence…",
            menu: "File/Export",
            shortcut: None,
            params: "{dir, from?, to? (movie frames: every scene in order), scale?}",
            mutates: false,
            run: export_sequence,
        },
        CommandSpec {
            id: "file.export_gif",
            label: "Export Animated GIF…",
            menu: "File/Export",
            shortcut: None,
            params: "{path, from?, to? (movie frames: every scene in order), scale?, loop?, confirm?}",
            mutates: false,
            run: export_gif,
        },
        CommandSpec { id: "edit.undo", label: "Undo", menu: "Edit", shortcut: Some("Ctrl+Z"), params: "{}", mutates: false, run: |s, _| s.undo() },
        CommandSpec { id: "edit.redo", label: "Redo", menu: "Edit", shortcut: Some("Ctrl+Y"), params: "{}", mutates: false, run: |s, _| s.redo() },
        CommandSpec {
            id: "doc.set_stage",
            label: "Document Settings",
            menu: "Modify",
            shortcut: None,
            params: "{width?, height?, fps?, background?: \"#rrggbb\"}",
            mutates: true,
            run: set_stage,
        },
        CommandSpec { id: "doc.info", label: "Document Info", menu: "", shortcut: None, params: "{}", mutates: false, run: |s, _| Ok(s.info()) },
    ]
}

fn stage_edge(p: &Value, key: &str, default: u32) -> Result<u32> {
    match opt_u32(p, key)? {
        None => Ok(default),
        Some(v) if (1..=MAX_STAGE_EDGE).contains(&v) => Ok(v),
        Some(_) => Err(EngineError::Param(format!("param '{key}' must be between 1 and {MAX_STAGE_EDGE}"))),
    }
}

fn fps(p: &Value, default: f32) -> Result<f32> {
    match opt_f32(p, "fps")? {
        None => Ok(default),
        Some(f) if (1.0..=240.0).contains(&f) => Ok(f),
        Some(_) => Err(EngineError::Param("param 'fps' must be between 1 and 240".into())),
    }
}

fn new_doc(s: &mut Session, p: &Value) -> Result<Value> {
    let d = Stage::default();
    let mut doc = Document::default();
    doc.stage.width = stage_edge(p, "width", d.width)?;
    doc.stage.height = stage_edge(p, "height", d.height)?;
    doc.stage.fps = fps(p, d.fps)?;
    s.replace_document(doc, None);
    Ok(s.info())
}

fn new_sample(s: &mut Session, _: &Value) -> Result<Value> {
    s.replace_document(sample::demo(), None);
    Ok(json!({"frames": s.frame_count()}))
}

/// Whether `path` is an Animate document: a `.fla` archive, an `.xfl` file or an XFL folder.
pub fn is_animate(path: &std::path::Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    matches!(ext.as_deref(), Some("fla" | "xfl")) || (path.is_dir() && path.join("DOMDocument.xml").is_file())
}

fn open(s: &mut Session, p: &Value) -> Result<Value> {
    let path = PathBuf::from(str_req(p, "path")?);
    if is_animate(&path) {
        // Imported, not adopted: the document is untitled, so Save asks for a new .anicraft
        // name and never writes over the Animate file.
        let (doc, report) = anicraft_xfl::import_path(&path).map_err(|e| EngineError::Format(format!("{}: {e}", path.display())))?;
        s.replace_document(doc, None);
        let mut info = s.info();
        info["imported"] = json!(path.display().to_string());
        info["report"] = json!(report.summary());
        info["skipped"] = json!(report.skipped);
        return Ok(info);
    }
    let doc = format::load(&path)?;
    s.replace_document(doc, Some(path));
    Ok(s.info())
}

fn save(s: &mut Session, p: &Value) -> Result<Value> {
    let path = match opt_str(p, "path")? {
        Some(x) => PathBuf::from(x),
        None => s.path.clone().ok_or_else(|| EngineError::Param("no file name yet: pass {\"path\": …}".into()))?,
    };
    format::save(&s.doc, &path)?;
    s.path = Some(path.clone());
    s.mark_saved();
    Ok(json!({"saved": path.display().to_string()}))
}

fn scale_param(p: &Value) -> Result<f32> {
    match opt_f32(p, "scale")? {
        None => Ok(1.0),
        Some(v) if v > 0.0 && v <= 8.0 => Ok(v),
        Some(_) => Err(EngineError::Param("param 'scale' must be in (0, 8]".into())),
    }
}

/// Renders movie frame `frame` (scenes in order) of `doc` to PNG bytes.
pub fn render_png(doc: &Document, frame: u32, scale: f32) -> Result<Vec<u8>> {
    let dl = anicraft_doc::evaluate(doc, frame);
    let (pm, _) = render(&dl, &View::stage(doc, scale))?;
    Ok(pm.encode_png()?)
}

fn export_png(s: &mut Session, p: &Value) -> Result<Value> {
    let path = PathBuf::from(str_req(p, "path")?);
    let frame = opt_u32(p, "frame")?.unwrap_or(s.frame);
    // `frame` is within the current scene; the renderer takes movie frames.
    let bytes = render_png(&s.doc, s.doc.scene_start(s.scene).saturating_add(frame), scale_param(p)?)?;
    std::fs::write(&path, &bytes)?;
    Ok(json!({"written": path.display().to_string(), "bytes": bytes.len(), "frame": frame}))
}

/// Largest file Import to Stage reads.
const MAX_IMPORT_BYTES: u64 = 256 * 1024 * 1024;

/// Import to Stage: the drawing's shapes become editable shapes on a new layer named after the
/// file, in a keyframe at the playhead, selected, as one undo step. Centred on the stage unless
/// `x`/`y` give its top-left corner.
fn import_to_stage(s: &mut Session, p: &Value) -> Result<Value> {
    let path = PathBuf::from(str_req(p, "path")?);
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    if !matches!(ext.as_deref(), Some("svg" | "png" | "jpg" | "jpeg")) {
        return Err(EngineError::Param("Import to Stage reads SVG drawings and PNG or JPEG images".into()));
    }
    let size = std::fs::metadata(&path)?.len();
    if size > MAX_IMPORT_BYTES {
        return Err(EngineError::Format(format!("{} is too large to import", path.display())));
    }
    let bytes = std::fs::read(&path)?;
    let drawing = if ext.as_deref() == Some("svg") {
        anicraft_svg::import(&bytes).map_err(|e| EngineError::Format(format!("{}: {e}", path.display())))?
    } else {
        import_image(s, &path, bytes)?
    };
    if drawing.shapes.is_empty() {
        return Err(EngineError::State(format!("{} has no shapes AniCraft can import yet. {}", path.display(), drawing.report.summary())));
    }
    let stage = s.doc.stage;
    let x = opt_f32(p, "x")?.unwrap_or((stage.width as f32 - drawing.width) * 0.5);
    let y = opt_f32(p, "y")?.unwrap_or((stage.height as f32 - drawing.height) * 0.5);
    if !(x.is_finite() && y.is_finite()) {
        return Err(EngineError::Param("'x' and 'y' must be numbers".into()));
    }
    let place = anicraft_geom::Affine::translate(x, y);
    let name: String = path.file_stem().map_or_else(|| "Imported".into(), |n| n.to_string_lossy().chars().take(128).collect());
    crate::cmds_timeline::new_layer(s, &json!({"name": name}))?;
    let frame = s.frame;
    let end = s.frame_count().max(frame + 1);
    let mut elements = Vec::with_capacity(drawing.shapes.len());
    for sh in drawing.shapes {
        let id = anicraft_doc::ElementId(s.doc.alloc_id());
        elements.push(std::sync::Arc::new(anicraft_doc::Element {
            id,
            name: sh.name,
            transform: place.mul(&sh.transform),
            color: anicraft_doc::ColorEffect { alpha: sh.alpha, ..Default::default() },
            kind: anicraft_doc::ElementKind::Shape(sh.shape),
        }));
    }
    let ids: Vec<anicraft_doc::ElementId> = elements.iter().map(|e| e.id).collect();
    let layer = s.layer_mut(None)?;
    // Empty before the playhead, the drawing from the playhead to the end of the timeline.
    let mut key = anicraft_doc::Keyframe::new(frame, end - frame);
    key.elements = elements;
    layer.keyframes = if frame == 0 { vec![std::sync::Arc::new(key)] } else { vec![std::sync::Arc::new(anicraft_doc::Keyframe::new(0, frame)), std::sync::Arc::new(key)] };
    layer.normalize();
    let layer_id = layer.id;
    s.selection = ids.clone();
    Ok(json!({
        "layer": layer_id.0,
        "elements": ids.len(),
        "width": drawing.width,
        "height": drawing.height,
        "report": drawing.report.summary(),
        "skipped": drawing.report.skipped,
    }))
}

/// A PNG or JPEG as a bitmap in the document, and the one shape that shows it: a rectangle of
/// the image's size filled with it. JPEGs are kept as PNG (lossless, so nothing degrades).
fn import_image(s: &mut Session, path: &std::path::Path, bytes: Vec<u8>) -> Result<anicraft_svg::SvgImport> {
    let unreadable = |why: &str| EngineError::Format(format!("{}: {why}", path.display()));
    let png = if bytes.starts_with(b"\x89PNG") {
        bytes
    } else {
        // JPEG → RGB(A) → PNG.
        let opts = zune_jpeg::zune_core::options::DecoderOptions::default().jpeg_set_out_colorspace(zune_jpeg::zune_core::colorspace::ColorSpace::RGBA);
        let mut dec = zune_jpeg::JpegDecoder::new_with_options(zune_jpeg::zune_core::bytestream::ZCursor::new(&bytes), opts);
        let rgba = dec.decode().map_err(|e| unreadable(&format!("not a readable JPEG ({e:?})")))?;
        let (w, h) = dec.dimensions().ok_or_else(|| unreadable("a JPEG without a size"))?;
        let (w, h) = (w as u32, h as u32);
        if w == 0 || h == 0 || w > anicraft_doc::MAX_BITMAP_EDGE || h > anicraft_doc::MAX_BITMAP_EDGE || rgba.len() != (w * h * 4) as usize {
            return Err(unreadable("the image is empty or larger than 8192 px a side"));
        }
        anicraft_render::encode_png(w, h, &rgba)?
    };
    let id = anicraft_doc::BitmapId(s.doc.alloc_id());
    let name: String = path.file_name().map_or_else(|| "image".into(), |n| n.to_string_lossy().chars().take(128).collect());
    let bitmap = anicraft_doc::Bitmap::from_png(id, name.clone(), png).ok_or_else(|| unreadable("not a readable PNG, or larger than 8192 px a side"))?;
    let (w, h) = (bitmap.width as f32, bitmap.height as f32);
    s.doc.bitmaps.push(std::sync::Arc::new(bitmap));
    let shape = anicraft_doc::Shape {
        path: std::sync::Arc::new(anicraft_geom::Path::rect(0.0, 0.0, w, h)),
        fill: Some(anicraft_doc::Paint::Bitmap { bitmap: id, repeat: false }),
        stroke: None,
        fill_rule: anicraft_geom::FillRule::NonZero,
    };
    Ok(anicraft_svg::SvgImport {
        width: w,
        height: h,
        shapes: vec![anicraft_svg::ImportedShape { name, transform: anicraft_geom::Affine::IDENTITY, alpha: 1.0, shape }],
        report: anicraft_svg::Report::default(),
    })
}

/// Movie frame `frame` as a standalone SVG document (vectors, gradients and masks kept).
pub fn frame_svg(doc: &Document, frame: u32) -> String {
    anicraft_render::svg::to_svg(&anicraft_doc::evaluate(doc, frame), &doc.stage)
}

fn export_svg(s: &mut Session, p: &Value) -> Result<Value> {
    let path = PathBuf::from(str_req(p, "path")?);
    let frame = opt_u32(p, "frame")?.unwrap_or(s.frame);
    if frame >= s.frame_count() {
        return Err(EngineError::Param(format!("frame {} is past the end (the timeline has {} frames)", frame + 1, s.frame_count())));
    }
    let svg = frame_svg(&s.doc, s.doc.scene_start(s.scene).saturating_add(frame));
    std::fs::write(&path, svg.as_bytes())?;
    Ok(json!({"written": path.display().to_string(), "bytes": svg.len(), "frame": frame}))
}

/// Export Sprite Sheet: movie frames in a grid on one PNG (transparent unless `background`),
/// and beside it a JSON frame map in the common "JSON hash" layout (frame rectangles, sizes
/// and per-frame durations) that game engines and tools read.
fn export_sprite_sheet(s: &mut Session, p: &Value) -> Result<Value> {
    let path = PathBuf::from(str_req(p, "path")?);
    let last = s.doc.frame_count().saturating_sub(1);
    let from = opt_u32(p, "from")?.unwrap_or(0).min(last);
    let to = opt_u32(p, "to")?.unwrap_or(last).min(last);
    if to < from {
        return Err(EngineError::Param("'to' must not be before 'from'".into()));
    }
    let n = (to - from + 1) as usize;
    let scale = scale_param(p)?;
    let pad = opt_u32(p, "padding")?.unwrap_or(0).min(256);
    let cols = match opt_u32(p, "columns")? {
        Some(0) => return Err(EngineError::Param("param 'columns' must be at least 1".into())),
        Some(c) => (c as usize).min(n),
        None => (n as f64).sqrt().ceil() as usize,
    }
    .max(1);
    let rows = n.div_ceil(cols);
    let mut view = View::stage(&s.doc, scale);
    if !opt_bool(p, "background")?.unwrap_or(false) {
        view.background = None;
    }
    let (fw, fh) = (view.width, view.height);
    let sheet_w = u64::from(fw) * cols as u64 + u64::from(pad) * (cols as u64 - 1);
    let sheet_h = u64::from(fh) * rows as u64 + u64::from(pad) * (rows as u64 - 1);
    if sheet_w * sheet_h > anicraft_render::MAX_PIXELS || sheet_w > u64::from(u32::MAX) || sheet_h > u64::from(u32::MAX) {
        return Err(EngineError::Param(format!(
            "that sheet would be {sheet_w}×{sheet_h} px (the limit is {} pixels); lower 'scale' or the frame range",
            anicraft_render::MAX_PIXELS
        )));
    }
    // Frames render in parallel, then go onto the sheet in order.
    let doc = &s.doc;
    let frames: Vec<anicraft_render::Pixmap> = (from..=to).into_par_iter().map(|f| render(&anicraft_doc::evaluate(doc, f), &view).map(|(pm, _)| pm)).collect::<std::result::Result<_, _>>()?;
    let mut sheet = anicraft_render::Pixmap::new(sheet_w as u32, sheet_h as u32)?;
    let stride = sheet_w as usize;
    let delays = gif_delays(n, doc.stage.fps);
    let stem = path.file_stem().map_or_else(|| "frame".to_string(), |x| x.to_string_lossy().into_owned());
    let mut map = serde_json::Map::new();
    for (k, pm) in frames.iter().enumerate() {
        let (x, y) = ((k % cols) as u32 * (fw + pad), (k / cols) as u32 * (fh + pad));
        for row in 0..fh as usize {
            let src = pm.data.get(row * fw as usize..(row + 1) * fw as usize);
            let at = (y as usize + row) * stride + x as usize;
            if let (Some(src), Some(dst)) = (src, sheet.data.get_mut(at..at + fw as usize)) {
                dst.copy_from_slice(src);
            }
        }
        let ms = u32::from(delays.get(k).copied().unwrap_or(4)) * 10;
        map.insert(
            format!("{stem}_{:04}", from as usize + k + 1),
            json!({
                "frame": {"x": x, "y": y, "w": fw, "h": fh},
                "rotated": false,
                "trimmed": false,
                "spriteSourceSize": {"x": 0, "y": 0, "w": fw, "h": fh},
                "sourceSize": {"w": fw, "h": fh},
                "duration": ms,
            }),
        );
    }
    let png = sheet.encode_png()?;
    let image = path.file_name().map_or_else(String::new, |x| x.to_string_lossy().into_owned());
    let atlas = json!({
        "frames": map,
        "meta": {
            "app": "AniCraft",
            "version": env!("CARGO_PKG_VERSION"),
            "image": image,
            "format": "RGBA8888",
            "size": {"w": sheet_w, "h": sheet_h},
            "scale": scale.to_string(),
            "frameRate": doc.stage.fps,
        }
    });
    let json_path = path.with_extension("json");
    let text = serde_json::to_string_pretty(&atlas).map_err(|e| EngineError::Format(e.to_string()))?;
    std::fs::write(&path, &png)?;
    std::fs::write(&json_path, text)?;
    Ok(json!({"written": path.display().to_string(), "map": json_path.display().to_string(), "frames": n, "columns": cols, "rows": rows, "width": sheet_w, "height": sheet_h}))
}

/// The ffmpeg to run: `ffmpeg` param, else `ANICRAFT_FFMPEG`, else `ffmpeg` on the PATH.
fn ffmpeg_program(p: &Value) -> Result<std::ffi::OsString> {
    if let Some(x) = opt_str(p, "ffmpeg")?.filter(|x| !x.trim().is_empty()) {
        return Ok(x.into());
    }
    Ok(std::env::var_os("ANICRAFT_FFMPEG").filter(|x| !x.is_empty()).unwrap_or_else(|| "ffmpeg".into()))
}

/// Export Video: movie frames streamed in order to the user's own ffmpeg (AniCraft bundles no
/// video encoder; see docs/legal/compliance.md). `.webm` is VP9 (royalty-free); `.mp4` is H.264
/// from whatever encoder that ffmpeg has. Frames render in parallel batches; odd sizes are
/// padded to even, as 4:2:0 video needs.
fn export_video(s: &mut Session, p: &Value) -> Result<Value> {
    use std::io::Write;
    let path = PathBuf::from(str_req(p, "path")?);
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    let codec: &[&str] = match ext.as_deref() {
        Some("webm") => &["-c:v", "libvpx-vp9", "-b:v", "0", "-crf", "30", "-row-mt", "1"],
        Some("mp4" | "m4v" | "mov") => &["-c:v", "libx264", "-crf", "18", "-preset", "medium", "-movflags", "+faststart"],
        _ => return Err(EngineError::Param("video goes to .mp4 or .webm".into())),
    };
    let last = s.doc.frame_count().saturating_sub(1);
    let from = opt_u32(p, "from")?.unwrap_or(0).min(last);
    let to = opt_u32(p, "to")?.unwrap_or(last).min(last);
    if to < from {
        return Err(EngineError::Param("'to' must not be before 'from'".into()));
    }
    let view = View::stage(&s.doc, scale_param(p)?);
    let (w, h, fps) = (view.width, view.height, s.doc.stage.fps);
    let program = ffmpeg_program(p)?;
    let size = format!("{w}x{h}");
    let rate = format!("{fps}");
    let mut args: Vec<&std::ffi::OsStr> = Vec::new();
    for a in ["-hide_banner", "-loglevel", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgba", "-s", &size, "-r", &rate, "-i", "-", "-an", "-vf", "pad=ceil(iw/2)*2:ceil(ih/2)*2", "-pix_fmt", "yuv420p"] {
        args.push(a.as_ref());
    }
    args.extend(codec.iter().map(std::ffi::OsStr::new));
    args.push(path.as_os_str());
    let mut child = std::process::Command::new(&program)
        .args(&args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| {
            EngineError::State(format!(
                "AniCraft makes video with ffmpeg, which could not be started ({e}). Install it from ffmpeg.org, or point ANICRAFT_FFMPEG at it."
            ))
        })?;
    // Read ffmpeg's messages as they come, so a full pipe never stalls it.
    let mut err_pipe = child.stderr.take();
    let messages = std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(e) = err_pipe.as_mut() {
            let _ = std::io::Read::read_to_string(e, &mut text);
        }
        text
    });
    let doc = &s.doc;
    let mut written = 0u32;
    let mut failure: Option<EngineError> = None;
    if let Some(mut stdin) = child.stdin.take() {
        let frames: Vec<u32> = (from..=to).collect();
        'batches: for batch in frames.chunks(8) {
            let images: Vec<Vec<u8>> = match batch.par_iter().map(|&f| render(&anicraft_doc::evaluate(doc, f), &view).map(|(pm, _)| pm.to_rgba8())).collect::<std::result::Result<_, _>>() {
                Ok(v) => v,
                Err(e) => {
                    failure = Some(e.into());
                    break;
                }
            };
            for img in images {
                if stdin.write_all(&img).is_err() {
                    break 'batches; // ffmpeg stopped reading: its message says why
                }
                written += 1;
            }
        }
    }
    let status = child.wait()?;
    let log = messages.join().unwrap_or_default();
    if let Some(e) = failure {
        return Err(e);
    }
    if !status.success() || written != to - from + 1 {
        let tail: Vec<&str> = log.lines().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
        return Err(EngineError::Format(format!("ffmpeg could not make the video ({status}): {}", if tail.is_empty() { "no message".to_string() } else { tail.join(" | ") })));
    }
    let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    Ok(json!({"written": path.display().to_string(), "frames": written, "bytes": bytes, "width": w, "height": h, "fps": fps}))
}

fn export_sequence(s: &mut Session, p: &Value) -> Result<Value> {
    let dir = PathBuf::from(str_req(p, "dir")?);
    // The whole movie: every scene in order.
    let last = s.doc.frame_count().saturating_sub(1);
    let from = opt_u32(p, "from")?.unwrap_or(0).min(last);
    let to = opt_u32(p, "to")?.unwrap_or(last).min(last);
    if to < from {
        return Err(EngineError::Param("'to' must not be before 'from'".into()));
    }
    let scale = scale_param(p)?;
    std::fs::create_dir_all(&dir)?;
    // Frames are independent: render and encode them in parallel (each render is itself
    // row-parallel; rayon balances both levels). Output is identical to a serial export.
    let doc = &s.doc;
    let files: Vec<String> = (from..=to)
        .into_par_iter()
        .map(|f| {
            let path = dir.join(format!("frame_{:05}.png", f + 1));
            std::fs::write(&path, render_png(doc, f, scale)?)?;
            Ok(path.display().to_string())
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(json!({"frames": files.len(), "dir": dir.display().to_string()}))
}

/// Largest GIF we produce (frames × pixels), keeping memory and time bounded.
const MAX_GIF_PIXELS: u64 = 600 * 1280 * 720;

/// Per-frame delays in 1/100 s that add up to the exact running time (error carried forward),
/// never below 2 cs (browsers slow down shorter delays).
pub fn gif_delays(frames: usize, fps: f32) -> Vec<u16> {
    let fps = if fps.is_finite() && fps > 0.0 { fps } else { 24.0 };
    (0..frames)
        .map(|i| {
            let t0 = (i as f64 * 100.0 / f64::from(fps)).round();
            let t1 = ((i + 1) as f64 * 100.0 / f64::from(fps)).round();
            ((t1 - t0) as u16).max(2)
        })
        .collect()
}

fn export_gif(s: &mut Session, p: &Value) -> Result<Value> {
    let path = PathBuf::from(str_req(p, "path")?);
    // The whole movie: every scene in order.
    let last = s.doc.frame_count().saturating_sub(1);
    let from = opt_u32(p, "from")?.unwrap_or(0).min(last);
    let to = opt_u32(p, "to")?.unwrap_or(last).min(last);
    if to < from {
        return Err(EngineError::Param("'to' must not be before 'from'".into()));
    }
    let scale = scale_param(p)?;
    let looping = opt_bool(p, "loop")?.unwrap_or(true);
    let view = View::stage(&s.doc, scale);
    let (w, h) = (u16::try_from(view.width), u16::try_from(view.height));
    let (Ok(w), Ok(h)) = (w, h) else { return Err(EngineError::Param("GIF frames are limited to 65535 px per side".into())) };
    let n = (to - from + 1) as u64;
    if n * u64::from(w) * u64::from(h) > MAX_GIF_PIXELS {
        return Err(EngineError::Param(format!("that GIF would be too large ({n} frames at {w}×{h}); lower 'scale' or the frame range")));
    }
    // Render and colour-quantize frames in parallel; encode in order.
    let doc = &s.doc;
    let delays = gif_delays(n as usize, doc.stage.fps);
    let frames: Vec<gif::Frame<'static>> = (from..to.saturating_add(1))
        .into_par_iter()
        .zip(delays.into_par_iter())
        .map(|(f, delay)| {
            let (pm, _) = render(&anicraft_doc::evaluate(doc, f), &view)?;
            let mut rgba = pm.to_rgba8();
            let mut frame = gif::Frame::from_rgba_speed(w, h, &mut rgba, 10);
            frame.delay = delay;
            Ok(frame)
        })
        .collect::<Result<Vec<_>>>()?;
    let mut bytes = Vec::new();
    {
        let mut enc = gif::Encoder::new(&mut bytes, w, h, &[]).map_err(|e| EngineError::Format(e.to_string()))?;
        if looping {
            enc.set_repeat(gif::Repeat::Infinite).map_err(|e| EngineError::Format(e.to_string()))?;
        }
        for fr in &frames {
            enc.write_frame(fr).map_err(|e| EngineError::Format(e.to_string()))?;
        }
    }
    std::fs::write(&path, &bytes)?;
    Ok(json!({"written": path.display().to_string(), "frames": n, "bytes": bytes.len(), "width": w, "height": h}))
}

fn set_stage(s: &mut Session, p: &Value) -> Result<Value> {
    let cur = s.doc.stage;
    s.doc.stage.width = stage_edge(p, "width", cur.width)?;
    s.doc.stage.height = stage_edge(p, "height", cur.height)?;
    s.doc.stage.fps = fps(p, cur.fps)?;
    if let Some(c) = opt_color(p, "background")? {
        s.doc.stage.background = c;
    }
    Ok(json!({"width": s.doc.stage.width, "height": s.doc.stage.height, "fps": s.doc.stage.fps, "background": s.doc.stage.background.to_hex()}))
}
