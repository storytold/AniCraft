//! The Brush tool's command (ADR 0033): a stroke of the brush paints a filled shape, the area its
//! tip sweeps, in the fill colour. Off Object Drawing it is a raw shape that merges with the
//! drawing (ADR 0031), placed by the paint mode.

use crate::cmds_draw::{MAX_EXTENT, object_param};
use crate::merge::{self, Order};
use crate::params::*;
use crate::{CommandSpec, EngineError, Result, Session};
use anicraft_doc::{ColorEffect, Element, ElementId, ElementKind, Paint, Shape};
use anicraft_geom::brush::{MAX_SAMPLES, Sample, Tip, TipShape, outline};
use anicraft_geom::{Affine, FillRule, PathCmd, Vec2};
use serde_json::{Value, json};
use std::sync::Arc;

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "draw.brush",
        label: "Brush Stroke",
        menu: "",
        shortcut: None,
        params: "{points: [[x, y] or [x, y, pressure 0..1 (clamped)], ..] (one point paints a dab), size? (stage units, default 10), shape?: round|square, angle? (degrees), roundness? (5..100 %, default 100), smoothing? (0..100, default 50), mode?: normal|fills|behind (Paint Normal covers fills and lines, Paint Fills leaves lines, Paint Behind paints only empty areas), color? (default: the fill colour), unit? (stage units per screen point, default 1: smoothing and accuracy follow it), object?}",
        mutates: true,
        run: draw_brush,
    }]
}

/// Brush sizes, in stage units.
pub const SIZE_RANGE: std::ops::RangeInclusive<f32> = 0.1..=2000.0;

/// A paint mode (Animate's names).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PaintMode {
    #[default]
    Normal,
    Fills,
    Behind,
}

impl PaintMode {
    pub const ALL: [PaintMode; 3] = [PaintMode::Normal, PaintMode::Fills, PaintMode::Behind];

    pub fn key(self) -> &'static str {
        match self {
            PaintMode::Normal => "normal",
            PaintMode::Fills => "fills",
            PaintMode::Behind => "behind",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            PaintMode::Normal => "Paint Normal",
            PaintMode::Fills => "Paint Fills",
            PaintMode::Behind => "Paint Behind",
        }
    }
    pub fn hint(self) -> &'static str {
        match self {
            PaintMode::Normal => "Paints over lines and fills on the layer.",
            PaintMode::Fills => "Paints over fills and empty areas, leaving lines alone.",
            PaintMode::Behind => "Paints only empty areas, behind the lines and fills on the layer.",
        }
    }
    fn order(self) -> Order {
        match self {
            PaintMode::Normal => Order::Top,
            PaintMode::Fills => Order::UnderLines,
            PaintMode::Behind => Order::Behind,
        }
    }
    pub fn from_key(k: &str) -> Option<PaintMode> {
        PaintMode::ALL.into_iter().find(|m| m.key() == k)
    }
}

pub(crate) fn samples(p: &Value) -> Result<Vec<Sample>> {
    let bad = || EngineError::Param("param 'points' must be a list of [x, y] or [x, y, pressure]".into());
    let list = p.get("points").and_then(Value::as_array).ok_or_else(bad)?;
    if list.is_empty() {
        return Err(EngineError::Param("param 'points' needs at least one point".into()));
    }
    if list.len() > MAX_SAMPLES {
        return Err(EngineError::Param(format!("param 'points' has more than {MAX_SAMPLES} points")));
    }
    list.iter()
        .map(|v| {
            let xyp = v.as_array().filter(|a| (2..=3).contains(&a.len())).ok_or_else(bad)?;
            let n = |i: usize| xyp.get(i).and_then(Value::as_f64).map(|f| f as f32).filter(|f| f.is_finite());
            let at = Vec2::new(n(0).ok_or_else(bad)?, n(1).ok_or_else(bad)?);
            // Pressure is a measurement: tablets and arithmetic stray a little past 0..1, so it
            // is clamped (only a non-number is an error).
            let pressure = match xyp.get(2) {
                None => 1.0,
                Some(_) => n(2).ok_or_else(|| EngineError::Param("a point's pressure must be a number from 0 to 1".into()))?.clamp(0.0, 1.0),
            };
            Ok(Sample { at, pressure })
        })
        .collect()
}

pub(crate) fn ranged(p: &Value, key: &str, range: std::ops::RangeInclusive<f32>, default: f32) -> Result<f32> {
    match opt_f32(p, key)? {
        None => Ok(default),
        Some(v) if range.contains(&v) => Ok(v),
        Some(_) => Err(EngineError::Param(format!("param '{key}' must be between {} and {}", range.start(), range.end()))),
    }
}

