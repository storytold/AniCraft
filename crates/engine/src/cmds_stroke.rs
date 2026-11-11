//! Stroke styles (ADR 0046): caps, joins, the miter limit, and dashed and dotted lines.
//!
//! The same style params work on `element.set_stroke`, the drawing commands and `paint.ink`
//! (see [`crate::params::opt_stroke_style`]); `stroke.set_style` is the one Properties uses.
//! Dash lengths and dot spacing are in the shape's own units, like the stroke width.

use crate::cmds_width::{id_param, shape_mut, visible_shape};
use crate::params::*;
use crate::{CommandSpec, EngineError, Result, Session};
use anicraft_doc::{Dash, ElementId, StrokeStyle};
use serde_json::{Value, json};

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: "stroke.set_style",
        label: "Stroke Style",
        menu: "",
        shortcut: None,
        params: "{cap?: none|round|square, join?: miter|round|bevel, miter? (limit, 1..=100; a corner whose point is longer than this many widths is bevelled), style?: solid|dashed|dotted, dash? ([on, off, ..] lengths in the shape's units; implies dashed), dash_offset?, dot_spacing? (between dots, edge to edge; implies dotted), id? (default: the selected shapes)}",
        mutates: true,
        run: |s, p| s.each_layer(false, |s| set_style(s, p)),
    }]
}

/// What a style is, as commands report it.
pub(crate) fn describe(st: &StrokeStyle) -> Value {
    let mut v = json!({"cap": st.cap.key(), "join": st.join.key(), "miter": st.miter_limit, "style": st.dash_kind().key()});
    match &st.dash {
        Some(Dash::Dashed { lengths, offset }) => {
            v["dash"] = json!(lengths.to_vec());
            v["dash_offset"] = json!(offset);
        }
        Some(Dash::Dotted { spacing }) => v["dot_spacing"] = json!(spacing),
        None => {}
    }
    v
}

