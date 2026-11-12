//! File › Publish Settings and File › Publish (ADR 0060).
//!
//! The document holds a [`PublishProfile`]: every format, on or off, with its options and an
//! output template relative to the document's folder (`{name}` is the document's name). Publish
//! turns the formats that are on into a [`Step`] each (the export command and its params), and
//! runs them in order through the same export commands File › Export uses, on a session of its
//! own: on the caller's thread for `file.publish` (CLI, MCP, the web build one step per frame),
//! or on a background worker in the desktop editor ([`Publisher`]). A [`Control`] cancels the
//! run between and inside exports and counts frames for the progress view. One [`Report`] says
//! what each format wrote, skipped or left out.

use crate::params::*;
use crate::{CommandSpec, EngineError, Result, Session};
use anicraft_doc::Document;
use anicraft_doc::publish::{FrameRange, MAX_OUTPUT_CHARS, MAX_PUBLISH_SCALE, MAX_TITLE_CHARS, PublishFormat, PublishProfile};
use serde_json::{Map, Value, json};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "publish.settings_get",
            label: "Publish Settings…",
            menu: "File",
            shortcut: Some("Ctrl+Shift+F12"),
            params: "{} (the document's publish profile, and where each format would go)",
            mutates: false,
            run: settings_get,
        },
        CommandSpec {
            id: "publish.settings_set",
            label: "Publish Settings",
            menu: "",
            shortcut: None,
            params: "{profile? (a whole profile, as publish.settings_get returns it), format? (html | gif | apng | webp | video | lottie | sprite_sheet | png_sequence | png | svg | audio), enabled?, output? (relative to the document's folder; {name} is the document's name), and that format's options: title, sound, fit (html); from, to, scale, loop (gif); from, to, scale, loop (true, false or how many times it plays; 0: forever), background (apng, webp); from, to, scale, audio (video); from, to, background (lottie); from, to, scale, columns, padding, background (sprite_sheet); from, to, scale (png_sequence); frame, scale (png); frame (svg); from, to, rate, bits (audio). from/to/frame are movie frames, null for the movie's ends}",
            mutates: true,
            run: settings_set,
        },
        CommandSpec {
            id: "file.publish",
            label: "Publish",
            menu: "File",
            shortcut: Some("Shift+F12"),
            params: "{formats? ([format, …]: only these; default: the ones Publish Settings has on), confirm? (to replace files already there)}",
            mutates: false,
            run: publish_cmd,
        },
    ]
}

// ----- cancel and progress -------------------------------------------------------------------

/// Shared between a running publish and whoever watches it: Cancel, and frames done of the
/// format running. Exports check it between frames (`tick`).
#[derive(Debug, Default)]
pub struct Control {
    cancel: AtomicBool,
    done: AtomicU64,
    total: AtomicU64,
}

impl Control {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
    /// Frames done and frames in all, of the format running (0 in all: no frame count).
    pub fn progress(&self) -> (u64, u64) {
        (self.done.load(Ordering::Relaxed), self.total.load(Ordering::Relaxed))
    }
    fn begin(&self, total: u64) {
        self.done.store(0, Ordering::Relaxed);
        self.total.store(total, Ordering::Relaxed);
    }
}

/// An export made `frames` more frames: counts them, and stops it when Publish was cancelled.
pub(crate) fn tick(job: Option<&Control>, frames: u64) -> Result<()> {
    let Some(j) = job else { return Ok(()) };
    j.done.fetch_add(frames, Ordering::Relaxed);
    if j.cancelled() { Err(EngineError::Cancelled) } else { Ok(()) }
}

// ----- where things go -----------------------------------------------------------------------

/// The folder outputs are relative to and the `{name}` they use: the document's folder and file
/// name. An untitled document has no folder on disk, so it can't publish until it is saved; in
/// the browser (no folders, files become downloads) it publishes as "Untitled".
pub fn base(path: Option<&Path>) -> std::result::Result<(PathBuf, String), String> {
    match path {
        Some(p) => {
            let name = p.file_stem().map_or_else(|| "Untitled".to_string(), |n| n.to_string_lossy().into_owned());
            Ok((p.parent().map(Path::to_path_buf).unwrap_or_default(), name))
        }
        None if crate::vfs::in_memory() => Ok((PathBuf::new(), "Untitled".into())),
        None => Err("save the document first: Publish writes its files beside it".into()),
    }
}