/// The stroke's samples, refused when they reach too far.
pub(crate) fn bounded_samples(p: &Value) -> Result<Vec<Sample>> {
    let pts = samples(p)?;
    let mut bounds = anicraft_geom::Rect::EMPTY;
    pts.iter().for_each(|x| bounds.include(x.at));
    if bounds.width() > MAX_EXTENT || bounds.height() > MAX_EXTENT || bounds.min.x.abs().max(bounds.max.x.abs()).max(bounds.min.y.abs()).max(bounds.max.y.abs()) > MAX_EXTENT {
        return Err(EngineError::Param("the stroke is too large".into()));
    }
    Ok(pts)
}

/// The tip from `size`, `shape`, `angle` and `roundness` (shared with the Eraser, ADR 0034).
pub(crate) fn tip(p: &Value) -> Result<Tip> {
    let size = ranged(p, "size", SIZE_RANGE, 10.0)?;
    let shape = match opt_str(p, "shape")?.unwrap_or("round") {
        "round" => TipShape::Round,
        "square" => TipShape::Square,
        other => return Err(EngineError::Param(format!("unknown tip shape '{other}' (round or square)"))),
    };
    let angle = opt_f32(p, "angle")?.unwrap_or(0.0);
    if !angle.is_finite() || angle.abs() > 1e6 {
        return Err(EngineError::Param("param 'angle' must be a finite number of degrees".into()));
    }
    let roundness = ranged(p, "roundness", 5.0..=100.0, 100.0)?;
    Ok(Tip { shape, size, angle, roundness: roundness / 100.0 })
}

fn draw_brush(s: &mut Session, p: &Value) -> Result<Value> {
    let pts = bounded_samples(p)?;
    let tip = tip(p)?;
    let size = tip.size;
    let smoothing = ranged(p, "smoothing", 0.0..=100.0, 50.0)?;
    let unit = ranged(p, "unit", 0.001..=1000.0, 1.0)?;
    let mode_key = opt_str(p, "mode")?.unwrap_or("normal");
    let mode = PaintMode::from_key(mode_key).ok_or_else(|| EngineError::Param(format!("unknown paint mode '{mode_key}' (normal, fills or behind)")))?;
    let color = opt_color(p, "color")?.unwrap_or(s.style.fill);
    let object = object_param(s, p)?;

    // Smoothing and accuracy are screen distances, so a stroke looks the same at any zoom: at 50
    // the centre line may wander 3.3 screen points from the hand, at 0 a third of a point.
    // The outline is fitted within a third of a screen point, or a hundredth of the size for
    // small brushes (so a small dab is still round).
    let path = outline(&pts, &tip, unit * (0.3 + smoothing * 0.06), (unit * 0.35).min(size * 0.01));
    if !path.cmds.iter().any(|c| matches!(c, PathCmd::CubicTo(..) | PathCmd::LineTo(..))) {
        return Err(EngineError::Param("the stroke is too small to paint".into()));
    }
    let c = path.bounds().center();
    let id = ElementId(s.doc.alloc_id());
    let el = Element {
        id,
        name: String::new(),
        transform: Affine::translate(c.x, c.y),
        color: ColorEffect::default(),
        kind: ElementKind::Shape(Shape { path: Arc::new(path.transformed(&Affine::translate(-c.x, -c.y))), fill: Some(Paint::Solid(color)), stroke: None, fill_rule: FillRule::EvenOdd, merge: !object }),
    };
    if s.editable_keyframe()?.elements.len() >= 100_000 {
        return Err(EngineError::State("too many elements on this keyframe".into()));
    }
    if object {
        let kf = s.editable_keyframe()?;
        match mode {
            // Behind everything on the layer.
            PaintMode::Behind => kf.elements.insert(0, Arc::new(el)),
            PaintMode::Normal | PaintMode::Fills => kf.elements.push(Arc::new(el)),
        }
        s.selection = vec![id];
        return Ok(json!({"element": id.0, "elements": [id.0], "mode": mode.key(), "merged": false}));
    }
    s.selection = merge::merge_in_order(s, vec![el], mode.order())?;
    let ids: Vec<u64> = s.selection.iter().map(|e| e.0).collect();
    Ok(json!({"element": ids.first(), "elements": ids, "mode": mode.key(), "merged": true}))
}
