//! Exporter plug-ins in the desktop app (ADR 0088): their formats in File › Export, below the
//! built-in ones, and the export window: the plug-in's options (generated from its manifest,
//! as a command plug-in's dialog is), the frames, the file, then a progress bar and Cancel.
//!
//! The export runs on a worker thread with a snapshot of the document (in a browser, where
//! there are no threads, in the frame), so the editor keeps working while it runs. Replacing a
//! file that is already there asks first, in the window (ADR 0004).

use crate::a11y::Named;
use crate::app::App;
use crate::theme::{self, FORM_LABEL_DOCK, T, TARGET, Type, form_row, text};
use anicraft_engine::plugin_export::{self, Job};
use anicraft_engine::plugins::Plugin;
use anicraft_engine::publish::Control;
use serde_json::{Map, Value, json};
use std::path::PathBuf;
use std::sync::Arc;

/// The open export window.
pub struct Export {
    /// The exporter plug-in's id.
    pub id: String,
    values: Map<String, Value>,
    /// Movie frames, 0-based (shown 1-based, as in the timeline).
    from: u32,
    to: u32,
    path: String,
    /// The file is there: waiting for Replace.
    asking: bool,
    running: Option<Running>,
    error: Option<String>,
}

/// What the worker gives back: the export's result, or whether it was cancelled and why not.
type Outcome = Result<Value, (bool, String)>;

struct Running {
    control: Arc<Control>,
    worker: std::thread::JoinHandle<Outcome>,
}

/// The plug-in's format name and extension.
fn format_of(p: &Plugin) -> (String, String) {
    let m = p.manifest();
    m.format.as_ref().map_or_else(|| (m.name.clone(), String::new()), |f| (f.name.clone(), f.extension.clone()))
}

/// File › Export's plug-in formats, after the built-in ones.
pub fn menu_items(app: &mut App, ui: &mut egui::Ui) {
    let list = plugin_export::exporters(&app.s);
    if list.is_empty() {
        return;
    }
    ui.separator();
    for p in list {
        let (name, ext) = format_of(&p);
        // The plug-in's format name is its own text, shown as written.
        let r = ui.button(format!("{name}…"));
        let tip = if p.manifest().description.is_empty() { tr!("Plug-in · .{ext}", ext = ext) } else { tr!("{description} (plug-in · .{ext})", description = p.manifest().description, ext = ext) };
        if r.on_hover_text(tip).clicked() {
            ui.close();
            open(app, p.id());
        }
    }
}

/// Opens the export window for exporter `id`, with its default options, the whole movie and
/// a file named after the document beside it.
pub fn open(app: &mut App, id: &str) {
    let Some(p) = app.s.plugins.get(id) else { return };
    let (_, ext) = format_of(&p);
    let stem = app.s.path.as_deref().and_then(std::path::Path::file_stem).map_or_else(|| "animation".to_string(), |n| n.to_string_lossy().into_owned());
    let dir = app.s.path.as_deref().and_then(std::path::Path::parent).map(std::path::Path::to_path_buf).unwrap_or_default();
    let path = dir.join(format!("{stem}.{ext}")).display().to_string();
    app.playing = false;
    app.plugin_export = Some(Export { id: id.to_string(), values: p.manifest().defaults(), from: 0, to: app.s.doc.frame_count().saturating_sub(1), path, asking: false, running: None, error: None });
}

fn params(e: &Export, confirm: bool) -> Value {
    json!({"format": e.id, "path": e.path, "from": e.from, "to": e.to, "options": e.values, "confirm": confirm})
}

/// Starts the export (after Replace, when the file is there).
fn start(app: &mut App, confirm: bool) {
    let Some(e) = app.plugin_export.as_mut() else { return };
    e.error = None;
    let p = params(e, confirm);
    if !confirm && app.s.confirmation("file.export_plugin", &p).is_some() {
        e.asking = true;
        return;
    }
    e.asking = false;
    if crate::platform::WEB {
        // No threads in a browser: the export runs now and its file becomes a download.
        if let Some(v) = app.run("file.export_plugin", p) {
            done(app, &v);
        }
        return;
    }
    let (plugin, job) = match plugin_export::plan(&app.s, &p) {
        Ok(x) => x,
        Err(err) => {
            e.error = Some(err.to_string());
            return;
        }
    };
    let control = Arc::new(Control::default());
    match spawn(plugin, app.s.doc.clone(), job, control.clone()) {
        Ok(worker) => e.running = Some(Running { control, worker }),
        Err(err) => e.error = Some(err),
    }
}

