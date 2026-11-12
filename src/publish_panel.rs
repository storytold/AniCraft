//! File › Publish Settings (Ctrl+Shift+F12) and File › Publish (Shift+F12), ADR 0060.
//!
//! Publish Settings is a window that stays open while you work: every format with a checkbox,
//! and for the one selected its output name (relative to the document's folder, `{name}` the
//! document's name, checked as you type) and the same options its File › Export command has.
//! Each change is `publish.settings_set`: saved in the document, one undo step. Presets (yours,
//! in Preferences) apply a whole profile at once.
//!
//! Publish runs the formats that are on, in order, on a worker thread (`engine::publish::
//! Publisher`, a session of its own), so the editor never waits. Its window shows each format's
//! progress and a Cancel button, then one report: what each wrote, skipped, left out or why it
//! failed. In a browser there are no threads: one format runs per frame, and the files come
//! back as downloads, as one ZIP when there are several.

use crate::a11y::Named;
use crate::app::App;
use crate::theme::{FORM_LABEL_DIALOG, T, TARGET, Type, form_note, form_row, text};
use anicraft_engine::doc::publish::{FrameRange, PublishFormat, PublishProfile};
use anicraft_engine::publish::{self, Control, Outcome, Publisher, Report, Status};
use serde_json::{Value, json};
use std::sync::Arc;

/// The Publish Settings window and the publish running or last run (view state).
pub struct PublishState {
    pub open: bool,
    /// The format whose options show.
    pub selected: PublishFormat,
    /// The output being typed, before it is checked and kept.
    output_edit: Option<(PublishFormat, String)>,
    /// The preset last applied or saved, and the name in the Save field.
    preset: Option<String>,
    preset_name: String,
    /// Delete was clicked once: the second click deletes (a preset is gone for good).
    confirm_delete: bool,
    job: Option<Job>,
    /// The last publish's report, while its window is open.
    pub report: Option<Report>,
}

impl Default for PublishState {
    fn default() -> Self {
        PublishState { open: false, selected: PublishFormat::Html, output_edit: None, preset: None, preset_name: String::new(), confirm_delete: false, job: None, report: None }
    }
}

impl PublishState {
    pub fn running(&self) -> bool {
        self.job.is_some()
    }
}

/// A message from the worker.
enum Msg {
    Started(PublishFormat),
    Done(Box<Outcome>),
    Finished,
}

/// Where the publish runs.
enum Runner {
    /// A worker thread (the desktop).
    Thread(std::sync::mpsc::Receiver<Msg>),
    /// Here, one format per frame (a browser, or when no thread could start).
    Here(Box<Publisher>),
}

/// A publish in progress.
struct Job {
    control: Arc<Control>,
    /// The formats it runs, and the file each writes first (for its row).
    formats: Vec<(PublishFormat, String)>,
    running: Option<PublishFormat>,
    done: Vec<Outcome>,
    runner: Runner,
    /// The document's name, for the browser's ZIP.
    name: String,
}

/// The format's name, in the interface language.
pub fn label(f: PublishFormat) -> &'static str {
    crate::i18n::t(match f {
        PublishFormat::Html => tr_noop!("HTML canvas"),
        PublishFormat::Gif => tr_noop!("Animated GIF"),
        PublishFormat::Apng => tr_noop!("Animated PNG"),
        PublishFormat::Webp => tr_noop!("Animated WebP"),
        PublishFormat::Video => tr_noop!("Video"),
        PublishFormat::Lottie => tr_noop!("Lottie"),
        PublishFormat::SpriteSheet => tr_noop!("Sprite sheet"),
        PublishFormat::PngSequence => tr_noop!("PNG sequence"),
        PublishFormat::Png => tr_noop!("PNG image"),
        PublishFormat::Svg => tr_noop!("SVG image"),
        PublishFormat::Audio => tr_noop!("Audio"),
    })
}

/// What the format makes, for its options' heading.
fn about(f: PublishFormat) -> &'static str {
    crate::i18n::t(match f {
        PublishFormat::Html => tr_noop!("One web page that plays the movie, buttons and actions included."),
        PublishFormat::Gif => tr_noop!("An animated GIF, with exact frame timing."),
        PublishFormat::Apng => tr_noop!("An animated PNG: full colour and real transparency, for the web and chat apps."),
        PublishFormat::Webp => tr_noop!("An animated WebP, lossless: full colour and real transparency, smaller than PNG."),
        PublishFormat::Video => tr_noop!("WebM or MP4 video, made with your ffmpeg (ffmpeg.org)."),
        PublishFormat::Lottie => tr_noop!("A Lottie animation (JSON) for web and apps."),
        PublishFormat::SpriteSheet => tr_noop!("Frames in a grid on one PNG, with a JSON frame map beside it."),
        PublishFormat::PngSequence => tr_noop!("One PNG per frame, in a folder."),
        PublishFormat::Png => tr_noop!("One frame as a PNG image."),
        PublishFormat::Svg => tr_noop!("One frame as an SVG drawing."),
        PublishFormat::Audio => tr_noop!("The movie's soundtrack as a WAV file."),
    })
}

/// File › Publish Settings (Ctrl+Shift+F12): opens the window.
pub fn open(app: &mut App) {
    app.publish.open = true;
}

/// File › Publish (Shift+F12), after any confirmation: starts publishing in the background.
pub fn start(app: &mut App, params: &Value) {
    begin(app, params, true);
}

