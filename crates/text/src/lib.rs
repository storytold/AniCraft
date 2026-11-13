//! AniCraft text (layer 1): the fonts on this computer, and text laid out into vector outlines.
//!
//! `layout` shapes text with `harfrust` (a HarfBuzz port), breaks it into lines, and draws each
//! glyph's outline with `skrifa` into one [`Path`], in pixels, with the layout box's top-left
//! corner at the origin. The engine stores that path with the text element (ADR 0012), so
//! rendering never depends on which fonts are installed.
//!
//! [`Fonts::layout_glyphs`] also hands back each glyph on its own ([`PlacedGlyph`]), for text
//! placed glyph by glyph: along a path ([`on_path`], ADR 0070) or at SVG's per-character
//! positions.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use anicraft_geom::{Path, PathCmd, Vec2};
use skrifa::MetadataProvider as _;
use std::sync::{Arc, OnceLock};

/// Families tried, in order, when none is asked for or the one asked for isn't installed.
pub const DEFAULT_FAMILIES: &[&str] = &["Inter", "Arial", "Helvetica", "Helvetica Neue", "Segoe UI", "DejaVu Sans", "Liberation Sans", "Noto Sans", "Roboto", "Ubuntu"];

/// Installed families tried for the device font `_serif`, in order.
const SERIF_FAMILIES: &[&str] = &["Times New Roman", "Times", "Georgia", "DejaVu Serif", "Liberation Serif", "Noto Serif"];

/// Installed families tried for the device font `_typewriter`, in order.
const MONO_FAMILIES: &[&str] = &["Courier New", "Courier", "JetBrains Mono", "DejaVu Sans Mono", "Liberation Mono", "Noto Sans Mono", "Consolas", "Menlo"];

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
    /// Lines laid out (at least one), and the distance between their baselines (pixels): a
    /// line that continued the block would start `lines × line_height` below its top.
    pub lines: usize,
    pub line_height: f32,
    /// From the top of the box to the first line's baseline (pixels), for placing text by its
    /// baseline, as SVG does.
    pub ascent: f32,
    /// Where a caret can stand, one entry per laid-out line, top first (ADR 0066: text fields
    /// that viewers type into).
    pub carets: Vec<CaretLine>,
    /// Each glyph on its own, in drawing order: only from [`Fonts::layout_glyphs`] (empty
    /// otherwise).
    pub glyphs: Vec<PlacedGlyph>,
}

/// One laid-out glyph on its own.
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedGlyph {
    /// Its outline, with its pen position on the baseline at the origin (pixels, y down).
    pub outline: Path,
    /// Its pen position in the block: from the box's left edge (alignment applied), and its
    /// line's baseline from the box's top.
    pub x: f32,
    pub baseline: f32,
    /// How far the pen moves past it (letter spacing included).
    pub advance: f32,
    /// The character it starts at (`char`s of the whole text, line breaks included).
    pub index: u32,
}

/// How text follows a path (ADR 0070).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PathPlacement {
    /// Where the text is anchored, as a distance along the path: left-aligned text starts
    /// there, centred text is centred on it, right-aligned text ends there.
    pub offset: f32,
    /// Along the path backwards: on its other side, upside down to the path's direction.
    pub flip: bool,
    pub align: Align,
}

/// Glyphs set along a path.
#[derive(Clone, Debug, PartialEq)]
pub struct PathText {
    /// Every glyph shown, in the path's space.
    pub outline: Path,
    /// The path's length.
    pub length: f32,
    /// Each shown glyph's character, the point on the path under its middle and its angle
    /// (radians, the direction of travel).
    pub placed: Vec<(u32, Vec2, f32)>,
    /// Glyphs whose middle falls past an open path's ends (not drawn, as SVG leaves them).
    pub hidden: usize,
}

