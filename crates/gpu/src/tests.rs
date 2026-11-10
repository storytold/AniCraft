//! Parity with the CPU oracle.
//!
//! Two layers: the binning is checked everywhere by running the fine shader's arithmetic on the
//! CPU over the binned scene (no GPU needed), and the real GPU path is checked against the
//! oracle on the sample scene and analytic scenes when an adapter exists. Without one those
//! tests print why and pass, so CI machines without a GPU stay green.

use super::*;
use crate::binning::{self as bn, Binned, PAINT_VEC4S, TILE, piece_cover, solid_of};
use anicraft_doc::{BlendMode, ColorEffect, DisplayItem, ElementId, ElementKind, GradientStop, Group, LayerId, LayerKind, Paint, Rgba, Stroke, sample};
use anicraft_geom::{Affine, FillRule, Path, Vec2};
use anicraft_render::{paint::Premul, render};
use std::sync::OnceLock;

/// Mean-error target from the roadmap (M3): 1/255 per channel.
const MEAN_TARGET: f64 = 1.0;

fn item(path: Path, fill: Paint) -> DisplayItem {
    DisplayItem {
        path: Arc::new(path),
        transform: Affine::IDENTITY,
        fill: Some(fill),
        stroke: None,
        fill_rule: FillRule::NonZero,
        color: ColorEffect::default(),
        layer: LayerId(0),
        element: ElementId(0),
        outline: None,
        clip: None,
        masks: None,
        guide: false,
        group: None,
        image: None,
    }
}

fn list(items: Vec<DisplayItem>) -> DisplayList {
    DisplayList { items, ..Default::default() }
}

fn view(w: u32, h: u32, bg: Option<Rgba>) -> View {
    View { transform: Affine::IDENTITY, width: w, height: h, background: bg }
}

fn stops(colors: &[(f32, Rgba)]) -> Vec<GradientStop> {
    colors.iter().map(|&(offset, color)| GradientStop { offset, color }).collect()
}

fn group(parent: Option<u32>, alpha: f32, blend: BlendMode) -> Group {
    Group { parent, alpha, blend, filters: Default::default(), clip: None }
}

fn clipped(mut i: DisplayItem, clip: u32) -> DisplayItem {
    i.clip = Some(clip);
    i
}

fn mask(path: Path, clip: u32) -> DisplayItem {
    let mut m = item(path, Paint::Solid(Rgba::new(1.0, 0.0, 1.0, 0.3)));
    m.masks = Some(clip);
    m
}

fn in_group(mut i: DisplayItem, g: u32) -> DisplayItem {
    i.group = Some(g);
    i
}

