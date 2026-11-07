//! AniCraft text (layer 1): the fonts on this computer, and text laid out into vector outlines.
//!
//! `layout` shapes text with `harfrust` (a HarfBuzz port), breaks it into lines, and draws each
//! glyph's outline with `skrifa` into one [`Path`], in pixels, with the layout box's top-left
//! corner at the origin. The engine stores that path with the text element (ADR 0012), so
//! rendering never depends on which fonts are installed.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use anicraft_geom::{Path, PathCmd, Vec2};
use skrifa::MetadataProvider as _;
use std::sync::{Arc, OnceLock};

/// Families tried, in order, when none is asked for or the one asked for isn't installed.
pub const DEFAULT_FAMILIES: &[&str] = &["Inter", "Arial", "Helvetica", "Helvetica Neue", "Segoe UI", "DejaVu Sans", "Liberation Sans", "Noto Sans", "Roboto", "Ubuntu"];

/// Largest text block laid out (characters).
pub const MAX_CHARS: usize = 100_000;

/// Horizontal alignment of each line in the block.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

/// How to lay text out.
#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    pub family: String,
    /// Font size in pixels (the em square).
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
    pub align: Align,
    /// Extra space between characters, in pixels.
    pub letter_spacing: f32,
    /// Extra space between lines, in pixels (added to the font's own line height).
    pub line_spacing: f32,
    /// Wrap lines to this width (pixels); `None` keeps each paragraph on one line.
    pub width: Option<f32>,
}

impl Default for Style {
    fn default() -> Self {
        Style { family: String::new(), size: 32.0, bold: false, italic: false, align: Align::Left, letter_spacing: 0.0, line_spacing: 0.0, width: None }
    }
}

/// Laid-out text.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    /// Every glyph's outline, in pixels, top-left of the box at the origin (y down).
    pub outline: Path,
    /// The layout box: the given width (or the widest line) by the lines' height.
    pub width: f32,
    pub height: f32,
    /// The family actually used (the one asked for, or the fallback).
    pub family: String,
    /// Characters the font has no glyph for (drawn as its "notdef" box).
    pub missing: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub enum TextError {
    /// No usable font on this computer.
    NoFonts,
    /// More than [`MAX_CHARS`] characters.
    TooLong,
}

impl std::fmt::Display for TextError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TextError::NoFonts => f.write_str("no fonts were found on this computer"),
            TextError::TooLong => write!(f, "text is limited to {MAX_CHARS} characters per block"),
        }
    }
}

impl std::error::Error for TextError {}

/// The fonts AniCraft can use: the system's, those in `CRAFT_FONTS_DIR`, and faces the app
/// registered (its embedded interface font).
pub struct Fonts {
    db: fontdb::Database,
    families: Vec<String>,
}

static FONTS: OnceLock<Fonts> = OnceLock::new();
static EXTRA: OnceLock<Vec<Arc<[u8]>>> = OnceLock::new();

impl Fonts {
    /// Faces to add to the library (call before the first [`Fonts::get`]; later calls are
    /// ignored). The app registers its embedded Inter so text has a known default everywhere.
    pub fn register(faces: Vec<Arc<[u8]>>) {
        let _ = EXTRA.set(faces);
    }

