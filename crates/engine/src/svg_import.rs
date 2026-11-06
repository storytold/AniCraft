//! Static SVG import into editable AniCraft shapes.

use anicraft_doc::{
    ColorEffect, Document, Element, ElementId, ElementKind, Keyframe, Layer, LayerId, Paint, Rgba,
    Shape, Stage, Stroke,
};
use anicraft_geom::{Affine, FillRule, Path, Vec2, svg::parse_path_data};
use anicraft_xfl::xml::{self, Element as XmlElement};
use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path as FsPath;
use std::sync::Arc;

const MAX_SVG_BYTES: u64 = 32 * 1024 * 1024;
const MAX_IMPORTED_SHAPES: usize = 100_000;
const MAX_XML_ELEMENTS: usize = 200_000;
const MAX_GEOMETRY_VALUES: usize = 200_000;
const MAX_IMPORT_DEPTH: usize = 128;
const DEFAULT_WIDTH: f32 = 300.0;
const DEFAULT_HEIGHT: f32 = 150.0;

#[derive(Debug, thiserror::Error)]
pub enum SvgImportError {
    #[error("{0}")]
    Xml(#[from] xml::XmlError),
    #[error("not an SVG document: {0}")]
    NotSvg(String),
    #[error("invalid SVG: {0}")]
    Invalid(String),
    #[error("I/O: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Report {
    pub skipped: BTreeMap<String, usize>,
    pub notes: Vec<String>,
}

impl Report {
    fn skip(&mut self, what: &str) {
        let count = self.skipped.entry(what.to_string()).or_default();
        *count = count.saturating_add(1);
    }

