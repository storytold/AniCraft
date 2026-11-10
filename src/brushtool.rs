//! The Brush tool (B, ADR 0033): view state over `draw.brush`. A drag collects pointer samples
//! (with pen pressure when the tablet reports it), shows the paint as it goes, and sends the
//! stroke as one command when the pointer lifts, so a stroke is one undo step. A click paints a
//! dab. `[` and `]` change the size.

use crate::app::{App, Drag, Tool};
use crate::pathtools::Xf;
use crate::theme::{T, to_color32};
use anicraft_engine::cmds_brush::PaintMode;
use anicraft_engine::geom::brush::{TipShape, size_factor};
use anicraft_engine::geom::{Affine, Vec2};
use egui::{Pos2, Stroke};
use serde_json::json;

/// The tool options (view state, as in Animate: they are the tool's, not the document's).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BrushOptions {
    /// Diameter (or side), stage units.
    pub size: f32,
    pub shape: TipShape,
    /// Degrees, clockwise.
    pub angle: f32,
    /// Percent: the narrow side as part of the wide one.
    pub roundness: f32,
    /// 0..100.
    pub smoothing: f32,
    pub mode: PaintMode,
}

impl Default for BrushOptions {
    fn default() -> Self {
        BrushOptions { size: 10.0, shape: TipShape::Round, angle: 0.0, roundness: 100.0, smoothing: 50.0, mode: PaintMode::Normal }
    }
}

/// Sizes the tool offers (the command takes 0.1..2000).
pub const SIZE_MIN: f32 = 1.0;
pub const SIZE_MAX: f32 = 400.0;

impl BrushOptions {
    /// The tip mapped onto the stage, for previews: unit circle or square → the tip at `at`.
    pub fn tip_to_stage(&self, at: Vec2, factor: f32) -> Affine {
        let tip = anicraft_engine::geom::brush::Tip { shape: self.shape, size: self.size * factor, angle: self.angle, roundness: self.roundness / 100.0 };
        Affine::translate(at.x, at.y).mul(&tip.to_stage())
    }

    /// The tip's outline as points on the unit circle or square.
    fn unit_outline(&self) -> Vec<Vec2> {
        match self.shape {
            TipShape::Square => vec![Vec2::new(-1.0, -1.0), Vec2::new(1.0, -1.0), Vec2::new(1.0, 1.0), Vec2::new(-1.0, 1.0)],
            TipShape::Round => (0..24).map(|i| i as f32 / 24.0 * std::f32::consts::TAU).map(|a| Vec2::new(a.cos(), a.sin())).collect(),
        }
    }
}

/// `[` and `]`: the next size down or up (steps of 1 below 10, 5 below 50, 10 below 200, then
/// 50), as in paint programs.
pub fn step_size(size: f32, larger: bool) -> f32 {
    let step = |s: f32| if s < 10.0 { 1.0 } else if s < 50.0 { 5.0 } else if s < 200.0 { 10.0 } else { 50.0 };
    let next = if larger { size + step(size) } else { size - step(size - 0.01) };
    // Land on the step grid, so sizes stay round numbers.
    let g = step(if larger { size } else { size - 0.01 });
    ((next / g).round() * g).clamp(SIZE_MIN, SIZE_MAX)
}

/// Changes the Brush's or the Eraser's size from the keyboard and says so.
pub fn resize(app: &mut App, larger: bool) {
    match app.tool {
        Tool::Brush => {
            app.brush.size = step_size(app.brush.size, larger);
            app.status_ok(tr!("Brush size {size} px", size = app.brush.size));
        }
        Tool::Eraser => {
            app.eraser.size = step_size(app.eraser.size, larger);
            app.status_ok(tr!("Eraser size {size} px", size = app.eraser.size));
        }
        _ => {}
    }
}

/// The pen's pressure this frame, when a tablet reports it (egui gives it with touch events).
pub fn frame_pressure(ctx: &egui::Context) -> Option<f32> {
    ctx.input(|i| {
        i.raw.events.iter().rev().find_map(|e| match e {
            egui::Event::Touch { force: Some(f), .. } if f.is_finite() => Some(f.clamp(0.0, 1.0)),
            _ => None,
        })
    })
}

/// Starts a stroke at stage point `p`. A mouse paints at full pressure.
pub fn start(app: &mut App, p: Vec2, pressure: Option<f32>) -> Drag {
    app.pen_pressure = pressure;
    app.brush_samples = vec![(p, pressure)];
    Drag::Brush
}

/// The stroke's samples as command points (with pressure when a pen gave any).
pub fn points(samples: &[(Vec2, Option<f32>)]) -> Vec<serde_json::Value> {
    let pen = samples.iter().any(|(_, p)| p.is_some());
    samples.iter().map(|(v, p)| if pen { json!([v.x, v.y, p.unwrap_or(1.0)]) } else { json!([v.x, v.y]) }).collect()
}

/// A tip shape's name in commands.
pub fn shape_key(s: TipShape) -> &'static str {
    match s {
        TipShape::Round => "round",
        TipShape::Square => "square",
    }
}

/// Adds a sample (closer than half a screen point adds nothing unless the pressure moved).
pub fn sample(app: &mut App, p: Vec2, pressure: Option<f32>, xf: Xf) {
    if pressure.is_some() {
        app.pen_pressure = pressure;
    }
    let pr = app.pen_pressure;
    let far = app.brush_samples.last().is_none_or(|(l, lp)| l.distance(p) * xf.zoom >= 0.5 || (pr.unwrap_or(1.0) - lp.unwrap_or(1.0)).abs() > 0.05);
    if far && app.brush_samples.len() < anicraft_engine::geom::brush::MAX_SAMPLES {
        app.brush_samples.push((p, pr));
    }
}