fn spawn(plugin: Arc<Plugin>, doc: anicraft_engine::doc::Document, job: Job, control: Arc<Control>) -> Result<std::thread::JoinHandle<Outcome>, String> {
    std::thread::Builder::new()
        .name("anicraft-plugin-export".into())
        .spawn(move || plugin_export::run(&plugin, &doc, &job, Some(&control)).map_err(|e| (matches!(e, anicraft_engine::EngineError::Cancelled), e.to_string())))
        .map_err(|e| e.to_string())
}

fn done(app: &mut App, v: &Value) {
    let path = v["written"].as_str().unwrap_or("").to_string();
    let kb = v["bytes"].as_u64().unwrap_or(0).div_ceil(1024);
    app.plugin_export = None;
    app.status_ok(tr!("Exported {path} ({size} KB)", path = path, size = kb));
}

/// Collects a finished worker (every frame while one runs).
pub fn tick(app: &mut App, ctx: &egui::Context) {
    let Some(e) = app.plugin_export.as_mut() else { return };
    let Some(r) = &e.running else { return };
    ctx.request_repaint_after(std::time::Duration::from_millis(50));
    if !r.worker.is_finished() {
        return;
    }
    let Some(r) = e.running.take() else { return };
    match r.worker.join() {
        Ok(Ok(v)) => done(app, &v),
        Ok(Err((true, _))) => {
            e.error = None;
            app.status_ok(tr!("Export cancelled: nothing was written"));
        }
        Ok(Err((false, msg))) => e.error = Some(msg),
        Err(_) => e.error = Some(tr!("The export stopped unexpectedly").to_string()),
    }
}

/// Picks the file with the platform's dialog.
fn browse(e: &mut Export, plugin: &Plugin) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let (name, ext) = format_of(plugin);
        let current = PathBuf::from(&e.path);
        let file = current.file_name().map_or_else(|| format!("animation.{ext}"), |f| f.to_string_lossy().into_owned());
        if let Some(p) = crate::platform::save_dialog(&tr!("Export {format}", format = name), &name, &ext, current.parent(), &file) {
            e.path = p.display().to_string();
        }
    }
    #[cfg(target_arch = "wasm32")]
    let _ = (e, plugin);
}