    pub fn summary(&self) -> String {
        if self.skipped.is_empty() && self.notes.is_empty() {
            return "Imported everything in the file.".into();
        }
        let mut parts: Vec<String> = self
            .skipped
            .iter()
            .map(|(kind, count)| format!("{count} {kind}"))
            .collect();
        parts.extend(self.notes.iter().cloned());
        format!("Imported. Not yet supported: {}.", parts.join(", "))
    }
}

#[derive(Clone)]
struct Style {
    fill: Option<String>,
    stroke: Option<String>,
    fill_opacity: f32,
    stroke_opacity: f32,
    opacity: f32,
    stroke_width: f32,
    fill_rule: FillRule,
    visible: bool,
    displayed: bool,
}

impl Default for Style {
    fn default() -> Self {
        Self {
            fill: Some("black".into()),
            stroke: None,
            fill_opacity: 1.0,
            stroke_opacity: 1.0,
            opacity: 1.0,
            stroke_width: 1.0,
            fill_rule: FillRule::NonZero,
            visible: true,
            displayed: true,
        }
    }
}

struct Builder {
    doc: Document,
    report: Report,
    shapes: usize,
}

fn local_name(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

fn number(src: &str) -> Result<f32, SvgImportError> {
    let value = src
        .trim()
        .parse::<f32>()
        .map_err(|_| SvgImportError::Invalid(format!("'{src}' is not a number")))?;
    if value.is_finite() {
        Ok(value)
    } else {
        Err(SvgImportError::Invalid("a number is not finite".into()))
    }
}

fn length(src: &str) -> Result<f32, SvgImportError> {
    let src = src.trim();
    let units = ["px", "in", "cm", "mm", "pt", "pc", "%"];
    let (value, unit) = units
        .iter()
        .find_map(|unit| src.strip_suffix(unit).map(|value| (value, *unit)))
        .unwrap_or((src, ""));
    let value = number(value)?;
    let scale = match unit {
        "" | "px" => 1.0,
        "in" => 96.0,
        "cm" => 96.0 / 2.54,
        "mm" => 96.0 / 25.4,
        "pt" => 96.0 / 72.0,
        "pc" => 16.0,
        _ => {
            return Err(SvgImportError::Invalid(format!(
                "unsupported length unit '{unit}'"
            )));
        }
    };
    let value = value * scale;
    if value.is_finite() {
        Ok(value)
    } else {
        Err(SvgImportError::Invalid("a length is out of range".into()))
    }
}

fn numbers(src: &str) -> Result<Vec<f32>, SvgImportError> {
    let bytes = src.as_bytes();
    let mut at = 0;
    let mut out = Vec::new();
    while at < bytes.len() {
        while bytes
            .get(at)
            .is_some_and(|c| c.is_ascii_whitespace() || *c == b',')
        {
            at += 1;
        }
        if at >= bytes.len() {
            break;
        }
        let start = at;
        if bytes.get(at).is_some_and(|c| matches!(c, b'+' | b'-')) {
            at += 1;
        }
        let mut digits = false;
        while bytes.get(at).is_some_and(u8::is_ascii_digit) {
            digits = true;
            at += 1;
        }
        if bytes.get(at) == Some(&b'.') {
            at += 1;
            while bytes.get(at).is_some_and(u8::is_ascii_digit) {
                digits = true;
                at += 1;
            }
        }
        if !digits {
            return Err(SvgImportError::Invalid(format!(
                "invalid number list near byte {at}"
            )));
        }
        if bytes.get(at).is_some_and(|c| matches!(c, b'e' | b'E')) {
            let exp = at;
            at += 1;
            if bytes.get(at).is_some_and(|c| matches!(c, b'+' | b'-')) {
                at += 1;
            }
            let exp_start = at;
            while bytes.get(at).is_some_and(u8::is_ascii_digit) {
                at += 1;
            }
            if at == exp_start {
                at = exp;
            }
        }
        let token = src.get(start..at).unwrap_or("");
        out.push(number(token)?);
        if out.len() > MAX_GEOMETRY_VALUES {
            return Err(SvgImportError::Invalid(format!(
                "a geometry has more than {MAX_GEOMETRY_VALUES} numbers"
            )));
        }
    }
    Ok(out)
}

fn parse_color(src: &str) -> Option<Rgba> {
    let src = src.trim().to_ascii_lowercase();
    if let Some(hex) = src.strip_prefix('#') {
        let expanded = match hex.len() {
            3 | 4 => {
                let mut out = String::with_capacity(hex.len() * 2);
                for c in hex.chars() {
                    out.push(c);
                    out.push(c);
                }
                out
            }
            6 | 8 => hex.to_string(),
            _ => return None,
        };
        return Rgba::parse_hex(&expanded);
    }
    let color = match src.as_str() {
        "black" => Rgba::BLACK,
        "white" => Rgba::WHITE,
        "red" => Rgba::hex(0xff0000),
        "green" => Rgba::hex(0x008000),
        "blue" => Rgba::hex(0x0000ff),
        "yellow" => Rgba::hex(0xffff00),
        "gray" | "grey" => Rgba::hex(0x808080),
        "silver" => Rgba::hex(0xc0c0c0),
        "maroon" => Rgba::hex(0x800000),
        "purple" => Rgba::hex(0x800080),
        "fuchsia" | "magenta" => Rgba::hex(0xff00ff),
        "lime" => Rgba::hex(0x00ff00),
        "olive" => Rgba::hex(0x808000),
        "navy" => Rgba::hex(0x000080),
        "teal" => Rgba::hex(0x008080),
        "aqua" | "cyan" => Rgba::hex(0x00ffff),
        "orange" => Rgba::hex(0xffa500),
        "transparent" => Rgba::TRANSPARENT,
        _ => return parse_rgb(&src),
    };
    Some(color)
}

fn parse_rgb(src: &str) -> Option<Rgba> {
    let inner = src.strip_prefix("rgb(")?.strip_suffix(')')?;
    let components: Vec<&str> = inner.split(',').map(str::trim).collect();
    if components.len() != 3 {
        return None;
    }
    let mut rgb = [0.0; 3];
    for (index, c) in components.iter().enumerate() {
        let value = c
            .strip_suffix('%')
            .and_then(|v| v.parse::<f32>().ok().map(|n| n * 2.55))
            .or_else(|| c.parse::<f32>().ok())?;
        if !value.is_finite() {
            return None;
        }
        rgb[index] = value.clamp(0.0, 255.0) / 255.0;
    }
    Some(Rgba::new(rgb[0], rgb[1], rgb[2], 1.0))
}

fn opacity(src: &str, fallback: f32) -> f32 {
    let src = src.trim();
    let (value, scale) = src.strip_suffix('%').map_or((src, 1.0), |v| (v, 0.01));
    value
        .parse::<f32>()
        .ok()
        .filter(|n| n.is_finite())
        .map_or(fallback, |v| (v * scale).clamp(0.0, 1.0))
}

fn properties(node: &XmlElement) -> Vec<(&str, &str)> {
    let mut attrs: Vec<(&str, &str)> = node
        .attrs
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    if let Some(style) = node.attr("style") {
        attrs.extend(style.split(';').filter_map(|decl| {
            let (key, value) = decl.split_once(':')?;
            Some((key.trim(), value.trim()))
        }));
    }
    attrs
}

fn unsupported_style(node: &XmlElement) -> bool {
    let unsupported = |key: &str| {
        matches!(
            key,
            "stroke-dasharray"
                | "stroke-dashoffset"
                | "stroke-linecap"
                | "stroke-linejoin"
                | "stroke-miterlimit"
                | "vector-effect"
                | "paint-order"
                | "mix-blend-mode"
        )
    };
    properties(node)
        .iter()
        .any(|(key, value)| unsupported(key) && *value != "none")
}

fn property<'a>(node: &'a XmlElement, name: &str) -> Option<&'a str> {
    let mut value = node.attr(name);
    if let Some(style) = node.attr("style") {
        for declaration in style.split(';') {
            if let Some((key, content)) = declaration.split_once(':')
                && key.trim() == name
            {
                value = Some(content.trim());
            }
        }
    }
    value
}

