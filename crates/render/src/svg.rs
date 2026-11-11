//! SVG output of a display list: the vector counterpart of [`crate::render`].
//!
//! It draws what the CPU renderer draws, from the same evaluated frame:
//! * paths are written in stage space, and strokes get the same stage-space width
//!   (`width × mean scale`) with their caps, joins and miter limit, and dashes scaled the same
//!   way (`stroke-dasharray`, `stroke-dashoffset`; dots are zero-length round-capped dashes),
//!   as the renderer measures them (ADR 0046);
//! * gradients keep their local geometry under a `gradientTransform`, padded at the ends;
//! * colour effects (every style, ADR 0056) are baked into every colour and stop, and drawn
//!   on bitmaps through an `feColorMatrix` (alpha alone as `fill-opacity`);
//! * masks become `clipPath`s (fill areas only, nested through `clip-path` on the clip), and an
//!   empty mask clips everything, as on the stage;
//! * outline-mode layers are hairlines in the layer colour;
//! * variable-width strokes are filled outlines: the polygons the renderer fills.
//!
//! The output is deterministic and contains only generated ids and numbers, so it needs no
//! escaping.

use anicraft_doc::{ColorTransform, DisplayItem, DisplayList, Paint, Rgba, Stage};
use anicraft_geom::{Affine, FillRule};
use std::fmt::Write;

