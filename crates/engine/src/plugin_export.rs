//! Exporter plug-ins (ADR 0088, `docs/plugins.md`): `file.export_plugin` and the list of every
//! export format, `file.export_formats`.
//!
//! An exporter never sees the document. Its host holds a snapshot and answers two requests:
//! `movie` (stage, frame rate, the range being exported, the scenes) and `frame` (one movie
//! frame evaluated into the same flattened display list the built-in exporters draw: paths in
//! local space with their stage transform, paints, strokes, colour effects, clips and groups).
//! The plug-in returns the file's bytes; AniCraft writes them. Progress is the frames it has
//! read, and Cancel (a publish [`Control`]) stops it at the next request or slice of fuel.

use crate::params::*;
use crate::publish::Control;
use crate::{CommandSpec, EngineError, Result, Session};
use anicraft_doc::{DisplayItem, DisplayList, Document, Paint, Rgba, Stroke};
use anicraft_plugins::{Host, Kind, Limits, Plugin};
use serde_json::{Map, Value, json};
use std::path::PathBuf;
use std::sync::Arc;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "file.export_plugin",
            label: "Export with Plug-in…",
            menu: "",
            shortcut: None,
            params: "{format: an exporter plug-in's id (see file.export_formats), path, from?, to? (movie frames: every scene in order), options?: {…} (the plug-in's; missing ones take their defaults), confirm?}",
            mutates: false,
            run: export_cmd,
        },
        CommandSpec {
            id: "file.export_formats",
            label: "Export Formats",
            menu: "",
            shortcut: None,
            params: "{} → every export format: the built-in ones (with their command) and the exporter plug-ins (with their extension and options)",
            mutates: false,
            run: |s, _| Ok(json!({"formats": formats(s)})),
        },
    ]
}

/// The built-in export formats: `(id, command, name, extensions, what the command's output
/// param is)`. The CLI's `export --format` takes these ids too.
pub const BUILT_IN: &[(&str, &str, &str, &[&str], &str)] = &[
    ("png", "file.export_png", "PNG image (one frame)", &["png"], "path"),
    ("svg", "file.export_svg", "SVG image (one frame)", &["svg"], "path"),
    ("png-sequence", "file.export_png_sequence", "PNG sequence", &[], "dir"),
    ("sprite-sheet", "file.export_sprite_sheet", "Sprite sheet", &["png"], "path"),
    ("gif", "file.export_gif", "Animated GIF", &["gif"], "path"),
    ("apng", "file.export_apng", "Animated PNG", &["png", "apng"], "path"),
    ("webp", "file.export_webp", "Animated WebP", &["webp"], "path"),
    ("video", "file.export_video", "Video", &["webm", "mp4"], "path"),
    ("lottie", "file.export_lottie", "Lottie animation", &["json"], "path"),
    ("html", "file.export_html", "HTML canvas", &["html"], "path"),
];

/// Every export format, built-in first, then the installed exporter plug-ins by id.
pub fn formats(s: &Session) -> Vec<Value> {
    let mut out: Vec<Value> = BUILT_IN.iter().map(|(id, cmd, name, ext, target)| json!({"id": id, "name": name, "extensions": ext, "command": cmd, "target": target, "plugin": false})).collect();
    for p in exporters(s) {
        let m = p.manifest();
        let ext = m.format.as_ref().map_or("", |f| f.extension.as_str());
        let name = m.format.as_ref().map_or(m.name.as_str(), |f| f.name.as_str());
        out.push(json!({
            "id": m.id,
            "name": name,
            "extensions": [ext],
            "command": "file.export_plugin",
            "target": "path",
            "plugin": true,
            "description": m.description,
            "options": crate::cmds_plugin::describe(s, &p)["params"],
        }));
    }
    out
}

/// The installed exporter plug-ins, by id.
pub fn exporters(s: &Session) -> Vec<Arc<Plugin>> {
    s.plugins.list().filter(|p| p.manifest().kind == Kind::Exporter).cloned().collect()
}

/// One export, checked: where, which movie frames, and the plug-in's resolved options.
#[derive(Clone, Debug, PartialEq)]
pub struct Job {
    pub path: PathBuf,
    pub from: u32,
    pub to: u32,
    pub options: Map<String, Value>,
}