/// Publishes the formats that are on, here and now, to the report (screenshots and tests).
pub fn publish_now(app: &mut App) {
    begin(app, &json!({"confirm": true}), false);
    if let Some(mut job) = app.publish.job.take() {
        if let Runner::Here(p) = &mut job.runner {
            while let Some(o) = p.step() {
                job.done.push(o.clone());
            }
        }
        finish(app, job);
    }
}

fn begin(app: &mut App, params: &Value, threaded: bool) {
    if app.publish.job.is_some() {
        app.status_err(tr!("Publishing is already running"));
        return;
    }
    let only = match publish::formats_param(params) {
        Ok(o) => o,
        Err(e) => return app.status_err(crate::i18n::t_dyn(&e.to_string()).into_owned()),
    };
    let steps = match publish::plan(&app.s.doc, app.s.path.as_deref(), only.as_deref()) {
        Ok(s) => s,
        Err(e) => {
            if app.s.path.is_none() && !crate::platform::WEB {
                // Untitled: files go beside the document, so it needs a folder first.
                app.status_err(tr!("Save the document first: Publish writes its files beside it."));
                app.invoke("file.save_as");
            } else {
                app.status_err(crate::i18n::t_dyn(&e).into_owned());
                app.publish.open = true;
            }
            return;
        }
    };
    if steps.is_empty() {
        app.status_err(tr!("No format is on in Publish Settings: turn one on first."));
        app.publish.open = true;
        return;
    }
    app.playing = false;
    let formats = steps.iter().map(|s| (s.format, s.outputs.first().and_then(|o| o.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())).collect();
    let control = Arc::new(Control::default());
    let (doc, path) = (app.s.doc.clone(), app.s.path.clone());
    let name = publish::base(path.as_deref()).map(|(_, n)| n).unwrap_or_default();
    let runner = threaded.then(|| spawn(doc.clone(), path.clone(), steps.clone(), control.clone())).flatten().unwrap_or_else(|| Runner::Here(Box::new(Publisher::new(doc, path, steps, control.clone()))));
    app.publish.report = None;
    app.publish.job = Some(Job { control, formats, running: None, done: Vec::new(), runner, name });
}

/// The worker thread, on the desktop; `None` where there are no threads (or none started).
fn spawn(doc: anicraft_engine::doc::Document, path: Option<std::path::PathBuf>, steps: Vec<publish::Step>, control: Arc<Control>) -> Option<Runner> {
    if crate::platform::WEB {
        return None;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("anicraft-publish".into())
        .spawn(move || {
            let mut job = Publisher::new(doc, path, steps, control);
            while let Some(f) = job.current() {
                let _ = tx.send(Msg::Started(f));
                let Some(out) = job.step().cloned() else { break };
                let _ = tx.send(Msg::Done(Box::new(out)));
            }
            let _ = tx.send(Msg::Finished);
        })
        .ok()?;
    Some(Runner::Thread(rx))
}

/// Moves a running publish along (every frame): takes the worker's news, or runs the next
/// format here; and when it is done, reports.
pub fn tick(app: &mut App, ctx: &egui::Context) {
    let Some(job) = app.publish.job.as_mut() else { return };
    let mut finished = false;
    match &mut job.runner {
        Runner::Thread(rx) => {
            while let Ok(m) = rx.try_recv() {
                match m {
                    Msg::Started(f) => job.running = Some(f),
                    Msg::Done(o) => {
                        job.running = None;
                        job.done.push(*o);
                    }
                    Msg::Finished => finished = true,
                }
            }
            if !finished && matches!(rx.try_recv(), Err(std::sync::mpsc::TryRecvError::Disconnected)) {
                finished = true; // the worker is gone: what it reported is the report
            }
        }
        Runner::Here(p) => {
            // A frame shows which format runs before it runs (it holds this thread).
            match (job.running, p.current()) {
                (None, Some(f)) => job.running = Some(f),
                (Some(_), Some(_)) => {
                    if let Some(o) = p.step() {
                        job.done.push(o.clone());
                    }
                    job.running = None;
                }
                (_, None) => finished = true,
            }
        }
    }
    ctx.request_repaint_after(std::time::Duration::from_millis(50));
    if finished && let Some(job) = app.publish.job.take() {
        finish(app, job);
    }
}

fn finish(app: &mut App, job: Job) {
    let report = Report { outcomes: job.done };
    let files = report.files();
    app.s.published.extend(files.iter().cloned());
    if crate::platform::WEB {
        // In a browser the files are in memory: they go to the downloads, in one ZIP when there
        // are several (ADR 0060).
        let written = anicraft_engine::vfs::take_written();
        if written.len() > 1 {
            match publish::zip(&written) {
                Ok(z) => crate::platform::deliver(vec![(std::path::PathBuf::from(format!("{}.zip", job.name)), z)]),
                Err(e) => {
                    app.status_err(tr!("Could not make the archive: {e}", e = e));
                    crate::platform::deliver(written);
                }
            }
        } else {
            crate::platform::deliver(written);
        }
    }
    let (ok, failed, skipped) = (report.count(Status::Published), report.count(Status::Failed), report.count(Status::Skipped));
    if report.count(Status::Cancelled) > 0 {
        app.status_ok(trn!("Publish cancelled after {n} format", "Publish cancelled after {n} formats", ok));
    } else if failed > 0 {
        app.status_err(trn!("Published {n} format; {failed} failed. The report says why.", "Published {n} formats; {failed} failed. The report says why.", ok, failed = failed));
    } else if skipped > 0 {
        app.status_ok(trn!("Published {n} format; {skipped} skipped (see the report)", "Published {n} formats; {skipped} skipped (see the report)", ok, skipped = skipped));
    } else {
        app.status_ok(trn!("Published {n} format ({files} files)", "Published {n} formats ({files} files)", ok, files = files.len()));
    }
    app.publish.report = Some(report);
}

/// Cancels the running publish: the format running stops at its next frame, the rest don't run.
pub fn cancel(app: &mut App) {
    if let Some(j) = &app.publish.job {
        j.control.cancel();
    }
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    tick(app, ctx);
    settings_window(app, ctx);
    progress_window(app, ctx);
}

// ----- Publish Settings -------------------------------------------------------------------------

fn settings_window(app: &mut App, ctx: &egui::Context) {
    if !app.publish.open {
        return;
    }
    let mut open = true;
    let at = ctx.content_rect().center() - egui::vec2(330.0, 260.0);
    let shown = egui::Window::new(tr!("Publish Settings")).id(egui::Id::new("publish_settings")).open(&mut open).default_pos(at).default_width(660.0).resizable(true).collapsible(false).show(ctx, |ui| settings(app, ui));
    crate::a11y::name_title_bar(&shown, tr!("Publish Settings"));
    if !open {
        app.publish.open = false;
        app.publish.output_edit = None;
    }
}

fn settings(app: &mut App, ui: &mut egui::Ui) {
    presets_row(app, ui);
    ui.separator();
    ui.horizontal_top(|ui| {
        // The formats, each with its switch; clicking a name shows its options.
        ui.vertical(|ui| {
            ui.set_width(190.0);
            ui.label(text(Type::Heading, tr!("Formats")));
            for f in PublishFormat::ALL {
                let mut on = app.s.doc.publish.target(f).0;
                ui.horizontal(|ui| {
                    if crate::theme::checkbox(ui, &mut on, "").named(&tr!("Publish {format}", format = label(f))).changed() {
                        app.run("publish.settings_set", json!({"format": f.key(), "enabled": on}));
                        app.publish.selected = f;
                    }
                    let sel = app.publish.selected == f;
                    let (r, resp) = ui.allocate_exact_size(egui::vec2(140.0, TARGET - 4.0), egui::Sense::click());
                    if sel || resp.hovered() {
                        ui.painter().rect_filled(r, T.rc(), if sel { T.accent_dim } else { T.hover });
                    }
                    let font = crate::theme::font(Type::Body);
                    ui.painter().text(egui::pos2(r.left() + 8.0, r.center().y), egui::Align2::LEFT_CENTER, label(f), font, crate::theme::color(Type::Body));
                    crate::a11y::control(ui, &resp, egui::WidgetType::SelectableLabel, label(f), Some(sel));
                    if resp.on_hover_text(about(f)).clicked() {
                        app.publish.selected = f;
                        app.publish.output_edit = None;
                    }
                });
            }
        });
        ui.add_space(12.0);
        ui.vertical(|ui| {
            ui.set_min_height(360.0);
            options(app, ui, app.publish.selected);
        });
    });
    ui.separator();
    footer(app, ui);
}

/// The output name field, checked as you type and kept when it is one.
fn output_row(app: &mut App, ui: &mut egui::Ui, f: PublishFormat) {
    let saved = app.s.doc.publish.target(f).1.to_string();
    let mut value = match &app.publish.output_edit {
        Some((g, v)) if *g == f => v.clone(),
        _ => saved.clone(),
    };
    let base = publish::base(app.s.path.as_deref());
    let name = base.as_ref().map_or_else(|_| tr!("Untitled").to_string(), |(_, n)| n.clone());
    let checked = publish::resolve(f, &value, &name);
    let resp = form_row(ui, FORM_LABEL_DIALOG, tr!("Output"), |ui| {
        let edit = egui::TextEdit::singleline(&mut value).font(crate::theme::font(Type::Path)).desired_width(300.0).char_limit(anicraft_engine::doc::publish::MAX_OUTPUT_CHARS).hint_text(f.default_output());
        let r = ui.add(edit).named(&tr!("{format} output name", format = label(f)));
        if ui.button(tr!("Default")).on_hover_text(tr!("Use the format's usual name: {name}", name = f.default_output())).clicked() {
            value = f.default_output().to_string();
            app.publish.output_edit = None;
            app.run("publish.settings_set", json!({"format": f.key(), "output": value}));
        }
        r
    });
    if resp.changed() {
        app.publish.output_edit = Some((f, value.clone()));
    }
    // Kept when you leave the field (or press Enter) and it is a name Publish can use.
    if resp.lost_focus() {
        if checked.is_ok() && value.trim() != saved {
            app.run("publish.settings_set", json!({"format": f.key(), "output": value.trim()}));
        }
        if checked.is_ok() {
            app.publish.output_edit = None;
        }
    }
    let note = match (&checked, &base) {
        (Err(e), _) => text(Type::Caption, crate::i18n::t_dyn(e)).color(T.error),
        (Ok(rel), Ok((dir, _))) => text(Type::Path, dir.join(rel).display().to_string()).color(T.text_dim),
        (Ok(rel), Err(_)) => text(Type::Caption, tr!("{path}, beside the document once it is saved", path = rel.display().to_string())),
    };
    form_note(ui, FORM_LABEL_DIALOG, note);
}

/// A setting changed: kept in the document (drags coalesce into one undo step).
fn set(app: &mut App, f: PublishFormat, key: &str, v: Value, drag: bool) {
    let p = json!({"format": f.key(), key: v});
    if drag {
        crate::panels::live(app, tr_noop!("Publish Settings"), "publish.settings_set", p);
    } else {
        app.run("publish.settings_set", p);
    }
}

fn check(app: &mut App, ui: &mut egui::Ui, f: PublishFormat, key: &str, mut on: bool, label: &str, tip: &str) {
    ui.horizontal(|ui| {
        ui.add_space(FORM_LABEL_DIALOG + ui.spacing().item_spacing.x);
        if crate::theme::checkbox(ui, &mut on, label).on_hover_text(tip).changed() {
            set(app, f, key, json!(on), false);
        }
    });
}

fn scale_row(app: &mut App, ui: &mut egui::Ui, f: PublishFormat, scale: f32) {
    let mut pct = (scale * 100.0).round();
    form_row(ui, FORM_LABEL_DIALOG, tr!("Scale"), |ui| {
        let r = ui.add(egui::DragValue::new(&mut pct).range(1.0..=800.0).suffix(" %").speed(1.0)).named(tr!("Scale"));
        if r.changed() {
            set(app, f, "scale", json!(pct / 100.0), true);
        }
        let s = &app.s.doc.stage;
        ui.label(text(Type::Data, format!("{} × {}", (s.width as f32 * pct / 100.0).ceil(), (s.height as f32 * pct / 100.0).ceil())));
    });
}

/// How many times it plays: forever, or a count.
fn plays_row(app: &mut App, ui: &mut egui::Ui, f: PublishFormat, plays: u32) {
    let mut forever = plays == 0;
    form_row(ui, FORM_LABEL_DIALOG, tr!("Playback"), |ui| {
        if crate::theme::checkbox(ui, &mut forever, tr!("Loop forever")).on_hover_text(tr!("Off: it plays the number of times you set, then stops on the last frame")).changed() {
            set(app, f, "loop", json!(if forever { 0 } else { 1 }), false);
        }
        if !forever {
            let mut n = plays.max(1);
            ui.label(text(Type::Label, tr!("Plays")));
            if ui.add(egui::DragValue::new(&mut n).range(1..=anicraft_engine::doc::publish::MAX_PLAYS).speed(0.1)).named(tr!("Plays")).changed() {
                set(app, f, "loop", json!(n), true);
            }
        }
    });
}

/// Which movie frames: all of them, or from one to another (shown 1-based, as the timeline).
fn range_row(app: &mut App, ui: &mut egui::Ui, f: PublishFormat, r: FrameRange) {
    let total = app.s.doc.frame_count();
    let mut all = r.from.is_none() && r.to.is_none();
    form_row(ui, FORM_LABEL_DIALOG, tr!("Frames"), |ui| {
        if crate::theme::checkbox(ui, &mut all, tr!("All")).on_hover_text(tr!("Every frame of every scene, in order")).changed() {
            if all {
                app.run("publish.settings_set", json!({"format": f.key(), "from": null, "to": null}));
            } else {
                app.run("publish.settings_set", json!({"format": f.key(), "from": 0, "to": total.saturating_sub(1)}));
            }
        }
        if all {
            ui.label(text(Type::Data, format!("1–{total}")));
            return;
        }
        let mut from = r.from.unwrap_or(0) + 1;
        let mut to = r.to.unwrap_or(total.saturating_sub(1)) + 1;
        if ui.add(egui::DragValue::new(&mut from).range(1..=to.max(1)).speed(0.2)).named(tr!("First frame")).changed() {
            set(app, f, "from", json!(from - 1), true);
        }
        ui.label(text(Type::Label, tr!("to")));
        if ui.add(egui::DragValue::new(&mut to).range(from..=total.max(from)).speed(0.2)).named(tr!("Last frame")).changed() {
            set(app, f, "to", json!(to - 1), true);
        }
    });
}

fn frame_row(app: &mut App, ui: &mut egui::Ui, f: PublishFormat, frame: u32) {
    let total = app.s.doc.frame_count();
    let mut shown = frame.min(total.saturating_sub(1)) + 1;
    form_row(ui, FORM_LABEL_DIALOG, tr!("Frame"), |ui| {
        if ui.add(egui::DragValue::new(&mut shown).range(1..=total.max(1)).speed(0.2)).named(tr!("Frame to publish")).changed() {
            set(app, f, "frame", json!(shown - 1), true);
        }
        if ui.button(tr!("Playhead")).on_hover_text(tr!("The frame the playhead is on")).clicked() {
            let movie = app.s.doc.scene_start(app.s.scene).saturating_add(app.s.frame);
            set(app, f, "frame", json!(movie), false);
        }
    });
}

fn options(app: &mut App, ui: &mut egui::Ui, f: PublishFormat) {
    ui.label(text(Type::Heading, label(f)));
    ui.label(text(Type::Caption, about(f)));
    ui.add_space(4.0);
    output_row(app, ui, f);
    let p: PublishProfile = (*app.s.doc.publish).clone();
    match f {
        PublishFormat::Html => {
            let mut title = p.html.title.clone();
            form_row(ui, FORM_LABEL_DIALOG, tr!("Page title"), |ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut title).desired_width(300.0).char_limit(anicraft_engine::doc::publish::MAX_TITLE_CHARS).hint_text(tr!("The file's name"))).named(tr!("Page title"));
                if r.changed() {
                    set(app, f, "title", json!(title), true);
                }
            });
            check(app, ui, f, "sound", p.html.sound, tr!("Include sounds"), tr!("The movie's sounds play in the page"));
            check(app, ui, f, "fit", p.html.fit, tr!("Scale to the browser window"), tr!("Off: the movie shows at its stage size"));
        }
        PublishFormat::Gif => {
            range_row(app, ui, f, p.gif.range);
            scale_row(app, ui, f, p.gif.scale);
            check(app, ui, f, "loop", p.gif.looping, tr!("Loop"), tr!("Off: it plays once and stops on the last frame"));
        }
        PublishFormat::Apng | PublishFormat::Webp => {
            let (range, scale, plays, background) = if f == PublishFormat::Apng { (p.apng.range, p.apng.scale, p.apng.plays, p.apng.background) } else { (p.webp.range, p.webp.scale, p.webp.plays, p.webp.background) };
            range_row(app, ui, f, range);
            scale_row(app, ui, f, scale);
            plays_row(app, ui, f, plays);
            check(app, ui, f, "background", background, tr!("Stage colour as background"), tr!("Off: transparent"));
        }
        PublishFormat::Video => {
            range_row(app, ui, f, p.video.range);
            scale_row(app, ui, f, p.video.scale);
            check(app, ui, f, "audio", p.video.audio, tr!("Include sound"), tr!("The movie's sounds: Opus in WebM, AAC in MP4"));
            form_note(ui, FORM_LABEL_DIALOG, text(Type::Caption, tr!("End the name in .webm (VP9) or .mp4 (H.264). AniCraft uses the ffmpeg on this computer.")));
        }
        PublishFormat::Lottie => {
            range_row(app, ui, f, p.lottie.range);
            check(app, ui, f, "background", p.lottie.background, tr!("Stage colour as background"), tr!("Off: transparent"));
        }
        PublishFormat::SpriteSheet => {
            let sh = &p.sprite_sheet;
            range_row(app, ui, f, sh.range);
            scale_row(app, ui, f, sh.scale);
            let (mut cols, mut pad) = (sh.columns, sh.padding);
            form_row(ui, FORM_LABEL_DIALOG, tr!("Columns"), |ui| {
                let r = ui.add(egui::DragValue::new(&mut cols).range(0..=1000).speed(0.1).custom_formatter(|v, _| if v == 0.0 { tr!("Auto").to_string() } else { format!("{v}") })).named(tr!("Columns"));
                if r.changed() {
                    set(app, f, "columns", json!(cols), true);
                }
                ui.label(text(Type::Label, tr!("Padding")));
                if ui.add(egui::DragValue::new(&mut pad).range(0..=256).suffix(" px").speed(0.2)).named(tr!("Padding")).changed() {
                    set(app, f, "padding", json!(pad), true);
                }
            });
            check(app, ui, f, "background", sh.background, tr!("Stage colour as background"), tr!("Off: transparent"));
        }
        PublishFormat::PngSequence => {
            range_row(app, ui, f, p.png_sequence.range);
            scale_row(app, ui, f, p.png_sequence.scale);
            form_note(ui, FORM_LABEL_DIALOG, text(Type::Caption, tr!("The output is a folder; the frames in it are frame_00001.png and on.")));
        }
        PublishFormat::Png => {
            frame_row(app, ui, f, p.png.frame);
            scale_row(app, ui, f, p.png.scale);
        }
        PublishFormat::Svg => frame_row(app, ui, f, p.svg.frame),
        PublishFormat::Audio => {
            range_row(app, ui, f, p.audio.range);
            let rates = [22_050u32, 44_100, 48_000, 96_000];
            form_row(ui, FORM_LABEL_DIALOG, tr!("Sample rate"), |ui| {
                egui::ComboBox::from_id_salt("publish_rate").selected_text(text(Type::Data, format!("{} Hz", p.audio.rate))).show_ui(ui, |ui| {
                    for r in rates {
                        if ui.selectable_label(p.audio.rate == r, text(Type::Data, format!("{r} Hz"))).clicked() {
                            set(app, f, "rate", json!(r), false);
                        }
                    }
                })
                .response
                .set_name(tr!("Sample rate"));
            });
            form_row(ui, FORM_LABEL_DIALOG, tr!("Bit depth"), |ui| {
                let name = |b: u32| if b == 32 { tr!("32-bit float").to_string() } else { tr!("{bits}-bit", bits = b) };
                egui::ComboBox::from_id_salt("publish_bits").selected_text(name(p.audio.bits)).show_ui(ui, |ui| {
                    for b in [16u32, 24, 32] {
                        if ui.selectable_label(p.audio.bits == b, name(b)).clicked() {
                            set(app, f, "bits", json!(b), false);
                        }
                    }
                })
                .response
                .set_name(tr!("Bit depth"));
            });
            if !anicraft_engine::doc::sound::has_sounds(&app.s.doc) {
                form_note(ui, FORM_LABEL_DIALOG, text(Type::Caption, tr!("The movie has no sounds yet, so Publish skips the audio.")));
            }
        }
    }
}