fn styled(parent: &Style, node: &XmlElement) -> Result<Style, SvgImportError> {
    let mut out = parent.clone();
    if let Some(value) = property(node, "fill") {
        out.fill = (value != "none").then(|| value.to_string());
    }
    if let Some(value) = property(node, "stroke") {
        out.stroke = (value != "none").then(|| value.to_string());
    }
    if let Some(value) = property(node, "fill-opacity") {
        out.fill_opacity = opacity(value, out.fill_opacity);
    }
    if let Some(value) = property(node, "stroke-opacity") {
        out.stroke_opacity = opacity(value, out.stroke_opacity);
    }
    if let Some(value) = property(node, "opacity") {
        out.opacity *= opacity(value, 1.0);
    }
    if let Some(value) = property(node, "stroke-width") {
        out.stroke_width = length(value)?.max(0.0);
    }
    if let Some(value) = property(node, "fill-rule") {
        out.fill_rule = if value == "evenodd" {
            FillRule::EvenOdd
        } else {
            FillRule::NonZero
        };
    }
    if let Some(value) = property(node, "visibility") {
        out.visible = value != "hidden" && value != "collapse";
    }
    if property(node, "display") == Some("none") {
        out.displayed = false;
    }
    Ok(out)
}

fn parse_transform(src: &str) -> Result<Affine, SvgImportError> {
    let mut rest = src.trim();
    let mut out = Affine::IDENTITY;
    while !rest.is_empty() {
        rest = rest.trim_start_matches(|c: char| c.is_ascii_whitespace() || c == ',');
        if rest.is_empty() {
            break;
        }
        let open = rest
            .find('(')
            .ok_or_else(|| SvgImportError::Invalid("malformed transform".into()))?;
        let name = rest.get(..open).unwrap_or("").trim();
        let close = rest
            .get(open + 1..)
            .and_then(|s| s.find(')'))
            .map(|i| i + open + 1)
            .ok_or_else(|| SvgImportError::Invalid("malformed transform".into()))?;
        let args = numbers(rest.get(open + 1..close).unwrap_or(""))?;
        let op = match (name, args.as_slice()) {
            ("matrix", [a, b, c, d, e, f]) => Affine {
                m: [*a, *b, *c, *d, *e, *f],
            },
            ("translate", [x]) => Affine::translate(*x, 0.0),
            ("translate", [x, y]) => Affine::translate(*x, *y),
            ("scale", [s]) => Affine::scale(*s, *s),
            ("scale", [x, y]) => Affine::scale(*x, *y),
            ("rotate", [r]) => Affine::rotate(r.to_radians()),
            ("rotate", [r, x, y]) => Affine::translate(*x, *y)
                .mul(&Affine::rotate(r.to_radians()))
                .mul(&Affine::translate(-*x, -*y)),
            ("skewX", [a]) => Affine {
                m: [1.0, 0.0, a.to_radians().tan(), 1.0, 0.0, 0.0],
            },
            ("skewY", [a]) => Affine {
                m: [1.0, a.to_radians().tan(), 0.0, 1.0, 0.0, 0.0],
            },
            _ => {
                return Err(SvgImportError::Invalid(format!(
                    "unsupported or malformed transform '{name}'"
                )));
            }
        };
        if !op.is_finite() {
            return Err(SvgImportError::Invalid("transform is out of range".into()));
        }
        out = out.mul(&op);
        rest = rest.get(close + 1..).unwrap_or("");
    }
    Ok(out)
}