/// Analytic scenes: each stresses one part of the coverage or paint model.
fn scenes() -> Vec<(&'static str, DisplayList, View)> {
    let red = Rgba::new(0.9, 0.1, 0.1, 1.0);
    let blue = Rgba::new(0.1, 0.2, 0.9, 0.6);
    let mut out = Vec::new();
    // Fractional rectangle edges across tile boundaries and render bands.
    out.push(("rects", list(vec![item(Path::rect(3.25, 20.5, 40.0, 61.75), Paint::Solid(red)), item(Path::rect(-30.0, 7.3, 64.6, 15.1), Paint::Solid(blue))]), view(96, 100, Some(Rgba::WHITE))));
    // A circle (curves flattened by the oracle), on transparent.
    out.push(("circle", list(vec![item(Path::ellipse(50.0, 50.0, 40.0, 40.0), Paint::Solid(Rgba::BLACK))]), view(100, 100, None)));
    // Even-odd and non-zero holes from the same rings.
    let mut ring = Path::rect(4.0, 4.0, 60.0, 60.0);
    ring.cmds.extend(Path::rect(16.0, 16.0, 36.0, 36.0).cmds);
    let mut eo = item(ring.clone(), Paint::Solid(red));
    eo.fill_rule = FillRule::EvenOdd;
    let mut nz = item(ring, Paint::Solid(blue));
    nz.transform = Affine::translate(60.0, 10.0);
    out.push(("fill rules", list(vec![eo, nz]), view(140, 80, Some(Rgba::WHITE))));
    // A self-intersecting star: winding 2 in the middle.
    let mut star = item(Path::star(40.0, 40.0, 36.0, 14.0, 5), Paint::Solid(Rgba::new(0.2, 0.6, 0.3, 1.0)));
    star.fill_rule = FillRule::EvenOdd;
    out.push(("star", list(vec![star]), view(80, 80, Some(Rgba::WHITE))));
    // Gradients under rotation and scale.
    let mut lin = item(Path::rect(0.0, 0.0, 100.0, 60.0), Paint::Linear { start: Vec2::new(0.0, 0.0), end: Vec2::new(100.0, 0.0), stops: stops(&[(0.0, red), (0.5, Rgba::WHITE), (1.0, blue)]) });
    lin.transform = Affine::translate(70.0, 10.0).mul(&Affine::rotate(0.4)).mul(&Affine::scale(1.2, 0.8));
    let rad = item(Path::ellipse(60.0, 90.0, 50.0, 30.0), Paint::Radial { center: Vec2::new(60.0, 90.0), radius: 50.0, stops: stops(&[(0.0, Rgba::WHITE), (1.0, Rgba::new(0.0, 0.0, 0.0, 0.3))]) });
    out.push(("gradients", list(vec![lin, rad]), view(180, 130, Some(Rgba::new(0.9, 0.9, 0.85, 1.0)))));
    // Strokes: wide, thin (sub-pixel, faded), on open and closed paths.
    let mut wide = item(Path::line(Vec2::new(10.0, 20.0), Vec2::new(150.0, 70.0)), Paint::Solid(red));
    wide.fill = None;
    wide.stroke = Some(Stroke { color: Rgba::BLACK, width: 9.0, profile: None });
    let mut thin = item(Path::ellipse(80.0, 50.0, 60.0, 30.0), Paint::Solid(Rgba::new(1.0, 0.9, 0.2, 1.0)));
    thin.stroke = Some(Stroke { color: Rgba::new(0.1, 0.1, 0.4, 1.0), width: 0.4, profile: None });
    out.push(("strokes", list(vec![thin, wide]), view(160, 100, Some(Rgba::WHITE))));
    // Variable-width strokes (ADR 0018) come prepared by the oracle, so the GPU draws them alike.
    let mut taper = item(Path::line(Vec2::new(10.0, 80.0), Vec2::new(150.0, 15.0)), Paint::Solid(red));
    taper.fill = None;
    taper.stroke = Some(Stroke::new(Rgba::BLACK, 12.0).with_profile(anicraft_doc::ProfilePreset::TaperBoth.profile()));
    let mut bulge = item(Path::ellipse(80.0, 50.0, 55.0, 28.0), Paint::Solid(Rgba::new(0.3, 0.8, 0.5, 1.0)));
    bulge.stroke = Some(Stroke::new(Rgba::new(0.1, 0.1, 0.4, 1.0), 8.0).with_profile(anicraft_doc::ProfilePreset::Bulge.profile()));
    out.push(("variable-width strokes", list(vec![bulge, taper]), view(160, 100, Some(Rgba::WHITE))));
    // Larger than the surface on every side; colour effects (onion-skin style tint and alpha).
    let mut huge = item(Path::ellipse(50.0, 40.0, 300.0, 200.0), Paint::Solid(red));
    huge.color = ColorEffect { alpha: 0.5, tint: Rgba::hex(0x4cd07d), tint_amount: 0.7 };
    out.push(("off-surface", list(vec![huge]), view(100, 80, Some(Rgba::WHITE))));
    out.extend(composited_scenes());
    out
}