/// The export window, when open.
pub fn show(app: &mut App, ctx: &egui::Context) {
    tick(app, ctx);
    let Some(mut e) = app.plugin_export.take() else { return };
    let Some(p) = app.s.plugins.get(&e.id) else { return };
    let (name, _) = format_of(&p);
    let frames = app.s.doc.frame_count().max(1);
    let title = tr!("Export {format}", format = name);
    let mut open = true;
    let (mut go, mut replace, mut close) = (false, false, false);
    let running = e.running.is_some();
    let at = ctx.content_rect().center() - egui::vec2(200.0, 180.0);
    let shown = egui::Window::new(title.clone()).id(egui::Id::new("plugin_export")).open(&mut open).default_pos(at).default_width(400.0).resizable(false).collapsible(false).show(ctx, |ui| {
        let m = p.manifest();
        let by = [m.author.as_str(), m.version.as_str()].iter().filter(|s| !s.is_empty()).copied().collect::<Vec<_>>().join(" · ");
        ui.label(text(Type::Caption, if by.is_empty() { tr!("Plug-in").to_string() } else { tr!("Plug-in · {by}", by = by) }));
        if !m.description.is_empty() {
            ui.label(text(Type::Label, m.description.clone()));
        }
        ui.add_space(8.0);
        ui.add_enabled_ui(!running, |ui| {
            for (key, spec) in &m.params {
                let v = e.values.entry(key.clone()).or_insert(Value::Null);
                form_row(ui, FORM_LABEL_DOCK + 40.0, &spec.label, |ui| {
                    crate::plugins_ui::control(ui, &spec.label, &spec.kind, v);
                });
            }
            form_row(ui, FORM_LABEL_DOCK + 40.0, tr!("Frames"), |ui| {
                let (mut a, mut b) = (e.from + 1, e.to + 1);
                ui.add(egui::DragValue::new(&mut a).range(1..=frames)).named(tr!("First frame"));
                ui.label(text(Type::Label, tr!("to")));
                ui.add(egui::DragValue::new(&mut b).range(1..=frames)).named(tr!("Last frame"));
                e.from = a.saturating_sub(1);
                e.to = b.saturating_sub(1).max(e.from);
                if ui.small_button(tr!("All")).on_hover_text(tr!("Every frame of the movie")).clicked() {
                    e.from = 0;
                    e.to = frames - 1;
                }
            });
            form_row(ui, FORM_LABEL_DOCK + 40.0, tr!("File"), |ui| {
                if ui.add(egui::TextEdit::singleline(&mut e.path).font(theme::font(Type::Path)).desired_width(200.0)).named(tr!("Export to")).changed() {
                    e.asking = false;
                }
                if !crate::platform::WEB && ui.button(tr!("Browse…")).clicked() {
                    browse(&mut e, &p);
                }
            });
        });
        if let Some(r) = &e.running {
            ui.add_space(6.0);
            let (done, total) = r.control.progress();
            let frac = if total > 0 { (done as f32 / total as f32).clamp(0.0, 1.0) } else { 0.0 };
            ui.add(egui::ProgressBar::new(frac).desired_width(ui.available_width()).text(text(Type::DataSmall, tr!("Frame {done} of {total}", done = done, total = total))));
        }
        if e.asking {
            ui.add_space(6.0);
            ui.label(text(Type::Label, tr!("A file with this name is already there. Replace it? This can't be undone.")).color(T.danger));
        }
        if let Some(err) = &e.error {
            ui.add_space(6.0);
            ui.label(text(Type::Caption, err.clone()).color(T.error));
        }
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            if running {
                let stopping = e.running.as_ref().is_some_and(|r| r.control.cancelled());
                if ui.add_enabled(!stopping, egui::Button::new(tr!("Cancel")).min_size(egui::vec2(96.0, TARGET + 4.0))).on_hover_text(tr!("Stop the export; nothing is written")).clicked()
                    && let Some(r) = &e.running
                {
                    r.control.cancel();
                }
                if stopping {
                    ui.label(text(Type::Caption, tr!("Stopping…")));
                }
                return;
            }
            if ui.add(egui::Button::new(tr!("Close")).min_size(egui::vec2(96.0, TARGET + 4.0))).clicked() {
                close = true;
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (label, ask) = if e.asking { (tr!("Replace"), true) } else { (tr!("Export"), false) };
                let b = egui::Button::new(text(Type::Body, label).strong()).fill(T.accent_dim).min_size(egui::vec2(96.0, TARGET + 4.0));
                if ui.add_enabled(!e.path.trim().is_empty(), b).clicked() {
                    if ask { replace = true } else { go = true }
                }
            });
        });
    });
    crate::a11y::name_title_bar(&shown, &title);
    if !running {
        let (esc, enter) = ctx.input_mut(|i| (i.consume_key(egui::Modifiers::NONE, egui::Key::Escape), i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)));
        close |= esc;
        if enter && !close {
            if e.asking { replace = true } else { go = true }
        }
    }
    if !open {
        if let Some(r) = &e.running {
            r.control.cancel();
        } else {
            close = true;
        }
    }
    if close {
        return;
    }
    app.plugin_export = Some(e);
    if go || replace {
        start(app, replace);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin_fixture_tests::{EXPORTER, exporter, install, temp};
    use anicraft_engine::Session;
    use anicraft_engine::doc::sample;

    fn app(ctx: &egui::Context) -> App {
        // Screenshot mode, as the other shell tests: no settings folder, no plug-ins folder.
        App::with_context(ctx, Session::new(sample::demo()), Some(std::path::PathBuf::from("unused.png")))
    }

    fn frame(ctx: &egui::Context, app: &mut App, events: Vec<egui::Event>) {
        let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 940.0))), events, ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| {
            menu_items(app, ui);
            show(app, ui.ctx());
        });
        out.textures_delta.clear();
    }

    /// Draws frames until the export finishes (at most ten seconds).
    fn finish(ctx: &egui::Context, app: &mut App) {
        for _ in 0..1000 {
            if app.plugin_export.as_ref().is_none_or(|e| e.running.is_none()) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
            frame(ctx, app, vec![]);
        }
        panic!("the export didn't finish");
    }

    fn key(k: egui::Key) -> egui::Event {
        egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }
    }

    #[test]
    fn the_window_exports_on_a_worker_and_asks_before_replacing() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        let dir = temp("export-window");
        frame(&ctx, &mut app, vec![]);
        install(&mut app, &dir, "dump.wasm", &exporter(false));
        open(&mut app, EXPORTER);
        let out = dir.join("movie.dump");
        let e = app.plugin_export.as_mut().expect("the window");
        assert!(e.path.ends_with("animation.dump"), "{}", e.path);
        assert_eq!((e.from, e.to, e.values["mode"].as_str()), (0, app.s.doc.frame_count() - 1, Some("a")));
        e.path = out.display().to_string();
        frame(&ctx, &mut app, vec![]);
        // Enter exports.
        frame(&ctx, &mut app, vec![key(egui::Key::Enter)]);
        assert!(app.plugin_export.as_ref().is_some_and(|e| e.running.is_some()), "running on a worker");
        finish(&ctx, &mut app);
        assert!(app.plugin_export.is_none(), "the window closes when done");
        assert!(app.status.as_ref().is_some_and(|s| !s.error && s.text.contains("movie.dump")));
        let written: serde_json::Value = serde_json::from_slice(&std::fs::read(&out).expect("written")).expect("JSON");
        assert_eq!(written["result"]["index"], 0);
        // Again: the file is there, so it asks; Replace writes it.
        open(&mut app, EXPORTER);
        app.plugin_export.as_mut().expect("window").path = out.display().to_string();
        start(&mut app, false);
        assert!(app.plugin_export.as_ref().is_some_and(|e| e.asking && e.running.is_none()));
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![key(egui::Key::Enter)]);
        finish(&ctx, &mut app);
        assert!(app.plugin_export.is_none());
        // A bad range or path is an error in the window; Esc closes it.
        open(&mut app, EXPORTER);
        app.plugin_export.as_mut().expect("window").path = String::new();
        start(&mut app, true);
        assert!(app.plugin_export.as_ref().is_some_and(|e| e.error.is_some()));
        frame(&ctx, &mut app, vec![]);
        frame(&ctx, &mut app, vec![key(egui::Key::Escape)]);
        assert!(app.plugin_export.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cancel_stops_an_endless_export_and_writes_nothing() {
        let ctx = egui::Context::default();
        let mut app = app(&ctx);
        let dir = temp("export-cancel");
        install(&mut app, &dir, "dump.wasm", &exporter(true));
        open(&mut app, EXPORTER);
        let out = dir.join("never.dump");
        app.plugin_export.as_mut().expect("window").path = out.display().to_string();
        start(&mut app, false);
        // The progress bar shows while it runs; closing the window cancels it.
        frame(&ctx, &mut app, vec![]);
        if let Some(r) = app.plugin_export.as_ref().and_then(|e| e.running.as_ref()) {
            r.control.cancel();
        }
        frame(&ctx, &mut app, vec![]);
        finish(&ctx, &mut app);
        let e = app.plugin_export.as_ref().expect("the window stays");
        assert!(e.error.is_none() && e.running.is_none());
        assert!(app.status.as_ref().is_some_and(|s| s.text.contains("cancelled")));
        assert!(!out.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
