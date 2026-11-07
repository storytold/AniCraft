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
        let laid = self.db.with_face_data(id, |data, index| lay_out(data, index, text, style)).flatten().ok_or(TextError::NoFonts)?;
        Ok(Layout { family, ..laid })
    }
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

/// One shaped glyph: id, where its pen position starts, its offsets and advance (pixels).
struct Glyph {
    id: skrifa::GlyphId,
    cluster: usize,
    x_offset: f32,
    y_offset: f32,
    advance: f32,
}

fn lay_out(data: &[u8], index: u32, text: &str, style: &Style) -> Option<Layout> {
    let style = sanitize(style);
    let font = skrifa::FontRef::from_index(data, index).ok()?;
    let size = skrifa::instance::Size::new(style.size);
    let metrics = font.metrics(size, skrifa::instance::LocationRef::default());
    let upem = f32::from(metrics.units_per_em.max(1));
    let scale = style.size / upem;
    let ascent = metrics.ascent;
    let line_height = (metrics.ascent - metrics.descent + metrics.leading).max(style.size * 0.5) + style.line_spacing;
    let shaper_data = harfrust::ShaperData::new(&font);
    let shaper = shaper_data.shaper(&font).build();
    let glyphs = font.outline_glyphs();
    let charmap = font.charmap();
    let missing = text.chars().filter(|c| !c.is_control() && !c.is_whitespace() && charmap.map(*c).is_none()).count();

    // Lines: paragraphs at line breaks, each wrapped at spaces to the width if there is one.
    let mut lines: Vec<(f32, Vec<(Glyph, f32)>)> = Vec::new();
    for para in text.split('\n') {
        let para = para.strip_suffix('\r').unwrap_or(para);
        let mut buffer = harfrust::UnicodeBuffer::new();
        buffer.push_str(para);
        buffer.guess_segment_properties();
        let shaped = shaper.shape(buffer, harfrust::ShapeOptions::new());
        let run: Vec<Glyph> = shaped
            .glyph_infos()
            .iter()
            .zip(shaped.glyph_positions())
            .map(|(info, pos)| Glyph {
                id: skrifa::GlyphId::new(info.glyph_id),
                cluster: info.cluster as usize,
                x_offset: pos.x_offset as f32 * scale,
                y_offset: pos.y_offset as f32 * scale,
                advance: pos.x_advance as f32 * scale + style.letter_spacing,
            })
            .collect();
        let is_space = |g: &Glyph| para.get(g.cluster..).and_then(|s| s.chars().next()).is_some_and(char::is_whitespace);
        let mut line: Vec<(Glyph, f32)> = Vec::new();
        let mut x = 0.0f32;
        // Where the current line could break: after its last space.
        let mut break_at: Option<usize> = None;
        for g in run {
            let space = is_space(&g);
            if let Some(w) = style.width
                && !space
                && x + g.advance > w
                && !line.is_empty()
            {
                // Wrap: at the last space if there is one (dropping trailing spaces), else here.
                let rest = match break_at {
                    Some(b) => line.split_off(b),
                    None => Vec::new(),
                };
                while line.last().is_some_and(|(g, _)| is_space(g)) {
                    line.pop();
                }
                let width = line.last().map_or(0.0, |(g, gx)| gx + g.advance);
                lines.push((width, std::mem::take(&mut line)));
                let shift = rest.first().map_or(0.0, |(_, gx)| *gx);
                line = rest.into_iter().map(|(g, gx)| (g, gx - shift)).collect();
                x = line.last().map_or(0.0, |(g, gx)| gx + g.advance);
                break_at = None;
            }
            line.push((g, x));
            x += line.last().map_or(0.0, |(g, _)| g.advance);
            if space {
                break_at = Some(line.len());
            }
        }
        while line.last().is_some_and(|(g, _)| is_space(g)) {
            line.pop();
        }
        let width = line.last().map_or(0.0, |(g, gx)| gx + g.advance);
        lines.push((width, line));
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
            let Some(glyph) = glyphs.get(g.id) else { continue };
            let mut pen = Pen { cmds: &mut outline.cmds, x: shift + gx + g.x_offset, y: baseline - g.y_offset };
            let settings = skrifa::outline::DrawSettings::unhinted(size, skrifa::instance::LocationRef::default());
            let _ = glyph.draw(settings, &mut pen);
        }
    }
    let height = (lines.len().max(1) as f32 - 1.0) * line_height + (metrics.ascent - metrics.descent);
    Some(Layout { outline, width: box_width.max(0.0), height: height.max(0.0), family: String::new(), missing })
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