/// Masks and groups (ADR 0029): each stresses one part of the oracle's band compositor.
fn composited_scenes() -> Vec<(&'static str, DisplayList, View)> {
    let red = Rgba::new(0.9, 0.1, 0.1, 1.0);
    let blue = Rgba::new(0.1, 0.2, 0.9, 0.6);
    let green = Rgba::new(0.2, 0.7, 0.3, 0.8);
    let grad = || Paint::Linear { start: Vec2::new(0.0, 0.0), end: Vec2::new(120.0, 40.0), stops: stops(&[(0.0, red), (0.5, Rgba::new(1.0, 0.9, 0.3, 1.0)), (1.0, blue)]) };
    let mut out = Vec::new();
    // A mask: a gradient seen through a circle, an unclipped shape over it, and one under it.
    let dl = DisplayList {
        items: vec![item(Path::rect(70.0, 0.0, 50.0, 100.0), Paint::Solid(green)), clipped(item(Path::rect(0.0, 0.0, 120.0, 100.0), grad()), 0), mask(Path::ellipse(50.0, 50.0, 37.3, 33.1), 0), item(Path::rect(10.0, 60.0, 30.0, 30.0), Paint::Solid(blue))],
        clip_parents: vec![None],
        groups: Vec::new(),
    };
    out.push(("mask", dl, view(120, 100, Some(Rgba::WHITE))));
    // Mask artwork that overlaps unites (no cancelled overlap), even-odd artwork keeps its hole,
    // and a nested mask is inside its parent's; shapes switch between the clips.
    let mut ring = Path::rect(20.0, 20.0, 80.0, 70.0);
    ring.cmds.extend(Path::rect(45.0, 40.0, 30.0, 25.0).cmds);
    let mut eo = mask(ring, 0);
    eo.fill_rule = FillRule::EvenOdd;
    let mut star = mask(Path::star(60.0, 55.0, 50.0, 20.0, 6), 1);
    star.transform = Affine::rotate(0.1);
    let mut stripes = Vec::new();
    for k in 0..6 {
        let c = if k % 2 == 0 { red } else { blue };
        stripes.push(clipped(item(Path::rect(0.0, 7.0 + 17.3 * k as f32, 130.0, 9.0), Paint::Solid(c)), 1));
        stripes.push(clipped(item(Path::rect(3.0 + 21.1 * k as f32, 0.0, 6.5, 110.0), Paint::Solid(green)), if k % 3 == 0 { 0 } else { 1 }));
    }
    let mut items = stripes;
    items.extend([eo, mask(Path::ellipse(95.0, 20.0, 30.0, 18.0), 0), star]);
    out.push(("nested masks", DisplayList { items, clip_parents: vec![None, Some(0)], groups: Vec::new() }, view(130, 110, Some(Rgba::new(0.95, 0.95, 0.9, 1.0)))));
    // Group alpha: overlapping shapes fade as one; nested groups multiply; a group spans tiles
    // and the render bands; a top-level shape between two groups.
    let dl = DisplayList {
        items: vec![
            in_group(item(Path::rect(5.0, 5.0, 60.0, 50.0), Paint::Solid(red)), 0),
            in_group(item(Path::ellipse(55.0, 40.0, 35.0, 30.0), grad()), 0),
            in_group(item(Path::rect(30.0, 20.0, 70.0, 20.0), Paint::Solid(blue)), 1),
            in_group(item(Path::star(80.0, 70.0, 30.0, 12.0, 5), Paint::Solid(green)), 2),
            in_group(item(Path::rect(70.0, 50.0, 40.0, 40.0), Paint::Solid(Rgba::BLACK)), 1),
            item(Path::rect(0.0, 75.0, 140.0, 8.0), Paint::Solid(Rgba::new(1.0, 1.0, 0.0, 0.7))),
            in_group(item(Path::ellipse(30.0, 85.0, 25.0, 20.0), Paint::Solid(red)), 3),
        ],
        clip_parents: Vec::new(),
        groups: vec![group(None, 0.5, BlendMode::Normal), group(Some(0), 0.7, BlendMode::Layer), group(Some(1), 0.4, BlendMode::Normal), group(None, 0.35, BlendMode::Normal)],
    };
    out.push(("group alpha", dl, view(140, 110, Some(Rgba::WHITE))));
    // Every blend mode, over a striped, partly transparent backdrop, at full and part alpha.
    let mut items = Vec::new();
    let mut groups = Vec::new();
    for k in 0..4 {
        items.push(item(Path::rect(0.0, k as f32 * 24.0, 14.0 * 24.0, 12.0), Paint::Solid(Rgba::new(0.2 + 0.2 * k as f32, 0.8 - 0.15 * k as f32, 0.4, 0.5 + 0.15 * k as f32))));
    }
    for (i, &m) in BlendMode::ALL.iter().enumerate() {
        let g = groups.len() as u32;
        groups.push(group(None, if i % 2 == 0 { 1.0 } else { 0.6 }, m));
        let x = i as f32 * 24.0;
        items.push(in_group(item(Path::ellipse(x + 12.0, 30.0, 11.0, 28.0), grad()), g));
        items.push(in_group(item(Path::rect(x + 2.0, 60.0, 20.0, 30.0), Paint::Solid(Rgba::new(0.9, 0.5, 0.2, 0.7))), g));
    }
    out.push(("blend modes", DisplayList { items, clip_parents: Vec::new(), groups }, view(14 * 24, 96, Some(Rgba::new(0.3, 0.3, 0.5, 0.8)))));
    // Masks inside groups and groups inside masks, with a blend mode, on transparent.
    let dl = DisplayList {
        items: vec![
            in_group(clipped(item(Path::rect(0.0, 0.0, 100.0, 60.0), grad()), 0), 0),
            in_group(item(Path::ellipse(70.0, 40.0, 30.0, 30.0), Paint::Solid(blue)), 0),
            mask(Path::ellipse(40.0, 30.0, 32.0, 26.0), 0),
            in_group(clipped(item(Path::rect(20.0, 20.0, 80.0, 60.0), Paint::Solid(red)), 1), 1),
            in_group(clipped(item(Path::star(60.0, 50.0, 30.0, 12.0, 7), Paint::Solid(green)), 1), 2),
            mask(Path::rect(30.0, 10.0, 50.0, 70.0), 1),
        ],
        clip_parents: vec![None, Some(0)],
        groups: vec![group(None, 0.8, BlendMode::Normal), group(None, 0.9, BlendMode::Multiply), group(Some(1), 0.5, BlendMode::Screen)],
    };
    out.push(("masks and groups", dl, view(110, 90, None)));
    // Malformed links, as the oracle treats them: a clip whose parent is later (empty, hides
    // what it clips), a group whose parent is later (top-level), a clip index past the list.
    let dl = DisplayList {
        items: vec![
            clipped(item(Path::rect(0.0, 0.0, 40.0, 40.0), Paint::Solid(red)), 0),
            mask(Path::rect(0.0, 0.0, 30.0, 30.0), 0),
            clipped(item(Path::rect(10.0, 10.0, 40.0, 40.0), Paint::Solid(blue)), 9),
            in_group(item(Path::rect(5.0, 20.0, 40.0, 10.0), Paint::Solid(green)), 1),
            in_group(item(Path::rect(20.0, 5.0, 10.0, 40.0), Paint::Solid(Rgba::BLACK)), 7),
        ],
        clip_parents: vec![Some(1), None],
        groups: vec![group(Some(1), 0.5, BlendMode::Normal), group(Some(1), 0.5, BlendMode::Difference)],
    };
    out.push(("malformed links", dl, view(50, 50, Some(Rgba::WHITE))));
    out
}

