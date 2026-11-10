//! Text commands (ADR 0012): create and edit text blocks, list fonts. Laying text out happens
//! here, when it changes; the document keeps the outlines, so rendering never needs fonts.

use crate::params::*;
use crate::{CommandSpec, EngineError, Result, Session};
use anicraft_doc::{ColorEffect, Element, ElementId, ElementKind, Rgba, TextAlign, TextBlock};
use anicraft_geom::{Affine, Path};
use anicraft_text::{Align, Fonts};
use serde_json::{Value, json};
use std::sync::Arc;

/// The text style new text gets: the last one used, as in Animate.
#[derive(Clone, Debug, PartialEq)]
pub struct TextStyle {
    pub font: String,
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
    pub color: Rgba,
    pub align: TextAlign,
    pub letter_spacing: f32,
    pub line_spacing: f32,
}

impl Default for TextStyle {
    fn default() -> Self {
        TextStyle { font: String::new(), size: 32.0, bold: false, italic: false, color: Rgba::BLACK, align: TextAlign::Left, letter_spacing: 0.0, line_spacing: 0.0 }
    }
}


pub fn specs() -> Vec<CommandSpec> {
    let c = |id, label, menu, shortcut, params, mutates, run| CommandSpec { id, label, menu, shortcut, params, mutates, run };
    vec![
        c("text.create", "New Text", "", None, "{text, x?, y? (top-left; default: centred), font?, size?, bold?, italic?, color?, align?, letter_spacing?, line_spacing?, width?}", true, create as crate::RunFn),
        c("text.edit", "Edit Text", "", None, "{text?, font?, size? (px), bold?, italic?, color?, align?: left|center|right, letter_spacing?, line_spacing?, width? (px, wraps; null: one line per paragraph)} on the selected text", true, edit),
        c("text.fonts", "Fonts", "", None, "{}", false, fonts),
    ]
}

fn align_param(p: &Value) -> Result<Option<TextAlign>> {
    Ok(match opt_str(p, "align")? {
        None => None,
        Some("left") => Some(TextAlign::Left),
        Some("center" | "centre") => Some(TextAlign::Center),
        Some("right") => Some(TextAlign::Right),
        Some(other) => return Err(EngineError::Param(format!("'align' is left, center or right, not '{other}'"))),
    })
}

/// `width`: a positive number wraps, `null` doesn't, absent keeps `current`.
fn width_param(p: &Value, current: Option<f32>) -> Result<Option<f32>> {
    match p.get("width") {
        None => Ok(current),
        Some(Value::Null) => Ok(None),
        Some(_) => match opt_f32(p, "width")? {
            Some(w) if w > 0.0 => Ok(Some(w.min(100_000.0))),
            _ => Err(EngineError::Param("'width' must be a positive number of pixels, or null".into())),
        },
    }
}

/// The style in `p` over `base`.
fn style_param(p: &Value, base: &TextStyle) -> Result<TextStyle> {
    let size = opt_f32(p, "size")?.unwrap_or(base.size);
    if !(1.0..=2000.0).contains(&size) {
        return Err(EngineError::Param("'size' must be between 1 and 2000 px".into()));
    }
    let spacing = |key: &str, d: f32| -> Result<f32> {
        let v = opt_f32(p, key)?.unwrap_or(d);
        if (-1000.0..=1000.0).contains(&v) { Ok(v) } else { Err(EngineError::Param(format!("'{key}' must be between -1000 and 1000 px"))) }
    };
    Ok(TextStyle {
        font: opt_str(p, "font")?.map_or_else(|| base.font.clone(), |f| f.chars().take(256).collect()),
        size,
        bold: opt_bool(p, "bold")?.unwrap_or(base.bold),
        italic: opt_bool(p, "italic")?.unwrap_or(base.italic),
        color: opt_color(p, "color")?.unwrap_or(base.color),
        align: align_param(p)?.unwrap_or(base.align),
        letter_spacing: spacing("letter_spacing", base.letter_spacing)?,
        line_spacing: spacing("line_spacing", base.line_spacing)?,
    })
}

