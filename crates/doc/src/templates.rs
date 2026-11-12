//! The templates AniCraft ships with (ADR 0065): small documents built by code, so nothing
//! binary is committed and every line of them is original work. Each sets up a stage size and
//! frame rate, named layers ready to draw on, and guides on a locked guide layer (shown while
//! editing, never in output). They open as new, untitled documents (`file.new_from_template`).

use crate::{ColorEffect, Document, Element, ElementId, ElementKind, Keyframe, Layer, LayerId, LayerKind, LoopMode, Paint, Rgba, Shape, Stage, Stroke, Symbol, SymbolId, SymbolKind, Timeline};
use anicraft_geom::{Affine, Path, Vec2};
use std::sync::Arc;

/// A built-in template: its id (for commands), name and category (English; the shell
/// translates them where they are drawn), what it is for, and how it is built.
pub struct Template {
    pub id: &'static str,
    pub name: &'static str,
    pub category: &'static str,
    pub description: &'static str,
    pub build: fn() -> Document,
}

/// Every built-in template, in the order the start screen shows them.
pub static TEMPLATES: [Template; 6] = [
    Template { id: "banner-728x90", name: "Leaderboard banner", category: "Advertising", description: "A 728 × 90 web banner at 24 fps, 15 seconds long, with its safe area on a guide layer", build: banner_728x90 },
    Template { id: "banner-300x250", name: "Rectangle banner", category: "Advertising", description: "A 300 × 250 web banner at 24 fps, 15 seconds long, with its safe area on a guide layer", build: banner_300x250 },
    Template { id: "social-square", name: "Social square", category: "Social", description: "A 1080 × 1080 post at 30 fps, 15 seconds long, with a safe margin and thirds on a guide layer", build: social_square },
    Template { id: "storyboard", name: "Storyboard 16:9", category: "Storyboard", description: "Six 2-second panels at 1920 × 1080, labelled Panel 1 to 6, with action- and title-safe areas", build: storyboard },
    Template { id: "character-turnaround", name: "Character turnaround", category: "Character", description: "Front, three-quarter, side and back views side by side, on head-height guides eight heads tall", build: turnaround },
    Template { id: "lip-sync", name: "Lip-sync mouth chart", category: "Character", description: "A Mouth graphic symbol with ten labelled mouth shapes to redraw, placed on a face guide", build: lip_sync },
];

/// The built-in template with this id.
pub fn find(id: &str) -> Option<&'static Template> {
    TEMPLATES.iter().find(|t| t.id == id)
}

/// The colour guides are drawn in (the guide layer's colour too, so the stage shows them alike).
const GUIDE: u32 = 0x4fa3ff;
/// A second, quieter guide colour for secondary lines.
const GUIDE_SOFT: u32 = 0x9cc8ff;

fn blank(width: u32, height: u32, fps: f32) -> Document {
    let mut d = Document { stage: Stage { width, height, fps, background: Rgba::WHITE }, ..Document::default() };
    d.timeline_mut(0).layers.clear();
    d
}

fn element(d: &mut Document, name: &str, transform: Affine, shape: Shape) -> Arc<Element> {
    Arc::new(Element { id: ElementId(d.alloc_id()), name: name.into(), transform, color: ColorEffect::default(), kind: ElementKind::Shape(shape) })
}

/// A stroked line from `a` to `b`.
fn line(d: &mut Document, name: &str, a: (f32, f32), b: (f32, f32), color: u32, width: f32) -> Arc<Element> {
    let mut p = Path::new();
    p.move_to(Vec2::new(a.0, a.1)).line_to(Vec2::new(b.0, b.1));
    element(d, name, Affine::IDENTITY, Shape { path: Arc::new(p), fill: None, stroke: Some(Stroke::new(Rgba::hex(color), width)), fill_rule: Default::default(), merge: false })
}

/// A stroked rectangle outline, `(x, y, w, h)`.
fn outline(d: &mut Document, name: &str, (x, y, w, h): (f32, f32, f32, f32), color: u32, width: f32) -> Arc<Element> {
    element(d, name, Affine::IDENTITY, Shape { path: Arc::new(Path::rect(x, y, w, h)), fill: None, stroke: Some(Stroke::new(Rgba::hex(color), width)), fill_rule: Default::default(), merge: false })
}

