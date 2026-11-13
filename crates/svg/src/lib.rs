//! AniCraft SVG import (layer L2): an SVG drawing into editable vector shapes and text.
//!
//! Written from the public W3C SVG 1.1 / SVG 2 and CSS Masking specifications. Supported: `svg`,
//! `g`, `a`, `switch`, `use`, `path`, `rect` (rounded), `circle`, `ellipse`, `line`, `polyline`
//! and `polygon`, read in document order. Styling comes from presentation attributes and
//! `style=""` declarations, with inheritance. Colours can be hex, `rgb()`/`rgba()`, CSS names,
//! `none`, `transparent` or `currentColor`. Also supported: `opacity`, `fill-opacity` and
//! `stroke-opacity`, `fill-rule`, `stroke-width` with units, `stroke-linecap`,
//! `stroke-linejoin`, `stroke-miterlimit`, `stroke-dasharray` and `stroke-dashoffset` (ADR 0046;
//! SVG's defaults: butt caps, miter joins, limit 4), the transform list (`matrix`,
//! `translate`, `scale`, `rotate`, `skewX`, `skewY`), `viewBox` with `preserveAspectRatio`, and
//! linear and radial gradients (both unit systems, `gradientTransform`, stops inherited through
//! `href`).
//!
//! **Clip paths and masks** (`clip`, ADR 0015) become AniCraft mask layers: the drawing comes
//! out as layers, bottom first, and artwork under a clip is a masked layer under a mask layer
//! holding the clip's shapes. A clip that cuts nothing is dropped, artwork it hides entirely is
//! left out, and clips inside clips are intersected exactly when all but one are convex
//! polygons. A `mask` comes across when its content is opaque (and white, for a luminance mask),
//! so that it is the coverage of its shapes. **Text** (`text`) becomes text blocks laid out by
//! `anicraft-text`, one per text chunk, with the missing-font reporting of Animate import
//! (ADR 0014).
//!
//! What AniCraft cannot represent yet is reported, never guessed silently: images, filters,
//! soft masks, patterns, style sheets, text on a path, reflected/repeated gradients and
//! skewed gradient transforms (approximated).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use anicraft_doc::{Cap, Dash, ElementKind, GradientStop, Join, Paint, Rgba, Shape, Spread, Stroke, StrokeStyle, MAX_FOCAL};
use anicraft_geom::svg::parse_path_data;
use anicraft_geom::{Affine, FillRule, Path, Rect, Vec2};
use anicraft_text::{FaceMatch, Fonts};
use anicraft_xml::Element as X;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::rc::Rc;
use std::sync::Arc;

mod clip;
mod text;

#[cfg(test)]
mod clip_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod text_tests;

/// Most shapes read from one file.
pub const MAX_SHAPES: usize = 200_000;
const MAX_DEPTH: usize = 64;
const MAX_USE_DEPTH: usize = 16;
/// Most elements visited while reading one file. Masks draw their content again, so this, not
/// the file's size, bounds the work.
const MAX_VISITS: usize = 2_000_000;

#[derive(Debug, thiserror::Error)]
pub enum SvgError {
    #[error(transparent)]
    Xml(#[from] anicraft_xml::XmlError),
    #[error("this is not an SVG drawing (its root element is <{0}>)")]
    NotSvg(String),
}

/// What could not come across, by kind ("2 images"), and the fonts the text asks for that
/// aren't on this computer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    pub skipped: BTreeMap<String, usize>,
    /// Font families the drawing's text asks for that aren't on this computer.
    pub missing_fonts: BTreeSet<String>,
    /// The family that text was laid out in instead.
    pub fallback_font: Option<String>,
}

impl Report {
    fn skip(&mut self, what: &str) {
        *self.skipped.entry(what.to_string()).or_default() += 1;
    }
    /// One line for a status bar: "Imported. Not yet supported: 2 images. Fonts not on this
    /// computer, drawn in Inter: Futura."
    pub fn summary(&self) -> String {
        if self.skipped.is_empty() && self.missing_fonts.is_empty() {
            return "Imported everything in the drawing.".into();
        }
        let mut out = String::from("Imported.");
        if !self.skipped.is_empty() {
            let parts: Vec<String> = self.skipped.iter().map(|(k, n)| format!("{n} {k}")).collect();
            out.push_str(&format!(" Not yet supported: {}.", parts.join(", ")));
        }
        if !self.missing_fonts.is_empty() {
            let names: Vec<&str> = self.missing_fonts.iter().map(String::as_str).collect();
            let instead = self.fallback_font.as_deref().map(|f| format!(", drawn in {f}")).unwrap_or_default();
            out.push_str(&format!(" Fonts not on this computer{instead}: {}.", names.join(", ")));
        }
        out
    }
}

/// One shape or text block, ready to become an element: its geometry in local coordinates,
/// the local → drawing transform, and the opacity of the element and its groups.
#[derive(Clone, Debug)]
pub struct ImportedElement {
    /// The element's `id`, if it had one.
    pub name: String,
    pub transform: Affine,
    pub alpha: f32,
    /// A shape or a text block.
    pub kind: ElementKind,
}

/// The artwork of a mask layer: its fills make the mask (ADR 0010).
#[derive(Clone, Debug)]
pub struct ImportedMask {
    /// The clip path's or mask's `id`, if it had one.
    pub name: String,
    pub elements: Vec<ImportedElement>,
    /// How the mask layer reveals (ADR 0070): coverage for clip paths and opaque masks, else
    /// the `mask`'s `mask-type` (luminance by default, or alpha), its artwork as drawn.
    pub mode: anicraft_doc::MaskMode,
}

/// Artwork drawn together on one layer, bottom first; masked by `mask` when there is one.
#[derive(Clone, Debug)]
pub struct ImportedLayer {
    pub mask: Option<ImportedMask>,
    pub elements: Vec<ImportedElement>,
}

