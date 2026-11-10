//! A `MeshLayer`'s `mesh` (points, curves, shapes) → AniCraft shape elements.
//!
//! Geometry stays in Moho's inch coordinates (y up); the layer's element transform carries
//! them onto the stage (see [`Builder::to_px`](crate::import::Builder)). A shape's outline is
//! the chain of `(curve, segment, flag)` runs in its `edges`: `segment` indexes a curve's
//! segments (a closed curve of n points has n, an open one n−1), and `flag` 1 traverses the
//! run backwards (corpus: stroke-only shapes flag their single run, filled loops run
//! everything forward). Handles are reconstructed per point from its neighbours: direction
//! along the curve, rotated by the stored `offset_*` (radians), length
//! `distance/3 · smoothness · weight_*`, zero at smoothness 0 — an approximation of Moho's
//! own handle placement, reported as such; the untouched mesh travels in the sidecar until a
//! later milestone checks reconstruction against Moho renders.

use crate::channel::{self, Channel};
use crate::import::Builder;
use crate::json::{self, array, get};
use anicraft_doc::{ColorEffect, Element, ElementId, ElementKind, GradientStop, Paint, Rgba, Shape, Stroke};
use anicraft_geom::{Affine, FillRule, Path, PathCmd, Vec2};
use serde_json::Value;
use std::sync::Arc;

/// Biggest mesh we take (about a hundred thousand points of hand-drawn work).
const MAX_POINTS: usize = 200_000;
const MAX_CURVES: usize = 40_000;
const MAX_SHAPES: usize = 20_000;
const MAX_STOPS: usize = 64;

/// One curve's point reference with its static handle parameters.
struct CurveRef {
    point: usize,
    smoothness: f32,
    weight_in: f32,
    weight_out: f32,
    offset_in: f32,
    offset_out: f32,
}

/// One parsed curve: point references plus handle data, closed or open.
struct Curve {
    closed: bool,
    pts: Vec<CurveRef>,
}

/// The mesh's static geometry, shared by all its shapes.
struct MeshData {
    positions: Vec<Vec2>,
    curves: Vec<Curve>,
}

/// Builds the shape elements of one mesh. `world` is the layer's accumulated inch-space
/// transform; `alpha` the accumulated layer alpha.
pub(crate) fn mesh_elements(b: &mut Builder, mesh: &Value, world: Affine, alpha: f32, out: &mut Vec<Arc<Element>>) {
    let Some(data) = parse_mesh(b, mesh) else { return };
    report_point_features(b, mesh);
    let Some(shapes) = array(get(mesh, "shapes")) else { return };
    let transform = b.to_px.mul(&world);
    for shape in shapes.iter().take(MAX_SHAPES) {
        shape_element(b, shape, &data, transform, alpha, out);
    }
    if shapes.len() > MAX_SHAPES {
        b.report.skip("meshes past 20,000 shapes");
    }
}

/// Reads points and curves. Broken entries drop their point or curve with a report line,
/// never a panic.
fn parse_mesh(b: &mut Builder, mesh: &Value) -> Option<MeshData> {
    let points = array(get(mesh, "points"))?;
    if points.len() > MAX_POINTS {
        b.report.skip("meshes past 200,000 points");
        return None;
    }
    let mut positions = Vec::with_capacity(points.len());
    for p in points {
        let pos = get(p, "position").and_then(Channel::parse).and_then(|c| channel::sample_vec2(&c, b.frame).map(|v| (c, v)));
        match pos {
            Some((c, v)) if v.is_finite() => {
                if c.is_animated() {
                    b.report.skip("animated point positions (drawn at frame 1)");
                }
                positions.push(v);
            }
            _ => {
                b.report.skip("mesh points without a readable position");
                positions.push(Vec2::ZERO);
            }
        }
    }
    let curves_json = array(get(mesh, "curves"))?;
    if curves_json.len() > MAX_CURVES {
        b.report.skip("meshes past 40,000 curves");
        return None;
    }
    let mut curves = Vec::with_capacity(curves_json.len());
    for c in curves_json {
        let closed = json::boolean(get(c, "closed")).unwrap_or(false);
        let refs = array(get(c, "points")).unwrap_or(&[]);
        let mut pts = Vec::with_capacity(refs.len());
        for r in refs {
            let idx = json::integer(get(r, "point")).and_then(|i| usize::try_from(i).ok());
            let Some(point) = idx.filter(|i| *i < positions.len()) else {
                b.report.skip("curves pointing past the point list");
                continue;
            };
            let ch = |k: &str| get(r, k).and_then(Channel::parse).and_then(|c| channel::sample_f64(&c, b.frame)).unwrap_or(0.0) as f32;
            pts.push(CurveRef {
                point,
                smoothness: ch("smoothness").clamp(0.0, 1.0),
                weight_in: ch("weight_in").max(0.0),
                weight_out: ch("weight_out").max(0.0),
                offset_in: ch("offset_in"),
                offset_out: ch("offset_out"),
            });
        }
        if pts.len() >= 2 {
            curves.push(Curve { closed, pts });
        }
    }
    Some(MeshData { positions, curves })
}