fn attr_length(node: &XmlElement, key: &str, default: f32) -> Result<f32, SvgImportError> {
    node.attr(key).map(length).unwrap_or(Ok(default))
}

fn geometry(node: &XmlElement, tag: &str) -> Result<Option<Path>, SvgImportError> {
    let x = attr_length(node, "x", 0.0)?;
    let y = attr_length(node, "y", 0.0)?;
    let w = attr_length(node, "width", 0.0)?;
    let h = attr_length(node, "height", 0.0)?;
    let path = match tag {
        "path" => {
            let d = node.attr("d").unwrap_or("");
            parse_path_data(d)
                .map_err(|e| SvgImportError::Invalid(format!("invalid path data: {e}")))?
        }
        "rect" => {
            if w <= 0.0 || h <= 0.0 {
                return Ok(None);
            }
            let rx = attr_length(node, "rx", 0.0)?;
            let ry = attr_length(node, "ry", 0.0)?;
            let radius = if rx > 0.0 && ry > 0.0 {
                rx.min(ry)
            } else {
                rx.max(ry)
            };
            if radius > 0.0 {
                Path::rounded_rect(x, y, w, h, radius)
            } else {
                Path::rect(x, y, w, h)
            }
        }
        "circle" => {
            let r = attr_length(node, "r", 0.0)?;
            if r <= 0.0 {
                return Ok(None);
            }
            Path::ellipse(
                attr_length(node, "cx", 0.0)?,
                attr_length(node, "cy", 0.0)?,
                r,
                r,
            )
        }
        "ellipse" => {
            let (rx, ry) = (attr_length(node, "rx", 0.0)?, attr_length(node, "ry", 0.0)?);
            if rx <= 0.0 || ry <= 0.0 {
                return Ok(None);
            }
            Path::ellipse(
                attr_length(node, "cx", 0.0)?,
                attr_length(node, "cy", 0.0)?,
                rx,
                ry,
            )
        }
        "line" => {
            let mut p = Path::new();
            p.move_to(Vec2::new(
                attr_length(node, "x1", 0.0)?,
                attr_length(node, "y1", 0.0)?,
            ))
            .line_to(Vec2::new(
                attr_length(node, "x2", 0.0)?,
                attr_length(node, "y2", 0.0)?,
            ));
            p
        }
        "polyline" | "polygon" => {
            let coords = numbers(node.attr("points").unwrap_or(""))?;
            if coords.len() < 4 || coords.len() % 2 != 0 {
                return Ok(None);
            }
            let mut p = Path::new();
            let (points, _) = coords.as_chunks::<2>();
            if let Some(first) = points.first() {
                p.move_to(Vec2::new(first[0], first[1]));
                for point in points.iter().skip(1) {
                    p.line_to(Vec2::new(point[0], point[1]));
                }
                if tag == "polygon" {
                    p.close();
                }
            }
            p
        }
        _ => return Ok(None),
    };
    if path.cmds.iter().any(|cmd| match cmd {
        anicraft_geom::PathCmd::MoveTo(p) | anicraft_geom::PathCmd::LineTo(p) => !p.is_finite(),
        anicraft_geom::PathCmd::QuadTo(a, b) => !a.is_finite() || !b.is_finite(),
        anicraft_geom::PathCmd::CubicTo(a, b, c) => {
            !a.is_finite() || !b.is_finite() || !c.is_finite()
        }
        anicraft_geom::PathCmd::Close => false,
    }) {
        return Err(SvgImportError::Invalid(
            "shape coordinates are out of range".into(),
        ));
    }
    Ok((!path.is_empty()).then_some(path))
}

