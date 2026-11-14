//! The Brush tool (B, ADR 0033) and the Paint Brush (Y, ADR 0068): view state over
//! `draw.brush`. A drag collects pointer samples (with pen pressure when the tablet reports it,
//! or the hand's speed standing in for it when Velocity is on), shows the paint as it goes, and
//! sends the stroke as one command when the pointer lifts, so a stroke is one undo step. A click
//! paints a dab. `[` and `]` change the size. The Paint Brush sends the same command with
//! `as_stroke`: a width-profiled line in the stroke colour.

use crate::app::{App, Drag, Tool};
use crate::pathtools::Xf;
use crate::theme::{T, to_color32};
use anicraft_engine::cmds_brush::PaintMode;
use anicraft_engine::brush_library::{BrushPreset, Shape};
use anicraft_engine::geom::brush::{TipShape, size_factor};
use anicraft_engine::geom::pressure::PressureCurve;
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
    /// How pen pressure maps (ADR 0068), and what it drives.
    pub curve: PressureCurve,
    pub pressure_size: bool,
    pub pressure_opacity: bool,
    /// With a mouse, the hand's speed stands in for pressure (faster is thinner). Off by default.
    pub velocity: bool,
    /// Percent of the stroke's length.
    pub taper_start: f32,
    pub taper_end: f32,
    /// 0, or percent of the size between dabs.
    pub spacing: f32,
    /// Percent of the size the edge wears in.
    pub jitter: f32,
    /// Percent: the paper grain through the paint (ADR 0093).
    pub texture: f32,
    /// Percent of the size dabs are thrown aside.
    pub scatter: f32,
    /// What the pen's tilt shapes, where the system reports it.
    pub tilt_angle: bool,
    pub tilt_width: bool,
}

impl Default for BrushOptions {
    /// The plain brush (ADR 0033).
    fn default() -> Self {
        BrushOptions::from_preset(&BrushPreset::default())
    }
}

/// Sizes the tool offers (the command takes 0.1..2000).
pub const SIZE_MIN: f32 = 1.0;
pub const SIZE_MAX: f32 = 400.0;

impl BrushOptions {
    /// The Paint Brush's own defaults: a thinner line.
    pub fn paint_brush() -> BrushOptions {
        BrushOptions { size: 6.0, ..BrushOptions::default() }
    }

    fn from_preset(p: &BrushPreset) -> BrushOptions {
        BrushOptions {
            size: p.size,
            shape: p.shape.tip(),
            angle: p.angle,
            roundness: p.roundness,
            smoothing: p.smoothing,
            mode: PaintMode::Normal,
            curve: p.curve,
            pressure_size: p.pressure_size,
            pressure_opacity: p.pressure_opacity,
            velocity: false,
            taper_start: p.taper_start,
            taper_end: p.taper_end,
            spacing: p.spacing,
            jitter: p.jitter,
            texture: p.texture,
            scatter: p.scatter,
            tilt_angle: p.tilt_angle,
            tilt_width: p.tilt_width,
        }
    }

    /// Takes a preset's settings (the paint mode and Velocity stay: they are the tool's).
    pub fn apply(&mut self, p: &BrushPreset) {
        *self = BrushOptions { mode: self.mode, velocity: self.velocity, ..BrushOptions::from_preset(&p.clone().sanitized()) };
    }

    /// The settings as a preset named `name`.
    pub fn preset(&self, name: &str) -> BrushPreset {
        BrushPreset {
            name: name.to_string(),
            size: self.size,
            shape: Shape::from_tip(self.shape),
            angle: self.angle,
            roundness: self.roundness,
            smoothing: self.smoothing,
            curve: self.curve,
            pressure_size: self.pressure_size,
            pressure_opacity: self.pressure_opacity,
            taper_start: self.taper_start,
            taper_end: self.taper_end,
            spacing: self.spacing,
            jitter: self.jitter,
            texture: self.texture,
            scatter: self.scatter,
            tilt_angle: self.tilt_angle,
            tilt_width: self.tilt_width,
        }
    }

    /// Whether these are a preset's settings, as they would be right after choosing it.
    pub fn matches(&self, p: &BrushPreset) -> bool {
        let mut o = *self;
        o.apply(p);
        o == *self
    }

