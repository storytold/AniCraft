//! 9-slice guides on the stage (ADR 0055).
//!
//! While a symbol with guides for 9-slice scaling is open for editing (ADR 0045), its four
//! guides show as dashed lines across the stage, as in Animate. The Selection tool drags them:
//! each drag is one undo step (`symbol.set_scale_grid` inside a gesture). A guide can't pass the
//! other guide of its pair. Properties has the same four numbers.

use crate::app::{App, Drag, Tool};
use crate::pathtools::Xf;
use crate::theme::T;
use anicraft_engine::doc::{ScaleGrid, SymbolId};
use egui::{Color32, CursorIcon, Painter, Pos2, Rect, Stroke, pos2};
use serde_json::json;

/// Which guide.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Guide {
    Left,
    Right,
    Top,
    Bottom,
}

impl Guide {
    const ALL: [Guide; 4] = [Guide::Left, Guide::Right, Guide::Top, Guide::Bottom];

    fn vertical(self) -> bool {
        matches!(self, Guide::Left | Guide::Right)
    }

    /// The command parameter it sets.
    fn key(self) -> &'static str {
        match self {
            Guide::Left => "left",
            Guide::Right => "right",
            Guide::Top => "top",
            Guide::Bottom => "bottom",
        }
    }

    /// Where it is, in the symbol's space.
    fn at(self, g: &ScaleGrid) -> f32 {
        match self {
            Guide::Left => g.left,
            Guide::Right => g.right,
            Guide::Top => g.top,
            Guide::Bottom => g.bottom,
        }
    }

    pub fn cursor(self) -> CursorIcon {
        if self.vertical() { CursorIcon::ResizeHorizontal } else { CursorIcon::ResizeVertical }
    }
}

/// How near (screen points) the pointer takes a guide.
const GRAB: f32 = 4.0;

/// The symbol open for editing and its guides, when it has them.
pub fn shown(app: &App) -> Option<(SymbolId, ScaleGrid)> {
    let id = app.s.editing_symbol()?;
    app.s.doc.symbol(id).and_then(|s| s.scale_grid).map(|g| (id, g))
}

/// The guide under screen point `p`, for the Selection tool.
pub fn hit(app: &App, p: Pos2, xf: Xf) -> Option<Guide> {
    if app.tool != Tool::Select {
        return None;
    }
    let (_, g) = shown(app)?;
    let near = |guide: Guide| {
        let s = xf.to_screen(anicraft_engine::geom::Vec2::new(guide.at(&g), guide.at(&g)));
        if guide.vertical() { (p.x - s.x).abs() } else { (p.y - s.y).abs() }
    };
    Guide::ALL.into_iter().map(|guide| (guide, near(guide))).filter(|(_, d)| *d <= GRAB).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(guide, _)| guide)
}

/// Starts dragging `guide`: one undo step until the drag ends.
pub fn drag_start(app: &mut App, guide: Guide) -> Drag {
    app.s.begin_gesture(tr_noop!("Move 9-Slice Guide"));
    Drag::SliceGuide { guide }
}

/// The guide follows the pointer (stage point `c`, in the open symbol's space), to a tenth of a
/// pixel, stopping at the other guide of its pair.
pub fn drag_to(app: &mut App, guide: Guide, c: Pos2) {
    let Some((id, g)) = shown(app) else { return };
    let v = if guide.vertical() { c.x } else { c.y };
    let v = (v * 10.0).round() / 10.0;
    let v = match guide {
        Guide::Left => v.min(g.right),
        Guide::Right => v.max(g.left),
        Guide::Top => v.min(g.bottom),
        Guide::Bottom => v.max(g.top),
    };
    if v.is_finite() && (v - guide.at(&g)).abs() > 1e-4 {
        app.run("symbol.set_scale_grid", json!({"symbol": id.0, guide.key(): v}));
    }
}

/// Draws the guides across the canvas: dashed, over a light underlay; the one under the pointer
/// or being dragged heavier.
pub fn draw(app: &App, painter: &Painter, xf: Xf, canvas: Rect, hot: Option<Guide>) {
    let Some((_, g)) = shown(app) else { return };
    for guide in Guide::ALL {
        let s = xf.to_screen(anicraft_engine::geom::Vec2::new(guide.at(&g), guide.at(&g)));
        let ends = if guide.vertical() { [pos2(s.x, canvas.top()), pos2(s.x, canvas.bottom())] } else { [pos2(canvas.left(), s.y), pos2(canvas.right(), s.y)] };
        let width = if hot == Some(guide) { 2.0 } else { 1.0 };
        painter.line_segment(ends, Stroke::new(width + 2.0, Color32::WHITE.gamma_multiply(0.7)));
        painter.extend(egui::Shape::dashed_line(&ends, Stroke::new(width, T.slice_guide), 6.0, 4.0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Stage and screen coordinates coincide.
    const XF: Xf = Xf { origin: Pos2::ZERO, zoom: 1.0 };

    /// The `nineslicedit` screenshot state: the Panel symbol open in place, its guides at
    /// x = ±70 and y = ±14 in its space.
    fn app() -> App {
        let ctx = egui::Context::default();
        let session = anicraft_engine::Session::new(anicraft_engine::doc::sample::demo());
        let mut app = App::with_context(&ctx, session, Some(std::path::PathBuf::from("unused.png")));
        app.open_panel(Some("nineslicedit"));
        app
    }

    #[test]
    fn the_selection_tool_takes_the_guide_under_the_pointer() {
        let mut app = app();
        assert!(shown(&app).is_some(), "the open symbol has guides");
        assert_eq!(hit(&app, pos2(-70.0, 300.0), XF), Some(Guide::Left));
        assert_eq!(hit(&app, pos2(72.5, -300.0), XF), Some(Guide::Right));
        assert_eq!(hit(&app, pos2(500.0, 14.0), XF), Some(Guide::Bottom));
        assert_eq!(hit(&app, pos2(0.0, 0.0), XF), None);
        app.tool = Tool::Subselect;
        assert_eq!(hit(&app, pos2(-70.0, 0.0), XF), None, "only the Selection tool drags guides");
    }

    #[test]
    fn a_guide_drag_is_one_undo_step_and_stops_at_its_partner() {
        let mut app = app();
        let before = app.s.history_len();
        let d = drag_start(&mut app, Guide::Left);
        assert_eq!(d, Drag::SliceGuide { guide: Guide::Left });
        for x in [-60.0, -40.04, 200.0] {
            drag_to(&mut app, Guide::Left, pos2(x, 0.0));
        }
        app.s.end_gesture();
        let g = shown(&app).map(|(_, g)| g).unwrap();
        assert_eq!((g.left, g.right), (70.0, 70.0), "the left guide stops at the right one");
        assert_eq!(app.s.history_len(), before + 1);
        assert_eq!(app.s.undo_label(), Some("Move 9-Slice Guide"));
        app.s.undo().unwrap();
        assert_eq!(shown(&app).map(|(_, g)| g.left), Some(-70.0));
    }
}
