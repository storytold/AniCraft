//! Entry points and document assembly: containers (`.moho` zip, `.mohoproj` file, an
//! extracted folder), the stage from `project_data`, the animated values (camera, markers),
//! and the `maop_maohou` sidecar that keeps the untouched original JSON on the document.

use crate::anim;
use crate::channel::{self, Channel};
use crate::json::{self, array, get};
use crate::layer::{self, FlatLayer};
use crate::zip::Archive;
use crate::MohoError;
use anicraft_doc::{Bitmap, BitmapId, Document, MaohouSidecar, Rgba, Scene, SceneCamera, Stage, Timeline};
use anicraft_doc::Channel as DocChannel;
use anicraft_geom::{Affine, Vec2};
use anicraft_text::{FaceMatch, Fonts};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

/// Last addressable frame (0-based), as in the engine.
const MAX_FRAME: u32 = 100_000;

/// Reads files of the package by their path inside it (`Project.mohoproj`, media next to it).
pub type Reader<'a> = dyn Fn(&str) -> Result<Option<Vec<u8>>, MohoError> + 'a;

/// What the import couldn't bring across, by kind ("3 sounds"), plus what it wants to say.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Report {
    pub skipped: BTreeMap<String, usize>,
    pub notes: Vec<String>,
    /// Font families the document's text asks for that aren't on this computer.
    pub missing_fonts: BTreeSet<String>,
    /// The family that text was laid out in instead.
    pub fallback_font: Option<String>,
}

impl Report {
    pub(crate) fn skip(&mut self, what: &str) {
        *self.skipped.entry(what.to_string()).or_default() += 1;
    }
    /// One line for a status bar: "Imported. Not yet supported: 23 skeleton bones."
    pub fn summary(&self) -> String {
        if self.skipped.is_empty() && self.notes.is_empty() && self.missing_fonts.is_empty() {
            return "Imported everything in the file.".into();
        }
        let mut out = String::from("Imported.");
        let mut parts: Vec<String> = self.skipped.iter().map(|(k, n)| format!("{n} {k}")).collect();
        parts.extend(self.notes.iter().cloned());
        if !parts.is_empty() {
            out.push_str(&format!(" Not yet supported: {}.", parts.join(", ")));
        }
        if !self.missing_fonts.is_empty() {
            let names: Vec<&str> = self.missing_fonts.iter().map(String::as_str).collect();
            let instead = self.fallback_font.as_deref().map(|f| format!(", drawn in {f}")).unwrap_or_default();
            out.push_str(&format!(" Fonts not on this computer{instead}: {}.", names.join(", ")));
        }
        out
    }
}

/// Moho's project scale: pixels per inch. CharWalk's rig (a walk-cycle character) measures
/// about 1.1 inches head to toe on its 432 px canvas, which puts the corpus at this scale;
/// a later milestone re-checks it against Moho renders.
pub const MOHO_DPI: f32 = 320.0;

/// Per-import state shared by the mapper modules.
pub(crate) struct Builder<'a> {
    pub doc: Document,
    pub report: Report,
    /// The fonts text is laid out with; the computer's, found on the first text layer.
    pub fonts: Option<&'a Fonts>,
    /// Face name → what it is on this computer, looked up once per name.
    pub faces: HashMap<String, FaceMatch>,
    pub read: &'a Reader<'a>,
    /// Inch document space (y up, origin centre) → stage pixels (y down, origin top-left).
    pub to_px: Affine,
    /// Its inverse, for content already in pixels (text).
    pub from_px: Affine,
    pub dpi: f32,
    /// The Moho frame (1-based) the static import samples.
    pub frame: f64,
    /// The one keyframe's span, from `project_data`'s frame range.
    pub duration: u32,
    /// The project's playback range `(start, end)`, 0-based with `end` exclusive.
    pub frames: (u32, u32),
}

impl<'a> Builder<'a> {
    /// Registers an image as a document bitmap (PNG or JPEG in, PNG stored).
    pub fn register_bitmap(&mut self, name: String, bytes: Vec<u8>) -> Option<BitmapId> {
        let mut bitmap = Bitmap::from_image(BitmapId(0), name, bytes).ok()?;
        let id = BitmapId(self.doc.alloc_id());
        bitmap.id = id;
        self.doc.bitmaps.push(Arc::new(bitmap));
        Some(id)
    }
}

