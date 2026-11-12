//! Text commands (ADR 0012): create and edit text blocks, list fonts. Laying text out happens
//! here, when it changes; the document keeps the outlines, so rendering never needs fonts.
//!
//! Dynamic and input text fields (ADR 0066): `text.field` sets a block's Text type and field
//! options. A field's box is fixed and its text is laid out inside the gutter, aligned in the box;
//! [`field_layout`] does that for the commands and for the player while a movie plays.

use crate::params::*;
use crate::{CommandSpec, EngineError, Result, Session};
use anicraft_doc::{ColorEffect, Element, ElementId, ElementKind, FIELD_GUTTER, LineType, MAX_FIELD_CHARS, MAX_FIELD_NAME, Rgba, TextAlign, TextBlock, TextField, TextKind};
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
        c("text.create", "New Text", "", None, "{text (may be empty for an input field), type?: static|dynamic|input, x?, y? (top-left; default: centred), font?, size?, bold?, italic?, color?, align?, letter_spacing?, line_spacing?, width?}", true, create as crate::RunFn),
        c("text.edit", "Edit Text", "", None, "{text?, font?, size? (px), bold?, italic?, color?, align?: left|center|right, letter_spacing?, line_spacing?, width? (px, wraps; null: one line per paragraph)} on the selected text", true, |s, p| s.each_layer(false, |s| edit(s, p))),
        c("text.fonts", "Fonts", "", None, "{}", false, fonts),
        c("text.field", "Text Type", "", None, "{type?: static|dynamic|input, lines?: single|multiline|multiline_no_wrap|password (input only), width?, height? (the field's box, px), selectable?, border?, background?, border_color?, background_color?, max_chars? (input; 0: no limit), variable? (kept, not run)} on the selected text; dynamic and input fields are named with instance.set_name and scripted in Test Movie and the HTML export (ADR 0066)", true, |s, p| s.each_layer(false, |s| set_field(s, p))),
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
        field: None,
    };
    Ok((block, note))
}

/// A field's text laid out (ADR 0066): `shown` in the block's style, wrapped at the box less its
/// gutters when it is multiline, aligned in the box, with the outline moved into the gutter.
/// The carets are from the text's left edge (the alignment applied, not the gutter).
pub fn field_layout(fonts: &Fonts, style: &TextStyle, field: &TextField, box_width: f32, shown: &str) -> std::result::Result<anicraft_text::Layout, anicraft_text::TextError> {
    let wrap = field.wrap_width(box_width);
    let mut l = fonts.layout(
        shown,
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
            width: wrap,
        },
    )?;
    // Lines that don't wrap align against the widest; the widest aligns in the box.
    let inner = (box_width - 2.0 * FIELD_GUTTER).max(0.0);
    let shift = match (wrap, style.align) {
        (Some(_), _) | (None, TextAlign::Left) => 0.0,
        (None, TextAlign::Center) => ((inner - l.width) * 0.5).max(0.0),
        (None, TextAlign::Right) => (inner - l.width).max(0.0),
    };
    l.outline = l.outline.transformed(&Affine::translate(FIELD_GUTTER + shift, FIELD_GUTTER));
    if shift != 0.0 {
        for line in &mut l.carets {
            for s in &mut line.stops {
                s.1 += shift;
            }
        }
        l.width += shift;
    }
    Ok(l)
}

/// A field block for `text`: laid out in its box (`box_width` × `box_height`).
pub(crate) fn lay_out_field(text: &str, style: &TextStyle, field: &TextField, box_width: f32, box_height: f32) -> Result<(TextBlock, Option<String>)> {
    let fonts = Fonts::get();
    let text: String = text.chars().take(MAX_FIELD_CHARS).collect();
    let l = field_layout(fonts, style, field, box_width, &field.shown(&text)).map_err(|e| EngineError::State(format!("can't lay out text: {e}")))?;
    let note = (!style.font.trim().is_empty() && !l.family.eq_ignore_ascii_case(style.font.trim())).then(|| format!("“{}” isn't installed on this computer; used {}", style.font.trim(), l.family));
    let block = TextBlock {
        text,
        font: l.family,
        size: style.size,
        bold: style.bold,
        italic: style.italic,
        color: style.color,
        align: style.align,
        letter_spacing: style.letter_spacing,
        line_spacing: style.line_spacing,
        width: field.wrap_width(box_width),
        outline: Arc::new(l.outline),
        box_width,
        box_height,
        field: Some(field.clone()),
    };
    Ok((block, note))
}