/// Output template `template` of format `f`, for a document named `name`: a path inside the
/// document's folder. `{name}` is the only placeholder; the path may go into folders below
/// but never above or elsewhere (a document opened from someone else can't write anywhere
/// else on this computer); a file without an extension gets the format's.
pub fn resolve(f: PublishFormat, template: &str, name: &str) -> std::result::Result<PathBuf, String> {
    let t = template.trim();
    if t.is_empty() {
        return Err("the output name is empty".into());
    }
    if t.chars().count() > MAX_OUTPUT_CHARS {
        return Err(format!("the output name is longer than {MAX_OUTPUT_CHARS} characters"));
    }
    let text = t.replace("{name}", name);
    if t.replace("{name}", "").contains(['{', '}']) {
        return Err(format!("“{t}” has a placeholder Publish doesn't know: {{name}} (the document's name) is the only one"));
    }
    let mut rel = PathBuf::new();
    for c in Path::new(&text).components() {
        match c {
            Component::Normal(part) => rel.push(part),
            Component::CurDir => {}
            _ => return Err(format!("“{t}” must stay in the document's folder (a name, or folders below it like web/{{name}}.html)")),
        }
    }
    if rel.as_os_str().is_empty() {
        return Err("the output name is empty".into());
    }
    let exts = f.extensions();
    if !exts.is_empty() {
        match rel.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase) {
            None => {
                let file = rel.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                rel.set_file_name(format!("{file}.{}", exts.first().copied().unwrap_or("")));
            }
            Some(e) if exts.contains(&e.as_str()) => {}
            Some(_) => return Err(format!("{} goes to {}", f.label(), exts.iter().map(|e| format!(".{e}")).collect::<Vec<_>>().join(" or "))),
        }
    }
    Ok(rel)
}

// ----- the plan --------------------------------------------------------------------------------

/// One format to publish: the export command that writes it and its params.
#[derive(Clone, Debug)]
pub struct Step {
    pub format: PublishFormat,
    pub command: &'static str,
    pub params: Value,
    /// The files (or, for the PNG sequence, the folder) it writes.
    pub outputs: Vec<PathBuf>,
    /// The scene the export runs in (one-frame formats), and frames it makes (0: not counted).
    pub scene: usize,
    pub frames: u64,
    /// Why it won't run here, when it won't.
    pub skip: Option<String>,
}

fn range_params(p: &mut Map<String, Value>, r: FrameRange, last: u32) -> u64 {
    if let Some(f) = r.from {
        p.insert("from".into(), json!(f));
    }
    if let Some(t) = r.to {
        p.insert("to".into(), json!(t));
    }
    let from = r.from.unwrap_or(0).min(last);
    let to = r.to.unwrap_or(last).min(last);
    u64::from(to.saturating_sub(from)) + 1
}