/// The fine shader's arithmetic, on the CPU, over a binned scene.
fn emulate(b: &Binned, v: &View) -> Vec<Premul> {
    let (w, h) = (v.width as usize, v.height as usize);
    let bg = v.background.map_or([0.0; 4], premultiply);
    let mut out = vec![[0.0f32; 4]; w * h];
    for y in 0..h {
        for x in 0..w {
            let tile = (y / TILE as usize) * b.tiles_x as usize + x / TILE as usize;
            let (lx, ly) = (x % TILE as usize, y % TILE as usize);
            let (start, count) = (b.index[2 * tile] as usize, b.index[2 * tile + 1] as usize);
            let mut c = bg;
            let mut stack: Vec<Premul> = Vec::new();
            let (mut mask, mut level) = (1.0f32, 0.0f32);
            for i in 0..count {
                let e = b.entries_at as usize + (start + i) * 4;
                let (op, first, n, bd) = (b.index[e] as usize, b.index[e + 1] as usize, b.index[e + 2] as usize, b.index[e + 3]);
                match b.index[e] {
                    bn::OPEN => {
                        assert!(stack.len() < bn::MAX_GROUP_DEPTH);
                        stack.push(c);
                        c = [0.0; 4];
                        continue;
                    }
                    bn::CLOSE => {
                        let below = stack.pop().unwrap();
                        c = close_group(c, below, f32::from_bits(b.index[e + 1]), b.index[e + 2]);
                        continue;
                    }
                    bn::MASK_RESET => {
                        mask = 1.0;
                        continue;
                    }
                    bn::LEVEL_BEGIN => {
                        level = 0.0;
                        continue;
                    }
                    bn::LEVEL_END => {
                        mask *= level;
                        continue;
                    }
                    k => assert_eq!(k & bn::CTRL, 0, "unknown control entry {k:x}"),
                }
                let mut wnd = if bd == binning::NONE { 0.0 } else { b.geo[bd as usize + ly / 4][ly % 4] };
                for k in 0..n {
                    let s = b.geo[first + k];
                    wnd += piece_cover(s, lx as f32, ly as f32);
                }
                let hd = b.ops_at as usize + op * 4;
                let mut cov = if b.index[hd + 1] == 0 { wnd.abs().min(1.0) } else { let t = wnd.abs() % 2.0; if t > 1.0 { 2.0 - t } else { t } };
                let flags = b.index[hd + 3];
                if flags & bn::ART != 0 {
                    let k = cov.clamp(0.0, 1.0);
                    level = level + k - level * k;
                    continue;
                }
                if flags & bn::CLIPPED != 0 {
                    cov *= mask;
                }
                if cov <= 1e-4 {
                    continue;
                }
                let s = match b.index[hd] {
                    0 => solid_of(b, op),
                    kind => {
                        let base = op * PAINT_VEC4S;
                        let (m, nn, g) = (b.paints[base + 1], b.paints[base + 2], b.paints[base + 3]);
                        let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                        let q = Vec2::new(m[0] * p.x + m[2] * p.y + nn[0], m[1] * p.x + m[3] * p.y + nn[1]);
                        let d = q - Vec2::new(nn[2], nn[3]);
                        let t = if kind == 1 { d.x * g[0] + d.y * g[1] } else { d.length() * g[2] };
                        let i = (t.clamp(0.0, 1.0) * 255.0 + 0.5) as usize & 255;
                        b.paints[b.index[hd + 2] as usize + i]
                    }
                };
                let sa = s[3] * cov;
                for k in 0..4 {
                    c[k] = s[k] * cov + c[k] * (1.0 - sa);
                }
            }
            assert!(stack.is_empty(), "every group closes");
            out[y * w + x] = c;
        }
    }
    out
}