    /// The font library, scanned once on first use.
    pub fn get() -> &'static Fonts {
        FONTS.get_or_init(|| {
            let mut db = fontdb::Database::new();
            for face in EXTRA.get().into_iter().flatten() {
                db.load_font_data(face.to_vec());
            }
            if let Some(dir) = std::env::var_os("CRAFT_FONTS_DIR") {
                db.load_fonts_dir(dir);
            }
            db.load_system_fonts();
            Fonts::from_db(db)
        })
    }

    fn from_db(db: fontdb::Database) -> Fonts {
        let mut families: Vec<String> = db.faces().filter_map(|f| f.families.first().map(|(n, _)| n.clone())).filter(|n| !n.starts_with('.')).collect();
        families.sort_by_key(|f| f.to_lowercase());
        families.dedup();
        Fonts { db, families }
    }

    /// A library of just these faces (tests, and documents carrying their own fonts).
    pub fn from_faces(faces: impl IntoIterator<Item = Vec<u8>>) -> Fonts {
        let mut db = fontdb::Database::new();
        for f in faces {
            db.load_font_data(f);
        }
        Fonts::from_db(db)
    }

    /// Installed family names, sorted.
    pub fn families(&self) -> &[String] {
        &self.families
    }

    /// The family used for `asked`: itself when installed (any case), else the default.
    pub fn resolve(&self, asked: &str) -> Option<String> {
        let find = |name: &str| self.families.iter().find(|f| f.eq_ignore_ascii_case(name)).cloned();
        if !asked.trim().is_empty()
            && let Some(f) = find(asked.trim())
        {
            return Some(f);
        }
        DEFAULT_FAMILIES.iter().find_map(|f| find(f)).or_else(|| self.families.first().cloned())
    }

    /// Lays `text` out in `style`.
    pub fn layout(&self, text: &str, style: &Style) -> Result<Layout, TextError> {
        if text.chars().count() > MAX_CHARS {
            return Err(TextError::TooLong);
        }
        let family = self.resolve(&style.family).ok_or(TextError::NoFonts)?;
        let q = fontdb::Query {
            families: &[fontdb::Family::Name(&family)],
            weight: if style.bold { fontdb::Weight::BOLD } else { fontdb::Weight::NORMAL },
            stretch: fontdb::Stretch::Normal,
            style: if style.italic { fontdb::Style::Italic } else { fontdb::Style::Normal },
        };
        let id = self.db.query(&q).ok_or(TextError::NoFonts)?;
        let face = |id| self.db.with_face_data(id, |data, index| (data.to_vec(), index));
        let mut faces = vec![face(id).ok_or(TextError::NoFonts)?];
        // Characters the face doesn't have (Arabic in Inter, say) come from the first fallback
        // family that has them; everything else stays in the chosen face.
        let charmap_lacks = |f: &(Vec<u8>, u32), c: char| skrifa::FontRef::from_index(&f.0, f.1).ok().is_none_or(|font| font.charmap().map(c).is_none());
        let mut missing: Vec<char> = text.chars().filter(|c| needs_glyph(*c)).collect();
        missing.sort_unstable();
        missing.dedup();
        missing.retain(|c| faces.first().is_none_or(|f| charmap_lacks(f, *c)));
        for fallback in FALLBACK_FAMILIES {
            if missing.is_empty() || faces.len() >= MAX_FACES {
                break;
            }
            let Some(fid) = self.db.query(&fontdb::Query { families: &[fontdb::Family::Name(fallback)], ..q }).filter(|f| *f != id) else { continue };
            let Some(f) = face(fid) else { continue };
            let before = missing.len();
            missing.retain(|c| charmap_lacks(&f, *c));
            if missing.len() < before {
                faces.push(f);
            }
        }
        let laid = lay_out(&faces, text, style).ok_or(TextError::NoFonts)?;
        Ok(Layout { family, ..laid })
    }
}

/// Families tried, in order, for characters the chosen face doesn't have: faces with wide
/// coverage that come with Windows, macOS and common Linux desktops, then Noto's script families.
pub const FALLBACK_FAMILIES: &[&str] = &[
    "Segoe UI",
    "Arial",
    "Tahoma",
    "Geeza Pro",
    "Arial Unicode MS",
    "DejaVu Sans",
    "Noto Sans",
    "Noto Sans Arabic",
    "Noto Naskh Arabic",
    "Noto Sans Hebrew",
];

/// Most faces one text block draws with (the chosen face and its fallbacks).
const MAX_FACES: usize = 6;

/// A character that is drawn (not a space, a line break or another control).
fn needs_glyph(c: char) -> bool {
    !c.is_control() && !c.is_whitespace()
}

/// Clamps a style to sane ranges (sizes from 1 to 2000 px, spacing within ±1000 px).
pub fn sanitize(style: &Style) -> Style {
    let fin = |v: f32, d: f32| if v.is_finite() { v } else { d };
    Style {
        size: fin(style.size, 32.0).clamp(1.0, 2000.0),
        letter_spacing: fin(style.letter_spacing, 0.0).clamp(-1000.0, 1000.0),
        line_spacing: fin(style.line_spacing, 0.0).clamp(-1000.0, 1000.0),
        width: style.width.filter(|w| w.is_finite() && *w > 0.0).map(|w| w.min(100_000.0)),
        ..style.clone()
    }
}