/// An imported drawing. Coordinates are SVG user units (CSS pixels) of the drawing, with the
/// `viewBox` already applied; `width` × `height` is its size.
#[derive(Clone, Debug)]
pub struct SvgImport {
    pub width: f32,
    pub height: f32,
    /// Bottom layer first. A drawing without clips or masks is one unmasked layer.
    pub layers: Vec<ImportedLayer>,
    pub report: Report,
}

impl SvgImport {
    /// A drawing of these elements on one layer.
    pub fn single(width: f32, height: f32, elements: Vec<ImportedElement>) -> SvgImport {
        SvgImport { width, height, layers: vec![ImportedLayer { mask: None, elements }], report: Report::default() }
    }
    /// Every element that draws (not mask artwork), bottom first.
    pub fn elements(&self) -> impl Iterator<Item = &ImportedElement> {
        self.layers.iter().flat_map(|l| l.elements.iter())
    }
    /// Whether nothing in the drawing could be imported.
    pub fn is_empty(&self) -> bool {
        self.layers.iter().all(|l| l.elements.is_empty())
    }
}

/// Elements whose character data keeps its place among their children (SVG text content).
const TEXT_CONTENT: &[&str] = &["text", "tspan", "a", "textPath"];

/// Reads an SVG document, laying any text out with this computer's fonts.
pub fn import(bytes: &[u8]) -> Result<SvgImport, SvgError> {
    import_from(bytes, None)
}

/// Reads an SVG document, laying text out with `fonts` (tests, and hosts with their own font
/// library).
pub fn import_with_fonts(bytes: &[u8], fonts: &Fonts) -> Result<SvgImport, SvgError> {
    import_from(bytes, Some(fonts))
}

fn import_from(bytes: &[u8], fonts: Option<&Fonts>) -> Result<SvgImport, SvgError> {
    let root = anicraft_xml::parse_mixed(bytes, TEXT_CONTENT)?;
    if local(&root.name) != "svg" {
        return Err(SvgError::NotSvg(root.name.clone()));
    }
    let mut ids: HashMap<&str, &X> = HashMap::new();
    index(&root, &mut ids, 0);
    // Size: width/height, else the viewBox, else the CSS default 300 × 150.
    let vb = root.attr("viewBox").and_then(view_box);
    let fallback = vb.map_or((300.0, 150.0), |r| (r.width(), r.height()));
    let size = |key: &str, d: f32| root.attr(key).and_then(|v| length(v, d)).filter(|v| *v > 0.0).unwrap_or(d);
    let (w, h) = (size("width", fallback.0), size("height", fallback.1));
    let mut b = Builder {
        ids,
        report: Report::default(),
        items: Vec::new(),
        viewport: vb.map_or((w, h), |r| (r.width(), r.height())),
        fonts,
        faces: HashMap::new(),
        visits: 0,
        resolving: Vec::new(),
    };
    let base = match vb {
        Some(r) => view_box_transform(r, w, h, root.attr("preserveAspectRatio").unwrap_or("")),
        None => Affine::IDENTITY,
    };
    let style = Style::default().with(&root, &mut b.report);
    b.children(&root, &base, &style, 1.0, 0, 0);
    let layers = b.layers();
    Ok(SvgImport { width: w, height: h, layers, report: b.report })
}

/// The element's `id`, as an element name.
fn name_of(el: &X) -> String {
    el.attr("id").unwrap_or("").chars().take(128).collect()
}

/// Whether `visibility` hides the element.
fn hidden(d: &[(String, String)]) -> bool {
    get(d, "visibility").is_some_and(|v| v == "hidden" || v == "collapse")
}

/// An element name without its namespace prefix (`svg:path` → `path`).
fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

fn index<'a>(el: &'a X, ids: &mut HashMap<&'a str, &'a X>, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    if let Some(id) = el.attr("id") {
        ids.entry(id).or_insert(el);
    }
    for c in &el.children {
        index(c, ids, depth + 1);
    }
}

// ----- values -----------------------------------------------------------------------------------

/// Numbers in an attribute (separated by spaces and/or commas, signs and exponents allowed).
fn numbers(s: &str) -> Vec<f32> {
    let mut out = Vec::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let start = i;
        let c = b.get(i).copied().unwrap_or(b' ');
        if c.is_ascii_digit() || c == b'-' || c == b'+' || c == b'.' {
            i += 1;
            let mut seen_dot = c == b'.';
            let mut seen_e = false;
            while let Some(&d) = b.get(i) {
                let prev = b.get(i - 1).copied().unwrap_or(b' ');
                let ok = d.is_ascii_digit()
                    || (d == b'.' && !seen_dot && !seen_e)
                    || ((d == b'e' || d == b'E') && !seen_e)
                    || ((d == b'-' || d == b'+') && (prev == b'e' || prev == b'E'));
                if !ok {
                    break;
                }
                seen_dot |= d == b'.';
                seen_e |= d == b'e' || d == b'E';
                i += 1;
            }
            if let Some(v) = s.get(start..i).and_then(|t| t.parse::<f32>().ok()).filter(|v| v.is_finite()) {
                out.push(v);
            }
        } else {
            i += 1;
        }
    }
    out
}

/// A length in user units. `%` is relative to `reference`; `em`/`ex` assume a 16 px font.
fn length(v: &str, reference: f32) -> Option<f32> {
    let v = v.trim();
    // The unit is a known suffix, so exponents ("1e2px") still parse.
    let units: [(&str, f32); 9] =
        [("%", reference / 100.0), ("px", 1.0), ("pt", 96.0 / 72.0), ("pc", 16.0), ("mm", 96.0 / 25.4), ("cm", 96.0 / 2.54), ("in", 96.0), ("em", 16.0), ("ex", 8.0)];
    let (num, k) = units.iter().find_map(|(u, k)| v.strip_suffix(u).map(|n| (n, *k))).unwrap_or((v, 1.0));
    let n: f32 = num.trim().parse().ok().filter(|n: &f32| n.is_finite())?;
    Some(n * k).filter(|x| x.is_finite())
}