    /// The size factor the tip has at a measured pressure (for previews).
    pub fn factor(&self, pressure: Option<f32>) -> f32 {
        size_factor(if self.pressure_size { self.curve.map(pressure.unwrap_or(1.0)) } else { 1.0 })
    }

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

/// The options of the brush tool in hand (the Paint Brush has its own).
pub fn options(app: &App) -> &BrushOptions {
    if app.tool == Tool::PaintBrush { &app.paint_brush } else { &app.brush }
}

pub fn options_mut(app: &mut App) -> &mut BrushOptions {
    if app.tool == Tool::PaintBrush { &mut app.paint_brush } else { &mut app.brush }
}

/// Changes the Brush's or the Eraser's size from the keyboard and says so.
pub fn resize(app: &mut App, larger: bool) {
    match app.tool {
        Tool::Brush | Tool::PaintBrush => {
            let o = options_mut(app);
            o.size = step_size(o.size, larger);
            let size = o.size;
            app.status_ok(tr!("Brush size {size} px", size = size));
        }
        Tool::Eraser => {
            app.eraser.size = step_size(app.eraser.size, larger);
            app.status_ok(tr!("Eraser size {size} px", size = app.eraser.size));
        }
        // Painting weights (ADR 0090).
        Tool::Bind if app.bind.paint => {
            app.bind.radius = step_size(app.bind.radius, larger).clamp(crate::bind_tool::RADIUS_MIN, crate::bind_tool::RADIUS_MAX);
            app.status_ok(tr!("Brush size {size} px", size = app.bind.radius));
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

/// The pen's tilt this frame, when the toolkit reports it (ADR 0093). egui 0.36's pointer and
/// touch events carry only pressure, and winit 0.30 reports tilt only on iOS, so on the desktop
/// this is `None` until they carry it. When it isn't, the Tilt options appear (see
/// [`App::pen_tilt_seen`]).
pub fn frame_tilt(ctx: &egui::Context) -> Option<anicraft_engine::geom::brush::Tilt> {
    // No egui event carries a tilt yet; this is where it is read once one does.
    let _ = ctx;
    None
}

/// Notes a tilt reported this frame, so the Tilt options show from then on.
pub fn note_tilt(app: &mut App, tilt: Option<anicraft_engine::geom::brush::Tilt>) {
    if tilt.is_some() {
        app.pen_tilt_seen = true;
    }
}

/// Starts a stroke at stage point `p` at time `now` (seconds). A mouse paints at full
/// pressure, or from half pressure when its speed stands in for it.
pub fn start(app: &mut App, p: Vec2, pressure: Option<f32>, now: f64) -> Drag {
    let pressure = pressure.or_else(|| options(app).velocity.then_some(VELOCITY_START));
    app.pen_pressure = pressure;
    app.brush_clock = now;
    app.brush_samples = vec![(p, pressure)];
    Drag::Brush
}

/// The pressure a mouse stroke starts at when its speed stands in for it.
pub const VELOCITY_START: f32 = 0.5;
/// Screen speed (points a second) at which a velocity stroke is thinnest.
const VELOCITY_FAST: f32 = 2400.0;

/// The pressure speed stands in for (ADR 0068): full when the hand is still, down to a fifth
/// at `VELOCITY_FAST` and beyond, eased from the last value so one jerky event doesn't show.
pub fn velocity_pressure(prev: f32, distance_pts: f32, dt: f32) -> f32 {
    let prev = if prev.is_finite() { prev.clamp(0.0, 1.0) } else { VELOCITY_START };
    if !(distance_pts.is_finite() && dt.is_finite() && dt > 1e-4) {
        return prev;
    }
    let speed = distance_pts / dt;
    let target = 1.0 - 0.8 * (speed / VELOCITY_FAST).clamp(0.0, 1.0);
    prev + (target - prev) * 0.3
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

/// Adds a sample at time `now` (closer than half a screen point adds nothing unless the
/// pressure moved).
pub fn sample(app: &mut App, p: Vec2, pressure: Option<f32>, xf: Xf, now: f64) {
    if pressure.is_some() {
        app.pen_pressure = pressure;
    } else if options(app).velocity
        && matches!(app.tool, Tool::Brush | Tool::PaintBrush)
        && let Some((last, _)) = app.brush_samples.last()
    {
        let moved = last.distance(p) * xf.zoom;
        if moved < 0.5 {
            return;
        }
        app.pen_pressure = Some(velocity_pressure(app.pen_pressure.unwrap_or(VELOCITY_START), moved, (now - app.brush_clock) as f32));
        app.brush_clock = now;
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
    let o = *options(app);
    let as_stroke = app.tool == Tool::PaintBrush;
    let mut params = o.preset("").params();
    params["points"] = json!(points);
    params["mode"] = json!(o.mode.key());
    params["unit"] = json!(xf.unit());
    params["as_stroke"] = json!(as_stroke);
    if as_stroke {
        if let Some(v) = app.draw("draw.brush", params)
            && v["width_points"].as_u64().is_some_and(|n| n > 1)
        {
            app.status_ok(tr!("A line whose width you can shape with the Width tool."));
        }
        return;
    }
    // Paint Selection keeps the selection it paints in, for the next stroke (ADR 0080).
    let v = if o.mode == PaintMode::Selection { app.run("draw.brush", params) } else { app.draw("draw.brush", params) };
    if let Some(v) = v
        && v["elements"].as_array().is_some_and(Vec::is_empty)
    {
        match o.mode {
            PaintMode::Selection => app.status_ok(tr!("Nothing painted: Paint Selection paints only inside the selected fills. Select them with the Selection tool first.")),
            PaintMode::Inside => app.status_ok(tr!("Nothing painted: Paint Inside paints only inside the fill where the stroke starts.")),
            _ => app.status_ok(tr!("Nothing to paint behind there: Paint Behind only paints empty areas.")),
        }
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
    if !matches!(app.tool, Tool::Brush | Tool::PaintBrush) {
        return;
    }
    // The Paint Brush draws a round line in the stroke colour (the tip's shape is the Brush's).
    let o = options(app);
    let (paint, tip) = if app.tool == Tool::PaintBrush { (app.s.style.stroke, BrushOptions { shape: TipShape::Round, roundness: 100.0, ..*o }) } else { (app.s.style.fill, *o) };
    if app.drag == Drag::Brush {
        // Opaque, so overlapping pieces don't darken each other.
        let color = to_color32(anicraft_engine::doc::Rgba { a: 1.0, ..paint });
        draw_sweep(painter, xf, &tip, &app.brush_samples, color);
    }
    if let Some(h) = hover {
        // The tip's size and shape at the pointer, as Animate shows it.
        draw_tip(painter, xf, &tip, xf.to_stage(h));
    }
}

/// The tip swept along `samples` (shared with the Eraser, ADR 0034), filled with `color`.
pub fn draw_sweep(painter: &egui::Painter, xf: Xf, o: &BrushOptions, samples: &[(Vec2, Option<f32>)], color: egui::Color32) {
    if samples.is_empty() {
        return;
    }
    let unit = o.unit_outline();
    let screen = |at: Vec2, pressure: Option<f32>| -> Vec<Pos2> {
        let m = o.tip_to_stage(at, o.factor(pressure));
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
    fn presets_go_in_and_out_of_the_options() {
        let ink = anicraft_engine::brush_library::builtin("ink_pen").unwrap();
        let mut o = BrushOptions { mode: PaintMode::Behind, velocity: true, ..BrushOptions::default() };
        assert!(!o.matches(&ink));
        o.apply(&ink);
        assert!(o.matches(&ink) && o.mode == PaintMode::Behind && o.velocity, "the tool's own settings stay");
        assert_eq!(o.preset("Ink Pen"), ink);
        o.size += 1.0;
        assert!(!o.matches(&ink), "changed since");
        // The defaults are the plain brush's (ADR 0033).
        assert!(BrushOptions::default().matches(&BrushPreset::default()));
        assert_eq!(BrushOptions::default().size, 10.0);
    }

    #[test]
    fn speed_stands_in_for_pressure() {
        // Still: it rises toward full; fast: it falls toward a fifth; never outside 0..1.
        let mut p = VELOCITY_START;
        for _ in 0..30 {
            p = velocity_pressure(p, 1.0, 0.1);
        }
        assert!(p > 0.95, "{p}");
        for _ in 0..30 {
            p = velocity_pressure(p, 500.0, 0.016);
        }
        assert!((p - 0.2).abs() < 0.02, "{p}");
        assert_eq!(velocity_pressure(0.4, 10.0, 0.0), 0.4, "no time passed");
        assert_eq!(velocity_pressure(f32::NAN, f32::INFINITY, 1.0), VELOCITY_START);
    }

    #[test]
    fn hull_of_two_tips() {

        let h = hull(vec![Pos2::new(0.0, 0.0), Pos2::new(1.0, 0.0), Pos2::new(0.5, 0.2), Pos2::new(1.0, 1.0), Pos2::new(0.0, 1.0), Pos2::new(0.0, 0.0)]);
        assert_eq!(h.len(), 4, "{h:?}");
    }
}