/// A layer holding `elements` in one keyframe `frames` long.
fn layer(d: &mut Document, name: &str, kind: LayerKind, color: u32, frames: u32, elements: Vec<Arc<Element>>) -> Arc<Layer> {
    let mut l = Layer::new(LayerId(d.alloc_id()), name);
    l.kind = kind;
    l.color = Rgba::hex(color);
    // Guides stay put while drawing over them.
    l.locked = kind == LayerKind::Guide;
    let mut k = Keyframe::new(0, frames.max(1));
    k.elements = elements;
    l.keyframes = vec![Arc::new(k)];
    Arc::new(l)
}

/// Drawing layers, empty, top first.
fn drawing_layers(d: &mut Document, names: &[&str], frames: u32) -> Vec<Arc<Layer>> {
    let palette = [0xff6b4a, 0x66bb6a, 0xffd54a, 0xba68c8, 0x4dd0e1, 0xff8a65];
    names.iter().enumerate().map(|(i, n)| layer(d, n, LayerKind::Normal, palette[i % palette.len()], frames, Vec::new())).collect()
}

/// A banner: the safe area (inset by a tenth of the short edge) and the edge on a guide layer,
/// layers for the call to action, text and artwork, and a soft background.
fn banner(width: u32, height: u32) -> Document {
    const FRAMES: u32 = 24 * 15;
    let mut d = blank(width, height, 24.0);
    let (w, h) = (width as f32, height as f32);
    let m = (w.min(h) * 0.1).round();
    let guides = vec![outline(&mut d, "Safe area", (m, m, w - 2.0 * m, h - 2.0 * m), GUIDE, 1.0), outline(&mut d, "Edge", (0.5, 0.5, w - 1.0, h - 1.0), GUIDE_SOFT, 1.0)];
    let mut layers = vec![layer(&mut d, "Safe area", LayerKind::Guide, GUIDE, FRAMES, guides)];
    layers.extend(drawing_layers(&mut d, &["Call to action", "Text", "Artwork"], FRAMES));
    let bg = element(&mut d, "background", Affine::IDENTITY, Shape::filled(Path::rect(0.0, 0.0, w, h), Paint::Solid(Rgba::hex(0xeef2f7))));
    let mut back = layer(&mut d, "Background", LayerKind::Normal, 0x7e57c2, FRAMES, vec![bg]);
    Arc::make_mut(&mut back).locked = true;
    layers.push(back);
    d.timeline_mut(0).layers = layers;
    d
}

fn banner_728x90() -> Document {
    banner(728, 90)
}

fn banner_300x250() -> Document {
    banner(300, 250)
}

/// A square post: a safe margin, the thirds and the centre on a guide layer.
fn social_square() -> Document {
    const FRAMES: u32 = 30 * 15;
    let mut d = blank(1080, 1080, 30.0);
    let s = 1080.0;
    let m = 60.0;
    let mut guides = vec![outline(&mut d, "Safe margin", (m, m, s - 2.0 * m, s - 2.0 * m), GUIDE, 2.0)];
    for (i, t) in [s / 3.0, s * 2.0 / 3.0].into_iter().enumerate() {
        guides.push(line(&mut d, &format!("Third {}", i + 1), (t, 0.0), (t, s), GUIDE_SOFT, 1.0));
        guides.push(line(&mut d, &format!("Third {}", i + 3), (0.0, t), (s, t), GUIDE_SOFT, 1.0));
    }
    guides.push(line(&mut d, "Centre", (s / 2.0 - 20.0, s / 2.0), (s / 2.0 + 20.0, s / 2.0), GUIDE, 2.0));
    guides.push(line(&mut d, "Centre", (s / 2.0, s / 2.0 - 20.0), (s / 2.0, s / 2.0 + 20.0), GUIDE, 2.0));
    let mut layers = vec![layer(&mut d, "Safe area", LayerKind::Guide, GUIDE, FRAMES, guides)];
    layers.extend(drawing_layers(&mut d, &["Text", "Artwork", "Background"], FRAMES));
    d.timeline_mut(0).layers = layers;
    d
}

