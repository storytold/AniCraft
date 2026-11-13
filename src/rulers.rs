//! Rulers, grid and guides on the stage (ADR 0076), as Animate has them: View › Rulers puts a
//! ruler along the top and the left of the stage, in stage pixels from its top-left corner,
//! following zoom and pan, with the pointer marked on both. Drag from a ruler onto the stage to
//! make a guide; drag a guide with the Selection tool to move it, and back onto a ruler (or off
//! the stage) to delete it. The grid shows under the artwork (or over it), the guides over it.
//!
//! What is drawn here is view only: it never reaches a picture or an export. Every change goes
//! through a `view.*` command, so the same works headless.

use crate::a11y::Named;
use crate::app::{App, Drag, Tool};
use crate::theme::{RULER, T, Type, font, to_color32};
use anicraft_engine::doc::{Guide, GuideAxis};
use egui::{Align2, CursorIcon, Painter, Pos2, Rect, Sense, Stroke, pos2, vec2};
use serde_json::json;

/// How near (screen points) the pointer must be to a guide to grab it.
const GRAB: f32 = 4.0;
/// Grid lines closer than this on screen are thinned out (every second, fifth… line).
const GRID_MIN_PX: f32 = 6.0;
/// Most lines one ruler, or the grid along one axis, draws.
const MAX_LINES: usize = 2_000;

/// A guide being dragged out of a ruler: where it would go (stage pixels), while the pointer is
/// over the stage.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NewGuide {
    pub axis: GuideAxis,
    pub at: Option<f32>,
}

/// Where the stage is on screen this frame: its top-left corner and the zoom (not an open
/// symbol's space: rulers, grid and guides are the stage's).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geo {
    /// The stage view, inside the rulers.
    pub canvas: Rect,
    pub origin: Pos2,
    pub zoom: f32,
}

impl Geo {
    /// A screen position as a stage coordinate along the axis a guide of `axis` measures.
    fn stage_at(&self, axis: GuideAxis, p: Pos2) -> f32 {
        match axis {
            GuideAxis::Horizontal => (p.y - self.origin.y) / self.zoom,
            GuideAxis::Vertical => (p.x - self.origin.x) / self.zoom,
        }
    }
    /// Where a guide is on screen (y for a horizontal one, x for a vertical one).
    fn screen_at(&self, g: &Guide) -> f32 {
        match g.axis {
            GuideAxis::Horizontal => self.origin.y + g.at * self.zoom,
            GuideAxis::Vertical => self.origin.x + g.at * self.zoom,
        }
    }
}

/// The rulers' strips, when they show.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Bars {
    pub top: Option<Rect>,
    pub left: Option<Rect>,
    pub corner: Option<Rect>,
}

/// Splits the stage's area: the rulers along the top and the left (View › Rulers), and the stage
/// view in what is left.
pub fn layout(app: &App, full: Rect) -> (Rect, Bars) {
    if !app.s.aids.rulers || full.width() < 4.0 * RULER || full.height() < 4.0 * RULER {
        return (full, Bars::default());
    }
    let inner = Rect::from_min_max(full.min + vec2(RULER, RULER), full.max);
    let bars = Bars {
        top: Some(Rect::from_min_max(pos2(inner.min.x, full.min.y), pos2(full.max.x, inner.min.y))),
        left: Some(Rect::from_min_max(pos2(full.min.x, inner.min.y), pos2(inner.min.x, full.max.y))),
        corner: Some(Rect::from_min_max(full.min, inner.min)),
    };
    (inner, bars)
}

/// A guide position as a drag leaves it: a whole stage pixel (the Edit Guides dialog and the
/// commands take any position).
fn whole(at: f32) -> f32 {
    at.round()
}

