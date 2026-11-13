//! One snap engine for every tool (ADR 0076): a point being drawn, dragged or moved snaps to
//! the artwork (Snap to Objects, ADR 0041/0045), the scene's guides, the grid and whole pixels,
//! in a fixed order of priority:
//!
//! 1. an object's **point** (corner or end) within the radius: both coordinates;
//! 2. a **guide**, each coordinate on its own (a vertical guide takes x, a horizontal one y; two
//!    together are their crossing);
//! 3. an object's **line** or fill edge, only when no guide took either coordinate;
//! 4. the **grid**, each coordinate not yet taken;
//! 5. a whole **pixel**, each coordinate not yet taken (Snap to Pixels).
//!
//! The radius is what the caller asks (`snap`, in the space the tool works in: 10 screen pixels
//! at the zoom of the moment), so snapping feels the same at every zoom. The guides' and grid's
//! own accuracy scales it. Guides and the grid are in stage pixels; while a symbol is open in
//! place (ADR 0045) points are mapped there and back, so they line up with what the stage shows.

use crate::Session;
use anicraft_doc::GuideAxis;
use anicraft_geom::Vec2;
use serde::{Deserialize, Serialize};

/// How near a point must come to a guide or a grid line to snap to it (Animate's Snap
/// accuracy), as a factor of the radius the tool asks for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Accuracy {
    /// Must be close: half the usual distance.
    Close,
    #[default]
    Normal,
    /// Can be distant: twice the usual distance.
    Distant,
    /// Always snap (the grid only): every point goes to the nearest line.
    Always,
}

impl Accuracy {
    pub const ALL: [Accuracy; 4] = [Accuracy::Close, Accuracy::Normal, Accuracy::Distant, Accuracy::Always];

    pub fn key(self) -> &'static str {
        match self {
            Accuracy::Close => "close",
            Accuracy::Normal => "normal",
            Accuracy::Distant => "distant",
            Accuracy::Always => "always",
        }
    }
    pub fn parse(s: &str) -> Option<Accuracy> {
        Accuracy::ALL.into_iter().find(|a| a.key() == s.trim().to_ascii_lowercase())
    }
    /// The name a menu or dialog shows (English; translated where it is drawn).
    pub fn label(self) -> &'static str {
        match self {
            Accuracy::Close => "Must be close",
            Accuracy::Normal => "Normal",
            Accuracy::Distant => "Can be distant",
            Accuracy::Always => "Always snap",
        }
    }
    fn factor(self) -> f32 {
        match self {
            Accuracy::Close => 0.5,
            Accuracy::Normal => 1.0,
            Accuracy::Distant => 2.0,
            Accuracy::Always => f32::INFINITY,
        }
    }
}

/// Whether the rulers, grid and guides show and snap (View › Rulers, Grid, Guides, Snapping):
/// the editor's working preferences, kept from session to session by the editor and never in
/// the document or its undo history (ADR 0076). How the grid and guides look, and where the
/// guides are, is the document's (`anicraft_doc::Aids`, `Scene::guides`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ViewAids {
    pub rulers: bool,
    pub show_grid: bool,
    pub show_guides: bool,
    /// Guides can't be dragged (commands that move or delete one refuse too).
    pub lock_guides: bool,
    pub snap_grid: bool,
    pub snap_guides: bool,
    pub snap_pixels: bool,
    pub grid_accuracy: Accuracy,
    pub guide_accuracy: Accuracy,
}

impl Default for ViewAids {
    fn default() -> Self {
        // As in Animate: guides show and snap; the rulers, grid and pixel snapping are off.
        ViewAids { rulers: false, show_grid: false, show_guides: true, lock_guides: false, snap_grid: false, snap_guides: true, snap_pixels: false, grid_accuracy: Accuracy::Normal, guide_accuracy: Accuracy::Normal }
    }
}

/// What a point snapped to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapTo {
    /// An anchor (corner or end).
    Point,
    /// A guide.
    Guide,
    /// A stroke or a fill's edge.
    Line,
    /// A grid line.
    Grid,
    /// A whole pixel.
    Pixel,
}