/// A storyboard for 16:9: the action-safe (93 %) and title-safe (90 %) areas and the centre on a
/// guide layer, and a Panels layer of six labelled 2-second keyframes (an animatic as it plays).
fn storyboard() -> Document {
    const PANEL: u32 = 48;
    const PANELS: u32 = 6;
    let mut d = blank(1920, 1080, 24.0);
    let (w, h) = (1920.0, 1080.0);
    let safe = |d: &mut Document, name: &str, keep: f32, color: u32| {
        let (mx, my) = (w * (1.0 - keep) / 2.0, h * (1.0 - keep) / 2.0);
        outline(d, name, (mx, my, w - 2.0 * mx, h - 2.0 * my), color, 2.0)
    };
    let guides = vec![
        safe(&mut d, "Action safe", 0.93, GUIDE),
        safe(&mut d, "Title safe", 0.90, GUIDE_SOFT),
        line(&mut d, "Centre", (w / 2.0 - 30.0, h / 2.0), (w / 2.0 + 30.0, h / 2.0), GUIDE, 2.0),
        line(&mut d, "Centre", (w / 2.0, h / 2.0 - 30.0), (w / 2.0, h / 2.0 + 30.0), GUIDE, 2.0),
    ];
    let total = PANEL * PANELS;
    let guide = layer(&mut d, "Safe areas", LayerKind::Guide, GUIDE, total, guides);
    let notes = layer(&mut d, "Notes", LayerKind::Normal, 0xffd54a, total, Vec::new());
    let mut panels = Layer::new(LayerId(d.alloc_id()), "Panels");
    panels.color = Rgba::hex(0xff6b4a);
    panels.keyframes = (0..PANELS)
        .map(|i| {
            let mut k = Keyframe::new(i * PANEL, PANEL);
            k.label = format!("Panel {}", i + 1);
            Arc::new(k)
        })
        .collect();
    d.timeline_mut(0).layers = vec![guide, notes, Arc::new(panels)];
    d
}

/// A turnaround: five views side by side (a centre line each) on horizontal lines a head
/// apart, eight heads from the top of the head to the ground, and a layer per view.
fn turnaround() -> Document {
    let mut d = blank(1920, 1080, 24.0);
    let (left, right, top, ground) = (60.0, 1860.0, 120.0, 960.0);
    let head = (ground - top) / 8.0;
    let mut guides = Vec::new();
    for i in 0..=8 {
        let y = top + head * i as f32;
        let (color, width) = if i == 0 || i == 8 { (GUIDE, 2.0) } else { (GUIDE_SOFT, 1.0) };
        let name = match i {
            0 => "Top of head".to_string(),
            8 => "Ground".to_string(),
            n => format!("Head {n}"),
        };
        guides.push(line(&mut d, &name, (left, y), (right, y), color, width));
    }
    let views = ["Front", "Three-quarter", "Side", "Three-quarter back", "Back"];
    let col = (right - left) / views.len() as f32;
    for (i, v) in views.iter().enumerate() {
        let x = left + col * (i as f32 + 0.5);
        guides.push(line(&mut d, v, (x, top - 30.0), (x, ground + 30.0), GUIDE, 1.5));
    }
    let mut layers = vec![layer(&mut d, "Turnaround guides", LayerKind::Guide, GUIDE, 1, guides)];
    layers.extend(drawing_layers(&mut d, &views, 1));
    d.timeline_mut(0).layers = layers;
    d
}

/// The mouth shapes of a lip-sync chart, as frame labels and placeholder shapes to redraw.
pub const MOUTHS: [&str; 10] = ["Rest", "AI", "E", "O", "U", "CDGKNRSThYZ", "FV", "L", "MBP", "WQ"];

