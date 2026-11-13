//! Drawing aids saved with the document (ADR 0076): each scene's guides, and how the grid and
//! the guides look. Whether they show and snap is the editor's (`anicraft_engine::ViewAids`):
//! a working preference, like Snap to Objects, not part of the artwork. None of this is ever
//! rendered into a picture or an export; the editor's stage draws it.

use crate::Rgba;
use serde::{Deserialize, Serialize};

/// Which way a guide runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GuideAxis {
    /// A horizontal line at `y = at` (dragged from the top ruler).
    Horizontal,
    /// A vertical line at `x = at` (dragged from the left ruler).
    Vertical,
}

impl GuideAxis {
    pub fn key(self) -> &'static str {
        match self {
            GuideAxis::Horizontal => "horizontal",
            GuideAxis::Vertical => "vertical",
        }
    }
    /// `horizontal`/`h` or `vertical`/`v`.
    pub fn parse(s: &str) -> Option<GuideAxis> {
        match s.trim().to_ascii_lowercase().as_str() {
            "horizontal" | "h" => Some(GuideAxis::Horizontal),
            "vertical" | "v" => Some(GuideAxis::Vertical),
            _ => None,
        }
    }
}

/// A guide: a line across the whole stage and pasteboard, in stage pixels from the stage's
/// top-left corner.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Guide {
    pub axis: GuideAxis,
    pub at: f32,
}

/// Most guides one scene keeps.
pub const MAX_GUIDES: usize = 1_000;
/// Farthest a guide may be from the stage's corner, in stage pixels (well past any pasteboard).
pub const GUIDE_REACH: f32 = 100_000.0;
/// The grid's spacing range, in stage pixels.
pub const GRID_MIN: f32 = 1.0;
pub const GRID_MAX: f32 = 4_096.0;

impl Guide {
    /// A guide with its position kept in range; `None` for a position that isn't a number.
    pub fn new(axis: GuideAxis, at: f32) -> Option<Guide> {
        at.is_finite().then(|| Guide { axis, at: at.clamp(-GUIDE_REACH, GUIDE_REACH) })
    }
}

/// How the grid looks (View › Grid › Edit Grid).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Grid {
    /// Between vertical lines, in stage pixels.
    pub spacing_x: f32,
    /// Between horizontal lines, in stage pixels.
    pub spacing_y: f32,
    pub color: Rgba,
    /// Drawn over the artwork; off (the default, as in Animate) it shows under it, on the stage.
    pub over_objects: bool,
}

impl Default for Grid {
    fn default() -> Self {
        Grid { spacing_x: 20.0, spacing_y: 20.0, color: Rgba::hex(0x949494), over_objects: false }
    }
}

impl Grid {
    /// Spacing clamped to [`GRID_MIN`]..=[`GRID_MAX`] (non-numbers back to the default).
    pub fn sanitized(self) -> Grid {
        let sp = |v: f32| if v.is_finite() { v.clamp(GRID_MIN, GRID_MAX) } else { 20.0 };
        Grid { spacing_x: sp(self.spacing_x), spacing_y: sp(self.spacing_y), color: self.color.sanitized(), over_objects: self.over_objects }
    }
}

/// The grid and the guides' looks, saved with the document.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Aids {
    pub grid: Grid,
    pub guide_color: Rgba,
}

impl Default for Aids {
    fn default() -> Self {
        Aids { grid: Grid::default(), guide_color: Rgba::hex(0x3ec8f0) }
    }
}

impl Aids {
    pub fn is_default(&self) -> bool {
        *self == Aids::default()
    }
    pub fn sanitized(self) -> Aids {
        Aids { grid: self.grid.sanitized(), guide_color: self.guide_color.sanitized() }
    }
}

/// Guides from a file kept to numbers in range, at most [`MAX_GUIDES`].
pub fn sanitize_guides(guides: &mut Vec<Guide>) {
    guides.retain(|g| g.at.is_finite());
    guides.truncate(MAX_GUIDES);
    for g in guides.iter_mut() {
        g.at = g.at.clamp(-GUIDE_REACH, GUIDE_REACH);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hostile_values_are_kept_in_range() {
        let mut g = vec![Guide { axis: GuideAxis::Vertical, at: f32::NAN }, Guide { axis: GuideAxis::Horizontal, at: 1e30 }];
        g.extend(std::iter::repeat_n(Guide { axis: GuideAxis::Vertical, at: 3.0 }, MAX_GUIDES + 5));
        sanitize_guides(&mut g);
        assert_eq!(g.len(), MAX_GUIDES);
        assert_eq!(g[0], Guide { axis: GuideAxis::Horizontal, at: GUIDE_REACH });
        let grid = Grid { spacing_x: 0.0, spacing_y: f32::INFINITY, ..Grid::default() }.sanitized();
        assert_eq!((grid.spacing_x, grid.spacing_y), (GRID_MIN, 20.0));
        assert_eq!(Guide::new(GuideAxis::Vertical, f32::NAN), None);
        assert_eq!(GuideAxis::parse(" V "), Some(GuideAxis::Vertical));
        assert_eq!(GuideAxis::parse("diagonal"), None);
    }
}