/// The rulers' input: a drag from the top ruler makes a horizontal guide, from the left one a
/// vertical guide, where it is let go over the stage.
pub fn input(app: &mut App, ui: &mut egui::Ui, bars: &Bars, geo: Geo, paused: bool) {
    for (rect, axis, name, id) in [(bars.top, GuideAxis::Horizontal, tr!("Horizontal ruler"), "ruler_top"), (bars.left, GuideAxis::Vertical, tr!("Vertical ruler"), "ruler_left")] {
        let Some(rect) = rect else { continue };
        let resp = ui.interact(rect, ui.id().with(id), Sense::drag()).named(name);
        let resp = resp.on_hover_text(tr!("Drag onto the stage to add a guide (View › Guides › Edit Guides adds one by position)"));
        if resp.hovered() || resp.dragged() {
            ui.ctx().set_cursor_icon(if axis == GuideAxis::Horizontal { CursorIcon::ResizeRow } else { CursorIcon::ResizeColumn });
        }
        if paused {
            continue;
        }
        if resp.drag_started() {
            app.new_guide = Some(NewGuide { axis, at: None });
        }
        if resp.dragged()
            && let Some(p) = resp.interact_pointer_pos()
        {
            app.new_guide = Some(NewGuide { axis, at: geo.canvas.contains(p).then(|| whole(geo.stage_at(axis, p))) });
        }
        if resp.drag_stopped()
            && let Some(NewGuide { axis, at }) = app.new_guide.take()
        {
            add_guide(app, axis, at);
        }
    }
}

/// Adds the guide a ruler drag let go of (`None`: let go off the stage, so nothing).
pub fn add_guide(app: &mut App, axis: GuideAxis, at: Option<f32>) {
    let Some(at) = at else { return };
    if app.run("view.guide_add", json!({"axis": axis.key(), "at": at})).is_some() {
        if !app.s.aids.show_guides {
            app.run("view.guides_show", json!({"on": true}));
        }
        app.status_ok(tr!("Added a guide. Drag it with the Selection tool to move it, or back onto a ruler to delete it. {undo} to undo.", undo = app.undo_key()));
    }
}