/// Where the files go, and Publish.
fn footer(app: &mut App, ui: &mut egui::Ui) {
    let on = app.s.doc.publish.enabled().len();
    ui.horizontal(|ui| {
        let where_ = match publish::base(app.s.path.as_deref()) {
            Ok((dir, _)) if crate::platform::WEB => {
                let _ = dir;
                tr!("Files go to your downloads (several as one ZIP).").to_string()
            }
            Ok((dir, _)) => tr!("Files go beside the document, in {folder}", folder = dir.display().to_string()),
            Err(_) => tr!("Save the document first: files go beside it.").to_string(),
        };
        ui.label(text(Type::Caption, where_));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let key = app.keymap.primary("file.publish").map(crate::platform::shortcut_label).unwrap_or_default();
            let busy = app.publish.running();
            let button = egui::Button::new(text(Type::Body, tr!("Publish")).strong().color(T.on_fill)).fill(T.accent).min_size(egui::vec2(110.0, TARGET + 4.0));
            let tip = crate::a11y::with_key(&trn!("Publish the {n} format that is on", "Publish the {n} formats that are on", on), Some(&key));
            if ui.add_enabled(on > 0 && !busy, button).on_hover_text(tip).on_disabled_hover_text(if busy { tr!("Publishing is running") } else { tr!("Turn a format on first") }).clicked() {
                app.request("file.publish", json!({}));
            }
            ui.label(text(Type::Caption, trn!("{n} format on", "{n} formats on", on)));
        });
    });
}