/// `fine.wgsl` › `close_group` and `blend`, on the CPU (the oracle's formulas, written again
/// from the shader's numbering).
fn close_group(s: Premul, below: Premul, alpha: f32, m: u32) -> Premul {
    if alpha <= 0.0 {
        return below;
    }
    if m != 0 {
        if s[3] <= 0.0 {
            return below;
        }
        let s = [s[0] * alpha, s[1] * alpha, s[2] * alpha, s[3] * alpha];
        let (sa, ba) = (s[3].clamp(0.0, 1.0), below[3].clamp(0.0, 1.0));
        if m == 11 {
            return [below[0] * sa, below[1] * sa, below[2] * sa, ba * sa];
        }
        if m == 12 {
            let k = 1.0 - sa;
            return [below[0] * k, below[1] * k, below[2] * k, ba * k];
        }
        let un = |c: f32, a: f32| if a > 1e-6 { (c / a).clamp(0.0, 1.0) } else { 0.0 };
        let hard = |cb: f32, cs: f32| if cs <= 0.5 { cb * 2.0 * cs } else { let t = 2.0 * cs - 1.0; cb + t - cb * t };
        let f = |cb: f32, cs: f32| match m {
            1 => cb * cs,
            2 => cb + cs - cb * cs,
            3 => cb.min(cs),
            4 => cb.max(cs),
            5 => (cb - cs).abs(),
            6 => (cb + cs).min(1.0),
            7 => (cb - cs).max(0.0),
            8 => 1.0 - cb,
            9 => hard(cb, cs),
            10 => hard(cs, cb),
            _ => cs,
        };
        let mut o = [0.0; 4];
        for i in 0..3 {
            o[i] = (1.0 - ba) * s[i] + (1.0 - sa) * below[i] + sa * ba * f(un(below[i], ba), un(s[i], sa));
        }
        o[3] = sa + ba - sa * ba;
        return o;
    }
    let sa = s[3] * alpha;
    if sa <= 0.0 {
        return below;
    }
    let inv = 1.0 - sa;
    [s[0] * alpha + below[0] * inv, s[1] * alpha + below[1] * inv, s[2] * alpha + below[2] * inv, sa + below[3] * inv]
}

fn f32_diff(a: &[Premul], b: &[Premul]) -> (f64, f32) {
    let mut sum = 0.0f64;
    let mut max = 0.0f32;
    for (p, q) in a.iter().zip(b) {
        for k in 0..4 {
            let d = (p[k] - q[k]).abs();
            sum += f64::from(d);
            max = max.max(d);
        }
    }
    (sum / (a.len().max(1) * 4) as f64, max)
}

/// The sample seen through a camera (ADR 0027): zoomed, turned and tinted, the sky a layer
/// deep, so every transform the camera adds reaches the GPU's fills.
fn camera_sample() -> anicraft_doc::Document {
    let mut doc = sample::demo();
    let cam = anicraft_doc::Camera { center: Vec2::new(700.0, 380.0), zoom: 1.6, rotation: -0.15, tint: Rgba::hex(0x3366ff), tint_amount: 0.3 };
    let mut layer = anicraft_doc::Layer::new(LayerId(9_000), "Camera");
    layer.kind = anicraft_doc::LayerKind::Camera;
    let mut k = anicraft_doc::Keyframe::new(0, doc.frame_count());
    k.elements.push(Arc::new(cam.to_element(ElementId(9_001))));
    layer.keyframes = vec![Arc::new(k)];
    let tl = doc.timeline_mut(0);
    if let Some(last) = tl.layers.last_mut() {
        Arc::make_mut(last).depth = 1500.0;
    }
    tl.layers.insert(0, Arc::new(layer));
    doc
}

/// The sample with what ADR 0029 draws on the GPU, built the way artists build it: the Sun
/// layer is a mask the Hills show through, the ball symbol multiplies at 60 % and the star
/// symbol (itself a nested symbol) fades to half.
fn composited_sample() -> anicraft_doc::Document {
    let mut doc = sample::demo();
    let tl = doc.timeline_mut(0);
    let sun = tl.layers.get(2).map(|l| l.id);
    for (i, layer) in tl.layers.iter_mut().enumerate() {
        let l = Arc::make_mut(layer);
        match i {
            0 | 1 => {
                for k in &mut l.keyframes {
                    for e in &mut Arc::make_mut(k).elements {
                        let e = Arc::make_mut(e);
                        e.color.alpha = if i == 0 { 0.6 } else { 0.5 };
                        if let ElementKind::Symbol { blend, .. } = &mut e.kind
                            && i == 0
                        {
                            *blend = BlendMode::Multiply;
                        }
                    }
                }
            }
            2 => l.kind = LayerKind::Mask,
            3 => l.parent = sun,
            _ => {}
        }
    }
    doc
}

#[test]
fn binned_coverage_matches_the_oracle_on_the_cpu() {
    let mut all = scenes();
    let doc = sample::demo();
    for (f, s) in [(0u32, 0.5f32), (13, 0.37), (40, 1.0)] {
        all.push(("sample", anicraft_doc::evaluate(&doc, f), View::stage(&doc, s)));
    }
    let cam = camera_sample();
    for (f, s) in [(0u32, 0.5f32), (20, 0.8)] {
        all.push(("camera", anicraft_doc::evaluate(&cam, f), View::stage(&cam, s)));
    }
    let comp = composited_sample();
    for (f, s) in [(0u32, 0.4f32), (30, 0.6)] {
        all.push(("composited sample", anicraft_doc::evaluate(&comp, f), View::stage(&comp, s)));
    }
    for (name, dl, v) in all {
        let (scene, _) = prepare_scene(&dl, &v).unwrap();
        let b = binning::bin(&scene, v.width, v.height).unwrap();
        let (oracle, _) = render(&dl, &v).unwrap();
        let (mean, max) = f32_diff(&emulate(&b, &v), &oracle.data);
        assert!(mean < 1e-4 && max < 2e-3, "{name}: mean {mean:e}, max {max:e}");
    }
}

