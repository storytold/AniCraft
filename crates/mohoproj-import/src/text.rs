//! `TextLayer` → a text element laid out with this computer's fonts (ADR 0014), as the XFL
//! importer does for Animate fields.
//!
//! Moho regenerates a text layer's geometry from its text, font and size at render time (the
//! saved `mesh_layer` is a derived view), so re-laying the text out here matches its intent.
//! Units: `textsize` is points at Moho's 72 pt/inch → pixels at the document's dpi. The
//! justification code maps 1/2/3 → left/centre/right (Moho's default 1 is left-aligned
//! text). Text strokes, kerning and leading, inherited styles and speech balloons have no
//! slot in a text block yet and are counted when non-default.

use crate::import::Builder;
use crate::json::{self, get};
use anicraft_doc::{ColorEffect, Element, ElementId, ElementKind, Rgba, TextAlign, TextBlock};
use anicraft_geom::{Affine, Vec2};
use anicraft_text::{Align, FaceMatch, Fonts, TextError};
use serde_json::Value;
use std::sync::Arc;

pub(crate) fn text_elements(b: &mut Builder, layer: &Value, world: Affine, alpha: f32, out: &mut Vec<Arc<Element>>) {
    let Some(txt) = json::string(get(layer, "text")).filter(|t| !t.trim().is_empty()) else { return };
    if !json::boolean(get(layer, "fill")).unwrap_or(true) {
        b.report.skip("text drawn as stroke only");
        return;
    }
    extras(b, layer);
    let size_pt = json::num(get(layer, "textsize")).unwrap_or(60.0);
    let font = json::string(get(layer, "font")).unwrap_or_default();
    let color = json::color255(get(layer, "fillcolor")).unwrap_or(Rgba::BLACK);
    let align = match json::integer(get(layer, "justification")).unwrap_or(1) {
        2 => TextAlign::Center,
        3 => TextAlign::Right,
        _ => TextAlign::Left,
    };
    // Points at Moho's 72 pt/inch, shown at the document's pixels per inch.
    let size = ((size_pt / 72.0) * b.dpi as f64).clamp(1.0, 2000.0) as f32;
    let fonts: &Fonts = match b.fonts {
        Some(f) => f,
        None => Fonts::get(),
    };
    b.fonts = Some(fonts);
    let face: FaceMatch = b.faces.entry(font.clone()).or_insert_with(|| fonts.find_face(&font)).clone();
    let style = anicraft_text::Style {
        family: face.family.clone().unwrap_or_default(),
        size,
        bold: face.bold,
        italic: face.italic,
        align: match align {
            TextAlign::Left => Align::Left,
            TextAlign::Center => Align::Center,
            TextAlign::Right => Align::Right,
        },
        letter_spacing: 0.0,
        line_spacing: 0.0,
        width: None,
    };
    let layout = match fonts.layout(&txt, &style) {
        Ok(l) => l,
        Err(TextError::NoFonts) => {
            b.report.skip("text (no fonts on this computer)");
            return;
        }
        Err(TextError::TooLong) => {
            b.report.skip("text cut to 100,000 characters");
            return;
        }
    };
    // A family that isn't installed falls back to the default, as the text engine does
    // (ADR 0012), and the report names it.
    if face.family.is_none() && !face.wanted.is_empty() {
        b.report.missing_fonts.insert(face.wanted.clone());
        b.report.fallback_font.get_or_insert_with(|| layout.family.clone());
    }
    let block = TextBlock {
        text: txt,
        font: layout.family,
        size: style.size,
        bold: style.bold,
        italic: style.italic,
        color,
        align,
        letter_spacing: 0.0,
        line_spacing: 0.0,
        width: None,
        outline: Arc::new(layout.outline),
        box_width: layout.width,
        box_height: layout.height,
    };
    // The block's top-left sits at the layer origin, carried by the px-space form of the
    // layer's world transform (so a rotated parent layer rotates the text with it).
    let at = b.to_px.apply(world.apply(Vec2::ZERO));
    let linear = b.to_px.mul(&world).mul(&b.from_px);
    let transform = Affine { m: [linear.m[0], linear.m[1], linear.m[2], linear.m[3], at.x, at.y] };
    out.push(Arc::new(Element {
        id: ElementId(b.doc.alloc_id()),
        name: String::new(),
        transform,
        color: ColorEffect { alpha: alpha.clamp(0.0, 1.0), ..ColorEffect::default() },
        kind: ElementKind::Text(Arc::new(block)),
    }));
}

/// Text features without a slot yet, counted when non-default. The embedded `mesh_layer` is
/// Moho's derived geometry and simply ignored (the fonts re-shape the text here).
fn extras(b: &mut Builder, layer: &Value) {
    if json::boolean(get(layer, "stroke")).unwrap_or(false) {
        b.report.skip("text strokes (drawn fill only)");
    }
    if json::integer(get(layer, "kerning")).is_some_and(|k| k != 0) {
        b.report.skip("text kerning (drawn as 0)");
    }
    if json::integer(get(layer, "leading")).is_some_and(|l| l != 0) {
        b.report.skip("text leading (drawn as 0)");
    }
    if !json::string(get(layer, "textinheritedstyle1")).is_none_or(|s| s.is_empty())
        || !json::string(get(layer, "textinheritedstyle2")).is_none_or(|s| s.is_empty())
    {
        b.report.skip("inherited text styles");
    }
    if !json::string(get(layer, "balloonstyle")).unwrap_or_default().is_empty() {
        b.report.skip("speech balloons");
    }
}