/// Writes `dl` as a standalone SVG document the size of `stage`, with its background.
pub fn to_svg(dl: &DisplayList, stage: &Stage) -> String {
    // Variable-width strokes are written as the filled outlines the renderer draws.
    let dl = &*crate::stroke::outline_variable_strokes(dl);
    let (w, h) = (stage.width, stage.height);
    let mut defs = String::new();
    let mut body = String::new();
    let mut grad = 0usize;
    // Each bitmap is embedded once (by its pixels), then used by every fill that shows it.
    let mut images: Vec<(usize, bool, String)> = Vec::new();

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

    // Groups that fade as one become `<g opacity>` around their run of items.
    let chains = crate::group_chains(&dl.groups);
    let mut open: Vec<u32> = Vec::new();
    for item in dl.items.iter().filter(|it| it.masks.is_none() && !it.guide) {
        let Some(d) = stage_path(item) else { continue };
        let want = item.group.and_then(|g| chains.get(g as usize)).map_or(&[][..], Vec::as_slice);
        let keep = open.iter().zip(want).take_while(|(o, g)| o == g).count();
        while open.len() > keep {
            open.pop();
            body.push_str("</g>");
        }
        for &g in want.iter().skip(keep) {
            let a = dl.groups.get(g as usize).map_or(1.0, |x| x.alpha.clamp(0.0, 1.0));
            let style = match dl.groups.get(g as usize).map(|x| x.blend) {
                // CSS has these; Layer is the isolated group itself. The rest have no SVG
                // equivalent and draw normally.
                Some(m @ (anicraft_doc::BlendMode::Multiply | anicraft_doc::BlendMode::Screen | anicraft_doc::BlendMode::Overlay | anicraft_doc::BlendMode::Darken | anicraft_doc::BlendMode::Lighten | anicraft_doc::BlendMode::HardLight | anicraft_doc::BlendMode::Difference)) => {
                    format!(" style=\"mix-blend-mode:{}\"", if m == anicraft_doc::BlendMode::HardLight { "hard-light" } else { m.key() })
                }
                Some(anicraft_doc::BlendMode::Add) => " style=\"mix-blend-mode:plus-lighter\"".to_string(),
                _ => String::new(),
            };
            let filter = dl.groups.get(g as usize).and_then(|x| svg_filter(&x.filters, (w, h), &mut defs, &mut grad)).map_or_else(String::new, |id| format!(" filter=\"url(#{id})\""));
            let _ = write!(body, "<g opacity=\"{}\"{style}{filter}>", num(a));
            open.push(g);
        }
        let clip = item.clip.filter(|&c| (c as usize) < dl.clip_parents.len()).map(|c| format!(" clip-path=\"url(#c{c})\"")).unwrap_or_default();
        if let Some(color) = item.outline {
            // Outline mode: a hairline in the layer colour, no fill.
            let _ = write!(body, "<path d=\"{d}\" fill=\"none\"{} stroke-width=\"1\"{clip}/>", color_attrs("stroke", color));
            continue;
        }
        if let Some(fill) = &item.fill
            && let Some(paint) = paint_attr(fill, &item.transform, &item.color, &mut defs, &mut grad, item.image.as_ref().map(|px| (px, item.smooth)), &mut images)
        {
            let _ = write!(body, "<path d=\"{d}\"{paint}{}{clip}/>", rule_attr("fill-rule", item.fill_rule));
        }
        if let Some(st) = &item.stroke {
            let width = st.width * item.transform.mean_scale();
            if width.is_finite() && width > 0.0 {
                let c = item.color.apply(st.color);
                let _ = write!(
                    body,
                    "<path d=\"{d}\" fill=\"none\"{} stroke-width=\"{}\"{}{clip}/>",
                    color_attrs("stroke", c),
                    num(width),
                    style_attrs(st, item.transform.mean_scale())
                );
            }
        }
    }
    for _ in open {
        body.push_str("</g>");
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

/// A stroke's join, cap, miter limit and dashes as SVG attributes (dash lengths scaled by `k`,
/// like the width). The default style writes `stroke-linejoin="round" stroke-linecap="round"`.
fn style_attrs(st: &anicraft_doc::Stroke, k: f32) -> String {
    let style = &st.style;
    let mut out = format!(" stroke-linejoin=\"{}\" stroke-linecap=\"{}\"", style.join.key(), style.effective_cap().key());
    if style.join == anicraft_doc::Join::Miter {
        let _ = write!(out, " stroke-miterlimit=\"{}\"", num(style.miter_limit));
    }
    if let Some(dash) = &style.dash {
        let (lengths, offset) = dash.pattern(st.width);
        let list: Vec<String> = lengths.iter().map(|l| num(l * k)).collect();
        let _ = write!(out, " stroke-dasharray=\"{}\"", list.join(" "));
        if num(offset * k) != "0" {
            let _ = write!(out, " stroke-dashoffset=\"{}\"", num(offset * k));
        }
    }
    out
}

fn hex(c: Rgba) -> String {
    c.sanitized().to_hex()
}

/// `fill="#rrggbb"` (or `stroke=…`), plus the opacity when not opaque.
fn color_attrs(attr: &str, c: Rgba) -> String {
    let c = c.sanitized();
    if c.a >= 1.0 { format!(" {attr}=\"{}\"", hex(c)) } else { format!(" {attr}=\"{}\" {attr}-opacity=\"{}\"", hex(c), num(c.a)) }
}

/// An SVG `<filter>` for `filters` in `defs`, and its id; `None` when nothing carries across.
/// Blur becomes a Gaussian of the same spread as the box-blur passes; outer drop shadows and
/// glows become `feDropShadow`; Adjust Color becomes its colour matrix; bevels and gradient
/// filters are built from standard primitives ([`effect_svg`]). Inner and knockout shadows and
/// glows have no simple SVG equivalent and are left out.
fn svg_filter(filters: &[anicraft_doc::Filter], stage: (u32, u32), defs: &mut String, n: &mut usize) -> Option<String> {
    use anicraft_doc::Filter;
    let mut prims = String::new();
    for (i, f) in filters.iter().map(Filter::sanitized).enumerate() {
        match f {
            Filter::Blur { blur_x, blur_y, quality } => {
                let _ = write!(prims, "<feGaussianBlur stdDeviation=\"{} {}\"/>", num(sigma(blur_x, quality)), num(sigma(blur_y, quality)));
            }
            Filter::DropShadow { blur_x, blur_y, strength, quality, angle, distance, color, inner: false, knockout: false, hide_object: false } => {
                let a = angle.to_radians();
                let _ = write!(
                    prims,
                    "<feDropShadow dx=\"{}\" dy=\"{}\" stdDeviation=\"{} {}\" flood-color=\"{}\" flood-opacity=\"{}\"/>",
                    num(distance * a.cos()),
                    num(distance * a.sin()),
                    num(sigma(blur_x, quality)),
                    num(sigma(blur_y, quality)),
                    hex(color),
                    num((color.a * strength).clamp(0.0, 1.0))
                );
            }
            Filter::Glow { blur_x, blur_y, strength, quality, color, inner: false, knockout: false } => {
                let _ = write!(prims, "<feDropShadow dx=\"0\" dy=\"0\" stdDeviation=\"{} {}\" flood-color=\"{}\" flood-opacity=\"{}\"/>", num(sigma(blur_x, quality)), num(sigma(blur_y, quality)), hex(color), num((color.a * strength).clamp(0.0, 1.0)));
            }
            Filter::AdjustColor { brightness, contrast, saturation, hue } => {
                let m = crate::filters::color_matrix(brightness, contrast, saturation, hue);
                let values: Vec<String> = m.iter().flatten().map(|v| num(*v)).collect();
                let _ = write!(prims, "<feColorMatrix type=\"matrix\" values=\"{}\"/>", values.join(" "));
            }
            Filter::Bevel { .. } | Filter::GradientGlow { .. } | Filter::GradientBevel { .. } => effect_svg(&f, i, &mut prims),
            Filter::DropShadow { .. } | Filter::Glow { .. } => {}
        }
    }
    if prims.is_empty() {
        return None;
    }
    let id = format!("f{n}");
    *n += 1;
    // The region: the stage, grown by how far the filters reach (what can show, and what an
    // effect reaching in from outside needs), in stage units.
    let m = filters.iter().map(Filter::margin).sum::<f32>().min(1e6);
    let (sw, sh) = (stage.0 as f32, stage.1 as f32);
    let _ = write!(
        defs,
        "<filter id=\"{id}\" filterUnits=\"userSpaceOnUse\" x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" color-interpolation-filters=\"sRGB\">{prims}</filter>",
        num(-m),
        num(-m),
        num(sw + 2.0 * m),
        num(sh + 2.0 * m)
    );
    Some(id)
}

/// Steps in a ramp table: the CPU renderer's (`filters::RAMP_STEPS`).
const RAMP_SAMPLES: usize = crate::filters::RAMP_STEPS;

/// The Gaussian of the same spread as `q` box-blur passes of width `b` (a box of width `b` has
/// variance (b² − 1) / 12; passes add).
fn sigma(b: f32, q: u8) -> f32 {
    ((f32::from(q) * (b * b - 1.0).max(0.0)) / 12.0).sqrt()
}

/// Primitives for a bevel, gradient glow or gradient bevel (the `i`th filter): they take the
/// previous result (or the source) and leave the filtered image as the last result. The same
/// steps as the CPU renderer (`filters.rs`): the blurred opacity offset towards and away from
/// the light, and their difference (`feComposite` arithmetic) for a bevel's highlight and
/// shadow, or the offset blurred opacity for a glow's intensity, mapped through the ramp
/// (`feComponentTransfer` tables); then put atop the artwork (inner), under it (outer), both
/// (full), or alone (knockout).
fn effect_svg(f: &anicraft_doc::Filter, i: usize, prims: &mut String) {
    use anicraft_doc::{Filter, FilterSide};
    let (blur_x, blur_y, strength, quality, angle, distance, side, knockout) = match f {
        Filter::Bevel { blur_x, blur_y, strength, quality, angle, distance, side, knockout, .. }
        | Filter::GradientGlow { blur_x, blur_y, strength, quality, angle, distance, side, knockout, .. }
        | Filter::GradientBevel { blur_x, blur_y, strength, quality, angle, distance, side, knockout, .. } => (*blur_x, *blur_y, *strength, *quality, *angle, *distance, *side, *knockout),
        _ => return,
    };
    let a = angle.to_radians();
    let (dx, dy) = (num(distance * a.cos()), num(distance * a.sin()));
    let (ndx, ndy) = (num(-distance * a.cos()), num(-distance * a.sin()));
    let blur = format!("stdDeviation=\"{} {}\"", num(sigma(blur_x, quality)), num(sigma(blur_y, quality)));
    let s = num(strength);
    // The input, and its opacity alone.
    let _ = write!(prims, "<feOffset result=\"k{i}\"/><feColorMatrix in=\"k{i}\" type=\"matrix\" values=\"0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 1 0\" result=\"a{i}\"/>");
    // `stops` over the opacity of `from`, into `to`: opaque ramp colours from tables, cut by
    // the ramp's own opacity.
    let ramp = |prims: &mut String, stops: &[anicraft_doc::GradientStop], from: &str, to: &str| {
        // As many samples as the renderer's ramp table, so narrow stops come across.
        let table = |ch: fn(anicraft_doc::Rgba) -> f32| (0..=RAMP_SAMPLES).map(|j| num(ch(anicraft_doc::ramp_at(stops, j as f32 / RAMP_SAMPLES as f32)))).collect::<Vec<_>>().join(" ");
        let _ = write!(
            prims,
            "<feColorMatrix in=\"{from}\" type=\"matrix\" values=\"0 0 0 1 0 0 0 0 1 0 0 0 0 1 0 0 0 0 0 1\" result=\"{to}g\"/>\
             <feComponentTransfer in=\"{to}g\" result=\"{to}c\"><feFuncR type=\"table\" tableValues=\"{}\"/><feFuncG type=\"table\" tableValues=\"{}\"/><feFuncB type=\"table\" tableValues=\"{}\"/></feComponentTransfer>\
             <feComponentTransfer in=\"{from}\" result=\"{to}m\"><feFuncA type=\"table\" tableValues=\"{}\"/></feComponentTransfer>\
             <feComposite in=\"{to}c\" in2=\"{to}m\" operator=\"in\" result=\"{to}\"/>",
            table(|c| c.r),
            table(|c| c.g),
            table(|c| c.b),
            table(|c| c.a)
        );
    };
    // Both bevels: the blurred opacity towards the light (l) and away from it (d).
    let bevel_offsets = |prims: &mut String| {
        let _ = write!(prims, "<feGaussianBlur in=\"a{i}\" {blur} result=\"b{i}\"/><feOffset in=\"b{i}\" dx=\"{ndx}\" dy=\"{ndy}\" result=\"l{i}\"/><feOffset in=\"b{i}\" dx=\"{dx}\" dy=\"{dy}\" result=\"d{i}\"/>");
    };
    let (inner, outer) = match f {
        Filter::Bevel { shadow_color, highlight_color, .. } => {
            bevel_offsets(prims);
            let flood = |c: anicraft_doc::Rgba| format!("flood-color=\"{}\" flood-opacity=\"{}\"", hex(c), num(c.a));
            let _ = write!(
                prims,
                "<feComposite in=\"l{i}\" in2=\"d{i}\" operator=\"arithmetic\" k2=\"{s}\" k3=\"-{s}\" result=\"hm{i}\"/>\
                 <feComposite in=\"d{i}\" in2=\"l{i}\" operator=\"arithmetic\" k2=\"{s}\" k3=\"-{s}\" result=\"sm{i}\"/>\
                 <feFlood {}/><feComposite in2=\"hm{i}\" operator=\"in\" result=\"hc{i}\"/>\
                 <feFlood {}/><feComposite in2=\"sm{i}\" operator=\"in\" result=\"sc{i}\"/>\
                 <feComposite in=\"hc{i}\" in2=\"sc{i}\" operator=\"arithmetic\" k2=\"1\" k3=\"1\" result=\"e{i}\"/>",
                flood(*highlight_color),
                flood(*shadow_color)
            );
            (format!("e{i}"), format!("e{i}"))
        }
        Filter::GradientBevel { stops, .. } => {
            // The ramp position: 0.5 − strength × (towards − away) / 2.
            bevel_offsets(prims);
            let half = num(strength * 0.5);
            let _ = write!(prims, "<feComposite in=\"l{i}\" in2=\"d{i}\" operator=\"arithmetic\" k2=\"-{half}\" k3=\"{half}\" k4=\"0.5\" result=\"t{i}\"/>");
            ramp(prims, stops, &format!("t{i}"), &format!("e{i}"));
            (format!("e{i}"), format!("e{i}"))
        }
        Filter::GradientGlow { stops, .. } => {
            // Intensity: the blurred opacity (outside) or transparency (inside), offset.
            let glow = |prims: &mut String, src: &str, to: &str| {
                let _ = write!(prims, "<feGaussianBlur in=\"{src}\" {blur}/><feOffset dx=\"{dx}\" dy=\"{dy}\"/><feComposite operator=\"arithmetic\" k2=\"{s}\" result=\"{to}t\"/>");
                ramp(prims, stops, &format!("{to}t"), to);
            };
            if side != FilterSide::Outer {
                let _ = write!(prims, "<feComponentTransfer in=\"a{i}\" result=\"v{i}\"><feFuncA type=\"table\" tableValues=\"1 0\"/></feComponentTransfer>");
                glow(prims, &format!("v{i}"), &format!("ei{i}"));
            }
            if side != FilterSide::Inner {
                glow(prims, &format!("a{i}"), &format!("eo{i}"));
            }
            (format!("ei{i}"), format!("eo{i}"))
        }
        _ => return,
    };
    let _ = match (side, knockout) {
        (FilterSide::Inner, false) => write!(prims, "<feComposite in=\"{inner}\" in2=\"k{i}\" operator=\"atop\"/>"),
        (FilterSide::Inner, true) => write!(prims, "<feComposite in=\"{inner}\" in2=\"k{i}\" operator=\"in\"/>"),
        (FilterSide::Outer, false) => write!(prims, "<feComposite in=\"k{i}\" in2=\"{outer}\" operator=\"over\"/>"),
        (FilterSide::Outer, true) => write!(prims, "<feComposite in=\"{outer}\" in2=\"k{i}\" operator=\"out\"/>"),
        (FilterSide::Full, false) => write!(prims, "<feComposite in=\"{inner}\" in2=\"k{i}\" operator=\"atop\" result=\"x{i}\"/><feComposite in=\"x{i}\" in2=\"{outer}\" operator=\"over\"/>"),
        (FilterSide::Full, true) => write!(
            prims,
            "<feComposite in=\"{inner}\" in2=\"k{i}\" operator=\"in\" result=\"x{i}\"/><feComposite in=\"{outer}\" in2=\"k{i}\" operator=\"out\" result=\"y{i}\"/><feComposite in=\"x{i}\" in2=\"y{i}\" operator=\"arithmetic\" k2=\"1\" k3=\"1\"/>"
        ),
    };
}

/// The fill attribute for `paint`, defining a gradient in `defs` when needed. `None` when the
/// paint draws nothing.
fn paint_attr(paint: &Paint, xf: &Affine, fx: &ColorTransform, defs: &mut String, n: &mut usize, image: Option<(&std::sync::Arc<anicraft_doc::Pixels>, bool)>, images: &mut Vec<(usize, bool, String)>) -> Option<String> {
    let stops = match paint {
        Paint::Solid(c) => {
            let c = fx.apply(*c).sanitized();
            return (c.a > 0.0).then(|| color_attrs("fill", c));
        }
        Paint::Linear { stops, .. } | Paint::Radial { stops, .. } => stops,
        Paint::Bitmap { matrix, .. } => {
            // A pattern of the bitmap at its own pixel size, placed by the shape's transform
            // and the fill's image matrix.
            // Without Allow Smoothing (ADR 0050) the image keeps hard pixel edges.
            let (px, smooth) = image?;
            let key = std::sync::Arc::as_ptr(px) as usize;
            let img_id = match images.iter().find(|(k, sm, _)| *k == key && *sm == smooth) {
                Some((_, _, id)) => id.clone(),
                None => {
                    let png = crate::encode_png(px.width, px.height, &px.rgba).ok()?;
                    let id = format!("img{}", images.len());
                    let look = if smooth { "" } else { " image-rendering=\"pixelated\"" };
                    let _ = write!(defs, "<image id=\"{id}\" width=\"{}\" height=\"{}\"{look} href=\"data:image/png;base64,{}\"/>", px.width, px.height, anicraft_doc::base64_encode(&png));
                    images.push((key, smooth, id.clone()));
                    id
                }
            };
            let id = format!("p{n}");
            *n += 1;
            // A colour effect beyond alpha is a colour matrix on the image's pixels (ADR 0056):
            // the same multiply and offset per straight channel, clamped, as the renderer's.
            let fxs = fx.sanitized();
            let recolour = fxs.changes_colour() || fxs.add[3] != 0.0;
            let filter = if recolour {
                let fid = format!("cm{n}");
                *n += 1;
                let [mr, mg, mb, ma] = fxs.mul;
                let [ar, ag, ab, aa] = fxs.add;
                let _ = write!(
                    defs,
                    "<filter id=\"{fid}\" color-interpolation-filters=\"sRGB\"><feColorMatrix type=\"matrix\" values=\"{} 0 0 0 {} 0 {} 0 0 {} 0 0 {} 0 {} 0 0 0 {} {}\"/></filter>",
                    num(mr),
                    num(ar),
                    num(mg),
                    num(ag),
                    num(mb),
                    num(ab),
                    num(ma),
                    num(aa)
                );
                format!(" filter=\"url(#{fid})\"")
            } else {
                String::new()
            };
            let m = xf.mul(matrix).m;
            let _ = write!(
                defs,
                "<pattern id=\"{id}\" patternUnits=\"userSpaceOnUse\" patternTransform=\"matrix({} {} {} {} {} {})\" width=\"{}\" height=\"{}\"><use href=\"#{img_id}\"{filter}/></pattern>",
                num(m[0]),
                num(m[1]),
                num(m[2]),
                num(m[3]),
                num(m[4]),
                num(m[5]),
                px.width,
                px.height
            );
            let a = if recolour { 1.0 } else { fxs.mul[3].clamp(0.0, 1.0) };
            return Some(if a < 1.0 { format!(" fill=\"url(#{id})\" fill-opacity=\"{}\"", num(a)) } else { format!(" fill=\"url(#{id})\"") });
        }
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
        Paint::Solid(_) | Paint::Bitmap { .. } => return None,
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
            color: ColorTransform::default(),
            layer: LayerId(0),
            element: ElementId(0),
            outline: None,
            clip: None,
            masks: None,
            guide: false,
            group: None,
            image: None,
            smooth: true,
        }
    }

    fn stage() -> Stage {
        Stage { width: 100, height: 50, fps: 24.0, background: Rgba::WHITE }
    }

    #[test]
    fn bitmaps_take_colour_effects_as_a_colour_matrix() {
        // ADR 0056: alpha alone is fill-opacity; anything more is an feColorMatrix on the image.
        let px = Arc::new(anicraft_doc::Pixels { width: 1, height: 1, rgba: vec![10, 20, 30, 255] });
        let mut b = item(Path::rect(0.0, 0.0, 10.0, 10.0), Some(Paint::Bitmap { bitmap: anicraft_doc::BitmapId(1), repeat: true, matrix: Affine::IDENTITY }));
        b.image = Some(px);
        b.color = ColorTransform::alpha(0.5);
        let svg = to_svg(&DisplayList { items: vec![b.clone()], ..Default::default() }, &stage());
        assert!(svg.contains("fill-opacity=\"0.5\"") && !svg.contains("feColorMatrix"), "{svg}");
        b.color = ColorTransform { mul: [-1.0, 0.5, 1.0, 0.8], add: [1.0, 0.25, 0.0, 0.1] };
        let svg = to_svg(&DisplayList { items: vec![b], ..Default::default() }, &stage());
        assert!(svg.contains("<feColorMatrix type=\"matrix\" values=\"-1 0 0 0 1 0 0.5 0 0 0.25 0 0 1 0 0 0 0 0 0.8 0.1\"/>"), "{svg}");
        assert!(svg.contains("filter=\"url(#cm") && !svg.contains("fill-opacity"), "{svg}");
    }

    #[test]
    fn shapes_strokes_and_effects_come_out_in_stage_space() {
        let mut a = item(Path::rect(0.0, 0.0, 10.0, 10.0), Some(Paint::Solid(Rgba::hex(0xff0000))));
        a.transform = Affine::translate(5.0, 7.0).mul(&Affine::scale(2.0, 2.0));
        a.stroke = Some(Stroke::new(Rgba::BLACK, 3.0));
        a.color = ColorTransform::alpha(0.5);
        a.fill_rule = FillRule::EvenOdd;
        let svg = to_svg(&DisplayList { items: vec![a], clip_parents: vec![], groups: Vec::new() }, &stage());
        assert!(svg.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"100\" height=\"50\" viewBox=\"0 0 100 50\">"));
        assert!(svg.contains("<rect width=\"100\" height=\"50\" fill=\"#ffffff\"/>"), "background");
        assert!(svg.contains("d=\"M5 7L25 7L25 27L5 27Z\" fill=\"#ff0000\" fill-opacity=\"0.5\" fill-rule=\"evenodd\""), "{svg}");
        assert!(svg.contains("stroke=\"#000000\" stroke-opacity=\"0.5\" stroke-width=\"6\""), "stroke width scales like the renderer");
        assert!(svg.trim_end().ends_with("</svg>"));
    }

    #[test]
    fn stroke_caps_joins_and_dashes_are_written_scaled() {
        use anicraft_doc::{Cap, Dash, Join, StrokeStyle};
        let svg_of = |st: Stroke| {
            let mut a = item(Path::line(Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0)), None);
            a.transform = Affine::scale(2.0, 2.0);
            a.stroke = Some(st);
            to_svg(&DisplayList { items: vec![a], clip_parents: vec![], groups: Vec::new() }, &stage())
        };
        // The default style is written as before: round, no dashes.
        let svg = svg_of(Stroke::new(Rgba::BLACK, 3.0));
        assert!(svg.contains("stroke-width=\"6\" stroke-linejoin=\"round\" stroke-linecap=\"round\"/>"), "{svg}");
        assert!(!svg.contains("dasharray") && !svg.contains("miterlimit"));
        // Dashes, offset and the miter limit; lengths scale with the shape, like the width.
        let dashed = StrokeStyle { cap: Cap::Butt, join: Join::Miter, miter_limit: 6.0, dash: Some(Dash::Dashed { lengths: vec![4.0, 1.5].into(), offset: 1.0 }) };
        let svg = svg_of(Stroke::new(Rgba::BLACK, 3.0).with_style(dashed));
        assert!(svg.contains("stroke-linejoin=\"miter\" stroke-linecap=\"butt\" stroke-miterlimit=\"6\" stroke-dasharray=\"8 3\" stroke-dashoffset=\"2\"/>"), "{svg}");
        // Dots: zero-length round dashes one width plus the spacing apart.
        let dotted = StrokeStyle { cap: Cap::Square, join: Join::Bevel, dash: Dash::dotted(2.0), ..StrokeStyle::default() };
        let svg = svg_of(Stroke::new(Rgba::BLACK, 3.0).with_style(dotted));
        assert!(svg.contains("stroke-linejoin=\"bevel\" stroke-linecap=\"round\" stroke-dasharray=\"0 10\"/>"), "{svg}");
    }

    #[test]
    fn variable_width_strokes_are_written_as_filled_outlines() {
        let mut a = item(Path::line(Vec2::new(0.0, 10.0), Vec2::new(80.0, 10.0)), Some(Paint::Solid(Rgba::hex(0xff0000))));
        a.stroke = Some(Stroke::new(Rgba::hex(0x0000ff), 6.0).with_profile(anicraft_doc::ProfilePreset::TaperBoth.profile()));
        let svg = to_svg(&DisplayList { items: vec![a], clip_parents: vec![], groups: Vec::new() }, &stage());
        assert!(!svg.contains("stroke-width"), "no SVG stroke: {svg}");
        assert!(svg.contains("fill=\"#0000ff\""), "the outline is filled in the stroke colour");
        assert!(svg.contains("fill=\"#ff0000\""), "the shape's own fill stays");
        assert!(!svg.contains("NaN") && !svg.contains("inf"));
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
        let svg = to_svg(&DisplayList { items: vec![g, outer, inner], clip_parents: vec![None, Some(0)], groups: Vec::new() }, &stage());
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
        let svg = to_svg(&DisplayList { items: vec![bad, empty, clear, lost], clip_parents: vec![Some(3)], groups: Vec::new() }, &stage());
        assert_eq!(svg.matches("<path").count(), 1, "{svg}");
        assert!(svg.contains("<clipPath id=\"c0\" clipPathUnits=\"userSpaceOnUse\"></clipPath>"), "a malformed clip clips everything");
        assert!(!svg.contains("NaN") && !svg.contains("inf"));
    }
}

