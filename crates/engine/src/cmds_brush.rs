//! The Brush tool's command (ADR 0033): a stroke of the brush paints a filled shape, the area its
//! tip sweeps, in the fill colour. Off Object Drawing it is a raw shape that merges with the
//! drawing (ADR 0031), placed by the paint mode.

use crate::brush_library::{BUILTIN, BrushLibrary, BrushPreset, MAX_FILE_BYTES, MAX_USER_PRESETS, builtin, builtin_hint, clean_name};
use crate::cmds_draw::{MAX_EXTENT, add_shape, object_param};
use crate::merge::{self, Order};
use crate::params::*;
use crate::{CommandSpec, EngineError, Result, Session};
use anicraft_doc::{ColorEffect, Element, ElementId, ElementKind, MAX_WIDTH_POINTS, Paint, Rgba, Shape, Stroke, WidthPoint, WidthProfile};
use anicraft_geom::planar;
use anicraft_geom::brush::{MAX_SAMPLES, Sample, Tilt, Tip, TipShape, centre_stroke, outline_shaped};
use anicraft_geom::{Affine, FillRule, Path, PathCmd, Vec2};
use serde_json::{Value, json};
use std::sync::Arc;

pub fn specs() -> Vec<CommandSpec> {
    let c = |id, label, params, mutates, run| CommandSpec { id, label, menu: "", shortcut: None, params, mutates, run };
    vec![
        c(
            "draw.brush",
            "Brush Stroke",
            "{points: [[x, y], [x, y, pressure 0..1 (clamped)] or [x, y, pressure, altitude, azimuth] (the pen's tilt in degrees: altitude 90 upright to 0 flat, azimuth the way it leans, clockwise from +x)], ..] (one point paints a dab), preset? (a brush from brush.preset_list, by key or name: its settings are the defaults for the rest), size? (stage units, default 10), shape?: round|square, angle? (degrees), roundness? (5..100 %, default 100), smoothing? (0..100, default 50), curve?: linear|soft|firm|[[pressure, output], ..] (how pressure maps, both 0..1), pressure_size? (default true: pressure varies the size from 10 to 100 %), pressure_opacity? (default false: the stroke's average pressure sets its opacity), taper_start?, taper_end? (0..50 % of the stroke's length), spacing? (0 continuous, or 5..1000 % of the size between dabs), jitter? (0..50 % of the size: a ragged edge), texture? (0..100 %: a paper grain of holes through the paint), scatter? (0..200 % of the size: dabs thrown either side of the line, varied in size), tilt_angle? (the pen's tilt turns the tip: its angle plus the azimuth), tilt_width? (the pen's tilt widens the tip, up to twice as wide flat; also the Paint Brush's width), as_stroke? (default false; true: the Paint Brush, a variable-width stroke in the stroke colour, editable with the Width tool), mode?: normal|fills|behind|selection|inside (Paint Normal covers fills and lines, Paint Fills leaves lines, Paint Behind paints only empty areas, Paint Selection only inside the selected fills, Paint Inside only inside the fill the stroke starts on or the empty area closed off around it; ADR 0080), color? (default: the fill colour, or the stroke colour as a stroke), unit? (stage units per screen point, default 1: smoothing and accuracy follow it), object?}",
            true,
            draw_brush as crate::RunFn,
        ),
        c("brush.preset_list", "Brush Presets", "{} the built-in brushes and yours, with their settings", false, preset_list),
        c("brush.preset_save", "Save Brush", "{name, from? (a preset to start from; default: the plain brush), size?, shape?, angle?, roundness?, smoothing?, curve?, pressure_size?, pressure_opacity?, taper_start?, taper_end?, spacing?, jitter?, texture?, scatter?, tilt_angle?, tilt_width? (as draw.brush)} saves a brush of your own (replacing yours of that name; kept in your preferences)", false, preset_save),
        c("brush.preset_rename", "Rename Brush", "{name, to}", false, preset_rename),
        c("brush.preset_delete", "Delete Brush", "{name} deletes a brush of your own (asks first: it can't be undone)", false, preset_delete),
        c("brush.preset_export", "Export Brushes", "{path, names? (default: all yours)} writes your brushes to a brush set file (JSON)", false, preset_export),
        c("brush.preset_import", "Import Brushes", "{path} adds the brushes in a brush set file to yours (a name you have already gets a number)", false, preset_import),
    ]
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
    /// Only inside the selected fills (ADR 0080).
    Selection,
    /// Only inside the fill the stroke starts on (ADR 0080).
    Inside,
}