/// Point-level extras that change what is drawn, counted when non-default.
fn report_point_features(b: &mut Builder, mesh: &Value) {
    let frame = b.frame;
    let Some(pts) = array(get(mesh, "points")) else { return };
    if pts.iter().any(|p| json::boolean(get(p, "colored")) == Some(true)) {
        b.report.skip("point-level colours");
    }
    if pts.iter().any(|p| get(p, "width").and_then(Channel::parse).is_some_and(|c| channel::sample_f64(&c, frame).is_some_and(|w| (w - 1.0).abs() > 1e-6))) {
        b.report.skip("variable point widths (drawn uniform)");
    }
    if pts.iter().any(|p| json::integer(get(p, "parent")).is_some_and(|i| i != -2)) {
        b.report.skip("points bound to bones");
    }
}

/// One shape: its outline plus fill/stroke, as one fill-only and one stroke-only element
/// (AniCraft draws them separately, as the XFL importer does).
fn shape_element(b: &mut Builder, shape: &Value, data: &MeshData, transform: Affine, alpha: f32, out: &mut Vec<Arc<Element>>) {
    let has_fill = json::boolean(get(shape, "has_fill")).unwrap_or(false);
    let has_outline = json::boolean(get(shape, "has_outline")).unwrap_or(false);
    if !has_fill && !has_outline {
        return;
    }
    let path = shape_path(b, shape, data);
    if path.is_empty() {
        return;
    }
    let (fill, stroke) = shape_paint(b, shape, &path, has_fill, has_outline);
    shape_flags(b, shape);
    let fx = ColorEffect { alpha: alpha.clamp(0.0, 1.0), ..ColorEffect::default() };
    if let Some(p) = fill {
        out.push(Arc::new(Element {
            id: ElementId(b.doc.alloc_id()),
            name: String::new(),
            transform,
            color: fx,
            kind: ElementKind::Shape(Shape { path: Arc::new(path.clone()), fill: Some(p), stroke: None, fill_rule: FillRule::NonZero }),
        }));
    }
    if let Some(s) = stroke {
        out.push(Arc::new(Element {
            id: ElementId(b.doc.alloc_id()),
            name: String::new(),
            transform,
            color: fx,
            kind: ElementKind::Shape(Shape { path: Arc::new(path), fill: None, stroke: Some(s), fill_rule: FillRule::NonZero }),
        }));
    }
}