/// Lays a block out with this computer's fonts. Also says when the family asked for isn't
/// installed and another was used.
pub(crate) fn lay_out(text: &str, style: &TextStyle, width: Option<f32>) -> Result<(TextBlock, Option<String>)> {
    let fonts = Fonts::get();
    let layout = fonts
        .layout(
            text,
            &anicraft_text::Style {
                family: style.font.clone(),
                size: style.size,
                bold: style.bold,
                italic: style.italic,
                align: match style.align {
                    TextAlign::Left => Align::Left,
                    TextAlign::Center => Align::Center,
                    TextAlign::Right => Align::Right,
                },
                letter_spacing: style.letter_spacing,
                line_spacing: style.line_spacing,
                width,
            },
        )
        .map_err(|e| EngineError::State(format!("can't lay out text: {e}")))?;
    let note = (!style.font.trim().is_empty() && !layout.family.eq_ignore_ascii_case(style.font.trim())).then(|| format!("“{}” isn't installed on this computer; used {}", style.font.trim(), layout.family));
    let block = TextBlock {
        text: text.to_string(),
        font: layout.family,
        size: style.size,
        bold: style.bold,
        italic: style.italic,
        color: style.color,
        align: style.align,
        letter_spacing: style.letter_spacing,
        line_spacing: style.line_spacing,
        width,
        outline: Arc::new(layout.outline),
        box_width: layout.width,
        box_height: layout.height,
    };
    Ok((block, note))
}

fn style_of(t: &TextBlock) -> TextStyle {
    TextStyle { font: t.font.clone(), size: t.size, bold: t.bold, italic: t.italic, color: t.color, align: t.align, letter_spacing: t.letter_spacing, line_spacing: t.line_spacing }
}

fn create(s: &mut Session, p: &Value) -> Result<Value> {
    let text: String = str_req(p, "text")?.to_string();
    if text.trim().is_empty() {
        return Err(EngineError::Param("'text' is empty".into()));
    }
    let style = style_param(p, &s.text_style)?;
    let width = width_param(p, None)?;
    let (block, note) = lay_out(&text, &style, width)?;
    let x = opt_f32(p, "x")?.unwrap_or((s.doc.stage.width as f32 - block.box_width) * 0.5);
    let y = opt_f32(p, "y")?.unwrap_or((s.doc.stage.height as f32 - block.box_height) * 0.5);
    let id = ElementId(s.doc.alloc_id());
    let out = json!({"element": id.0, "font": block.font, "width": block.box_width, "height": block.box_height, "note": note});
    s.text_style = TextStyle { font: block.font.clone(), ..style };
    let el = Element { id, name: String::new(), transform: Affine::translate(x, y), color: ColorEffect::default(), kind: ElementKind::Text(Arc::new(block)) };
    s.editable_keyframe()?.elements.push(Arc::new(el));
    s.selection = vec![id];
    Ok(out)
}

fn edit(s: &mut Session, p: &Value) -> Result<Value> {
    let new_text = match opt_str(p, "text")? {
        Some(t) if t.trim().is_empty() => return Err(EngineError::Param("'text' is empty (delete the text instead)".into())),
        t => t.map(str::to_string),
    };
    let mut notes = Vec::new();
    let mut edited = 0;
    let mut last_style = None;
    for e in s.selected_elements_mut()? {
        let ElementKind::Text(t) = &e.kind else { continue };
        let style = style_param(p, &style_of(t))?;
        let width = width_param(p, t.width)?;
        let text = new_text.clone().unwrap_or_else(|| t.text.clone());
        let (block, note) = lay_out(&text, &style, width)?;
        notes.extend(note);
        last_style = Some(TextStyle { font: block.font.clone(), ..style });
        e.kind = ElementKind::Text(Arc::new(block));
        edited += 1;
    }
    if edited == 0 {
        return Err(EngineError::State("the selection has no text".into()));
    }
    if let Some(st) = last_style {
        s.text_style = st;
    }
    notes.dedup();
    Ok(json!({"edited": edited, "note": notes.first()}))
}

fn fonts(_: &mut Session, _: &Value) -> Result<Value> {
    let f = Fonts::get();
    Ok(json!({"families": f.families(), "default": f.resolve("")}))
}

/// A text block as a plain shape with the same outlines (Break Apart).
pub(crate) fn to_shape(t: &TextBlock) -> anicraft_doc::Shape {
    let path: Arc<Path> = t.outline.clone();
    anicraft_doc::Shape { path, fill: Some(anicraft_doc::Paint::Solid(t.color)), stroke: None, fill_rule: anicraft_geom::FillRule::NonZero, merge: false }
}