fn view_box(s: &str) -> Option<Rect> {
    let n = numbers(s);
    let (x, y, w, h) = (*n.first()?, *n.get(1)?, *n.get(2)?, *n.get(3)?);
    (w > 0.0 && h > 0.0).then(|| Rect::new(x, y, x + w, y + h))
}

/// The viewBox → viewport mapping for `preserveAspectRatio` (default `xMidYMid meet`).
fn view_box_transform(vb: Rect, w: f32, h: f32, par: &str) -> Affine {
    let (sx, sy) = (w / vb.width(), h / vb.height());
    let mut words = par.split_whitespace();
    let align = words.next().unwrap_or("xMidYMid");
    if align == "none" {
        return Affine::scale(sx, sy).mul(&Affine::translate(-vb.min.x, -vb.min.y));
    }
    let slice = words.next() == Some("slice");
    let s = if slice { sx.max(sy) } else { sx.min(sy) };
    let frac = |key: &str| if align.contains(&format!("{key}Min")) { 0.0 } else if align.contains(&format!("{key}Max")) { 1.0 } else { 0.5 };
    let tx = (w - vb.width() * s) * frac("x");
    let ty = (h - vb.height() * s) * frac("Y");
    Affine::translate(tx, ty).mul(&Affine::scale(s, s)).mul(&Affine::translate(-vb.min.x, -vb.min.y))
}

/// An SVG transform list.
fn transform(s: &str) -> Affine {
    let mut m = Affine::IDENTITY;
    let mut rest = s;
    while let Some(open) = rest.find('(') {
        let name = rest.get(..open).unwrap_or("").trim().trim_start_matches(',').trim();
        let Some(close) = rest.get(open..).and_then(|r| r.find(')')).map(|c| c + open) else { break };
        let a = numbers(rest.get(open + 1..close).unwrap_or(""));
        let g = |i: usize| a.get(i).copied();
        let t = match (name, a.len()) {
            ("matrix", 6..) => Affine { m: [a[0], a[1], a[2], a[3], a[4], a[5]] },
            ("translate", 1..) => Affine::translate(a[0], g(1).unwrap_or(0.0)),
            ("scale", 1..) => Affine::scale(a[0], g(1).unwrap_or(a[0])),
            ("rotate", 1..) => {
                let r = Affine::rotate(a[0].to_radians());
                match (g(1), g(2)) {
                    (Some(cx), Some(cy)) => Affine::translate(cx, cy).mul(&r).mul(&Affine::translate(-cx, -cy)),
                    _ => r,
                }
            }
            ("skewX", 1..) => Affine { m: [1.0, 0.0, a[0].to_radians().tan(), 1.0, 0.0, 0.0] },
            ("skewY", 1..) => Affine { m: [1.0, a[0].to_radians().tan(), 0.0, 1.0, 0.0, 0.0] },
            _ => Affine::IDENTITY,
        };
        m = m.mul(&t);
        rest = rest.get(close + 1..).unwrap_or("");
    }
    if m.is_finite() { m } else { Affine::IDENTITY }
}

fn named(name: &str) -> Option<u32> {
    Some(match name {
        "black" => 0x000000,
        "white" => 0xffffff,
        "red" => 0xff0000,
        "lime" => 0x00ff00,
        "green" => 0x008000,
        "blue" => 0x0000ff,
        "yellow" => 0xffff00,
        "cyan" | "aqua" => 0x00ffff,
        "magenta" | "fuchsia" => 0xff00ff,
        "gray" | "grey" => 0x808080,
        "silver" => 0xc0c0c0,
        "maroon" => 0x800000,
        "olive" => 0x808000,
        "purple" => 0x800080,
        "teal" => 0x008080,
        "navy" => 0x000080,
        "orange" => 0xffa500,
        "pink" => 0xffc0cb,
        "brown" => 0xa52a2a,
        "gold" => 0xffd700,
        "indigo" => 0x4b0082,
        "violet" => 0xee82ee,
        "coral" => 0xff7f50,
        "salmon" => 0xfa8072,
        "tomato" => 0xff6347,
        "crimson" => 0xdc143c,
        "khaki" => 0xf0e68c,
        "beige" => 0xf5f5dc,
        "ivory" => 0xfffff0,
        "tan" => 0xd2b48c,
        "chocolate" => 0xd2691e,
        "sienna" => 0xa0522d,
        "turquoise" => 0x40e0d0,
        "skyblue" => 0x87ceeb,
        "steelblue" => 0x4682b4,
        "royalblue" => 0x4169e1,
        "darkblue" => 0x00008b,
        "lightblue" => 0xadd8e6,
        "darkgreen" => 0x006400,
        "lightgreen" => 0x90ee90,
        "forestgreen" => 0x228b22,
        "darkred" => 0x8b0000,
        "darkgray" | "darkgrey" => 0xa9a9a9,
        "lightgray" | "lightgrey" => 0xd3d3d3,
        "dimgray" | "dimgrey" => 0x696969,
        "darkorange" => 0xff8c00,
        "orchid" => 0xda70d6,
        "plum" => 0xdda0dd,
        "lavender" => 0xe6e6fa,
        "slategray" | "slategrey" => 0x708090,
        "whitesmoke" => 0xf5f5f5,
        "gainsboro" => 0xdcdcdc,
        _ => return None,
    })
}