/// The steps Publish runs for `doc` saved at `path`: the formats that are on (or `only`), in
/// order, or why it can't run (an output that isn't one, two formats writing the same file).
pub fn plan(doc: &Document, path: Option<&Path>, only: Option<&[PublishFormat]>) -> std::result::Result<Vec<Step>, String> {
    let (dir, name) = base(path)?;
    let prof = &doc.publish;
    let formats: Vec<PublishFormat> = match only {
        Some(list) => PublishFormat::ALL.into_iter().filter(|f| list.contains(f)).collect(),
        None => prof.enabled(),
    };
    let last = doc.frame_count().saturating_sub(1);
    let mut steps: Vec<Step> = Vec::new();
    for f in formats {
        let rel = resolve(f, prof.target(f).1, &name).map_err(|e| format!("{}: {e}", f.label()))?;
        let out = dir.join(&rel);
        let path_s = out.display().to_string();
        let mut p = Map::new();
        let (mut scene, mut skip) = (0, None);
        let (command, frames, outputs) = match f {
            PublishFormat::Html => {
                let h = &prof.html;
                if !h.title.trim().is_empty() {
                    p.insert("title".into(), json!(h.title));
                }
                p.insert("sound".into(), json!(h.sound));
                p.insert("fit".into(), json!(h.fit));
                ("file.export_html", 0, vec![out])
            }
            PublishFormat::Gif => {
                let g = &prof.gif;
                let n = range_params(&mut p, g.range, last);
                p.insert("scale".into(), json!(g.scale));
                p.insert("loop".into(), json!(g.looping));
                ("file.export_gif", n, vec![out])
            }
            PublishFormat::Apng | PublishFormat::Webp => {
                let (range, scale, plays, background, command) = match f {
                    PublishFormat::Apng => (prof.apng.range, prof.apng.scale, prof.apng.plays, prof.apng.background, "file.export_apng"),
                    _ => (prof.webp.range, prof.webp.scale, prof.webp.plays, prof.webp.background, "file.export_webp"),
                };
                let n = range_params(&mut p, range, last);
                p.insert("scale".into(), json!(scale));
                p.insert("loop".into(), json!(plays));
                p.insert("background".into(), json!(background));
                (command, n, vec![out])
            }
            PublishFormat::Video => {
                let v = &prof.video;
                let n = range_params(&mut p, v.range, last);
                p.insert("scale".into(), json!(v.scale));
                p.insert("audio".into(), json!(v.audio));
                if crate::vfs::in_memory() {
                    skip = Some("video is made with ffmpeg, which a web browser can't run".to_string());
                }
                ("file.export_video", n, vec![out])
            }
            PublishFormat::Lottie => {
                let l = &prof.lottie;
                range_params(&mut p, l.range, last);
                p.insert("background".into(), json!(l.background));
                ("file.export_lottie", 0, vec![out])
            }
            PublishFormat::SpriteSheet => {
                let sh = &prof.sprite_sheet;
                let n = range_params(&mut p, sh.range, last);
                p.insert("scale".into(), json!(sh.scale));
                if sh.columns > 0 {
                    p.insert("columns".into(), json!(sh.columns));
                }
                p.insert("padding".into(), json!(sh.padding));
                p.insert("background".into(), json!(sh.background));
                let map = out.with_extension("json");
                ("file.export_sprite_sheet", n, vec![out, map])
            }
            PublishFormat::PngSequence => {
                let q = &prof.png_sequence;
                let n = range_params(&mut p, q.range, last);
                p.insert("scale".into(), json!(q.scale));
                ("file.export_png_sequence", n, vec![out])
            }
            PublishFormat::Png => {
                let (sc, local) = doc.locate(prof.png.frame.min(last));
                scene = sc;
                p.insert("frame".into(), json!(local));
                p.insert("scale".into(), json!(prof.png.scale));
                ("file.export_png", 1, vec![out])
            }
            PublishFormat::Svg => {
                let (sc, local) = doc.locate(prof.svg.frame.min(last));
                scene = sc;
                p.insert("frame".into(), json!(local));
                ("file.export_svg", 1, vec![out])
            }
            PublishFormat::Audio => {
                let a = &prof.audio;
                range_params(&mut p, a.range, last);
                p.insert("rate".into(), json!(a.rate));
                p.insert("bits".into(), json!(a.bits));
                if !anicraft_doc::sound::has_sounds(doc) {
                    skip = Some("the movie has no sounds".to_string());
                }
                ("file.export_audio", 0, vec![out])
            }
        };
        let key = if f == PublishFormat::PngSequence { "dir" } else { "path" };
        p.insert(key.into(), json!(path_s));
        // Publish's own confirmation covers replacing files (`existing`).
        p.insert("confirm".into(), json!(true));
        // Two formats writing one file would leave only the second (names compared as the
        // case-insensitive file systems of Windows and macOS see them).
        let same = |a: &Path, b: &Path| a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase();
        if let Some(other) = steps.iter().find(|s| s.outputs.iter().any(|o| outputs.iter().any(|n| same(o, n)))) {
            return Err(format!("{} and {} would both write {}: give one of them another output name", other.format.label(), f.label(), rel.display()));
        }
        steps.push(Step { format: f, command, params: Value::Object(p), outputs, scene, frames, skip });
    }
    Ok(steps)
}