/// Reads `file.export_plugin`'s params against the session: the plug-in and the job.
pub fn plan(s: &Session, p: &Value) -> Result<(Arc<Plugin>, Job)> {
    let id = str_req(p, "format")?;
    let plugin = s.plugins.get(id).ok_or_else(|| {
        let ids: Vec<String> = exporters(s).iter().map(|p| p.id().to_string()).collect();
        EngineError::State(if ids.is_empty() { format!("no exporter plug-in “{id}” is installed (see file.export_formats)") } else { format!("no exporter plug-in “{id}” is installed (installed: {})", ids.join(", ")) })
    })?;
    if plugin.manifest().kind != Kind::Exporter {
        return Err(EngineError::State(format!("{} is not an exporter plug-in", plugin.manifest().name)));
    }
    let path = PathBuf::from(str_req(p, "path")?.trim());
    if path.as_os_str().is_empty() {
        return Err(EngineError::Param("param 'path' is empty: name the file to write".into()));
    }
    let last = s.doc.frame_count().saturating_sub(1);
    let from = opt_u32(p, "from")?.unwrap_or(0).min(last);
    let to = opt_u32(p, "to")?.unwrap_or(last).min(last);
    if to < from {
        return Err(EngineError::Param("'to' must not be before 'from'".into()));
    }
    let options = plugin.manifest().resolve_params(p.get("options").unwrap_or(&Value::Null)).map_err(|e| EngineError::Param(e.to_string()))?;
    Ok((plugin, Job { path, from, to, options }))
}

fn export_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let (plugin, job) = plan(s, p)?;
    run(&plugin, &s.doc, &job, s.job.as_deref())
}

/// Runs an export and writes its file: in a command, or on the app's worker thread with a
/// snapshot of the document and a [`Control`] for progress and Cancel.
pub fn run(plugin: &Plugin, doc: &Document, job: &Job, control: Option<&Control>) -> Result<Value> {
    let frames = u64::from(job.to - job.from) + 1;
    if let Some(c) = control {
        c.begin(frames);
    }
    let started = web_time::Instant::now();
    let name: String = job.path.file_stem().map_or_else(|| "Untitled".into(), |n| n.to_string_lossy().chars().take(128).collect());
    let mut host = FrameHost { doc, from: job.from, to: job.to, name, control, read: vec![false; frames as usize], served: 0 };
    let (bytes, report) = plugin.export(&mut host, &job.options, &Limits::exporter()).map_err(|e| match e {
        anicraft_plugins::Error::Cancelled => EngineError::Cancelled,
        e => EngineError::State(e.to_string()),
    })?;
    crate::vfs::write(&job.path, &bytes)?;
    let m = plugin.manifest();
    Ok(json!({
        "written": job.path.display().to_string(),
        "bytes": bytes.len(),
        "format": m.id,
        "name": m.format.as_ref().map_or(m.name.as_str(), |f| f.name.as_str()),
        "frames": frames,
        "frames_read": host.served,
        "options": job.options,
        "log": report.log,
        "calls": report.calls,
        "fuel": report.fuel,
        "ms": started.elapsed().as_millis() as u64,
    }))
}

/// What an exporter talks to: a snapshot of the document, never the session.
struct FrameHost<'a> {
    doc: &'a Document,
    from: u32,
    to: u32,
    name: String,
    control: Option<&'a Control>,
    /// Which frames of the range it has read (progress counts each once).
    read: Vec<bool>,
    served: u64,
}

impl Host for FrameHost<'_> {
    fn request(&mut self, op: &str, req: &Value) -> std::result::Result<Value, String> {
        match op {
            "movie" => Ok(self.movie()),
            "frame" => {
                let n = u64::from(self.to - self.from) + 1;
                let i = req.get("index").and_then(Value::as_u64).ok_or("a frame request needs an \"index\": 0 for the first frame exported")?;
                if i >= n {
                    return Err(format!("frame index {i} is past the end: the export has {n} frames (0..{})", n - 1));
                }
                if let Some(seen) = self.read.get_mut(i as usize)
                    && !*seen
                {
                    *seen = true;
                    self.served += 1;
                    let _ = crate::publish::tick(self.control, 1);
                }
                Ok(frame_json(self.doc, self.from + i as u32, i))
            }
            "command" | "document" | "elements" => Err("an exporter reads the movie's frames (movie, frame); it can't read or change the document".into()),
            _ => Err(format!("unknown op '{op}' (an exporter has movie, frame and log)")),
        }
    }

    fn cancelled(&self) -> bool {
        self.control.is_some_and(Control::cancelled)
    }
}