/// The glyphs of a one-line layout set along `path`'s first subpath (ADR 0070): each glyph's
/// middle at its distance along the path (measured on the curve itself), the glyph turned to
/// the path's direction there, its baseline on the path, standing on the path's left (above a
/// path drawn left to right; `flip` walks it backwards, onto the other side). On a closed path
/// glyphs wrap round; on an open one those past its ends are left out. `None` when the path has
/// no length.
pub fn on_path(glyphs: &[PlacedGlyph], path: &Path, how: PathPlacement) -> Option<PathText> {
    let walk = anicraft_geom::walk::CurveWalk::new(path)?;
    let length = walk.length();
    let closed = first_subpath_closed(path);
    // The run's width: from its pen start (0) to the end of its last glyph.
    let width = glyphs.iter().map(|g| g.x + g.advance).filter(|w| w.is_finite()).fold(0.0f32, f32::max);
    let offset = if how.offset.is_finite() { how.offset.clamp(-1e6, 1e6) } else { 0.0 };
    let start = offset
        - match how.align {
            Align::Left => 0.0,
            Align::Center => width * 0.5,
            Align::Right => width,
        };
    let mut out = PathText { outline: Path::new(), length, placed: Vec::with_capacity(glyphs.len()), hidden: 0 };
    for g in glyphs {
        let mid = start + g.x + g.advance * 0.5;
        if !mid.is_finite() {
            out.hidden += 1;
            continue;
        }
        let s = if closed {
            mid.rem_euclid(length)
        } else if (0.0..=length).contains(&mid) {
            mid
        } else {
            out.hidden += 1;
            continue;
        };
        let (p, t) = if how.flip { (walk.at(length - s), walk.tangent(length - s) * -1.0) } else { (walk.at(s), walk.tangent(s)) };
        let angle = t.y.atan2(t.x);
        let m = anicraft_geom::Affine::translate(p.x, p.y).mul(&anicraft_geom::Affine::rotate(angle)).mul(&anicraft_geom::Affine::translate(-g.advance * 0.5, 0.0));
        out.outline.cmds.extend(g.outline.transformed(&m).cmds);
        out.placed.push((g.index, p, angle));
    }
    Some(out)
}

/// Text on a path in a box of its own, as text blocks keep it (ADR 0070): the glyphs set along
/// the path, then the glyphs and the path moved together so their bounds start at the origin.
#[derive(Clone, Debug, PartialEq)]
pub struct BoxedPathText {
    pub outline: Path,
    pub path: Path,
    pub width: f32,
    pub height: f32,
    /// Where the box's origin is in the path's space: what to translate by to put it back.
    pub shift: Vec2,
    pub hidden: usize,
}

/// [`on_path`], boxed ([`BoxedPathText`]).
pub fn on_path_boxed(glyphs: &[PlacedGlyph], path: &Path, how: PathPlacement) -> Option<BoxedPathText> {
    let set = on_path(glyphs, path, how)?;
    let b = set.outline.bounds().union(&path.bounds());
    let shift = if b.min.x.is_finite() && b.min.y.is_finite() { b.min } else { Vec2::ZERO };
    let back = anicraft_geom::Affine::translate(-shift.x, -shift.y);
    let size = |v: f32| if v.is_finite() { v.max(0.0) } else { 0.0 };
    Some(BoxedPathText { outline: set.outline.transformed(&back), path: path.transformed(&back), width: size(b.width()), height: size(b.height()), shift, hidden: set.hidden })
}

/// Whether a path's first subpath ends with a close.
fn first_subpath_closed(path: &Path) -> bool {
    for (i, c) in path.cmds.iter().enumerate() {
        match c {
            PathCmd::Close => return true,
            PathCmd::MoveTo(_) if i > 0 => return false,
            _ => {}
        }
    }
    false
}

/// One laid-out line's characters and caret positions. Indices count `char`s of the whole text
/// (line breaks included); `x` is from the box's left edge, alignment applied.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CaretLine {
    /// The first character on the line and the boundary after its last (a wrapped line's
    /// trailing spaces belong to it; a line break belongs to neither line).
    pub start: u32,
    pub end: u32,
    /// `(index, x)` of the boundaries glyphs start at and of the line's end, ascending.
    pub stops: Vec<(u32, f32)>,
}

impl CaretLine {
    /// The caret's x at character boundary `i` on this line: at the glyph starting there, or
    /// after the line's last glyph, or (inside a ligature) at the glyph it is part of.
    pub fn x_at(&self, i: u32) -> f32 {
        let mut x = self.stops.first().map_or(0.0, |s| s.1);
        for &(c, sx) in &self.stops {
            if c > i {
                break;
            }
            x = sx;
        }
        x
    }