/// The shape's outline: `edges` runs chained into subpaths.
fn shape_path(b: &mut Builder, shape: &Value, data: &MeshData) -> Path {
    let Some(edges) = get(shape, "edges") else { return Path::default() };
    let cs = array(get(edges, "curve")).unwrap_or(&[]);
    let ss = array(get(edges, "segment")).unwrap_or(&[]);
    let fs = array(get(edges, "flag")).unwrap_or(&[]);
    if cs.is_empty() {
        return Path::default();
    }
    if cs.len() != ss.len() || cs.len() != fs.len() || cs.len() > MAX_SHAPES {
        b.report.skip("shapes with mismatched edge lists");
        return Path::default();
    }
    let mut path = Path::default();
    let mut pen: Option<Vec2> = None;
    let mut first: Option<Vec2> = None;
    for i in 0..cs.len() {
        let (Some(ci), Some(si), Some(fi)) = (json::integer(cs.get(i)), json::integer(ss.get(i)), json::integer(fs.get(i))) else { continue };
        let (Ok(ci), Ok(si)) = (usize::try_from(ci), usize::try_from(si)) else { continue };
        let Some(curve) = data.curves.get(ci) else {
            b.report.skip("shape edges pointing past the curve list");
            continue;
        };
        let n = curve.pts.len();
        let segments = if curve.closed { n } else { n - 1 };
        if si >= segments {
            b.report.skip("shape edges with out-of-range segments");
            continue;
        }
        let after = if si + 1 < n { si + 1 } else { 0 };
        let (a, d) = (&curve.pts[si], &curve.pts[after]);
        let (Some(p0), Some(p1)) = (data.positions.get(a.point), data.positions.get(d.point)) else { continue };
        let (start, end, h0, h1) = if fi == 1 {
            (*p1, *p0, handle(data, curve, after, true), handle(data, curve, si, false))
        } else {
            (*p0, *p1, handle(data, curve, si, false), handle(data, curve, after, true))
        };
        // Chained runs continue the subpath; a gap starts a new one.
        if !pen.is_some_and(|p| (p - start).length() < 1e-4) {
            path.cmds.push(PathCmd::MoveTo(start));
            first = Some(start);
        }
        if h0.length() > 1e-9 || h1.length() > 1e-9 {
            path.cmds.push(PathCmd::CubicTo(start + h0, end + h1, end));
        } else {
            path.cmds.push(PathCmd::LineTo(end));
        }
        pen = Some(end);
    }
    // A run that came home closes explicitly, so fills see a closed ring.
    if let (Some(f), Some(l)) = (first, pen)
        && path.cmds.len() > 1
        && (f - l).length() < 1e-4
    {
        path.cmds.push(PathCmd::Close);
    }
    path
}

/// The outgoing (or, with `in_side`, incoming) handle of curve point `i`, from its
/// neighbour's tangent and the stored smoothness/weight/offset. Closed curves wrap; an open
/// curve's ends have no handle on the outside (no segment arrives there).
fn handle(data: &MeshData, curve: &Curve, i: usize, in_side: bool) -> Vec2 {
    let n = curve.pts.len();
    if n < 2 {
        return Vec2::ZERO;
    }
    let r = &curve.pts[i];
    if r.smoothness <= 0.001 {
        return Vec2::ZERO;
    }
    let j = if in_side {
        if i > 0 {
            i - 1
        } else if curve.closed {
            n - 1
        } else {
            return Vec2::ZERO;
        }
    } else if i + 1 < n {
        i + 1
    } else if curve.closed {
        0
    } else {
        return Vec2::ZERO;
    };
    let (Some(p), Some(q)) = (data.positions.get(r.point), data.positions.get(curve.pts[j].point)) else { return Vec2::ZERO };
    let (weight, offset) = if in_side { (r.weight_in, r.offset_in) } else { (r.weight_out, r.offset_out) };
    let dir = if in_side { *p - *q } else { *q - *p };
    let len = dir.length() / 3.0 * r.smoothness * weight;
    if !len.is_finite() || len <= 1e-9 || dir.length() <= 1e-9 {
        return Vec2::ZERO;
    }
    let unit = dir.normalized();
    // The stored offset is a small angle (radians) off the tangent.
    let (sin, cos) = offset.sin_cos();
    let rotated = Vec2::new(unit.x * cos - unit.y * sin, unit.x * sin + unit.y * cos);
    if !rotated.is_finite() {
        return Vec2::ZERO;
    }
    let sign = if in_side { -1.0 } else { 1.0 };
    rotated * (len * sign)
}