/// Presets (yours, kept in Preferences): apply one, save the settings as one, delete one.
fn presets_row(app: &mut App, ui: &mut egui::Ui) {
    form_row(ui, FORM_LABEL_DIALOG, tr!("Preset"), |ui| {
        let names: Vec<String> = app.prefs.publish_presets.keys().cloned().collect();
        let shown = app.publish.preset.clone().filter(|p| names.contains(p)).unwrap_or_else(|| tr!("Choose a preset").to_string());
        let mut chosen = None;
        egui::ComboBox::from_id_salt("publish_preset")
            .width(170.0)
            .selected_text(shown)
            .show_ui(ui, |ui| {
                if ui.selectable_label(false, tr!("Default")).on_hover_text(tr!("The HTML page on, every other format off")).clicked() {
                    chosen = Some((tr!("Default").to_string(), PublishProfile::default()));
                }
                for n in &names {
                    if ui.selectable_label(app.publish.preset.as_deref() == Some(n.as_str()), n.as_str()).clicked() {
                        chosen = app.prefs.publish_presets.get(n).map(|p| (n.clone(), p.clone()));
                    }
                }
            })
            .response
            .set_name(tr!("Apply a preset"));
        if let Some((n, p)) = chosen {
            if app.run("publish.settings_set", json!({"profile": p})).is_some() {
                app.status_ok(tr!("Applied the preset “{name}” — {undo} to undo", name = n, undo = app.undo_key()));
            }
            app.publish.preset = Some(n);
            app.publish.output_edit = None;
            app.publish.confirm_delete = false;
        }
        let name_ok = !app.publish.preset_name.trim().is_empty();
        ui.add(egui::TextEdit::singleline(&mut app.publish.preset_name).desired_width(130.0).char_limit(crate::prefs::MAX_PRESET_NAME).hint_text(tr!("New preset name"))).set_name(tr!("New preset name"));
        if ui.add_enabled(name_ok, egui::Button::new(tr!("Save Preset"))).on_hover_text(tr!("Keep these settings as a preset you can apply to any document")).clicked() {
            save_preset(app);
        }
        let current = app.publish.preset.clone().filter(|p| app.prefs.publish_presets.contains_key(p));
        if let Some(n) = current {
            if app.publish.confirm_delete {
                if ui.add(egui::Button::new(text(Type::Body, tr!("Delete “{name}”", name = n)).color(T.on_fill)).fill(T.danger)).on_hover_text(tr!("Deleting a preset can't be undone")).clicked() {
                    delete_preset(app, &n);
                }
                if ui.button(tr!("Keep")).clicked() {
                    app.publish.confirm_delete = false;
                }
            } else if ui.button(tr!("Delete Preset")).on_hover_text(tr!("Delete the preset “{name}” (asks once more)", name = n)).clicked() {
                app.publish.confirm_delete = true;
            }
        }
    });
}

