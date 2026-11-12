//! SVG text (`text`, `tspan`) → text blocks laid out by `anicraft-text` (ADR 0021).
//!
//! SVG places text in chunks: a chunk starts at the `text` element and at every `tspan` with
//! an absolute `x`, and runs on until the next one. Each chunk becomes one single-line text
//! block, placed by its baseline (`x`, `y`, plus a leading `dx`/`dy`) and its `text-anchor`.
//! An AniCraft block has one style, so a chunk whose pieces differ in style takes the style of
//! most of its visible characters, and the report counts it (as in ADR 0014). Whitespace is
//! collapsed as browsers do, unless `xml:space="preserve"`.

use crate::{Anchor, Builder, ImportedElement, PaintSpec, Style, declarations, get, hidden, length, local, name_of};
use anicraft_doc::{ElementKind, Paint, TextAlign, TextBlock};
use anicraft_geom::{Affine, Rect};
use anicraft_text::{Align, FaceMatch, Fonts, MAX_CHARS, TextError};
use anicraft_xml::Element as X;
use std::sync::Arc;

/// Most chunks read from one `text` element.
const MAX_CHUNKS: usize = 10_000;
/// Deepest nesting of `tspan`s read.
const MAX_TSPAN_DEPTH: usize = 32;

/// A run of text that starts at one position.
struct Chunk {
    x: f32,
    y: f32,
    anchor: Anchor,
    pieces: Vec<(String, Style)>,
    chars: usize,
}

impl Chunk {
    fn has_text(&self) -> bool {
        self.pieces.iter().any(|(t, _)| !t.trim().is_empty())
    }
}

/// What a `text` element used that AniCraft draws differently, reported once per element.
#[derive(Default)]
struct Flags {
    per_char: bool,
    rotated: bool,
    on_path: bool,
    decorated: bool,
    vertical: bool,
    cut: bool,
}

/// Whether two pieces would be drawn the same.
fn same_look(a: &Style, b: &Style) -> bool {
    a.font_family == b.font_family && a.font_size == b.font_size && a.bold == b.bold && a.italic == b.italic && a.fill == b.fill && a.fill_opacity == b.fill_opacity && a.letter_spacing == b.letter_spacing
}

/// The CSS generic family a name stands for, as the device font `anicraft-text` knows.
fn generic(name: &str) -> Option<&'static str> {
    match name.to_ascii_lowercase().as_str() {
        "sans-serif" | "system-ui" | "ui-sans-serif" | "ui-rounded" | "-apple-system" | "blinkmacsystemfont" | "cursive" | "fantasy" => Some("_sans"),
        "serif" | "ui-serif" => Some("_serif"),
        "monospace" | "ui-monospace" => Some("_typewriter"),
        _ => None,
    }
}

/// The names in a `font-family` list, unquoted.
fn families(css: &str) -> Vec<String> {
    css.split(',').map(|n| n.trim().trim_matches(['"', '\'']).trim().to_string()).filter(|n| !n.is_empty()).take(16).collect()
}

/// Collapses whitespace across the chunks as browsers do: line breaks and tabs are spaces,
/// runs of spaces are one, and spaces at the start go. `xml:space="preserve"` keeps them.
fn collapse(chunks: &mut [Chunk]) {
    let mut prev_space = true;
    for c in chunks {
        for (t, st) in &mut c.pieces {
            let mut out = String::with_capacity(t.len());
            for ch in t.chars() {
                let ch = if matches!(ch, '\n' | '\r' | '\t') { ' ' } else { ch };
                if st.preserve {
                    out.push(ch);
                } else if ch == ' ' {
                    if !prev_space {
                        out.push(' ');
                    }
                } else {
                    out.push(ch);
                }
                prev_space = ch == ' ';
            }
            *t = out;
        }
    }
}

