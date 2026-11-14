//! Interface icons: Lucide (ISC licence, `assets/icons/`, see ATTRIBUTION.md).
//!
//! Lucide icons are 24 × 24 outlines made of a few SVG shapes. Rather than pull in an SVG
//! rasterizer, the shapes are parsed once with the engine's own SVG path reader
//! (`geom::svg`) and stroked as vectors at the exact on-screen size, so they stay crisp at any
//! interface scale and cost nothing in build size. Only icons the app uses are embedded; add a
//! file to `assets/icons/` and a variant here to use another.

use anicraft_engine::geom::svg::parse_path_data;
use anicraft_engine::geom::{Affine, Path, Vec2};
use egui::{Color32, Pos2, Rect, Stroke, pos2};
use std::collections::HashMap;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    Select,
    Subselect,
    Transform,
    Pen,
    Pencil,
    Brush,
    PaintBrush,
    Eraser,
    Eyedropper,
    Rect,
    Oval,
    PolyStar,
    Line,
    Bucket,
    Ink,
    Hand,
    Zoom,
    First,
    Last,
    Prev,
    Next,
    Play,
    Pause,
    Loop,
    Onion,
    Eye,
    EyeOff,
    Lock,
    Unlock,
    ChevronRight,
    ChevronDown,
    AlignLeft,
    AlignHCenter,
    AlignRight,
    AlignTop,
    AlignVCenter,
    AlignBottom,
    DistributeH,
    DistributeV,
    Mask,
    Masked,
    Folder,
    FolderOpen,
    ChevronUp,
    FolderPlus,
    Guide,
    Image,
    Type,
    Width,
    GradientTransform,
    Sound,
    SoundOff,
    Camera,
    Magnet,
    Bone,
    Bind,
    Warp,
    EditMultiple,
    Layer,
    InsertFrame,
    InsertKeyframe,
    InsertBlankKeyframe,
    ClipStage,
    Plus,
    Trash,
    Copy,
    Ellipsis,
    PanelProperties,
    PanelSwatches,
    PanelLibrary,
    PanelExplorer,
    PanelHistory,
    PanelOutput,
    PanelActions,
}

impl Icon {
    #[cfg(test)]
    pub const ALL: [Icon; 74] = [
        Icon::PanelProperties,
        Icon::PanelSwatches,
        Icon::PanelLibrary,
        Icon::PanelExplorer,
        Icon::PanelHistory,
        Icon::PanelOutput,
        Icon::PanelActions,
        Icon::Warp,
        Icon::Plus,
        Icon::Trash,
        Icon::Copy,
        Icon::Ellipsis,
        Icon::ClipStage,
        Icon::InsertFrame,
        Icon::InsertKeyframe,
        Icon::InsertBlankKeyframe,
        Icon::Bind,
        Icon::Layer,
        Icon::EditMultiple,
        Icon::Bone,
        Icon::Magnet,
        Icon::SoundOff,
        Icon::Camera,
        Icon::Width,
        Icon::GradientTransform,
        Icon::Sound,
        Icon::Guide,
        Icon::Image,
        Icon::Type,
        Icon::ChevronUp,
        Icon::FolderPlus,
        Icon::Folder,
        Icon::FolderOpen,
        Icon::Mask,
        Icon::Masked,
        Icon::AlignLeft,
        Icon::AlignHCenter,
        Icon::AlignRight,
        Icon::AlignTop,
        Icon::AlignVCenter,
        Icon::AlignBottom,
        Icon::DistributeH,
        Icon::DistributeV,
        Icon::Select,
        Icon::Subselect,
        Icon::Transform,
        Icon::Pen,
        Icon::Pencil,
        Icon::Brush,
        Icon::PaintBrush,
        Icon::Eraser,
        Icon::Eyedropper,
        Icon::Rect,
        Icon::Oval,
        Icon::PolyStar,
        Icon::Line,
        Icon::Bucket,
        Icon::Ink,
        Icon::Hand,
        Icon::Zoom,
        Icon::First,
        Icon::Last,
        Icon::Prev,
        Icon::Next,
        Icon::Play,
        Icon::Pause,
        Icon::Loop,
        Icon::Onion,
        Icon::Eye,
        Icon::EyeOff,
        Icon::Lock,
        Icon::Unlock,
        Icon::ChevronRight,
        Icon::ChevronDown,
    ];

