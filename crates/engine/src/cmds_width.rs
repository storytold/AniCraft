//! Variable-width strokes (ADR 0018): width profiles and the Width tool's width points.
//!
//! Width points are addressed by index on a shape's profile (sorted by position). Positions
//! `t` are fractions of each subpath's arc length; widths are factors of the stroke's nominal
//! width on each side (1 = the width set in Properties). Stage points in params are found on
//! the stroke by arc length in the shape's own space.

use crate::params::*;
use crate::{CommandSpec, EngineError, Result, Session};
use anicraft_doc::width::{nearest_on_stroke, stroke_walks};
use anicraft_doc::{Element, ElementId, ElementKind, MAX_WIDTH_FACTOR, Shape, WidthPoint, WidthProfile};
use anicraft_geom::Vec2;
use serde_json::{Value, json};

pub fn specs() -> Vec<CommandSpec> {
    let c = |id, label, menu, shortcut, params, mutates, run| CommandSpec { id, label, menu, shortcut, params, mutates, run };
    vec![
        c(
            "stroke.set_profile",
            "Width Profile",
            "",
            None,
            "{profile: uniform|taper_both|taper_start|taper_end|bulge|pinch | [[t, left, right], ..], id? (default: the selected shapes)}",
            true,
            (|s, p| s.each_layer(false, |s| set_profile(s, p))) as crate::RunFn,
        ),
        c(
            "stroke.set_width_point",
            "Width",
            "",
            None,
            "{id, index? (an existing point; else one is added), t? 0..1 | x, y (stage point on the stroke), left?, right?, width? (both sides; factors of the nominal width)}",
            true,
            set_width_point,
        ),
        c("stroke.delete_width_point", "Delete Width Point", "", None, "{id, index}", true, delete_width_point),
        c("stroke.width_at", "Width at Point", "", None, "{x, y, id? (default: the topmost stroke there on the active layer), slop?}", false, width_at),
    ]
}

/// Flattening tolerance for finding places on a stroke, in stage units.
const FIND_TOL: f32 = 0.2;

pub(crate) fn id_param(p: &Value) -> Result<Option<ElementId>> {
    Ok(opt_u64(p, "id")?.map(ElementId))
}

/// The shape element `id` as it shows at the playhead on the active layer.
pub(crate) fn visible_shape(s: &Session, id: ElementId) -> Result<std::sync::Arc<Element>> {
    let el = s.visible_elements().into_iter().find(|e| e.id == id).ok_or_else(|| EngineError::State(format!("element {} is not on the active layer at this frame", id.0)))?;
    match &el.kind {
        ElementKind::Shape(sh) if sh.stroke.is_some() => Ok(el),
        ElementKind::Shape(_) => Err(EngineError::State("this shape has no stroke (give it one in Properties or with the Ink Bottle)".into())),
        ElementKind::Symbol { .. } | ElementKind::Text(_) | ElementKind::Camera | ElementKind::Group { .. } => Err(EngineError::State("only shapes have strokes (break it apart with Ctrl+B to edit its lines)".into())),
    }
}

/// The editable copy of shape `id` (copy-on-write; keys a tween's frame like any edit).
pub(crate) fn shape_mut(s: &mut Session, id: ElementId) -> Result<&mut Shape> {
    visible_shape(s, id)?;
    let kf = s.editable_keyframe()?;
    let el = kf.elements_mut_where(|e| e == id).into_iter().next().ok_or_else(|| EngineError::State(format!("no element {}", id.0)))?;
    match &mut el.kind {
        ElementKind::Shape(sh) => Ok(sh),
        ElementKind::Symbol { .. } | ElementKind::Text(_) | ElementKind::Camera | ElementKind::Group { .. } => Err(EngineError::State("only shapes have strokes".into())),
    }
}

/// Where stage point `at` is on element `el`'s stroke: `(t, distance in stage units, side)`.
fn place(el: &Element, at: Vec2) -> Option<(f32, f32, f32)> {
    let ElementKind::Shape(sh) = &el.kind else { return None };
    let inv = el.transform.inverse()?;
    let scale = el.transform.mean_scale();
    let walks = stroke_walks(&sh.path, FIND_TOL / scale.max(1e-3));
    let (_, t, d, side) = nearest_on_stroke(&walks, inv.apply(at))?;
    // A mirrored shape has its sides swapped on the stage.
    let side = if el.transform.determinant() < 0.0 { -side } else { side };
    Some((t, d * scale, side))
}

fn factor(p: &Value, key: &str) -> Result<Option<f32>> {
    match opt_f32(p, key)? {
        Some(v) if (0.0..=MAX_WIDTH_FACTOR).contains(&v) => Ok(Some(v)),
        Some(_) => Err(EngineError::Param(format!("param '{key}' must be between 0 and {MAX_WIDTH_FACTOR} (a factor of the stroke's width)"))),
        None => Ok(None),
    }
}