#[cfg(test)]
mod group_tests {
    use super::*;
    use anicraft_doc::{ElementId, Group, LayerId};
    use anicraft_geom::Path;
    use std::sync::Arc;

    #[test]
    fn groups_become_opacity_groups_around_their_run() {
        let it = |g: Option<u32>| DisplayItem {
            path: Arc::new(Path::rect(0.0, 0.0, 4.0, 4.0)),
            transform: Affine::IDENTITY,
            fill: Some(Paint::Solid(Rgba::BLACK)),
            stroke: None,
            fill_rule: FillRule::NonZero,
            color: ColorTransform::default(),
            layer: LayerId(0),
            element: ElementId(0),
            outline: None,
            clip: None,
            masks: None,
            guide: false,
            group: g,
            image: None,
            smooth: true,
        };
        let dl = DisplayList { items: vec![it(None), it(Some(0)), it(Some(1)), it(Some(0)), it(None)], clip_parents: vec![], groups: vec![Group { parent: None, alpha: 0.5, blend: Default::default(), filters: Default::default(), clip: None }, Group { parent: Some(0), alpha: 0.25, blend: Default::default(), filters: Default::default(), clip: None }] };
        let svg = to_svg(&dl, &Stage { width: 4, height: 4, fps: 24.0, background: Rgba::WHITE });
        assert!(svg.contains("<g opacity=\"0.5\">") && svg.contains("<g opacity=\"0.25\">"), "{svg}");
        assert_eq!(svg.matches("<g ").count(), svg.matches("</g>").count(), "balanced");
        // path, open 0.5, path, open 0.25, path, close, path, close, path
        let order: Vec<&str> = svg.split('<').filter_map(|t| t.split([' ', '>']).next()).filter(|t| ["path", "g", "/g"].contains(t)).collect();
        assert_eq!(order, ["path", "g", "path", "g", "path", "/g", "path", "/g", "path"], "{svg}");
    }
}