impl Builder {
    fn add_shape(
        &mut self,
        path: Path,
        transform: Affine,
        style: &Style,
    ) -> Result<(), SvgImportError> {
        let bounds = path.bounds().transformed(&transform);
        if !bounds.min.is_finite() || !bounds.max.is_finite() {
            return Err(SvgImportError::Invalid(
                "transformed shape coordinates are out of range".into(),
            ));
        }
        let mut fill = None;
        if let Some(value) = &style.fill {
            if value.starts_with("url(") {
                self.report.skip("gradient or pattern fills");
            } else if let Some(color) = parse_color(value) {
                fill = (color.a * style.fill_opacity > 0.0)
                    .then(|| Paint::Solid(color.with_alpha(color.a * style.fill_opacity)));
            } else {
                self.report.skip("unsupported fill colors");
            }
        }
        let stroke = if let Some(value) = &style.stroke {
            if value.starts_with("url(") {
                self.report.skip("gradient or pattern strokes");
                None
            } else if let Some(color) = parse_color(value) {
                let color = color.with_alpha(color.a * style.stroke_opacity);
                (color.a > 0.0 && style.stroke_width > 0.0).then_some(Stroke {
                    color,
                    width: style.stroke_width,
                })
            } else {
                self.report.skip("unsupported stroke colors");
                None
            }
        } else {
            None
        };
        if fill.is_none() && stroke.is_none() {
            return Ok(());
        }
        if self.shapes >= MAX_IMPORTED_SHAPES {
            return Err(SvgImportError::Invalid(format!(
                "the SVG contains more than {MAX_IMPORTED_SHAPES} drawable shapes"
            )));
        }
        self.shapes += 1;
        let id = ElementId(self.doc.alloc_id());
        let shape = Shape {
            path: Arc::new(path),
            fill,
            stroke,
            fill_rule: style.fill_rule,
        };
        let element = Element {
            id,
            name: String::new(),
            transform,
            color: ColorEffect {
                alpha: style.opacity,
                ..ColorEffect::default()
            },
            kind: ElementKind::Shape(shape),
        };
        if let Some(layer) = self.doc.timeline.layers.first_mut()
            && let Some(keyframe) = Arc::make_mut(layer).keyframe_mut(0)
        {
            keyframe.elements.push(Arc::new(element));
        }
        Ok(())
    }