/// A lip-sync chart: a Mouth graphic symbol with a labelled keyframe per mouth shape (each with
/// a simple placeholder), placed at Single Frame on a face guide, so each keyframe of the Mouth
/// layer picks a shape by its first frame.
fn lip_sync() -> Document {
    const FRAMES: u32 = 24;
    let mut d = blank(1280, 720, 24.0);
    let lips = Rgba::hex(0x8e3b46);
    let mouth = |path: Path| Shape::filled(path, Paint::Solid(lips)).with_stroke(Rgba::hex(0x5a1f28), 3.0);
    let shapes = [
        mouth(Path::rounded_rect(-50.0, -4.0, 100.0, 8.0, 4.0)),
        mouth(Path::ellipse(0.0, 0.0, 55.0, 35.0)),
        mouth(Path::ellipse(0.0, 0.0, 60.0, 20.0)),
        mouth(Path::ellipse(0.0, 0.0, 35.0, 40.0)),
        mouth(Path::ellipse(0.0, 0.0, 22.0, 26.0)),
        mouth(Path::rounded_rect(-55.0, -14.0, 110.0, 28.0, 12.0)),
        mouth(Path::rounded_rect(-50.0, -10.0, 100.0, 20.0, 8.0)),
        mouth(Path::ellipse(0.0, 0.0, 45.0, 30.0)),
        mouth(Path::rounded_rect(-50.0, -3.0, 100.0, 6.0, 3.0)),
        mouth(Path::ellipse(0.0, 0.0, 18.0, 20.0)),
    ];
    let sym_id = SymbolId(d.alloc_id());
    let mut inner = Layer::new(LayerId(d.alloc_id()), "Mouth");
    inner.color = Rgba::hex(0xff6b4a);
    let mut keys = Vec::with_capacity(shapes.len());
    for (i, (label, shape)) in MOUTHS.iter().zip(shapes).enumerate() {
        let mut k = Keyframe::new(i as u32, 1);
        k.label = (*label).to_string();
        k.elements = vec![element(&mut d, &format!("{label} mouth"), Affine::IDENTITY, shape)];
        keys.push(Arc::new(k));
    }
    inner.keyframes = keys;
    d.library.push(Arc::new(Symbol { id: sym_id, name: "Mouth".into(), kind: SymbolKind::Graphic, linkage: None, scale_grid: None, scale_grid_kept: None, timeline: Timeline { layers: vec![Arc::new(inner)] } }));
    let (cx, cy) = (640.0, 340.0);
    let face = element(&mut d, "Head", Affine::IDENTITY, Shape { path: Arc::new(Path::ellipse(cx, cy, 200.0, 250.0)), fill: None, stroke: Some(Stroke::new(Rgba::hex(GUIDE), 2.0)), fill_rule: Default::default(), merge: false });
    let guides = vec![
        face,
        line(&mut d, "Eye line", (cx - 200.0, cy), (cx + 200.0, cy), GUIDE_SOFT, 1.0),
        line(&mut d, "Centre line", (cx, cy - 250.0), (cx, cy + 250.0), GUIDE_SOFT, 1.0),
        line(&mut d, "Mouth line", (cx - 120.0, cy + 130.0), (cx + 120.0, cy + 130.0), GUIDE_SOFT, 1.0),
    ];
    let instance = Arc::new(Element {
        id: ElementId(d.alloc_id()),
        name: "mouth".into(),
        transform: Affine::translate(cx, cy + 130.0),
        color: ColorEffect::default(),
        kind: ElementKind::Symbol { symbol: sym_id, loop_mode: LoopMode::SingleFrame, first_frame: 0, blend: crate::BlendMode::Normal, filters: Vec::new(), events: Vec::new() },
    });
    let layers = vec![
        layer(&mut d, "Face guide", LayerKind::Guide, GUIDE, FRAMES, guides),
        layer(&mut d, "Mouth", LayerKind::Normal, 0xff6b4a, FRAMES, vec![instance]),
        layer(&mut d, "Head", LayerKind::Normal, 0x66bb6a, FRAMES, Vec::new()),
    ];
    d.timeline_mut(0).layers = layers;
    d
}