/// Files already there that publishing `steps` would replace (a PNG sequence's folder when it
/// holds exported frames), leaving out those this session published itself.
pub fn existing(s: &Session, steps: &[Step]) -> Vec<PathBuf> {
    let mut v = Vec::new();
    for st in steps.iter().filter(|st| st.skip.is_none()) {
        for o in &st.outputs {
            if s.published.contains(o) {
                continue;
            }
            let there = if st.format == PublishFormat::PngSequence {
                crate::vfs::list(o, 10_000).iter().any(|e| {
                    let n = e.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
                    n.starts_with("frame_") && n.ends_with(".png")
                })
            } else {
                crate::vfs::exists(o)
            };
            if there {
                v.push(o.clone());
            }
        }
    }
    v
}

/// The formats `file.publish` params name, if they name any.
pub fn formats_param(p: &Value) -> Result<Option<Vec<PublishFormat>>> {
    match p.get("formats") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(list)) => list
            .iter()
            .take(64)
            .map(|v| v.as_str().and_then(PublishFormat::from_key).ok_or_else(|| EngineError::Param(format!("unknown format {v} (html, gif, apng, webp, video, lottie, sprite_sheet, png_sequence, png, svg or audio)"))))
            .collect::<Result<Vec<_>>>()
            .map(Some),
        Some(_) => Err(EngineError::Param("param 'formats' must be a list of format names".into())),
    }
}

/// The plan `file.publish` with `p` would run, for the confirmation (ADR 0004).
pub(crate) fn planned(s: &Session, p: &Value) -> Option<Vec<Step>> {
    plan(&s.doc, s.path.as_deref(), formats_param(p).ok()?.as_deref()).ok()
}

// ----- running ---------------------------------------------------------------------------------

/// How one format went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Published,
    Skipped,
    Failed,
    Cancelled,
}

impl Status {
    pub fn key(self) -> &'static str {
        match self {
            Status::Published => "published",
            Status::Skipped => "skipped",
            Status::Failed => "failed",
            Status::Cancelled => "cancelled",
        }
    }
}

/// What one format did: the files it wrote, and what its exporter left out or why it failed.
#[derive(Clone, Debug)]
pub struct Outcome {
    pub format: PublishFormat,
    pub status: Status,
    pub files: Vec<PathBuf>,
    /// The exporter's report of what it approximated or left out (HTML, Lottie), or why the
    /// format was skipped, failed or stopped.
    pub message: String,
}

impl Outcome {
    pub fn to_json(&self) -> Value {
        json!({
            "format": self.format.key(),
            "label": self.format.label(),
            "status": self.status.key(),
            "files": self.files.iter().map(|f| f.display().to_string()).collect::<Vec<_>>(),
            "message": self.message,
        })
    }
}

/// Every format's outcome, in order.
#[derive(Clone, Debug, Default)]
pub struct Report {
    pub outcomes: Vec<Outcome>,
}