    fn walk(
        &mut self,
        node: &XmlElement,
        parent_transform: &Affine,
        parent_style: &Style,
        depth: usize,
    ) -> Result<(), SvgImportError> {
        if depth > MAX_IMPORT_DEPTH {
            return Err(SvgImportError::Invalid(
                "the SVG element nesting is too deep".into(),
            ));
        }
        let tag = local_name(&node.name);
        let style = styled(parent_style, node)?;
        if !style.displayed || !style.visible {
            return Ok(());
        }
        if matches!(tag, "defs" | "metadata" | "title" | "desc" | "namedview") {
            return Ok(());
        }
        let local = node
            .attr("transform")
            .map(parse_transform)
            .unwrap_or(Ok(Affine::IDENTITY))?;
        let transform = parent_transform.mul(&local);
        if !transform.is_finite() {
            return Err(SvgImportError::Invalid(
                "an element transform is out of range".into(),
            ));
        }
        for (key, value) in properties(node) {
            if matches!(
                key,
                "clip-path" | "filter" | "mask" | "marker-start" | "marker-mid" | "marker-end"
            ) && value != "none"
            {
                self.report.skip("clipping, filters, masks, or markers");
                break;
            }
        }
        if unsupported_style(node) {
            self.report.skip("unsupported stroke or paint styles");
        }
        if matches!(tag, "a" | "switch") {
            self.report.skip("SVG links or switches");
            return Ok(());
        }
        if tag == "svg" && depth > 1 {
            self.report.skip("nested SVG viewports");
            return Ok(());
        }
        if matches!(tag, "svg" | "g") {
            for child in &node.children {
                self.walk(child, &transform, &style, depth + 1)?;
            }
            return Ok(());
        }
        if matches!(
            tag,
            "path" | "rect" | "circle" | "ellipse" | "line" | "polyline" | "polygon"
        ) {
            if let Some(path) = geometry(node, tag)? {
                self.add_shape(path, transform, &style)?;
            }
            return Ok(());
        }
        let what = match tag {
            "text" | "tspan" | "textPath" => "text elements",
            "image" => "embedded images",
            "use" => "reused elements",
            "style" => "CSS stylesheets",
            "linearGradient" | "radialGradient" => "gradient definitions",
            "clipPath" => "clip path definitions",
            "mask" => "mask definitions",
            "filter" => "filter definitions",
            "symbol" => "symbol definitions",
            _ => {
                let short: String = tag.chars().take(64).collect();
                self.report.skip(&format!("<{short}> elements"));
                return Ok(());
            }
        };
        self.report.skip(what);
        Ok(())
    }
}

fn view_box(root: &XmlElement) -> Result<Option<[f32; 4]>, SvgImportError> {
    let Some(value) = root.attr("viewBox").or_else(|| root.attr("viewbox")) else {
        return Ok(None);
    };
    let values = numbers(value)?;
    let [x, y, w, h] = values.as_slice() else {
        return Err(SvgImportError::Invalid(
            "viewBox must contain four numbers".into(),
        ));
    };
    if *w <= 0.0 || *h <= 0.0 {
        return Err(SvgImportError::Invalid(
            "viewBox width and height must be positive".into(),
        ));
    }
    Ok(Some([*x, *y, *w, *h]))
}

