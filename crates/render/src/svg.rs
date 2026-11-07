//! SVG output of a display list: the vector counterpart of [`crate::render`].
//!
//! It draws what the CPU renderer draws, from the same evaluated frame:
//! * paths are written in stage space, and strokes get the same stage-space width
//!   (`width × mean scale`, round joins and caps);
//! * gradients keep their local geometry under a `gradientTransform`, padded at the ends;
//! * colour effects (alpha, tint) are baked into every colour and stop;
//! * masks become `clipPath`s (fill areas only, nested through `clip-path` on the clip), and an
//!   empty mask clips everything, as on the stage;
//! * outline-mode layers are hairlines in the layer colour.
//!
//! The output is deterministic and contains only generated ids and numbers, so it needs no
//! escaping.

use anicraft_doc::{ColorEffect, DisplayItem, DisplayList, Paint, Rgba, Stage};
use anicraft_geom::{Affine, FillRule};
use std::fmt::Write;

/// Writes `dl` as a standalone SVG document the size of `stage`, with its background.
pub fn to_svg(dl: &DisplayList, stage: &Stage) -> String {
    let (w, h) = (stage.width, stage.height);
    let mut defs = String::new();
    let mut body = String::new();
    let mut grad = 0usize;

    // Clips, in order (a parent always precedes its children; anything else clips all).
    for (i, parent) in dl.clip_parents.iter().enumerate() {
        let parent = parent.filter(|&p| (p as usize) < i);
        let malformed = dl.clip_parents.get(i).copied().flatten().is_some() && parent.is_none();
        let _ = write!(defs, "<clipPath id=\"c{i}\" clipPathUnits=\"userSpaceOnUse\"");
        if let Some(p) = parent {
            let _ = write!(defs, " clip-path=\"url(#c{p})\"");
        }
        defs.push('>');
        if !malformed {
            for item in dl.items.iter().filter(|it| it.masks == Some(i as u32) && it.fill.is_some()) {
                if let Some(d) = stage_path(item) {
                    let _ = write!(defs, "<path d=\"{d}\"{}/>", rule_attr("clip-rule", item.fill_rule));
                }
            }
        }
        defs.push_str("</clipPath>");
    }

    for item in dl.items.iter().filter(|it| it.masks.is_none() && !it.guide) {
        let Some(d) = stage_path(item) else { continue };
        let clip = item.clip.filter(|&c| (c as usize) < dl.clip_parents.len()).map(|c| format!(" clip-path=\"url(#c{c})\"")).unwrap_or_default();
        if let Some(color) = item.outline {
            // Outline mode: a hairline in the layer colour, no fill.
            let _ = write!(body, "<path d=\"{d}\" fill=\"none\"{} stroke-width=\"1\"{clip}/>", color_attrs("stroke", color));
            continue;
        }
        if let Some(fill) = &item.fill
            && let Some(paint) = paint_attr(fill, &item.transform, &item.color, &mut defs, &mut grad)
        {
            let _ = write!(body, "<path d=\"{d}\"{paint}{}{clip}/>", rule_attr("fill-rule", item.fill_rule));
        }
        if let Some(st) = item.stroke {
            let width = st.width * item.transform.mean_scale();
            if width.is_finite() && width > 0.0 {
                let c = item.color.apply(st.color);
                let _ = write!(
                    body,
                    "<path d=\"{d}\" fill=\"none\"{} stroke-width=\"{}\" stroke-linejoin=\"round\" stroke-linecap=\"round\"{clip}/>",
                    color_attrs("stroke", c),
                    num(width)
                );
            }
        }
    }

    let mut out = String::with_capacity(defs.len() + body.len() + 256);
    let _ = write!(out, "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\">");
    let _ = write!(out, "<!-- AniCraft {} -->", env!("CARGO_PKG_VERSION"));
    if !defs.is_empty() {
        let _ = write!(out, "<defs>{defs}</defs>");
    }
    let bg = stage.background.sanitized();
    if bg.a > 0.0 {
        let _ = write!(out, "<rect width=\"{w}\" height=\"{h}\"{}/>", color_attrs("fill", bg));
    }
    out.push_str(&body);
    out.push_str("</svg>\n");
    out
}

/// The item's path in stage space, or `None` when it has no geometry or a broken transform.
fn stage_path(item: &DisplayItem) -> Option<String> {
    if !item.transform.is_finite() || item.path.is_empty() {
        return None;
    }
    let d = item.path.transformed(&item.transform).to_svg();
    // Huge coordinates can overflow in the transform; such a shape is skipped, never written.
    (!d.is_empty() && !d.contains("inf") && !d.contains("NaN")).then_some(d)
}