impl Report {
    pub fn count(&self, s: Status) -> usize {
        self.outcomes.iter().filter(|o| o.status == s).count()
    }
    pub fn files(&self) -> Vec<PathBuf> {
        self.outcomes.iter().flat_map(|o| o.files.iter().cloned()).collect()
    }
    /// One sentence per format with something to say (left out, skipped, failed), in order.
    pub fn summary(&self) -> String {
        self.outcomes
            .iter()
            .filter(|o| !o.message.is_empty())
            .map(|o| {
                let m = o.message.trim_end_matches('.');
                match o.status {
                    Status::Published => format!("{}: {m}.", o.format.label()),
                    Status::Skipped => format!("{} skipped: {m}.", o.format.label()),
                    Status::Failed => format!("{} failed: {m}.", o.format.label()),
                    Status::Cancelled => format!("{}: {m}.", o.format.label()),
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }
    pub fn to_json(&self) -> Value {
        json!({
            "published": self.count(Status::Published),
            "skipped": self.count(Status::Skipped),
            "failed": self.count(Status::Failed),
            "cancelled": self.count(Status::Cancelled) > 0,
            "files": self.files().iter().map(|f| f.display().to_string()).collect::<Vec<_>>(),
            "formats": self.outcomes.iter().map(Outcome::to_json).collect::<Vec<_>>(),
            "report": self.summary(),
        })
    }
}

/// Runs the steps of a publish one at a time, on a session of its own (the document as it was
/// when Publish started), so the editor keeps working while it runs.
pub struct Publisher {
    s: Session,
    steps: Vec<Step>,
    next: usize,
    report: Report,
    control: Arc<Control>,
}

impl Publisher {
    pub fn new(doc: Document, path: Option<PathBuf>, steps: Vec<Step>, control: Arc<Control>) -> Publisher {
        let mut s = Session::new(doc);
        s.path = path;
        s.job = Some(control.clone());
        Publisher { s, steps, next: 0, report: Report::default(), control }
    }

    /// The format running next (or now), if any is left.
    pub fn current(&self) -> Option<PublishFormat> {
        self.steps.get(self.next).map(|s| s.format)
    }

    pub fn done(&self) -> bool {
        self.next >= self.steps.len()
    }

    /// Runs the next format; `None` when every format has run. Once cancelled, the formats
    /// left are cancelled without running.
    pub fn step(&mut self) -> Option<&Outcome> {
        let st = self.steps.get(self.next)?.clone();
        self.next += 1;
        let out = self.run(&st);
        self.report.outcomes.push(out);
        self.report.outcomes.last()
    }

    fn run(&mut self, st: &Step) -> Outcome {
        let mut out = Outcome { format: st.format, status: Status::Published, files: Vec::new(), message: String::new() };
        if self.control.cancelled() {
            out.status = Status::Cancelled;
            out.message = "not published (cancelled)".into();
            return out;
        }
        if let Some(why) = &st.skip {
            out.status = Status::Skipped;
            out.message.clone_from(why);
            return out;
        }
        self.control.begin(st.frames);
        // Folders below the document's are made as needed.
        for o in &st.outputs {
            let dir = if st.format == PublishFormat::PngSequence { Some(o.as_path()) } else { o.parent() };
            if let Some(d) = dir.filter(|d| !d.as_os_str().is_empty())
                && let Err(e) = crate::vfs::create_dir_all(d)
            {
                out.status = Status::Failed;
                out.message = format!("could not make the folder {}: {e}", d.display());
                return out;
            }
        }
        self.s.scene = st.scene.min(self.s.doc.scenes.len().saturating_sub(1));
        self.s.editing.clear();
        self.s.frame = 0;
        let result = crate::find_command(st.command).ok_or_else(|| EngineError::UnknownCommand(st.command.into())).and_then(|spec| (spec.run)(&mut self.s, &st.params));
        match result {
            Ok(v) => {
                out.files.clone_from(&st.outputs);
                out.message = v.get("report").and_then(Value::as_str).unwrap_or("").to_string();
            }
            Err(EngineError::Cancelled) => {
                out.status = Status::Cancelled;
                out.message = if st.format == PublishFormat::PngSequence { "stopped part-way (the frames written so far are kept)".into() } else { "stopped before it was written".into() };
            }
            Err(e) => {
                out.status = Status::Failed;
                out.message = e.to_string();
            }
        }
        out
    }

    /// Runs every format left.
    pub fn run_all(&mut self) {
        while self.step().is_some() {}
    }

    pub fn report(&self) -> &Report {
        &self.report
    }
}

fn publish_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let steps = plan(&s.doc, s.path.as_deref(), formats_param(p)?.as_deref()).map_err(EngineError::State)?;
    if steps.is_empty() {
        return Err(EngineError::State("no format is on in Publish Settings: turn one on first".into()));
    }
    let control = s.job.clone().unwrap_or_default();
    let mut job = Publisher::new(s.doc.clone(), s.path.clone(), steps, control);
    job.run_all();
    let report = job.report();
    s.published.extend(report.files());
    Ok(report.to_json())
}

// ----- the settings commands -------------------------------------------------------------------

fn settings_get(s: &mut Session, _: &Value) -> Result<Value> {
    Ok(settings_json(&s.doc, s.path.as_deref()))
}

/// The profile, and for each format whether it is on, its output and where that goes now (or
/// why it can't go anywhere).
pub fn settings_json(doc: &Document, path: Option<&Path>) -> Value {
    let base = base(path);
    let formats: Vec<Value> = PublishFormat::ALL
        .iter()
        .map(|f| {
            let (on, output) = doc.publish.target(*f);
            let mut o = json!({"format": f.key(), "label": f.label(), "enabled": on, "output": output});
            match base.as_ref().map_err(Clone::clone).and_then(|(dir, name)| resolve(*f, output, name).map(|r| dir.join(r))) {
                Ok(at) => {
                    o["path"] = json!(at.display().to_string());
                    o["exists"] = json!(crate::vfs::exists(&at));
                }
                Err(e) => o["error"] = json!(e),
            }
            o
        })
        .collect();
    let (folder, name) = base.map_or((Value::Null, Value::Null), |(d, n)| (json!(d.display().to_string()), json!(n)));
    json!({"profile": serde_json::to_value(&*doc.publish).unwrap_or(Value::Null), "formats": formats, "folder": folder, "name": name})
}

/// Keys each format's options take (besides `enabled` and `output`).
fn option_keys(f: PublishFormat) -> &'static [&'static str] {
    match f {
        PublishFormat::Html => &["title", "sound", "fit"],
        PublishFormat::Gif => &["from", "to", "scale", "loop"],
        PublishFormat::Apng | PublishFormat::Webp => &["from", "to", "scale", "loop", "background"],
        PublishFormat::Video => &["from", "to", "scale", "audio"],
        PublishFormat::Lottie => &["from", "to", "background"],
        PublishFormat::SpriteSheet => &["from", "to", "scale", "columns", "padding", "background"],
        PublishFormat::PngSequence => &["from", "to", "scale"],
        PublishFormat::Png => &["frame", "scale"],
        PublishFormat::Svg => &["frame"],
        PublishFormat::Audio => &["from", "to", "rate", "bits"],
    }
}

/// A movie-frame param: absent (unchanged), null (the movie's end) or a frame.
fn opt_end(p: &Value, key: &str) -> Result<Option<Option<u32>>> {
    match p.get(key) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(_) => opt_u32(p, key).map(|v| Some(v.map(|f| f.min(crate::MAX_FRAME)))),
    }
}