impl SnapTo {
    pub fn key(&self) -> &'static str {
        match self {
            SnapTo::Point => "point",
            SnapTo::Guide => "guide",
            SnapTo::Line => "line",
            SnapTo::Grid => "grid",
            SnapTo::Pixel => "pixel",
        }
    }
    /// Lower wins (the order of priority above).
    pub(crate) fn rank(self) -> u8 {
        match self {
            SnapTo::Point => 0,
            SnapTo::Guide => 1,
            SnapTo::Line => 2,
            SnapTo::Grid => 3,
            SnapTo::Pixel => 4,
        }
    }
}

/// The artwork's points and lines to snap to, as each caller gathers them.
pub(crate) trait ObjectTargets {
    /// The nearest point within `r` of `p`, and how far it is.
    fn nearest_point(&self, p: Vec2, r: f32) -> Option<(Vec2, f32)>;
    /// The nearest point on a line or fill edge within `r` of `p`.
    fn nearest_line(&self, p: Vec2, r: f32) -> Option<(Vec2, f32)>;
}

/// Where stage space is from the space tools work in: `stage = origin + scale · p`.
fn space(s: &Session) -> (Vec2, f32) {
    match s.edit_view() {
        Some(v) if v.scale.is_finite() && v.scale > 1e-4 && v.origin.is_finite() => (v.origin, v.scale),
        _ => (Vec2::ZERO, 1.0),
    }
}

impl Session {
    /// Whether anything snaps at all: a radius, and Snap to Objects, Guides, Grid or Pixels on.
    pub(crate) fn snaps(&self, r: f32) -> bool {
        r > 0.0 && (self.style.snap_objects || self.aids.snap_guides || self.aids.snap_grid || self.aids.snap_pixels)
    }

    /// The scene's guides (the main timeline's, also while a symbol is open).
    pub fn guides(&self) -> &[anicraft_doc::Guide] {
        self.doc.scenes.get(self.scene).map_or(&[], |sc| &sc.guides[..])
    }
}

/// The nearest of `lines` within `r` of `v`.
fn nearest(v: f32, r: f32, lines: impl Iterator<Item = f32>) -> Option<f32> {
    lines.map(|q| (q, (q - v).abs())).filter(|(_, d)| *d <= r).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(q, _)| q)
}

/// The nearest grid line within `r` of `v`, lines every `spacing`.
fn grid_line(v: f32, spacing: f32, r: f32) -> Option<f32> {
    if spacing.is_nan() || spacing <= 0.0 {
        return None;
    }
    let q = (v / spacing).round() * spacing;
    ((q - v).abs() <= r && q.is_finite()).then_some(q)
}