/// Imports a Moho project through `read` (paths inside the package). Text is laid out with
/// the fonts on this computer.
pub fn import_with(read: &Reader<'_>) -> Result<(Document, Report), MohoError> {
    import_from(read, None)
}

/// Imports a Moho project, laying text out with `fonts` (tests, and hosts with their own
/// font library).
pub fn import_with_fonts(read: &Reader<'_>, fonts: &Fonts) -> Result<(Document, Report), MohoError> {
    import_from(read, Some(fonts))
}

fn import_from(read: &Reader<'_>, fonts: Option<&Fonts>) -> Result<(Document, Report), MohoError> {
    let bytes = read(PROJECT_ENTRY)?.ok_or_else(|| MohoError::NotMoho(format!("no {PROJECT_ENTRY} in the package")))?;
    if bytes.len() > json::MAX_JSON_BYTES {
        return Err(MohoError::NotMoho("the project file is too large".into()));
    }
    let root: Value = serde_json::from_slice(&bytes).map_err(|e| MohoError::Json(e.to_string()))?;
    import_json(root, read, fonts)
}

/// The project JSON's name inside a `.moho` zip (fixed capitalisation).
pub const PROJECT_ENTRY: &str = "Project.mohoproj";

/// Imports a parsed project JSON (the bare `.mohoproj` form, or the zip's payload).
pub(crate) fn import_json(root: Value, read: &Reader<'_>, fonts: Option<&Fonts>) -> Result<(Document, Report), MohoError> {
    let version = json::integer(get(&root, "version")).unwrap_or(0);
    // The mime type identifies the family; 13.x-era files spell it without the dash, and
    // that spelling loads fine here (Moho 14.4 itself refuses it).
    match json::string(get(&root, "mime_type")) {
        Some(m) if m.starts_with("application/x") && m.contains("mohodoc") => {}
        None => {}
        Some(other) => return Err(MohoError::NotMoho(format!("not a Moho document (mime_type {other})"))),
    }
    let major = json::integer(get(&root, "major_version")).unwrap_or(1);
    if major > 1 {
        return Err(MohoError::NotMoho(format!("major_version {major} is not supported")));
    }
    if version > 1045 {
        return Err(MohoError::NotMoho(format!("version {version} is newer than this importer knows (1045); imported best-effort")));
    }
    let layers = array(get(&root, "layers")).ok_or_else(|| MohoError::NotMoho("the file has no layers".into()))?;
    if layers.is_empty() {
        // Moho itself refuses a document without layers (compat-gate g1).
        return Err(MohoError::NotMoho("the file has no layers".into()));
    }

    let mut doc = Document::default();
    doc.library.clear();
    doc.scenes.clear();

    // Stage from project_data: size, rate, background; the frame range becomes the one
    // keyframe's span (Moho frames are 1-based and inclusive).
    let empty = Value::Null;
    let pd = get(&root, "project_data").unwrap_or(&empty);
    let width = json::integer(get(pd, "width")).unwrap_or(1280).clamp(1, 8192) as u32;
    let height = json::integer(get(pd, "height")).unwrap_or(720).clamp(1, 8192) as u32;
    let fps = json::num(get(pd, "fps")).unwrap_or(24.0).clamp(1.0, 240.0) as f32;
    let background = json::color255(get(pd, "back_color")).unwrap_or_else(|| Rgba::hex(0xeaeaea));
    let start = json::integer(get(pd, "start_frame")).unwrap_or(1).max(1) as f64;
    let end = json::integer(get(pd, "end_frame")).unwrap_or(240).max(1);
    let duration = (end.saturating_sub(start as i64) + 1).clamp(1, MAX_FRAME as i64) as u32;
    doc.stage = Stage { width, height, fps, background };

    let dpi = MOHO_DPI;
    // F(p): x ↦ (x + W_in/2) · dpi, y ↦ (H_in/2 − y) · dpi — and W_in/2 · dpi is just half
    // the stage in pixels.
    let half_w = width as f32 * 0.5;
    let half_h = height as f32 * 0.5;
    let to_px = Affine::translate(half_w, half_h).mul(&Affine::scale(dpi, -dpi));
    let from_px = to_px.inverse().unwrap_or(Affine::IDENTITY);

    let frames = ((start as u64 - 1).clamp(0, MAX_FRAME as u64) as u32, (end as u64).clamp(1, MAX_FRAME as u64) as u32);
    let mut b = Builder { doc, report: Report::default(), fonts, faces: HashMap::new(), read, to_px, from_px, dpi, frame: start, duration, frames };

    // Render/document settings without a slot, counted when non-default.
    document_settings(&mut b.report, pd);
    let scene_anim = scene_animation(&mut b, get(&root, "animated_values").unwrap_or(&empty));
    if array(get(&root, "layercomps")).is_some_and(|s| !s.is_empty()) {
        b.report.skip("layer comps");
    }
    let actions = array(get(&root, "action_refs")).map(|a| a.len()).unwrap_or(0);
    if actions > 0 {
        *b.report.skipped.entry("smart bone actions".into()).or_default() += actions;
    }

    // The inline layer tree, flattened (bottom-first in, top-first out).
    let mut flat: Vec<FlatLayer> = Vec::new();
    let into = layer::Inherit { parent: None, world: Affine::IDENTITY, alpha: 1.0, hide: None };
    layer::walk(&mut b, layers, &into, 0, &mut flat);
    let mut layers_out: Vec<_> = flat.into_iter().map(|f| Arc::new(f.layer)).collect();
    if layers_out.is_empty() {
        layers_out.push(Arc::new(anicraft_doc::Layer::new(anicraft_doc::LayerId(b.doc.alloc_id()), "Layer 1")));
    }
    let mut timeline = Timeline { layers: layers_out };
    timeline.repair_masks();
    let mut scene = Scene { name: "Scene 1".into(), timeline, ..Scene::new("") };
    scene.camera = scene_anim.camera;
    scene.markers = scene_anim.markers;
    // Moho's project range only needs storing when it isn't just "everything that's there".
    let content = scene.timeline.frame_count();
    let (range_start, range_end) = scene_anim.frames;
    if (range_start, range_end) != (0, content) && range_start < range_end {
        scene.frames = Some((range_start, range_end));
    }
    b.doc.scenes.push(Arc::new(scene));

    // The sidecar: the untouched original, the `.maop` dialect's `maop_maohou` node, so a
    // later milestone re-projects bones, channels and effects without the source file.
    b.doc.maop_maohou = Some(Arc::new(MaohouSidecar {
        schema: 1,
        saved_by: format!("AniCraft {}", env!("CARGO_PKG_VERSION")),
        source: format!("mohoproj {version}"),
        raw: root,
    }));

    b.doc.sanitize();
    Ok((b.doc, b.report))
}