impl<'a> Builder<'a> {
    /// The text blocks of a `text` element (whose own style is `style`), placed by `m`.
    pub(crate) fn text(&mut self, el: &'a X, m: &Affine, style: &Style, alpha: f32) -> Vec<ImportedElement> {
        let mut chunks: Vec<Chunk> = Vec::new();
        let mut flags = Flags::default();
        self.text_content(el, style, &mut chunks, &mut flags, 0);
        collapse(&mut chunks);
        let report = [
            (flags.per_char, "per-character text positions (ignored)"),
            (flags.rotated, "rotated characters (drawn upright)"),
            (flags.on_path, "text on a path"),
            (flags.decorated, "underlined or struck-through text (lines left out)"),
            (flags.vertical, "vertical text (drawn horizontal)"),
            (flags.cut, "text cut to 100,000 characters per block"),
        ];
        for (on, what) in report {
            if on {
                self.report.skip(what);
            }
        }
        let mut out = Vec::new();
        for c in chunks {
            if !c.has_text() {
                continue;
            }
            // The style most of the visible characters have (the first on a tie).
            let mut totals: Vec<(usize, usize)> = Vec::new();
            for (i, (t, st)) in c.pieces.iter().enumerate() {
                let n = t.chars().filter(|ch| !ch.is_whitespace()).count();
                match totals.iter_mut().find(|(j, _)| c.pieces.get(*j).is_some_and(|(_, s)| same_look(s, st))) {
                    Some(e) => e.1 += n,
                    None => totals.push((i, n)),
                }
            }
            let mut best: Option<(usize, usize)> = None;
            for &(i, n) in &totals {
                if best.is_none_or(|(_, b)| n > b) {
                    best = Some((i, n));
                }
            }
            if totals.iter().filter(|(_, n)| *n > 0).count() > 1 {
                self.report.skip("text with mixed styles (drawn in its main style)");
            }
            let Some((_, st)) = best.and_then(|(i, _)| c.pieces.get(i)) else { continue };
            let st = st.clone();
            let text: String = c.pieces.iter().map(|(t, _)| t.as_str()).collect();
            match self.text_block(text, &st, alpha) {
                Ok(Some((block, ascent))) => {
                    let w = block.box_width;
                    let shift = match c.anchor {
                        Anchor::Start => 0.0,
                        Anchor::Middle => -w * 0.5,
                        Anchor::End => -w,
                    };
                    let transform = m.mul(&Affine::translate(c.x + shift, c.y - ascent));
                    if !transform.is_finite() {
                        continue;
                    }
                    let name = if out.is_empty() { name_of(el) } else { String::new() };
                    let block_alpha = alpha;
                    let block = TextBlock { align: match c.anchor { Anchor::Start => TextAlign::Left, Anchor::Middle => TextAlign::Center, Anchor::End => TextAlign::Right }, ..block };
                    out.push(ImportedElement { name, transform, alpha: block_alpha, kind: ElementKind::Text(Arc::new(block)) });
                }
                Ok(None) => {}
                Err(()) => break,
            }
        }
        out
    }

    /// Reads `el`'s positions and content into chunks.
    fn text_content(&mut self, el: &'a X, style: &Style, chunks: &mut Vec<Chunk>, flags: &mut Flags, depth: usize) {
        let (vw, vh) = self.viewport;
        let list = |key: &str, reference: f32| -> Vec<f32> { el.attr(key).map(|v| v.split([' ', ',', '\t', '\n', '\r']).filter(|p| !p.is_empty()).filter_map(|p| length(p, reference)).collect()).unwrap_or_default() };
        let (xs, ys, dxs, dys) = (list("x", vw), list("y", vh), list("dx", vw), list("dy", vh));
        if xs.len() > 1 || ys.len() > 1 || dxs.len() > 1 || dys.len() > 1 {
            flags.per_char = true;
        }
        flags.rotated |= el.attr("rotate").is_some_and(|r| r.split([' ', ',']).any(|v| v.trim().parse::<f32>().is_ok_and(|a| a != 0.0)));
        let d = declarations(el);
        flags.decorated |= get(&d, "text-decoration").is_some_and(|v| v != "none");
        flags.vertical |= get(&d, "writing-mode").is_some_and(|v| v.starts_with("tb") || v.starts_with("vertical"));
        let root = depth == 0;
        if root || !xs.is_empty() {
            if chunks.len() >= MAX_CHUNKS {
                flags.cut = true;
                return;
            }
            let y = ys.first().copied().or_else(|| chunks.last().map(|c| c.y)).unwrap_or(0.0);
            chunks.push(Chunk { x: xs.first().copied().unwrap_or(0.0), y, anchor: style.anchor, pieces: Vec::new(), chars: 0 });
        } else if let Some(&y) = ys.first() {
            // A new line without a new x continues after the text before it, which only the
            // layout knows: kept on the line when there is text already.
            match chunks.last_mut() {
                Some(c) if !c.has_text() => c.y = y,
                _ => flags.per_char = true,
            }
        }
        let (dx, dy) = (dxs.first().copied().unwrap_or(0.0), dys.first().copied().unwrap_or(0.0));
        if dx != 0.0 || dy != 0.0 {
            match chunks.last_mut() {
                Some(c) if !c.has_text() => {
                    c.x += dx;
                    c.y += dy;
                }
                _ => flags.per_char = true,
            }
        }
        for child in &el.children {
            match local(&child.name) {
                anicraft_xml::TEXT_NODE => {
                    let Some(c) = chunks.last_mut() else { continue };
                    let room = MAX_CHARS.saturating_sub(c.chars);
                    let n = child.text.chars().count();
                    if n > room {
                        flags.cut = true;
                    }
                    c.chars += n.min(room);
                    c.pieces.push((child.text.chars().take(room).collect(), style.clone()));
                }
                "tspan" | "a" => {
                    let cd = declarations(child);
                    if get(&cd, "display") == Some("none") || hidden(&cd) || depth >= MAX_TSPAN_DEPTH {
                        continue;
                    }
                    let st = style.with(child, &mut self.report);
                    self.text_content(child, &st, chunks, flags, depth + 1);
                }
                "textPath" => flags.on_path = true,
                _ => {}
            }
        }
    }