    /// Drawn filled as well as outlined: the Selection arrow is solid, the Subselection arrow
    /// hollow, the way animators tell the two apart at a glance.
    fn solid(self) -> bool {
        matches!(self, Icon::Select | Icon::InsertKeyframe | Icon::Ellipsis)
    }

    /// Lucide icon file and whether it is drawn mirrored.
    fn source(self) -> (&'static str, bool) {
        match self {
            Icon::Select => ("mouse-pointer-2", false),
            // A hollow arrowhead, mirrored to point like a cursor: the subselection arrow.
            Icon::Subselect => ("navigation", true),
            Icon::Transform => ("scaling", false),
            // A stroke swelling and tapering, with a width point across it (AniCraft original).
            Icon::Width => ("anicraft-width", false),
            // A box of gradient bands with a turning knob on its corner (AniCraft original).
            Icon::GradientTransform => ("anicraft-gradient-transform", false),
            Icon::Pen => ("pen-tool", false),
            Icon::Pencil => ("pencil", false),
            // A round brush: handle, ferrule and a painted tip (AniCraft original, on Lucide's grid).
            Icon::Brush => ("anicraft-brush", false),
            // The brush over a swelling and thinning line: the Paint Brush draws strokes
            // (AniCraft original, on Lucide's grid).
            Icon::PaintBrush => ("anicraft-paint-brush", false),
            // A block eraser on its edge, its sleeve marked off (AniCraft original).
            Icon::Eraser => ("anicraft-eraser", false),
            // A dropper: bulb, collar, glass tube and a drop (AniCraft original).
            Icon::Eyedropper => ("anicraft-eyedropper", false),
            Icon::Rect => ("rectangle-horizontal", false),
            Icon::Oval => ("circle", false),
            Icon::PolyStar => ("pentagon", false),
            Icon::Line => ("slash", false),
            Icon::Bucket => ("paint-bucket", false),
            Icon::Ink => ("droplet", false),
            Icon::Hand => ("hand", false),
            Icon::Zoom => ("zoom-in", false),
            Icon::First => ("chevrons-left", false),
            Icon::Last => ("chevrons-right", false),
            Icon::Prev => ("chevron-right", true),
            Icon::Next => ("chevron-right", false),
            Icon::Play => ("play", false),
            Icon::Pause => ("pause", false),
            Icon::Loop => ("rotate-cw", false),
            Icon::Onion => ("layers", false),
            Icon::Eye => ("eye", false),
            Icon::EyeOff => ("eye-off", false),
            Icon::Lock => ("lock", false),
            Icon::Unlock => ("lock-open", false),
            Icon::ChevronRight => ("chevron-right", false),
            Icon::ChevronDown => ("chevron-down", false),
            Icon::ChevronUp => ("chevron-up", false),
            Icon::FolderPlus => ("folder-plus", false),
            Icon::Guide => ("spline", false),
            Icon::Image => ("image", false),
            Icon::Type => ("type", false),
            Icon::Sound => ("volume-2", false),
            // The speaker crossed out (a muted layer).
            Icon::SoundOff => ("anicraft-sound-off", false),
            // Lucide's object-align icons aren't in the local set; these are AniCraft originals
            // drawn on the same 24 grid with the same 2-unit round strokes.
            Icon::AlignLeft => ("anicraft-align-left", false),
            Icon::AlignHCenter => ("anicraft-align-hcenter", false),
            Icon::AlignRight => ("anicraft-align-right", false),
            Icon::AlignTop => ("anicraft-align-top", false),
            Icon::AlignVCenter => ("anicraft-align-vcenter", false),
            Icon::AlignBottom => ("anicraft-align-bottom", false),
            Icon::DistributeH => ("anicraft-distribute-h", false),
            Icon::DistributeV => ("anicraft-distribute-v", false),
            // Layer types: a stencil (mask) and the area inside it (masked).
            Icon::Mask => ("anicraft-mask", false),
            Icon::Masked => ("anicraft-masked", false),
            Icon::Folder => ("folder", false),
            // A film camera with its two reels (AniCraft original, on Lucide's grid).
            Icon::Camera => ("anicraft-camera", false),
            // A horseshoe magnet, poles up (AniCraft original, on Lucide's grid): Snap to Objects.
            Icon::Magnet => ("anicraft-magnet", false),
            // A bone: a wedge from its joint to its end (AniCraft original, on Lucide's grid).
            Icon::Bone => ("anicraft-bone", false),
            // A bone whose end is tied to two points (AniCraft original, on Lucide's grid): Bind.
            Icon::Bind => ("anicraft-bind", false),
            // A mesh bent around a pin (AniCraft original, on Lucide's grid): Asset Warp.
            Icon::Warp => ("anicraft-warp", false),
            // Frames stacked one behind the other, all edited at once (AniCraft original, on
            // Lucide's grid): Edit Multiple Frames.
            Icon::EditMultiple => ("anicraft-edit-multiple", false),
            Icon::FolderOpen => ("folder-open", false),
            // A layer: a row with a keyframe and its span (AniCraft original, on Lucide's grid).
            Icon::Layer => ("anicraft-layer", false),
            // Insert Frame / Keyframe / Blank Keyframe: the timeline's own marks (a frame cell, a
            // filled keyframe dot, a hollow blank-keyframe dot), each with a plus (AniCraft
            // originals, on Lucide's grid).
            Icon::InsertFrame => ("anicraft-insert-frame", false),
            Icon::InsertKeyframe => ("anicraft-insert-keyframe", false),
            Icon::InsertBlankKeyframe => ("anicraft-insert-blank-keyframe", false),
            // The stage with crop marks at its corners (AniCraft original, on Lucide's grid):
            // Clip Content Outside the Stage.
            Icon::ClipStage => ("anicraft-clip-stage", false),
            // New layer, Delete, Duplicate and a panel's options menu.
            Icon::Plus => ("plus", false),
            Icon::Trash => ("trash", false),
            Icon::Copy => ("copy", false),
            Icon::Ellipsis => ("ellipsis", false),
            // The panels, as icons in a dock collapsed to icons (ADR 0087; AniCraft originals,
            // on Lucide's grid): sliders, swatches, books, a tree, a clock turning back, a
            // console, code.
            Icon::PanelProperties => ("anicraft-panel-properties", false),
            Icon::PanelSwatches => ("anicraft-panel-swatches", false),
            Icon::PanelLibrary => ("anicraft-panel-library", false),
            Icon::PanelExplorer => ("anicraft-panel-explorer", false),
            Icon::PanelHistory => ("anicraft-panel-history", false),
            Icon::PanelOutput => ("anicraft-panel-output", false),
            Icon::PanelActions => ("anicraft-panel-actions", false),
        }
    }
}