fn set_profile(s: &mut Session, p: &Value) -> Result<Value> {
    let profile = opt_profile(p, "profile")?.ok_or_else(|| EngineError::Param("param 'profile' is required".into()))?;
    let ids: Vec<ElementId> = match id_param(p)? {
        Some(id) => vec![id],
        None => s.selection.on(s.layer),
    };
    if ids.is_empty() {
        return Err(EngineError::State("nothing is selected".into()));
    }
    let stroked: Vec<ElementId> = ids.into_iter().filter(|id| visible_shape(s, *id).is_ok()).collect();
    if stroked.is_empty() {
        return Err(EngineError::State("the selection has no stroked shapes".into()));
    }
    for id in &stroked {
        if let Some(st) = shape_mut(s, *id)?.stroke.as_mut() {
            st.profile = profile.clone();
        }
    }
    Ok(json!({"changed": stroked.len(), "profile": profile.as_ref().and_then(WidthProfile::preset).map_or(if profile.is_some() { "custom" } else { "uniform" }, |pr| pr.key())}))
}

fn set_width_point(s: &mut Session, p: &Value) -> Result<Value> {
    let id = id_param(p)?.ok_or_else(|| EngineError::Param("param 'id' is required".into()))?;
    let index = opt_u64(p, "index")?.map(|i| usize::try_from(i).unwrap_or(usize::MAX));
    let (left, right, both) = (factor(p, "left")?, factor(p, "right")?, factor(p, "width")?);
    let t = match opt_f32(p, "t")? {
        Some(t) if (0.0..=1.0).contains(&t) => Some(t),
        Some(_) => return Err(EngineError::Param("param 't' must be between 0 and 1".into())),
        None => match (opt_f32(p, "x")?, opt_f32(p, "y")?) {
            (Some(x), Some(y)) => {
                let el = visible_shape(s, id)?;
                Some(place(&el, Vec2::new(x, y)).ok_or_else(|| EngineError::State("cannot find that point on the stroke".into()))?.0)
            }
            _ => None,
        },
    };
    let sh = shape_mut(s, id)?;
    let st = sh.stroke.as_mut().ok_or_else(|| EngineError::State("this shape has no stroke".into()))?;
    let current = st.profile.clone();
    let next = match index {
        Some(i) => {
            let pr = current.as_ref().ok_or_else(|| EngineError::Param("this stroke has no width points (it is uniform)".into()))?;
            let old = *pr.points().get(i).ok_or_else(|| EngineError::Param(format!("no width point {i} (the stroke has {})", pr.points().len())))?;
            let w = WidthPoint::new(t.unwrap_or(old.t), both.or(left).unwrap_or(old.left), both.or(right).unwrap_or(old.right));
            pr.with_replaced(i, w)
        }
        None => {
            let t = t.ok_or_else(|| EngineError::Param("param 't' (or 'x' and 'y') is required to add a width point".into()))?;
            let (l, r) = current.as_ref().map_or((1.0, 1.0), |pr| pr.at(t));
            let w = WidthPoint::new(t, both.or(left).unwrap_or(l), both.or(right).unwrap_or(r));
            match &current {
                Some(pr) => {
                    if pr.points().len() >= anicraft_doc::MAX_WIDTH_POINTS && pr.index_at(t).is_none() {
                        return Err(EngineError::State(format!("a stroke keeps at most {} width points", anicraft_doc::MAX_WIDTH_POINTS)));
                    }
                    pr.with_point(w)
                }
                None => WidthProfile::new(vec![w]),
            }
        }
    };
    st.profile = next;
    // Where the point is now (moving one keeps it between its neighbours).
    let at = match (index, &st.profile) {
        (Some(i), Some(pr)) => pr.points().get(i).map(|q| (i, *q)),
        (None, Some(pr)) => t.and_then(|t| pr.index_at(t)).and_then(|i| pr.points().get(i).map(|q| (i, *q))),
        _ => None,
    };
    Ok(match at {
        Some((i, q)) => json!({"index": i, "t": q.t, "left": q.left, "right": q.right, "points": st.profile.as_ref().map_or(0, |p| p.points().len())}),
        None => json!({"index": null, "points": 0}),
    })
}

fn delete_width_point(s: &mut Session, p: &Value) -> Result<Value> {
    let id = id_param(p)?.ok_or_else(|| EngineError::Param("param 'id' is required".into()))?;
    let index = opt_u64(p, "index")?.ok_or_else(|| EngineError::Param("param 'index' is required".into()))?;
    let index = usize::try_from(index).unwrap_or(usize::MAX);
    let sh = shape_mut(s, id)?;
    let st = sh.stroke.as_mut().ok_or_else(|| EngineError::State("this shape has no stroke".into()))?;
    let pr = st.profile.as_ref().ok_or_else(|| EngineError::Param("this stroke has no width points".into()))?;
    if index >= pr.points().len() {
        return Err(EngineError::Param(format!("no width point {index} (the stroke has {})", pr.points().len())));
    }
    st.profile = pr.without(index);
    Ok(json!({"points": st.profile.as_ref().map_or(0, |p| p.points().len())}))
}

