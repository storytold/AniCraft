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
        CommandSpec { id: "file.export_png", label: "Export Frame as PNG…", menu: "File/Export", shortcut: None, params: "{path, frame?, scale?}", mutates: false, run: export_png },
        CommandSpec { id: "file.export_svg", label: "Export Frame as SVG…", menu: "File/Export", shortcut: None, params: "{path, frame?, confirm?}", mutates: false, run: export_svg },
        CommandSpec {
            id: "file.export_png_sequence",
            label: "Export PNG Sequence…",
            menu: "File/Export",
            shortcut: None,
            params: "{dir, from?, to?, scale?}",
            mutates: false,
            run: export_sequence,
        },
        CommandSpec {
            id: "file.export_gif",
            label: "Export Animated GIF…",
            menu: "File/Export",
            shortcut: None,
            params: "{path, from?, to?, scale?, loop?, confirm?}",
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

/// Renders a frame of the session's document to PNG bytes.
pub fn render_png(doc: &Document, frame: u32, scale: f32) -> Result<Vec<u8>> {
    let dl = anicraft_doc::evaluate(doc, frame);
    let (pm, _) = render(&dl, &View::stage(doc, scale))?;
    Ok(pm.encode_png()?)
}

fn export_png(s: &mut Session, p: &Value) -> Result<Value> {
    let path = PathBuf::from(str_req(p, "path")?);
    let frame = opt_u32(p, "frame")?.unwrap_or(s.frame);
    let bytes = render_png(&s.doc, frame, scale_param(p)?)?;
    std::fs::write(&path, &bytes)?;
    Ok(json!({"written": path.display().to_string(), "bytes": bytes.len(), "frame": frame}))
}

/// A frame as a standalone SVG document (vectors, gradients and masks kept).
pub fn frame_svg(doc: &Document, frame: u32) -> String {
    anicraft_render::svg::to_svg(&anicraft_doc::evaluate(doc, frame), &doc.stage)
}

fn export_svg(s: &mut Session, p: &Value) -> Result<Value> {
    let path = PathBuf::from(str_req(p, "path")?);
    let frame = opt_u32(p, "frame")?.unwrap_or(s.frame);
    if frame >= s.frame_count() {
        return Err(EngineError::Param(format!("frame {} is past the end (the timeline has {} frames)", frame + 1, s.frame_count())));
    }
    let svg = frame_svg(&s.doc, frame);
    std::fs::write(&path, svg.as_bytes())?;
    Ok(json!({"written": path.display().to_string(), "bytes": svg.len(), "frame": frame}))
}

fn export_sequence(s: &mut Session, p: &Value) -> Result<Value> {
    let dir = PathBuf::from(str_req(p, "dir")?);
    let last = s.frame_count().saturating_sub(1);
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
    let last = s.frame_count().saturating_sub(1);
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