fn save_presets(app: &mut App) {
    if app.shot.is_none()
        && let Err(e) = app.prefs.save()
    {
        app.status_err(tr!("Could not save preferences: {e}", e = e));
    }
}

/// Keeps the document's settings as the preset named in the field (replacing one of that name).
pub fn save_preset(app: &mut App) {
    let name: String = app.publish.preset_name.trim().chars().take(crate::prefs::MAX_PRESET_NAME).collect();
    if name.is_empty() {
        return;
    }
    let presets = &mut app.prefs.publish_presets;
    if !presets.contains_key(&name) && presets.len() >= crate::prefs::MAX_PUBLISH_PRESETS {
        app.status_err(tr!("You have {n} presets, the most there can be: delete one first", n = crate::prefs::MAX_PUBLISH_PRESETS));
        return;
    }
    let replaced = presets.insert(name.clone(), (*app.s.doc.publish).clone()).is_some();
    save_presets(app);
    app.status_ok(if replaced { tr!("Updated the preset “{name}”", name = name) } else { tr!("Saved the preset “{name}”", name = name) });
    app.publish.preset = Some(name);
    app.publish.preset_name.clear();
    app.publish.confirm_delete = false;
}

fn delete_preset(app: &mut App, name: &str) {
    app.prefs.publish_presets.remove(name);
    save_presets(app);
    app.publish.preset = None;
    app.publish.confirm_delete = false;
    app.status_ok(tr!("Deleted the preset “{name}”", name = name));
}