impl PaintMode {
    pub const ALL: [PaintMode; 5] = [PaintMode::Normal, PaintMode::Fills, PaintMode::Behind, PaintMode::Selection, PaintMode::Inside];

    pub fn key(self) -> &'static str {
        match self {
            PaintMode::Normal => "normal",
            PaintMode::Fills => "fills",
            PaintMode::Behind => "behind",
            PaintMode::Selection => "selection",
            PaintMode::Inside => "inside",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            PaintMode::Normal => "Paint Normal",
            PaintMode::Fills => "Paint Fills",
            PaintMode::Behind => "Paint Behind",
            PaintMode::Selection => "Paint Selection",
            PaintMode::Inside => "Paint Inside",
        }
    }
    pub fn hint(self) -> &'static str {
        match self {
            PaintMode::Normal => "Paints over lines and fills on the layer.",
            PaintMode::Fills => "Paints over fills and empty areas, leaving lines alone.",
            PaintMode::Behind => "Paints only empty areas, behind the lines and fills on the layer.",
            PaintMode::Selection => "Paints only inside the selected fills, leaving lines alone.",
            PaintMode::Inside => "Paints only inside the fill where the stroke starts, leaving lines alone. Started on empty stage, it paints only the empty area there.",
        }
    }
    fn order(self) -> Order {
        match self {
            PaintMode::Normal => Order::Top,
            // Clipped paint never covers a line either (ADR 0080).
            PaintMode::Fills | PaintMode::Selection | PaintMode::Inside => Order::UnderLines,
            PaintMode::Behind => Order::Behind,
        }
    }
    pub fn from_key(k: &str) -> Option<PaintMode> {
        PaintMode::ALL.into_iter().find(|m| m.key() == k)
    }
}