static FILES: &[(&str, &str)] = &[
    ("mouse-pointer-2", include_str!("../assets/icons/mouse-pointer-2.svg")),
    ("navigation", include_str!("../assets/icons/navigation.svg")),
    ("scaling", include_str!("../assets/icons/scaling.svg")),
    ("pen-tool", include_str!("../assets/icons/pen-tool.svg")),
    ("pencil", include_str!("../assets/icons/pencil.svg")),
    ("rectangle-horizontal", include_str!("../assets/icons/rectangle-horizontal.svg")),
    ("circle", include_str!("../assets/icons/circle.svg")),
    ("pentagon", include_str!("../assets/icons/pentagon.svg")),
    ("slash", include_str!("../assets/icons/slash.svg")),
    ("paint-bucket", include_str!("../assets/icons/paint-bucket.svg")),
    ("droplet", include_str!("../assets/icons/droplet.svg")),
    ("hand", include_str!("../assets/icons/hand.svg")),
    ("zoom-in", include_str!("../assets/icons/zoom-in.svg")),
    ("chevrons-left", include_str!("../assets/icons/chevrons-left.svg")),
    ("chevrons-right", include_str!("../assets/icons/chevrons-right.svg")),
    ("chevron-right", include_str!("../assets/icons/chevron-right.svg")),
    ("chevron-down", include_str!("../assets/icons/chevron-down.svg")),
    ("chevron-up", include_str!("../assets/icons/chevron-up.svg")),
    ("folder-plus", include_str!("../assets/icons/folder-plus.svg")),
    ("spline", include_str!("../assets/icons/spline.svg")),
    ("image", include_str!("../assets/icons/image.svg")),
    ("type", include_str!("../assets/icons/type.svg")),
    ("volume-2", include_str!("../assets/icons/volume-2.svg")),
    ("anicraft-sound-off", include_str!("../assets/icons/anicraft-sound-off.svg")),
    ("anicraft-magnet", include_str!("../assets/icons/anicraft-magnet.svg")),
    ("play", include_str!("../assets/icons/play.svg")),
    ("pause", include_str!("../assets/icons/pause.svg")),
    ("rotate-cw", include_str!("../assets/icons/rotate-cw.svg")),
    ("layers", include_str!("../assets/icons/layers.svg")),
    ("eye", include_str!("../assets/icons/eye.svg")),
    ("eye-off", include_str!("../assets/icons/eye-off.svg")),
    ("lock", include_str!("../assets/icons/lock.svg")),
    ("lock-open", include_str!("../assets/icons/lock-open.svg")),
    ("anicraft-align-left", include_str!("../assets/icons/anicraft-align-left.svg")),
    ("anicraft-align-hcenter", include_str!("../assets/icons/anicraft-align-hcenter.svg")),
    ("anicraft-align-right", include_str!("../assets/icons/anicraft-align-right.svg")),
    ("anicraft-align-top", include_str!("../assets/icons/anicraft-align-top.svg")),
    ("anicraft-align-vcenter", include_str!("../assets/icons/anicraft-align-vcenter.svg")),
    ("anicraft-align-bottom", include_str!("../assets/icons/anicraft-align-bottom.svg")),
    ("anicraft-distribute-h", include_str!("../assets/icons/anicraft-distribute-h.svg")),
    ("anicraft-distribute-v", include_str!("../assets/icons/anicraft-distribute-v.svg")),
    ("anicraft-mask", include_str!("../assets/icons/anicraft-mask.svg")),
    ("anicraft-width", include_str!("../assets/icons/anicraft-width.svg")),
    ("anicraft-gradient-transform", include_str!("../assets/icons/anicraft-gradient-transform.svg")),
    ("anicraft-brush", include_str!("../assets/icons/anicraft-brush.svg")),
    ("anicraft-eraser", include_str!("../assets/icons/anicraft-eraser.svg")),
    ("anicraft-paint-brush", include_str!("../assets/icons/anicraft-paint-brush.svg")),
    ("anicraft-eyedropper", include_str!("../assets/icons/anicraft-eyedropper.svg")),
    ("anicraft-masked", include_str!("../assets/icons/anicraft-masked.svg")),
    ("anicraft-layer", include_str!("../assets/icons/anicraft-layer.svg")),
    ("anicraft-insert-frame", include_str!("../assets/icons/anicraft-insert-frame.svg")),
    ("anicraft-insert-keyframe", include_str!("../assets/icons/anicraft-insert-keyframe.svg")),
    ("anicraft-insert-blank-keyframe", include_str!("../assets/icons/anicraft-insert-blank-keyframe.svg")),
    ("anicraft-clip-stage", include_str!("../assets/icons/anicraft-clip-stage.svg")),
    ("anicraft-camera", include_str!("../assets/icons/anicraft-camera.svg")),
    ("anicraft-bone", include_str!("../assets/icons/anicraft-bone.svg")),
    ("anicraft-bind", include_str!("../assets/icons/anicraft-bind.svg")),
    ("anicraft-warp", include_str!("../assets/icons/anicraft-warp.svg")),
    ("anicraft-edit-multiple", include_str!("../assets/icons/anicraft-edit-multiple.svg")),
    ("folder", include_str!("../assets/icons/folder.svg")),
    ("folder-open", include_str!("../assets/icons/folder-open.svg")),
    ("plus", include_str!("../assets/icons/plus.svg")),
    ("trash", include_str!("../assets/icons/trash.svg")),
    ("copy", include_str!("../assets/icons/copy.svg")),
    ("ellipsis", include_str!("../assets/icons/ellipsis.svg")),
    ("anicraft-panel-properties", include_str!("../assets/icons/anicraft-panel-properties.svg")),
    ("anicraft-panel-swatches", include_str!("../assets/icons/anicraft-panel-swatches.svg")),
    ("anicraft-panel-library", include_str!("../assets/icons/anicraft-panel-library.svg")),
    ("anicraft-panel-explorer", include_str!("../assets/icons/anicraft-panel-explorer.svg")),
    ("anicraft-panel-history", include_str!("../assets/icons/anicraft-panel-history.svg")),
    ("anicraft-panel-output", include_str!("../assets/icons/anicraft-panel-output.svg")),
    ("anicraft-panel-actions", include_str!("../assets/icons/anicraft-panel-actions.svg")),
];