fn stage(root: &XmlElement, report: &mut Report) -> Result<(Stage, Affine), SvgImportError> {
    let view_box = view_box(root)?;
    let width = root
        .attr("width")
        .map(length)
        .transpose()?
        .or_else(|| view_box.map(|v| v[2]))
        .unwrap_or(DEFAULT_WIDTH);
    let height = root
        .attr("height")
        .map(length)
        .transpose()?
        .or_else(|| view_box.map(|v| v[3]))
        .unwrap_or(DEFAULT_HEIGHT);
    if width <= 0.0 || height <= 0.0 {
        return Err(SvgImportError::Invalid(
            "viewport width and height must be positive".into(),
        ));
    }
    let w = width
        .round()
        .clamp(1.0, anicraft_doc::MAX_STAGE_EDGE as f32) as u32;
    let h = height
        .round()
        .clamp(1.0, anicraft_doc::MAX_STAGE_EDGE as f32) as u32;
    if width > anicraft_doc::MAX_STAGE_EDGE as f32 || height > anicraft_doc::MAX_STAGE_EDGE as f32 {
        report.notes.push(format!(
            "stage dimensions were limited to {} px",
            anicraft_doc::MAX_STAGE_EDGE
        ));
    }
    let viewport_scale = Affine::scale(w as f32 / width, h as f32 / height);
    let map = if let Some([x, y, vw, vh]) = view_box {
        let preserve = root.attr("preserveAspectRatio").unwrap_or("xMidYMid meet");
        if preserve.trim_start().starts_with("none") {
            viewport_scale
                .mul(&Affine::translate(-x, -y).mul(&Affine::scale(width / vw, height / vh)))
        } else {
            let scale = (width / vw).min(height / vh);
            let extra_x = width - vw * scale;
            let extra_y = height - vh * scale;
            let align = preserve.split_whitespace().next().unwrap_or("xMidYMid");
            let dx = if align.contains("xMin") {
                0.0
            } else if align.contains("xMax") {
                extra_x
            } else {
                extra_x * 0.5
            };
            let dy = if align.contains("YMin") {
                0.0
            } else if align.contains("YMax") {
                extra_y
            } else {
                extra_y * 0.5
            };
            if preserve.split_whitespace().any(|v| v == "slice") {
                report
                    .notes
                    .push("preserveAspectRatio slice is treated as meet".into());
            }
            viewport_scale.mul(
                &Affine::translate(dx, dy)
                    .mul(&Affine::scale(scale, scale))
                    .mul(&Affine::translate(-x, -y)),
            )
        }
    } else {
        viewport_scale
    };
    if !map.is_finite() {
        return Err(SvgImportError::Invalid(
            "viewBox mapping is out of range".into(),
        ));
    }
    let stage = Stage {
        width: w,
        height: h,
        background: Rgba::TRANSPARENT,
        ..Stage::default()
    };
    Ok((stage, map))
}

pub fn import_bytes(bytes: &[u8]) -> Result<(Document, Report), SvgImportError> {
    if bytes.len() as u64 > MAX_SVG_BYTES {
        return Err(SvgImportError::NotSvg(format!(
            "the file is larger than {} MiB",
            MAX_SVG_BYTES / (1024 * 1024)
        )));
    }
    let root = xml::parse_limited(
        bytes,
        MAX_SVG_BYTES as usize,
        MAX_IMPORT_DEPTH,
        MAX_XML_ELEMENTS,
    )?;
    if local_name(&root.name) != "svg" {
        return Err(SvgImportError::NotSvg(format!(
            "the root element is <{}>",
            root.name
        )));
    }
    let mut report = Report::default();
    let (stage, root_transform) = stage(&root, &mut report)?;
    let mut doc = Document {
        stage,
        ..Document::default()
    };
    doc.timeline.layers.clear();
    let layer_id = LayerId(doc.alloc_id());
    let mut layer = Layer::new(layer_id, "SVG Artwork");
    layer.keyframes = vec![Arc::new(Keyframe::new(0, 1))];
    doc.timeline.layers.push(Arc::new(layer));
    let mut builder = Builder {
        doc,
        report,
        shapes: 0,
    };
    let root_style = styled(&Style::default(), &root)?;
    let root_local = root
        .attr("transform")
        .map(parse_transform)
        .unwrap_or(Ok(Affine::IDENTITY))?;
    let root_transform = root_transform.mul(&root_local);
    for child in &root.children {
        builder.walk(child, &root_transform, &root_style, 1)?;
    }
    Ok((builder.doc, builder.report))
}