/// A copy of `doc` with its guide layers drawn as artwork, their lines `widen` times as wide: a
/// template's picture shows its guides (which output never does), and a small picture needs
/// them wider to see them.
pub fn with_guides_shown(doc: &Document, widen: f32) -> Document {
    let widen = if widen.is_finite() { widen.clamp(1.0, 16.0) } else { 1.0 };
    let show = |tl: &mut Timeline| {
        for l in &mut tl.layers {
            if l.kind != LayerKind::Guide {
                continue;
            }
            let l = Arc::make_mut(l);
            l.kind = LayerKind::Normal;
            for k in &mut l.keyframes {
                for e in &mut Arc::make_mut(k).elements {
                    if let ElementKind::Shape(sh) = &e.kind
                        && let Some(st) = &sh.stroke
                    {
                        let mut sh = sh.clone();
                        sh.stroke = Some(Stroke { width: st.width * widen, ..st.clone() });
                        Arc::make_mut(e).kind = ElementKind::Shape(sh);
                    }
                }
            }
        }
    };
    let mut d = doc.clone();
    for scene in &mut d.scenes {
        show(&mut Arc::make_mut(scene).timeline);
    }
    for sym in &mut d.library {
        show(&mut Arc::make_mut(sym).timeline);
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_template_builds_a_consistent_document() {
        let mut ids = std::collections::HashSet::new();
        for t in &TEMPLATES {
            assert!(ids.insert(t.id), "{} is listed once", t.id);
            assert!(find(t.id).is_some_and(|f| std::ptr::eq(f, t)));
            let d = (t.build)();
            assert!(d.stage.width > 0 && d.stage.height > 0 && d.stage.fps > 0.0, "{}", t.id);
            let tl = d.timeline(0);
            assert!(tl.layers.len() >= 3, "{}: named layers to draw on", t.id);
            assert!(tl.layers.iter().any(|l| l.kind == LayerKind::Guide && l.locked), "{}: a locked guide layer", t.id);
            // Ids are unique and below `next_id`, so editing never reuses one.
            let mut seen = std::collections::HashSet::new();
            let mut check = |id: u64| assert!(id < d.next_id && seen.insert(id), "{}: id {id}", t.id);
            for l in &tl.layers {
                check(l.id.0);
                l.keyframes.iter().flat_map(|k| k.elements.iter()).for_each(|e| check(e.id.0));
            }
            for s in &d.library {
                check(s.id.0);
            }
            // Guides never render; the picture with them shown is not blank.
            assert!(crate::evaluate(&d, 0).items.iter().all(|i| i.guide || tl.layer(i.layer).is_some_and(|l| l.kind != LayerKind::Guide)));
            assert!(crate::evaluate(&with_guides_shown(&d, 4.0), 0).items.iter().any(|i| !i.guide), "{}", t.id);
        }
        // The sizes the task asks for.
        let size = |id: &str| find(id).map(|t| (t.build)().stage).map(|s| (s.width, s.height, s.fps));
        assert_eq!(size("banner-728x90"), Some((728, 90, 24.0)));
        assert_eq!(size("banner-300x250"), Some((300, 250, 24.0)));
        assert_eq!(size("social-square"), Some((1080, 1080, 30.0)));
        assert_eq!(size("storyboard"), Some((1920, 1080, 24.0)));
    }

    #[test]
    fn the_storyboard_and_mouth_chart_are_labelled() {
        let d = storyboard();
        let panels = d.timeline(0).layers.iter().find(|l| l.name == "Panels").map(|l| l.keyframes.iter().map(|k| k.label.clone()).collect::<Vec<_>>());
        assert_eq!(panels.map(|p| p.len()), Some(6));
        assert_eq!(d.timeline(0).frame_count(), 6 * 48);
        let d = lip_sync();
        let labels: Vec<String> = d.library.first().map(|s| s.timeline.layers.iter().flat_map(|l| l.keyframes.iter().map(|k| k.label.clone())).collect()).unwrap_or_default();
        assert_eq!(labels, MOUTHS.map(str::to_string).to_vec());
    }
}