fn set_style(s: &mut Session, p: &Value) -> Result<Value> {
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
    let mut last = None;
    for id in &stroked {
        if let Some(st) = shape_mut(s, *id)?.stroke.as_mut() {
            // Each stroke keeps what the params leave out (and sizes a new pattern by its width).
            let style = opt_stroke_style(p, &st.style, st.width)?.ok_or_else(|| EngineError::Param(format!("give a style to set ({STROKE_STYLE_PARAMS})")))?;
            st.style = style;
            last = Some(st.style.clone());
        }
    }
    let mut out = last.as_ref().map_or_else(|| json!({}), describe);
    out["changed"] = json!(stroked.len());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anicraft_doc::{Cap, ElementKind, Join, Stroke};

    fn line(s: &mut Session) -> u64 {
        let v = s.execute("draw.line", &json!({"x0": 100, "y0": 100, "x1": 300, "y1": 100, "stroke_width": 4})).unwrap();
        v["element"].as_u64().unwrap()
    }

    fn stroke(s: &Session, id: u64) -> Option<Stroke> {
        s.visible_elements().into_iter().find(|e| e.id.0 == id).and_then(|e| match &e.kind {
            ElementKind::Shape(sh) => sh.stroke.clone(),
            _ => None,
        })
    }

    #[test]
    fn styles_apply_to_the_selection_undo_and_round_trip() {
        let mut s = Session::default();
        let id = line(&mut s);
        assert!(stroke(&s, id).unwrap().style.is_default(), "new strokes are round and solid");
        let v = s.execute("stroke.set_style", &json!({"cap": "none", "join": "miter", "miter": 6})).unwrap();
        assert_eq!((v["cap"].as_str(), v["join"].as_str(), v["miter"].as_f64(), v["changed"].as_u64()), (Some("butt"), Some("miter"), Some(6.0), Some(1)));
        // Dashed with no lengths: sized by the stroke (4 wide: 12 on, 8 off).
        let v = s.execute("stroke.set_style", &json!({"style": "dashed"})).unwrap();
        assert_eq!(v["dash"], json!([12.0, 8.0]));
        let st = stroke(&s, id).unwrap().style;
        assert_eq!((st.cap, st.join, st.miter_limit), (Cap::Butt, Join::Miter, 6.0), "what the params leave out stays");
        // A pattern of its own, then dots; each is one undo step.
        s.execute("stroke.set_style", &json!({"dash": [5, 2, 1, 2], "dash_offset": 3})).unwrap();
        assert_eq!(stroke(&s, id).unwrap().style.dash, Dash::Dashed { lengths: vec![5.0, 2.0, 1.0, 2.0].into(), offset: 3.0 }.sanitized());
        let v = s.execute("stroke.set_style", &json!({"dot_spacing": 3, "id": id})).unwrap();
        assert_eq!((v["style"].as_str(), v["dot_spacing"].as_f64()), (Some("dotted"), Some(3.0)));
        s.undo().unwrap();
        assert_eq!(stroke(&s, id).unwrap().style.dash_kind(), anicraft_doc::DashKind::Dashed);
        // The .anicraft file keeps it.
        let text = crate::format::to_json(&s.doc).unwrap();
        assert!(text.contains("\"butt\"") && text.contains("\"lengths\""), "styled strokes are written");
        assert_eq!(crate::format::from_json(&text).unwrap(), s.doc);
        // Back to solid.
        s.execute("stroke.set_style", &json!({"style": "solid", "cap": "round", "join": "round", "miter": 3})).unwrap();
        assert!(stroke(&s, id).unwrap().style.is_default());
        let text = crate::format::to_json(&s.doc).unwrap();
        assert!(!text.contains("\"style\""), "a default style is written as nothing, as before styles");
    }

    #[test]
    fn bad_style_params_are_refused_and_change_nothing() {
        let mut s = Session::default();
        let id = line(&mut s);
        for bad in [
            json!({}),
            json!({"cap": "pointy"}),
            json!({"join": 3}),
            json!({"miter": 0.5}),
            json!({"miter": 1e9}),
            json!({"style": "ragged"}),
            json!({"dash": []}),
            json!({"dash": [5, 0]}),
            json!({"dash": [-1, 2]}),
            json!({"dash": [1e30, 2]}),
            json!({"dash": (0..40).collect::<Vec<_>>()}),
            json!({"dash": "5 2"}),
            json!({"dash_offset": 1e30}),
            json!({"dot_spacing": -2}),
        ] {
            assert!(s.execute("stroke.set_style", &bad).is_err(), "{bad}");
            assert!(s.execute("element.set_stroke", &bad).is_err() || bad == json!({}), "{bad}");
        }
        assert!(stroke(&s, id).unwrap().style.is_default());
        s.execute("edit.deselect", &json!({})).unwrap();
        assert!(s.execute("stroke.set_style", &json!({"cap": "square"})).is_err(), "nothing selected");
    }

    #[test]
    fn drawing_set_stroke_and_the_ink_bottle_carry_the_style() {
        let mut s = Session::default();
        // Drawing commands take the tool's line style, or params.
        s.style.line = StrokeStyle { cap: Cap::Square, ..StrokeStyle::default() };
        let id = line(&mut s);
        assert_eq!(stroke(&s, id).unwrap().style.cap, Cap::Square);
        let v = s.execute("draw.rect", &json!({"x": 400, "y": 400, "w": 50, "h": 40, "stroke": "#000000", "join": "bevel", "dash": [4, 2]})).unwrap();
        let rect = v["element"].as_u64().unwrap();
        let st = stroke(&s, rect).unwrap().style;
        assert_eq!((st.cap, st.join, st.dash_kind()), (Cap::Square, Join::Bevel, anicraft_doc::DashKind::Dashed));
        // element.set_stroke takes the same params.
        s.execute("element.set_stroke", &json!({"width": 6, "style": "dotted"})).unwrap();
        assert_eq!(stroke(&s, rect).unwrap().style.dash, Dash::dotted(6.0), "dots one width apart");
        // The Ink Bottle gives the tool's whole line.
        s.style.line = StrokeStyle { join: Join::Miter, dash: Dash::dashed(7.0, 3.0), ..StrokeStyle::default() };
        s.execute("paint.ink", &json!({"x": 200, "y": 100})).unwrap();
        let st = stroke(&s, id).unwrap().style;
        assert_eq!((st.join, st.dash), (Join::Miter, Dash::dashed(7.0, 3.0)));
        s.execute("paint.ink", &json!({"x": 200, "y": 100, "cap": "none", "style": "solid"})).unwrap();
        let st = stroke(&s, id).unwrap().style;
        assert_eq!((st.cap, st.dash), (Cap::Butt, None));
    }

    #[test]
    fn the_eyedropper_picks_the_style_and_scales_its_dashes() {
        let mut s = Session::default();
        let id = line(&mut s);
        s.execute("stroke.set_style", &json!({"cap": "square", "dash": [6, 2]})).unwrap();
        s.execute("element.set_transform", &json!({"sx": 2, "sy": 2})).unwrap();
        let v = s.execute("paint.pick", &json!({"x": 200, "y": 100})).unwrap();
        assert_eq!((v["cap"].as_str(), v["style"].as_str()), (Some("square"), Some("dashed")), "{v}");
        assert_eq!(s.style.line.dash, Dash::dashed(12.0, 4.0), "as drawn: twice as long");
        assert!(stroke(&s, id).is_some());
    }
}