/// The value of attribute `name` in an element's tag text.
fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let key = format!(" {name}=\"");
    let at = tag.find(&key)? + key.len();
    let rest = tag.get(at..)?;
    rest.get(..rest.find('"')?)
}

fn num(tag: &str, name: &str) -> f32 {
    attr(tag, name).and_then(|v| v.trim().parse().ok()).filter(|v: &f32| v.is_finite()).unwrap_or(0.0)
}

fn points(tag: &str, close: bool) -> Option<Path> {
    let v: Vec<f32> = attr(tag, "points")?.split(|c: char| c.is_whitespace() || c == ',').filter_map(|t| t.parse().ok()).collect();
    let mut p = Path::new();
    for (i, &[x, y]) in v.as_chunks::<2>().0.iter().enumerate() {
        let q = Vec2::new(x, y);
        if i == 0 {
            p.move_to(q);
        } else {
            p.line_to(q);
        }
    }
    if close {
        p.close();
    }
    (!p.is_empty()).then_some(p)
}

/// The outline shapes of one SVG file (the subset icon sets use).
pub fn parse_svg(src: &str) -> Vec<Path> {
    let mut out = Vec::new();
    let mut rest = src;
    while let Some(i) = rest.find('<') {
        rest = rest.get(i + 1..).unwrap_or("");
        let end = rest.find('>').unwrap_or(rest.len());
        let tag = rest.get(..end).unwrap_or("");
        rest = rest.get(end..).unwrap_or("");
        let shape = match tag.split_whitespace().next().unwrap_or("") {
            "path" => attr(tag, "d").and_then(|d| parse_path_data(d).ok()),
            "circle" => Some(Path::ellipse(num(tag, "cx"), num(tag, "cy"), num(tag, "r"), num(tag, "r"))),
            "ellipse" => Some(Path::ellipse(num(tag, "cx"), num(tag, "cy"), num(tag, "rx"), num(tag, "ry"))),
            "rect" => Some(Path::rounded_rect(num(tag, "x"), num(tag, "y"), num(tag, "width"), num(tag, "height"), num(tag, "rx").max(num(tag, "ry")))),
            "line" => Some(Path::line(Vec2::new(num(tag, "x1"), num(tag, "y1")), Vec2::new(num(tag, "x2"), num(tag, "y2")))),
            "polyline" => points(tag, false),
            "polygon" => points(tag, true),
            _ => None,
        };
        out.extend(shape.filter(|p| !p.is_empty()));
    }
    out
}