/// A CSS colour (`#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`, `rgb()`, `rgba()`, a name).
fn color(v: &str) -> Option<Rgba> {
    let v = v.trim();
    let lower = v.to_ascii_lowercase();
    if let Some(h) = lower.strip_prefix('#') {
        let expand = |s: &str| s.chars().flat_map(|c| [c, c]).collect::<String>();
        return match h.len() {
            3 | 4 => Rgba::parse_hex(&expand(h)),
            6 | 8 => Rgba::parse_hex(h),
            _ => None,
        };
    }
    if let Some(args) = lower.strip_prefix("rgba(").or_else(|| lower.strip_prefix("rgb(")) {
        let args = args.trim_end_matches(')');
        let parts: Vec<&str> = args.split([',', ' ', '/']).filter(|p| !p.is_empty()).collect();
        let chan = |p: &str| -> Option<f32> {
            match p.strip_suffix('%') {
                Some(pc) => pc.parse::<f32>().ok().map(|x| x / 100.0),
                None => p.parse::<f32>().ok().map(|x| x / 255.0),
            }
            .filter(|x| x.is_finite())
            .map(|x| x.clamp(0.0, 1.0))
        };
        let alpha = |p: &str| -> Option<f32> {
            match p.strip_suffix('%') {
                Some(pc) => pc.parse::<f32>().ok().map(|x| x / 100.0),
                None => p.parse::<f32>().ok(),
            }
            .filter(|x| x.is_finite())
            .map(|x| x.clamp(0.0, 1.0))
        };
        let (r, g, b) = (chan(parts.first()?)?, chan(parts.get(1)?)?, chan(parts.get(2)?)?);
        let a = parts.get(3).map_or(Some(1.0), |p| alpha(p))?;
        return Some(Rgba::new(r, g, b, a));
    }
    if lower == "transparent" {
        return Some(Rgba::TRANSPARENT);
    }
    named(&lower).map(Rgba::hex)
}

// ----- style ------------------------------------------------------------------------------------

/// What `fill` or `stroke` asks for.
#[derive(Clone, Debug, PartialEq)]
enum PaintSpec {
    None,
    Color(Rgba),
    CurrentColor,
    /// A paint server by id, with the fallback after it (`url(#g) red`).
    Url(String, Option<Rgba>),
}

fn paint_spec(v: &str) -> Option<PaintSpec> {
    let v = v.trim();
    if v == "none" {
        return Some(PaintSpec::None);
    }
    if v.eq_ignore_ascii_case("currentcolor") {
        return Some(PaintSpec::CurrentColor);
    }
    if let Some(rest) = v.strip_prefix("url(") {
        let close = rest.find(')')?;
        let id = rest.get(..close)?.trim().trim_matches(['"', '\'']).trim_start_matches('#').to_string();
        let fallback = rest.get(close + 1..).map(str::trim).filter(|f| !f.is_empty()).and_then(color);
        return Some(PaintSpec::Url(id, fallback));
    }
    color(v).map(PaintSpec::Color)
}

/// Where a text chunk sits relative to its position (`text-anchor`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Anchor {
    #[default]
    Start,
    Middle,
    End,
}

/// Inherited presentation properties.
#[derive(Clone, Debug)]
struct Style {
    fill: PaintSpec,
    fill_opacity: f32,
    fill_rule: FillRule,
    clip_rule: FillRule,
    stroke: PaintSpec,
    stroke_opacity: f32,
    stroke_width: f32,
    cap: Cap,
    join: Join,
    miter_limit: f32,
    /// `stroke-dasharray` lengths (user units); `None`: solid.
    dash: Option<Vec<f32>>,
    dash_offset: f32,
    current: Rgba,
    /// The CSS `font-family` list, as written.
    font_family: String,
    font_size: f32,
    bold: bool,
    italic: bool,
    anchor: Anchor,
    letter_spacing: f32,
    /// `xml:space="preserve"`: keep every space.
    preserve: bool,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            fill: PaintSpec::Color(Rgba::BLACK),
            fill_opacity: 1.0,
            fill_rule: FillRule::NonZero,
            clip_rule: FillRule::NonZero,
            stroke: PaintSpec::None,
            stroke_opacity: 1.0,
            stroke_width: 1.0,
            // SVG's initial values, not AniCraft's (round) defaults.
            cap: Cap::Butt,
            join: Join::Miter,
            miter_limit: 4.0,
            dash: None,
            dash_offset: 0.0,
            current: Rgba::BLACK,
            font_family: String::new(),
            font_size: 16.0,
            bold: false,
            italic: false,
            anchor: Anchor::Start,
            letter_spacing: 0.0,
            preserve: false,
        }
    }
}

/// An element's declarations: presentation attributes, overridden by `style=""`.
fn declarations(el: &X) -> Vec<(String, String)> {
    const KEYS: &[&str] = &[
        "fill",
        "fill-opacity",
        "fill-rule",
        "clip-rule",
        "stroke",
        "stroke-opacity",
        "stroke-width",
        "stroke-linecap",
        "stroke-linejoin",
        "stroke-miterlimit",
        "stroke-dasharray",
        "stroke-dashoffset",
        "color",
        "opacity",
        "display",
        "visibility",
        "filter",
        "clip-path",
        "mask",
        "mask-type",
        "stop-color",
        "stop-opacity",
        "font-family",
        "font-size",
        "font-weight",
        "font-style",
        "text-anchor",
        "letter-spacing",
        "text-decoration",
        "writing-mode",
    ];
    let mut out: Vec<(String, String)> = KEYS.iter().filter_map(|k| el.attr(k).map(|v| (k.to_string(), v.trim().to_string()))).collect();
    if let Some(style) = el.attr("style") {
        for decl in style.split(';') {
            if let Some((k, v)) = decl.split_once(':') {
                let v = v.trim().trim_end_matches("!important").trim();
                out.retain(|(key, _)| key != k.trim());
                out.push((k.trim().to_ascii_lowercase(), v.to_string()));
            }
        }
    }
    out
}

