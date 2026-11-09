//! Text fields (`DOMStaticText`, `DOMDynamicText`, `DOMInputText`) → text blocks laid out by
//! `anicraft-text` (ADR 0014).
//!
//! A field holds text runs (`<textRuns><DOMTextRun><characters>…</characters><textAttrs>
//! <DOMTextAttrs face="Arial-BoldMT" size="24" fillColor="#FF0000" alignment="center"
//! letterSpacing="1" lineSpacing="2"/>`). An AniCraft text block has one style, so a field
//! becomes one block per run of consecutive lines that share a style, stacked as Animate
//! stacks them; a line that mixes styles takes the style most of its characters have, and the
//! report says so.

use crate::import::{Report, color};
use crate::xml::Element as X;
use anicraft_doc::{Rgba, TextAlign, TextBlock};
use anicraft_geom::Vec2;
use anicraft_text::{Align, FaceMatch, Fonts, TextError};
use std::collections::HashMap;
use std::sync::Arc;

/// The space between a text field's box and its text, on every side (pixels), as in Animate's
/// text fields.
pub const GUTTER: f32 = 2.0;

/// Most runs read from one field.
const MAX_RUNS: usize = 10_000;

/// One run's style, as `DOMTextAttrs` gives it.
#[derive(Clone, Debug, PartialEq)]
struct RunStyle {
    face: String,
    size: f32,
    bold: bool,
    italic: bool,
    color: Rgba,
    align: TextAlign,
    letter_spacing: f32,
    line_spacing: f32,
    left_margin: f32,
    right_margin: f32,
}

/// A laid-out block and where its box's top-left goes, in the field's coordinates.
pub struct Placed {
    pub block: TextBlock,
    pub offset: Vec2,
}

fn run_style(attrs: Option<&X>, flags: &mut Flags) -> RunStyle {
    let Some(a) = attrs else {
        return RunStyle { face: String::new(), size: 12.0, bold: false, italic: false, color: Rgba::BLACK, align: TextAlign::Left, letter_spacing: 0.0, line_spacing: 0.0, left_margin: 0.0, right_margin: 0.0 };
    };
    let num = |k: &str, lo: f32, hi: f32| a.f32(k).unwrap_or(0.0).clamp(lo, hi);
    let align = match a.attr("alignment") {
        Some("center") => TextAlign::Center,
        Some("right") => TextAlign::Right,
        Some(j) if j.starts_with("justify") => {
            flags.justified = true;
            TextAlign::Left
        }
        _ => TextAlign::Left,
    };
    if a.f32("indent").is_some_and(|i| i != 0.0) {
        flags.indented = true;
    }
    if a.attr("characterPosition").is_some_and(|p| p != "normal") {
        flags.shifted = true;
    }
    RunStyle {
        face: a.attr("face").unwrap_or("").chars().take(256).collect(),
        size: a.f32("size").unwrap_or(12.0).clamp(1.0, 2000.0),
        bold: a.bool("bold").unwrap_or(false),
        italic: a.bool("italic").unwrap_or(false),
        color: if a.attr("fillColor").is_some() { color(a, "fillColor", "alpha") } else { Rgba::BLACK.with_alpha(a.f32("alpha").unwrap_or(1.0).clamp(0.0, 1.0)) },
        align,
        letter_spacing: num("letterSpacing", -1000.0, 1000.0),
        line_spacing: num("lineSpacing", -1000.0, 1000.0),
        left_margin: num("leftMargin", 0.0, 10_000.0),
        right_margin: num("rightMargin", 0.0, 10_000.0),
    }
}

/// What a field used that AniCraft draws differently, reported once per field.
#[derive(Default)]
struct Flags {
    justified: bool,
    indented: bool,
    shifted: bool,
}

/// A line of the field: its pieces, each with the style of the run it came from.
type Line = Vec<(String, usize)>;

/// The style a line is drawn in: that of the most visible characters (the first on a tie), or
/// of its first piece when it is empty.
fn main_style(line: &Line) -> (usize, bool) {
    // Visible characters per style, in the order the styles first appear.
    let mut totals: Vec<(usize, usize)> = Vec::new();
    for (t, s) in line {
        let n = t.chars().filter(|c| !c.is_whitespace()).count();
        match totals.iter_mut().find(|(x, _)| x == s) {
            Some(e) => e.1 += n,
            None => totals.push((*s, n)),
        }
    }
    let mut best: Option<(usize, usize)> = None;
    for &(s, n) in &totals {
        if best.is_none_or(|(_, b)| n > b) {
            best = Some((s, n));
        }
    }
    (best.map_or(0, |(s, _)| s), totals.iter().filter(|(_, n)| *n > 0).count() > 1)
}