/// The block `t` with new text or style: a field keeps its box (or takes `width` as its new
/// width); static text wraps at `width`.
fn relay(t: &TextBlock, text: &str, style: &TextStyle, width: Option<f32>) -> Result<(TextBlock, Option<String>)> {
    match &t.field {
        Some(f) => lay_out_field(text, style, f, width.unwrap_or(t.box_width), t.box_height),
        None => lay_out(text, style, width),
    }
}

/// The box a block becomes a field in: the text's own size and the gutters (at least 20 px wide).
fn field_box(t: &TextBlock) -> (f32, f32) {
    ((t.width.unwrap_or(t.box_width) + 2.0 * FIELD_GUTTER).max(20.0), t.box_height + 2.0 * FIELD_GUTTER)
}

fn kind_param(p: &Value) -> Result<Option<TextKind>> {
    opt_str(p, "type")?.map(|k| TextKind::from_key(k).ok_or_else(|| EngineError::Param(format!("'type' is static, dynamic or input, not '{k}'")))).transpose()
}

fn set_field(s: &mut Session, p: &Value) -> Result<Value> {
    let kind = kind_param(p)?;
    let lines = opt_str(p, "lines")?.map(|k| LineType::from_key(k).ok_or_else(|| EngineError::Param(format!("'lines' is single, multiline, multiline_no_wrap or password, not '{k}'")))).transpose()?;
    let size = |key: &str| -> Result<Option<f32>> {
        match opt_f32(p, key)? {
            Some(v) if (1.0..=100_000.0).contains(&v) => Ok(Some(v)),
            Some(_) => Err(EngineError::Param(format!("'{key}' must be between 1 and 100000 px"))),
            None => Ok(None),
        }
    };
    let (width, height) = (size("width")?, size("height")?);
    let max_chars = opt_u32(p, "max_chars")?;
    if max_chars.is_some_and(|n| n as usize > MAX_FIELD_CHARS) {
        return Err(EngineError::Param(format!("'max_chars' is at most {MAX_FIELD_CHARS}")));
    }
    let variable = opt_str(p, "variable")?.map(|v| v.trim().chars().take(MAX_FIELD_NAME).collect::<String>());
    let (selectable, border, background) = (opt_bool(p, "selectable")?, opt_bool(p, "border")?, opt_bool(p, "background")?);
    let (border_color, background_color) = (opt_color(p, "border_color")?, opt_color(p, "background_color")?);
    let mut changed = 0;
    let mut notes = Vec::new();
    for e in s.selected_elements_mut()? {
        let ElementKind::Text(t) = &e.kind else { continue };
        let to = kind.unwrap_or(t.kind());
        let style = style_of(t);
        let (block, note) = if to == TextKind::Static {
            if t.field.is_none() {
                continue;
            }
            // Back to static text: laid out as before fields, the words where they were.
            e.transform = e.transform.mul(&Affine::translate(FIELD_GUTTER, FIELD_GUTTER));
            e.name.clear();
            lay_out(&t.text, &style, t.field.as_ref().and_then(|f| f.wrap_width(t.box_width)))?
        } else {
            let (bw, bh) = if t.field.is_some() { (t.box_width, t.box_height) } else { field_box(t) };
            // Wrapped text becomes a multiline field, text with line breaks a multiline one
            // that doesn't wrap, the rest a single line.
            let lines_of = if t.width.is_some() {
                LineType::Multiline
            } else if t.text.contains('\n') {
                LineType::MultilineNoWrap
            } else {
                LineType::Single
            };
            let mut f = t.field.clone().unwrap_or_else(|| TextField { lines: lines_of, ..TextField::new(to) });
            if t.field.is_none() {
                // The text stays where it was; the box grows round it.
                e.transform = e.transform.mul(&Affine::translate(-FIELD_GUTTER, -FIELD_GUTTER));
            }
            f.kind = to;
            if let Some(l) = lines {
                if l == LineType::Password && to != TextKind::Input {
                    return Err(EngineError::Param("only input text can be a password field".into()));
                }
                f.lines = l;
            }
            if f.lines == LineType::Password && to != TextKind::Input {
                f.lines = LineType::Single;
            }
            f.selectable = selectable.unwrap_or(f.selectable);
            f.border = border.unwrap_or(f.border);
            f.background = background.unwrap_or(f.background);
            f.border_color = border_color.unwrap_or(f.border_color);
            f.background_color = background_color.unwrap_or(f.background_color);
            f.max_chars = max_chars.unwrap_or(f.max_chars);
            if let Some(v) = &variable {
                f.variable.clone_from(v);
            }
            lay_out_field(&t.text, &style, &f.sanitized(), width.unwrap_or(bw), height.unwrap_or(bh))?
        };
        notes.extend(note);
        e.kind = ElementKind::Text(Arc::new(block));
        changed += 1;
    }
    if changed == 0 {
        return Err(EngineError::State("the selection has no text to change".into()));
    }
    notes.dedup();
    Ok(json!({"changed": changed, "note": notes.first()}))
}