/// Document-level render settings without a slot, counted when they carry values.
fn document_settings(report: &mut Report, pd: &Value) {
    if json::integer(get(pd, "noise_grain")).is_some_and(|v| v != 0) || json::integer(get(pd, "pixelation")).is_some_and(|v| v != 0) {
        report.skip("document noise and pixelation settings");
    }
    if json::boolean(get(pd, "depth_of_field")).unwrap_or(false) {
        report.skip("document depth of field");
    }
    if json::integer(get(pd, "stereo_mode")).is_some_and(|v| v != 0) {
        report.skip("document stereo settings");
    }
    if !json::string(get(pd, "soundtrack")).is_none_or(|s| s.is_empty()) {
        report.skip("document soundtrack");
    }
}

/// The scene-level animation of stage 2: the camera channels, the timeline markers and the
/// project frame range (Moho's `animated_values` and `project_data`). The camera's z and
/// pan/tilt have no 2D slot — counted when they carry values.
fn scene_animation(b: &mut Builder, animated: &Value) -> SceneAnim {
    let mut camera = SceneCamera::default();
    let mut zed = 0.0f64;
    // The camera position is a *point* in Moho's inch space: it maps through the full
    // inch→pixel transform (canvas centre included), not the translation delta.
    let to_px = b.to_px;
    anim::read_channel(&mut camera.track, animated, "camera_track", &mut b.report, 2, |ch, i| {
        let (x, y, z) = channel::sample_vec3_at(ch, i).unwrap_or((0.0, 0.0, 0.0));
        zed = zed.max(((z as f64) - anim::CAMERA_Z_DEFAULT).abs());
        to_px.apply(Vec2::new(x, y))
    });
    if zed > 1e-4 {
        b.report.skip("camera depth (z) parallax (2D view ignores it)");
    }
    anim::read_channel(&mut camera.zoom, animated, "camera_zoom", &mut b.report, 1, |ch, i| channel::sample_f64_at(ch, i).unwrap_or(2.0) as f32);
    anim::read_channel(&mut camera.roll, animated, "camera_roll", &mut b.report, 1, |ch, i| -(channel::sample_f64_at(ch, i).unwrap_or(0.0)) as f32);
    if let Some(c) = get(animated, "camera_pan_tilt").and_then(Channel::parse)
        && (c.is_animated() || c.val().iter().any(|v| json::vec2(Some(v)).is_some_and(|p| p.length() > 1e-6)))
    {
        b.report.skip("camera pan/tilt (3D)");
    }
    // A camera whose channels hold only the defaults is no camera at all.
    let centre = Vec2::new(b.doc.stage.width as f32 * 0.5, b.doc.stage.height as f32 * 0.5);
    if camera.track.is_static() && camera.track.sample_or(0.0, centre) == centre {
        camera.track = DocChannel::default();
    }
    if camera.zoom.is_static() && (camera.zoom.sample_or(0.0, 2.0) - 2.0).abs() < 1e-6 {
        camera.zoom = DocChannel::default();
    }
    if camera.roll.is_static() && camera.roll.sample_or(0.0, 0.0).abs() < 1e-9 {
        camera.roll = DocChannel::default();
    }

    // Timeline markers: the string channel's keys, one per frame, empty labels dropped.
    let mut markers: Vec<(u32, String)> = Vec::new();
    if let Some(c) = get(animated, "timeline_markers").and_then(Channel::parse) {
        for i in 0..c.len() {
            let Some(frame) = c.frame(i) else { continue };
            let label = channel::sample_string_at(&c, i).unwrap_or_default();
            let label = label.trim();
            if label.is_empty() {
                continue;
            }
            markers.retain(|(f, _)| *f != frame);
            markers.push((frame, label.chars().take(256).collect()));
        }
        markers.sort_by_key(|(f, _)| *f);
    }

    SceneAnim { camera, markers, frames: b.frames }
}