fn width_at(s: &mut Session, p: &Value) -> Result<Value> {
    let at = Vec2::new(f32_req(p, "x")?, f32_req(p, "y")?);
    let slop = opt_f32(p, "slop")?.unwrap_or(4.0).clamp(0.0, 1000.0);
    let describe = |el: &Element, (t, d, side): (f32, f32, f32)| -> Option<Value> {
        let ElementKind::Shape(sh) = &el.kind else { return None };
        let st = sh.stroke.as_ref()?;
        let (l, r) = st.profile.as_ref().map_or((1.0, 1.0), |pr| pr.at(t));
        let point = st.profile.as_ref().and_then(|pr| pr.points().iter().position(|q| (q.t - t).abs() < 0.01));
        Some(json!({"element": el.id.0, "t": t, "distance": d, "side": if side > 0.0 { "left" } else { "right" }, "left": l, "right": r, "width": st.width, "point": point}))
    };
    if let Some(id) = id_param(p)? {
        let el = visible_shape(s, id)?;
        let found = place(&el, at).ok_or_else(|| EngineError::State("cannot find that point on the stroke".into()))?;
        return describe(&el, found).ok_or_else(|| EngineError::State("this shape has no stroke".into()));
    }
    // The topmost stroke whose drawn edge (at its width there) is within `slop` of the point.
    for el in s.visible_elements().iter().rev().take(20_000) {
        let ElementKind::Shape(sh) = &el.kind else { continue };
        let Some(st) = &sh.stroke else { continue };
        let scale = el.transform.mean_scale();
        let reach = st.reach() * scale + slop;
        if !sh.path.bounds().transformed(&el.transform).inflate(reach).contains(at) {
            continue;
        }
        let Some(found @ (t, d, side)) = place(el, at) else { continue };
        let (l, r) = st.profile.as_ref().map_or((1.0, 1.0), |pr| pr.at(t));
        // Stage-space side: left factor on the left.
        let here = (if side > 0.0 { l } else { r }) * st.width * 0.5 * scale;
        if d <= here + slop {
            return describe(el, found).ok_or_else(|| EngineError::State("no stroke there".into()));
        }
    }
    Ok(json!({"element": null}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use anicraft_doc::ProfilePreset;

    fn line(s: &mut Session) -> u64 {
        let v = s.execute("draw.line", &json!({"x0": 100, "y0": 100, "x1": 300, "y1": 100, "stroke_width": 10})).unwrap();
        v["element"].as_u64().unwrap()
    }

    fn profile(s: &Session, id: u64) -> Option<WidthProfile> {
        s.visible_elements().into_iter().find(|e| e.id.0 == id).and_then(|e| match &e.kind {
            ElementKind::Shape(sh) => sh.stroke.as_ref().and_then(|st| st.profile.clone()),
            _ => None,
        })
    }

    #[test]
    fn presets_apply_to_the_selection_and_round_trip() {
        let mut s = Session::default();
        let id = line(&mut s);
        let v = s.execute("stroke.set_profile", &json!({"profile": "taper_both"})).unwrap();
        assert_eq!(v["profile"], "taper_both");
        assert_eq!(profile(&s, id), ProfilePreset::TaperBoth.profile());
        // Custom points, then back to uniform; each is one undo step.
        s.execute("stroke.set_profile", &json!({"profile": [[0.2, 2, 0.5], {"t": 0.9, "left": 0, "right": 1}]})).unwrap();
        assert_eq!(profile(&s, id).unwrap().points().len(), 2);
        s.execute("stroke.set_profile", &json!({"profile": "uniform", "id": id})).unwrap();
        assert!(profile(&s, id).is_none());
        s.undo().unwrap();
        assert_eq!(profile(&s, id).unwrap().points().len(), 2);
        // The .anicraft file keeps it; older files (no profile) still load as uniform.
        let text = crate::format::to_json(&s.doc).unwrap();
        assert!(text.contains("\"profile\""));
        let back = crate::format::from_json(&text).unwrap();
        assert_eq!(back, s.doc);
        for bad in [json!({"profile": "wobbly"}), json!({"profile": [[2, 1, 1]]}), json!({"profile": [[0.5, 99, 1]]}), json!({"profile": 5}), json!({})] {
            assert!(s.execute("stroke.set_profile", &bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn width_points_add_move_and_delete() {
        let mut s = Session::default();
        let id = line(&mut s);
        // A stage point on the stroke adds a point where it is, at the width it has there.
        let v = s.execute("stroke.set_width_point", &json!({"id": id, "x": 150, "y": 103, "width": 3})).unwrap();
        assert_eq!(v["index"], 0);
        assert!((v["t"].as_f64().unwrap() - 0.25).abs() < 1e-3, "{v}");
        let at = s.execute("stroke.width_at", &json!({"x": 150, "y": 90})).unwrap();
        assert_eq!(at["element"], id);
        assert!((at["left"].as_f64().unwrap() - 3.0).abs() < 1e-4, "a single point widens it everywhere: {at}");
        assert_eq!(at["side"], "left", "above a line heading right is its left");
        assert_eq!(s.execute("stroke.width_at", &json!({"x": 150, "y": 110})).unwrap()["side"], "right");
        assert_eq!(at["point"], 0);
        // One side only, then move the point along the stroke.
        let v = s.execute("stroke.set_width_point", &json!({"id": id, "t": 0.75, "left": 0.5})).unwrap();
        assert_eq!(v["index"], 1);
        assert_eq!((v["left"].as_f64(), v["right"].as_f64()), (Some(0.5), Some(3.0)), "the other side keeps its width");
        let v = s.execute("stroke.set_width_point", &json!({"id": id, "index": 1, "t": 0.6})).unwrap();
        assert!((v["t"].as_f64().unwrap() - 0.6).abs() < 1e-5);
        s.execute("stroke.delete_width_point", &json!({"id": id, "index": 0})).unwrap();
        assert_eq!(profile(&s, id).unwrap().points().len(), 1);
        // Deleting the last one leaves a uniform stroke.
        s.execute("stroke.delete_width_point", &json!({"id": id, "index": 0})).unwrap();
        assert!(profile(&s, id).is_none());
        for bad in [json!({"id": id, "index": 5}), json!({"id": id}), json!({"id": id, "t": 2}), json!({"id": 999, "t": 0.5}), json!({"id": id, "t": 0.5, "left": -1})] {
            assert!(s.execute("stroke.set_width_point", &bad).is_err(), "{bad}");
        }
        assert!(s.execute("stroke.delete_width_point", &json!({"id": id, "index": 0})).is_err());
        // Far from any stroke: nothing.
        assert_eq!(s.execute("stroke.width_at", &json!({"x": 500, "y": 500})).unwrap()["element"], Value::Null);
    }

    #[test]
    fn a_drag_is_one_undo_step_and_tools_draw_with_the_chosen_profile() {
        let mut s = Session::default();
        let id = line(&mut s);
        let before = s.history_len();
        s.begin_gesture("Width");
        let v = s.execute("stroke.set_width_point", &json!({"id": id, "t": 0.5, "width": 1.2})).unwrap();
        let i = v["index"].as_u64().unwrap();
        for w in [1.5, 2.0, 2.5] {
            s.execute("stroke.set_width_point", &json!({"id": id, "index": i, "width": w})).unwrap();
        }
        s.end_gesture();
        assert_eq!(s.history_len(), before + 1);
        s.undo().unwrap();
        assert!(profile(&s, id).is_none());
        s.style.stroke_profile = ProfilePreset::TaperEnd;
        let v = s.execute("draw.line", &json!({"x0": 0, "y0": 0, "x1": 50, "y1": 50})).unwrap();
        assert_eq!(profile(&s, v["element"].as_u64().unwrap()), ProfilePreset::TaperEnd.profile());
        let v = s.execute("draw.line", &json!({"x0": 0, "y0": 0, "x1": 50, "y1": 9, "profile": "uniform"})).unwrap();
        assert!(profile(&s, v["element"].as_u64().unwrap()).is_none(), "a param overrides the tool's profile");
    }

    #[test]
    fn shape_tweens_blend_width_profiles() {
        let mut s = Session::default();
        let id = line(&mut s);
        s.execute("stroke.set_profile", &json!({"profile": "taper_both"})).unwrap();
        s.execute("timeline.insert_keyframe", &json!({"frame": 10})).unwrap();
        s.set_frame(10);
        s.execute("edit.select_all", &json!({})).unwrap();
        s.execute("stroke.set_profile", &json!({"profile": "bulge"})).unwrap();
        s.set_frame(0);
        s.execute("timeline.create_shape_tween", &json!({})).unwrap();
        s.set_frame(5);
        let mid = profile(&s, id).expect("tweened profile");
        let (a, b) = (ProfilePreset::TaperBoth.profile().unwrap(), ProfilePreset::Bulge.profile().unwrap());
        let w = mid.at(0.5).0;
        assert!(w > a.at(0.5).0 && w < b.at(0.5).0, "between the two keyframes: {w}");
    }
}
