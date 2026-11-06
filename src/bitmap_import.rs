//! Background bitmap decoding; the regular import command applies the result as one undo step.

use crate::app::App;
use anicraft_engine::cmds_bitmap::{self, Prepared};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError, channel};

pub struct Job {
    rx: Receiver<Result<Prepared, String>>,
    params: Value,
    generation: u64,
}

pub fn start(app: &mut App, params: Value) {
    if app.bitmap_job.is_some() { app.status_err("A bitmap import is already running."); return }
    let Some(path) = params.get("path").and_then(Value::as_str).map(PathBuf::from) else { app.status_err("Choose an image file to import."); return };
    let (tx, rx) = channel();
    match std::thread::Builder::new().name("bitmap-import".into()).spawn(move || {
        let result = cmds_bitmap::read(&path).map(|image| Prepared { path, image }).map_err(|e| e.to_string());
        let _ = tx.send(result);
    }) {
        Ok(_) => {
            app.bitmap_job = Some(Job { rx, params, generation: app.document_generation });
            app.status_ok("Importing bitmap…");
        }
        Err(e) => app.status_err(format!("Could not start bitmap import: {e}")),
    }
}

pub fn poll(app: &mut App, ctx: &egui::Context) {
    let Some(job) = &app.bitmap_job else { return };
    let result = match job.rx.try_recv() {
        Ok(result) => result,
        Err(TryRecvError::Empty) => { ctx.request_repaint_after(std::time::Duration::from_millis(100)); return }
        Err(TryRecvError::Disconnected) => Err("Bitmap import worker stopped before finishing.".into()),
    };
    let Some(job) = app.bitmap_job.take() else { return };
    if job.generation != app.document_generation {
        app.status_ok("Bitmap import cancelled because the document was replaced.");
        return;
    }
    match result {
        Ok(prepared) => {
            app.s.prepared_bitmap = Some(prepared);
            app.perform("file.import_bitmap", job.params);
            app.s.prepared_bitmap = None;
        }
        Err(e) => app.status_err(e),
    }
}

/// Original procedural artwork for the screenshot check; no external asset is bundled.
pub fn preview(app: &mut App) {
    let mut rgba = Vec::new();
    for y in 0..64 {
        for x in 0..64 {
            let p = if (x / 8 + y / 8) % 2 == 0 { [36, 130, 232, 255] } else { [255, 188, 68, 200] };
            rgba.extend_from_slice(&p);
        }
    }
    match anicraft_engine::doc::Bitmap::new(64, 64, rgba) {
        Ok(image) => {
            let path = PathBuf::from("Colour study.png");
            app.s.prepared_bitmap = Some(Prepared { path: path.clone(), image: std::sync::Arc::new(image) });
            app.perform("file.import_bitmap", serde_json::json!({"path": path, "x": 640, "y": 340}));
            app.run("element.set_transform", serde_json::json!({"sx": 5, "sy": 5, "rotation": -12}));
        }
        Err(e) => app.status_err(e),
    }
}