/// One shaped glyph: the face it comes from, its id, the byte it starts at in its paragraph, its
/// offsets and advance (pixels).
struct Glyph {
    face: usize,
    id: skrifa::GlyphId,
    cluster: usize,
    x_offset: f32,
    y_offset: f32,
    advance: f32,
}

/// Lays `text` out with `faces`: the chosen face first (its metrics set the lines), then the
/// fallbacks for characters it doesn't have.
fn lay_out(faces: &[(Vec<u8>, u32)], text: &str, style: &Style) -> Option<Layout> {
    let style = sanitize(style);
    let fonts: Vec<skrifa::FontRef> = faces.iter().filter_map(|(data, index)| skrifa::FontRef::from_index(data, *index).ok()).collect();
    let size = skrifa::instance::Size::new(style.size);
    let metrics = fonts.first()?.metrics(size, skrifa::instance::LocationRef::default());
    let ascent = metrics.ascent;
    let line_height = (metrics.ascent - metrics.descent + metrics.leading).max(style.size * 0.5) + style.line_spacing;
    let shaper_data: Vec<harfrust::ShaperData> = fonts.iter().map(harfrust::ShaperData::new).collect();
    let shapers: Vec<harfrust::Shaper> = fonts.iter().zip(&shaper_data).map(|(f, d)| d.shaper(f).build()).collect();
    let scales: Vec<f32> = fonts.iter().map(|f| style.size / f32::from(f.metrics(size, skrifa::instance::LocationRef::default()).units_per_em.max(1))).collect();
    let outlines: Vec<_> = fonts.iter().map(|f| f.outline_glyphs()).collect();
    let charmaps: Vec<_> = fonts.iter().map(|f| f.charmap()).collect();
    let has = |face: usize, c: char| charmaps.get(face).is_some_and(|m| m.map(c).is_some());
    let missing = text.chars().filter(|c| needs_glyph(*c) && !(0..fonts.len()).any(|f| has(f, *c))).count();

    // Lines: paragraphs at line breaks, each wrapped at spaces to the width if there is one. Text
    // that mixes directions follows the Unicode bidi algorithm (UAX #9): a paragraph's lines are
    // found in reading order, then each line's runs are shaped in their own direction and placed
    // in visual order, so a number or a Latin word inside Arabic reads the right way round, and a
    // wrapped Arabic paragraph starts on the first line.
    let mut lines: Vec<(f32, Vec<(Glyph, f32)>)> = Vec::new();
    for para in text.split('\n') {
        let para = para.strip_suffix('\r').unwrap_or(para);
        let bidi = unicode_bidi::ParagraphBidiInfo::new(para, None);
        // The face drawing each byte: the chosen one where it has the character, else the first
        // fallback that does. Spaces and marks stay with what comes before them, so a letter and
        // its harakat are shaped together.
        let mut face_of = vec![0usize; para.len()];
        let mut current = 0;
        for (i, c) in para.char_indices() {
            let joins = !needs_glyph(c) || matches!(unicode_bidi::bidi_class(c), unicode_bidi::BidiClass::NSM | unicode_bidi::BidiClass::BN);
            if !joins || !has(current, c) {
                current = (0..fonts.len()).find(|f| has(*f, c)).unwrap_or(if joins { current } else { 0 });
            }
            face_of.iter_mut().skip(i).take(c.len_utf8()).for_each(|f| *f = current);
        }
        // A run shaped in one direction, split where the face changes; glyphs in visual order.
        let shape_run = |run: std::ops::Range<usize>, rtl: bool| {
            let mut parts: Vec<Vec<Glyph>> = Vec::new();
            let mut start = run.start;
            while start < run.end {
                let face = face_of.get(start).copied().unwrap_or(0);
                let end = (start..run.end).find(|i| face_of.get(*i).copied().unwrap_or(0) != face).unwrap_or(run.end);
                if let (Some(s), Some(shaper)) = (para.get(start..end), shapers.get(face)) {
                    parts.push(shape(shaper, face, s, start, rtl, scales.get(face).copied().unwrap_or(1.0), style.letter_spacing));
                }
                start = end;
            }
            if rtl {
                parts.reverse();
            }
            parts.into_iter().flatten().collect::<Vec<_>>()
        };
        // Each byte's share of the width (a cluster's advance on its first byte), run by run.
        let mut advance = vec![0.0f32; para.len()];
        let mut start = 0;
        while start < para.len() {
            let level = bidi.levels.get(start).copied();
            let end = (start..para.len()).find(|i| bidi.levels.get(*i).copied() != level).unwrap_or(para.len());
            for g in shape_run(start..end, level.is_some_and(|l| l.is_rtl())) {
                if let Some(a) = advance.get_mut(g.cluster) {
                    *a += g.advance;
                }
            }
            start = end;
        }
        for range in break_lines(para, &advance, style.width) {
            let mut line: Vec<(Glyph, f32)> = Vec::new();
            let mut x = 0.0f32;
            if !range.is_empty() {
                let (levels, runs) = bidi.visual_runs(range);
                for run in runs {
                    let rtl = levels.get(run.start).is_some_and(|l| l.is_rtl());
                    for g in shape_run(run, rtl) {
                        let a = g.advance;
                        line.push((g, x));
                        x += a;
                    }
                }
            }
            lines.push((x, line));
        }
    }

    let box_width = style.width.unwrap_or_else(|| lines.iter().map(|(w, _)| *w).fold(0.0, f32::max));
    let mut outline = Path::new();
    for (row, (width, line)) in lines.iter().enumerate() {
        let shift = match style.align {
            Align::Left => 0.0,
            Align::Center => (box_width - width) * 0.5,
            Align::Right => box_width - width,
        };
        let baseline = ascent + row as f32 * line_height;
        for (g, gx) in line {
            let Some(glyph) = outlines.get(g.face).and_then(|o| o.get(g.id)) else { continue };
            let mut pen = Pen { cmds: &mut outline.cmds, x: shift + gx + g.x_offset, y: baseline - g.y_offset };
            let settings = skrifa::outline::DrawSettings::unhinted(size, skrifa::instance::LocationRef::default());
            let _ = glyph.draw(settings, &mut pen);
        }
    }
    let height = (lines.len().max(1) as f32 - 1.0) * line_height + (metrics.ascent - metrics.descent);
    Some(Layout { outline, width: box_width.max(0.0), height: height.max(0.0), family: String::new(), missing })
}