fn rule_attr(name: &str, rule: FillRule) -> String {
    match rule {
        FillRule::EvenOdd => format!(" {name}=\"evenodd\""),
        FillRule::NonZero => String::new(),
    }
}

/// A number written briefly (no exponents; non-finite values become 0).
fn num(v: f32) -> String {
    if !v.is_finite() {
        return "0".into();
    }
    let r = (v * 10_000.0).round() / 10_000.0;
    if r == 0.0 { "0".into() } else { format!("{r}") }
}

fn hex(c: Rgba) -> String {
    c.sanitized().to_hex()
}

/// `fill="#rrggbb"` (or `stroke=…`), plus the opacity when not opaque.
fn color_attrs(attr: &str, c: Rgba) -> String {
    let c = c.sanitized();
    if c.a >= 1.0 { format!(" {attr}=\"{}\"", hex(c)) } else { format!(" {attr}=\"{}\" {attr}-opacity=\"{}\"", hex(c), num(c.a)) }
}

/// The fill attribute for `paint`, defining a gradient in `defs` when needed. `None` when the
/// paint draws nothing.
fn paint_attr(paint: &Paint, xf: &Affine, fx: &ColorEffect, defs: &mut String, n: &mut usize) -> Option<String> {
    let stops = match paint {
        Paint::Solid(c) => {
            let c = fx.apply(*c).sanitized();
            return (c.a > 0.0).then(|| color_attrs("fill", c));
        }
        Paint::Linear { stops, .. } | Paint::Radial { stops, .. } => stops,
    };
    let mut sorted: Vec<_> = stops.iter().filter(|s| s.offset.is_finite()).copied().collect();
    if sorted.is_empty() {
        return None;
    }
    sorted.sort_by(|a, b| a.offset.total_cmp(&b.offset));
    let id = format!("g{n}");
    *n += 1;
    let m = xf.m;
    let gt = format!(" gradientUnits=\"userSpaceOnUse\" gradientTransform=\"matrix({} {} {} {} {} {})\"", num(m[0]), num(m[1]), num(m[2]), num(m[3]), num(m[4]), num(m[5]));
    match paint {
        Paint::Linear { start, end, .. } => {
            let _ = write!(defs, "<linearGradient id=\"{id}\"{gt} x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\">", num(start.x), num(start.y), num(end.x), num(end.y));
        }
        Paint::Radial { center, radius, .. } => {
            let _ = write!(defs, "<radialGradient id=\"{id}\"{gt} cx=\"{}\" cy=\"{}\" r=\"{}\">", num(center.x), num(center.y), num(radius.max(0.0)));
        }
        Paint::Solid(_) => return None,
    }
    for s in &sorted {
        let c = fx.apply(s.color).sanitized();
        let _ = write!(defs, "<stop offset=\"{}\" stop-color=\"{}\"", num(s.offset.clamp(0.0, 1.0)), hex(c));
        if c.a < 1.0 {
            let _ = write!(defs, " stop-opacity=\"{}\"", num(c.a));
        }
        defs.push_str("/>");
    }
    defs.push_str(if matches!(paint, Paint::Linear { .. }) { "</linearGradient>" } else { "</radialGradient>" });
    Some(format!(" fill=\"url(#{id})\""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use anicraft_doc::{ElementId, GradientStop, LayerId, Stroke, evaluate, sample};
    use anicraft_geom::{Path, Vec2};
    use std::sync::Arc;

    fn item(path: Path, fill: Option<Paint>) -> DisplayItem {
        DisplayItem {
            path: Arc::new(path),
            transform: Affine::IDENTITY,
            fill,
            stroke: None,
            fill_rule: FillRule::NonZero,
            color: ColorEffect::default(),
            layer: LayerId(0),
            element: ElementId(0),
            outline: None,
            clip: None,
            masks: None,
            guide: false,
        }
    }

    fn stage() -> Stage {
        Stage { width: 100, height: 50, fps: 24.0, background: Rgba::WHITE }
    }

    #[test]
    fn shapes_strokes_and_effects_come_out_in_stage_space() {
        let mut a = item(Path::rect(0.0, 0.0, 10.0, 10.0), Some(Paint::Solid(Rgba::hex(0xff0000))));
        a.transform = Affine::translate(5.0, 7.0).mul(&Affine::scale(2.0, 2.0));
        a.stroke = Some(Stroke { color: Rgba::BLACK, width: 3.0 });
        a.color = ColorEffect { alpha: 0.5, ..Default::default() };
        a.fill_rule = FillRule::EvenOdd;
        let svg = to_svg(&DisplayList { items: vec![a], clip_parents: vec![] }, &stage());
        assert!(svg.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"100\" height=\"50\" viewBox=\"0 0 100 50\">"));
        assert!(svg.contains("<rect width=\"100\" height=\"50\" fill=\"#ffffff\"/>"), "background");
        assert!(svg.contains("d=\"M5 7L25 7L25 27L5 27Z\" fill=\"#ff0000\" fill-opacity=\"0.5\" fill-rule=\"evenodd\""), "{svg}");
        assert!(svg.contains("stroke=\"#000000\" stroke-opacity=\"0.5\" stroke-width=\"6\""), "stroke width scales like the renderer");
        assert!(svg.trim_end().ends_with("</svg>"));
    }

    #[test]
    fn gradients_keep_local_geometry_and_masks_become_clip_paths() {
        let stops = vec![GradientStop { offset: 0.0, color: Rgba::BLACK }, GradientStop { offset: 1.0, color: Rgba::WHITE.with_alpha(0.25) }];
        let mut g = item(Path::rect(0.0, 0.0, 10.0, 10.0), Some(Paint::Linear { start: Vec2::new(0.0, 0.0), end: Vec2::new(10.0, 0.0), stops }));
        g.transform = Affine::translate(20.0, 0.0);
        g.clip = Some(1);
        let mut outer = item(Path::rect(0.0, 0.0, 50.0, 50.0), Some(Paint::Solid(Rgba::BLACK)));
        outer.masks = Some(0);
        let mut inner = item(Path::ellipse(25.0, 25.0, 10.0, 10.0), Some(Paint::Solid(Rgba::BLACK)));
        inner.masks = Some(1);
        let svg = to_svg(&DisplayList { items: vec![g, outer, inner], clip_parents: vec![None, Some(0)] }, &stage());
        assert!(svg.contains("gradientTransform=\"matrix(1 0 0 1 20 0)\" x1=\"0\" y1=\"0\" x2=\"10\" y2=\"0\""), "{svg}");
        assert!(svg.contains("<stop offset=\"1\" stop-color=\"#ffffff\" stop-opacity=\"0.25\"/>"));
        assert!(svg.contains("<clipPath id=\"c1\" clipPathUnits=\"userSpaceOnUse\" clip-path=\"url(#c0)\">"), "nested clip");
        assert!(svg.contains("clip-path=\"url(#c1)\"/>"), "the masked shape is clipped");
        assert_eq!(svg.matches("<path").count(), 3, "mask artwork is only in the clip paths");
    }

    #[test]
    fn the_sample_exports_deterministically_and_hostile_input_is_safe() {
        let d = sample::demo();
        let a = to_svg(&evaluate(&d, 10), &d.stage);
        assert_eq!(a, to_svg(&evaluate(&d, 10), &d.stage));
        assert!(a.matches("<path").count() >= 5);
        assert!(!a.contains("NaN") && !a.contains("inf"));
        // A broken transform, an empty gradient, an invisible fill and a malformed clip link.
        let mut bad = item(Path::rect(0.0, 0.0, 1.0, 1.0), Some(Paint::Solid(Rgba::BLACK)));
        bad.transform = Affine::scale(f32::NAN, 1.0);
        let empty = item(Path::rect(0.0, 0.0, 1.0, 1.0), Some(Paint::Radial { center: Vec2::new(0.0, 0.0), radius: f32::INFINITY, stops: vec![] }));
        let clear = item(Path::rect(0.0, 0.0, 1.0, 1.0), Some(Paint::Solid(Rgba::TRANSPARENT)));
        let mut lost = item(Path::rect(0.0, 0.0, 1.0, 1.0), Some(Paint::Solid(Rgba::BLACK)));
        lost.clip = Some(7);
        let svg = to_svg(&DisplayList { items: vec![bad, empty, clear, lost], clip_parents: vec![Some(3)] }, &stage());
        assert_eq!(svg.matches("<path").count(), 1, "{svg}");
        assert!(svg.contains("<clipPath id=\"c0\" clipPathUnits=\"userSpaceOnUse\"></clipPath>"), "a malformed clip clips everything");
        assert!(!svg.contains("NaN") && !svg.contains("inf"));
    }
}