/// A `stroke-dasharray`: lengths separated by commas or spaces. `none`, an empty list, a
/// negative or unreadable length all draw solid (SVG renders an invalid array as none).
fn dash_array(v: &str) -> Option<Vec<f32>> {
    if v.trim() == "none" {
        return None;
    }
    let mut out = Vec::new();
    for part in v.split(|c: char| c == ',' || c.is_whitespace()).filter(|p| !p.trim().is_empty()).take(1024) {
        let x = length(part, 100.0)?;
        if x < 0.0 {
            return None;
        }
        out.push(x);
    }
    (!out.is_empty()).then_some(out)
}

fn get<'a>(d: &'a [(String, String)], key: &str) -> Option<&'a str> {
    d.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

fn opacity(v: &str) -> Option<f32> {
    let v = v.trim();
    let x = match v.strip_suffix('%') {
        Some(p) => p.parse::<f32>().ok()? / 100.0,
        None => v.parse::<f32>().ok()?,
    };
    x.is_finite().then(|| x.clamp(0.0, 1.0))
}

/// A `font-size`: a keyword, a length, or relative to the parent's size (`em`, `ex`, `%`).
fn font_size(v: &str, parent: f32) -> Option<f32> {
    let v = v.trim();
    let keyword = match v {
        "xx-small" => Some(9.0),
        "x-small" => Some(10.0),
        "small" => Some(13.0),
        "medium" => Some(16.0),
        "large" => Some(18.0),
        "x-large" => Some(24.0),
        "xx-large" => Some(32.0),
        "larger" => Some(parent * 1.2),
        "smaller" => Some(parent / 1.2),
        _ => None,
    };
    let rel = |suffix: &str, k: f32| v.strip_suffix(suffix).and_then(|n| n.trim().parse::<f32>().ok()).map(|n| n * k);
    let size = keyword.or_else(|| rel("em", parent)).or_else(|| rel("ex", parent * 0.5)).or_else(|| rel("%", parent / 100.0)).or_else(|| length(v, parent))?;
    size.is_finite().then(|| size.clamp(0.0, 2000.0))
}

impl Style {
    /// This style inherited by `el`, with `el`'s own declarations applied.
    fn with(&self, el: &X, report: &mut Report) -> Style {
        let d = declarations(el);
        let mut s = self.clone();
        if let Some(c) = get(&d, "color").and_then(color) {
            s.current = c;
        }
        if let Some(p) = get(&d, "fill").and_then(paint_spec) {
            s.fill = p;
        }
        if let Some(p) = get(&d, "stroke").and_then(paint_spec) {
            s.stroke = p;
        }
        if let Some(o) = get(&d, "fill-opacity").and_then(opacity) {
            s.fill_opacity = o;
        }
        if let Some(o) = get(&d, "stroke-opacity").and_then(opacity) {
            s.stroke_opacity = o;
        }
        if let Some(w) = get(&d, "stroke-width").and_then(|v| length(v, 100.0)).filter(|w| *w >= 0.0) {
            s.stroke_width = w;
        }
        let rule = |v: Option<&str>, old: FillRule| match v {
            Some("evenodd") => FillRule::EvenOdd,
            Some("nonzero") => FillRule::NonZero,
            _ => old,
        };
        s.fill_rule = rule(get(&d, "fill-rule"), s.fill_rule);
        s.clip_rule = rule(get(&d, "clip-rule"), s.clip_rule);
        if let Some(f) = get(&d, "font-family").filter(|f| !f.is_empty() && *f != "inherit") {
            s.font_family = f.chars().take(512).collect();
        }
        if let Some(z) = get(&d, "font-size").and_then(|v| font_size(v, self.font_size)) {
            s.font_size = z;
        }
        match get(&d, "font-weight") {
            Some("bold" | "bolder") => s.bold = true,
            Some("normal" | "lighter") => s.bold = false,
            Some(n) => {
                if let Ok(w) = n.trim().parse::<f32>() {
                    s.bold = w >= 600.0;
                }
            }
            None => {}
        }
        match get(&d, "font-style") {
            Some("italic" | "oblique") => s.italic = true,
            Some("normal") => s.italic = false,
            _ => {}
        }
        match get(&d, "text-anchor") {
            Some("start") => s.anchor = Anchor::Start,
            Some("middle") => s.anchor = Anchor::Middle,
            Some("end") => s.anchor = Anchor::End,
            _ => {}
        }
        match get(&d, "letter-spacing") {
            Some("normal") => s.letter_spacing = 0.0,
            Some(v) => {
                let em = v.strip_suffix("em").and_then(|n| n.trim().parse::<f32>().ok()).map(|n| n * s.font_size);
                if let Some(x) = em.or_else(|| length(v, s.font_size)).filter(|x| x.is_finite()) {
                    s.letter_spacing = x.clamp(-1000.0, 1000.0);
                }
            }
            None => {}
        }
        match el.attr("xml:space") {
            Some("preserve") => s.preserve = true,
            Some("default") => s.preserve = false,
            _ => {}
        }
        match get(&d, "stroke-linecap") {
            Some("butt") => s.cap = Cap::Butt,
            Some("round") => s.cap = Cap::Round,
            Some("square") => s.cap = Cap::Square,
            _ => {}
        }
        match get(&d, "stroke-linejoin") {
            Some("miter" | "miter-clip") => s.join = Join::Miter,
            Some("round" | "arcs") => s.join = Join::Round,
            Some("bevel") => s.join = Join::Bevel,
            _ => {}
        }
        if let Some(m) = get(&d, "stroke-miterlimit").and_then(|v| v.trim().parse::<f32>().ok()).filter(|m| m.is_finite() && *m >= 1.0) {
            s.miter_limit = m;
        }
        if let Some(v) = get(&d, "stroke-dasharray").filter(|v| *v != "inherit") {
            s.dash = dash_array(v);
            if s.dash.as_ref().is_some_and(|l| l.len() > anicraft_doc::stroke_style::MAX_DASH_ENTRIES) {
                report.skip("dash patterns of more than 16 lengths (shortened)");
            }
        }
        if let Some(o) = get(&d, "stroke-dashoffset").and_then(|v| length(v, 100.0)) {
            s.dash_offset = o;
        }
        if get(&d, "filter").is_some_and(|v| v != "none") {
            report.skip("filters");
        }
        s
    }
}