/// Fill and stroke paint for one shape, from its inline `style`.
fn shape_paint(b: &mut Builder, shape: &Value, path: &Path, has_fill: bool, has_outline: bool) -> (Option<Paint>, Option<Stroke>) {
    let empty = Value::Null;
    let style = get(shape, "style").unwrap_or(&empty);
    let frame = b.frame;
    let fill = if has_fill {
        let gradient = get(style, "fill_style").filter(|_| json::integer(get(style, "fill_style_id")) == Some(9));
        match gradient {
            Some(g) if matches!(json::integer(get(g, "gradient_type")), Some(0) | Some(1)) => {
                gradient_paint(b, g, path).or_else(|| {
                    b.report.skip("unreadable gradient fills (drawn solid)");
                    solid_fill(style, frame)
                })
            }
            // Reflected and angle gradients have no paint variant yet: the nearest
            // supported stand-in is the style's solid colour, counted.
            Some(g) => {
                b.report.skip("reflected and angle gradient fills");
                let _ = g;
                solid_fill(style, frame)
            }
            None => solid_fill(style, frame),
        }
    } else {
        None
    };
    let stroke = if has_outline {
        let color = get(style, "line_color").and_then(Channel::parse).and_then(|c| channel::sample_color01(&c, frame)).unwrap_or(Rgba::BLACK);
        let width = json::num(get(style, "line_width")).unwrap_or(0.022) as f32;
        Some(Stroke::new(color, width.clamp(1e-4, 10.0)))
    } else {
        None
    };
    (fill, stroke)
}

fn solid_fill(style: &Value, frame: f64) -> Option<Paint> {
    let c = get(style, "fill_color").and_then(Channel::parse).and_then(|c| channel::sample_color01(&c, frame))?;
    Some(Paint::Solid(c))
}

/// A linear (0) or radial (1) `SS_Gradient2` fill: stops from `{location, color}` channel
/// pairs. Moho keeps the gradient's geometry elsewhere (its home is unverified), so the
/// gradients are oriented from the shape's bounds — counted here, and exact once that home
/// is known. (Reflected 2 / angle 3 are rejected by the caller.)
fn gradient_paint(b: &mut Builder, g: &Value, path: &Path) -> Option<Paint> {
    let kind = json::integer(get(g, "gradient_type")).unwrap_or(0);
    let marks = array(get(g, "gradients"))?;
    let frame = b.frame;
    let mut stops: Vec<GradientStop> = marks
        .iter()
        .take(MAX_STOPS)
        .filter_map(|m| {
            let o = get(m, "location").and_then(Channel::parse).and_then(|c| channel::sample_f64(&c, frame))?;
            let c = get(m, "color").and_then(Channel::parse).and_then(|c| channel::sample_color01(&c, frame))?;
            Some(GradientStop { offset: (o as f32).clamp(0.0, 1.0), color: c })
        })
        .collect();
    if stops.is_empty() {
        return None;
    }
    stops.sort_by(|a, s| a.offset.total_cmp(&s.offset));
    b.report.skip("gradient geometry (approximated from shape bounds)");
    let bounds = path.bounds();
    let centre = Vec2::new((bounds.min.x + bounds.max.x) * 0.5, (bounds.min.y + bounds.max.y) * 0.5);
    if kind == 0 {
        Some(Paint::Linear { start: Vec2::new(bounds.min.x, centre.y), end: Vec2::new(bounds.max.x, centre.y), stops })
    } else {
        let r = (bounds.max.x - bounds.min.x).hypot(bounds.max.y - bounds.min.y) * 0.5;
        Some(Paint::Radial { center: centre, radius: r.max(1e-4), stops })
    }
}

/// Per-shape features stage 1 draws at rest (or not at all), counted when non-default.
/// (`3d_thickness` is not counted: Moho writes its default 0.125 on every shape, and it only
/// renders under a layer `3d_mode`, which is counted at the layer.)
fn shape_flags(b: &mut Builder, shape: &Value) {
    let frame = b.frame;
    let ch = |k: &str| get(shape, k).and_then(Channel::parse);
    if ch("effect_offset").and_then(|c| channel::sample_vec2(&c, frame)).is_some_and(|v| v.length() > 1e-6) {
        b.report.skip("shape effect offsets (drawn at rest)");
    }
    if ch("effect_scale").and_then(|c| channel::sample_f64(&c, frame)).is_some_and(|v| (v - 1.0).abs() > 1e-6) {
        b.report.skip("shape effect scales (drawn at rest)");
    }
    if ch("effect_rotation").and_then(|c| channel::sample_f64(&c, frame)).is_some_and(|v| v.abs() > 1e-6) {
        b.report.skip("shape effect rotations (drawn at rest)");
    }
    if json::integer(get(shape, "combo_mode")).is_some_and(|v| v != 0) {
        b.report.skip("shape combo modes");
    }
}