/// Sends the stroke (one command, one undo step). Smoothing and accuracy follow the screen.
pub fn commit(app: &mut App, xf: Xf) {
    let samples = std::mem::take(&mut app.brush_samples);
    if samples.is_empty() {
        return;
    }
    let points = points(&samples);
    let o = app.brush;
    let params = json!({"points": points, "size": o.size, "shape": shape_key(o.shape), "angle": o.angle, "roundness": o.roundness, "smoothing": o.smoothing, "mode": o.mode.key(), "unit": xf.unit()});
    if let Some(v) = app.run("draw.brush", params)
        && v["elements"].as_array().is_some_and(Vec::is_empty)
    {
        app.status_ok(tr!("Nothing to paint behind there: Paint Behind only paints empty areas."));
    }
}

/// Convex hull (Andrew's monotone chain), counter-clockwise on screen.
fn hull(mut pts: Vec<Pos2>) -> Vec<Pos2> {
    pts.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    pts.dedup();
    if pts.len() < 3 {
        return pts;
    }
    let cross = |o: Pos2, a: Pos2, b: Pos2| (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x);
    let mut lower: Vec<Pos2> = Vec::with_capacity(pts.len());
    for &p in &pts {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], p) <= 0.0 {
            lower.pop();
        }
        lower.push(p);
    }
    let mut upper: Vec<Pos2> = Vec::with_capacity(pts.len());
    for &p in pts.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], p) <= 0.0 {
            upper.pop();
        }
        upper.push(p);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// Most preview pieces drawn per frame (a long stroke skips samples in the preview only).
const MAX_PREVIEW: usize = 4000;

/// The paint as it goes (each pair of samples is the hull of the tip at both, so the union is
/// what the stroke covers before smoothing), and the tip under the pointer.
pub fn draw_overlay(app: &App, painter: &egui::Painter, xf: Xf, hover: Option<Pos2>) {
    if app.tool != Tool::Brush {
        return;
    }
    if app.drag == Drag::Brush {
        // Opaque, so overlapping pieces don't darken each other.
        let color = to_color32(anicraft_engine::doc::Rgba { a: 1.0, ..app.s.style.fill });
        draw_sweep(painter, xf, &app.brush, &app.brush_samples, color);
    }
    if let Some(h) = hover {
        // The tip's size and shape at the pointer, as Animate shows it.
        draw_tip(painter, xf, &app.brush, xf.to_stage(h));
    }
}

/// The tip swept along `samples` (shared with the Eraser, ADR 0034), filled with `color`.
pub fn draw_sweep(painter: &egui::Painter, xf: Xf, o: &BrushOptions, samples: &[(Vec2, Option<f32>)], color: egui::Color32) {
    if samples.is_empty() {
        return;
    }
    let unit = o.unit_outline();
    let screen = |at: Vec2, pressure: Option<f32>| -> Vec<Pos2> {
        let m = o.tip_to_stage(at, size_factor(pressure.unwrap_or(1.0)));
        unit.iter().map(|u| xf.to_screen(m.apply(*u))).collect()
    };
    let step = samples.len().div_ceil(MAX_PREVIEW).max(1);
    let picked: Vec<&(Vec2, Option<f32>)> = samples.iter().step_by(step).chain(samples.last()).collect();
    if let [only] = picked.as_slice() {
        painter.add(egui::Shape::convex_polygon(hull(screen(only.0, only.1)), color, Stroke::NONE));
    }
    for pair in picked.windows(2) {
        let &[a, b] = pair else { continue };
        let mut pts = screen(a.0, a.1);
        pts.extend(screen(b.0, b.1));
        painter.add(egui::Shape::convex_polygon(hull(pts), color, Stroke::NONE));
    }
}

/// The tip's outline at stage point `at`.
pub fn draw_tip(painter: &egui::Painter, xf: Xf, o: &BrushOptions, at: Vec2) {
    let m = o.tip_to_stage(at, 1.0);
    let mut outline: Vec<Pos2> = o.unit_outline().iter().map(|u| xf.to_screen(m.apply(*u))).collect();
    if let Some(first) = outline.first().copied() {
        outline.push(first);
    }
    painter.add(egui::Shape::line(outline, Stroke::new(1.0, T.selection)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_steps_are_round_and_bounded() {
        assert_eq!(step_size(10.0, true), 15.0);
        assert_eq!(step_size(10.0, false), 9.0);
        assert_eq!(step_size(50.0, false), 45.0);
        assert_eq!(step_size(7.0, true), 8.0);
        assert_eq!(step_size(12.3, true), 15.0, "lands on the grid");
        assert_eq!(step_size(1.0, false), SIZE_MIN);
        assert_eq!(step_size(SIZE_MAX, true), SIZE_MAX);
    }

    #[test]
    fn hull_of_two_tips() {
        let h = hull(vec![Pos2::new(0.0, 0.0), Pos2::new(1.0, 0.0), Pos2::new(0.5, 0.2), Pos2::new(1.0, 1.0), Pos2::new(0.0, 1.0), Pos2::new(0.0, 0.0)]);
        assert_eq!(h.len(), 4, "{h:?}");
    }
}