/// Where `p` (in the space tools work in) snaps within `r`, by the order of priority above;
/// `objects` are the artwork's points and lines (`None`: not looked at). `None` when nothing is
/// near or nothing snaps.
pub(crate) fn resolve(s: &Session, p: Vec2, r: f32, objects: Option<&dyn ObjectTargets>) -> Option<(Vec2, SnapTo)> {
    if !(r > 0.0 && r.is_finite() && p.is_finite()) {
        return None;
    }
    let objects = objects.filter(|_| s.style.snap_objects);
    if let Some((q, _)) = objects.and_then(|t| t.nearest_point(p, r)) {
        return Some((q, SnapTo::Point));
    }
    let (o, k) = space(s);
    let sp = o + p * k;
    let sr = r * k;
    let a = &s.aids;
    // Each coordinate: where it goes and why.
    type Axis = Option<(f32, SnapTo)>;
    let (mut x, mut y): (Axis, Axis) = (None, None);
    if a.snap_guides {
        // Guides snap within the radius at most ("always" is the grid's).
        let gr = sr * a.guide_accuracy.factor().min(2.0);
        let at = |axis: GuideAxis| s.guides().iter().filter(move |g| g.axis == axis).map(|g| g.at);
        x = nearest(sp.x, gr, at(GuideAxis::Vertical)).map(|v| (v, SnapTo::Guide));
        y = nearest(sp.y, gr, at(GuideAxis::Horizontal)).map(|v| (v, SnapTo::Guide));
    }
    if x.is_none()
        && y.is_none()
        && let Some((q, _)) = objects.and_then(|t| t.nearest_line(p, r))
    {
        return Some((q, SnapTo::Line));
    }
    if a.snap_grid {
        let g = s.doc.aids.grid;
        let gr = sr * a.grid_accuracy.factor();
        x = x.or_else(|| grid_line(sp.x, g.spacing_x, gr).map(|v| (v, SnapTo::Grid)));
        y = y.or_else(|| grid_line(sp.y, g.spacing_y, gr).map(|v| (v, SnapTo::Grid)));
    }
    if a.snap_pixels {
        x = x.or(Some((sp.x.round(), SnapTo::Pixel)));
        y = y.or(Some((sp.y.round(), SnapTo::Pixel)));
    }
    let to = [x, y].into_iter().flatten().map(|(_, t)| t).min_by_key(|t| t.rank())?;
    let q = Vec2::new(x.map_or(sp.x, |v| v.0), y.map_or(sp.y, |v| v.0));
    let back = (q - o) * (1.0 / k);
    back.is_finite().then_some((back, to))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Session;
    use anicraft_doc::{Document, Guide};
    use std::sync::Arc;

    fn session(guides: &[Guide]) -> Session {
        let mut d = Document::default();
        Arc::make_mut(&mut d.scenes[0]).guides = guides.to_vec();
        let mut s = Session::new(d);
        s.style.snap_objects = false;
        s
    }

    #[test]
    fn guides_snap_each_axis_and_their_crossing() {
        let s = session(&[Guide { axis: GuideAxis::Vertical, at: 100.0 }, Guide { axis: GuideAxis::Horizontal, at: 50.0 }]);
        assert_eq!(resolve(&s, Vec2::new(104.0, 300.0), 10.0, None), Some((Vec2::new(100.0, 300.0), SnapTo::Guide)));
        assert_eq!(resolve(&s, Vec2::new(97.0, 58.0), 10.0, None), Some((Vec2::new(100.0, 50.0), SnapTo::Guide)));
        assert_eq!(resolve(&s, Vec2::new(120.0, 300.0), 10.0, None), None, "too far");
        assert_eq!(resolve(&s, Vec2::new(104.0, 300.0), 0.0, None), None, "no radius: nothing snaps (Shift)");
    }

    #[test]
    fn the_radius_follows_the_zoom() {
        // The shell asks for 10 screen pixels: 10 / zoom stage pixels.
        let s = session(&[Guide { axis: GuideAxis::Vertical, at: 100.0 }]);
        for (zoom, off, snaps) in [(1.0, 8.0, true), (1.0, 12.0, false), (4.0, 2.0, true), (4.0, 3.0, false), (0.25, 39.0, true), (0.25, 41.0, false)] {
            let r = 10.0 / zoom;
            assert_eq!(resolve(&s, Vec2::new(100.0 + off, 0.0), r, None).is_some(), snaps, "zoom {zoom}, {off} px off");
        }
    }

    #[test]
    fn grid_pixels_and_accuracy() {
        let mut s = session(&[]);
        s.aids.snap_grid = true;
        assert_eq!(resolve(&s, Vec2::new(43.0, 18.0), 10.0, None), Some((Vec2::new(40.0, 20.0), SnapTo::Grid)));
        s.aids.grid_accuracy = Accuracy::Close;
        assert_eq!(resolve(&s, Vec2::new(46.0, 13.0), 10.0, None), None, "Must be close: within 5");
        s.aids.grid_accuracy = Accuracy::Always;
        assert_eq!(resolve(&s, Vec2::new(49.0, 9.0), 1.0, None), Some((Vec2::new(40.0, 0.0), SnapTo::Grid)));
        s.aids.snap_grid = false;
        s.aids.snap_pixels = true;
        assert_eq!(resolve(&s, Vec2::new(4.4, 7.6), 10.0, None), Some((Vec2::new(4.0, 8.0), SnapTo::Pixel)));
        // A guide on x wins over the grid there; y still goes to the grid, then pixels.
        let mut s = session(&[Guide { axis: GuideAxis::Vertical, at: 33.0 }]);
        s.aids.snap_grid = true;
        assert_eq!(resolve(&s, Vec2::new(36.0, 22.0), 10.0, None), Some((Vec2::new(33.0, 20.0), SnapTo::Guide)));
    }
}