    /// The boundary nearest to `x` on this line.
    pub fn index_at(&self, x: f32) -> u32 {
        let mut best = (self.start, f32::INFINITY);
        for &(c, sx) in &self.stops {
            let d = (sx - x).abs();
            if d < best.1 {
                best = (c, d);
            }
        }
        best.0.clamp(self.start, self.end)
    }
}

/// What a font name written by another program means on this computer: a PostScript name
/// (`Arial-BoldMT`, as Animate documents store faces), a family name, or a device font
/// (`_sans`, `_serif`, `_typewriter`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FaceMatch {
    /// The installed family, or `None` when it isn't installed (layout then falls back to the
    /// default family).
    pub family: Option<String>,
    /// The family the name stands for, installed or not ("Times New Roman"), for reports.
    pub wanted: String,
    pub bold: bool,
    pub italic: bool,
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
            // A web page can't see the computer's fonts: there, text uses the faces the app
            // registered (ADR 0036).
            #[cfg(not(target_arch = "wasm32"))]
            {
                if let Some(dir) = std::env::var_os("CRAFT_FONTS_DIR") {
                    db.load_fonts_dir(dir);
                }
                db.load_system_fonts();
            }
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

    /// The installed family named `name`, ignoring case and spaces ("SegoeUI" finds "Segoe UI").
    fn installed(&self, name: &str) -> Option<String> {
        let key = |s: &str| s.chars().filter(|c| !c.is_whitespace()).flat_map(char::to_lowercase).collect::<String>();
        let want = key(name);
        if want.is_empty() {
            return None;
        }
        self.families.iter().find(|f| key(f) == want).cloned()
    }

    /// What `name` (a PostScript name, a family or a device font) is on this computer: the
    /// face with that PostScript name, else the family, else the family the name suggests
    /// ([`guess_family`]).
    pub fn find_face(&self, name: &str) -> FaceMatch {
        let name = name.trim();
        if name.is_empty() {
            return FaceMatch { family: None, wanted: String::new(), bold: false, italic: false };
        }
        if let Some(f) = self.db.faces().find(|f| f.post_script_name.eq_ignore_ascii_case(name))
            && let Some((family, _)) = f.families.first()
        {
            return FaceMatch { family: Some(family.clone()), wanted: family.clone(), bold: f.weight.0 >= 600, italic: f.style != fontdb::Style::Normal };
        }
        let device: Option<&[&str]> = match name {
            "_sans" => Some(DEFAULT_FAMILIES),
            "_serif" => Some(SERIF_FAMILIES),
            "_typewriter" => Some(MONO_FAMILIES),
            _ => None,
        };
        if let Some(list) = device {
            let family = list.iter().find_map(|f| self.installed(f));
            return FaceMatch { wanted: family.clone().unwrap_or_else(|| name.to_string()), family, bold: false, italic: false };
        }
        if let Some(family) = self.installed(name) {
            return FaceMatch { wanted: family.clone(), family: Some(family), bold: false, italic: false };
        }
        let (wanted, bold, italic) = guess_family(name);
        FaceMatch { family: self.installed(&wanted), wanted, bold, italic }
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
        self.lay_out_with(text, style, false)
    }

    /// Lays `text` out in `style`, with each glyph on its own too ([`Layout::glyphs`]).
    pub fn layout_glyphs(&self, text: &str, style: &Style) -> Result<Layout, TextError> {
        self.lay_out_with(text, style, true)
    }

    fn lay_out_with(&self, text: &str, style: &Style, glyphs: bool) -> Result<Layout, TextError> {
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
        let laid = self.db.with_face_data(id, |data, index| lay_out(data, index, text, style, glyphs)).flatten().ok_or(TextError::NoFonts)?;
        Ok(Layout { family, ..laid })
    }
}