/// The guide under screen point `p` that the Selection tool would grab (shown, not locked).
pub fn guide_at(app: &App, geo: Geo, p: Pos2) -> Option<usize> {
    if app.tool != Tool::Select || !app.s.aids.show_guides || app.s.aids.lock_guides || !geo.canvas.contains(p) {
        return None;
    }
    app.s
        .guides()
        .iter()
        .enumerate()
        .map(|(i, g)| {
            let along = if g.axis == GuideAxis::Horizontal { p.y } else { p.x };
            (i, (geo.screen_at(g) - along).abs())
        })
        .filter(|(_, d)| *d <= GRAB)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// The cursor over guide `i`: the way it can move.
pub fn cursor(app: &App, i: usize) -> CursorIcon {
    match app.s.guides().get(i).map(|g| g.axis) {
        Some(GuideAxis::Horizontal) => CursorIcon::ResizeRow,
        _ => CursorIcon::ResizeColumn,
    }
}

/// A press on guide `i`: the drag that moves it (one undo step).
pub fn drag_start(app: &mut App, i: usize) -> Drag {
    app.s.begin_gesture(tr_noop!("Move Guide"));
    Drag::Guide { index: i, off: false }
}

/// Guide `index` follows the pointer at screen point `p` while it is over the stage.
pub fn drag_to(app: &mut App, index: usize, geo: Geo, p: Pos2) {
    let Some(g) = app.s.guides().get(index).copied() else { return };
    let off = !geo.canvas.contains(p);
    app.drag = Drag::Guide { index, off };
    if off {
        return;
    }
    let at = whole(geo.stage_at(g.axis, p));
    if at != g.at {
        let _ = app.s.execute("view.guide_move", &json!({"index": index, "at": at}));
    }
}

/// The guide drag ends: let go over a ruler or off the stage, the guide is deleted, as in
/// Animate; the move or the deletion is one undo step.
pub fn drag_end(app: &mut App, index: usize, geo: Geo, p: Option<Pos2>) {
    let off = p.is_none_or(|p| !geo.canvas.contains(p));
    if off && app.run("view.guide_delete", json!({"index": index})).is_some() {
        app.s.end_gesture();
        app.status_ok(tr!("Deleted the guide. {undo} to undo.", undo = app.undo_key()));
        return;
    }
    app.s.end_gesture();
}

/// Whether rulers, grid and guides show and snap changed: into the preferences (not in
/// screenshot or test runs, which never write the user's preferences).
pub fn sync(app: &mut App) {
    if app.s.aids == app.prefs.aids {
        return;
    }
    app.prefs.aids = app.s.aids;
    if app.shot.is_none()
        && let Err(e) = app.prefs.save()
    {
        app.status_err(tr!("Could not save preferences: {e}", e = e));
    }
}

/// Whether the grid shows under the artwork (the stage's tiles are then rendered without the
/// stage colour, which is painted under the grid instead).
pub fn grid_under(app: &App) -> bool {
    app.s.aids.show_grid && !app.s.doc.aids.grid.over_objects
}

/// Crisp one-pixel lines: on the middle of a device pixel.
fn crisp(v: f32, ppp: f32) -> f32 {
    ((v * ppp).floor() + 0.5) / ppp
}

/// The grid, over the stage's rectangle (where it is in view).
pub fn paint_grid(app: &App, painter: &Painter, geo: Geo, stage_rect: Rect, ppp: f32) {
    let area = stage_rect.intersect(geo.canvas);
    if !app.s.aids.show_grid || area.width() <= 0.0 || area.height() <= 0.0 || geo.zoom.is_nan() || geo.zoom <= 0.0 {
        return;
    }
    let g = app.s.doc.aids.grid;
    let stroke = Stroke::new(1.0 / ppp, to_color32(g.color));
    for (spacing, vertical) in [(g.spacing_x, true), (g.spacing_y, false)] {
        let px = spacing * geo.zoom;
        if px.is_nan() || px <= 0.0 {
            continue;
        }
        // Too dense to see: every k-th line.
        let step = spacing * (GRID_MIN_PX / px).ceil().max(1.0);
        let (lo, hi, o) = if vertical { (area.min.x, area.max.x, geo.origin.x) } else { (area.min.y, area.max.y, geo.origin.y) };
        let (first, last) = (((lo - o) / geo.zoom / step).ceil(), ((hi - o) / geo.zoom / step).floor());
        if !(first.is_finite() && last.is_finite()) || last - first > MAX_LINES as f32 {
            continue;
        }
        let mut n = first;
        while n <= last {
            let v = crisp(o + n * step * geo.zoom, ppp);
            let seg = if vertical { [pos2(v, area.min.y), pos2(v, area.max.y)] } else { [pos2(area.min.x, v), pos2(area.max.x, v)] };
            painter.line_segment(seg, stroke);
            n += 1.0;
        }
    }
}

/// The scene's guides across the stage view, the one being dragged a little bolder, and a
/// guide coming out of a ruler.
pub fn paint_guides(app: &App, painter: &Painter, geo: Geo, ppp: f32) {
    let c = geo.canvas;
    let colour = to_color32(app.s.doc.aids.guide_color);
    let line = |at: f32, axis: GuideAxis, width: f32| {
        let v = crisp(at, ppp);
        let seg = match axis {
            GuideAxis::Horizontal => [pos2(c.min.x, v), pos2(c.max.x, v)],
            GuideAxis::Vertical => [pos2(v, c.min.y), pos2(v, c.max.y)],
        };
        let inside = match axis {
            GuideAxis::Horizontal => v >= c.min.y && v <= c.max.y,
            GuideAxis::Vertical => v >= c.min.x && v <= c.max.x,
        };
        if inside {
            painter.line_segment(seg, Stroke::new(width, colour));
        }
    };
    if app.s.aids.show_guides {
        let dragged = match app.drag {
            Drag::Guide { index, off } => Some((index, off)),
            _ => None,
        };
        for (i, g) in app.s.guides().iter().enumerate() {
            match dragged {
                // Over a ruler, the guide being dragged is about to go: it isn't drawn.
                Some((d, true)) if d == i => {}
                Some((d, false)) if d == i => line(geo.screen_at(g), g.axis, 2.0 / ppp),
                _ => line(geo.screen_at(g), g.axis, 1.0 / ppp),
            }
        }
    }
    if let Some(NewGuide { axis, at: Some(at) }) = app.new_guide {
        line(geo.screen_at(&Guide { axis, at }), axis, 2.0 / ppp);
    }
}

/// The step between labelled ticks: the first that leaves room for a number on screen.
fn major_step(zoom: f32) -> f32 {
    const STEPS: [f32; 17] = [1.0, 2.0, 5.0, 10.0, 20.0, 25.0, 50.0, 100.0, 200.0, 250.0, 500.0, 1_000.0, 2_000.0, 2_500.0, 5_000.0, 10_000.0, 20_000.0];
    STEPS.into_iter().find(|s| s * zoom >= 56.0).unwrap_or(50_000.0)
}

/// Ticks between labelled ones, as many as stay at least 5 screen points apart.
fn minor_count(major: f32, zoom: f32) -> f32 {
    [10.0, 5.0, 2.0].into_iter().find(|n| major / n * zoom >= 5.0 && (major / n).fract() == 0.0).unwrap_or(1.0)
}

/// Both rulers, the corner where they meet, and the pointer's place on each.
pub fn paint(ui: &egui::Ui, bars: &Bars, geo: Geo, pointer: Option<Pos2>) {
    let painter = ui.painter();
    if let Some(c) = bars.corner {
        painter.rect_filled(c, 0.0, T.panel_alt);
    }
    for (rect, horizontal) in [(bars.top, true), (bars.left, false)] {
        let Some(rect) = rect else { continue };
        let p = painter.with_clip_rect(rect);
        p.rect_filled(rect, 0.0, T.panel_alt);
        let (lo, hi, o) = if horizontal { (rect.min.x, rect.max.x, geo.origin.x) } else { (rect.min.y, rect.max.y, geo.origin.y) };
        let major = major_step(geo.zoom);
        let n = minor_count(major, geo.zoom);
        let minor = major / n;
        let (first, last) = (((lo - o) / geo.zoom / minor).floor(), ((hi - o) / geo.zoom / minor).ceil());
        if first.is_finite() && last.is_finite() && last - first <= MAX_LINES as f32 {
            let mut k = first;
            while k <= last {
                let v = k * minor;
                let s = o + v * geo.zoom;
                let at_major = (k / n).fract() == 0.0;
                let half = n >= 2.0 && ((k * 2.0) / n).fract() == 0.0;
                let len = if at_major { RULER * 0.7 } else if half { RULER * 0.4 } else { RULER * 0.22 };
                let (a, b) = if horizontal { (pos2(s, rect.max.y - len), pos2(s, rect.max.y)) } else { (pos2(rect.max.x - len, s), pos2(rect.max.x, s)) };
                p.line_segment([a, b], Stroke::new(1.0, T.text_faint));
                if at_major {
                    let label = format!("{}", v.round() as i64);
                    if horizontal {
                        p.text(pos2(s + 3.0, rect.min.y + 1.0), Align2::LEFT_TOP, label, font(Type::DataSmall), T.text_dim);
                    } else {
                        let galley = p.layout_no_wrap(label, font(Type::DataSmall), T.text_dim);
                        // Read bottom to top, as rulers are.
                        let at = pos2(rect.min.x + 1.0, s - 3.0);
                        p.add(egui::epaint::TextShape::new(at, galley, T.text_dim).with_angle(-std::f32::consts::FRAC_PI_2));
                    }
                }
                k += 1.0;
            }
        }
        // The edge against the stage.
        let edge = if horizontal { [pos2(rect.min.x, rect.max.y - 0.5), pos2(rect.max.x, rect.max.y - 0.5)] } else { [pos2(rect.max.x - 0.5, rect.min.y), pos2(rect.max.x - 0.5, rect.max.y)] };
        p.line_segment(edge, Stroke::new(1.0, T.border));
        // Where the pointer is.
        if let Some(q) = pointer.filter(|q| geo.canvas.contains(*q) || rect.contains(*q)) {
            let seg = if horizontal { [pos2(q.x, rect.min.y), pos2(q.x, rect.max.y)] } else { [pos2(rect.min.x, q.y), pos2(rect.max.x, q.y)] };
            p.line_segment(seg, Stroke::new(1.0, T.accent_text));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anicraft_engine::Session;
    use anicraft_engine::doc::Document;

    fn app() -> App {
        let ctx = egui::Context::default();
        App::with_context(&ctx, Session::new(Document::default()), Some(std::path::PathBuf::from("unused.png")))
    }

    /// A 1280 × 720 stage at 50 %, its corner at (100, 50) on screen.
    const GEO: Geo = Geo { canvas: Rect { min: Pos2 { x: 18.0, y: 18.0 }, max: Pos2 { x: 1000.0, y: 700.0 } }, origin: Pos2 { x: 100.0, y: 50.0 }, zoom: 0.5 };

    #[test]
    fn rulers_take_the_top_and_left_edges_only_while_shown() {
        let mut app = app();
        let full = Rect::from_min_max(pos2(0.0, 0.0), pos2(800.0, 600.0));
        assert_eq!(layout(&app, full), (full, Bars::default()));
        app.run("view.rulers", json!({}));
        let (canvas, bars) = layout(&app, full);
        assert_eq!(canvas.min, pos2(RULER, RULER));
        assert_eq!(bars.top.map(|r| r.height()), Some(RULER));
        assert_eq!(bars.left.map(|r| r.width()), Some(RULER));
    }

    #[test]
    fn a_guide_dragged_from_a_ruler_lands_where_it_was_let_go() {
        let mut app = app();
        // Let go at screen y = 150: (150 - 50) / 0.5 = stage y 200.
        app.new_guide = Some(NewGuide { axis: GuideAxis::Horizontal, at: Some(whole(GEO.stage_at(GuideAxis::Horizontal, pos2(400.0, 150.2)))) });
        let NewGuide { axis, at } = app.new_guide.take().unwrap();
        add_guide(&mut app, axis, at);
        assert_eq!(app.s.guides(), [Guide { axis: GuideAxis::Horizontal, at: 200.0 }]);
        // Let go back over the ruler: nothing.
        add_guide(&mut app, GuideAxis::Vertical, None);
        assert_eq!(app.s.guides().len(), 1);
    }

    #[test]
    fn a_guide_is_grabbed_moved_and_dragged_off_to_delete() {
        let mut app = app();
        app.run("view.guide_add", json!({"axis": "vertical", "at": 300}));
        // Stage x 300 is screen x 250.
        assert_eq!(guide_at(&app, GEO, pos2(252.0, 400.0)), Some(0));
        assert_eq!(guide_at(&app, GEO, pos2(260.0, 400.0)), None, "too far");
        app.tool = Tool::Rect;
        assert_eq!(guide_at(&app, GEO, pos2(250.0, 400.0)), None, "only the Selection tool moves guides");
        app.tool = Tool::Select;
        // Moved to screen x 450 (stage 700), in one undo step.
        app.drag = drag_start(&mut app, 0);
        drag_to(&mut app, 0, GEO, pos2(400.0, 300.0));
        drag_to(&mut app, 0, GEO, pos2(450.2, 300.0));
        drag_end(&mut app, 0, GEO, Some(pos2(450.2, 300.0)));
        assert_eq!(app.s.guides(), [Guide { axis: GuideAxis::Vertical, at: 700.0 }]);
        assert_eq!(app.s.undo_label(), Some("Move Guide"));
        app.run("edit.undo", json!({}));
        assert_eq!(app.s.guides()[0].at, 300.0);
        // Dragged onto the left ruler: deleted, one step with the move before it.
        app.drag = drag_start(&mut app, 0);
        drag_to(&mut app, 0, GEO, pos2(200.0, 300.0));
        drag_to(&mut app, 0, GEO, pos2(8.0, 300.0));
        assert_eq!(app.drag, Drag::Guide { index: 0, off: true });
        drag_end(&mut app, 0, GEO, Some(pos2(8.0, 300.0)));
        assert!(app.s.guides().is_empty());
        app.run("edit.undo", json!({}));
        assert_eq!(app.s.guides()[0].at, 300.0, "one undo brings it back where it was");
        // Locked guides can't be grabbed.
        app.run("view.guides_lock", json!({}));
        assert_eq!(guide_at(&app, GEO, pos2(250.0, 400.0)), None);
    }

    #[test]
    fn ruler_ticks_stay_readable_at_every_zoom() {
        for zoom in [0.05f32, 0.25, 0.5, 1.0, 3.0, 16.0, 64.0] {
            let major = major_step(zoom);
            assert!(major * zoom >= 56.0 || major == 50_000.0, "zoom {zoom}");
            let minor = major / minor_count(major, zoom);
            assert!(minor * zoom >= 5.0 || minor == major, "zoom {zoom}: minor {minor}");
        }
    }
}