/// Shapes `text` (which starts at byte `at` of its paragraph) with one face in one direction.
/// Glyphs come back in visual order (left to right), clusters counted from the paragraph's start.
fn shape(shaper: &harfrust::Shaper<'_>, face: usize, text: &str, at: usize, rtl: bool, scale: f32, letter_spacing: f32) -> Vec<Glyph> {
    let mut buffer = harfrust::UnicodeBuffer::new();
    buffer.push_str(text);
    buffer.set_direction(if rtl { harfrust::Direction::RightToLeft } else { harfrust::Direction::LeftToRight });
    buffer.guess_segment_properties();
    let shaped = shaper.shape(buffer, harfrust::ShapeOptions::new());
    shaped
        .glyph_infos()
        .iter()
        .zip(shaped.glyph_positions())
        .map(|(info, pos)| Glyph {
            face,
            id: skrifa::GlyphId::new(info.glyph_id),
            cluster: at + info.cluster as usize,
            x_offset: pos.x_offset as f32 * scale,
            y_offset: pos.y_offset as f32 * scale,
            advance: pos.x_advance as f32 * scale + letter_spacing,
        })
        .collect()
}

/// A paragraph's lines as byte ranges in reading order, given each byte's share of the width:
/// wrapped after the last space before `width` is passed (or within a word wider than the line),
/// with the spaces at a line's end left out.
fn break_lines(para: &str, advance: &[f32], width: Option<f32>) -> Vec<std::ops::Range<usize>> {
    let trimmed = |start: usize, end: usize| start..para.get(start..end).map_or(start, |s| start + s.trim_end().len());
    let mut lines = Vec::new();
    let (mut start, mut x, mut break_at) = (0usize, 0.0f32, None::<usize>);
    for (i, c) in para.char_indices() {
        let a = advance.get(i).copied().unwrap_or(0.0);
        let space = c.is_whitespace();
        if let Some(w) = width
            && !space
            && x + a > w
            && i > start
        {
            let end = break_at.unwrap_or(i);
            lines.push(trimmed(start, end));
            start = end;
            x = advance.get(start..i).map_or(0.0, |s| s.iter().sum());
            break_at = None;
        }
        x += a;
        if space {
            break_at = Some(i + c.len_utf8());
        }
    }
    lines.push(trimmed(start, para.len()));
    lines
}