/// The scene-level animation read out of one import.
struct SceneAnim {
    camera: SceneCamera,
    markers: Vec<(u32, String)>,
    frames: (u32, u32),
}

/// Imports from disk: a `.moho` package, a bare `.mohoproj` file, or an extracted folder
/// holding `Project.mohoproj`. Media referenced by image layers is looked up in the package
/// or beside the project file.
pub fn import_path(path: &std::path::Path) -> Result<(Document, Report), MohoError> {
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    if path.is_file() && ext.as_deref() == Some("moho") {
        let meta = std::fs::metadata(path)?;
        if meta.len() as usize > crate::zip::MAX_ENTRY * 4 {
            return Err(MohoError::NotMoho("the file is too large".into()));
        }
        let data = std::fs::read(path)?;
        let archive = Archive::open(&data)?;
        let read = |p: &str| -> Result<Option<Vec<u8>>, MohoError> {
            // Zip entry names may carry either slash; try the given name, then the other one.
            if let Some(b) = archive.read(p)? {
                return Ok(Some(b));
            }
            let swapped = if p.contains('\\') { p.replace('\\', "/") } else { p.replace('/', "\\") };
            Ok(archive.read(&swapped)?)
        };
        return import_with(&read);
    }
    // A bare project file, or a folder form.
    let dir = if path.is_dir() { path.to_path_buf() } else { path.parent().map(std::path::Path::to_path_buf).unwrap_or_default() };
    let read = |p: &str| -> Result<Option<Vec<u8>>, MohoError> {
        // Only plain relative paths beside the project; package-internal names go nowhere.
        if p.split('/').any(|c| c == ".." || c.is_empty()) || p.contains(':') {
            return Ok(None);
        }
        let full = dir.join(p);
        match std::fs::read(&full) {
            Ok(b) if b.len() <= json::MAX_JSON_BYTES => Ok(Some(b)),
            Ok(_) => Err(MohoError::NotMoho(format!("{p} is too large"))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    };
    let bytes = if path.is_dir() {
        let p = dir.join(PROJECT_ENTRY);
        match std::fs::read(&p) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(MohoError::NotMoho(format!("no {PROJECT_ENTRY} in {}", path.display()))),
            Err(e) => return Err(e.into()),
        }
    } else if ext.as_deref() == Some("mohoproj") {
        std::fs::read(path)?
    } else {
        return Err(MohoError::NotMoho(format!("{} is not a .moho or .mohoproj file", path.display())));
    };
    if bytes.len() > json::MAX_JSON_BYTES {
        return Err(MohoError::NotMoho("the project file is too large".into()));
    }
    let root: Value = serde_json::from_slice(&bytes).map_err(|e| MohoError::Json(e.to_string()))?;
    import_json(root, &read, None)
}