fn set_range(r: &mut FrameRange, p: &Value) -> Result<()> {
    if let Some(v) = opt_end(p, "from")? {
        r.from = v;
    }
    if let Some(v) = opt_end(p, "to")? {
        r.to = v;
    }
    if let (Some(a), Some(b)) = (r.from, r.to)
        && b < a
    {
        return Err(EngineError::Param("'to' must not be before 'from'".into()));
    }
    Ok(())
}

fn set_scale(v: &mut f32, p: &Value) -> Result<()> {
    if let Some(x) = opt_f32(p, "scale")? {
        if !(x > 0.0 && x <= MAX_PUBLISH_SCALE) {
            return Err(EngineError::Param(format!("param 'scale' must be in (0, {MAX_PUBLISH_SCALE}]")));
        }
        *v = x;
    }
    Ok(())
}

fn set_bool(v: &mut bool, p: &Value, key: &str) -> Result<()> {
    if let Some(b) = opt_bool(p, key)? {
        *v = b;
    }
    Ok(())
}

/// Changes format `f`'s switch, output and options from `p`.
fn apply(prof: &mut PublishProfile, f: PublishFormat, p: &Value) -> Result<()> {
    if let Some(obj) = p.as_object() {
        let known = option_keys(f);
        if let Some(k) = obj.keys().find(|k| !matches!(k.as_str(), "format" | "enabled" | "output" | "confirm") && !known.contains(&k.as_str())) {
            return Err(EngineError::Param(format!("{} has no option '{k}' (it has {})", f.label(), known.join(", "))));
        }
    }
    if let Some(o) = opt_str(p, "output")? {
        resolve(f, o, "name").map_err(|e| EngineError::Param(format!("{}: {e}", f.label())))?;
        *prof.target_mut(f).1 = o.trim().to_string();
    }
    if let Some(b) = opt_bool(p, "enabled")? {
        *prof.target_mut(f).0 = b;
    }
    match f {
        PublishFormat::Html => {
            let h = &mut prof.html;
            if let Some(t) = opt_str(p, "title")? {
                if t.chars().count() > MAX_TITLE_CHARS {
                    return Err(EngineError::Param(format!("param 'title' is at most {MAX_TITLE_CHARS} characters")));
                }
                h.title = t.to_string();
            }
            set_bool(&mut h.sound, p, "sound")?;
            set_bool(&mut h.fit, p, "fit")?;
        }
        PublishFormat::Gif => {
            let g = &mut prof.gif;
            set_range(&mut g.range, p)?;
            set_scale(&mut g.scale, p)?;
            set_bool(&mut g.looping, p, "loop")?;
        }
        PublishFormat::Apng | PublishFormat::Webp => {
            let (range, scale, plays, background) = match f {
                PublishFormat::Apng => (&mut prof.apng.range, &mut prof.apng.scale, &mut prof.apng.plays, &mut prof.apng.background),
                _ => (&mut prof.webp.range, &mut prof.webp.scale, &mut prof.webp.plays, &mut prof.webp.background),
            };
            set_range(range, p)?;
            set_scale(scale, p)?;
            if p.get("loop").is_some() {
                *plays = crate::anim::plays(p)?;
            }
            set_bool(background, p, "background")?;
        }
        PublishFormat::Video => {
            let v = &mut prof.video;
            set_range(&mut v.range, p)?;
            set_scale(&mut v.scale, p)?;
            set_bool(&mut v.audio, p, "audio")?;
        }
        PublishFormat::Lottie => {
            set_range(&mut prof.lottie.range, p)?;
            set_bool(&mut prof.lottie.background, p, "background")?;
        }
        PublishFormat::SpriteSheet => {
            let sh = &mut prof.sprite_sheet;
            set_range(&mut sh.range, p)?;
            set_scale(&mut sh.scale, p)?;
            if let Some(c) = opt_u32(p, "columns")? {
                sh.columns = c.min(10_000);
            }
            if let Some(x) = opt_u32(p, "padding")? {
                if x > 256 {
                    return Err(EngineError::Param("param 'padding' must be at most 256".into()));
                }
                sh.padding = x;
            }
            set_bool(&mut sh.background, p, "background")?;
        }
        PublishFormat::PngSequence => {
            set_range(&mut prof.png_sequence.range, p)?;
            set_scale(&mut prof.png_sequence.scale, p)?;
        }
        PublishFormat::Png => {
            if let Some(fr) = opt_u32(p, "frame")? {
                prof.png.frame = fr.min(crate::MAX_FRAME);
            }
            set_scale(&mut prof.png.scale, p)?;
        }
        PublishFormat::Svg => {
            if let Some(fr) = opt_u32(p, "frame")? {
                prof.svg.frame = fr.min(crate::MAX_FRAME);
            }
        }
        PublishFormat::Audio => {
            let a = &mut prof.audio;
            set_range(&mut a.range, p)?;
            if let Some(r) = opt_u32(p, "rate")? {
                if !(8_000..=192_000).contains(&r) {
                    return Err(EngineError::Param("param 'rate' must be between 8000 and 192000".into()));
                }
                a.rate = r;
            }
            if let Some(b) = opt_u32(p, "bits")? {
                if !matches!(b, 16 | 24 | 32) {
                    return Err(EngineError::Param("param 'bits' must be 16, 24 or 32 (float)".into()));
                }
                a.bits = b;
            }
        }
    }
    Ok(())
}