#[test]
fn the_composited_sample_really_has_masks_and_groups() {
    let comp = composited_sample();
    let dl = anicraft_doc::evaluate(&comp, 10);
    assert!(dl.items.iter().any(|i| i.masks.is_some()) && dl.items.iter().any(|i| i.clip.is_some()));
    assert!(dl.groups.iter().any(|g| g.blend == BlendMode::Multiply) && dl.groups.iter().any(|g| (g.alpha - 0.5).abs() < 1e-6));
    // ...and that they show: it differs from the plain sample.
    let v = View::stage(&comp, 0.25);
    let (a, _) = render(&dl, &v).unwrap();
    let (b, _) = render(&anicraft_doc::evaluate(&sample::demo(), 10), &v).unwrap();
    assert!(f32_diff(&a.data, &b.data).0 > 1e-3);
}

#[test]
fn binning_shapes_the_scene_into_tiles() {
    // One rectangle exactly covering tiles 1..3 × 1..2 of a 64 × 48 surface: its edges sit on
    // tile boundaries, so the interior tiles carry a backdrop and no pieces.
    let dl = list(vec![item(Path::rect(16.0, 16.0, 32.0, 16.0), Paint::Solid(Rgba::BLACK))]);
    let v = view(64, 48, None);
    let bin = |dl: &DisplayList, v: &View| binning::bin(&prepare_scene(dl, v).unwrap().0, v.width, v.height).unwrap();
    let b = bin(&dl, &v);
    assert_eq!((b.tiles_x, b.tiles_y), (4, 3));
    let count = |b: &Binned, tx: usize, ty: usize| b.index[2 * (ty * b.tiles_x as usize + tx) + 1];
    assert_eq!((count(&b, 0, 1), count(&b, 1, 1), count(&b, 2, 1), count(&b, 3, 1)), (0, 1, 1, 0));
    assert_eq!(count(&b, 1, 0) + count(&b, 1, 2), 0, "nothing above or below");
    // Odd sizes: partial tiles at the right and bottom still bin.
    let dl = list(vec![item(Path::ellipse(20.0, 10.0, 30.0, 30.0), Paint::Solid(Rgba::BLACK))]);
    let v = view(37, 21, None);
    let b = bin(&dl, &v);
    assert_eq!((b.tiles_x, b.tiles_y), (3, 2));
    let (oracle, _) = render(&dl, &v).unwrap();
    assert!(f32_diff(&emulate(&b, &v), &oracle.data).1 < 2e-3);
    // Nothing to draw: an empty scene bins to empty tile lists.
    let b = bin(&list(vec![]), &view(40, 40, None));
    assert_eq!((b.entries, b.pieces, b.work), (0, 0, 0));
    assert!(b.index.iter().all(|&v| v == 0));
    // A group in one tile: open, the fill, close; a clipped fill: the clip's level first. In
    // the middle tile the mask has no artwork, so the clipped fill is left out there.
    let dl = DisplayList {
        items: vec![
            in_group(item(Path::rect(1.0, 1.0, 4.0, 4.0), Paint::Solid(Rgba::BLACK)), 0),
            clipped(item(Path::rect(0.0, 0.0, 40.0, 16.0), Paint::Solid(Rgba::BLACK)), 0),
            mask(Path::rect(2.0, 2.0, 5.0, 5.0), 0),
            mask(Path::rect(34.0, 2.0, 4.0, 4.0), 0),
        ],
        clip_parents: vec![None],
        groups: vec![group(None, 0.5, BlendMode::Screen)],
    };
    let b = bin(&dl, &view(40, 16, None));
    let stream = |b: &Binned, t: usize| -> Vec<u32> {
        let (s, n) = (b.index[2 * t] as usize, b.index[2 * t + 1] as usize);
        (s..s + n).map(|i| b.index[b.entries_at as usize + 4 * i]).collect()
    };
    // (The clip coverage is per pixel, apart from the group stack: it may be computed first.)
    assert_eq!(stream(&b, 0), vec![bn::OPEN, 0, bn::MASK_RESET, bn::LEVEL_BEGIN, 2, bn::LEVEL_END, bn::CLOSE, 1]);
    assert_eq!(stream(&b, 1), Vec::<u32>::new(), "no mask artwork in the middle tile");
    assert_eq!(stream(&b, 2), vec![bn::MASK_RESET, bn::LEVEL_BEGIN, 3, bn::LEVEL_END, 1]);
    let (oracle, _) = render(&dl, &view(40, 16, None)).unwrap();
    assert!(f32_diff(&emulate(&b, &view(40, 16, None)), &oracle.data).1 < 2e-3);
    let close = (0..b.entries).map(|i| b.entries_at as usize + 4 * i).find(|&e| b.index[e] == bn::CLOSE).unwrap();
    assert_eq!((f32::from_bits(b.index[close + 1]), b.index[close + 2]), (0.5, bn::blend_code(BlendMode::Screen)));
    assert!(b.work >= b.entries as u64);
}