pub fn import_path(path: &FsPath) -> Result<(Document, Report), SvgImportError> {
    let metadata = std::fs::metadata(path)?;
    if !metadata.is_file() {
        return Err(SvgImportError::NotSvg("expected an SVG file".into()));
    }
    if metadata.len() > MAX_SVG_BYTES {
        return Err(SvgImportError::NotSvg(format!(
            "the file is larger than {} MiB",
            MAX_SVG_BYTES / (1024 * 1024)
        )));
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    std::fs::File::open(path)?
        .take(MAX_SVG_BYTES + 1)
        .read_to_end(&mut bytes)?;
    import_bytes(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn imports_paths_shapes_styles_transforms_and_viewbox() {
        let src = br##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100" viewBox="10 20 100 50">
          <g transform="translate(5 3)" fill="#f00" stroke="blue" stroke-width="2" opacity=".5">
            <rect x="10" y="20" width="20" height="10" rx="2"/>
            <path d="M10 20h10v10z" fill="none" style="stroke:#00ff00;stroke-width:4" fill-rule="evenodd"/>
            <circle cx="40" cy="30" r="5"/>
            <ellipse cx="60" cy="30" rx="8" ry="4"/>
            <line x1="70" y1="20" x2="80" y2="30"/>
            <polyline points="10,40 20,40 20,45"/>
            <polygon points="30,40 40,40 35,48"/>
          </g>
          <text x="0" y="0">not imported</text>
        </svg>"##;
        let (doc, report) = import_bytes(src).expect("valid SVG imports");
        assert_eq!((doc.stage.width, doc.stage.height), (200, 100));
        assert_eq!(doc.frame_count(), 1);
        let layer = doc.timeline.layers.first().expect("an import layer");
        let key = layer.keyframes.first().expect("a first keyframe");
        assert_eq!(key.elements.len(), 7);
        assert!(report.summary().contains("1 text elements"));
        let first = key.elements.first().expect("rectangle");
        assert_eq!(
            first.transform.apply(Vec2::new(0.0, 0.0)),
            Vec2::new(-10.0, -34.0)
        );
        assert_eq!(first.color.alpha, 0.5);
        let ElementKind::Shape(rect) = &first.kind else {
            panic!("imported shapes stay editable")
        };
        assert!(matches!(rect.fill, Some(Paint::Solid(c)) if c == Rgba::hex(0xff0000)));
        assert_eq!(
            rect.stroke,
            Some(Stroke {
                color: Rgba::hex(0x0000ff),
                width: 2.0
            })
        );
        let second = key.elements.get(1).expect("path");
        let ElementKind::Shape(path) = &second.kind else {
            panic!("imported path stays editable")
        };
        assert_eq!(path.fill, None);
        assert_eq!(
            path.stroke,
            Some(Stroke {
                color: Rgba::hex(0x00ff00),
                width: 4.0
            })
        );
        assert_eq!(path.fill_rule, FillRule::EvenOdd);
    }

    #[test]
    fn rejects_non_svg_and_oversized_svg_geometry() {
        assert!(matches!(
            import_bytes(b"<html/>"),
            Err(SvgImportError::NotSvg(_))
        ));
        assert!(import_bytes(b"<svg><path d=\"M1e100 0L2 2\"/></svg>").is_err());
        assert!(xml::parse_limited(b"<svg><g><path/></g></svg>", 1024, 1, 100).is_err());
        assert!(xml::parse_limited(b"<svg><g/></svg>", 1024, 10, 1).is_err());
    }

    #[test]
    fn parses_exponential_lengths_and_preserves_viewport_size() {
        let (doc, _) = import_bytes(
            br#"<svg width="1e2" height="5e1"><rect width="1e1" height="2e1"/></svg>"#,
        )
        .expect("scientific notation is valid SVG number syntax");
        assert_eq!((doc.stage.width, doc.stage.height), (100, 50));
        assert_eq!(doc.timeline.layers[0].keyframes[0].elements.len(), 1);
    }

    #[test]
    fn reports_content_outside_the_supported_static_subset() {
        let src = br##"<svg width="20" height="10">
          <defs><linearGradient id="g"/></defs>
          <rect width="10" height="10" fill="url(#g)" filter="url(#shadow)"/>
          <style>.label { fill: red }</style><image href="image.png"/>
        </svg>"##;
        let (doc, report) = import_bytes(src).expect("unsupported SVG features are reported");
        let summary = report.summary();
        assert!(summary.contains("gradient or pattern fills"), "{summary}");
        assert!(
            summary.contains("clipping, filters, masks, or markers"),
            "{summary}"
        );
        assert!(summary.contains("CSS stylesheets"), "{summary}");
        assert!(summary.contains("embedded images"), "{summary}");
        assert!(doc.timeline.layers[0].keyframes[0].elements.is_empty());
    }
}