impl FrameHost<'_> {
    fn movie(&self) -> Value {
        let d = self.doc;
        let st = &d.stage;
        let scenes: Vec<Value> = d.scenes.iter().enumerate().map(|(i, sc)| json!({"name": sc.name, "start": d.scene_start(i), "frames": d.timeline(i).frame_count()})).collect();
        json!({
            "abi": anicraft_plugins::ABI_VERSION,
            "name": self.name,
            "width": st.width,
            "height": st.height,
            "fps": st.fps,
            "background": rgba_hex(st.background),
            "from": self.from,
            "to": self.to,
            "frames": self.to - self.from + 1,
            "total_frames": d.frame_count(),
            "scenes": scenes,
        })
    }
}

/// `#rrggbbaa`.
fn rgba_hex(c: Rgba) -> String {
    format!("{}{:02x}", c.to_hex(), (if c.a.is_finite() { c.a.clamp(0.0, 1.0) } else { 1.0 } * 255.0).round() as u8)
}

fn stops_json(stops: &[anicraft_doc::GradientStop]) -> Value {
    Value::Array(stops.iter().map(|s| json!([s.offset, rgba_hex(s.color)])).collect())
}

fn paint_json(p: &Paint) -> Value {
    match p {
        Paint::Solid(c) => json!({"type": "solid", "color": rgba_hex(*c)}),
        Paint::Linear { start, end, stops, spread, linear_rgb } => json!({"type": "linear", "start": [start.x, start.y], "end": [end.x, end.y], "stops": stops_json(stops), "spread": format!("{spread:?}").to_lowercase(), "linear_rgb": linear_rgb}),
        Paint::Radial { center, radius, stops, spread, linear_rgb, focal, matrix } => json!({"type": "radial", "center": [center.x, center.y], "radius": radius, "focal": focal, "matrix": matrix.m, "stops": stops_json(stops), "spread": format!("{spread:?}").to_lowercase(), "linear_rgb": linear_rgb}),
        Paint::Bitmap { bitmap, repeat, matrix } => json!({"type": "bitmap", "bitmap": bitmap.0, "repeat": repeat, "matrix": matrix.m}),
    }
}

fn stroke_json(s: &Stroke) -> Value {
    let mut v = json!({"color": rgba_hex(s.color), "width": s.width, "cap": s.style.cap.key(), "join": s.style.join.key(), "miter_limit": s.style.miter_limit});
    if let Some(d) = &s.style.dash {
        v["dash"] = serde_json::to_value(d).unwrap_or(Value::Null);
    }
    if s.profile.is_some() {
        v["variable_width"] = json!(true);
    }
    v
}

fn item_json(it: &DisplayItem) -> Value {
    let mut v = json!({
        "layer": it.layer.0,
        "element": it.element.0,
        "path": it.path.to_svg(),
        "transform": it.transform.m,
        "fill": it.fill.as_ref().map_or(Value::Null, paint_json),
        "stroke": it.stroke.as_ref().map_or(Value::Null, stroke_json),
        "fill_rule": format!("{:?}", it.fill_rule).to_lowercase(),
        "alpha": it.color.mul[3],
    });
    if it.color != anicraft_doc::ColorTransform::IDENTITY {
        v["color"] = json!({"mul": it.color.mul, "add": it.color.add});
    }
    for (k, x) in [("clip", it.clip), ("masks", it.masks), ("group", it.group)] {
        if let Some(i) = x {
            v[k] = json!(i);
        }
    }
    v
}

/// Movie frame `frame` as an exporter sees it (`index` is its place in the export).
pub fn frame_json(doc: &Document, frame: u32, index: u64) -> Value {
    let dl: DisplayList = anicraft_doc::evaluate(doc, frame);
    let (scene, local) = doc.locate(frame);
    let items: Vec<Value> = dl.items.iter().filter(|i| !i.guide).map(item_json).collect();
    let groups: Vec<Value> = dl.groups.iter().map(|g| json!({"parent": g.parent, "alpha": g.alpha, "blend": format!("{:?}", g.blend).to_lowercase(), "clip": g.clip, "filters": g.filters.len()})).collect();
    json!({
        "index": index,
        "frame": frame,
        "scene": scene,
        "scene_frame": local,
        "items": items,
        "clips": dl.clip_parents,
        "groups": groups,
    })
}