/// Where Paint Selection and Paint Inside may paint (ADR 0080).
enum Region {
    /// Inside this outline (keyframe space, even-odd).
    Clip(Path),
    /// Only empty areas: Paint Inside started on open, empty stage (as Paint Behind).
    Empty,
    /// Nowhere, and why.
    Nowhere(&'static str),
}

const TOO_MUCH: &str = "there is too much artwork there to paint inside at once; paint a smaller area";

/// The region a clipped paint mode paints in: the union of the selected shapes' fills on the
/// active layer (Paint Selection), or the topmost fill under `start` there, else the empty area
/// the lines and fills close off around it (Paint Inside). Merged paint lies under drawing
/// objects, so off Object Drawing only raw fills count (an object is edited inside it).
fn paint_region(s: &mut Session, mode: PaintMode, start: Vec2, object: bool) -> Result<Region> {
    let els = s.editable_keyframe()?.elements.clone();
    let filled = |e: &Element| match &e.kind {
        ElementKind::Shape(sh) if sh.fill.is_some() && (object || sh.merge) => Some((sh.path.transformed(&e.transform), sh.fill_rule)),
        _ => None,
    };
    let too_much = || EngineError::State(TOO_MUCH.into());
    if mode == PaintMode::Selection {
        let here = s.layer;
        let picked: Vec<(Path, FillRule)> = els.iter().filter(|e| s.selection.contains(here, e.id)).filter_map(|e| filled(e)).collect();
        if picked.is_empty() {
            return Ok(Region::Nowhere(if s.selection.on(here).is_empty() { "select the fills to paint inside first" } else { "the selection has no fills to paint inside" }));
        }
        let refs: Vec<(&Path, FillRule)> = picked.iter().map(|(p, r)| (p, *r)).collect();
        return Ok(Region::Clip(planar::union(&refs).ok_or_else(too_much)?));
    }
    let on = els.iter().rev().find_map(|e| filled(e).filter(|(p, r)| p.contains(start, &Affine::IDENTITY, *r)));
    if let Some((path, rule)) = on {
        // The fill's own rule, as even-odd loops.
        return Ok(Region::Clip(planar::union(&[(&path, rule)]).ok_or_else(too_much)?));
    }
    Ok(match merge::enclosed_area(&els, start) {
        Some(area) => Region::Clip(area),
        None => Region::Empty,
    })
}

pub(crate) fn samples(p: &Value) -> Result<Vec<Sample>> {
    let bad = || EngineError::Param("param 'points' must be a list of [x, y], [x, y, pressure] or [x, y, pressure, altitude, azimuth]".into());
    let list = p.get("points").and_then(Value::as_array).ok_or_else(bad)?;
    if list.is_empty() {
        return Err(EngineError::Param("param 'points' needs at least one point".into()));
    }
    if list.len() > MAX_SAMPLES {
        return Err(EngineError::Param(format!("param 'points' has more than {MAX_SAMPLES} points")));
    }
    list.iter()
        .map(|v| {
            let xyp = v.as_array().filter(|a| matches!(a.len(), 2 | 3 | 5)).ok_or_else(bad)?;
            let n = |i: usize| xyp.get(i).and_then(Value::as_f64).map(|f| f as f32).filter(|f| f.is_finite());
            let at = Vec2::new(n(0).ok_or_else(bad)?, n(1).ok_or_else(bad)?);
            // Pressure is a measurement: tablets and arithmetic stray a little past 0..1, so it
            // is clamped (only a non-number is an error).
            let pressure = match xyp.get(2) {
                None => 1.0,
                Some(_) => n(2).ok_or_else(|| EngineError::Param("a point's pressure must be a number from 0 to 1".into()))?.clamp(0.0, 1.0),
            };
            // Tilt (ADR 0093): the pen's altitude (90 upright .. 0 flat) and azimuth (the way
            // it leans on the stage, clockwise from +x), both in degrees, as Pointer Events
            // report them. A measurement too: the altitude is clamped, the azimuth wraps.
            let tilt = match (xyp.get(3), xyp.get(4)) {
                (Some(_), Some(_)) => {
                    let bad_tilt = || EngineError::Param("a point's altitude and azimuth must be numbers of degrees".into());
                    let altitude = n(3).filter(|a| a.abs() <= 1e6).ok_or_else(bad_tilt)?.clamp(0.0, 90.0);
                    let azimuth = n(4).filter(|a| a.abs() <= 1e6).ok_or_else(bad_tilt)?;
                    Some(Tilt { altitude, azimuth })
                }
                _ => None,
            };
            Ok(Sample { at, pressure, tilt })
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

/// The brush a command paints with: its `preset` (or the plain brush) with the params' own
/// settings on top.
pub fn brush_param(s: &Session, p: &Value) -> Result<BrushPreset> {
    let base = match opt_str(p, "preset")? {
        Some(name) => s.brushes.find(name).ok_or_else(|| EngineError::Param(format!("no brush named '{name}' (brush.preset_list lists them)")))?,
        None => BrushPreset::default(),
    };
    base.with_params(p)
}

/// The stroke's opacity factor when pressure drives it: the average mapped pressure.
pub fn pressure_opacity(b: &BrushPreset, pts: &[Sample]) -> f32 {
    if !b.pressure_opacity || pts.is_empty() {
        return 1.0;
    }
    (pts.iter().map(|x| b.curve.map(x.pressure)).sum::<f32>() / pts.len() as f32).clamp(0.05, 1.0)
}

fn draw_brush(s: &mut Session, p: &Value) -> Result<Value> {
    let pts = bounded_samples(p)?;
    let b = brush_param(s, p)?;
    let tip = b.tip();
    let size = tip.size;
    let as_stroke = opt_bool(p, "as_stroke")?.unwrap_or(false);
    let unit = ranged(p, "unit", 0.001..=1000.0, 1.0)?;
    let mode_key = opt_str(p, "mode")?.unwrap_or("normal");
    let mode = PaintMode::from_key(mode_key).ok_or_else(|| EngineError::Param(format!("unknown paint mode '{mode_key}' (normal, fills, behind, selection or inside)")))?;
    let color = opt_color(p, "color")?.unwrap_or(if as_stroke { s.style.stroke } else { s.style.fill });
    let color = Rgba { a: color.a * pressure_opacity(&b, &pts), ..color };
    let object = object_param(s, p)?;
    // Pressure through the curve (ADR 0068) before it sizes the tip.
    let pts: Vec<Sample> = pts.iter().map(|x| Sample { pressure: b.size_pressure(x.pressure), ..*x }).collect();

    // Smoothing and accuracy are screen distances, so a stroke looks the same at any zoom: at 50
    // the centre line may wander 3.3 screen points from the hand, at 0 a third of a point.
    // The outline is fitted within a third of a screen point, or a hundredth of the size for
    // small brushes (so a small dab is still round).
    let (smoothing, tol) = (unit * (0.3 + b.smoothing * 0.06), (unit * 0.35).min(size * 0.01));
    if as_stroke {
        return paint_stroke(s, &pts, &b, smoothing, tol, color, object);
    }
    let start = pts.first().map_or(Vec2::ZERO, |x| x.at);
    let mut path = outline_shaped(&pts, &tip, &b.shaping(), smoothing, tol);
    if !path.cmds.iter().any(|c| matches!(c, PathCmd::CubicTo(..) | PathCmd::LineTo(..))) {
        return Err(EngineError::Param("the stroke is too small to paint".into()));
    }
    // Paint Selection and Paint Inside (ADR 0080): the outline clipped to the region, then
    // merged as usual. Nothing changes when nothing is painted (no keyframe made, no step).
    let mut order = mode.order();
    // Paint Selection keeps the selection: the region, to select again after the merge.
    let mut kept_region: Option<Path> = None;
    if matches!(mode, PaintMode::Selection | PaintMode::Inside) {
        let before = s.doc.clone();
        let region = paint_region(s, mode, start, object);
        let clipped = match region {
            Ok(Region::Clip(r)) => {
                let c = planar::intersect(&path, FillRule::EvenOdd, &r).ok_or_else(|| EngineError::State(TOO_MUCH.into()));
                kept_region = (mode == PaintMode::Selection).then_some(r);
                Some(c)
            }
            Ok(Region::Empty) => None,
            Ok(Region::Nowhere(why)) => {
                s.doc = before;
                return Ok(json!({"painted": false, "mode": mode.key(), "elements": [], "reason": why}));
            }
            Err(e) => Some(Err(e)),
        };
        match clipped {
            None => order = Order::Behind,
            Some(Err(e)) => {
                s.doc = before;
                return Err(e);
            }
            Some(Ok(p)) if !p.cmds.iter().any(|c| matches!(c, PathCmd::CubicTo(..) | PathCmd::LineTo(..))) => {
                s.doc = before;
                let why = if mode == PaintMode::Selection { "the stroke doesn't reach the selected fills" } else { "nothing to paint there" };
                return Ok(json!({"painted": false, "mode": mode.key(), "elements": [], "reason": why}));
            }
            Some(Ok(p)) => path = p,
        }
    }
    let c = path.bounds().center();
    let local = path.transformed(&Affine::translate(-c.x, -c.y));
    // A gradient or bitmap in the fill swatch paints too, locked or fitted (ADR 0074), unless a
    // colour was given or pressure sets the opacity.
    let fill = match &s.style.fill_paint {
        Some(_) if p.get("color").is_none_or(Value::is_null) && color.a >= 1.0 => s.style.fill_paint_for(&Affine::translate(-c.x, -c.y), &local.bounds()),
        _ => Paint::Solid(color),
    };
    let id = ElementId(s.doc.alloc_id());
    let el = Element {
        id,
        name: String::new(),
        transform: Affine::translate(c.x, c.y),
        color: ColorEffect::default(),
        kind: ElementKind::Shape(Shape { path: Arc::new(local), fill: Some(fill), stroke: None, fill_rule: FillRule::EvenOdd, merge: !object, warp: None }),
    };
    if s.editable_keyframe()?.elements.len() >= 100_000 {
        return Err(EngineError::State("too many elements on this keyframe".into()));
    }
    if object {
        let kf = s.editable_keyframe()?;
        match order {
            // Behind everything on the layer.
            Order::Behind => kf.elements.insert(0, Arc::new(el)),
            Order::Top | Order::UnderLines => kf.elements.push(Arc::new(el)),
        }
        // Paint Selection: the selected objects stay selected, for the next stroke.
        if kept_region.is_none() {
            s.select_only([id]);
        }
        return Ok(json!({"element": id.0, "elements": [id.0], "mode": mode.key(), "merged": false}));
    }
    let merged = merge::merge_in_order(s, vec![el], order)?;
    let ids: Vec<u64> = merged.iter().map(|e| e.0).collect();
    match &kept_region {
        // Paint Selection: what lies in the region (the selected fills' pieces and the paint)
        // stays selected, for the next stroke.
        Some(region) => {
            let inside = pieces_in(s, region);
            s.select_only(inside);
        }
        None => s.select_only(merged),
    }
    Ok(json!({"element": ids.first(), "elements": ids, "mode": mode.key(), "merged": true}))
}

/// The raw fills on the active keyframe that cover part of `region` (keyframe space, even-odd).
fn pieces_in(s: &mut Session, region: &Path) -> Vec<ElementId> {
    let Ok(kf) = s.editable_keyframe() else { return Vec::new() };
    let bounds = region.bounds();
    kf.elements
        .iter()
        .filter(|e| matches!(&e.kind, ElementKind::Shape(sh) if sh.merge && sh.fill.is_some() && !sh.path.bounds().transformed(&e.transform).intersect(&bounds).is_empty()))
        .filter(|e| match &e.kind {
            ElementKind::Shape(sh) => planar::intersect(&sh.path.transformed(&e.transform), sh.fill_rule, region).is_some_and(|p| !p.is_empty()),
            _ => false,
        })
        .map(|e| e.id)
        .collect()
}

/// Width points a Paint Brush stroke's profile is simplified from (evenly along its length).
const PROFILE_SAMPLES: usize = 256;
/// How closely the simplified profile follows the brush's widths (a factor of the size).
const PROFILE_TOLERANCE: f32 = 0.015;

/// The Paint Brush (ADR 0068): the smoothed centre line as a stroke of the brush's size in the
/// stroke colour, its width profile following the pressure and tapers, so the Width tool edits
/// it afterwards. A stroke of one width is a plain line and merges like the Pencil's.
fn paint_stroke(s: &mut Session, pts: &[Sample], b: &BrushPreset, smoothing: f32, tol: f32, color: Rgba, object: bool) -> Result<Value> {
    let (path, factors) = centre_stroke(pts, &b.shaping(), smoothing, tol * 0.25, PROFILE_SAMPLES);
    if !path.cmds.iter().any(|c| matches!(c, PathCmd::CubicTo(..) | PathCmd::LineTo(..))) {
        return Err(EngineError::Param("the stroke is too short to draw a line".into()));
    }
    let kept = simplify_profile(&factors, PROFILE_TOLERANCE, MAX_WIDTH_POINTS);
    let profile = WidthProfile::new(kept.iter().map(|(t, f)| WidthPoint::new(*t, *f, *f)).collect());
    let stroke = Stroke::new(color, b.size).with_profile(profile);
    let c = path.bounds().center();
    let mut v = add_shape(s, "paint_brush", path.transformed(&Affine::translate(-c.x, -c.y)), c, None, Some(stroke), object)?;
    v["width_points"] = json!(kept.len());
    Ok(v)
}

/// The fewest of `pts` (`(t, factor)`, sorted) that stay within `tol` of the rest by straight
/// lines between them (Ramer–Douglas–Peucker), widening the tolerance until at most `max` are
/// kept. The ends always stay.
pub fn simplify_profile(pts: &[(f32, f32)], tol: f32, max: usize) -> Vec<(f32, f32)> {
    let n = pts.len();
    if n <= 2 {
        return pts.to_vec();
    }
    let mut tol = tol.max(1e-4);
    for _ in 0..12 {
        let mut keep = vec![false; n];
        if let Some(k) = keep.first_mut() {
            *k = true;
        }
        if let Some(k) = keep.last_mut() {
            *k = true;
        }
        let mut stack = vec![(0usize, n - 1)];
        while let Some((a, z)) = stack.pop() {
            let (Some(pa), Some(pz)) = (pts.get(a), pts.get(z)) else { continue };
            let mut worst = (0.0f32, a);
            for i in a + 1..z {
                let Some(q) = pts.get(i) else { continue };
                let t = if pz.0 - pa.0 > 1e-9 { (q.0 - pa.0) / (pz.0 - pa.0) } else { 0.0 };
                let d = (q.1 - (pa.1 + (pz.1 - pa.1) * t)).abs();
                if d > worst.0 {
                    worst = (d, i);
                }
            }
            if worst.0 > tol {
                if let Some(k) = keep.get_mut(worst.1) {
                    *k = true;
                }
                stack.push((a, worst.1));
                stack.push((worst.1, z));
            }
        }
        let out: Vec<(f32, f32)> = pts.iter().zip(&keep).filter(|(_, k)| **k).map(|(p, _)| *p).collect();
        if out.len() <= max.max(2) {
            return out;
        }
        tol *= 1.6;
    }
    // Still too many (a wildly varying pressure): evenly spaced points.
    let m = max.max(2);
    (0..m).filter_map(|i| pts.get(i * (n - 1) / (m - 1))).copied().collect()
}

// ----- the brush library (ADR 0068) ---------------------------------------------------------

fn preset_json(p: &BrushPreset, key: Option<&str>) -> Value {
    let mut v = p.params();
    v["name"] = json!(p.name);
    v["builtin"] = json!(key.is_some());
    if let Some(k) = key {
        v["key"] = json!(k);
        v["hint"] = json!(builtin_hint(k));
    }
    v
}

fn preset_list(s: &mut Session, _: &Value) -> Result<Value> {
    let mut list: Vec<Value> = BUILTIN.iter().filter_map(|(k, _)| builtin(k).map(|p| preset_json(&p, Some(k)))).collect();
    list.extend(s.brushes.user().iter().map(|p| preset_json(p, None)));
    Ok(json!({"presets": list, "yours": s.brushes.user().len()}))
}

fn preset_save(s: &mut Session, p: &Value) -> Result<Value> {
    let name = str_req(p, "name")?;
    let base = match opt_str(p, "from")? {
        Some(from) => s.brushes.find(from).ok_or_else(|| EngineError::Param(format!("no brush named '{from}'")))?,
        None => BrushPreset::default(),
    };
    let b = BrushPreset { name: name.to_string(), ..base.with_params(p)? };
    let replaced = s.brushes.save(b)?;
    let saved = clean_name(name).and_then(|n| s.brushes.position(&n)).and_then(|i| s.brushes.user().get(i)).map(|x| x.name.clone());
    Ok(json!({"name": saved, "replaced": replaced, "yours": s.brushes.user().len()}))
}

fn preset_rename(s: &mut Session, p: &Value) -> Result<Value> {
    let to = s.brushes.rename(str_req(p, "name")?, str_req(p, "to")?)?;
    Ok(json!({"name": to}))
}

fn preset_delete(s: &mut Session, p: &Value) -> Result<Value> {
    let gone = s.brushes.delete(str_req(p, "name")?)?;
    Ok(json!({"deleted": gone.name, "yours": s.brushes.user().len()}))
}

fn preset_export(s: &mut Session, p: &Value) -> Result<Value> {
    let path = std::path::PathBuf::from(str_req(p, "path")?);
    let chosen: Vec<BrushPreset> = match p.get("names") {
        None | Some(Value::Null) => s.brushes.user().to_vec(),
        Some(Value::Array(names)) => names
            .iter()
            .take(MAX_USER_PRESETS * 2)
            .map(|n| {
                let n = n.as_str().ok_or_else(|| EngineError::Param("param 'names' must be a list of brush names".into()))?;
                s.brushes.find(n).ok_or_else(|| EngineError::Param(format!("no brush named '{n}'")))
            })
            .collect::<Result<_>>()?,
        Some(_) => return Err(EngineError::Param("param 'names' must be a list of brush names".into())),
    };
    if chosen.is_empty() {
        return Err(EngineError::State("there are no brushes of your own to export yet (save one first)".into()));
    }
    let text = BrushLibrary::file_json(&chosen);
    crate::vfs::write(&path, text.as_bytes())?;
    Ok(json!({"written": path.display().to_string(), "brushes": chosen.len(), "bytes": text.len()}))
}

fn preset_import(s: &mut Session, p: &Value) -> Result<Value> {
    let path = std::path::PathBuf::from(str_req(p, "path")?);
    if crate::vfs::len(&path)? > MAX_FILE_BYTES {
        return Err(EngineError::Param(format!("{} is too large to be a brush set", path.display())));
    }
    let found = BrushLibrary::parse_file(&crate::vfs::read_to_string(&path)?)?;
    if found.is_empty() {
        return Err(EngineError::Param("the brush set has no brushes in it".into()));
    }
    let mut names = Vec::with_capacity(found.len());
    for b in found {
        match s.brushes.add_unique(b) {
            Ok(n) => names.push(n),
            Err(e) if names.is_empty() => return Err(e),
            // The library filled up part way: say what came in.
            Err(_) => break,
        }
    }
    Ok(json!({"imported": names, "yours": s.brushes.user().len()}))
}

/// Where a preview's stroke runs on a `w` × `h` area: an S across it, the pressure swelling
/// in the middle (so pressure, curves and tapers show).
pub fn preview_points(w: f32, h: f32) -> Vec<[f32; 3]> {
    (0..=48)
        .map(|i| {
            let t = i as f32 / 48.0;
            let x = w * (0.1 + 0.8 * t);
            let y = h * (0.5 - 0.22 * (t * std::f32::consts::TAU).sin());
            [x, y, 0.25 + 0.75 * (t * std::f32::consts::PI).sin()]
        })
        .collect()
}

/// A brush's stroke preview, drawn by the CPU renderer on a transparent `w` × `h` image (in
/// pixels; `scale` pixels per point): the preview stroke in `color`, the brush's size kept
/// where it fits (between 2 and 40 % of the height). `as_stroke` previews the Paint Brush.
pub fn preview(preset: &BrushPreset, as_stroke: bool, w: u32, h: u32, scale: f32, color: Rgba) -> Result<anicraft_render::Pixmap> {
    let scale = if scale.is_finite() { scale.clamp(0.25, 8.0) } else { 1.0 };
    let (pw, ph) = (w as f32 / scale, h as f32 / scale);
    if !(pw >= 8.0 && ph >= 8.0 && w <= 4096 && h <= 4096) {
        return Err(EngineError::Param("a brush preview is 8 to 4096 pixels across".into()));
    }
    let mut doc = anicraft_doc::Document::default();
    doc.stage.width = pw.ceil() as u32;
    doc.stage.height = ph.ceil() as u32;
    let mut scratch = Session::new(doc);
    let mut params = BrushPreset { size: preset.size.clamp(2.0, (ph * 0.4).max(2.0)), ..preset.clone() }.params();
    params["points"] = json!(preview_points(pw, ph));
    params["color"] = json!(format!("#{:02x}{:02x}{:02x}", (color.r.clamp(0.0, 1.0) * 255.0).round() as u8, (color.g.clamp(0.0, 1.0) * 255.0).round() as u8, (color.b.clamp(0.0, 1.0) * 255.0).round() as u8));
    params["object"] = json!(true);
    params["as_stroke"] = json!(as_stroke);
    scratch.execute("draw.brush", &params)?;
    let dl = anicraft_doc::evaluate(&scratch.doc, 0);
    let view = anicraft_render::View { transform: Affine::scale(scale, scale), width: w, height: h, background: None };
    Ok(anicraft_render::render(&dl, &view)?.0)
}