#[test]
fn nesting_deeper_than_the_shader_holds_goes_to_the_cpu() {
    let deep = |n: usize| DisplayList {
        items: vec![in_group(item(Path::rect(0.0, 0.0, 8.0, 8.0), Paint::Solid(Rgba::BLACK)), n as u32 - 1)],
        clip_parents: Vec::new(),
        groups: (0..n).map(|i| group(i.checked_sub(1).map(|p| p as u32), 0.9, BlendMode::Normal)).collect(),
    };
    let v = view(8, 8, None);
    let bin = |dl: &DisplayList| binning::bin(&prepare_scene(dl, &v).unwrap().0, 8, 8).map(|_| ());
    assert!(bin(&deep(bn::MAX_GROUP_DEPTH)).is_ok());
    assert_eq!(bin(&deep(bn::MAX_GROUP_DEPTH + 1)).err(), Some(NeedsFull::Groups));
    let masks = |n: usize| {
        let mut items: Vec<DisplayItem> = (0..n).map(|c| mask(Path::rect(0.0, 0.0, 8.0, 8.0), c as u32)).collect();
        items.push(clipped(item(Path::rect(0.0, 0.0, 8.0, 8.0), Paint::Solid(Rgba::BLACK)), n as u32 - 1));
        DisplayList { items, clip_parents: (0..n).map(|i| i.checked_sub(1).map(|p| p as u32)).collect(), groups: Vec::new() }
    };
    assert!(bin(&masks(bn::MAX_CLIP_DEPTH)).is_ok());
    assert_eq!(bin(&masks(bn::MAX_CLIP_DEPTH + 1)).err(), Some(NeedsFull::Clips));
    // At the limit, still the oracle's picture.
    for dl in [deep(bn::MAX_GROUP_DEPTH), masks(bn::MAX_CLIP_DEPTH)] {
        let b = binning::bin(&prepare_scene(&dl, &v).unwrap().0, 8, 8).unwrap();
        assert!(f32_diff(&emulate(&b, &v), &render(&dl, &v).unwrap().0.data).1 < 2e-3);
    }
}

#[test]
fn degenerate_geometry_bins_without_trouble() {
    // NaN and infinite points, zero-area polygons, edges far outside the surface; clip and
    // group indices past their tables.
    let fill = |clip, group| anicraft_render::Fill {
        polys: vec![
            vec![Vec2::new(f32::NAN, 0.0), Vec2::new(5.0, 5.0), Vec2::new(1.0, f32::INFINITY)],
            vec![Vec2::new(3.0, 3.0), Vec2::new(3.0, 3.0), Vec2::new(3.0, 3.0)],
            vec![Vec2::new(-1e9, -1e9), Vec2::new(1e9, 5.0), Vec2::new(-1e9, 1e9)],
            vec![Vec2::new(1.0, 1.0)],
        ],
        rule: FillRule::NonZero,
        shader: anicraft_render::paint::Shader::Solid([1.0; 4]),
        bounds: anicraft_render::raster::IRect { x0: 0, y0: 0, x1: 32, y1: 32 },
        clip,
        group,
    };
    let scene = anicraft_render::Scene { fills: vec![fill(None, None), fill(Some(7), Some(9))], clips: Vec::new(), groups: Vec::new() };
    let b = binning::bin(&scene, 32, 32).unwrap();
    assert!(b.geo.iter().flatten().all(|v| v.is_finite()));
}

// ----- the real GPU ----------------------------------------------------------------------------

/// One device for all tests; `None` (with the reason printed) when this machine has no usable
/// adapter.
fn gpu() -> Option<&'static GpuRenderer> {
    static GPU: OnceLock<Option<GpuRenderer>> = OnceLock::new();
    GPU.get_or_init(|| match GpuRenderer::new() {
        Ok(g) => {
            eprintln!("GPU parity tests on {}", g.adapter);
            Some(g)
        }
        Err(e) => {
            eprintln!("skipping GPU parity tests: {e}");
            None
        }
    })
    .as_ref()
}

/// The GPU's picture against the oracle's, read back and through a texture (byte for byte
/// the same as each other).
fn assert_parity(g: &GpuRenderer, name: &str, dl: &DisplayList, v: &View) -> Diff {
    let f = g.render(dl, v, Output::Premul8).unwrap();
    let (oracle, st) = render(dl, v).unwrap();
    let d = compare_premul8(&f.premul8, &oracle.to_premul_rgba8()).unwrap();
    assert!(d.mean <= MEAN_TARGET / 16.0 && d.max <= 2, "{name}: {d:?}");
    assert_eq!((f.stats.drawn, f.stats.culled), (st.drawn, st.culled), "{name}");
    let t = g.render_texture(dl, v).unwrap();
    assert_eq!((t.width, t.height), (v.width, v.height));
    assert!(g.read_texture(&t).unwrap() == f.premul8, "{name}: the texture holds the read-back bytes");
    d
}