fn library() -> &'static HashMap<&'static str, Vec<Path>> {
    static LIB: OnceLock<HashMap<&'static str, Vec<Path>>> = OnceLock::new();
    LIB.get_or_init(|| FILES.iter().map(|(name, src)| (*name, parse_svg(src))).collect())
}

/// Strokes `icon` centred in `rect` (sized to its shorter side) in `color`.
///
/// Crispness: the icon's 24-unit grid is placed on whole device pixels and the stroke width is
/// rounded to whole device pixels, so lines land on the pixel grid the way the set is drawn
/// (at 24 px it is exact; other sizes stay consistent instead of smearing across pixels).
pub fn paint(p: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    let (name, flip) = icon.source();
    let Some(paths) = library().get(name) else { return };
    let ppp = p.pixels_per_point().max(0.25);
    let s = rect.width().min(rect.height()) / 24.0;
    let snap = |v: f32| (v * ppp).round() / ppp;
    let (ox, oy) = (snap(rect.center().x - 12.0 * s), snap(rect.center().y - 12.0 * s));
    let m = if flip {
        Affine::translate(ox + 24.0 * s, oy).mul(&Affine::scale(-s, s))
    } else {
        Affine::translate(ox, oy).mul(&Affine::scale(s, s))
    };
    // Lucide draws 2-unit strokes on its 24 grid; a touch lighter reads better at UI sizes.
    let w = ((1.75 * s * ppp).round().max(1.0)) / ppp;
    let stroke = Stroke::new(w, color);
    let solid = icon.solid();
    for path in paths {
        for pl in path.flatten(&m, 0.08) {
            let pts: Vec<Pos2> = pl.points.iter().map(|q| pos2(q.x, q.y)).collect();
            if pl.closed {
                if solid {
                    fill_polygon(p, &pts, color);
                }
                p.add(egui::Shape::closed_line(pts, stroke));
            } else {
                // Round caps, as the icons are designed.
                for end in [pts.first(), pts.last()].into_iter().flatten() {
                    p.circle_filled(*end, w * 0.5, color);
                }
                p.add(egui::Shape::line(pts, stroke));
            }
        }
    }
}