/// Draws glyph outlines (font space, y up) into a path (pixels, y down) at a pen position.
struct Pen<'a> {
    cmds: &'a mut Vec<PathCmd>,
    x: f32,
    y: f32,
}

impl Pen<'_> {
    fn at(&self, x: f32, y: f32) -> Vec2 {
        Vec2::new(self.x + x, self.y - y)
    }
}

impl skrifa::outline::OutlinePen for Pen<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        let p = self.at(x, y);
        self.cmds.push(PathCmd::MoveTo(p));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let p = self.at(x, y);
        self.cmds.push(PathCmd::LineTo(p));
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        let (c, p) = (self.at(cx, cy), self.at(x, y));
        self.cmds.push(PathCmd::QuadTo(c, p));
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        let (a, b, p) = (self.at(cx0, cy0), self.at(cx1, cy1), self.at(x, y));
        self.cmds.push(PathCmd::CubicTo(a, b, p));
    }
    fn close(&mut self) {
        self.cmds.push(PathCmd::Close);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A font from this computer, if it has any (CI images and desktops do; a bare container
    /// may not, and then these tests say so and pass).
    fn fonts() -> Option<&'static Fonts> {
        let f = Fonts::get();
        if f.families().is_empty() {
            eprintln!("no fonts on this computer: text layout tests skipped");
            return None;
        }
        Some(f)
    }

    fn style(size: f32) -> Style {
        Style { size, ..Style::default() }
    }

    #[test]
    fn text_becomes_outlines_in_a_box() {
        let Some(fonts) = fonts() else { return };
        let l = fonts.layout("Hello", &style(40.0)).unwrap();
        assert!(!l.family.is_empty());
        assert!(l.outline.cmds.len() > 20, "five glyphs of outlines");
        let b = l.outline.bounds();
        assert!(b.min.x >= -2.0 && b.max.x <= l.width + 2.0, "{b:?} in {}", l.width);
        assert!(b.min.y >= -2.0 && b.max.y <= l.height + 2.0, "{b:?} in {}", l.height);
        assert!(l.width > 40.0 && l.height > 30.0 && l.height < 80.0, "{} × {}", l.width, l.height);
        // Bigger type, bigger box; letter spacing widens it; empty text is an empty box.
        let big = fonts.layout("Hello", &style(80.0)).unwrap();
        assert!((big.width / l.width - 2.0).abs() < 0.05);
        let spaced = fonts.layout("Hello", &Style { letter_spacing: 10.0, ..style(40.0) }).unwrap();
        assert!((spaced.width - l.width - 50.0).abs() < 0.5, "{} vs {}", spaced.width, l.width);
        assert!(fonts.layout("", &style(40.0)).unwrap().outline.cmds.is_empty());
    }

    #[test]
    fn lines_break_at_newlines_and_wrap_at_spaces() {
        let Some(fonts) = fonts() else { return };
        let one = fonts.layout("one", &style(20.0)).unwrap();
        let two = fonts.layout("one\ntwo", &style(20.0)).unwrap();
        assert!(two.height > one.height * 1.8, "{} vs {}", two.height, one.height);
        let spaced = fonts.layout("one\ntwo", &Style { line_spacing: 10.0, ..style(20.0) }).unwrap();
        assert!((spaced.height - two.height - 10.0).abs() < 0.01);
        // A width wraps at spaces, never wider than the width when the words fit.
        let wrapped = fonts.layout("one two three four five six", &Style { width: Some(80.0), ..style(20.0) }).unwrap();
        assert_eq!(wrapped.width, 80.0);
        assert!(wrapped.height > one.height * 2.5, "several lines: {}", wrapped.height);
        assert!(wrapped.outline.bounds().max.x <= 82.0);
        // Alignment moves lines within the box.
        let boxed = |align| fonts.layout("a", &Style { align, width: Some(300.0), ..style(20.0) }).unwrap().outline.bounds();
        assert!(boxed(Align::Left).max.x < 20.0);
        assert!((140.0..160.0).contains(&boxed(Align::Center).center().x), "{:?}", boxed(Align::Center));
        assert!(boxed(Align::Right).min.x > 280.0);
    }

    #[test]
    fn mixed_directions_read_the_right_way_round() {
        // Lines are found in reading order: after the last space that fits, or within a word.
        assert_eq!(break_lines("ab cd ef", &[1.0; 8], Some(5.0)), vec![0..5, 6..8]);
        assert_eq!(break_lines("abcdefgh", &[1.0; 8], Some(3.0)), vec![0..3, 3..6, 6..8]);
        let Some(fonts) = fonts() else { return };
        let lay = |t: &str, width| fonts.layout(t, &Style { width, ..style(40.0) }).unwrap();
        let arabic = lay("مرحبا 2026", None);
        if arabic.missing > 0 {
            eprintln!("{} has no Arabic letters: right-to-left layout test skipped", arabic.family);
            return;
        }
        // A number inside Arabic keeps its digits in order, at the line's left end ...
        assert!(arabic.outline.cmds.starts_with(&lay("2026", None).outline.cmds));
        // ... and a wrapped Arabic paragraph starts with its first word, on the first line.
        let first = lay("بيت", None);
        let wrapped = lay("بيت جديد", Some(first.width + 1.0));
        assert!(wrapped.height > first.height * 1.8, "two lines: {}", wrapped.height);
        assert!(wrapped.outline.cmds.starts_with(&first.outline.cmds));
    }

    #[test]
    fn letters_the_font_lacks_come_from_a_fallback_face() {
        let Some(fonts) = fonts() else { return };
        let has = |family: &str, c: char| {
            let q = fontdb::Query { families: &[fontdb::Family::Name(family)], ..fontdb::Query::default() };
            fonts.db.query(&q).and_then(|id| fonts.db.with_face_data(id, |d, i| skrifa::FontRef::from_index(d, i).is_ok_and(|f| f.charmap().map(c).is_some()))).unwrap_or(false)
        };
        // A family with Latin letters and no Arabic ones, and a fallback that has Arabic.
        let Some(latin) = fonts.families().iter().find(|f| has(f, 'a') && !has(f, 'ب')) else { return };
        if !FALLBACK_FAMILIES.iter().any(|f| has(f, 'ب')) {
            eprintln!("no fallback family with Arabic letters here: fallback test skipped");
            return;
        }
        let lay = |t: &str| fonts.layout(t, &Style { family: latin.clone(), ..style(40.0) }).unwrap();
        let arabic = lay("بيت");
        assert_eq!((arabic.missing, arabic.family.as_str()), (0, latin.as_str()), "the block keeps its family");
        assert!(!arabic.outline.cmds.is_empty());
        // The Latin letters are still drawn in the chosen face, the Arabic ones after them.
        let mixed = lay("ab بيت");
        assert!(mixed.outline.cmds.starts_with(&lay("ab").outline.cmds));
        assert!(mixed.outline.cmds.len() > lay("ab").outline.cmds.len());
        // A character no font has is counted, so the command can say so.
        assert_eq!(lay("a\u{10FFFD}").missing, 1);
    }

    #[test]
    fn families_resolve_and_styles_are_clamped() {
        let Some(fonts) = fonts() else { return };
        let first = fonts.families()[0].clone();
        assert_eq!(fonts.resolve(&first.to_uppercase()), Some(first));
        let fallback = fonts.resolve("No Such Family 12345").unwrap();
        assert!(fonts.families().contains(&fallback));
        let s = sanitize(&Style { size: f32::NAN, letter_spacing: 1e9, width: Some(-5.0), ..Style::default() });
        assert_eq!((s.size, s.letter_spacing, s.width), (32.0, 1000.0, None));
        assert_eq!(fonts.layout(&"x".repeat(MAX_CHARS + 1), &style(10.0)), Err(TextError::TooLong));
        assert_eq!(Fonts::from_faces(Vec::new()).layout("x", &style(10.0)), Err(TextError::NoFonts));
    }
}