// ----- progress and report ----------------------------------------------------------------------

fn status_text(s: Status) -> (&'static str, egui::Color32) {
    match s {
        Status::Published => (tr!("Published"), T.ok),
        Status::Skipped => (tr!("Skipped"), T.text_dim),
        Status::Failed => (tr!("Failed"), T.error),
        Status::Cancelled => (tr!("Cancelled"), T.text_dim),
    }
}

fn progress_window(app: &mut App, ctx: &egui::Context) {
    if app.publish.job.is_none() && app.publish.report.is_none() {
        return;
    }
    let running = app.publish.job.is_some();
    let mut open = true;
    let title = if running { tr!("Publishing") } else { tr!("Publish report") };
    let at = ctx.content_rect().center() - egui::vec2(220.0, 160.0);
    let shown = egui::Window::new(title).id(egui::Id::new("publish_progress")).open(&mut open).default_pos(at).default_width(460.0).resizable(true).collapsible(false).show(ctx, |ui| {
        if let Some(job) = &app.publish.job {
            job_rows(ui, job);
        } else if let Some(r) = &app.publish.report {
            report_rows(ui, r);
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if running {
                let cancelling = app.publish.job.as_ref().is_some_and(|j| j.control.cancelled());
                if ui.add_enabled(!cancelling, egui::Button::new(tr!("Cancel")).min_size(egui::vec2(96.0, TARGET))).on_hover_text(tr!("Stop after the frame being made; formats not started don't run")).clicked() {
                    cancel(app);
                }
                if cancelling {
                    ui.label(text(Type::Caption, tr!("Stopping…")));
                }
            } else {
                if ui.add(egui::Button::new(tr!("Close")).min_size(egui::vec2(96.0, TARGET))).clicked() {
                    app.publish.report = None;
                }
                if ui.button(tr!("Publish Settings…")).clicked() {
                    open_settings_from_report(app);
                }
            }
        });
    });
    crate::a11y::name_title_bar(&shown, title);
    if !open {
        if running {
            cancel(app);
        } else {
            app.publish.report = None;
        }
    }
}