    /// Lays one chunk out: the block (aligned left, at the origin) and its first baseline.
    /// `Ok(None)` draws nothing; `Err` means no text can be laid out at all.
    fn text_block(&mut self, text: String, st: &Style, alpha: f32) -> Result<Option<(TextBlock, f32)>, ()> {
        if st.font_size <= 0.0 || alpha <= 0.0 {
            return Ok(None);
        }
        let unit = Rect::new(0.0, 0.0, 1.0, 1.0);
        let color = match self.paint(&st.fill, st.fill_opacity, st, &unit) {
            None => {
                if st.stroke != PaintSpec::None {
                    self.report.skip("outlined text without a fill (left out)");
                }
                return Ok(None);
            }
            Some(Paint::Solid(c)) => c,
            Some(other) => {
                self.report.skip("gradient-filled text (drawn in one colour)");
                other.primary_color()
            }
        };
        if st.stroke != PaintSpec::None && st.stroke_width > 0.0 {
            self.report.skip("strokes on text (left out)");
        }
        let fonts = self.fonts();
        let face = self.face(fonts, &st.font_family);
        let style = anicraft_text::Style {
            family: face.family.clone().unwrap_or_default(),
            size: st.font_size,
            bold: st.bold || face.bold,
            italic: st.italic || face.italic,
            align: Align::Left,
            letter_spacing: st.letter_spacing,
            line_spacing: 0.0,
            width: None,
        };
        let layout = match fonts.layout(&text, &style) {
            Ok(l) => l,
            Err(TextError::NoFonts) => {
                self.report.skip("text (no fonts on this computer)");
                return Err(());
            }
            Err(TextError::TooLong) => {
                self.report.skip("text cut to 100,000 characters per block");
                return Ok(None);
            }
        };
        // A family that isn't installed falls back to the default, as the text engine does
        // (ADR 0012), and the report names it.
        if face.family.is_none() && !face.wanted.is_empty() {
            self.report.missing_fonts.insert(face.wanted.clone());
            self.report.fallback_font.get_or_insert_with(|| layout.family.clone());
        }
        let ascent = layout.ascent;
        let block = TextBlock {
            text,
            font: layout.family,
            size: style.size,
            bold: style.bold,
            italic: style.italic,
            color,
            align: TextAlign::Left,
            letter_spacing: style.letter_spacing,
            line_spacing: 0.0,
            width: None,
            outline: Arc::new(layout.outline),
            box_width: layout.width,
            box_height: layout.height,
            field: None,
        };
        Ok(Some((block, ascent)))
    }

    /// The installed face for a `font-family` list: its first installed family, or (when none
    /// is) the first named family, uninstalled, for the report. No family is serif, as in
    /// browsers.
    fn face(&mut self, fonts: &Fonts, css: &str) -> FaceMatch {
        let mut names = families(css);
        if names.is_empty() {
            names.push("serif".into());
        }
        let mut first: Option<FaceMatch> = None;
        for n in &names {
            let key = generic(n).unwrap_or(n);
            let m = self.faces.entry(key.to_string()).or_insert_with(|| fonts.find_face(key)).clone();
            if m.family.is_some() {
                return m;
            }
            if first.is_none() && generic(n).is_none() {
                first = Some(m);
            }
        }
        // Only generic families, none installed: the default family, nothing to report.
        first.unwrap_or(FaceMatch { family: None, wanted: String::new(), bold: false, italic: false })
    }
}