#[test]
fn gpu_matches_the_oracle_on_analytic_scenes() {
    let Some(g) = gpu() else { return };
    for (name, dl, v) in scenes() {
        assert_parity(g, name, &dl, &v);
        // Float output agrees closely too.
        let ff = g.render(&dl, &v, Output::Float).unwrap();
        let (oracle, _) = render(&dl, &v).unwrap();
        let (mean, max) = f32_diff(&ff.pixmap.unwrap().data, &oracle.data);
        assert!(mean < 1e-4 && max < 4e-3, "{name} (f32): mean {mean:e}, max {max:e}");
    }
}

#[test]
fn gpu_matches_the_oracle_on_masks_groups_and_blend_modes() {
    let Some(g) = gpu() else { return };
    for (name, dl, v) in composited_scenes() {
        let d = assert_parity(g, name, &dl, &v);
        eprintln!("{name}: mean {:.5} / max {} / {} of {} px differ", d.mean, d.max, d.differing, d.pixels);
    }
    let comp = composited_sample();
    for (f, s) in [(0u32, 1.0f32), (7, 0.5), (24, 1.5), (40, 0.37)] {
        let d = assert_parity(g, "composited sample", &anicraft_doc::evaluate(&comp, f), &View::stage(&comp, s));
        eprintln!("composited sample {f} at {s}: mean {:.5} / max {} / {} of {} px differ", d.mean, d.max, d.differing, d.pixels);
    }
}

#[test]
fn gpu_matches_the_oracle_on_the_sample_scene() {
    let Some(g) = gpu() else { return };
    let doc = sample::demo();
    let cam = camera_sample();
    let frames = doc.frame_count();
    for (d, f, s) in [(&doc, 0u32, 1.0f32), (&doc, 7, 0.5), (&doc, 13, 0.37), (&doc, frames / 2, 1.5), (&doc, frames.saturating_sub(1), 0.25), (&cam, 0, 1.0), (&cam, 20, 0.5)] {
        assert_parity(g, &format!("frame {f} at {s}"), &anicraft_doc::evaluate(d, f), &View::stage(d, s));
    }
}

#[test]
fn gpu_matches_the_oracle_on_many_shapes_and_tiles() {
    let Some(g) = gpu() else { return };
    let doc = sample::stress(6, 300, 24);
    let dl = anicraft_doc::evaluate(&doc, 5);
    // A whole frame and an offset, zoomed tile like the stage asks for.
    for v in [View::stage(&doc, 0.5), View { transform: Affine::translate(-512.0, -256.0).mul(&Affine::scale(1.7, 1.7)), width: 512, height: 512, background: Some(doc.stage.background) }] {
        assert_parity(g, "stress", &dl, &v);
    }
}

#[test]
fn gpu_hands_back_what_it_does_not_draw_and_stays_usable() {
    let Some(g) = gpu() else { return };
    let mut filtered = in_group(item(Path::rect(0.0, 0.0, 8.0, 8.0), Paint::Solid(Rgba::BLACK)), 0);
    filtered.color.alpha = 0.5;
    let mut grp = group(None, 1.0, BlendMode::Normal);
    grp.filters = Arc::from(vec![anicraft_doc::Filter::Blur { blur_x: 4.0, blur_y: 4.0, quality: 1 }]);
    let dl = DisplayList { items: vec![filtered], clip_parents: Vec::new(), groups: vec![grp] };
    assert!(matches!(g.render(&dl, &view(8, 8, None), Output::Premul8), Err(GpuError::NeedsFull(NeedsFull::Filters))));
    assert!(matches!(g.render_texture(&dl, &view(8, 8, None)), Err(GpuError::NeedsFull(NeedsFull::Filters))));
    assert!(matches!(g.render(&list(vec![]), &view(0, 8, None), Output::Premul8), Err(GpuError::TooLarge(_))));
    assert!(matches!(g.render(&list(vec![]), &view(100_000, 100_000, None), Output::Premul8), Err(GpuError::TooLarge(_))));
    assert!(matches!(g.render_texture(&list(vec![]), &view(0, 8, None)), Err(GpuError::TooLarge(_))));
    // An empty list is just the background; and the device is still fine afterwards.
    let f = g.render(&list(vec![]), &view(20, 3, Some(Rgba::new(0.0, 1.0, 0.0, 1.0))), Output::Premul8).unwrap();
    assert!(f.premul8.as_chunks::<4>().0.iter().all(|p| *p == [0, 255, 0, 255]));
    assert!(g.broken().is_none());
}

#[test]
fn gpu_renders_from_many_threads_at_once() {
    let Some(g) = gpu() else { return };
    let doc = composited_sample();
    let dl = anicraft_doc::evaluate(&doc, 9);
    let v = View::stage(&doc, 0.25);
    let first = g.render(&dl, &v, Output::Premul8).unwrap().premul8;
    use rayon::prelude::*;
    let all: Vec<Vec<u8>> = (0..8).into_par_iter().map(|i| if i % 2 == 0 { g.render(&dl, &v, Output::Premul8).unwrap().premul8 } else { g.read_texture(&g.render_texture(&dl, &v).unwrap()).unwrap() }).collect();
    assert!(all.iter().all(|a| *a == first), "deterministic under concurrency");
}