// ----- building ---------------------------------------------------------------------------------

/// Something drawn, and the clips (clip paths and masks) of everything around it, innermost
/// first. The clips are resolved into layers at the end ([`Builder::layers`]).
struct Item {
    el: ImportedElement,
    clips: Vec<Rc<clip::Region>>,
}

struct Builder<'a> {
    ids: HashMap<&'a str, &'a X>,
    report: Report,
    items: Vec<Item>,
    /// The size percentages refer to (the viewBox, or the drawing size).
    viewport: (f32, f32),
    /// The fonts text is laid out with; this computer's, found on the first text.
    fonts: Option<&'a Fonts>,
    /// Font name → what it is on this computer, looked up once per name.
    faces: HashMap<String, FaceMatch>,
    /// Elements visited so far (see [`MAX_VISITS`]).
    visits: usize,
    /// The clip paths and masks being read, innermost last (a reference back to one of them is
    /// a loop).
    resolving: Vec<&'a X>,
}

impl<'a> Builder<'a> {
    fn children(&mut self, el: &'a X, m: &Affine, style: &Style, alpha: f32, depth: usize, uses: usize) {
        for c in &el.children {
            self.node(c, m, style, alpha, depth + 1, uses);
        }
    }

    fn node(&mut self, el: &'a X, parent: &Affine, inherited: &Style, alpha: f32, depth: usize, uses: usize) {
        if depth > MAX_DEPTH || self.items.len() >= MAX_SHAPES {
            return;
        }
        self.visits += 1;
        if self.visits > MAX_VISITS {
            if self.visits == MAX_VISITS + 1 {
                self.report.skip("parts of a drawing too complex to read (left out)");
            }
            return;
        }
        let d = declarations(el);
        if get(&d, "display") == Some("none") {
            return;
        }
        let name = local(&el.name);
        // Containers that never draw by themselves.
        match name {
            "defs" | "linearGradient" | "radialGradient" | "stop" | "symbol" | "metadata" | "title" | "desc" | "clipPath" | "mask" | "marker" | "pattern" | "filter" | "script" => return,
            anicraft_xml::TEXT_NODE => return,
            "style" => {
                self.report.skip("style sheets (inline styles are read)");
                return;
            }
            "image" => {
                self.report.skip("images");
                return;
            }
            "foreignObject" => {
                self.report.skip("embedded HTML");
                return;
            }
            _ => {}
        }
        let style = inherited.with(el, &mut self.report);
        let alpha = alpha * get(&d, "opacity").and_then(opacity).unwrap_or(1.0);
        let m = parent.mul(&el.attr("transform").map_or(Affine::IDENTITY, transform));
        let start = self.items.len();
        self.draw(el, name, &d, &m, &style, alpha, depth, uses);
        if self.items.len() > start {
            self.clip(&d, &m, start, depth);
        }
    }

    /// What `el` draws, into `items`.
    #[allow(clippy::too_many_arguments)]
    fn draw(&mut self, el: &'a X, name: &str, d: &[(String, String)], m: &Affine, style: &Style, alpha: f32, depth: usize, uses: usize) {
        match name {
            "svg" if depth > 0 => {
                // A nested viewport: x/y offset, then its own viewBox.
                let (vw, vh) = self.viewport;
                let x = el.attr("x").and_then(|v| length(v, vw)).unwrap_or(0.0);
                let y = el.attr("y").and_then(|v| length(v, vh)).unwrap_or(0.0);
                let w = el.attr("width").and_then(|v| length(v, vw)).unwrap_or(vw);
                let h = el.attr("height").and_then(|v| length(v, vh)).unwrap_or(vh);
                let inner = match el.attr("viewBox").and_then(view_box) {
                    Some(vb) => view_box_transform(vb, w, h, el.attr("preserveAspectRatio").unwrap_or("")),
                    None => Affine::IDENTITY,
                };
                let m = m.mul(&Affine::translate(x, y)).mul(&inner);
                self.children(el, &m, style, alpha, depth, uses);
            }
            "svg" | "g" | "a" | "switch" => self.children(el, m, style, alpha, depth, uses),
            "use" => {
                let href = el.attr("href").or_else(|| el.attr("xlink:href")).unwrap_or("").trim_start_matches('#');
                let Some(target) = self.ids.get(href).copied() else {
                    self.report.skip("references to missing elements");
                    return;
                };
                if uses >= MAX_USE_DEPTH {
                    self.report.skip("references nested too deeply");
                    return;
                }
                let (vw, vh) = self.viewport;
                let x = el.attr("x").and_then(|v| length(v, vw)).unwrap_or(0.0);
                let y = el.attr("y").and_then(|v| length(v, vh)).unwrap_or(0.0);
                let m = m.mul(&Affine::translate(x, y));
                if local(&target.name) == "symbol" {
                    let inner = style.with(target, &mut self.report);
                    self.children(target, &m, &inner, alpha, depth, uses + 1);
                } else {
                    self.node(target, &m, style, alpha, depth + 1, uses + 1);
                }
            }
            "text" => {
                if hidden(d) {
                    return;
                }
                for el in self.text(el, m, style, alpha) {
                    self.items.push(Item { el, clips: Vec::new() });
                }
            }
            _ => {
                let Some((path, closed_shape)) = self.geometry(el, name) else { return };
                if !m.is_finite() || path.is_empty() || hidden(d) {
                    return;
                }
                let bounds = path.bounds();
                let fill = if closed_shape { self.paint(&style.fill, style.fill_opacity, style, &bounds) } else { None };
                let stroke = self.stroke(style, &bounds);
                if fill.is_none() && stroke.is_none() {
                    return;
                }
                let shape = Shape { path: Arc::new(path), fill, stroke, fill_rule: style.fill_rule, merge: false, warp: None };
                self.items.push(Item { el: ImportedElement { name: name_of(el), transform: *m, alpha, kind: ElementKind::Shape(shape) }, clips: Vec::new() });
            }
        }
    }