/// Fills a simple (possibly concave) polygon by ear clipping. Icon outlines have a few dozen
/// points, so the quadratic search is trivial; anything degenerate just isn't filled.
fn fill_polygon(p: &egui::Painter, pts: &[Pos2], color: Color32) {
    let (v, tris) = triangulate(pts);
    if tris.is_empty() {
        return;
    }
    let mut mesh = egui::Mesh::default();
    for q in &v {
        mesh.colored_vertex(*q, color);
    }
    for [a, b, c] in tris {
        mesh.add_triangle(a, b, c);
    }
    p.add(egui::Shape::mesh(mesh));
}

/// Ear-clipping triangulation: the cleaned-up vertices and triangles indexing them.
fn triangulate(pts: &[Pos2]) -> (Vec<Pos2>, Vec<[u32; 3]>) {
    let mut v: Vec<Pos2> = pts.to_vec();
    v.dedup_by(|a, b| a.distance(*b) < 1e-3);
    if v.len() > 2 && v.first().zip(v.last()).is_some_and(|(a, b)| a.distance(*b) < 1e-3) {
        v.pop();
    }
    let mut tris = Vec::new();
    if v.len() < 3 || v.len() > 4096 {
        return (v, tris);
    }
    let cross = |a: Pos2, b: Pos2, c: Pos2| (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
    let area: f32 = (0..v.len()).map(|i| cross(pos2(0.0, 0.0), v[i], v[(i + 1) % v.len()])).sum();
    let ccw = area > 0.0;
    let mut idx: Vec<usize> = (0..v.len()).collect();
    let mut guard = 0;
    while idx.len() > 3 && guard < 10_000 {
        guard += 1;
        let n = idx.len();
        let mut clipped = false;
        for i in 0..n {
            let (ia, ib, ic) = (idx[(i + n - 1) % n], idx[i], idx[(i + 1) % n]);
            let (a, b, c) = (v[ia], v[ib], v[ic]);
            let turn = cross(a, b, c);
            if (turn > 0.0) != ccw || turn.abs() < 1e-6 {
                continue;
            }
            let inside = idx.iter().any(|&j| {
                if j == ia || j == ib || j == ic {
                    return false;
                }
                let q = v[j];
                let (d1, d2, d3) = (cross(a, b, q), cross(b, c, q), cross(c, a, q));
                if ccw { d1 > 0.0 && d2 > 0.0 && d3 > 0.0 } else { d1 < 0.0 && d2 < 0.0 && d3 < 0.0 }
            });
            if !inside {
                tris.push([ia as u32, ib as u32, ic as u32]);
                idx.remove(i);
                clipped = true;
                break;
            }
        }
        if !clipped {
            break;
        }
    }
    if let [a, b, c] = idx[..] {
        tris.push([a as u32, b as u32, c as u32]);
    }
    (v, tris)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_embedded_and_parses() {
        for icon in Icon::ALL {
            let (name, _) = icon.source();
            let paths = library().get(name).unwrap_or_else(|| panic!("{icon:?}: {name}.svg is not embedded"));
            assert!(!paths.is_empty(), "{name}.svg has no shapes");
            for p in paths {
                let b = p.bounds();
                assert!(b.min.x >= -0.5 && b.min.y >= -0.5 && b.max.x <= 24.5 && b.max.y <= 24.5, "{name}: {b:?} outside the 24 grid");
            }
        }
    }

    #[test]
    fn concave_outlines_triangulate_completely() {
        // An arrowhead with a notch (concave), like the Selection arrow.
        let arrow = [pos2(0.0, 0.0), pos2(10.0, 4.0), pos2(5.0, 5.0), pos2(4.0, 10.0)];
        let (v, tris) = triangulate(&arrow);
        assert_eq!(tris.len(), v.len() - 2);
        let area = |t: &[u32; 3]| {
            let (a, b, c) = (v[t[0] as usize], v[t[1] as usize], v[t[2] as usize]);
            ((b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)).abs() * 0.5
        };
        // Shoelace area of the arrow is 30; the triangles cover exactly that.
        assert!((tris.iter().map(area).sum::<f32>() - 30.0).abs() < 1e-3);
        assert!(triangulate(&[pos2(0.0, 0.0), pos2(1.0, 1.0)]).1.is_empty());
        // The Selection icon really is filled: its outline triangulates.
        let (name, _) = Icon::Select.source();
        let outline = library().get(name).unwrap()[0].flatten(&Affine::IDENTITY, 0.08);
        let pts: Vec<Pos2> = outline[0].points.iter().map(|q| pos2(q.x, q.y)).collect();
        assert!(!triangulate(&pts).1.is_empty());
    }

    #[test]
    fn reads_each_shape_kind() {
        let svg = r#"<svg><rect width="18" height="11" x="3" y="11" rx="2" ry="2" /><circle cx="12" cy="12" r="3" /><line x1="1" y1="2" x2="3" y2="4" /><polygon points="3 11 22 2 13 21" /><path d="M7 11V7a5 5 0 0 1 10 0v4" /><bogus d="M0 0" /></svg>"#;
        assert_eq!(parse_svg(svg).len(), 5);
        assert!(parse_svg("<path d=\"nonsense\" />").is_empty(), "bad shapes are skipped, never panics");
    }
}