fn settings_set(s: &mut Session, p: &Value) -> Result<Value> {
    let mut prof = match p.get("profile") {
        None | Some(Value::Null) => (*s.doc.publish).clone(),
        Some(v) => {
            let mut prof: PublishProfile = serde_json::from_value(v.clone()).map_err(|e| EngineError::Param(format!("param 'profile' is not a publish profile: {e}")))?;
            prof.sanitize();
            prof
        }
    };
    match opt_str(p, "format")? {
        Some(k) => {
            let f = PublishFormat::from_key(k).ok_or_else(|| EngineError::Param(format!("unknown format '{k}' (html, gif, apng, webp, video, lottie, sprite_sheet, png_sequence, png, svg or audio)")))?;
            apply(&mut prof, f, p)?;
        }
        None if p.get("profile").is_none_or(Value::is_null) && p.as_object().is_some_and(|o| o.keys().any(|k| k != "confirm" && k != "profile")) => {
            return Err(EngineError::Param("name the 'format' to change, or pass a whole 'profile'".into()));
        }
        None => {}
    }
    if prof != *s.doc.publish {
        s.doc.publish = Arc::new(prof);
    }
    Ok(settings_json(&s.doc, s.path.as_deref()))
}

/// Writes `files` (path, bytes) as one uncompressed ZIP archive, per PKWARE's APPNOTE (what the browser downloads
/// when Publish writes more than one file). Paths keep their folders, with `/` between them.
pub fn zip(files: &[(PathBuf, Vec<u8>)]) -> std::result::Result<Vec<u8>, String> {
    let mut out: Vec<u8> = Vec::new();
    let mut central: Vec<u8> = Vec::new();
    let n = u16::try_from(files.len()).map_err(|_| "too many files for one archive".to_string())?;
    let le16 = |v: &mut Vec<u8>, x: u16| v.extend_from_slice(&x.to_le_bytes());
    let le32 = |v: &mut Vec<u8>, x: u32| v.extend_from_slice(&x.to_le_bytes());
    for (path, data) in files {
        let name: String = path.components().filter_map(|c| if let Component::Normal(p) = c { Some(p.to_string_lossy().into_owned()) } else { None }).collect::<Vec<_>>().join("/");
        let name_len = u16::try_from(name.len()).map_err(|_| "a file name is too long".to_string())?;
        let size = u32::try_from(data.len()).map_err(|_| format!("{name} is too large for a ZIP archive (4 GB)"))?;
        let offset = u32::try_from(out.len()).map_err(|_| "the archive is too large (4 GB)".to_string())?;
        let crc = anicraft_xfl::zip::crc32(data);
        // Local file header: version 2.0, UTF-8 names (bit 11), stored.
        le32(&mut out, 0x0403_4b50);
        for x in [20u16, 0x0800, 0, 0, 0x21] {
            le16(&mut out, x);
        }
        for x in [crc, size, size] {
            le32(&mut out, x);
        }
        le16(&mut out, name_len);
        le16(&mut out, 0);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
        // Its central directory entry.
        le32(&mut central, 0x0201_4b50);
        for x in [20u16, 20, 0x0800, 0, 0, 0x21] {
            le16(&mut central, x);
        }
        for x in [crc, size, size] {
            le32(&mut central, x);
        }
        for x in [name_len, 0, 0, 0, 0] {
            le16(&mut central, x);
        }
        le32(&mut central, 0);
        le32(&mut central, offset);
        central.extend_from_slice(name.as_bytes());
    }
    let at = u32::try_from(out.len()).map_err(|_| "the archive is too large (4 GB)".to_string())?;
    let len = u32::try_from(central.len()).map_err(|_| "the archive is too large".to_string())?;
    out.extend_from_slice(&central);
    le32(&mut out, 0x0605_4b50);
    for x in [0u16, 0, n, n] {
        le16(&mut out, x);
    }
    le32(&mut out, len);
    le32(&mut out, at);
    le16(&mut out, 0);
    Ok(out)
}

#[cfg(test)]
#[path = "publish_tests.rs"]
mod tests;