pub(crate) fn style_of(t: &TextBlock) -> TextStyle {
    TextStyle { font: t.font.clone(), size: t.size, bold: t.bold, italic: t.italic, color: t.color, align: t.align, letter_spacing: t.letter_spacing, line_spacing: t.line_spacing }
}

fn create(s: &mut Session, p: &Value) -> Result<Value> {
    let text: String = str_req(p, "text")?.to_string();
    let kind = kind_param(p)?.unwrap_or_default();
    // An input field may start empty (the viewer fills it in); static text may not.
    if text.trim().is_empty() && kind == TextKind::Static {
        return Err(EngineError::Param("'text' is empty".into()));
    }
    let style = style_param(p, &s.text_style)?;
    let width = width_param(p, None)?;
    let (block, note) = lay_out(&text, &style, width)?;
    let (block, note) = if kind == TextKind::Static {
        (block, note)
    } else {
        let (bw, bh) = field_box(&block);
        let f = TextField { lines: if width.is_some() { LineType::Multiline } else if text.contains('\n') { LineType::MultilineNoWrap } else { LineType::Single }, ..TextField::new(kind) };
        lay_out_field(&text, &style, &f, bw, bh)?
    };
    let x = opt_f32(p, "x")?.unwrap_or((s.doc.stage.width as f32 - block.box_width) * 0.5);
    let y = opt_f32(p, "y")?.unwrap_or((s.doc.stage.height as f32 - block.box_height) * 0.5);
    let id = ElementId(s.doc.alloc_id());
    let out = json!({"element": id.0, "font": block.font, "width": block.box_width, "height": block.box_height, "note": note});
    s.text_style = TextStyle { font: block.font.clone(), ..style };
    let el = Element { id, name: String::new(), transform: Affine::translate(x, y), color: ColorEffect::default(), kind: ElementKind::Text(Arc::new(block)) };
    s.editable_keyframe()?.elements.push(Arc::new(el));
    s.select_only([id]);
    Ok(out)
}

fn edit(s: &mut Session, p: &Value) -> Result<Value> {
    let new_text = opt_str(p, "text")?.map(str::to_string);
    let mut notes = Vec::new();
    let mut edited = 0;
    let mut last_style = None;
    for e in s.selected_elements_mut()? {
        let ElementKind::Text(t) = &e.kind else { continue };
        let style = style_param(p, &style_of(t))?;
        let text = new_text.clone().unwrap_or_else(|| t.text.clone());
        // Fields may be empty (a script or the viewer fills them); static text may not.
        if text.trim().is_empty() && t.field.is_none() {
            return Err(EngineError::Param("'text' is empty (delete the text instead)".into()));
        }
        let width = if t.field.is_some() { width_param(p, Some(t.box_width))?.or(Some(t.box_width)).map(|w| w.max(1.0)) } else { width_param(p, t.width)? };
        let (block, note) = relay(t, &text, &style, width)?;
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
