//! File, edit-history and document commands.

use crate::params::*;
use crate::{CommandSpec, EngineError, Result, Session, format};
use anicraft_doc::{Document, MAX_STAGE_EDGE, Stage, sample};
use anicraft_render::{View, render};
use rayon::prelude::*;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;

pub fn specs() -> Vec<CommandSpec> {
    let mut v = vec![
        CommandSpec { id: "file.new", label: "New", menu: "File", shortcut: Some("Ctrl+N"), params: "{width?, height?, fps?}", mutates: false, run: new_doc },
        CommandSpec { id: "file.new_sample", label: "New from Sample", menu: "File", shortcut: None, params: "{which?: demo (default) | interactive (a menu with a Play button, for Test Movie)}", mutates: false, run: new_sample },
        CommandSpec { id: "file.open", label: "Open…", menu: "File", shortcut: Some("Ctrl+O"), params: "{path, unsaved? (opens it as unsaved work: a restored copy), untitled? (with no file name, so Save asks for one)}", mutates: false, run: open },
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
            params: "{path (.svg, .png, .jpg; or a .wav, .ogg or .mp3 sound: onto a new layer at the playhead), x?, y? (top-left on the stage; default: centred), cx?, cy? (centred on that point instead), layer? (new (default) | active: a one-layer drawing goes into the active layer's keyframe at the playhead)}",
            mutates: true,
            run: import_to_stage,
        },
        CommandSpec { id: "file.export_svg", label: "Export Frame as SVG…", menu: "File/Export", shortcut: None, params: "{path, frame? (in the current scene), confirm?}", mutates: false, run: export_svg },
        CommandSpec {
            id: "file.export_video",
            label: "Export Video…",
            menu: "File/Export",
            shortcut: None,
            params: "{path (.mp4 or .webm), from?, to? (movie frames), scale?, audio? (default true: the movie's sounds, Opus in WebM, AAC in MP4), ffmpeg? (path; else ANICRAFT_FFMPEG, else ffmpeg on the PATH), confirm?}",
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
        CommandSpec {
            id: "file.export_apng",
            label: "Export Animated PNG…",
            menu: "File/Export",
            shortcut: None,
            params: "{path (.png or .apng), from?, to? (movie frames: every scene in order), scale?, loop? (true (default): forever; false: once; or how many times it plays), background? (default false: transparent), confirm?}",
            mutates: false,
            run: |s, p| crate::anim::export(s, p, crate::anim::Kind::Apng),
        },
        CommandSpec {
            id: "file.export_webp",
            label: "Export Animated WebP…",
            menu: "File/Export",
            shortcut: None,
            params: "{path (.webp; lossless), from?, to? (movie frames: every scene in order), scale?, loop? (true (default): forever; false: once; or how many times it plays), background? (default false: transparent), confirm?}",
            mutates: false,
            run: |s, p| crate::anim::export(s, p, crate::anim::Kind::Webp),
        },
        CommandSpec {
            id: "file.export_lottie",
            label: "Export Lottie Animation…",
            menu: "File/Export",
            shortcut: None,
            params: "{path, from?, to? (movie frames: every scene in order), background? (default false: transparent), confirm?}",
            mutates: false,
            run: export_lottie,
        },
        CommandSpec {
            id: "file.export_html",
            label: "Export HTML Canvas…",
            menu: "File/Export",
            shortcut: None,
            params: "{path (.html: one self-contained page), title? (default: the file name), sound? (default true: embed the movie's sounds), fit? (default true: scale to the browser window), confirm?}",
            mutates: false,
            run: export_html,
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
    ];
    // In the File menu as in Animate: New from Template under New, Save as Template under
    // Save As (ADR 0065).
    for t in crate::cmds_template::specs() {
        let after = match t.id {
            "file.new_from_template" => "file.new_sample",
            "file.save_as_template" => "file.save_as",
            _ => "doc.info",
        };
        let at = v.iter().position(|c| c.id == after).map_or(v.len(), |i| i + 1);
        v.insert(at, t);
    }
    v
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

fn new_sample(s: &mut Session, p: &Value) -> Result<Value> {
    let doc = match opt_str(p, "which")? {
        None | Some("demo") => sample::demo(),
        Some("interactive") => sample::interactive(),
        Some(other) => return Err(EngineError::Param(format!("unknown sample '{other}' (demo or interactive)"))),
    };
    s.replace_document(doc, None);
    Ok(json!({"frames": s.frame_count()}))
}

/// Whether `path` is an Animate document: a `.fla` archive, an `.xfl` file or an XFL folder.
pub fn is_animate(path: &std::path::Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    matches!(ext.as_deref(), Some("fla" | "xfl")) || (crate::vfs::is_dir(path) && crate::vfs::is_file(&path.join("DOMDocument.xml")))
}

pub(crate) fn open(s: &mut Session, p: &Value) -> Result<Value> {
    let path = PathBuf::from(str_req(p, "path")?);
    let unsaved = opt_bool(p, "unsaved")?.unwrap_or(false);
    let untitled = opt_bool(p, "untitled")?.unwrap_or(false);
    if is_animate(&path) {
        // Imported, not adopted: the document is untitled, so Save asks for a new .anicraft
        // name and never writes over the Animate file.
        let fla = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("fla")) && crate::vfs::is_file(&path);
        let imported = if fla {
            // Through `vfs`, so an archive the browser handed over opens too (ADR 0036).
            if crate::vfs::len(&path)? > anicraft_xfl::MAX_FLA_BYTES {
                return Err(EngineError::Format(format!("{} is too large to open", path.display())));
            }
            anicraft_xfl::import_fla(&crate::vfs::read(&path)?)
        } else if crate::vfs::in_memory() {
            return Err(EngineError::Param("an XFL document is a folder, which a web browser can't hand over: open the .fla instead".into()));
        } else {
            anicraft_xfl::import_path(&path)
        };
        let (mut doc, report) = imported.map_err(|e| EngineError::Format(format!("{}: {e}", path.display())))?;
        // Raw artwork arrives one shape per style: each keyframe's becomes one merged drawing,
        // as it was in Animate (ADR 0042).
        let merged = crate::merge::merge_document(&mut doc);
        s.replace_document(doc, None);
        // Open Recent reopens the Animate file (ADR 0059).
        s.recent.add(&path);
        let mut info = s.info();
        info["imported"] = json!(path.display().to_string());
        info["report"] = json!(report.summary());
        info["skipped"] = json!(report.skipped);
        info["kept"] = json!(report.kept);
        info["runs"] = json!(report.runs);
        info["missing_fonts"] = json!(report.missing_fonts);
        info["merged_keyframes"] = json!(merged);
        return Ok(info);
    }
    let doc = format::load(&path)?;
    s.replace_document(doc, (!untitled).then(|| path.clone()));
    if unsaved {
        s.mark_unsaved();
    } else if !untitled {
        s.recent.add(&path);
    }
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
    s.recent.add(&path);
    Ok(json!({"saved": path.display().to_string()}))
}

pub(crate) fn scale_param(p: &Value) -> Result<f32> {
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
    crate::vfs::write(&path, &bytes)?;
    Ok(json!({"written": path.display().to_string(), "bytes": bytes.len(), "frame": frame}))
}

/// Largest file Import to Stage reads.
const MAX_IMPORT_BYTES: u64 = 256 * 1024 * 1024;

/// Import to Stage: the drawing's shapes and text become editable elements on a new layer
/// named after the file, in a keyframe at the playhead, selected, as one undo step. A drawing
/// with clip paths or masks becomes a folder named after the file, with mask layers over the
/// artwork they clip (ADR 0021). Centred on the stage unless `x`/`y` give its top-left corner.
pub(crate) fn import_to_stage(s: &mut Session, p: &Value) -> Result<Value> {
    let path = PathBuf::from(str_req(p, "path")?);
    if crate::cmds_sound::is_sound(&path) {
        return crate::cmds_sound::import_to_stage(s, &path);
    }
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    if !matches!(ext.as_deref(), Some("svg" | "png" | "jpg" | "jpeg")) {
        return Err(EngineError::Param("Import to Stage reads SVG drawings, PNG or JPEG images, and WAV, Ogg Vorbis or MP3 sounds".into()));
    }
    let size = crate::vfs::len(&path)?;
    if size > MAX_IMPORT_BYTES {
        return Err(EngineError::Format(format!("{} is too large to import", path.display())));
    }
    let bytes = crate::vfs::read(&path)?;
    let (drawing, bitmap) = if ext.as_deref() == Some("svg") {
        (anicraft_svg::import(&bytes).map_err(|e| EngineError::Format(format!("{}: {e}", path.display())))?, None)
    } else {
        let drawing = import_image(s, &path, bytes)?;
        (drawing, s.doc.bitmaps.last().map(|b| b.id.0))
    };
    if drawing.is_empty() {
        return Err(EngineError::State(format!("{} has no shapes AniCraft can import yet. {}", path.display(), drawing.report.summary())));
    }
    let stage = s.doc.stage;
    // `cx`/`cy` centre it on a point (a drop, ADR 0059); `x`/`y` give its top-left corner.
    let x = match opt_f32(p, "cx")? {
        Some(cx) => cx - drawing.width * 0.5,
        None => opt_f32(p, "x")?.unwrap_or((stage.width as f32 - drawing.width) * 0.5),
    };
    let y = match opt_f32(p, "cy")? {
        Some(cy) => cy - drawing.height * 0.5,
        None => opt_f32(p, "y")?.unwrap_or((stage.height as f32 - drawing.height) * 0.5),
    };
    let onto_active = match opt_str(p, "layer")? {
        None | Some("new") => false,
        Some("active") => true,
        Some(other) => return Err(EngineError::Param(format!("param 'layer' must be new or active, not '{other}'"))),
    };
    if !(x.is_finite() && y.is_finite()) {
        return Err(EngineError::Param("'x' and 'y' must be numbers".into()));
    }
    let place = anicraft_geom::Affine::translate(x, y);
    let name: String = path.file_stem().map_or_else(|| "Imported".into(), |n| n.to_string_lossy().chars().take(128).collect());
    let frame = s.frame;
    let end = s.frame_count().max(frame + 1);
    let elements = |s: &mut Session, list: Vec<anicraft_svg::ImportedElement>| -> Vec<Arc<anicraft_doc::Element>> {
        list.into_iter()
            .map(|e| {
                let id = anicraft_doc::ElementId(s.doc.alloc_id());
                Arc::new(anicraft_doc::Element { id, name: e.name, transform: place.mul(&e.transform), color: anicraft_doc::ColorEffect::opacity(e.alpha), kind: e.kind })
            })
            .collect()
    };
    // Empty before the playhead, the drawing from the playhead to the end of the timeline.
    let keys = |els: Vec<Arc<anicraft_doc::Element>>| -> Vec<Arc<anicraft_doc::Keyframe>> {
        let mut key = anicraft_doc::Keyframe::new(frame, end - frame);
        key.elements = els;
        if frame == 0 { vec![Arc::new(key)] } else { vec![Arc::new(anicraft_doc::Keyframe::new(0, frame)), Arc::new(key)] }
    };
    let texts = drawing.elements().filter(|e| matches!(e.kind, anicraft_doc::ElementKind::Text(_))).count();
    let count = drawing.elements().count();
    let masks = drawing.layers.iter().filter(|l| l.mask.is_some()).count();
    let (width, height, report) = (drawing.width, drawing.height, drawing.report);
    let mut layers = drawing.layers;
    let (layer_id, ids, made, folder) = if onto_active && layers.len() == 1 && masks == 0 {
        // Onto the active layer's keyframe at the playhead, as a drop on the stage puts it.
        let els = elements(s, layers.pop().map(|l| l.elements).unwrap_or_default());
        let ids: Vec<anicraft_doc::ElementId> = els.iter().map(|e| e.id).collect();
        s.editable_keyframe()?.elements.extend(els);
        (s.layer, ids, 0, None)
    } else if layers.len() == 1 && masks == 0 {
        // One layer, named after the file, as Import to Stage always made.
        crate::cmds_timeline::new_layer(s, &json!({"name": name}))?;
        let els = elements(s, layers.pop().map(|l| l.elements).unwrap_or_default());
        let ids: Vec<anicraft_doc::ElementId> = els.iter().map(|e| e.id).collect();
        let layer = s.layer_mut(None)?;
        layer.keyframes = keys(els);
        layer.normalize();
        (layer.id, ids, 1, None)
    } else {
        // Clips and masks (ADR 0021): a folder named after the file holding, top first, each
        // run of artwork, with a mask layer over the runs that are masked.
        let made = layers.len() + masks + 1;
        let tl = s.timeline();
        if tl.layers.len() + made > 1000 {
            return Err(EngineError::State(format!("importing {} would make {made} layers; a timeline holds at most 1000", path.display())));
        }
        // Above the active layer, beside it; above its whole mask group when it is masked.
        let mut at = s.active_layer_index().unwrap_or(0);
        let mut parent = tl.layers.get(at).and_then(|l| l.parent);
        if let Some(m) = parent.and_then(|pid| tl.layer(pid)).filter(|m| m.kind == anicraft_doc::LayerKind::Mask) {
            parent = m.parent;
            at = tl.layers.iter().position(|l| l.id == m.id).unwrap_or(at);
        }
        let palette = [0x4fa3ff, 0xff6b4a, 0x66bb6a, 0xffd54a, 0xba68c8, 0x4dd0e1, 0xff8a65];
        let mut color = tl.layers.len();
        let mut new_layer = |s: &mut Session, name: String, kind: anicraft_doc::LayerKind, parent: Option<anicraft_doc::LayerId>| {
            let mut l = anicraft_doc::Layer::new(anicraft_doc::LayerId(s.doc.alloc_id()), name);
            l.kind = kind;
            l.parent = parent;
            color += 1;
            l.color = anicraft_doc::Rgba::hex(palette[color % palette.len()]);
            l
        };
        let mut folder = new_layer(s, name, anicraft_doc::LayerKind::Folder, parent);
        folder.keyframes.clear();
        let fid = folder.id;
        let mut stack = vec![folder];
        let mut top: Option<(anicraft_doc::LayerId, Vec<anicraft_doc::ElementId>)> = None;
        for (n, l) in layers.into_iter().enumerate().rev() {
            let mut inside = Some(fid);
            if let Some(mask) = l.mask {
                let label = if mask.name.is_empty() { "Mask".to_string() } else { mask.name.clone() };
                let mut ml = new_layer(s, label, anicraft_doc::LayerKind::Mask, Some(fid));
                // Mask layers are locked, as Animate keeps them: the mask shows, its edge hides.
                ml.locked = true;
                ml.keyframes = keys(elements(s, mask.elements));
                inside = Some(ml.id);
                stack.push(ml);
            }
            let mut cl = new_layer(s, format!("Layer {}", n + 1), anicraft_doc::LayerKind::Normal, inside);
            let els = elements(s, l.elements);
            if top.is_none() {
                top = Some((cl.id, els.iter().map(|e| e.id).collect()));
            }
            cl.keyframes = keys(els);
            stack.push(cl);
        }
        let tl = crate::editing::edited_mut(&mut s.doc, s.scene, &s.editing);
        for (k, l) in stack.into_iter().enumerate() {
            tl.layers.insert((at + k).min(tl.layers.len()), Arc::new(l));
        }
        tl.repair_masks();
        let (lid, ids) = top.unwrap_or((fid, Vec::new()));
        s.layer = lid;
        (lid, ids, made, Some(fid.0))
    };
    s.select_only(ids.iter().copied());
    Ok(json!({
        "layer": layer_id.0,
        "folder": folder,
        "layers": made,
        "masks": masks,
        "texts": texts,
        "elements": count,
        "bitmap": bitmap,
        "width": width,
        "height": height,
        "report": report.summary(),
        "skipped": report.skipped,
        "missing_fonts": report.missing_fonts,
    }))
}

/// An SVG drawing read from `path` (size-checked), with at least one shape.
pub(crate) fn read_svg(path: &std::path::Path) -> Result<anicraft_svg::SvgImport> {
    if crate::vfs::len(path)? > MAX_IMPORT_BYTES {
        return Err(EngineError::Format(format!("{} is too large to import", path.display())));
    }
    let drawing = anicraft_svg::import(&crate::vfs::read(path)?).map_err(|e| EngineError::Format(format!("{}: {e}", path.display())))?;
    if drawing.is_empty() {
        return Err(EngineError::State(format!("{} has no shapes AniCraft can import yet. {}", path.display(), drawing.report.summary())));
    }
    Ok(drawing)
}

/// Whether a drawing fits on one layer (one run of artwork, nothing masked).
pub(crate) fn one_layer(d: &anicraft_svg::SvgImport) -> bool {
    d.layers.len() == 1 && d.layers.iter().all(|l| l.mask.is_none())
}

/// Import to Library with an SVG (ADR 0065): a graphic symbol named after the file (a free
/// name at the Library's top level) holding the drawing's shapes, its registration point at
/// the drawing's centre. Clips and masks keep their mask layers inside the symbol.
pub(crate) fn import_svg_symbol(s: &mut Session, path: &std::path::Path, name: Option<&str>) -> Result<Value> {
    use anicraft_doc::{Layer, LayerId, LayerKind, Symbol, SymbolId, SymbolKind, Timeline};
    let drawing = read_svg(path)?;
    let stem: String = path.file_stem().map_or_else(|| "Drawing".into(), |n| n.to_string_lossy().chars().take(128).collect());
    let name = s.doc.unique_name(None, name.map_or(stem.as_str(), str::trim));
    let place = anicraft_geom::Affine::translate(-drawing.width * 0.5, -drawing.height * 0.5);
    let elements = |s: &mut Session, list: Vec<anicraft_svg::ImportedElement>| -> Vec<Arc<anicraft_doc::Element>> {
        list.into_iter()
            .map(|e| Arc::new(anicraft_doc::Element { id: anicraft_doc::ElementId(s.doc.alloc_id()), name: e.name, transform: place.mul(&e.transform), color: anicraft_doc::ColorEffect::opacity(e.alpha), kind: e.kind }))
            .collect()
    };
    let one_key = |els: Vec<Arc<anicraft_doc::Element>>| {
        let mut k = anicraft_doc::Keyframe::new(0, 1);
        k.elements = els;
        vec![Arc::new(k)]
    };
    let count = drawing.elements().count();
    let (width, height, report) = (drawing.width, drawing.height, drawing.report);
    let runs = drawing.layers.len();
    // Top first: the drawing lists its runs in paint order (bottom first).
    let mut layers: Vec<Arc<Layer>> = Vec::new();
    for (n, l) in drawing.layers.into_iter().enumerate().rev() {
        let mut parent = None;
        if let Some(mask) = l.mask {
            let mut ml = Layer::new(LayerId(s.doc.alloc_id()), if mask.name.is_empty() { "Mask".to_string() } else { mask.name.clone() });
            ml.kind = LayerKind::Mask;
            ml.locked = true;
            ml.keyframes = one_key(elements(s, mask.elements));
            parent = Some(ml.id);
            layers.push(Arc::new(ml));
        }
        let mut cl = Layer::new(LayerId(s.doc.alloc_id()), if runs == 1 { "Layer 1".to_string() } else { format!("Layer {}", n + 1) });
        cl.parent = parent;
        cl.keyframes = one_key(elements(s, l.elements));
        layers.push(Arc::new(cl));
    }
    let mut timeline = Timeline { layers };
    timeline.repair_masks();
    let id = SymbolId(s.doc.alloc_id());
    s.doc.library.push(Arc::new(Symbol { id, name: name.clone(), kind: SymbolKind::Graphic, linkage: None, scale_grid: None, scale_grid_kept: None, timeline }));
    Ok(json!({"symbol": id.0, "name": name, "elements": count, "layers": timeline_len(s, id), "width": width, "height": height, "report": report.summary(), "skipped": report.skipped}))
}

fn timeline_len(s: &Session, id: anicraft_doc::SymbolId) -> usize {
    s.doc.library.iter().find(|x| x.id == id).map_or(0, |x| x.timeline.layers.len())
}

/// A PNG or JPEG as a bitmap in the document, and the one shape that shows it: a rectangle of
/// the image's size filled with it. JPEGs are kept as PNG (lossless, so nothing degrades).
fn import_image(s: &mut Session, path: &std::path::Path, bytes: Vec<u8>) -> Result<anicraft_svg::SvgImport> {
    let id = anicraft_doc::BitmapId(s.doc.alloc_id());
    let name: String = path.file_name().map_or_else(|| "image".into(), |n| n.to_string_lossy().chars().take(128).collect());
    let bitmap = anicraft_doc::Bitmap::from_image(id, name.clone(), bytes).map_err(|e| EngineError::Format(format!("{}: {e}", path.display())))?;
    let (w, h) = (bitmap.width as f32, bitmap.height as f32);
    s.doc.bitmaps.push(std::sync::Arc::new(bitmap));
    let shape = anicraft_doc::Shape {
        path: std::sync::Arc::new(anicraft_geom::Path::rect(0.0, 0.0, w, h)),
        fill: Some(anicraft_doc::Paint::Bitmap { bitmap: id, repeat: false, matrix: anicraft_geom::Affine::IDENTITY }),
        stroke: None,
        fill_rule: anicraft_geom::FillRule::NonZero,
        merge: false,
    };
    Ok(anicraft_svg::SvgImport::single(w, h, vec![anicraft_svg::ImportedElement { name, transform: anicraft_geom::Affine::IDENTITY, alpha: 1.0, kind: anicraft_doc::ElementKind::Shape(shape) }]))
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
    crate::vfs::write(&path, svg.as_bytes())?;
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
    // A publish counts the frames, and stops between them when cancelled (ADR 0060).
    let job = s.job.as_deref();
    let frames: Vec<anicraft_render::Pixmap> = (from..=to)
        .into_par_iter()
        .map(|f| {
            let (pm, _) = render(&anicraft_doc::evaluate(doc, f), &view)?;
            crate::publish::tick(job, 1)?;
            Ok(pm)
        })
        .collect::<Result<_>>()?;
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
    crate::vfs::write(&path, &png)?;
    crate::vfs::write(&json_path, text)?;
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
    if crate::vfs::in_memory() {
        // ffmpeg writes the file itself, and a web browser can't start it (ADR 0036).
        return Err(EngineError::State("video export runs ffmpeg, which can't run in a web browser: export an animated GIF, a PNG sequence or an HTML page instead".into()));
    }
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    // Sound is encoded by the same ffmpeg: Opus in WebM (royalty-free), AAC in MP4.
    let (codec, audio_codec): (&[&str], &[&str]) = match ext.as_deref() {
        Some("webm") => (&["-c:v", "libvpx-vp9", "-b:v", "0", "-crf", "30", "-row-mt", "1"], &["-c:a", "libopus", "-b:a", "160k"]),
        Some("mp4" | "m4v" | "mov") => (&["-c:v", "libx264", "-crf", "18", "-preset", "medium", "-movflags", "+faststart"], &["-c:a", "aac", "-b:a", "192k"]),
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
    // The soundtrack (when the movie has sounds) goes to ffmpeg as a WAV beside the frames.
    let track = if opt_bool(p, "audio")?.unwrap_or(true) && anicraft_doc::sound::has_sounds(&s.doc) { Some(crate::cmds_sound::temp_track(s, from, to)?) } else { None };
    let mut args: Vec<&std::ffi::OsStr> = Vec::new();
    for a in ["-hide_banner", "-loglevel", "error", "-y", "-f", "rawvideo", "-pix_fmt", "rgba", "-s", &size, "-r", &rate, "-i", "-"] {
        args.push(a.as_ref());
    }
    match &track {
        Some(t) => {
            args.extend(["-i".as_ref(), t.0.as_os_str()]);
            args.extend(["-map", "0:v", "-map", "1:a"].map(std::ffi::OsStr::new));
        }
        None => args.push("-an".as_ref()),
    }
    for a in ["-vf", "pad=ceil(iw/2)*2:ceil(ih/2)*2", "-pix_fmt", "yuv420p"] {
        args.push(a.as_ref());
    }
    args.extend(codec.iter().map(std::ffi::OsStr::new));
    if track.is_some() {
        args.extend(audio_codec.iter().map(std::ffi::OsStr::new));
    }
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
    let job = s.job.as_deref();
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
            if let Err(e) = crate::publish::tick(job, batch.len() as u64) {
                failure = Some(e);
                break;
            }
        }
    }
    if matches!(failure, Some(EngineError::Cancelled)) {
        // Cancelled while publishing: ffmpeg stops, and its unfinished file goes.
        let _ = child.kill();
    }
    let status = child.wait()?;
    let log = messages.join().unwrap_or_default();
    if let Some(e) = failure {
        if matches!(e, EngineError::Cancelled) {
            let _ = crate::vfs::remove_file(&path);
        }
        return Err(e);
    }
    if !status.success() || written != to - from + 1 {
        let tail: Vec<&str> = log.lines().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
        return Err(EngineError::Format(format!("ffmpeg could not make the video ({status}): {}", if tail.is_empty() { "no message".to_string() } else { tail.join(" | ") })));
    }
    let bytes = crate::vfs::len(&path).unwrap_or(0);
    Ok(json!({"written": path.display().to_string(), "frames": written, "bytes": bytes, "width": w, "height": h, "fps": fps, "audio": track.is_some()}))
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
    crate::vfs::create_dir_all(&dir)?;
    // Frames are independent: render and encode them in parallel (each render is itself
    // row-parallel; rayon balances both levels), a batch at a time so memory stays bounded.
    // Output is identical to a serial export. Files are written here, on the command's thread,
    // where `vfs` knows whether they go to disk or to the browser (ADR 0036).
    let doc = &s.doc;
    let job = s.job.as_deref();
    let mut files = 0usize;
    let frames: Vec<u32> = (from..=to).collect();
    for batch in frames.chunks(64) {
        let pngs = batch.par_iter().map(|&f| render_png(doc, f, scale)).collect::<Result<Vec<_>>>()?;
        for (f, png) in batch.iter().zip(pngs) {
            crate::vfs::write(&dir.join(format!("frame_{:05}.png", f + 1)), png)?;
            files += 1;
        }
        crate::publish::tick(job, batch.len() as u64)?;
    }
    Ok(json!({"frames": files, "dir": dir.display().to_string()}))
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

/// The movie as a Lottie (JSON) animation, frame-accurate (see `anicraft_render::lottie`).
fn export_lottie(s: &mut Session, p: &Value) -> Result<Value> {
    let path = PathBuf::from(str_req(p, "path")?);
    let last = s.doc.frame_count().saturating_sub(1);
    let from = opt_u32(p, "from")?.unwrap_or(0).min(last);
    let to = opt_u32(p, "to")?.unwrap_or(last).min(last);
    if to < from {
        return Err(EngineError::Param("'to' must not be before 'from'".into()));
    }
    let background = opt_bool(p, "background")?.unwrap_or(false);
    let name: String = path.file_stem().map_or_else(|| "AniCraft".into(), |n| n.to_string_lossy().chars().take(128).collect());
    let out = anicraft_render::lottie::to_lottie(&s.doc, &name, from, to, background);
    crate::vfs::write(&path, out.json.as_bytes())?;
    let summary = if out.report.is_empty() {
        String::new()
    } else {
        format!("Approximated or left out: {}.", out.report.iter().map(|(k, n)| format!("{k} on {n}")).collect::<Vec<_>>().join(", "))
    };
    Ok(json!({"written": path.display().to_string(), "frames": to - from + 1, "layers": out.layers, "bytes": out.json.len(), "report": summary, "notes": out.report}))
}

/// The movie as one web page that plays in a browser, buttons and frame actions included
/// (see `anicraft_render::html`, ADR 0028).
fn export_html(s: &mut Session, p: &Value) -> Result<Value> {
    let path = PathBuf::from(str_req(p, "path")?);
    let stem: String = path.file_stem().map_or_else(|| "AniCraft movie".into(), |n| n.to_string_lossy().chars().take(128).collect());
    let title = match opt_str(p, "title")? {
        Some(t) if t.chars().count() > 256 => return Err(EngineError::Param("param 'title' is at most 256 characters".into())),
        Some(t) if !t.trim().is_empty() => t.to_string(),
        _ => stem,
    };
    let opts = anicraft_render::html::HtmlOptions { title, sound: opt_bool(p, "sound")?.unwrap_or(true), fit: opt_bool(p, "fit")?.unwrap_or(true) };
    let started = web_time::Instant::now();
    let out = anicraft_render::html::to_html(&s.doc, &opts).map_err(EngineError::Param)?;
    crate::vfs::write(&path, out.html.as_bytes())?;
    let summary = if out.report.is_empty() {
        String::new()
    } else {
        format!("Approximated or left out: {}.", out.report.iter().map(|(k, n)| format!("{k} on {n}")).collect::<Vec<_>>().join(", "))
    };
    let st = out.stats;
    Ok(json!({
        "written": path.display().to_string(),
        "bytes": out.html.len(),
        "frames": s.doc.frame_count(),
        "scenes": s.doc.scenes.len(),
        "timelines": st.timelines,
        "stored_frames": st.contents,
        "paths": st.paths,
        "images": st.images,
        "sounds": st.sounds,
        "data_bytes": st.data_bytes,
        "code_bytes": st.code_bytes,
        "ms": started.elapsed().as_millis() as u64,
        "report": summary,
        "notes": out.report,
    }))
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
    let job = s.job.as_deref();
    let delays = gif_delays(n as usize, doc.stage.fps);
    let frames: Vec<gif::Frame<'static>> = (from..to.saturating_add(1))
        .into_par_iter()
        .zip(delays.into_par_iter())
        .map(|(f, delay)| {
            let (pm, _) = render(&anicraft_doc::evaluate(doc, f), &view)?;
            let mut rgba = pm.to_rgba8();
            let mut frame = gif::Frame::from_rgba_speed(w, h, &mut rgba, 10);
            frame.delay = delay;
            crate::publish::tick(job, 1)?;
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
    crate::vfs::write(&path, &bytes)?;
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
