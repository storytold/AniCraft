//! Which Properties rows say "mixed" for a selection of several elements (ADR 0053).
//!
//! A row is mixed when the selected elements it applies to differ there: the field shows the
//! first one's value, and a change applies to all of them. Each row compares only the elements
//! it edits (Blending only movie clips and buttons, Looping only graphics, Fill only shapes), so
//! a shape beside a movie clip never makes Blending mixed. Pure, so every row is tested here.

use anicraft_engine::doc::{ColorEffect, Document, Element, ElementKind, Paint, Shape, SymbolKind};
use std::sync::Arc;

/// The rows whose values differ across the selection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mixed {
    pub scale: bool,
    pub rotation: bool,
    /// The colour effect's rows (ADR 0056).
    pub color: ColorMixed,
    /// The instances show different symbols (the Symbol row, Swap…).
    pub symbol: bool,
    /// Their symbols are of different types.
    pub kind: bool,
    pub filters: bool,
    pub blend: bool,
    pub looping: bool,
    pub first_frame: bool,
    pub fill: bool,
    /// The bitmap a bitmap fill shows (Swap Bitmap).
    pub bitmap: bool,
    /// Stroke colour or width.
    pub stroke: bool,
    pub profile: bool,
    /// Line style, caps, joins and dashes.
    pub style: bool,
}

/// Which colour effect rows differ. Each value compares the elements as the first one's style
/// sees them (an element in another style counts as that style's value it would start from),
/// which is what a change sets on all of them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ColorMixed {
    pub style: bool,
    pub alpha: bool,
    pub brightness: bool,
    pub tint: bool,
    pub tint_amount: bool,
    /// Red, green, blue, alpha multipliers, then their offsets.
    pub advanced: [bool; 8],
}

fn color_mixed(all: &[Arc<Element>]) -> ColorMixed {
    let Some(first) = all.first() else { return ColorMixed::default() };
    let style = first.color.style();
    let seen: Vec<ColorEffect> = all.iter().map(|e| e.color.to_style(style)).collect();
    let mut m = ColorMixed { style: differ(all.iter().map(|e| e.color.style())), ..ColorMixed::default() };
    match style {
        anicraft_engine::doc::ColorStyle::None => {}
        anicraft_engine::doc::ColorStyle::Alpha => m.alpha = differ_f32(seen.iter().map(ColorEffect::alpha)),
        anicraft_engine::doc::ColorStyle::Brightness => {
            m.brightness = differ_f32(seen.iter().map(|f| if let ColorEffect::Brightness(b) = f { *b } else { 0.0 }));
        }
        anicraft_engine::doc::ColorStyle::Tint => {
            let tints: Vec<_> = seen.iter().filter_map(|f| if let ColorEffect::Tint { color, amount } = f { Some((*color, *amount)) } else { None }).collect();
            m.tint = differ(tints.iter().map(|t| t.0.to_hex()));
            m.tint_amount = differ_f32(tints.iter().map(|t| t.1));
        }
        anicraft_engine::doc::ColorStyle::Advanced => {
            let ts: Vec<_> = seen.iter().map(ColorEffect::transform).collect();
            for (i, slot) in m.advanced.iter_mut().enumerate() {
                *slot = differ_f32(ts.iter().map(|t| if i < 4 { t.mul.get(i).copied().unwrap_or(1.0) } else { t.add.get(i - 4).copied().unwrap_or(0.0) }));
            }
        }
    }
    m
}

/// Whether `values` holds two that differ.
fn differ<T: PartialEq>(mut values: impl Iterator<Item = T>) -> bool {
    match values.next() {
        Some(first) => values.any(|v| v != first),
        None => false,
    }
}

/// Close enough to show the same number in a field.
fn near(a: f32, b: f32) -> bool {
    (a - b).abs() <= 1e-4
}

fn differ_f32(values: impl Iterator<Item = f32>) -> bool {
    let v: Vec<f32> = values.collect();
    v.first().is_some_and(|f| v.iter().any(|x| !near(*x, *f)))
}