/// Lays out one text field. Fonts are looked up once per face name (`faces`); missing ones go
/// into the report.
pub fn field(el: &X, fonts: &Fonts, faces: &mut HashMap<String, FaceMatch>, report: &mut Report) -> Vec<Placed> {
    if el.name != "DOMStaticText" {
        report.skip("dynamic and input text fields (imported as static text)");
    }
    if el.attr("orientation").is_some_and(|o| o != "horizontal") {
        report.skip("vertical text (drawn horizontal)");
    }
    if !el.list("filters").is_empty() {
        report.skip("filters on text");
    }
    let mut flags = Flags::default();
    let mut styles: Vec<RunStyle> = Vec::new();
    let mut lines: Vec<Line> = vec![Vec::new()];
    let mut chars = 0usize;
    let runs = el.list("textRuns");
    let mut cut = runs.len() > MAX_RUNS;
    for run in runs.iter().filter(|r| r.name == "DOMTextRun").take(MAX_RUNS) {
        let style = run_style(run.child("textAttrs").and_then(|t| t.child("DOMTextAttrs")), &mut flags);
        let s = match styles.iter().position(|x| *x == style) {
            Some(i) => i,
            None => {
                styles.push(style);
                styles.len() - 1
            }
        };
        let text = run.child("characters").map_or("", |c| c.text.as_str());
        // Animate ends lines with a carriage return; any line break starts a new line here.
        let text = text.replace("\r\n", "\n").replace(['\r', '\u{2028}', '\u{2029}'], "\n");
        for (i, piece) in text.split('\n').enumerate() {
            if i > 0 {
                lines.push(Vec::new());
                chars += 1;
            }
            let room = anicraft_text::MAX_CHARS.saturating_sub(chars);
            let n = piece.chars().count();
            if n > room {
                cut = true;
            }
            let piece: String = piece.chars().take(room).collect();
            chars += n.min(room);
            if let Some(l) = lines.last_mut() {
                l.push((piece, s));
            }
        }
        if chars >= anicraft_text::MAX_CHARS {
            break;
        }
    }
    if cut {
        report.skip("text cut to 100,000 characters per field");
    }
    // Trailing empty lines draw nothing.
    while lines.last().is_some_and(|l| l.iter().all(|(t, _)| t.trim().is_empty())) {
        lines.pop();
    }
    if lines.is_empty() || styles.is_empty() {
        return Vec::new();
    }
    if flags.justified {
        report.skip("justified text (drawn left-aligned)");
    }
    if flags.indented {
        report.skip("first-line indents");
    }
    if flags.shifted {
        report.skip("superscript and subscript (drawn on the line)");
    }

    // Consecutive lines in the same style make one block.
    let mut blocks: Vec<(usize, String)> = Vec::new();
    for line in &lines {
        let (s, mixed) = main_style(line);
        if mixed {
            report.skip("lines with mixed text styles (drawn in their main style)");
        }
        let text: String = line.iter().map(|(t, _)| t.as_str()).collect();
        match blocks.last_mut() {
            Some((ls, t)) if *ls == s => {
                t.push('\n');
                t.push_str(&text);
            }
            _ => blocks.push((s, text)),
        }
    }

    let auto = el.bool("autoExpand").unwrap_or(false);
    let field_width = el.f32("width").filter(|w| *w > 0.0).map(|w| w.min(100_000.0));
    let origin = Vec2::new(el.f32("left").unwrap_or(0.0) + GUTTER, el.f32("top").unwrap_or(0.0) + GUTTER);
    let mut out: Vec<Placed> = Vec::new();
    let mut y = 0.0f32;
    for (s, text) in blocks {
        let Some(st) = styles.get(s) else { continue };
        let face = faces.entry(st.face.clone()).or_insert_with(|| fonts.find_face(&st.face)).clone();
        // A fixed-width field wraps inside its margins; an auto-size one never wraps.
        let width = if auto { None } else { field_width.map(|w| (w - st.left_margin - st.right_margin).max(1.0)) };
        let style = anicraft_text::Style {
            family: face.family.clone().unwrap_or_default(),
            size: st.size,
            bold: st.bold || face.bold,
            italic: st.italic || face.italic,
            align: match st.align {
                TextAlign::Left => Align::Left,
                TextAlign::Center => Align::Center,
                TextAlign::Right => Align::Right,
            },
            letter_spacing: st.letter_spacing,
            line_spacing: st.line_spacing,
            width,
        };
        let layout = match fonts.layout(&text, &style) {
            Ok(l) => l,
            Err(TextError::NoFonts) => {
                report.skip("text fields (no fonts on this computer)");
                return Vec::new();
            }
            Err(TextError::TooLong) => {
                report.skip("text cut to 100,000 characters per field");
                continue;
            }
        };
        // A family that isn't installed falls back to the default, as the text engine does
        // (ADR 0012), and the report names it.
        if face.family.is_none() && !face.wanted.is_empty() {
            report.missing_fonts.insert(face.wanted.clone());
            report.fallback_font.get_or_insert_with(|| layout.family.clone());
        }
        let block = TextBlock {
            text,
            font: layout.family,
            size: style.size,
            bold: style.bold,
            italic: style.italic,
            color: st.color,
            align: st.align,
            letter_spacing: st.letter_spacing,
            line_spacing: st.line_spacing,
            width,
            outline: Arc::new(layout.outline),
            box_width: layout.width,
            box_height: layout.height,
        };
        out.push(Placed { block, offset: Vec2::new(origin.x + st.left_margin, origin.y + y) });
        y += layout.lines as f32 * layout.line_height;
    }
    // Auto-size blocks align against the widest one, as lines of one field do.
    if auto {
        let widest = out.iter().map(|p| p.block.box_width).fold(0.0, f32::max);
        for p in &mut out {
            p.offset.x += match p.block.align {
                TextAlign::Left => 0.0,
                TextAlign::Center => (widest - p.block.box_width) * 0.5,
                TextAlign::Right => widest - p.block.box_width,
            };
        }
    }
    out
}
