//! The Eraser (E) and the Eyedropper (I), ADR 0034: view state over `draw.erase`, `draw.faucet`
//! and `paint.pick`. An eraser drag collects samples like the Brush, shows the stage colour
//! where it goes and sends one command when the pointer lifts, so a stroke is one undo step.

use crate::app::{App, Drag, Tool};
use crate::brushtool::{BrushOptions, draw_sweep, draw_tip, points, shape_key};
use crate::pathtools::Xf;
use crate::theme::to_color32;
use anicraft_engine::cmds_erase::EraseMode;
use anicraft_engine::doc::ElementKind;
use anicraft_engine::geom::Vec2;
use anicraft_engine::geom::brush::TipShape;
use egui::Pos2;
use serde_json::json;

/// The Eraser's options (view state: the tool's, not the document's).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EraserOptions {
    /// Diameter (or side), stage units.
    pub size: f32,
    pub shape: TipShape,
    pub mode: EraseMode,
    /// The Faucet: a click deletes a whole fill or line.
    pub faucet: bool,
}

impl Default for EraserOptions {
    fn default() -> Self {
        EraserOptions { size: 16.0, shape: TipShape::Round, mode: EraseMode::Normal, faucet: false }
    }
}

impl EraserOptions {
    /// The tip as the Brush draws it (for the shared previews).
    fn tip(&self) -> BrushOptions {
        BrushOptions { size: self.size, shape: self.shape, ..BrushOptions::default() }
    }
}

/// Starts an eraser stroke at stage point `p` (the eraser ignores pen pressure).
pub fn start(app: &mut App, p: Vec2) -> Drag {
    app.pen_pressure = None;
    app.brush_samples = vec![(p, None)];
    Drag::Eraser
}

/// Sends the stroke: one command, one undo step.
pub fn commit(app: &mut App, xf: Xf) {
    let samples = std::mem::take(&mut app.brush_samples);
    if samples.is_empty() {
        return;
    }
    let o = app.eraser;
    let params = json!({"points": points(&samples), "size": o.size, "shape": shape_key(o.shape), "mode": o.mode.key(), "unit": xf.unit()});
    let undo = crate::platform::shortcut_label("Ctrl+Z");
    match app.run("draw.erase", params) {
        Some(v) if v["erased"] == true => app.status_ok(format!("Erased. {undo} to undo.")),
        Some(v) => {
            let why = v["reason"].as_str().unwrap_or("nothing to erase there");
            app.status_ok(format!("Nothing erased: {why}. The eraser works on lines and fills drawn with Object Drawing off; drawing objects and symbols are kept."));
        }
        None => {}
    }
}

/// The Faucet: deletes the whole fill or line under the click.
pub fn faucet(app: &mut App, p: Vec2, zoom: f32) {
    if let Some(v) = app.run("draw.faucet", json!({"x": p.x, "y": p.y, "slop": 4.0 / zoom.max(1e-3)})) {
        let what = if v["kind"] == "line" { "line" } else { "fill" };
        app.status_ok(format!("Deleted the {what}. {} to undo.", crate::platform::shortcut_label("Ctrl+Z")));
    }
}

/// Double-clicking the Eraser tool: erase everything on the stage at this frame (asks first).
pub fn erase_everything(app: &mut App) {
    // The engine asks only when there is something to erase.
    if app.s.confirmation("draw.erase_all", &json!({})).is_none() {
        app.status_ok("Nothing to erase: the visible, unlocked layers are empty at this frame.");
        return;
    }
    app.request("draw.erase_all", json!({}));
}

/// The stroke as it goes (in the stage colour, as if erased) and the tip under the pointer.
pub fn draw_overlay(app: &App, painter: &egui::Painter, xf: Xf, hover: Option<Pos2>) {
    if app.tool != Tool::Eraser {
        return;
    }
    let tip = app.eraser.tip();
    if app.drag == Drag::Eraser {
        let bg = to_color32(anicraft_engine::doc::Rgba { a: 1.0, ..app.s.doc.stage.background });
        draw_sweep(painter, xf, &tip, &app.brush_samples, bg);
    }
    if let Some(h) = hover
        && !app.eraser.faucet
    {
        draw_tip(painter, xf, &tip, xf.to_stage(h));
    }
}

/// The Eyedropper's click. It picks the fill or line under the point into the tool settings
/// (Shift: into the other swatch only). As in Animate, it then becomes the Paint Bucket (a
/// fill) or the Ink Bottle (a line), and with shapes selected it gives them the picked style.
pub fn eyedropper(app: &mut App, p: Vec2, shift: bool, zoom: f32) {
    let Some(v) = app.run("paint.pick", json!({"x": p.x, "y": p.y, "other": shift, "slop": 3.0 / zoom.max(1e-3)})) else { return };
    let color = v["color"].as_str().unwrap_or("").to_string();
    let into_fill = v["into"] == "fill";
    if shift {
        app.status_ok(format!("Picked {color} into the {} colour.", if into_fill { "fill" } else { "stroke" }));
        return;
    }
    let what = match v["paint"].as_str() {
        Some("linear") => "linear gradient".to_string(),
        Some("radial") => "radial gradient".to_string(),
        Some("bitmap") => "bitmap fill".to_string(),
        _ if into_fill => format!("fill {color}"),
        _ => format!("line {color}, {} px", v["width"].as_f64().map_or(String::new(), |w| format!("{w:.1}"))),
    };
    let sel = &app.s.selection;
    let shapes = app.s.visible_elements().iter().filter(|e| sel.contains(&e.id) && matches!(e.kind, ElementKind::Shape(_))).count();
    if shapes > 0 {
        if let Some(r) = app.run("paint.apply_style", json!({"fill": into_fill, "stroke": !into_fill})) {
            let n = r["changed"].as_u64().unwrap_or(0);
            app.status_ok(format!("Applied the picked {what} to {n} {}. {} to undo.", if n == 1 { "shape" } else { "shapes" }, crate::platform::shortcut_label("Ctrl+Z")));
        }
    } else {
        let next = if into_fill { "Click a fill or an enclosed area to pour it" } else { "Click a shape to give it this line" };
        app.status_ok(format!("Picked the {what}. {next}."));
    }
    app.tool = if into_fill { Tool::PaintBucket } else { Tool::InkBottle };
}