    /// The fonts to lay text out with: the ones given, else this computer's (scanned once).
    fn fonts(&mut self) -> &'a Fonts {
        match self.fonts {
            Some(f) => f,
            None => {
                let f = Fonts::get();
                self.fonts = Some(f);
                f
            }
        }
    }

    /// The element's path in its own coordinates, and whether it can be filled (lines can't).
    fn geometry(&mut self, el: &X, name: &str) -> Option<(Path, bool)> {
        let (vw, vh) = self.viewport;
        let diag = ((vw * vw + vh * vh) / 2.0).sqrt();
        let len = |key: &str, reference: f32| el.attr(key).and_then(|v| length(v, reference)).unwrap_or(0.0);
        match name {
            "path" => match parse_path_data(el.attr("d").unwrap_or("")) {
                Ok(p) => Some((p, true)),
                Err(_) => {
                    self.report.skip("unreadable paths");
                    None
                }
            },
            "rect" => {
                let (x, y, w, h) = (len("x", vw), len("y", vh), len("width", vw), len("height", vh));
                if !(w > 0.0 && h > 0.0) {
                    return None;
                }
                let rx = el.attr("rx").and_then(|v| length(v, vw));
                let ry = el.attr("ry").and_then(|v| length(v, vh));
                let (rx, ry) = match (rx, ry) {
                    (Some(a), Some(b)) => (a, b),
                    (Some(a), None) | (None, Some(a)) => (a, a),
                    (None, None) => (0.0, 0.0),
                };
                Some((rounded_rect(x, y, w, h, rx.clamp(0.0, w / 2.0), ry.clamp(0.0, h / 2.0)), true))
            }
            "circle" => {
                let r = len("r", diag);
                (r > 0.0).then(|| (Path::ellipse(len("cx", vw), len("cy", vh), r, r), true))
            }
            "ellipse" => {
                let (rx, ry) = (len("rx", vw), len("ry", vh));
                (rx > 0.0 && ry > 0.0).then(|| (Path::ellipse(len("cx", vw), len("cy", vh), rx, ry), true))
            }
            "line" => Some((Path::line(Vec2::new(len("x1", vw), len("y1", vh)), Vec2::new(len("x2", vw), len("y2", vh))), false)),
            "polyline" | "polygon" => {
                let n = numbers(el.attr("points").unwrap_or(""));
                let mut p = Path::new();
                for (i, &[x, y]) in n.as_chunks::<2>().0.iter().enumerate() {
                    let q = Vec2::new(x, y);
                    if i == 0 { p.move_to(q) } else { p.line_to(q) };
                }
                if name == "polygon" && !p.is_empty() {
                    p.close();
                }
                (n.len() >= 4).then_some((p, true))
            }
            other => {
                self.report.skip(&format!("<{other}> elements"));
                None
            }
        }
    }

    fn stroke(&mut self, style: &Style, bounds: &Rect) -> Option<Stroke> {
        if style.stroke_width <= 0.0 {
            return None;
        }
        let color = match self.paint(&style.stroke, style.stroke_opacity, style, bounds)? {
            Paint::Solid(c) => c,
            other => {
                self.report.skip("gradient strokes (drawn in one colour)");
                other.primary_color()
            }
        };
        let dash = style.dash.as_ref().and_then(|l| Dash::Dashed { lengths: l.iter().copied().collect(), offset: style.dash_offset }.sanitized());
        Some(Stroke::new(color, style.stroke_width).with_style(StrokeStyle { cap: style.cap, join: style.join, miter_limit: style.miter_limit, dash }))
    }

    /// A fill or stroke paint, with `opacity` folded in; `None` draws nothing.
    fn paint(&mut self, spec: &PaintSpec, opacity: f32, style: &Style, bounds: &Rect) -> Option<Paint> {
        let solid = |c: Rgba| (c.a * opacity > 0.0).then(|| Paint::Solid(c.with_alpha(c.a * opacity)));
        match spec {
            PaintSpec::None => None,
            PaintSpec::Color(c) => solid(*c),
            PaintSpec::CurrentColor => solid(style.current),
            PaintSpec::Url(id, fallback) => match self.gradient(id, opacity, bounds) {
                Some(p) => Some(p),
                None => {
                    if !self.ids.get(id.as_str()).is_some_and(|e| matches!(local(&e.name), "linearGradient" | "radialGradient")) {
                        self.report.skip("pattern or unknown fills");
                    }
                    fallback.and_then(solid)
                }
            },
        }
    }

    /// The gradient `id` as a paint for a shape with these local `bounds`.
    fn gradient(&mut self, id: &str, opacity: f32, bounds: &Rect) -> Option<Paint> {
        // Follow the href chain: attributes and stops come from the nearest gradient that has them.
        let mut chain: Vec<&X> = Vec::new();
        let mut cur = self.ids.get(id).copied();
        while let Some(g) = cur {
            if chain.len() >= 8 || !matches!(local(&g.name), "linearGradient" | "radialGradient") {
                break;
            }
            chain.push(g);
            let next = g.attr("href").or_else(|| g.attr("xlink:href")).map(|h| h.trim_start_matches('#'));
            cur = next.and_then(|h| self.ids.get(h).copied()).filter(|n| !chain.iter().any(|c| std::ptr::eq(*c, *n)));
        }
        let first = *chain.first()?;
        let attr = |key: &str| chain.iter().find_map(|g| g.attr(key));
        let stop_el = chain.iter().find(|g| g.children.iter().any(|c| local(&c.name) == "stop"))?;
        let mut stops: Vec<GradientStop> = Vec::new();
        let mut last = 0.0f32;
        for s in stop_el.children.iter().filter(|c| local(&c.name) == "stop") {
            let d = declarations(s);
            let off = s.attr("offset").and_then(self::opacity).unwrap_or(0.0).clamp(last, 1.0);
            last = off;
            let c = get(&d, "stop-color").and_then(color).unwrap_or(Rgba::BLACK);
            let a = get(&d, "stop-opacity").and_then(self::opacity).unwrap_or(1.0);
            stops.push(GradientStop { offset: off, color: c.with_alpha(c.a * a * opacity) });
        }
        if stops.is_empty() {
            return None;
        }
        if stops.len() == 1 {
            return stops.first().map(|s| Paint::Solid(s.color));
        }
        let user = attr("gradientUnits") == Some("userSpaceOnUse");
        let (vw, vh) = self.viewport;
        // Gradient space → the shape's local space.
        let units = if user { Affine::IDENTITY } else { Affine::translate(bounds.min.x, bounds.min.y).mul(&Affine::scale(bounds.width().max(1e-6), bounds.height().max(1e-6))) };
        let g = units.mul(&attr("gradientTransform").map_or(Affine::IDENTITY, transform));
        if !g.is_finite() {
            return None;
        }
        let coord = |key: &str, default: f32, reference: f32| -> f32 {
            match attr(key) {
                Some(v) if !user => v.trim().strip_suffix('%').and_then(|p| p.parse::<f32>().ok()).map(|p| p / 100.0).or_else(|| v.trim().parse().ok()).filter(|x: &f32| x.is_finite()).unwrap_or(default),
                Some(v) => length(v, reference).unwrap_or(default),
                None => if user { default * reference } else { default },
            }
        };
        // What shows past the ends and how the stops mix carry over (ADR 0074).
        let spread = attr("spreadMethod").and_then(|s| Spread::from_key(s.trim())).unwrap_or_default();
        let linear_rgb = attr("color-interpolation").is_some_and(|s| s.trim() == "linearRGB");
        let diag = ((vw * vw + vh * vh) / 2.0).sqrt();
        Some(match local(&first.name) {
            "linearGradient" => {
                let (p0, p1) = (Vec2::new(coord("x1", 0.0, vw), coord("y1", 0.0, vh)), Vec2::new(coord("x2", 1.0, vw), coord("y2", 0.0, vh)));
                // Exact when the transform keeps the gradient's bands perpendicular to its axis
                // (any similarity, or a stretch along the axis); otherwise the bands tilt.
                let (dir, band) = (g.apply_vector(p1 - p0), g.apply_vector((p1 - p0).perp()));
                if dir.dot(band).abs() > 1e-3 * dir.length() * band.length() {
                    self.report.skip("stretched or skewed gradients (approximated)");
                }
                Paint::Linear { start: g.apply(p0), end: g.apply(p1), stops, spread, linear_rgb }
            }
            _ => {
                // Exact: the ring is carried by the gradient transform (any ellipse), turned so
                // its own x axis runs through the focal point.
                let (cx, cy) = (coord("cx", 0.5, vw), coord("cy", 0.5, vh));
                let (fx, fy) = (coord("fx", cx, vw), coord("fy", cy, vh));
                let r = coord("r", 0.5, diag);
                if !(r.is_finite() && r > 0.0) {
                    return stops.last().map(|s| Paint::Solid(s.color));
                }
                let off = Vec2::new(fx - cx, fy - cy);
                let (focal, turn) = if off.length() > 1e-6 * r { ((off.length() / r).min(MAX_FOCAL), off.y.atan2(off.x)) } else { (0.0, 0.0) };
                let frame = g.mul(&Affine::translate(cx, cy)).mul(&Affine::rotate(turn)).mul(&Affine::scale(r, r));
                Paint::Radial { center: Vec2::ZERO, radius: 1.0, stops, spread, linear_rgb, focal, matrix: Affine::IDENTITY }.with_frame(&frame, None)
            }
        })
    }
}

