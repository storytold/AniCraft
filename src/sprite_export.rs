//! Sprite sheet export options and an isolated background command session.

use crate::app::App;
use crate::theme::{self, T, Type, text, with_type};
use anicraft_engine::{Session, doc::Document, sprite_sheet::Layout};
use serde_json::{Value, json};
use std::sync::mpsc::{Receiver, TryRecvError, channel};

pub struct Options {
    path: String,
    from: u32,
    to: u32,
    scale: f32,
    columns: u32,
    transparent: bool,
}

impl Options {
    pub fn new(s: &Session) -> Self {
        let mut options = Self { path: "sprites.png".into(), from: 1, to: s.frame_count(), scale: 1.0, columns: 0, transparent: true };
        // Start with a useful size for the whole animation; the artist can change it.
        while Layout::new(&s.doc, &options.params()).is_err() && options.scale > 0.01 {
            options.scale *= 0.5;
        }
        options
    }

    fn params(&self) -> Value {
        let mut p = json!({"path": self.path, "from": self.from.saturating_sub(1), "to": self.to.saturating_sub(1), "scale": self.scale, "transparent": self.transparent});
        if self.columns > 0 {
            p["columns"] = json!(self.columns);
        }
        p
    }
}

pub struct Job(Receiver<Result<Value, String>>);

impl Job {
    pub fn start(doc: Document, params: Value) -> Result<Self, String> {
        let (tx, rx) = channel();
        std::thread::Builder::new().name("sprite-export".into()).spawn(move || {
            let result = Session::new(doc).execute("file.export_sprite_sheet", &params).map_err(|e| e.to_string());
            let _ = tx.send(result);
        }).map_err(|e| format!("could not start export: {e}"))?;
        Ok(Self(rx))
    }
}

pub fn poll(app: &mut App, ctx: &egui::Context) {
    let Some(job) = &app.sprite_job else { return };
    match job.0.try_recv() {
        Ok(result) => {
            app.sprite_job = None;
            match result {
                Ok(v) => app.status_ok(format!("Exported {} and {}", v.get("written").and_then(Value::as_str).unwrap_or("PNG"), v.get("json").and_then(Value::as_str).unwrap_or("JSON"))),
                Err(e) => app.status_err(e),
            }
        }
        Err(TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(100)),
        Err(TryRecvError::Disconnected) => {
            app.sprite_job = None;
            app.status_err("The sprite sheet export worker stopped before finishing.");
        }
    }
}

pub fn show(app: &mut App, ctx: &egui::Context) {
    let Some(o) = &mut app.sprite_options else { return };
    let mut export = false;
    let mut cancel = false;
    egui::Modal::new(egui::Id::new("sprite_export")).show(ctx, |ui| {
        ui.set_width(480.0);
        ui.label(text(Type::Title, "Export sprite sheet + JSON"));
        ui.label(text(Type::Body, "Frames fill a fixed grid from left to right, then top to bottom."));
        theme::section(ui, "sprite_output", theme::Section::Rule, "Output", None, |ui| {
            theme::form_row(ui, theme::FORM_LABEL_DIALOG, "PNG file", |ui| {
                with_type(ui, Type::Path, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut o.path).desired_width(f32::INFINITY));
                });
            });
            theme::form_row(ui, theme::FORM_LABEL_DIALOG, "", |ui| {
                if ui.button("Browse…").clicked()
                    && let Some(path) = crate::platform::file_dialog("file.export_sprite_sheet", "Export sprite sheet", &o.path)
                {
                    o.path = path.display().to_string();
                }
            });
            theme::form_note(ui, theme::FORM_LABEL_DIALOG, "A JSON file with the same name is saved beside the PNG.");
        });
        theme::section(ui, "sprite_frames", theme::Section::Rule, "Frames", None, |ui| {
            theme::form_row(ui, theme::FORM_LABEL_DIALOG, "Range", |ui| {
                with_type(ui, Type::Data, |ui| {
                    ui.add(egui::DragValue::new(&mut o.from).range(1..=app.s.frame_count()));
                });
                ui.label(text(Type::Label, "to"));
                with_type(ui, Type::Data, |ui| {
                    ui.add(egui::DragValue::new(&mut o.to).range(1..=app.s.frame_count()));
                });
            });
            theme::form_row(ui, theme::FORM_LABEL_DIALOG, "Scale", |ui| {
                with_type(ui, Type::Data, |ui| {
                    ui.add(egui::DragValue::new(&mut o.scale).range(0.001..=8.0).speed(0.01).max_decimals(4).suffix(" ×"));
                });
            });
            theme::form_row(ui, theme::FORM_LABEL_DIALOG, "Columns", |ui| {
                with_type(ui, Type::Data, |ui| {
                    ui.add(egui::DragValue::new(&mut o.columns).range(0..=anicraft_engine::sprite_sheet::MAX_FRAMES));
                });
            });
            theme::form_note(ui, theme::FORM_LABEL_DIALOG, "0 chooses columns automatically.");
            theme::form_row(ui, theme::FORM_LABEL_DIALOG, "Background", |ui| {
                theme::checkbox(ui, &mut o.transparent, text(Type::Body, "Transparent"));
            });
        });
        let params = o.params();
        let layout = Layout::new(&app.s.doc, &params).and_then(|l| anicraft_engine::sprite_sheet::output_paths(&params).map(|_| l));
        match &layout {
            Ok(l) => {
                ui.label(text(Type::Data, format!("{} × {} px · {} frames · {} × {} grid", l.width, l.height, l.to - l.from + 1, l.columns, l.rows)));
                ui.label(text(Type::DataSmall, format!("Each frame: {} × {} px · {:.2} fps", l.frame_width, l.frame_height, app.s.doc.stage.fps)));
            }
            Err(e) => { ui.label(text(Type::Body, e.to_string()).color(T.error)); }
        }
        ui.horizontal(|ui| {
            cancel = ui.button("Cancel").clicked();
            export = ui.add_enabled(layout.is_ok(), egui::Button::new(text(Type::Body, "Export")).fill(T.accent)).clicked();
        });
    });
    if ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        cancel = true;
    }
    if cancel {
        app.sprite_options = None;
    } else if export && let Some(o) = app.sprite_options.take() {
        app.request("file.export_sprite_sheet", o.params());
    }
}