fn open_settings_from_report(app: &mut App) {
    app.publish.open = true;
    if let Some(f) = app.publish.report.as_ref().and_then(|r| r.outcomes.iter().find(|o| o.status == Status::Failed)).map(|o| o.format) {
        app.publish.selected = f;
    }
}

/// A format's name at the start of its row, in a column of its own.
fn name_cell(ui: &mut egui::Ui, name: &str) {
    ui.allocate_ui_with_layout(egui::vec2(120.0, TARGET), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_min_size(egui::vec2(120.0, TARGET));
        ui.label(text(Type::Body, name));
    });
}

/// A message as a sentence: capital first, full stop at the end.
fn sentence(m: &str) -> String {
    let mut c = m.trim().chars();
    let mut s: String = c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default();
    if !s.is_empty() && !s.ends_with(['.', ')', '!', '?']) {
        s.push('.');
    }
    s
}

/// One row per format: done, running (with its frames), or waiting.
fn job_rows(ui: &mut egui::Ui, job: &Job) {
    let (done, total) = job.control.progress();
    for (f, file) in &job.formats {
        ui.horizontal(|ui| {
            ui.set_min_height(TARGET);
            name_cell(ui, label(*f));
            if let Some(o) = job.done.iter().find(|o| o.format == *f) {
                let (s, c) = status_text(o.status);
                ui.label(text(Type::Label, s).color(c));
            } else if job.running == Some(*f) {
                if total > 0 {
                    let frac = (done as f32 / total as f32).clamp(0.0, 1.0);
                    ui.add(egui::ProgressBar::new(frac).desired_width(160.0).text(text(Type::DataSmall, format!("{done} / {total}"))));
                } else {
                    ui.add(egui::Spinner::new());
                    ui.label(text(Type::Label, tr!("Working…")));
                }
            } else {
                ui.label(text(Type::Label, tr!("Waiting")));
            }
            ui.label(text(Type::Path, file.as_str()).color(T.text_dim));
        });
    }
}