/// The family, boldness and slant a PostScript name suggests, for fonts that aren't installed:
/// `TimesNewRomanPS-BoldItalicMT` is Times New Roman, bold and italic.
pub fn guess_family(postscript: &str) -> (String, bool, bool) {
    let name: String = postscript.trim().chars().take(256).collect();
    let (base, style) = name.split_once(['-', ',']).unwrap_or((&name, ""));
    let style = style.to_ascii_lowercase();
    let bold = ["bold", "black", "heavy"].iter().any(|w| style.contains(w));
    let italic = ["italic", "oblique"].iter().any(|w| style.contains(w));
    // The suffixes PostScript names give Monotype and PostScript versions of a face.
    let mut base = base;
    for suffix in ["MT", "PS"] {
        base = base.strip_suffix(suffix).filter(|b| !b.is_empty()).unwrap_or(base);
    }
    // CamelCase to words: "HelveticaNeue" → "Helvetica Neue"; capital runs stay ("ComicSansMS").
    let mut family = String::with_capacity(base.len() + 4);
    let mut prev: Option<char> = None;
    for c in base.chars() {
        if c.is_uppercase() && prev.is_some_and(|p| p.is_lowercase() || p.is_ascii_digit()) {
            family.push(' ');
        }
        family.push(c);
        prev = Some(c);
    }
    (family.trim().to_string(), bold, italic)
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

fn lay_out(data: &[u8], index: u32, text: &str, style: &Style, want_glyphs: bool) -> Option<Layout> {
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
    // Per line: the character index its paragraph starts at, the paragraph's length in
    // characters, and the byte → character map of the paragraph (shared by its lines).
    let mut spans: Vec<(u32, u32, Arc<Vec<u32>>)> = Vec::new();
    let mut para_start = 0u32;
    for raw in text.split('\n') {
        let para = raw.strip_suffix('\r').unwrap_or(raw);
        let mut to_char = vec![0u32; para.len() + 1];
        let mut n = 0u32;
        for (b, _) in para.char_indices() {
            if let Some(slot) = to_char.get_mut(b) {
                *slot = n;
            }
            n += 1;
        }
        if let Some(last) = to_char.last_mut() {
            *last = n;
        }
        let to_char = Arc::new(to_char);
        let lines_before = lines.len();
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
        for _ in lines_before..lines.len() {
            spans.push((para_start, n, to_char.clone()));
        }
        para_start += raw.chars().count() as u32 + 1;
    }

    let box_width = style.width.unwrap_or_else(|| lines.iter().map(|(w, _)| *w).fold(0.0, f32::max));
    let mut outline = Path::new();
    let mut placed: Vec<PlacedGlyph> = Vec::new();
    for (row, (width, line)) in lines.iter().enumerate() {
        let shift = match style.align {
            Align::Left => 0.0,
            Align::Center => (box_width - width) * 0.5,
            Align::Right => box_width - width,
        };
        let baseline = ascent + row as f32 * line_height;
        let span = spans.get(row);
        for (g, gx) in line {
            let Some(glyph) = glyphs.get(g.id) else { continue };
            if want_glyphs {
                let settings = skrifa::outline::DrawSettings::unhinted(size, skrifa::instance::LocationRef::default());
                let mut own = Path::new();
                let mut pen = Pen { cmds: &mut own.cmds, x: g.x_offset, y: -g.y_offset };
                let _ = glyph.draw(settings, &mut pen);
                let index = span.map_or(0, |(pstart, plen, to_char)| pstart + to_char.get(g.cluster).copied().unwrap_or(*plen).min(*plen));
                placed.push(PlacedGlyph { outline: own, x: shift + gx, baseline, advance: g.advance, index });
            }
            let settings = skrifa::outline::DrawSettings::unhinted(size, skrifa::instance::LocationRef::default());
            let mut pen = Pen { cmds: &mut outline.cmds, x: shift + gx + g.x_offset, y: baseline - g.y_offset };
            let _ = glyph.draw(settings, &mut pen);
        }
    }
    let height = (lines.len().max(1) as f32 - 1.0) * line_height + (metrics.ascent - metrics.descent);
    let carets = caret_lines(&lines, &spans, box_width, style.align);
    Some(Layout { outline, width: box_width.max(0.0), height: height.max(0.0), family: String::new(), missing, lines: lines.len().max(1), line_height, ascent, carets, glyphs: placed })
}

/// The caret positions of laid-out lines (see [`CaretLine`]).
fn caret_lines(lines: &[(f32, Vec<(Glyph, f32)>)], spans: &[(u32, u32, Arc<Vec<u32>>)], box_width: f32, align: Align) -> Vec<CaretLine> {
    let mut out: Vec<CaretLine> = Vec::with_capacity(lines.len());
    for (row, ((width, line), (pstart, plen, to_char))) in lines.iter().zip(spans).enumerate() {
        let shift = match align {
            Align::Left => 0.0,
            Align::Center => (box_width - width) * 0.5,
            Align::Right => box_width - width,
        };
        let ch = |cluster: usize| pstart + to_char.get(cluster).copied().unwrap_or(*plen).min(*plen);
        // A paragraph's first line starts with it; a wrapped line at its first glyph.
        let first_of_para = row == 0 || spans.get(row - 1).is_none_or(|p| p.0 != *pstart);
        let start = if first_of_para { *pstart } else { line.first().map_or(*pstart, |(g, _)| ch(g.cluster)) };
        let mut stops: Vec<(u32, f32)> = line.iter().map(|(g, gx)| (ch(g.cluster), shift + gx)).collect();
        stops.sort_by_key(|s| s.0);
        stops.dedup_by_key(|s| s.0);
        if first_of_para && stops.first().is_none_or(|s| s.0 > start) {
            stops.insert(0, (start, shift));
        }
        // The boundary after the last glyph (its trailing spaces, if any, follow it).
        if let Some(&(c, _)) = stops.last() {
            stops.push((c + 1, shift + width));
        }
        out.push(CaretLine { start, end: start, stops });
        // The previous line of the same paragraph ends where this one starts.
        if !first_of_para && let Some(prev) = out.len().checked_sub(2).and_then(|i| out.get_mut(i)) {
            prev.end = start;
        }
    }
    // The last line of each paragraph ends with it.
    for (row, (pstart, plen, _)) in spans.iter().enumerate() {
        let last_of_para = spans.get(row + 1).is_none_or(|p| p.0 != *pstart);
        if last_of_para && let Some(l) = out.get_mut(row) {
            l.end = pstart + plen;
        }
    }
    for l in &mut out {
        l.end = l.end.max(l.start);
        let end = l.end;
        l.stops.retain(|s| s.0 <= end);
        if l.stops.last().is_none_or(|s| s.0 < end) {
            let x = l.stops.last().map_or(0.0, |s| s.1);
            l.stops.push((end, x));
        }
    }
    out
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

    #[test]
    fn postscript_names_suggest_families() {
        let g = |n: &str| guess_family(n);
        assert_eq!(g("ArialMT"), ("Arial".into(), false, false));
        assert_eq!(g("Arial-BoldMT"), ("Arial".into(), true, false));
        assert_eq!(g("TimesNewRomanPS-BoldItalicMT"), ("Times New Roman".into(), true, true));
        assert_eq!(g("TimesNewRomanPSMT"), ("Times New Roman".into(), false, false));
        assert_eq!(g("HelveticaNeue-LightOblique"), ("Helvetica Neue".into(), false, true));
        assert_eq!(g("ComicSansMS"), ("Comic Sans MS".into(), false, false));
        assert_eq!(g("Myriad Pro"), ("Myriad Pro".into(), false, false));
        assert_eq!(g("Gotham-Black"), ("Gotham".into(), true, false));
        assert_eq!(g(""), (String::new(), false, false));
        assert_eq!(g("MT"), ("MT".into(), false, false), "a suffix alone is the name");
    }

    #[test]
    fn faces_are_found_by_postscript_name_family_or_device_font() {
        let none = Fonts::from_faces(Vec::new());
        let m = none.find_face("Futura-Bold");
        assert_eq!((m.family, m.wanted.as_str(), m.bold, m.italic), (None, "Futura", true, false));
        assert_eq!(none.find_face("  ").wanted, "");
        let Some(fonts) = fonts() else { return };
        let face = fonts.db.faces().find(|f| !f.post_script_name.is_empty() && f.families.first().is_some_and(|(n, _)| !n.starts_with('.')));
        if let Some(face) = face {
            let m = fonts.find_face(&face.post_script_name);
            assert!(m.family.is_some(), "{}", face.post_script_name);
        }
        let first = fonts.families()[0].clone();
        assert_eq!(fonts.find_face(&first.to_uppercase()).family, Some(first.clone()));
        assert_eq!(fonts.find_face(&first.replace(' ', "")).family, Some(first), "spaces don't matter");
        assert!(fonts.find_face("_sans").family.is_some(), "the default families include an installed one");
        let missing = fonts.find_face("NoSuchFamily-Italic");
        assert_eq!((missing.family, missing.wanted.as_str(), missing.italic), (None, "No Such Family", true));
    }

    #[test]
    fn layouts_report_their_lines() {
        let Some(fonts) = fonts() else { return };
        let one = fonts.layout("one", &style(20.0)).unwrap();
        let three = fonts.layout("one\ntwo\nthree", &style(20.0)).unwrap();
        assert_eq!((one.lines, three.lines), (1, 3));
        assert!((one.line_height - three.line_height).abs() < 1e-3 && one.line_height >= 10.0);
        assert!((three.height - one.height - 2.0 * one.line_height).abs() < 0.01);
        assert_eq!(fonts.layout("", &style(20.0)).unwrap().lines, 1);
        // The first baseline is inside the box, below its top by most of a line.
        assert!(one.ascent > 10.0 && one.ascent < one.height, "{} in {}", one.ascent, one.height);
        let b = one.outline.bounds();
        assert!(b.max.y > one.ascent - 1.0 && b.min.y < one.ascent, "glyphs sit on the baseline: {b:?}");
    }

    #[test]
    fn glyphs_on_their_own_add_up_to_the_outline() {
        let Some(fonts) = fonts() else { return };
        let l = fonts.layout_glyphs("AVA b", &style(30.0)).unwrap();
        assert_eq!(l.outline, fonts.layout("AVA b", &style(30.0)).unwrap().outline, "the same outline");
        assert_eq!(fonts.layout("AVA b", &style(30.0)).unwrap().glyphs, Vec::new(), "only when asked");
        assert_eq!(l.glyphs.iter().map(|g| g.index).collect::<Vec<_>>(), vec![0, 1, 2, 3, 4]);
        let mut joined = Path::new();
        for g in &l.glyphs {
            joined.cmds.extend(g.outline.transformed(&anicraft_geom::Affine::translate(g.x, g.baseline)).cmds);
            assert_eq!(g.baseline, l.ascent);
        }
        let (a, b) = (joined.bounds(), l.outline.bounds());
        assert!((a.min - b.min).length() < 1e-3 && (a.max - b.max).length() < 1e-3, "{a:?} vs {b:?}");
        let last = l.glyphs.last().unwrap();
        assert!((last.x + last.advance - l.width).abs() < 1e-3);
    }

    /// Synthetic glyphs: a 10 × 20 box standing on the baseline, `n` of them 10 apart.
    fn boxes(n: u32) -> Vec<PlacedGlyph> {
        (0..n).map(|i| PlacedGlyph { outline: Path::rect(0.0, -20.0, 10.0, 20.0), x: i as f32 * 10.0, baseline: 20.0, advance: 10.0, index: i }).collect()
    }

    #[test]
    fn glyphs_follow_a_path_by_arc_length() {
        let left = PathPlacement { offset: 0.0, flip: false, align: Align::Left };
        // A straight line: glyph k's middle at 10 k + 5, upright, standing on the line (above it).
        let mut line = Path::new();
        line.cmds = vec![PathCmd::MoveTo(Vec2::new(0.0, 100.0)), PathCmd::LineTo(Vec2::new(200.0, 100.0))];
        let t = on_path(&boxes(3), &line, left).unwrap();
        assert_eq!(t.length, 200.0);
        for (k, (i, p, a)) in t.placed.iter().enumerate() {
            assert_eq!(*i, k as u32);
            assert!((p.x - (10.0 * k as f32 + 5.0)).abs() < 1e-3 && (p.y - 100.0).abs() < 1e-3 && a.abs() < 1e-4, "{p:?} {a}");
        }
        let b = t.outline.bounds();
        assert!((b.min.x).abs() < 1e-3 && (b.max.x - 30.0).abs() < 1e-3 && (b.min.y - 80.0).abs() < 1e-3 && (b.max.y - 100.0).abs() < 1e-3, "{b:?}");
        // Offset, centring and right alignment move the run along the path.
        let at = |offset, align| on_path(&boxes(3), &line, PathPlacement { offset, flip: false, align }).unwrap().placed[0].1.x;
        assert!((at(50.0, Align::Left) - 55.0).abs() < 1e-3);
        assert!((at(100.0, Align::Center) - 90.0).abs() < 1e-3);
        assert!((at(200.0, Align::Right) - 175.0).abs() < 1e-3);
        // Past an open path's end, glyphs are left out.
        let off = on_path(&boxes(3), &line, PathPlacement { offset: 180.0, flip: false, align: Align::Left }).unwrap();
        assert_eq!((off.placed.len(), off.hidden), (2, 1));
        // Flipped: walked from the other end, upside down, below the line.
        let f = on_path(&boxes(1), &line, PathPlacement { offset: 0.0, flip: true, align: Align::Left }).unwrap();
        let (_, p, a) = f.placed[0];
        assert!((p.x - 195.0).abs() < 1e-3 && (a.abs() - std::f32::consts::PI).abs() < 1e-3, "{p:?} {a}");
        assert!(f.outline.bounds().min.y >= 100.0 - 1e-3, "{:?}", f.outline.bounds());
    }

    #[test]
    fn glyphs_turn_with_a_circle_and_wrap_round_it() {
        // A circle of radius 100 (clockwise on screen, from the right): a glyph at distance s
        // sits at angle s / r, turned a quarter more.
        let circle = Path::ellipse(0.0, 0.0, 100.0, 100.0);
        let walk = anicraft_geom::walk::CurveWalk::new(&circle).unwrap();
        let len = walk.length();
        assert!((len - 2.0 * std::f32::consts::PI * 100.0).abs() < 0.5, "{len}");
        let t = on_path(&boxes(4), &circle, PathPlacement { offset: len - 15.0, flip: false, align: Align::Left }).unwrap();
        assert_eq!((t.placed.len(), t.hidden), (4, 0), "wrapped past the start of a closed path");
        for (_, p, a) in &t.placed {
            assert!((p.length() - 100.0).abs() < 0.05, "on the circle: {p:?}");
            let tangent = Vec2::new(a.cos(), a.sin());
            assert!(tangent.dot(*p).abs() < 0.5, "turned along it: {p:?} {a}");
        }
        // The second glyph's middle is at distance 0: the path's start.
        let (_, p, _) = t.placed[1];
        assert!((p - walk.at(0.0)).length() < 0.05, "{p:?}");
        // No length, no text.
        let mut dot = Path::new();
        dot.cmds = vec![PathCmd::MoveTo(Vec2::ZERO), PathCmd::LineTo(Vec2::ZERO)];
        assert!(on_path(&boxes(1), &dot, PathPlacement { offset: 0.0, flip: false, align: Align::Left }).is_none());
    }

    #[test]
    fn carets_stand_at_every_boundary_line_by_line() {
        let Some(fonts) = fonts() else { return };
        let l = fonts.layout("ab
cd", &style(20.0)).unwrap();
        assert_eq!(l.carets.len(), 2);
        let (a, b) = (&l.carets[0], &l.carets[1]);
        assert_eq!((a.start, a.end, b.start, b.end), (0, 2, 3, 5), "the line break belongs to neither line");
        assert_eq!(a.x_at(0), 0.0);
        assert!(a.x_at(1) > 0.0 && a.x_at(2) > a.x_at(1));
        assert!((a.x_at(2) - fonts.layout("ab", &style(20.0)).unwrap().width).abs() < 0.01, "after the last glyph");
        assert_eq!(a.index_at(-5.0), 0);
        assert_eq!(a.index_at(1000.0), 2);
        assert_eq!(b.index_at(a.x_at(1)), 4, "the same place on the next line");
        // Wrapped lines: the space stays on the first line, the next word starts the second.
        let w = fonts.layout("aaa bbb", &Style { width: Some(fonts.layout("aaa b", &style(20.0)).unwrap().width), ..style(20.0) }).unwrap();
        assert_eq!(w.carets.len(), 2);
        assert_eq!((w.carets[0].start, w.carets[0].end, w.carets[1].start, w.carets[1].end), (0, 4, 4, 7));
        // Empty text has one line with one place for the caret; centred text starts mid-box.
        let e = fonts.layout("", &style(20.0)).unwrap();
        assert_eq!(e.carets, vec![CaretLine { start: 0, end: 0, stops: vec![(0, 0.0)] }]);
        let c = fonts.layout("", &Style { width: Some(100.0), align: Align::Center, ..style(20.0) }).unwrap();
        assert_eq!(c.carets[0].x_at(0), 50.0);
    }
}