/// What Properties shows as mixed for `all` (the selection, in order).
pub fn of(doc: &Document, all: &[Arc<Element>]) -> Mixed {
    if all.len() < 2 {
        return Mixed::default();
    }
    let kind_of = |e: &Element| match &e.kind {
        ElementKind::Symbol { symbol, .. } => doc.symbol(*symbol).map(|s| s.kind),
        _ => None,
    };
    let decomposed: Vec<_> = all.iter().map(|e| e.transform.decompose()).collect();
    let instances = || all.iter().filter(|e| matches!(e.kind, ElementKind::Symbol { .. }));
    // Blending and filters: movie clips and buttons (Animate offers them on those only).
    let clips = || all.iter().filter(|e| kind_of(e).is_some_and(|k| k != SymbolKind::Graphic));
    let graphics = || all.iter().filter(|e| kind_of(e) == Some(SymbolKind::Graphic));
    let shapes: Vec<&Shape> = all
        .iter()
        .filter_map(|e| match &e.kind {
            ElementKind::Shape(s) => Some(s),
            _ => None,
        })
        .collect();
    let strokes = || shapes.iter().filter_map(|s| s.stroke.as_ref());
    Mixed {
        scale: differ_f32(decomposed.iter().map(|d| d.sx)) || differ_f32(decomposed.iter().map(|d| d.sy)),
        rotation: differ_f32(decomposed.iter().map(|d| d.rotation)),
        color: color_mixed(all),
        symbol: differ(instances().filter_map(|e| match &e.kind {
            ElementKind::Symbol { symbol, .. } => Some(*symbol),
            _ => None,
        })),
        kind: differ(instances().map(|e| kind_of(e))),
        filters: differ(clips().filter_map(|e| match &e.kind {
            ElementKind::Symbol { filters, .. } => Some(filters),
            _ => None,
        })),
        blend: differ(clips().filter_map(|e| match &e.kind {
            ElementKind::Symbol { blend, .. } => Some(*blend),
            _ => None,
        })),
        looping: differ(graphics().filter_map(|e| match &e.kind {
            ElementKind::Symbol { loop_mode, .. } => Some(*loop_mode),
            _ => None,
        })),
        first_frame: differ(graphics().filter_map(|e| match &e.kind {
            ElementKind::Symbol { first_frame, .. } => Some(*first_frame),
            _ => None,
        })),
        fill: differ(shapes.iter().map(|s| s.fill.as_ref())),
        bitmap: differ(shapes.iter().filter_map(|s| match &s.fill {
            Some(Paint::Bitmap { bitmap, .. }) => Some(*bitmap),
            _ => None,
        })),
        // A shape without a line differs from one with a line.
        stroke: differ(shapes.iter().map(|s| s.stroke.as_ref().map(|st| (st.color, st.width.to_bits())))),
        profile: differ(strokes().map(|st| st.profile.as_ref())),
        style: differ(strokes().map(|st| &st.style)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anicraft_engine::Session;
    use anicraft_engine::doc::sample;
    use serde_json::json;

    fn run(s: &mut Session, id: &str, p: serde_json::Value) -> serde_json::Value {
        s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
    }

    fn id(v: &serde_json::Value) -> u64 {
        v["element"].as_u64().unwrap()
    }

    /// The selection's elements, in order, from every layer.
    fn selected(s: &Session) -> Vec<Arc<Element>> {
        s.selection.iter().filter_map(|(l, e)| s.elements_on(l).iter().find(|x| x.id == e).cloned()).collect()
    }

    fn mixed(s: &Session) -> Mixed {
        of(&s.doc, &selected(s))
    }

    fn select(s: &mut Session, ids: &[u64]) {
        run(s, "element.select", json!({"ids": ids}));
    }

    /// Two movie clips and a graphic of the sample's Ball and Star, on two layers, and two shapes.
    fn session() -> (Session, [u64; 3], [u64; 2]) {
        let mut s = Session::new(sample::demo());
        run(&mut s, "edit.deselect", json!({}));
        let sym = |s: &Session, n: &str| s.doc.library.iter().find(|x| x.name == n).unwrap().id.0;
        let (ball, star) = (sym(&s, "Ball"), sym(&s, "Star"));
        let a = id(&run(&mut s, "library.place", json!({"symbol": ball, "x": 100, "y": 100})));
        let b = id(&run(&mut s, "library.place", json!({"symbol": ball, "x": 200, "y": 100})));
        run(&mut s, "layer.new", json!({}));
        let g = id(&run(&mut s, "library.place", json!({"symbol": star, "x": 300, "y": 100})));
        run(&mut s, "symbol.set_kind", json!({"symbol": ball, "kind": "movie_clip"}));
        run(&mut s, "symbol.set_kind", json!({"symbol": star, "kind": "graphic"}));
        let r1 = id(&run(&mut s, "draw.rect", json!({"x": 10, "y": 300, "w": 20, "h": 20, "fill": "#ff0000", "stroke": "#000000", "stroke_width": 2, "object": true})));
        let r2 = id(&run(&mut s, "draw.rect", json!({"x": 60, "y": 300, "w": 20, "h": 20, "fill": "#ff0000", "stroke": "#000000", "stroke_width": 2, "object": true})));
        (s, [a, b, g], [r1, r2])
    }

    #[test]
    fn alike_elements_are_not_mixed_and_one_element_never_is() {
        let (mut s, [a, b, g], [r1, r2]) = session();
        select(&mut s, &[a, b, g, r1, r2]);
        // Positions differ, and so do the instances' symbols and types (Ball and Star): those
        // rows say mixed; nothing else does.
        assert_eq!(mixed(&s), Mixed { symbol: true, kind: true, ..Mixed::default() });
        select(&mut s, &[a, b]);
        assert_eq!(mixed(&s), Mixed::default());
        select(&mut s, &[a]);
        assert_eq!(mixed(&s), Mixed::default());
    }

    #[test]
    fn scale_rotation_and_alpha_rows() {
        let (mut s, [a, b, g], _) = session();
        select(&mut s, &[b]);
        run(&mut s, "element.set_transform", json!({"sx": 2.0}));
        select(&mut s, &[a, b, g]);
        assert!(mixed(&s).scale && !mixed(&s).rotation && mixed(&s).color == ColorMixed::default());
        select(&mut s, &[g]);
        run(&mut s, "element.set_transform", json!({"rotation": 30}));
        run(&mut s, "element.set_alpha", json!({"alpha": 0.5}));
        select(&mut s, &[a, g]);
        let m = mixed(&s);
        assert!(m.rotation && m.color.style && !m.scale);
        // Editing a mixed row sets all: then it isn't mixed any more.
        run(&mut s, "element.set_alpha", json!({"alpha": 0.25}));
        run(&mut s, "element.set_transform", json!({"rotation": 0}));
        assert!(mixed(&s).color == ColorMixed::default() && !mixed(&s).rotation);
    }

    #[test]
    fn colour_effect_rows_compare_in_the_first_ones_style() {
        let (mut s, [a, b, g], _) = session();
        select(&mut s, &[a]);
        run(&mut s, "element.set_color_effect", json!({"tint": "#ff0000", "tint_amount": 50}));
        select(&mut s, &[b]);
        run(&mut s, "element.set_color_effect", json!({"tint": "#ff0000", "tint_amount": 20}));
        select(&mut s, &[a, b]);
        let m = mixed(&s).color;
        assert!(!m.style && !m.tint && m.tint_amount);
        // An element in another style differs in style, and counts as a tint of 0 % of white.
        select(&mut s, &[a, g]);
        let m = mixed(&s).color;
        assert!(m.style && m.tint && m.tint_amount);
        // Setting the amount on all makes them alike tints of their own colours.
        run(&mut s, "element.set_color_effect", json!({"tint_amount": 30}));
        let m = mixed(&s).color;
        assert!(!m.style && !m.tint_amount && m.tint);
        // Advanced: only the channel that differs.
        select(&mut s, &[a, b]);
        run(&mut s, "element.set_color_effect", json!({"tint_amount": 30}));
        run(&mut s, "element.set_color_effect", json!({"style": "advanced"}));
        assert_eq!(mixed(&s).color, ColorMixed::default(), "the same tint, as Advanced");
        select(&mut s, &[b]);
        run(&mut s, "element.set_color_effect", json!({"green_offset": 40}));
        select(&mut s, &[a, b]);
        assert_eq!(mixed(&s).color.advanced, [false, false, false, false, false, true, false, false]);
    }

    #[test]
    fn filters_and_blending_rows_compare_movie_clips_only() {
        let (mut s, [a, b, g], [r1, _]) = session();
        select(&mut s, &[b]);
        run(&mut s, "element.set_blend", json!({"mode": "multiply"}));
        run(&mut s, "element.set_filters", json!({"filters": [{"type": "blur"}]}));
        select(&mut s, &[a, b, g, r1]);
        let m = mixed(&s);
        assert!(m.blend && m.filters);
        // A graphic and a shape beside one movie clip: nothing to compare.
        select(&mut s, &[b, g, r1]);
        let m = mixed(&s);
        assert!(!m.blend && !m.filters);
        // Setting either across the selection makes the movie clips alike.
        select(&mut s, &[a, b, g]);
        run(&mut s, "element.set_blend", json!({"mode": "screen"}));
        run(&mut s, "element.set_filters", json!({"filters": []}));
        let m = mixed(&s);
        assert!(!m.blend && !m.filters);
    }

    #[test]
    fn looping_and_first_frame_rows_compare_graphics_only() {
        let (mut s, [a, _, g], _) = session();
        let star = s.doc.library.iter().find(|x| x.name == "Star").unwrap().id.0;
        let g2 = id(&run(&mut s, "library.place", json!({"symbol": star, "x": 400, "y": 100})));
        select(&mut s, &[g2]);
        run(&mut s, "element.set_loop", json!({"loop": "single_frame", "first_frame": 1}));
        select(&mut s, &[g, g2, a]);
        let m = mixed(&s);
        assert!(m.looping && m.first_frame);
        run(&mut s, "element.set_loop", json!({"loop": "play_once", "first_frame": 0}));
        let m = mixed(&s);
        assert!(!m.looping && !m.first_frame);
    }

    #[test]
    fn symbol_and_type_rows_and_editing_the_type_sets_every_symbol() {
        let (mut s, [a, b, g], _) = session();
        select(&mut s, &[a, b]);
        assert!(!mixed(&s).symbol && !mixed(&s).kind);
        select(&mut s, &[a, g]);
        assert!(mixed(&s).symbol && mixed(&s).kind);
        // Type: every selected instance's symbol, in one undo step.
        let v = run(&mut s, "symbol.set_kind", json!({"kind": "button"}));
        assert_eq!(v["symbols"].as_array().map(Vec::len), Some(2));
        assert!(!mixed(&s).kind && mixed(&s).symbol);
        s.undo().unwrap();
        assert!(mixed(&s).kind);
        // Swap Symbol sets them all too.
        let ball = s.doc.library.iter().find(|x| x.name == "Ball").unwrap().id.0;
        run(&mut s, "element.swap_symbol", json!({"symbol": ball}));
        assert!(!mixed(&s).symbol);
    }

    #[test]
    fn fill_and_bitmap_rows() {
        let (mut s, _, [r1, r2]) = session();
        select(&mut s, &[r2]);
        run(&mut s, "element.set_fill", json!({"color": "#00ff00"}));
        select(&mut s, &[r1, r2]);
        assert!(mixed(&s).fill);
        run(&mut s, "element.set_fill", json!({"color": "#0000ff"}));
        assert!(!mixed(&s).fill);
        // Fill on beside fill off is mixed.
        select(&mut s, &[r1]);
        run(&mut s, "element.set_fill", json!({"color": null}));
        select(&mut s, &[r1, r2]);
        assert!(mixed(&s).fill);
        // Two placed bitmaps of different images.
        let mut ids = Vec::new();
        for (n, w) in [("a.png", 2), ("b.png", 3)] {
            let b = anicraft_engine::doc::BitmapId(s.doc.alloc_id());
            let png = anicraft_engine::render::encode_png(w, 2, &[0, 0, 255, 255].repeat(w as usize * 2)).unwrap();
            s.doc.bitmaps.push(Arc::new(anicraft_engine::doc::Bitmap::from_png(b, n, png).unwrap()));
            ids.push((b, id(&run(&mut s, "library.place", json!({"bitmap": b.0})))));
        }
        select(&mut s, &[ids[0].1, ids[1].1]);
        assert!(mixed(&s).bitmap && mixed(&s).fill);
        run(&mut s, "element.swap_bitmap", json!({"bitmap": ids[0].0.0}));
        assert!(!mixed(&s).bitmap);
    }

    #[test]
    fn stroke_profile_and_style_rows() {
        let (mut s, _, [r1, r2]) = session();
        select(&mut s, &[r2]);
        run(&mut s, "element.set_stroke", json!({"width": 6}));
        select(&mut s, &[r1, r2]);
        assert!(mixed(&s).stroke && !mixed(&s).profile && !mixed(&s).style);
        run(&mut s, "element.set_stroke", json!({"width": 3}));
        assert!(!mixed(&s).stroke);
        select(&mut s, &[r1]);
        run(&mut s, "stroke.set_profile", json!({"profile": "taper_both"}));
        run(&mut s, "element.set_stroke", json!({"style": "dashed"}));
        select(&mut s, &[r1, r2]);
        let m = mixed(&s);
        assert!(m.profile && m.style && !m.stroke);
        run(&mut s, "stroke.set_profile", json!({"profile": "taper_both"}));
        run(&mut s, "element.set_stroke", json!({"style": "dashed"}));
        let m = mixed(&s);
        assert!(!m.profile && !m.style);
    }
}