/// The report: each format's outcome, and what its exporter left out, skipped or why it failed.
fn report_rows(ui: &mut egui::Ui, r: &Report) {
    egui::ScrollArea::vertical().max_height(320.0).auto_shrink([false, true]).show(ui, |ui| {
        for o in &r.outcomes {
            ui.horizontal(|ui| {
                ui.set_min_height(TARGET);
                name_cell(ui, label(o.format));
                let (s, c) = status_text(o.status);
                ui.label(text(Type::Label, s).color(c));
                let files: Vec<String> = o.files.iter().filter_map(|f| f.file_name()).map(|n| n.to_string_lossy().into_owned()).collect();
                if !files.is_empty() {
                    ui.label(text(Type::Path, files.join(", ")).color(T.text_dim));
                }
            });
            if !o.message.is_empty() {
                // The exporter's English, translated when a catalog has it.
                ui.horizontal(|ui| {
                    ui.add_space(128.0);
                    ui.add(egui::Label::new(text(Type::Caption, sentence(&crate::i18n::t_dyn(&o.message)))).wrap());
                });
            }
        }
    });
    if let Some(dir) = r.files().first().and_then(|f| f.parent()).filter(|d| !d.as_os_str().is_empty()) {
        ui.label(text(Type::Caption, tr!("In {folder}", folder = dir.display().to_string())));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anicraft_engine::Session;
    use anicraft_engine::doc::sample;

    fn app(name: &str) -> (App, egui::Context, std::path::PathBuf) {
        let ctx = egui::Context::default();
        let mut app = App::with_context(&ctx, Session::new(sample::demo()), Some(std::path::PathBuf::from("unused.png")));
        let dir = std::env::temp_dir().join(format!("anicraft-publish-ui-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        app.run("file.save", json!({"path": dir.join("walk.anicraft").display().to_string()}));
        (app, ctx, dir)
    }

    /// Draws the windows and moves the publish along until it is done (or `frames` run out).
    fn frames(app: &mut App, ctx: &egui::Context, n: usize) {
        for _ in 0..n {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(app, &ui.ctx().clone()));
            out.textures_delta.clear();
            if !app.publish.running() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn publish_runs_on_a_worker_and_reports_once() {
        let (mut app, ctx, dir) = app("worker");
        app.run("publish.settings_set", json!({"format": "gif", "enabled": true, "scale": 0.1, "to": 5}));
        app.run("publish.settings_set", json!({"format": "audio", "enabled": true}));
        app.publish.open = true;
        app.request("file.publish", json!({}));
        assert!(app.publish.running(), "it starts in the background");
        app.request("file.publish", json!({}));
        frames(&mut app, &ctx, 2000);
        assert!(!app.publish.running());
        let r = app.publish.report.clone().expect("a report");
        assert_eq!((r.count(Status::Published), r.count(Status::Skipped)), (2, 1));
        assert!(dir.join("walk.gif").is_file() && dir.join("walk.html").is_file());
        assert!(app.s.published.contains(&dir.join("walk.gif")), "the session knows what it published");
        // Published again: no question, this session wrote them.
        assert!(app.s.confirmation("file.publish", &json!({})).is_none());
        frames(&mut app, &ctx, 1);
        open_settings_from_report(&mut app);
        assert!(app.publish.open);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cancel_stops_the_worker() {
        let (mut app, ctx, dir) = app("cancel");
        app.run("publish.settings_set", json!({"format": "png_sequence", "enabled": true, "scale": 0.2}));
        app.run("publish.settings_set", json!({"format": "svg", "enabled": true}));
        start(&mut app, &json!({}));
        cancel(&mut app);
        frames(&mut app, &ctx, 2000);
        let r = app.publish.report.clone().expect("a report");
        assert!(r.count(Status::Cancelled) >= 1, "{:?}", r.to_json());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn publishing_here_steps_one_format_per_frame() {
        let (mut app, ctx, dir) = app("here");
        app.run("publish.settings_set", json!({"format": "svg", "enabled": true}));
        begin(&mut app, &json!({}), false);
        let mut seen_running = false;
        for _ in 0..20 {
            seen_running |= app.publish.job.as_ref().is_some_and(|j| j.running.is_some());
            frames(&mut app, &ctx, 1);
        }
        assert!(seen_running && !app.publish.running());
        assert_eq!(app.publish.report.as_ref().map(|r| r.count(Status::Published)), Some(2));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn what_publish_cant_run_says_why() {
        let ctx = egui::Context::default();
        let mut app = App::with_context(&ctx, Session::new(sample::demo()), Some(std::path::PathBuf::from("unused.png")));
        start(&mut app, &json!({}));
        assert!(!app.publish.running(), "untitled: saves first");
        start(&mut app, &json!({"formats": 5}));
        assert!(!app.publish.running());
        let (mut app, _, dir) = self::app("none");
        app.run("publish.settings_set", json!({"format": "html", "enabled": false}));
        start(&mut app, &json!({}));
        assert!(!app.publish.running() && app.publish.open, "nothing on: the settings open");
        app.run("publish.settings_set", json!({"format": "sprite_sheet", "enabled": true}));
        app.run("publish.settings_set", json!({"format": "png", "enabled": true, "output": "{name}_sheet.png"}));
        start(&mut app, &json!({}));
        assert!(!app.publish.running(), "two formats on one file");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn presets_save_apply_and_delete() {
        let (mut app, ctx, dir) = app("presets");
        app.run("publish.settings_set", json!({"format": "gif", "enabled": true, "scale": 0.5}));
        app.publish.preset_name = "  Small GIF ".into();
        save_preset(&mut app);
        assert!(app.prefs.publish_presets.get("Small GIF").is_some_and(|p| p.gif.enabled && p.gif.scale == 0.5));
        app.publish.preset_name = "Small GIF".into();
        save_preset(&mut app);
        assert_eq!(app.prefs.publish_presets.len(), 1, "the same name replaces it");
        app.publish.preset_name = "   ".into();
        save_preset(&mut app);
        assert_eq!(app.prefs.publish_presets.len(), 1);
        for i in 0..crate::prefs::MAX_PUBLISH_PRESETS {
            app.prefs.publish_presets.insert(format!("p{i}"), PublishProfile::default());
        }
        app.publish.preset_name = "one too many".into();
        save_preset(&mut app);
        assert!(!app.prefs.publish_presets.contains_key("one too many"));
        delete_preset(&mut app, "Small GIF");
        assert!(!app.prefs.publish_presets.contains_key("Small GIF") && app.publish.preset.is_none());
        // Every format's options draw.
        app.publish.open = true;
        for f in PublishFormat::ALL {
            app.publish.selected = f;
            app.publish.output_edit = Some((f, "../nope".into()));
            frames(&mut app, &ctx, 1);
        }
        app.publish.preset = Some("p1".into());
        app.publish.confirm_delete = true;
        frames(&mut app, &ctx, 1);
        assert_eq!(sentence("the movie has no sounds"), "The movie has no sounds.");
        assert_eq!(sentence(""), "");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