/// A rectangle with elliptical corners.
fn rounded_rect(x: f32, y: f32, w: f32, h: f32, rx: f32, ry: f32) -> Path {
    if rx <= 0.0 || ry <= 0.0 {
        return Path::rect(x, y, w, h);
    }
    const K: f32 = 0.552_284_8;
    let (kx, ky) = (rx * K, ry * K);
    let (x1, y1) = (x + w, y + h);
    let mut p = Path::new();
    p.move_to(Vec2::new(x + rx, y))
        .line_to(Vec2::new(x1 - rx, y))
        .cubic_to(Vec2::new(x1 - rx + kx, y), Vec2::new(x1, y + ry - ky), Vec2::new(x1, y + ry))
        .line_to(Vec2::new(x1, y1 - ry))
        .cubic_to(Vec2::new(x1, y1 - ry + ky), Vec2::new(x1 - rx + kx, y1), Vec2::new(x1 - rx, y1))
        .line_to(Vec2::new(x + rx, y1))
        .cubic_to(Vec2::new(x + rx - kx, y1), Vec2::new(x, y1 - ry + ky), Vec2::new(x, y1 - ry))
        .line_to(Vec2::new(x, y + ry))
        .cubic_to(Vec2::new(x, y + ry - ky), Vec2::new(x + rx - kx, y), Vec2::new(x + rx, y))
        .close();
    p
}
